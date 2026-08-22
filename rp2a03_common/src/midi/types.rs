//! `rp2a03_common\src\midi\types.rs`

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum ChannelMode {
    #[default]
    Pulse = 0,
    Triangle = 1,
    Noise = 2,
    Vrc6Pulse = 3,
    Vrc6Saw = 4,
    S5B = 5,

    Fds = 6,
}

impl ChannelMode {
    #[must_use]
    pub fn from_i32(val: i32) -> Self {
        match val {
            1 => Self::Triangle,
            2 => Self::Noise,
            3 => Self::Vrc6Pulse,
            4 => Self::Vrc6Saw,
            5 => Self::S5B,
            6 => Self::Fds,
            _ => Self::Pulse,
        }
    }
}

use crate::gui::{FDS_WAVE_LEN, FdsSettings, WaveSynthParams};
use basedrop::Shared;
use rp2a03_core::NTSC_CPU_CLOCK;
use rp2a03_core::sequencer::{Sequence, VolMode, VolMode5B};
use rp2a03_core::software_lfo::DEFAULT_LFO_SPEED;

/// One of the 5 FamiTracker-style envelope lanes every instrument has: volume,
/// arpeggio, pitch, hi-pitch, duty. Replaces the `tab: usize` (0..=4) index
/// that used to thread through the whole codebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Lane {
    Vol = 0,
    Arp = 1,
    Pitch = 2,
    HiPitch = 3,
    Duty = 4,
}

impl Lane {
    pub const COUNT: usize = 5;

    pub const ALL: [Self; Self::COUNT] =
        [Self::Vol, Self::Arp, Self::Pitch, Self::HiPitch, Self::Duty];

    /// The wire-format name used in `.rp2a03patch` field/error text
    /// (`docs/format.md`'s `sequences`/`active_indices` keys). Load-bearing
    /// for anything that touches the patch format — do not change these
    /// strings.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Vol => "vol",
            Self::Arp => "arp",
            Self::Pitch => "pitch",
            Self::HiPitch => "hipitch",
            Self::Duty => "duty",
        }
    }

    /// Whether this lane is offered at all for the given channel: the
    /// Triangle channel has no duty control, Noise has no pitch/hi-pitch
    /// (its period comes from the noise-period table, not a tuned note), and
    /// FDS's own pitch/hi-pitch pair collapses hi-pitch into the (finer-grain)
    /// pitch lane.
    #[must_use]
    pub fn available_for(self, mode: ChannelMode) -> bool {
        match self {
            Self::Vol | Self::Arp => true,
            Self::Pitch => mode != ChannelMode::Noise,
            Self::HiPitch => mode != ChannelMode::Noise && mode != ChannelMode::Fds,
            Self::Duty => mode != ChannelMode::Triangle,
        }
    }

    /// The editor label for this lane. Duty's label is chip-dependent since
    /// that lane doubles as the Noise/S5B mode control and the FDS wave index.
    #[must_use]
    pub fn label(self, mode: ChannelMode) -> &'static str {
        match self {
            Self::Vol => "Volume",
            Self::Arp => "Arpeggio",
            Self::Pitch => "Pitch",
            Self::HiPitch => "Hi-Pitch",
            Self::Duty => match mode {
                ChannelMode::S5B => "Noise / Mode",
                ChannelMode::Noise => "Mode",
                ChannelMode::Fds => "Wave Index",
                _ => "Duty / Noise",
            },
        }
    }

    /// The legal step-value range for this lane's sequence editor, given the
    /// current channel and (for the volume lane) its step resolution.
    #[must_use]
    pub fn value_range(
        self,
        channel_mode: ChannelMode,
        vol_mode: VolMode,
        vol_mode_5b: VolMode5B,
        wave_slot_count: usize,
    ) -> (i16, i16) {
        match self {
            Self::Vol => {
                if channel_mode == ChannelMode::Vrc6Saw && vol_mode == VolMode::Steps64 {
                    (0, 63)
                } else if channel_mode == ChannelMode::S5B && vol_mode_5b == VolMode5B::Steps32 {
                    (0, 31)
                } else if channel_mode == ChannelMode::Fds {
                    (0, 32)
                } else {
                    (0, 15)
                }
            }
            Self::Arp => (-96, 96),
            Self::Pitch | Self::HiPitch => (-128, 127),
            Self::Duty => match channel_mode {
                ChannelMode::Vrc6Pulse => (0, 7),
                ChannelMode::Vrc6Saw | ChannelMode::Noise => (0, 1),
                ChannelMode::S5B => (0, 31),
                ChannelMode::Fds => (
                    0,
                    wave_slot_count.saturating_sub(1).min(i16::MAX as usize) as i16,
                ),
                _ => (0, 3),
            },
        }
    }
}

impl serde::Serialize for Lane {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (*self as u8).serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Lane {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Matches a persisted session from before `Lane` existed, when
        // `selected_tab` was a plain `usize`. An out-of-range value (should
        // never happen from a well-formed session) falls back to `Vol`
        // instead of panicking.
        Ok(match u8::deserialize(deserializer)? {
            1 => Self::Arp,
            2 => Self::Pitch,
            3 => Self::HiPitch,
            4 => Self::Duty,
            _ => Self::Vol,
        })
    }
}

impl<T> std::ops::Index<Lane> for [T; Lane::COUNT] {
    type Output = T;

    fn index(&self, lane: Lane) -> &T {
        &self[lane as usize]
    }
}

impl<T> std::ops::IndexMut<Lane> for [T; Lane::COUNT] {
    fn index_mut(&mut self, lane: Lane) -> &mut T {
        &mut self[lane as usize]
    }
}

#[cfg(test)]
mod lane_tests {
    use super::*;

    #[test]
    fn all_traverses_every_lane_in_discriminant_order() {
        assert_eq!(
            Lane::ALL,
            [Lane::Vol, Lane::Arp, Lane::Pitch, Lane::HiPitch, Lane::Duty]
        );
        assert_eq!(Lane::ALL.len(), Lane::COUNT);
        for (i, lane) in Lane::ALL.iter().enumerate() {
            assert_eq!(*lane as usize, i);
        }
    }

    #[test]
    fn index_and_index_mut_round_trip_through_a_five_element_array() {
        let mut arr = [0i32; Lane::COUNT];
        for lane in Lane::ALL {
            arr[lane] = lane as i32 * 10;
        }
        assert_eq!(arr[Lane::Vol], 0);
        assert_eq!(arr[Lane::Duty], 40);
    }

    #[test]
    fn pitch_and_hipitch_are_unavailable_on_noise() {
        assert!(!Lane::Pitch.available_for(ChannelMode::Noise));
        assert!(!Lane::HiPitch.available_for(ChannelMode::Noise));
        assert!(Lane::Vol.available_for(ChannelMode::Noise));
        assert!(Lane::Arp.available_for(ChannelMode::Noise));
        assert!(Lane::Duty.available_for(ChannelMode::Noise));
    }

    #[test]
    fn hipitch_is_unavailable_on_fds_but_pitch_is_not() {
        assert!(!Lane::HiPitch.available_for(ChannelMode::Fds));
        assert!(Lane::Pitch.available_for(ChannelMode::Fds));
    }

    #[test]
    fn duty_is_unavailable_on_triangle_only() {
        assert!(!Lane::Duty.available_for(ChannelMode::Triangle));
        for mode in [
            ChannelMode::Pulse,
            ChannelMode::Noise,
            ChannelMode::Vrc6Pulse,
            ChannelMode::Vrc6Saw,
            ChannelMode::S5B,
            ChannelMode::Fds,
        ] {
            assert!(Lane::Duty.available_for(mode));
        }
    }

    #[test]
    fn vol_range_depends_on_channel_and_step_mode() {
        assert_eq!(
            Lane::Vol.value_range(ChannelMode::Pulse, VolMode::Steps16, VolMode5B::Steps16, 0),
            (0, 15)
        );
        assert_eq!(
            Lane::Vol.value_range(
                ChannelMode::Vrc6Saw,
                VolMode::Steps64,
                VolMode5B::Steps16,
                0
            ),
            (0, 63)
        );
        assert_eq!(
            Lane::Vol.value_range(ChannelMode::S5B, VolMode::Steps16, VolMode5B::Steps32, 0),
            (0, 31)
        );
        assert_eq!(
            Lane::Vol.value_range(ChannelMode::Fds, VolMode::Steps16, VolMode5B::Steps16, 0),
            (0, 32)
        );
    }

    #[test]
    fn duty_range_on_fds_tracks_wave_slot_count() {
        assert_eq!(
            Lane::Duty.value_range(ChannelMode::Fds, VolMode::Steps16, VolMode5B::Steps16, 5),
            (0, 4)
        );
        assert_eq!(
            Lane::Duty.value_range(ChannelMode::Fds, VolMode::Steps16, VolMode5B::Steps16, 0),
            (0, 0)
        );
    }

    #[test]
    fn arp_and_pitch_ranges_are_fixed() {
        assert_eq!(
            Lane::Arp.value_range(ChannelMode::Pulse, VolMode::Steps16, VolMode5B::Steps16, 0),
            (-96, 96)
        );
        assert_eq!(
            Lane::Pitch.value_range(ChannelMode::Pulse, VolMode::Steps16, VolMode5B::Steps16, 0),
            (-128, 127)
        );
        assert_eq!(
            Lane::HiPitch.value_range(ChannelMode::Pulse, VolMode::Steps16, VolMode5B::Steps16, 0),
            (-128, 127)
        );
    }

    #[test]
    fn wire_names_match_the_patch_format_spec() {
        assert_eq!(Lane::Vol.wire_name(), "vol");
        assert_eq!(Lane::Arp.wire_name(), "arp");
        assert_eq!(Lane::Pitch.wire_name(), "pitch");
        assert_eq!(Lane::HiPitch.wire_name(), "hipitch");
        assert_eq!(Lane::Duty.wire_name(), "duty");
    }
}

#[must_use]
pub fn midi_note_to_freq(note: u8) -> f32 {
    440.0 * 2.0f32.powf((f32::from(note) - 69.0) / 12.0)
}

#[must_use]
pub fn freq_to_period(freq: f32) -> u16 {
    if freq <= 0.0 {
        return 2047;
    }
    let t = (NTSC_CPU_CLOCK as f32 / (16.0 * freq)) - 0.5;
    t.round().clamp(0.0, 2047.0) as u16
}

#[must_use]
pub fn freq_to_triangle_period(freq: f32) -> u16 {
    if freq <= 0.0 {
        return 4095;
    }
    let t = (NTSC_CPU_CLOCK as f32 / (16.0 * freq)) - 0.5;
    t.round().clamp(0.0, 4095.0) as u16
}

pub fn freq_to_vrc6_saw_period(freq: f32) -> u16 {
    if freq <= 0.0 {
        return 4095;
    }
    let t = (NTSC_CPU_CLOCK as f32 / (14.0 * freq)) - 0.5;
    t.round().clamp(0.0, 4095.0) as u16
}

/// VRC6 Pulse's divider has no `*2+1` timer-doubling like the 2A03 pulse (its
/// reload is `frequency + 1`, clocked every CPU cycle, not every other), and a
/// real 12-bit frequency register (`hi_mask: 0x0F` — see `channel.rs`'s
/// `VRC6_PERIOD_REGISTER`), not the 2A03's 11-bit one. `freq_to_period` is
/// shaped for the 2A03 and under-ranges it below ~54.6 Hz.
pub fn freq_to_vrc6_pulse_period(freq: f32) -> u16 {
    if freq <= 0.0 {
        return 4095;
    }
    let t = (NTSC_CPU_CLOCK as f32 / (16.0 * freq)) - 0.5;
    t.round().clamp(0.0, 4095.0) as u16
}

pub fn freq_to_s5b_period(freq: f32) -> u16 {
    if freq <= 0.0 {
        return 4095;
    }
    let t = NTSC_CPU_CLOCK as f32 / (32.0 * freq);
    t.round().clamp(1.0, 4095.0) as u16
}

pub fn freq_to_fds_frequency(freq: f32) -> u16 {
    if freq <= 0.0 {
        return 0;
    }
    let t = freq * 4_194_304.0 / NTSC_CPU_CLOCK as f32;
    t.round().clamp(0.0, 4095.0) as u16
}

#[inline]
pub fn midi_note_to_noise_period(note: u8) -> u8 {
    let internal = (i16::from(note) - 11).clamp(1, 96) as u8;
    (internal & 0x0F) ^ 0x0F
}

#[cfg(test)]
mod vrc6_pulse_period_tests {
    use super::*;

    /// A note low enough that the 2A03-shaped `freq_to_period` (11-bit, ceiling
    /// 2047) would truncate it, but well inside VRC6 Pulse's real 12-bit range.
    #[test]
    fn reaches_past_the_2a03_elevenbit_ceiling() {
        let freq = 50.0; // below the ~54.6 Hz where the true period crosses 2047
        let period = freq_to_vrc6_pulse_period(freq);
        assert!(
            period > 2047,
            "period {period} must exceed the 2A03-shaped ceiling for a note this low"
        );
        assert!(
            period <= 4095,
            "must stay inside VRC6 Pulse's real 12-bit register"
        );
    }

    /// Same formula shape as `freq_to_vrc6_saw_period` (undoubled divider, `-0.5`
    /// rounding constant), swapping the saw's 14-step cycle for the pulse's 16.
    #[test]
    fn matches_the_undoubled_16step_divider_math() {
        let freq = 440.0;
        let expected = ((NTSC_CPU_CLOCK as f32 / (16.0 * freq)) - 0.5).round() as u16;
        assert_eq!(freq_to_vrc6_pulse_period(freq), expected);
    }

    #[test]
    fn clamps_at_zero_and_at_the_12bit_ceiling() {
        assert_eq!(freq_to_vrc6_pulse_period(0.0), 4095);
        assert_eq!(freq_to_vrc6_pulse_period(f32::INFINITY), 0);
    }
}

// `basedrop::Shared` has no `Debug` impl, so this struct can't derive it —
// nothing needs it (grepped: never `{:?}`-formatted, no struct embedding it
// derives `Debug`).
#[derive(Clone, Default)]
pub struct ActiveSequences {
    pub seq: [Sequence; Lane::COUNT],
    pub enabled: [bool; Lane::COUNT],

    /// `None` when there are no FDS wave slots yet — same meaning as the old
    /// `Vec::is_empty()`, just without allocating a `Shared` for nothing.
    pub fds_waves: Option<Shared<Vec<[u8; FDS_WAVE_LEN]>>>,

    pub fds_current_wave: usize,

    pub wavesynth: WaveSynthParams,

    pub fds_settings: FdsSettings,
}

impl ActiveSequences {
    /// A lane is actually playable: enabled and non-empty. Does not account
    /// for `Lane::available_for` (channel-mode gating) — callers that need
    /// that check it separately, since e.g. hi-pitch on FDS is unavailable
    /// even when enabled and non-empty.
    #[must_use]
    pub fn lane_active(&self, lane: Lane) -> bool {
        self.enabled[lane] && !self.seq[lane].values.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SequenceReload {
    reload: [bool; Lane::COUNT],
}

impl SequenceReload {
    pub const ALL: Self = Self {
        reload: [true; Lane::COUNT],
    };

    #[must_use]
    pub fn any(&self) -> bool {
        self.reload.iter().any(|&r| r)
    }
}

impl std::ops::Index<Lane> for SequenceReload {
    type Output = bool;

    fn index(&self, lane: Lane) -> &bool {
        &self.reload[lane]
    }
}

impl std::ops::IndexMut<Lane> for SequenceReload {
    fn index_mut(&mut self, lane: Lane) -> &mut bool {
        &mut self.reload[lane]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostAutomationSnapshot {
    pub vibrato_depth: u8,
    pub vibrato_speed: u8,
    pub vibrato_delay: u8,
    pub tremolo_depth: u8,
    pub tremolo_speed: u8,
    pub tremolo_delay: u8,
    pub delay_speed: u8,
    pub hardware_volume: u8,
    pub fine_pitch: i8,
    pub hi_pitch: i8,

    pub pitch_slide: i16,

    pub pitch_slide_range: u8,
    pub step_time_hz: u16,
    pub portamento_enabled: bool,
    pub portamento_speed: u8,
}

impl Default for HostAutomationSnapshot {
    fn default() -> Self {
        Self {
            vibrato_depth: 0,
            vibrato_speed: DEFAULT_LFO_SPEED,
            vibrato_delay: 0,
            tremolo_depth: 0,
            tremolo_speed: DEFAULT_LFO_SPEED,
            tremolo_delay: 0,
            delay_speed: 0,
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

pub type HostAutomationControls = HostAutomationSnapshot;
