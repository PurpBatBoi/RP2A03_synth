//! rp2a03_common\src\gui\state.rs
//! State structures for sequence banks and instrument settings.

use crate::ChannelMode;
use rp2a03_core::sequencer::Sequence;

/// The FamiTracker-compatible sequence-number range (`0..=127`).
pub const MAX_SEQUENCES: usize = 128;
pub const SEQUENCE_TYPE_COUNT: usize = 5;
pub const NO_PLAYHEAD_STEP: usize = usize::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequencePlayheads {
    steps: [Option<usize>; SEQUENCE_TYPE_COUNT],
}

impl Default for SequencePlayheads {
    fn default() -> Self {
        Self {
            steps: [None; SEQUENCE_TYPE_COUNT],
        }
    }
}

impl SequencePlayheads {
    pub fn from_steps(steps: [Option<usize>; SEQUENCE_TYPE_COUNT]) -> Self {
        Self { steps }
    }

    pub fn step(&self, tab: usize) -> Option<usize> {
        self.steps[Self::tab_index(tab)]
    }

    fn tab_index(tab: usize) -> usize {
        tab.min(SEQUENCE_TYPE_COUNT - 1)
    }
}

/// A numbered sequence and its editable FamiTracker text representation.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SequenceSlot {
    pub text: String,
    pub sequence: Sequence,
    /// Whether this envelope editor is enabled for this numbered sequence.
    ///
    /// This belongs to the slot rather than the envelope-type bank so changing
    /// sequence indexes does not carry the previous index's enabled state over.
    enabled: bool,
}

/// The complete set of sequences available to one envelope type.
///
/// Keeping each envelope type in its own bank lets an instrument select, for
/// example, volume sequence 1 and duty sequence 8 independently. This type is
/// channel-agnostic so triangle, noise, and future expansion-chip editors can
/// reuse it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SequenceBank {
    slots: Vec<SequenceSlot>,
}

impl Default for SequenceBank {
    fn default() -> Self {
        Self {
            slots: vec![SequenceSlot::default(); MAX_SEQUENCES],
        }
    }
}

impl SequenceBank {
    pub fn slot(&self, index: usize) -> &SequenceSlot {
        &self.slots[index.min(MAX_SEQUENCES - 1)]
    }

    pub fn slot_mut(&mut self, index: usize) -> &mut SequenceSlot {
        &mut self.slots[index.min(MAX_SEQUENCES - 1)]
    }
}

/// Shared sequence data for one instrument instance.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SharedSequences {
    /// The envelope type currently shown by the editor.
    pub selected_tab: usize,
    /// Active channel mode (Pulse / Triangle / Noise). Persisted across editor
    /// frames and drives tab visibility (e.g. Duty tab hidden for Triangle).
    pub channel_mode: ChannelMode,
    /// Whether the plugin should allocate more than one internal voice.
    pub polyphony: bool,
    /// Maximum number of internal voices to allocate (1..=8).
    pub max_voices: i32,
    pub portamento_enabled: bool,
    pub portamento_speed: i32,
    sequence_indices: [usize; SEQUENCE_TYPE_COUNT],
    sequence_banks: [SequenceBank; SEQUENCE_TYPE_COUNT],
    /// Bumped whenever sequence content is mutated (see `selected_sequence_mut`
    /// and `sequence_enabled_mut`). The audio thread compares this against a
    /// cached value to avoid re-locking and re-cloning sequence data on every
    /// `process()` call when nothing has actually changed. Not persisted —
    /// callers only ever compare it within one plugin instance's lifetime.
    #[serde(skip)]
    revision: u64,
}

impl Default for SharedSequences {
    fn default() -> Self {
        Self {
            selected_tab: 0,
            channel_mode: ChannelMode::Pulse,
            polyphony: false,
            max_voices: 8,
            portamento_enabled: false,
            portamento_speed: 0,
            sequence_indices: [0; SEQUENCE_TYPE_COUNT],
            sequence_banks: std::array::from_fn(|_| SequenceBank::default()),
            revision: 0,
        }
    }
}

impl SharedSequences {
    fn tab_index(tab: usize) -> usize {
        tab.min(SEQUENCE_TYPE_COUNT - 1)
    }

    pub fn selected_sequence_index(&self, tab: usize) -> usize {
        self.sequence_indices[Self::tab_index(tab)]
    }

    pub fn set_selected_sequence_index(&mut self, tab: usize, index: usize) {
        self.sequence_indices[Self::tab_index(tab)] = index.min(MAX_SEQUENCES - 1);
    }

    /// Select one numbered slot for every envelope type in this instrument.
    ///
    /// The shared host parameter uses this so a single automation lane selects
    /// matching sequence numbers without making the envelope banks share data.
    pub fn set_all_selected_sequence_indices(&mut self, index: usize) {
        self.sequence_indices.fill(index.min(MAX_SEQUENCES - 1));
    }

    pub fn selected_sequence(&self, tab: usize) -> &Sequence {
        let tab = Self::tab_index(tab);
        &self.sequence_banks[tab]
            .slot(self.sequence_indices[tab])
            .sequence
    }

    pub fn selected_sequence_mut(&mut self, tab: usize) -> (&mut String, &mut Sequence) {
        let tab = Self::tab_index(tab);
        let index = self.sequence_indices[tab];
        self.revision += 1;
        let slot = self.sequence_banks[tab].slot_mut(index);
        (&mut slot.text, &mut slot.sequence)
    }

    pub fn sequence_enabled(&self, tab: usize) -> bool {
        let tab = Self::tab_index(tab);
        self.sequence_banks[tab]
            .slot(self.sequence_indices[tab])
            .enabled
    }

    pub fn sequence_enabled_mut(&mut self, tab: usize) -> &mut bool {
        let tab = Self::tab_index(tab);
        let index = self.sequence_indices[tab];
        self.revision += 1;
        &mut self.sequence_banks[tab].slot_mut(index).enabled
    }

    /// Monotonic counter bumped on every content mutation. Used by the audio
    /// thread to detect whether cached sequence data is stale.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sequence_banks_are_empty_and_select_zero() {
        let state = SharedSequences::default();
        assert_eq!(state.selected_tab, 0);
        assert_eq!(state.selected_sequence_index(0), 0);
        assert_eq!(state.selected_sequence(0).len(), 0);
        assert_eq!(state.selected_sequence(4).len(), 0);
        for tab in 0..SEQUENCE_TYPE_COUNT {
            assert!(!state.sequence_enabled(tab));
        }
    }

    #[test]
    fn each_envelope_type_keeps_its_own_selected_sequence() {
        let mut state = SharedSequences::default();
        state.set_selected_sequence_index(0, 1);
        state.set_selected_sequence_index(4, 8);
        state.selected_sequence_mut(0).1.values.push(15);
        state.selected_sequence_mut(4).1.values.push(3);

        assert_eq!(state.selected_sequence_index(0), 1);
        assert_eq!(state.selected_sequence_index(4), 8);
        assert_eq!(state.selected_sequence(0).values, vec![15]);
        assert_eq!(state.selected_sequence(4).values, vec![3]);
        assert!(state.sequence_banks[0].slot(0).sequence.is_empty());
    }

    #[test]
    fn shared_sequence_number_selects_the_same_slot_for_every_envelope() {
        let mut state = SharedSequences::default();
        state.set_all_selected_sequence_indices(42);

        for tab in 0..SEQUENCE_TYPE_COUNT {
            assert_eq!(state.selected_sequence_index(tab), 42);
        }
    }

    #[test]
    fn enabled_state_belongs_to_each_numbered_sequence_slot() {
        let mut state = SharedSequences::default();

        *state.sequence_enabled_mut(0) = true;
        state.set_selected_sequence_index(0, 1);
        assert!(!state.sequence_enabled(0));

        *state.sequence_enabled_mut(0) = true;
        state.set_selected_sequence_index(0, 0);
        assert!(state.sequence_enabled(0));
        state.set_selected_sequence_index(0, 1);
        assert!(state.sequence_enabled(0));
    }
}
