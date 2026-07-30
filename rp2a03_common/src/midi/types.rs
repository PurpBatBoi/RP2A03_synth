//! rp2a03_common\src\midi\types.rs
//! Core value types and note/frequency conversions used by the MIDI handler.

/// Which APU channel waveform is active for this plugin instance.
///
/// `Noise` is defined for future use but is not yet functional.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum ChannelMode {
    #[default]
    Pulse = 0,
    Triangle = 1,
    Noise = 2,
}

impl ChannelMode {
    pub fn from_i32(val: i32) -> Self {
        match val {
            1 => ChannelMode::Triangle,
            2 => ChannelMode::Noise,
            _ => ChannelMode::Pulse,
        }
    }
}

use rp2a03_core::sequencer::Sequence;
use rp2a03_core::software_lfo::DEFAULT_LFO_SPEED;
use rp2a03_core::NTSC_CPU_CLOCK;

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

/// Container holding owned copies of all 5 active sequences and their enable statuses.
#[derive(Debug, Clone)]
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
    pub step_time_hz: u16,
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
            step_time_hz: 60,
        }
    }
}
