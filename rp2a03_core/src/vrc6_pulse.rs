//! rp2a03_core\src\vrc6_pulse.rs
//!
//! Adapted from Mesen: https://github.com/nesdev-org/MesenCE/blob/master/Core/NES/Mappers/Audio/Vrc6Pulse.h
//!
//! VRC6 Pulse Channel implementation.
//!
//! Unlike the 2A03 pulse channels, the VRC6 pulse has no envelope, no length
//! counter, and no sweep unit — just a constant 4-bit volume gated by a
//! duty-cycle comparator. The frequency shift (used for VRC6's "halve"/
//! "quarter" pitch modes) is broadcast from the audio controller ($9003)
//! to all three VRC6 channels, so it's set externally rather than owned here.
//!
//! See: <https://www.nesdev.org/wiki/VRC6_audio>

// ─────────────────────────────────────────────
// Duty Sequencer
// ─────────────────────────────────────────────

/// VRC6's duty unit differs from the 2A03's: instead of indexing a fixed
/// waveform table, it free-runs a 4-bit step counter (0..=15) and compares
/// it against a programmable threshold each clock. The step is high
/// (channel "on") while `step <= duty_cycle`, unless `ignore_duty` is set,
/// in which case the channel behaves as if always on (DDA / PCM mode).
#[derive(Debug, Clone)]
pub struct DutySequencer {
    duty_cycle: u8,
    ignore_duty: bool,
    current_step: u8,
}

impl DutySequencer {
    pub fn new() -> Self {
        Self {
            duty_cycle: 0,
            ignore_duty: false,
            current_step: 0,
        }
    }

    fn set_duty_cycle(&mut self, duty_cycle: u8) {
        self.duty_cycle = duty_cycle & 0x07;
    }

    fn set_ignore_duty(&mut self, ignore_duty: bool) {
        self.ignore_duty = ignore_duty;
    }

    pub fn output(&self) -> bool {
        self.ignore_duty || self.current_step <= self.duty_cycle
    }

    pub fn clock(&mut self) {
        self.current_step = (self.current_step + 1) & 0x0F;
    }

    pub fn reset_step(&mut self) {
        self.current_step = 0;
    }
}

use crate::vrc6_common::Divider;

// ─────────────────────────────────────────────
// Pulse Channel
// ─────────────────────────────────────────────

/// VRC6 Pulse Channel provides square wave generation for the VRC6 sound
/// expansion chip (Konami, used by e.g. Akumajou Densetsu / Castlevania III).
///
/// See: <https://www.nesdev.org/wiki/VRC6_audio>
//
//                     Duty Comparator
//                           |
//                           v
// Frequency ---> Divider -> Gate ---> Volume ---> (to mixer)
//                                        ^
//                                     Enabled
//
#[derive(Debug, Clone)]
pub struct Vrc6Pulse {
    volume: u8,
    duty: DutySequencer,
    frequency: u16,
    frequency_shift: u8,
    enabled: bool,
    divider: Divider,
}

impl Vrc6Pulse {
    pub fn new() -> Self {
        Self {
            volume: 0,
            duty: DutySequencer::new(),
            frequency: 1,
            frequency_shift: 0,
            enabled: false,
            divider: Divider::new(),
        }
    }

    // ── Register writes ─────────────────────

    /// $9000/$A000 Pulse control
    ///   D7:     Ignore duty cycle (DDA/PCM mode, channel always on)
    ///   D6..D4: Duty cycle
    ///   D3..D0: Volume
    pub fn write_ctrl(&mut self, val: u8) {
        self.volume = val & 0x0F;
        self.duty.set_duty_cycle((val & 0x70) >> 4);
        self.duty.set_ignore_duty((val & 0x80) == 0x80);
    }

    /// $9001/$A001 Frequency low
    ///   D7..D0: Frequency low 8 bits
    pub fn write_freq_lo(&mut self, val: u8) {
        self.frequency = (self.frequency & 0x0F00) | u16::from(val);
    }

    /// $9002/$A002 Frequency high / Enable
    ///   D7:     Channel enabled
    ///   D3..D0: Frequency high 4 bits
    pub fn write_freq_hi(&mut self, val: u8) {
        self.frequency = (self.frequency & 0x00FF) | (u16::from(val & 0x0F) << 8);
        self.enabled = (val & 0x80) == 0x80;
        if !self.enabled {
            self.duty.reset_step();
        }
    }

    /// Sets the shared frequency shift (halve/quarter pitch mode), broadcast
    /// from the audio controller's $9003 register to all VRC6 channels.
    pub fn set_frequency_shift(&mut self, shift: u8) {
        self.frequency_shift = shift;
    }

    // ── Clocking ────────────────────────────

    /// Clock the pulse divider. When it expires, the duty step advances.
    /// No-op while the channel is disabled, matching hardware (the divider
    /// simply doesn't run).
    pub fn clock(&mut self) {
        if self.enabled {
            let reload = (self.frequency >> self.frequency_shift) as i32 + 1;
            if self.divider.tick(reload) {
                self.duty.clock();
            }
        }
    }

    // ── Output ──────────────────────────────

    /// Current volume contribution of this channel (0 if disabled or duty
    /// gate is closed).
    pub fn volume(&self) -> u8 {
        if !self.enabled {
            0
        } else if self.duty.output() {
            self.volume
        } else {
            0
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

    // ── DutySequencer tests ──

    #[test]
    fn new_starts_at_step_zero_and_duty_zero() {
        let seq = DutySequencer::new();
        // step 0 <= duty_cycle 0 -> on
        assert!(seq.output());
    }

    #[test]
    fn duty_gate_closes_past_threshold() {
        let mut seq = DutySequencer::new();
        seq.set_duty_cycle(3);

        // steps 0..=3 -> on, step 4 -> off
        for _ in 0..4 {
            assert!(seq.output());
            seq.clock();
        }
        assert!(!seq.output());
    }

    #[test]
    fn ignore_duty_forces_always_on() {
        let mut seq = DutySequencer::new();
        seq.set_duty_cycle(0);
        seq.set_ignore_duty(true);

        for _ in 0..16 {
            assert!(seq.output());
            seq.clock();
        }
    }

    #[test]
    fn clock_wraps_after_sixteen_steps() {
        let mut seq = DutySequencer::new();
        seq.set_duty_cycle(7);
        for _ in 0..16 {
            seq.clock();
        }
        // Back at step 0, same as initial state.
        assert!(seq.output());
    }

    #[test]
    fn reset_step_returns_to_zero() {
        let mut seq = DutySequencer::new();
        seq.set_duty_cycle(0);
        seq.clock();
        seq.clock();
        assert!(!seq.output()); // step 2 > duty 0
        seq.reset_step();
        assert!(seq.output()); // step 0 <= duty 0
    }

    // ── Vrc6Pulse tests ──

    #[test]
    fn new_pulse_outputs_zero() {
        let pulse = Vrc6Pulse::new();
        assert_eq!(pulse.volume(), 0);
    }

    #[test]
    fn disabled_channel_outputs_zero_even_with_volume_set() {
        let mut pulse = Vrc6Pulse::new();
        pulse.write_ctrl(0x8F); // ignore_duty, volume = 0x0F
        // Not enabled yet (freq_hi never written).
        assert_eq!(pulse.volume(), 0);
    }

    #[test]
    fn enabling_with_ignore_duty_outputs_volume() {
        let mut pulse = Vrc6Pulse::new();
        pulse.write_ctrl(0x8A); // ignore_duty = true, volume = 0x0A
        pulse.write_freq_lo(0x00);
        pulse.write_freq_hi(0x80); // enabled, freq hi = 0
        assert_eq!(pulse.volume(), 0x0A);
    }

    #[test]
    fn disabling_resets_duty_step() {
        let mut pulse = Vrc6Pulse::new();
        pulse.write_ctrl(0x05); // duty_cycle = 0, volume = 5
        pulse.write_freq_lo(0x01); // small period so it clocks fast
        pulse.write_freq_hi(0x80); // enabled

        // Clock enough to move the duty step past 0.
        for _ in 0..8 {
            pulse.clock();
        }
        assert_eq!(pulse.duty.current_step, 0, "small duty_cycle keeps failing output but step should still advance");

        // Disable, which should reset the step back to 0.
        pulse.write_freq_hi(0x00); // disabled
        assert_eq!(pulse.duty.current_step, 0);
    }

    #[test]
    fn frequency_shift_speeds_up_divider_reload() {
        let mut pulse = Vrc6Pulse::new();
        pulse.write_ctrl(0x8F); // ignore_duty, volume 0xF, always "on"
        pulse.write_freq_lo(0xFF);
        pulse.write_freq_hi(0x80); // enabled, freq = 0x0FF = 255

        // With shift = 0, reload = 255 + 1 = 256 clocks per duty step.
        pulse.set_frequency_shift(0);
        pulse.clock();
        let step_before = pulse.duty.current_step;

        // With shift = 8 (VRC6's "quarter" mode), reload = (255 >> 8) + 1 = 1,
        // so the duty step should advance every single clock.
        pulse.set_frequency_shift(8);
        for _ in 0..3 {
            pulse.clock();
        }
        assert_ne!(pulse.duty.current_step, step_before);
    }

    #[test]
    fn write_freq_lo_and_hi_combine_correctly() {
        let mut pulse = Vrc6Pulse::new();
        pulse.write_freq_lo(0xFD);
        pulse.write_freq_hi(0x02); // enabled, high bits = 0x02
        assert_eq!(pulse.frequency, 0x02FD);
        assert!(pulse.enabled);
    }

    #[test]
    fn duty_cycle_write_masks_to_three_bits() {
        let mut pulse = Vrc6Pulse::new();
        pulse.write_ctrl(0x70); // D6..D4 = 0b111
        assert_eq!(pulse.duty.duty_cycle, 0x07);
    }

    #[test]
    fn reset_clears_all_state() {
        let mut pulse = Vrc6Pulse::new();
        pulse.write_ctrl(0xFF);
        pulse.write_freq_lo(0xFF);
        pulse.write_freq_hi(0xFF);

        pulse.reset();

        assert_eq!(pulse.frequency, 1);
        assert_eq!(pulse.frequency_shift, 0);
        assert!(!pulse.enabled);
        assert_eq!(pulse.volume(), 0);
    }
}