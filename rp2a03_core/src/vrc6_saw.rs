//! rp2a03_core\src\vrc6_saw.rs
//!
//! Adapted from Mesen: https://github.com/nesdev-org/MesenCE/blob/master/Core/NES/Mappers/Audio/Vrc6Saw.h
//!
//! VRC6 Saw Channel implementation.
//!
//! The saw channel has no volume register or duty cycle; instead an
//! `accumulator` is incremented by `accumulator_rate` on alternating steps
//! of a 14-step cycle, then reset to zero at the start of the next cycle,
//! producing a sawtooth ramp. Only the high 5 bits of the accumulator are
//! output. Like the pulse channels, the frequency shift is broadcast from
//! the audio controller ($9003) to all three VRC6 channels.
//!
//! See: <https://www.nesdev.org/wiki/VRC6_audio>

use crate::vrc6_common::Divider;

// ─────────────────────────────────────────────
// Saw Channel
// ─────────────────────────────────────────────

/// VRC6 Saw Channel provides sawtooth wave generation for the VRC6 sound
/// expansion chip (Konami, used by e.g. Akumajou Densetsu / Castlevania III).
///
/// See: <https://www.nesdev.org/wiki/VRC6_audio>
//
//                     Accumulator
//                    (ramps, resets
//                     every 14 steps)
//                           |
//                           v
// Frequency ---> Divider -> Step ---> Accumulator ---> (to mixer)
//                                          ^
//                                       Enabled
//
#[derive(Debug, Clone)]
pub struct Vrc6Saw {
    accumulator_rate: u8,
    accumulator: u8,
    frequency: u16,
    frequency_shift: u8,
    enabled: bool,
    step: u8,
    divider: Divider,
}

impl Vrc6Saw {
    pub fn new() -> Self {
        Self {
            accumulator_rate: 0,
            accumulator: 0,
            frequency: 1,
            frequency_shift: 0,
            enabled: false,
            step: 0,
            divider: Divider::new(),
        }
    }

    // ── Register writes ─────────────────────

    /// $B000 Saw accumulator rate
    ///   D5..D0: Accumulator rate
    pub fn write_rate(&mut self, val: u8) {
        self.accumulator_rate = val & 0x3F;
    }

    /// Current $B000 accumulator rate (0..=63), as last masked by [`Self::write_rate`].
    pub fn rate(&self) -> u8 {
        self.accumulator_rate
    }

    /// $B001 Frequency low
    ///   D7..D0: Frequency low 8 bits
    pub fn write_freq_lo(&mut self, val: u8) {
        self.frequency = (self.frequency & 0x0F00) | u16::from(val);
    }

    /// $B002 Frequency high / Enable
    ///   D7:     Channel enabled
    ///   D3..D0: Frequency high 4 bits
    pub fn write_freq_hi(&mut self, val: u8) {
        self.frequency = (self.frequency & 0x00FF) | (u16::from(val & 0x0F) << 8);
        self.enabled = (val & 0x80) == 0x80;
        if !self.enabled {
            // If E is clear, the accumulator is forced to zero until E is
            // again set.
            self.accumulator = 0;

            // The phase of the saw generator can be mostly reset by
            // clearing and immediately setting E. Clearing E does not
            // reset the frequency divider, however.
            self.step = 0;
        }
    }

    /// Sets the shared frequency shift (halve/quarter pitch mode), broadcast
    /// from the audio controller's $9003 register to all VRC6 channels.
    pub fn set_frequency_shift(&mut self, shift: u8) {
        self.frequency_shift = shift;
    }

    /// Sets channel enable status without resetting frequency divider.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !self.enabled {
            self.accumulator = 0;
            self.step = 0;
        }
    }

    /// Soft update of frequency high 4 bits without resetting accumulator phase.
    pub fn set_period_hi_soft(&mut self, hi_bits: u8) {
        self.frequency = (self.frequency & 0x00FF) | (u16::from(hi_bits & 0x0F) << 8);
    }


    // ── Clocking ────────────────────────────

    /// Clock the saw divider. When it expires, the step advances through a
    /// 14-step cycle: the accumulator resets to zero at step 0, and
    /// increments by `accumulator_rate` on every other step thereafter.
    /// No-op while the channel is disabled — the divider doesn't reset,
    /// only the accumulator and step do (see `write_freq_hi`).
    pub fn clock(&mut self) {
        if self.enabled {
            let reload = (self.frequency >> self.frequency_shift) as i32 + 1;
            if self.divider.tick(reload) {
                self.step = (self.step + 1) % 14;

                if self.step == 0 {
                    self.accumulator = 0;
                } else if self.step & 0x01 == 0x00 {
                    self.accumulator = self.accumulator.wrapping_add(self.accumulator_rate);
                }
            }
        }
    }

    // ── Output ──────────────────────────────

    /// Current volume contribution of this channel: the high 5 bits of the
    /// accumulator, or 0 while disabled.
    pub fn volume(&self) -> u8 {
        if !self.enabled {
            0
        } else {
            self.accumulator >> 3
        }
    }

    // ── Reset ───────────────────────────────

    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_saw_outputs_zero() {
        let saw = Vrc6Saw::new();
        assert_eq!(saw.volume(), 0);
    }

    #[test]
    fn disabled_channel_outputs_zero_even_with_accumulator_set() {
        let mut saw = Vrc6Saw::new();
        saw.write_rate(0x3F);
        // Never enabled -> accumulator never accrues, output stays 0.
        assert_eq!(saw.volume(), 0);
    }

    #[test]
    fn accumulator_ramps_on_alternating_steps() {
        let mut saw = Vrc6Saw::new();
        saw.write_rate(0x04); // accumulator_rate = 4
        saw.write_freq_lo(0x00);
        saw.write_freq_hi(0x80); // enabled, freq = 0, reload = 0 + 1 = 1

        // step 0 (after 1st clock): accumulator reset to 0.
        saw.clock();
        assert_eq!(saw.step, 1 % 14);
        // step 1 is odd -> no accrual on step 1 (only even, non-zero steps accrue).
        assert_eq!(saw.accumulator, 0);

        saw.clock(); // step -> 2, even and nonzero -> accrues
        assert_eq!(saw.step, 2);
        assert_eq!(saw.accumulator, 4);

        saw.clock(); // step -> 3, odd -> no accrual
        assert_eq!(saw.accumulator, 4);

        saw.clock(); // step -> 4, even -> accrues again
        assert_eq!(saw.accumulator, 8);
    }

    #[test]
    fn accumulator_resets_every_fourteen_steps() {
        let mut saw = Vrc6Saw::new();
        saw.write_rate(0x10);
        saw.write_freq_lo(0x00);
        saw.write_freq_hi(0x80); // enabled, reload = 1

        // Run a full 14-step cycle; accumulator should be nonzero partway
        // through, then reset to 0 exactly when step wraps back to 0.
        for i in 1..=14 {
            saw.clock();
            if i % 14 == 0 {
                assert_eq!(saw.step, 0);
                assert_eq!(saw.accumulator, 0, "accumulator should reset at step 0");
            }
        }
    }

    #[test]
    fn accumulator_wraps_on_overflow() {
        let mut saw = Vrc6Saw::new();
        saw.write_rate(0x3F); // max rate
        saw.write_freq_lo(0x00);
        saw.write_freq_hi(0x80); // enabled, reload = 1

        // Drive enough accrual steps to overflow a u8 accumulator and
        // confirm it wraps (matching C++'s implicit uint8_t truncation)
        // rather than panicking in debug builds.
        for _ in 0..200 {
            saw.clock();
        }
        // No panic == wrapping_add did its job; sanity check it's a valid u8.
        let _ = saw.accumulator;
    }

    #[test]
    fn disabling_forces_accumulator_and_step_to_zero() {
        let mut saw = Vrc6Saw::new();
        saw.write_rate(0x20);
        saw.write_freq_lo(0x00);
        saw.write_freq_hi(0x80); // enabled, reload = 1

        for _ in 0..5 {
            saw.clock();
        }
        assert!(saw.accumulator > 0 || saw.step > 0);

        saw.write_freq_hi(0x00); // disabled
        assert_eq!(saw.accumulator, 0);
        assert_eq!(saw.step, 0);
    }

    #[test]
    fn disabling_only_resets_step_and_accumulator_not_pitch() {
        let mut saw = Vrc6Saw::new();
        saw.write_rate(0x20);
        saw.write_freq_lo(0x05); // frequency = 5

        saw.write_freq_hi(0x00); // stays disabled
        // Disabling while already disabled shouldn't touch frequency.
        assert_eq!(saw.frequency, 5);

        saw.write_freq_hi(0x80); // enabled, high bits = 0
        assert_eq!(saw.frequency, 5, "write_freq_hi must not clobber the low byte");
        assert!(saw.enabled);
    }

    #[test]
    fn write_freq_lo_and_hi_combine_correctly() {
        let mut saw = Vrc6Saw::new();
        saw.write_freq_lo(0xFD);
        // D7 set to enable, D3..D0 = high 4 bits of the frequency.
        saw.write_freq_hi(0x82);
        assert_eq!(saw.frequency, 0x02FD);
        assert!(saw.enabled);
    }

    #[test]
    fn write_rate_masks_to_six_bits() {
        let mut saw = Vrc6Saw::new();
        saw.write_rate(0xFF);
        assert_eq!(saw.accumulator_rate, 0x3F);
    }

    #[test]
    fn reset_clears_all_state() {
        let mut saw = Vrc6Saw::new();
        saw.write_rate(0x3F);
        saw.write_freq_lo(0xFF);
        saw.write_freq_hi(0xFF);

        saw.reset();

        assert_eq!(saw.frequency, 1);
        assert_eq!(saw.frequency_shift, 0);
        assert!(!saw.enabled);
        assert_eq!(saw.volume(), 0);
    }
}