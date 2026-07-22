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
        let vol_text = "15".to_string();
        let arp_text = "0".to_string();
        let pitch_text = "0".to_string();
        let hipitch_text = "0".to_string();
        let duty_text = "2".to_string();

        let (vol_seq, _) = Sequence::parse_clamped(&vol_text, 0, 15);
        let (arp_seq, _) = Sequence::parse_clamped(&arp_text, -96, 96);
        let (pitch_seq, _) = Sequence::parse_clamped(&pitch_text, -128, 127);
        let (hipitch_seq, _) = Sequence::parse_clamped(&hipitch_text, -64, 63);
        let (duty_seq, _) = Sequence::parse_clamped(&duty_text, 0, 3);

        Self {
            selected_tab: 0,
            vol_text,
            arp_text,
            pitch_text,
            hipitch_text,
            duty_text,
            vol_seq,
            arp_seq,
            pitch_seq,
            hipitch_seq,
            duty_seq,
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
        assert_eq!(state.vol_seq.len(), 1);
        assert_eq!(state.duty_seq.values[0], 2);
    }
}