//! rp2a03_ui\src\state.rs
//! 
//! State structures for tracking sequence configuration across audio and GUI threads.

use rp2a03_core::sequence::Sequence;

/// Shared state container holding sequence configurations and editor settings.
#[derive(Debug, Clone)]
pub struct SharedSequences {
    /// Active tab selection: 0=Volume, 1=Arpeggio, 2=Pitch, 3=Hi-Pitch, 4=Duty
    pub selected_tab: usize,

    /// Text representation of each sequence in FamiTracker notation
    pub vol_text: String,
    pub arp_text: String,
    pub pitch_text: String,
    pub hipitch_text: String,
    pub duty_text: String,

    /// Parsed Sequence engines
    pub vol_seq: Sequence,
    pub arp_seq: Sequence,
    pub pitch_seq: Sequence,
    pub hipitch_seq: Sequence,
    pub duty_seq: Sequence,

    /// Modulation enable flags
    pub vol_enabled: bool,
    pub arp_enabled: bool,
    pub pitch_enabled: bool,
    pub hipitch_enabled: bool,
    pub duty_enabled: bool,
}

impl Default for SharedSequences {
    fn default() -> Self {
        Self {
            selected_tab: 0,
            vol_text: String::new(),
            arp_text: String::new(),
            pitch_text: String::new(),
            hipitch_text: String::new(),
            duty_text: String::new(),
            vol_seq: Sequence::default(),
            arp_seq: Sequence::default(),
            pitch_seq: Sequence::default(),
            hipitch_seq: Sequence::default(),
            duty_seq: Sequence::default(),
            vol_enabled: true,
            arp_enabled: false,
            pitch_enabled: false,
            hipitch_enabled: false,
            duty_enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_sequences_initialization() {
        let state = SharedSequences::default();
        assert_eq!(state.selected_tab, 0);
        assert!(state.vol_enabled);
        assert!(!state.arp_enabled);
        assert_eq!(state.vol_seq.len(), 0);
        assert_eq!(state.duty_seq.len(), 0);
        assert!(state.vol_text.is_empty());
        assert!(state.arp_text.is_empty());
        assert!(state.pitch_text.is_empty());
        assert!(state.hipitch_text.is_empty());
        assert!(state.duty_text.is_empty());
    }
}