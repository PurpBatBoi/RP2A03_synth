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
    ///
    /// `pub(crate)` rather than fully private: `.rp2a03patch` load needs to set
    /// this directly when writing an arbitrary slot (see `rp2a03_common::format::patch`),
    /// bypassing the tab/selection-scoped accessors below which only reach the
    /// currently-selected slot.
    pub(crate) enabled: bool,
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

    pub(crate) fn slots(&self) -> &[SequenceSlot] {
        &self.slots
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
    /// The waveform each numbered slot remembers, so selecting a sequence index
    /// behaves like recalling an instrument. One value per index (not per
    /// envelope bank) because `set_all_selected_sequence_indices` already drives
    /// all 5 banks in lockstep.
    ///
    /// Needs a serde default: this whole struct is persisted by the host through
    /// nice-plug's `#[persist = "envelope_data"]`, and DAW project states saved
    /// before this field existed won't contain it.
    ///
    /// A `Vec` rather than `[ChannelMode; MAX_SEQUENCES]` for the same reason
    /// `SequenceBank` holds one: serde implements `Deserialize` for arrays only
    /// up to length 32. Always `MAX_SEQUENCES` long once constructed; the
    /// accessors below clamp against the real length so a truncated or padded
    /// deserialized value still can't index out of bounds.
    #[serde(default = "default_slot_waveforms")]
    slot_waveforms: Vec<ChannelMode>,
    /// Bumped whenever sequence content is mutated (see `selected_sequence_mut`
    /// and `sequence_enabled_mut`). The audio thread compares this against a
    /// cached value to avoid re-locking and re-cloning sequence data on every
    /// `process()` call when nothing has actually changed. Not persisted —
    /// callers only ever compare it within one plugin instance's lifetime.
    #[serde(skip)]
    revision: u64,
}

fn default_slot_waveforms() -> Vec<ChannelMode> {
    vec![ChannelMode::Pulse; MAX_SEQUENCES]
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
            slot_waveforms: default_slot_waveforms(),
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

    /// Clamps an index into `slot_waveforms`' real length, returning `None` only
    /// if the vec is empty. Length-clamping rather than `MAX_SEQUENCES`-clamping
    /// because a hand-edited or corrupt persisted project state can deserialize
    /// a shorter vec than `default_slot_waveforms` builds.
    fn slot_waveform_index(&self, index: usize) -> Option<usize> {
        let last = self.slot_waveforms.len().checked_sub(1)?;
        Some(index.min(MAX_SEQUENCES - 1).min(last))
    }

    /// The waveform remembered by one numbered slot. Out-of-range indexes clamp
    /// to the last slot, matching `selected_sequence_index`.
    pub fn slot_waveform(&self, index: usize) -> ChannelMode {
        self.slot_waveform_index(index)
            .map_or(ChannelMode::default(), |i| self.slot_waveforms[i])
    }

    /// Records the waveform for one numbered slot. Called when the user picks a
    /// waveform manually (writing through to whichever slot is selected) and
    /// when a `.rp2a03patch` restores saved per-slot waveforms.
    pub fn set_slot_waveform(&mut self, index: usize, mode: ChannelMode) {
        if let Some(i) = self.slot_waveform_index(index) {
            self.slot_waveforms[i] = mode;
        }
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

    pub(crate) fn sequence_bank(&self, tab: usize) -> &SequenceBank {
        &self.sequence_banks[Self::tab_index(tab)]
    }

    pub(crate) fn sequence_bank_mut(&mut self, tab: usize) -> &mut SequenceBank {
        self.revision += 1;
        let tab = Self::tab_index(tab);
        &mut self.sequence_banks[tab]
    }

    /// Discards all sequence-bank content, active-slot selections, and per-slot
    /// remembered waveforms across all 5 envelope types, replacing them with
    /// defaults. Leaves every other field (`channel_mode`, `polyphony`,
    /// `max_voices`, portamento settings) untouched — `.rp2a03patch` load only
    /// owns sequence data, not those params.
    ///
    /// `slot_waveforms` *is* patch-owned: a patch encodes only the slots whose
    /// waveform differs from the default, so without resetting here, loading a
    /// patch would leave stale per-slot waveforms from the previous instrument
    /// on every slot the new patch does not mention.
    pub(crate) fn clear_all_sequences(&mut self) {
        self.sequence_banks = std::array::from_fn(|_| SequenceBank::default());
        self.sequence_indices = [0; SEQUENCE_TYPE_COUNT];
        self.slot_waveforms = default_slot_waveforms();
        self.revision += 1;
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

    #[test]
    fn sequence_bank_slots_exposes_all_128_slots() {
        let state = SharedSequences::default();
        assert_eq!(state.sequence_bank(0).slots().len(), MAX_SEQUENCES);
    }

    #[test]
    fn sequence_bank_mut_allows_writing_an_arbitrary_slot() {
        let mut state = SharedSequences::default();
        state
            .sequence_bank_mut(2)
            .slot_mut(50)
            .sequence
            .values
            .push(7);
        assert_eq!(state.sequence_bank(2).slots()[50].sequence.values, vec![7]);
    }

    #[test]
    fn clear_all_sequences_resets_sequence_data_but_not_other_fields() {
        let mut state = SharedSequences::default();
        state.set_selected_sequence_index(0, 10);
        state.selected_sequence_mut(0).1.values.push(1);
        state.set_slot_waveform(10, ChannelMode::Noise);
        state.set_slot_waveform(127, ChannelMode::Vrc6Saw);
        state.channel_mode = ChannelMode::Triangle;
        state.polyphony = true;
        state.max_voices = 3;
        state.portamento_enabled = true;
        state.portamento_speed = 42;

        state.clear_all_sequences();

        assert_eq!(state.selected_sequence_index(0), 0);
        assert!(state.selected_sequence(0).is_empty());
        assert!(state.sequence_bank(0).slots()[10].sequence.is_empty());
        // Patch-owned: a `.rp2a03patch` only encodes slots that differ from the
        // default, so any slot the incoming patch omits must already be back to
        // `Pulse` rather than holding the previous instrument's waveform.
        assert_eq!(state.slot_waveform(10), ChannelMode::Pulse);
        assert_eq!(state.slot_waveform(127), ChannelMode::Pulse);
        // Not patch-owned, so these survive the clear.
        assert_eq!(state.channel_mode, ChannelMode::Triangle);
        assert!(state.polyphony);
        assert_eq!(state.max_voices, 3);
        assert!(state.portamento_enabled);
        assert_eq!(state.portamento_speed, 42);
    }

    #[test]
    fn slot_waveform_accessors_clamp_out_of_range_indexes() {
        let mut state = SharedSequences::default();

        state.set_slot_waveform(MAX_SEQUENCES, ChannelMode::Noise);
        assert_eq!(
            state.slot_waveform(MAX_SEQUENCES - 1),
            ChannelMode::Noise,
            "an out-of-range write must land on the last slot, matching \
             `set_selected_sequence_index`"
        );
        assert_eq!(state.slot_waveform(usize::MAX), ChannelMode::Noise);
        assert_eq!(
            state.slot_waveform(0),
            ChannelMode::Pulse,
            "clamping must not spill onto unrelated slots"
        );
    }

    #[test]
    fn slot_waveform_accessors_survive_a_truncated_vec() {
        // Only reachable through a hand-edited or corrupt persisted project
        // state, since `default_slot_waveforms` always builds MAX_SEQUENCES
        // entries. The accessors clamp against the real length so this can
        // never index out of bounds.
        let mut state = SharedSequences::default();
        state.slot_waveforms.truncate(2);

        state.set_slot_waveform(MAX_SEQUENCES - 1, ChannelMode::Triangle);
        assert_eq!(
            state.slot_waveform(MAX_SEQUENCES - 1),
            ChannelMode::Triangle
        );
        assert_eq!(state.slot_waveform(1), ChannelMode::Triangle);
        assert_eq!(state.slot_waveform(0), ChannelMode::Pulse);

        state.slot_waveforms.clear();
        assert_eq!(
            state.slot_waveform(0),
            ChannelMode::default(),
            "an empty vec must read back the default, not panic"
        );
        state.set_slot_waveform(0, ChannelMode::Noise);
        assert_eq!(state.slot_waveform(0), ChannelMode::default());
    }
}
