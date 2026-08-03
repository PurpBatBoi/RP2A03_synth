//! rp2a03_common\src\midi\types.rs
//! Core value types and note/frequency conversions used by the MIDI handler.

/// Which APU channel waveform is active for this plugin instance.
///
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum ChannelMode {
    #[default]
    Pulse = 0,
    Triangle = 1,
    Noise = 2,
    Vrc6Pulse = 3,
    Vrc6Saw = 4,
}

impl ChannelMode {
    pub fn from_i32(val: i32) -> Self {
        match val {
            1 => ChannelMode::Triangle,
            2 => ChannelMode::Noise,
            3 => ChannelMode::Vrc6Pulse,
            4 => ChannelMode::Vrc6Saw,
            _ => ChannelMode::Pulse,
        }
    }
}

use rp2a03_core::NTSC_CPU_CLOCK;
use rp2a03_core::sequencer::Sequence;
use rp2a03_core::software_lfo::DEFAULT_LFO_SPEED;

/// Converts MIDI note number to frequency in Hz.
pub fn midi_note_to_freq(note: u8) -> f32 {
    440.0 * 2.0f32.powf((note as f32 - 69.0) / 12.0)
}

/// Converts frequency in Hz to NES APU 11-bit timer period value.
pub fn freq_to_period(freq: f32) -> u16 {
    if freq <= 0.0 {
        return 2047;
    }
    let t = (NTSC_CPU_CLOCK as f32 / (16.0 * freq)) - 0.5;
    t.round().clamp(0.0, 2047.0) as u16
}

/// Converts a frequency to the extended pulse-domain period used before the
/// triangle channel's octave compensation. Triangle periods are halved at the
/// register boundary, so they need the full 12-bit intermediate range to use
/// the NES timer's complete 11-bit range.
pub fn freq_to_triangle_period(freq: f32) -> u16 {
    if freq <= 0.0 {
        return 4095;
    }
    let t = (NTSC_CPU_CLOCK as f32 / (16.0 * freq)) - 0.5;
    t.round().clamp(0.0, 4095.0) as u16
}

/// Converts a frequency in Hz to VRC6 Saw channel period (14-stage accumulator cycle).
pub fn freq_to_vrc6_saw_period(freq: f32) -> u16 {
    if freq <= 0.0 {
        return 4095;
    }
    let t = (NTSC_CPU_CLOCK as f32 / (14.0 * freq)) - 0.5;
    t.round().clamp(0.0, 4095.0) as u16
}


/// Maps a MIDI key to the 4-bit NES noise period used by FamiStudio.
/// FamiStudio first converts MIDI to its internal C0-based note (`key - 11`),
/// then masks and inverts the low nibble at the `$400E` register boundary.
#[inline]
pub fn midi_note_to_noise_period(note: u8) -> u8 {
    let internal = (note as i16 - 11).clamp(1, 96) as u8;
    (internal & 0x0F) ^ 0x0F
}

#[cfg(test)]
mod noise_mapping_tests {
    use super::midi_note_to_noise_period;

    #[test]
    fn matches_famistudio_first_noise_octave() {
        assert_eq!(midi_note_to_noise_period(12), 14); // C0
        assert_eq!(midi_note_to_noise_period(26), 0); // D1, highest
        assert_eq!(midi_note_to_noise_period(27), 15); // D#1 wraps
    }

    #[test]
    fn noise_mapping_repeats_every_sixteen_keys() {
        for note in 12..=91 {
            assert_eq!(
                midi_note_to_noise_period(note),
                midi_note_to_noise_period(note + 16)
            );
        }
    }
}

/// Container holding owned copies of all 5 active sequences and their enable statuses.
#[derive(Debug, Clone, Default)]
pub struct ActiveSequences {
    pub vol_seq: Sequence,
    pub vol_enabled: bool,
    pub arp_seq: Sequence,
    pub arp_enabled: bool,
    pub pitch_seq: Sequence,
    pub pitch_enabled: bool,
    pub hipitch_seq: Sequence,
    pub hipitch_enabled: bool,
    pub duty_seq: Sequence,
    pub duty_enabled: bool,
}

/// Which of the five envelope slots actually changed when the active sequence
/// slot was switched.
///
/// dn `CSeqInstHandler::LoadInstrument` compares the incoming `CSequence*`
/// against the one the handler already holds and only calls `SetupSequence` when
/// they differ, so switching to an instrument that reuses the same envelope
/// leaves that envelope running untouched. Content equality is this plugin's
/// analog of dn's pointer identity — `ActiveSequences` owns its copies, so there
/// is no stable pointer to compare.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SequenceReload {
    pub vol: bool,
    pub arp: bool,
    pub pitch: bool,
    pub hipitch: bool,
    pub duty: bool,
}

impl SequenceReload {
    /// Every envelope changed — used when the caller cannot diff (e.g. tests) or
    /// when the whole slot is known to be new.
    pub const ALL: Self = Self {
        vol: true,
        arp: true,
        pitch: true,
        hipitch: true,
        duty: true,
    };

    pub fn any(&self) -> bool {
        self.vol || self.arp || self.pitch || self.hipitch || self.duty
    }
}

/// Host-automatable controls that mirror the corresponding MIDI CC functions.
///
/// They are synchronized only when their parameter value changes, allowing MIDI
/// CC messages to continue controlling a value until host automation changes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostAutomationControls {
    pub vibrato_depth: u8,
    pub vibrato_speed: u8,
    pub tremolo_depth: u8,
    pub tremolo_speed: u8,
    pub hardware_volume: u8,
    pub fine_pitch: i8,
    pub hi_pitch: i8,
    /// MIDI-style bipolar pitch bend value (-8192..=8191).
    pub pitch_slide: i16,
    /// Pitch-bend range in semitones (0..=24).
    pub pitch_slide_range: u8,
    pub step_time_hz: u16,
    pub portamento_enabled: bool,
    pub portamento_speed: u8,
}

impl Default for HostAutomationControls {
    fn default() -> Self {
        Self {
            vibrato_depth: 0,
            vibrato_speed: DEFAULT_LFO_SPEED,
            tremolo_depth: 0,
            tremolo_speed: DEFAULT_LFO_SPEED,
            hardware_volume: 15,
            fine_pitch: 0,
            hi_pitch: 0,
            pitch_slide: 0,
            pitch_slide_range: 2,
            step_time_hz: 60,
            portamento_enabled: false,
            portamento_speed: 0,
        }
    }
}
