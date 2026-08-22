//! `rp2a03_core\src\apu_noise.rs`
//!
//! Adapted from `TetaNES`: <https://github.com/lukexor/tetanes/blob/main/tetanes-core/src/apu/noise.rs>
//! Implementation based on `NESdev` APU Noise documentation.
//!
//! APU Noise Channel implementation.
//!
//! See: <https://www.nesdev.org/wiki/APU_Noise>

use crate::apu::{Envelope, LengthCounter, Timer};

/// Noise shift mode.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum ShiftMode {
    /// Zero (XOR bits 0 and 1 - 32767-bit long sequence)
    Zero,
    /// One (XOR bits 0 and 6 - 93-bit short metallic sequence)
    One,
}

/// APU Noise Channel provides pseudo-random noise generation.
///
/// See: <https://www.nesdev.org/wiki/APU_Noise>
//
//                 Timer --> Shift Register   Length Counter
//                                 |                |
//                                 v                v
// Envelope --------------------> Gate ----------> Gate ---> (to mixer)
//
#[derive(Debug, Clone)]
pub struct Noise {
    /// Index into `PERIOD_TABLE_NTSC` last written via `write_timer`.
    pub period_index: u8,
    /// Drives the LFSR shift at the selected period.
    pub timer: Timer,
    /// The 15-bit linear-feedback shift register itself.
    pub shift: u16,
    /// Which feedback tap (bit 1 or bit 6) is `XORed` into the shift register.
    pub shift_mode: ShiftMode,
    /// Silences the channel a hardware-defined number of frames after $400F.
    pub length: LengthCounter,
    /// Decaying/constant-volume generator shared with the other envelope channels.
    pub envelope: Envelope,
    /// Voice-allocation mute, independent of the hardware envelope/length state.
    pub force_silent: bool,
}

impl Default for Noise {
    fn default() -> Self {
        Self::new()
    }
}

impl Noise {
    /// `FamiStudio`'s bundled `NesSndEmu` starts the noise generator from this
    /// deterministic non-zero seed, which also happens to sit on one of the
    /// 93-length cycles the short mode's tap produces (see `write_timer`).
    /// The LFSR is not reset when a note writes $400F; only the length
    /// counter and envelope are retriggered.
    pub const INITIAL_SHIFT: u16 = 4141;

    /// NTSC noise period table, indexed 0..15 by `write_timer`'s low nibble.
    pub const PERIOD_TABLE_NTSC: [u16; 16] = [
        4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
    ];

    /// A silenced noise channel at period index 0, LFSR seeded to `INITIAL_SHIFT`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            period_index: 0,
            timer: Timer::new(Self::PERIOD_TABLE_NTSC[0] - 1),
            shift: Self::INITIAL_SHIFT,
            shift_mode: ShiftMode::Zero,
            length: LengthCounter::new(),
            envelope: Envelope::new(),
            force_silent: false,
        }
    }

    /// True when the LFSR's bit 0 is set (hardware mute) or `force_silent` is.
    #[inline]
    #[must_use]
    pub fn is_muted(&self) -> bool {
        (self.shift & 0x01) == 0x01 || self.force_silent
    }

    /// Whether the voice-allocation mute (`force_silent`) is currently set.
    #[must_use]
    pub fn silent(&self) -> bool {
        self.force_silent
    }

    /// Sets or clears the voice-allocation mute, independent of hardware state.
    pub fn set_silent(&mut self, silent: bool) {
        self.force_silent = silent;
    }

    /// The timer period (already `-1`-adjusted) for a `write_timer` low-nibble index.
    #[must_use]
    pub fn period(index: u8) -> u16 {
        Self::PERIOD_TABLE_NTSC[(index & 0x0F) as usize] - 1
    }

    /// $400C Noise control
    ///   D5:     Length counter halt / Envelope loop
    ///   D4:     Constant volume flag
    ///   D3..D0: Volume / Envelope period
    pub fn write_ctrl(&mut self, val: u8) {
        self.length.write_ctrl((val & 0x20) == 0x20);
        self.envelope.write_ctrl(val);
    }

    /// $400E Noise timer / mode
    ///   D7:     Mode flag (0 = 32767-bit sequence, 1 = 93-bit short sequence)
    ///   D3..D0: Noise period index (0..15)
    pub fn write_timer(&mut self, val: u8) {
        self.period_index = val & 0x0F;
        self.timer.period = Self::period(self.period_index);
        let new_mode = if (val & 0x80) == 0x80 {
            ShiftMode::One
        } else {
            ShiftMode::Zero
        };
        // The tap-6 feedback used by short mode does not have one 93-length
        // cycle — the 15-bit state space splits into 352 disjoint 93-length
        // cycles (plus one small 31-length one), verified by brute-forcing
        // every seed. Free-running long mode can land on any of them, so
        // re-entering short mode from long mode at an arbitrary point picks
        // an arbitrary cycle: a different "metallic" pitch every time the
        // mode toggles. Snapping to `INITIAL_SHIFT` on the long-to-short edge
        // pins every engagement to the same cycle, so the tone the mode is
        // named for is actually reproducible. Staying in short mode (no edge)
        // is left alone so it keeps free-running around that cycle, same as
        // long mode does around its one.
        if new_mode == ShiftMode::One && self.shift_mode == ShiftMode::Zero {
            self.shift = Self::INITIAL_SHIFT;
        }
        self.shift_mode = new_mode;
    }

    /// $400F Length counter load & envelope restart
    ///   D7..D3: Length counter load index
    pub fn write_length(&mut self, val: u8) {
        self.length.write(val >> 3);
        self.envelope.restart();
    }

    /// Reseed the LFSR to its deterministic startup state without changing
    /// the selected period or mode. Not called on note-on (real hardware
    /// and `FamiTracker` never reset the LFSR there); kept for callers that
    /// want an explicit, deterministic restart (e.g. tests, full APU reset).
    pub fn retrigger(&mut self) {
        self.shift = Self::INITIAL_SHIFT;
        self.timer.counter = 0;
    }

    /// Enable or disable length counter from $4015.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.length.set_enabled(enabled);
    }

    /// Returns the current envelope volume (0 if length counter is expired).
    #[must_use]
    pub fn volume(&self) -> u8 {
        if self.length.counter() > 0 {
            self.envelope.volume()
        } else {
            0
        }
    }

    /// Clock the noise timer and shift register.
    /// Also applies any pending length counter reload (must happen each cycle).
    pub fn clock(&mut self) {
        self.length.reload();
        if self.timer.tick() {
            let shift_by = if self.shift_mode == ShiftMode::One {
                6
            } else {
                1
            };
            let feedback = (self.shift & 0x01) ^ ((self.shift >> shift_by) & 0x01);
            self.shift >>= 1;
            self.shift |= feedback << 14;
        }
    }

    /// Final sample output of this noise channel (0.0 or envelope volume float).
    #[must_use]
    pub fn output(&self) -> f32 {
        if self.is_muted() {
            0.0
        } else {
            f32::from(self.volume())
        }
    }

    /// Reset all sub-components and LFSR state.
    pub fn reset(&mut self) {
        self.period_index = 0;
        self.timer = Timer::new(Self::PERIOD_TABLE_NTSC[0] - 1);
        self.shift = Self::INITIAL_SHIFT;
        self.shift_mode = ShiftMode::Zero;
        self.length.reset();
        self.envelope.reset();
        self.force_silent = false;
    }
}
