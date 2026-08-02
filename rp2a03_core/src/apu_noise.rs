//! rp2a03_core\src\apu_noise.rs
//!
//! Adapted from TetaNES: https://github.com/lukexor/tetanes/blob/main/tetanes-core/src/apu/noise.rs
//! Implementation based on NESdev APU Noise documentation.
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
    pub period_index: u8,
    pub timer: Timer,
    pub shift: u16,
    pub shift_mode: ShiftMode,
    pub length: LengthCounter,
    pub envelope: Envelope,
    pub force_silent: bool,
}

impl Default for Noise {
    fn default() -> Self {
        Self::new()
    }
}

impl Noise {
    /// FamiStudio's bundled NesSndEmu starts the noise generator from this
    /// deterministic non-zero seed. The LFSR is not reset when a note writes
    /// $400F; only the length counter and envelope are retriggered.
    pub const INITIAL_SHIFT: u16 = 4141;

    pub const PERIOD_TABLE_NTSC: [u16; 16] = [
        4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
    ];
    pub const PERIOD_TABLE_PAL: [u16; 16] = [
        4, 8, 14, 30, 60, 88, 118, 148, 188, 236, 354, 472, 708, 944, 1890, 3778,
    ];

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

    #[inline]
    pub fn is_muted(&self) -> bool {
        (self.shift & 0x01) == 0x01 || self.force_silent
    }

    pub fn silent(&self) -> bool {
        self.force_silent
    }

    pub fn set_silent(&mut self, silent: bool) {
        self.force_silent = silent;
    }

    pub fn period(index: u8) -> u16 {
        Self::PERIOD_TABLE_NTSC[(index & 0x0F) as usize] - 1
    }

    /// Clock on quarter frame (240Hz): envelope only.
    pub fn clock_quarter_frame(&mut self) {
        self.envelope.clock();
    }

    /// Clock on half frame (120Hz): envelope + length counter.
    pub fn clock_half_frame(&mut self) {
        self.clock_quarter_frame();
        self.length.clock();
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
        self.shift_mode = if (val & 0x80) == 0x80 {
            ShiftMode::One
        } else {
            ShiftMode::Zero
        };
    }

    /// $400F Length counter load & envelope restart
    ///   D7..D3: Length counter load index
    pub fn write_length(&mut self, val: u8) {
        self.length.write(val >> 3);
        self.envelope.restart();
    }

    /// Enable or disable length counter from $4015.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.length.set_enabled(enabled);
    }

    /// Returns the current envelope volume (0 if length counter is expired).
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

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_noise_initial_state() {
        let noise = Noise::new();
        assert_eq!(noise.shift, Noise::INITIAL_SHIFT);
        assert_eq!(noise.shift_mode, ShiftMode::Zero);
        assert!(!noise.silent());
        assert_eq!(noise.period_index, 0);
    }

    #[test]
    fn write_ctrl_sets_envelope_and_length_halt() {
        let mut noise = Noise::new();
        noise.set_enabled(true);
        // D5 = halt, D4 = constant volume, D3..D0 = volume (10)
        noise.write_ctrl(0x3A);
        assert_eq!(noise.envelope.volume(), 10);
    }

    #[test]
    fn write_timer_sets_mode_and_period() {
        let mut noise = Noise::new();
        // Mode 1 (bit 7 set), index 5
        noise.write_timer(0x85);
        assert_eq!(noise.shift_mode, ShiftMode::One);
        assert_eq!(noise.period_index, 5);
        assert_eq!(noise.timer.period, Noise::PERIOD_TABLE_NTSC[5] - 1);
    }

    #[test]
    fn write_length_loads_counter_and_restarts_envelope() {
        let mut noise = Noise::new();
        noise.set_enabled(true);
        noise.write_ctrl(0x05); // volume = 5, not constant volume
        noise.write_length(0x08); // load index 1 (length = 254)
        noise.length.reload();

        assert_eq!(noise.length.counter(), 254);

        // Clock quarter frame so envelope processes restart -> sets counter to 15
        noise.clock_quarter_frame();
        assert_eq!(noise.volume(), 15);
    }

    #[test]
    fn note_trigger_does_not_reset_lfsr_state() {
        let mut noise = Noise::new();
        noise.set_enabled(true);
        noise.shift = 0x2345;
        noise.write_length(0x08);

        assert_eq!(noise.shift, 0x2345);
    }

    #[test]
    fn muting_conditions() {
        let mut noise = Noise::new();
        noise.set_enabled(true);
        noise.write_ctrl(0x1A); // constant volume 10
        noise.write_length(0x08); // active length counter
        noise.length.reload();

        // shift = 1 (bit 0 is 1) -> muted
        assert!(noise.is_muted());
        assert_eq!(noise.output(), 0.0);

        // Force bit 0 to 0 (unmuted)
        noise.shift = 2; // binary 10 -> bit 0 is 0
        assert!(!noise.is_muted());
        assert_eq!(noise.output(), 10.0);

        // Force silent
        noise.set_silent(true);
        assert!(noise.is_muted());
        assert_eq!(noise.output(), 0.0);
    }

    #[test]
    fn lfsr_shift_mode_zero_feedback() {
        let mut noise = Noise::new();
        noise.timer.period = 0; // clock on every tick
        noise.shift = 0b000_0000_0000_0011; // bit 0 = 1, bit 1 = 1 -> XOR = 0
        noise.clock();
        // Shift right by 1, bit 14 gets feedback 0
        assert_eq!(noise.shift, 0b000_0000_0000_0001);

        noise.shift = 0b000_0000_0000_0001; // bit 0 = 1, bit 1 = 0 -> XOR = 1
        noise.clock();
        // Shift right by 1, bit 14 gets feedback 1
        assert_eq!(noise.shift, 0x4000);
    }

    #[test]
    fn lfsr_shift_mode_one_feedback() {
        let mut noise = Noise::new();
        noise.shift_mode = ShiftMode::One;
        noise.timer.period = 0;

        // Mode 1: XOR bit 0 and bit 6
        noise.shift = (1 << 6) | 1; // bit 6 = 1, bit 0 = 1 -> XOR = 0
        noise.clock();
        assert_eq!(noise.shift & 0x4000, 0);

        noise.shift = 1; // bit 6 = 0, bit 0 = 1 -> XOR = 1
        noise.clock();
        assert_eq!(noise.shift & 0x4000, 0x4000);
    }

    #[test]
    fn reset_clears_noise_state() {
        let mut noise = Noise::new();
        noise.set_enabled(true);
        noise.write_ctrl(0xFF);
        noise.write_timer(0xFF);
        noise.write_length(0xFF);
        noise.set_silent(true);

        noise.reset();

        assert_eq!(noise.shift, Noise::INITIAL_SHIFT);
        assert_eq!(noise.shift_mode, ShiftMode::Zero);
        assert_eq!(noise.period_index, 0);
        assert!(!noise.silent());
        assert_eq!(noise.output(), 0.0);
    }
}
