//! rp2a03_common\src\gui\state.rs
//! State structures for sequence banks and instrument settings.

use rp2a03_core::sequencer::Sequence;

/// The FamiTracker-compatible sequence-number range (`0..=127`).
pub const MAX_SEQUENCES: usize = 128;
pub const SEQUENCE_TYPE_COUNT: usize = 5;

/// A numbered sequence and its editable FamiTracker text representation.
#[derive(Debug, Clone, Default)]
pub struct SequenceSlot {
    pub text: String,
    pub sequence: Sequence,
}

/// The complete set of sequences available to one envelope type.
///
/// Keeping each envelope type in its own bank lets an instrument select, for
/// example, volume sequence 1 and duty sequence 8 independently. This type is
/// channel-agnostic so triangle, noise, and future expansion-chip editors can
/// reuse it.
#[derive(Debug, Clone)]
pub struct SequenceBank {
    slots: [SequenceSlot; MAX_SEQUENCES],
}

impl Default for SequenceBank {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| SequenceSlot::default()),
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
#[derive(Debug, Clone)]
pub struct SharedSequences {
    /// The envelope type currently shown by the editor.
    pub selected_tab: usize,
    sequence_indices: [usize; SEQUENCE_TYPE_COUNT],
    sequence_banks: [SequenceBank; SEQUENCE_TYPE_COUNT],
    enabled: [bool; SEQUENCE_TYPE_COUNT],
}

impl Default for SharedSequences {
    fn default() -> Self {
        Self {
            selected_tab: 0,
            sequence_indices: [0; SEQUENCE_TYPE_COUNT],
            sequence_banks: std::array::from_fn(|_| SequenceBank::default()),
            enabled: [false; SEQUENCE_TYPE_COUNT],
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
        let slot = self.sequence_banks[tab].slot_mut(index);
        (&mut slot.text, &mut slot.sequence)
    }

    pub fn sequence_enabled(&self, tab: usize) -> bool {
        self.enabled[Self::tab_index(tab)]
    }

    pub fn sequence_enabled_mut(&mut self, tab: usize) -> &mut bool {
        &mut self.enabled[Self::tab_index(tab)]
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
}
