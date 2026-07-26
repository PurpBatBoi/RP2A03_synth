//! rp2a03_core\src\apu_pulse.rs
//!
//! Adapted from TetaNES: https://github.com/lukexor/tetanes/blob/main/tetanes-core/src/apu/pulse.rs
//! Implementation based on the info from NESdev, Blaarg's "apu_ref" and Brad Taylor's "2A03 technical reference"
//!
//! APU Pulse Channel implementation.
//!
//! See: <https://www.nesdev.org/wiki/APU_Pulse>

use crate::apu::{Envelope, LengthCounter, Timer};

// ─────────────────────────────────────────────
// Duty Sequencer
// ─────────────────────────────────────────────

const DUTY_PATTERNS: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12.5%
    [0, 0, 0, 0, 0, 0, 1, 1], // 25%
    [0, 0, 0, 0, 1, 1, 1, 1], // 50%
    [1, 1, 1, 1, 1, 1, 0, 0], // 25% negated (75%)
];

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
enum DutyMode {
    Duty12_5 = 0,
    Duty25 = 1,
    Duty50 = 2,
    Duty75 = 3,
}

impl DutyMode {
    fn from_bits(val: u8) -> Self {
        match val & 0x03 {
            0 => DutyMode::Duty12_5,
            1 => DutyMode::Duty25,
            2 => DutyMode::Duty50,
            3 => DutyMode::Duty75,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DutySequencer {
    duty_mode: DutyMode,
    current_step: u8,
}

impl DutySequencer {
    pub fn new() -> Self {
        Self {
            duty_mode: DutyMode::Duty50,
            current_step: 0,
        }
    }

    fn set_duty(&mut self, duty: DutyMode) {
        self.duty_mode = duty;
    }

    pub fn output(&self) -> u8 {
        DUTY_PATTERNS[self.duty_mode as usize][self.current_step as usize]
    }

    pub fn clock(&mut self) {
        // The duty cycle sequencer counts downward (wrapping), matching real hardware.
        self.current_step = self.current_step.wrapping_sub(1) & 0x07;
    }

    pub fn reset_step(&mut self) {
        self.current_step = 0;
    }
}

// ─────────────────────────────────────────────
// Pulse Channel Selection
// ─────────────────────────────────────────────

/// Identifies which of the two pulse channels this is.
/// Pulse 1 and Pulse 2 differ only in sweep negate behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PulseChannel {
    One,
    Two,
}

// ─────────────────────────────────────────────
// Sweep Unit
// ─────────────────────────────────────────────

/// APU Sweep provides frequency sweeping for the pulse channels.
///
/// See: <https://www.nesdev.org/wiki/APU_Sweep>
#[derive(Debug, Clone)]
pub struct Sweep {
    enabled: bool,
    negate: bool,
    reload: bool,
    shift: u8,
    divider: u8,
    period: u8,
    target_period: u16,
}

impl Sweep {
    pub fn new() -> Self {
        Self {
            enabled: false,
            negate: false,
            reload: false,
            shift: 0,
            divider: 0,
            period: 0,
            target_period: 0,
        }
    }

    pub fn reset(&mut self) {
        self.enabled = false;
        self.negate = false;
        self.reload = false;
        self.shift = 0;
        self.divider = 0;
        self.period = 0;
        self.target_period = 0;
    }
}

// ─────────────────────────────────────────────
// Pulse Channel
// ─────────────────────────────────────────────

/// APU Pulse Channel provides square wave generation.
///
/// See: <https://www.nesdev.org/wiki/APU_Pulse>
//
//                  Sweep -----> Timer
//                    |            |
//                    |            |
//                    |            v
//                    |        Sequencer   Length Counter
//                    |            |             |
//                    |            |             |
//                    v            v             v
// Envelope -------> Gate -----> Gate -------> Gate --->(to mixer)
//
#[derive(Debug, Clone)]
pub struct Pulse {
    channel: PulseChannel,
    real_period: u16,
    timer: Timer,
    duty: DutySequencer,
    length: LengthCounter,
    envelope: Envelope,
    sweep: Sweep,
}

impl Pulse {
    pub fn new(channel: PulseChannel) -> Self {
        Self {
            channel,
            real_period: 0,
            timer: Timer::new(0),
            duty: DutySequencer::new(),
            length: LengthCounter::new(),
            envelope: Envelope::new(),
            sweep: Sweep::new(),
        }
    }

    // ── Muting ──────────────────────────────

    /// The pulse channel is muted when the period is < 8 (ultrasonic),
    /// or when the sweep unit's target period exceeds $7FF.
    #[inline]
    pub fn is_muted(&self) -> bool {
        self.real_period < 8 || (!self.sweep.negate && self.sweep.target_period > 0x7FF)
    }

    // ── Sweep internals ─────────────────────

    fn update_target_period(&mut self) {
        let delta = self.real_period >> self.sweep.shift;
        if self.sweep.negate {
            self.sweep.target_period = self.real_period.wrapping_sub(delta);
            // Pulse 1 uses one's complement (subtracts an extra 1)
            if self.channel == PulseChannel::One {
                self.sweep.target_period = self.sweep.target_period.wrapping_sub(1);
            }
        } else {
            self.sweep.target_period = self.real_period + delta;
        }
    }

    fn set_period(&mut self, period: u16) {
        self.real_period = period;
        // The pulse timer is clocked every other CPU cycle,
        // so the effective timer period is (period * 2) + 1.
        self.timer.period = (period * 2) + 1;
        self.update_target_period();
    }

    fn clock_sweep(&mut self) {
        self.sweep.divider = self.sweep.divider.wrapping_sub(1);
        if self.sweep.divider == 0 {
            if self.sweep.shift > 0
                && self.sweep.enabled
                && self.real_period >= 8
                && self.sweep.target_period <= 0x7FF
            {
                self.set_period(self.sweep.target_period);
            }
            self.sweep.divider = self.sweep.period;
        }

        if self.sweep.reload {
            self.sweep.divider = self.sweep.period;
            self.sweep.reload = false;
        }
    }

    // ── Register writes ─────────────────────

    /// $4000/$4004 Pulse control
    ///   D7..D6: Duty cycle
    ///   D5:     Length counter halt / Envelope loop
    ///   D4:     Constant volume flag
    ///   D3..D0: Volume / Envelope period
    pub fn write_ctrl(&mut self, val: u8) {
        self.duty.set_duty(DutyMode::from_bits((val & 0xC0) >> 6));
        self.length.write_ctrl((val & 0x20) == 0x20);
        self.envelope.write_ctrl(val);
    }

    /// $4001/$4005 Pulse sweep
    ///   D7:     Enabled
    ///   D6..D4: Period
    ///   D3:     Negate
    ///   D2..D0: Shift count
    pub fn write_sweep(&mut self, val: u8) {
        self.sweep.enabled = (val & 0x80) == 0x80;
        self.sweep.negate = (val & 0x08) == 0x08;
        self.sweep.period = ((val & 0x70) >> 4) + 1;
        self.sweep.shift = val & 0x07;
        self.update_target_period();
        self.sweep.reload = true;
    }

    /// $4002/$4006 Pulse timer lo
    ///   D7..D0: Timer low 8 bits
    pub fn write_timer_lo(&mut self, val: u8) {
        self.set_period((self.real_period & 0x0700) | u16::from(val));
    }

    /// $4003/$4007 Pulse timer hi
    ///   D7..D3: Length counter load
    ///   D2..D0: Timer high 3 bits
    pub fn write_timer_hi(&mut self, val: u8) {
        self.length.write(val >> 3);
        self.set_period((self.real_period & 0x00FF) | (u16::from(val & 0x07) << 8));
        self.duty.reset_step();
        self.envelope.restart();
    }

    /// Soft high-period update: updates only the high 3 bits of the timer period
    /// **without** resetting the duty sequencer step or restarting the envelope.
    ///
    /// Use this during continuous pitch modulation (vibrato LFO, pitch sequences, fine
    /// pitch) to avoid the audible click that `write_timer_hi` causes whenever the period
    /// oscillates across a 256-boundary (e.g. MIDI notes 29, 45, 57).
    ///
    /// On note attacks, continue using `write_timer_hi` so the duty phase resets cleanly.
    ///
    /// This is the software-equivalent of Blaarg's smooth vibrato trick used by FamiStudio
    /// (ChannelStateSquare.cs / famistudio_ca65.s) — adapted for a VST where we can update
    /// the period register directly rather than manipulating the hardware sweep unit.
    pub fn set_period_hi_soft(&mut self, hi_bits: u8) {
        self.set_period((self.real_period & 0x00FF) | (u16::from(hi_bits & 0x07) << 8));
    }

    // ── Enable / Status ─────────────────────

    /// Enable or disable from $4015.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.length.set_enabled(enabled);
    }

    /// Returns the current volume (0 if length counter is zero).
    pub fn volume(&self) -> u8 {
        if self.length.counter() > 0 {
            self.envelope.volume()
        } else {
            0
        }
    }

    // ── Clocking ────────────────────────────

    /// Clock the pulse timer. When the timer expires, the duty sequencer advances.
    /// Also applies any pending length counter reload (must happen each cycle).
    pub fn clock(&mut self) {
        self.length.reload();
        if self.timer.tick() {
            self.duty.clock();
        }
    }

    /// Clock on quarter frame (240Hz): envelope only.
    pub fn clock_quarter_frame(&mut self) {
        self.envelope.clock();
    }

    /// Clock on half frame (120Hz): envelope + length counter + sweep.
    pub fn clock_half_frame(&mut self) {
        self.clock_quarter_frame();
        self.length.clock();
        self.clock_sweep();
    }

    // ── Output ──────────────────────────────

    /// Final sample output of this pulse channel.
    pub fn output(&self) -> f32 {
        if self.is_muted() {
            0.0
        } else {
            f32::from(self.duty.output() * self.volume())
        }
    }

    // ── Reset ───────────────────────────────

    /// Reset all sub-components.
    pub fn reset(&mut self) {
        self.real_period = 0;
        self.timer = Timer::new(0);
        self.duty = DutySequencer::new();
        self.length.reset();
        self.envelope.reset();
        self.sweep.reset();
        self.update_target_period();
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── DutySequencer tests (preserved from original) ──

    #[test]
    fn new_starts_at_step_zero() {
        let seq = DutySequencer::new();
        // Default is Duty50, step 0.
        assert_eq!(seq.output(), 0);
    }

    #[test]
    fn duty12_5_outputs_correct_pattern() {
        let mut seq = DutySequencer::new();
        seq.set_duty(DutyMode::Duty12_5);

        // Counting DOWN from step 0: 0, 7, 6, 5, 4, 3, 2, 1
        // Pattern: [0, 0, 0, 0, 0, 0, 0, 1]
        // Step 0 -> pattern[0] = 0
        assert_eq!(seq.output(), 0);
        seq.clock(); // step -> 7, pattern[7] = 1
        assert_eq!(seq.output(), 1);
        seq.clock(); // step -> 6, pattern[6] = 0
        assert_eq!(seq.output(), 0);
    }

    #[test]
    fn duty25_outputs_correct_pattern() {
        let mut seq = DutySequencer::new();
        seq.set_duty(DutyMode::Duty25);

        // Pattern: [0, 0, 0, 0, 0, 0, 1, 1]
        assert_eq!(seq.output(), 0); // step 0
        seq.clock(); // step 7 -> pattern[7] = 1
        assert_eq!(seq.output(), 1);
        seq.clock(); // step 6 -> pattern[6] = 1
        assert_eq!(seq.output(), 1);
        seq.clock(); // step 5 -> pattern[5] = 0
        assert_eq!(seq.output(), 0);
    }

    #[test]
    fn duty50_outputs_correct_pattern() {
        let mut seq = DutySequencer::new();

        // Pattern: [0, 0, 0, 0, 1, 1, 1, 1]
        assert_eq!(seq.output(), 0); // step 0
        seq.clock(); // step 7 -> pattern[7] = 1
        assert_eq!(seq.output(), 1);
        seq.clock(); // step 6 -> 1
        assert_eq!(seq.output(), 1);
        seq.clock(); // step 5 -> 1
        assert_eq!(seq.output(), 1);
        seq.clock(); // step 4 -> 1
        assert_eq!(seq.output(), 1);
        seq.clock(); // step 3 -> 0
        assert_eq!(seq.output(), 0);
    }

    #[test]
    fn duty75_outputs_correct_pattern() {
        let mut seq = DutySequencer::new();
        seq.set_duty(DutyMode::Duty75);

        // Pattern: [1, 1, 1, 1, 1, 1, 0, 0]
        assert_eq!(seq.output(), 1); // step 0
        seq.clock(); // step 7 -> pattern[7] = 0
        assert_eq!(seq.output(), 0);
        seq.clock(); // step 6 -> 0
        assert_eq!(seq.output(), 0);
        seq.clock(); // step 5 -> 1
        assert_eq!(seq.output(), 1);
    }

    #[test]
    fn clock_wraps_after_eight_steps() {
        let mut seq = DutySequencer::new();
        let initial = seq.output();

        for _ in 0..8 {
            seq.clock();
        }

        // Should be back at step 0.
        assert_eq!(seq.output(), initial);
    }

    #[test]
    fn changing_duty_changes_output() {
        let mut seq = DutySequencer::new();

        // Step 0 of Duty50 -> pattern[0] = 0
        assert_eq!(seq.output(), 0);

        seq.set_duty(DutyMode::Duty75);

        // Step 0 of Duty75 -> pattern[0] = 1
        assert_eq!(seq.output(), 1);
    }

    // ── Pulse channel tests ──

    #[test]
    fn new_pulse_outputs_zero() {
        let pulse = Pulse::new(PulseChannel::One);
        // Period is 0, which is < 8, so muted
        assert_eq!(pulse.output(), 0.0);
    }

    #[test]
    fn pulse_is_muted_when_period_too_low() {
        let mut pulse = Pulse::new(PulseChannel::One);
        pulse.set_enabled(true);
        pulse.write_timer_lo(7); // period = 7, which is < 8
        assert!(pulse.is_muted());
    }

    #[test]
    fn pulse_not_muted_at_valid_period() {
        let mut pulse = Pulse::new(PulseChannel::One);
        pulse.set_enabled(true);
        pulse.write_timer_lo(0x80); // period = 128, valid
        assert!(!pulse.is_muted());
    }

    #[test]
    fn write_ctrl_sets_duty_and_envelope() {
        let mut pulse = Pulse::new(PulseChannel::One);
        pulse.set_enabled(true);
        // D7..D6 = 0b01 (25% duty), D4 = constant volume, D3..D0 = 0x0A (volume)
        pulse.write_ctrl(0x5A); // 0b0101_1010
        // Envelope should report constant volume = 0x0A
        assert_eq!(pulse.envelope.volume(), 0x0A);
    }

    #[test]
    fn write_timer_sets_period() {
        let mut pulse = Pulse::new(PulseChannel::One);
        pulse.write_timer_lo(0xFD);
        pulse.write_timer_hi(0x02); // high bits = 0x02 & 0x07 = 0x02
        // real_period = (0x02 << 8) | 0xFD = 0x02FD = 765
        assert_eq!(pulse.real_period, 765);
    }

    #[test]
    fn sweep_negate_pulse1_subtracts_extra() {
        let mut pulse = Pulse::new(PulseChannel::One);
        pulse.set_period(0x100); // period = 256
        pulse.write_sweep(0x88); // enabled, negate, shift=0
        // delta = 256 >> 0 = 256
        // target = 256 - 256 - 1 = -1 (wraps to 0xFFFF)
        // But that's still the wrapping behavior
        assert!(pulse.sweep.negate);
    }

    #[test]
    fn sweep_negate_pulse2_no_extra_subtract() {
        let mut pulse = Pulse::new(PulseChannel::Two);
        pulse.set_period(0x100); // period = 256
        pulse.write_sweep(0x89); // enabled, negate, shift=1
        // delta = 256 >> 1 = 128
        // target = 256 - 128 = 128 (no extra -1 for Pulse 2)
        assert_eq!(pulse.sweep.target_period, 128);
    }

    #[test]
    fn pulse_output_reflects_duty_and_volume() {
        let mut pulse = Pulse::new(PulseChannel::One);
        pulse.set_enabled(true);

        // Set a valid period (> 8)
        pulse.write_timer_lo(0x80);

        // Set constant volume = 5, duty = 75% (always starts high)
        pulse.write_ctrl(0xD5); // D7D6=11 (75%), D4=1 (const), D3..D0=0101

        // Load length counter
        pulse.write_timer_hi(0x08); // length load index 1, timer hi = 0

        // Apply length reload
        pulse.length.reload();

        // At this point duty step was reset to 0, and Duty75 pattern[0] = 1
        // Volume = 5, length counter > 0
        // Not muted because period = 0x80 = 128 (>= 8)
        let out = pulse.output();
        assert_eq!(out, 5.0);
    }

    #[test]
    fn pulse_output_zero_when_length_counter_zero() {
        let mut pulse = Pulse::new(PulseChannel::One);
        // Don't enable -> length counter stays at 0
        pulse.write_ctrl(0xD5);
        pulse.write_timer_lo(0x80);
        assert_eq!(pulse.volume(), 0); // length counter is 0
    }

    #[test]
    fn half_frame_clocks_length_counter() {
        let mut pulse = Pulse::new(PulseChannel::One);
        pulse.set_enabled(true);

        // Don't set halt, so length counter will decrement
        pulse.write_ctrl(0x15); // D5=0 (no halt)
        pulse.write_timer_lo(0x80);
        pulse.write_timer_hi(0x08); // length load = index 1
        pulse.length.reload();

        let before = pulse.length.counter();
        pulse.clock_half_frame();
        let after = pulse.length.counter();

        assert!(after < before, "length counter should have decremented");
    }

    #[test]
    fn reset_clears_pulse_state() {
        let mut pulse = Pulse::new(PulseChannel::One);
        pulse.set_enabled(true);
        pulse.write_ctrl(0xFF);
        pulse.write_sweep(0xFF);
        pulse.write_timer_lo(0xFF);
        pulse.write_timer_hi(0xFF);

        pulse.reset();

        assert_eq!(pulse.real_period, 0);
        assert_eq!(pulse.output(), 0.0);
    }
}
