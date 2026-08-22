//! `rp2a03_core\src\channel.rs`
//!
//! `Channel` is the register-level interface every emulated chip implements.
//! Each chip owns its own "I ignore that register" facts through the no-op
//! defaults below, instead of a hand-matched enum listing all seven chips at
//! every call site (the old `AnyChannel`).

use crate::apu_noise::Noise;
use crate::apu_pulse::Pulse;
use crate::apu_triangle::Triangle;
use crate::fds_audio::Fds;
use crate::s5b_audio::Sunsoft;
use crate::vrc6_audio::{Vrc6Pulse, Vrc6Saw};

/// When a fresh top note should reset a chip's running phase, per
/// `MidiHandler::note_on`. Three policies cover all seven chips today; a new
/// chip that doesn't override [`Channel::phase_reset`] gets the common case
/// (`OnRetrigger`) for free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseReset {
    /// Reset exactly once, the first time this chip is ever used (Pulse,
    /// VRC6 Pulse — both track their own "have I been triggered yet" latch).
    OnFirstUse,
    /// Reset whenever the previous note released before this one started.
    OnRetrigger,
    /// Always reset — this chip has no continuous phase worth preserving
    /// across notes (Noise).
    Always,
}

/// The shape of a chip's period register: how many high bits it has, what
/// fixed control bits share that byte, and which direction a larger value
/// moves the pitch. These three facts always travel together per chip, so
/// they're one descriptor instead of three separate trait methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodRegister {
    /// Bitmask the high timer byte applies to a shifted-down period.
    pub hi_mask: u8,
    /// Fixed bits folded into every timer-hi write alongside the masked
    /// high bits (e.g. Pulse/Triangle/Noise's length-counter-adjacent 0xF8).
    pub hi_control_bits: u8,
    /// True for chips whose register runs frequency-proportional (a larger
    /// value is a higher pitch) instead of the usual period-proportional
    /// direction (FDS only).
    pub inverted: bool,
}

impl Default for PeriodRegister {
    fn default() -> Self {
        Self {
            hi_mask: 0x07,
            hi_control_bits: 0xF8,
            inverted: false,
        }
    }
}

/// One emulated sound channel, addressed generically by the MIDI handler and
/// the per-voice mixdown. Every method has a sensible no-op/identity default
/// so a new chip only overrides what it actually does.
pub trait Channel {
    /// Required so the conformance suite (`tests::assert_conformant`) can
    /// drive every chip identically. Production code never calls `reset` or
    /// `clock` through `dyn Channel` — `output_delta`'s own impl reaches
    /// each chip's inherent `clock`/output path directly — so both are
    /// intentional API-completeness surface, not dead code.
    fn reset(&mut self);
    /// See `reset`'s doc above — same rationale applies here.
    fn clock(&mut self);

    /// Clocks the chip and returns its amplitude-scaled contribution to the
    /// mix for this sample, honoring `gate` (whether a note is currently
    /// held) however this chip's hardware actually behaves when ungated.
    fn output_delta(&mut self, gate: bool) -> i32;

    /// Enables or disables the chip's length counter, where it has one. No-op default.
    fn set_enabled(&mut self, _enabled: bool) {}
    /// Writes the timer/period register's low byte. No-op default.
    fn write_timer_lo(&mut self, _val: u8) {}
    /// Writes the timer/period register's high byte, latching any side effects
    /// (length counter load, linear counter reload) real hardware ties to it. No-op default.
    fn write_timer_hi(&mut self, _val: u8) {}
    /// Updates only the period register's high bits, without `write_timer_hi`'s side effects.
    /// No-op default.
    fn set_period_hi_soft(&mut self, _hi_bits: u8) {}

    /// Register writes specific to a freshly triggered top note (sweep
    /// reset, linear counter kick, noise length latch). No-op default.
    fn on_top_note(&mut self, _reset_phase: bool) {}

    /// This chip's [`PhaseReset`] policy. Defaults to the common case, `OnRetrigger`.
    fn phase_reset(&self) -> PhaseReset {
        PhaseReset::OnRetrigger
    }

    /// This chip's period-register shape. Defaults to the 2A03-common layout
    /// (Pulse/Triangle/Noise); chips with a different register override it.
    fn period_register(&self) -> PeriodRegister {
        PeriodRegister::default()
    }

    /// Chip-specific correction applied to the final computed period before
    /// it's split into timer-lo/timer-hi (only Triangle needs one today).
    fn fixup_final_period(&self, period: i32) -> i32 {
        period
    }
}

impl Channel for Pulse {
    fn reset(&mut self) {
        Self::reset(self);
    }

    fn clock(&mut self) {
        Self::clock(self);
    }

    fn output_delta(&mut self, gate: bool) -> i32 {
        self.clock();
        if gate && !self.is_muted() {
            self.output() as i32 * AMPLITUDE_SCALE
        } else {
            0
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        Self::set_enabled(self, enabled);
    }

    fn write_timer_lo(&mut self, val: u8) {
        Self::write_timer_lo(self, val);
    }

    fn write_timer_hi(&mut self, val: u8) {
        Self::write_timer_hi(self, val);
    }

    fn set_period_hi_soft(&mut self, hi_bits: u8) {
        Self::set_period_hi_soft(self, hi_bits);
    }

    fn on_top_note(&mut self, reset_phase: bool) {
        if reset_phase {
            self.write_sweep(0x08);
        }
    }

    fn phase_reset(&self) -> PhaseReset {
        PhaseReset::OnFirstUse
    }
}

const AMPLITUDE_SCALE: i32 = 1500;

// Pulse, Triangle and Noise all use `PeriodRegister::default()` (0x07 mask,
// 0xF8 control bits, not inverted) — no override needed.

impl Channel for Triangle {
    fn reset(&mut self) {
        Self::reset(self);
    }

    fn clock(&mut self) {
        Self::clock(self);
    }

    fn output_delta(&mut self, gate: bool) -> i32 {
        if gate {
            self.clock();
            if self.is_muted() {
                0
            } else {
                let output = (self.output() * TRIANGLE_AMPLITUDE_SCALE as f32) as i32;
                self.last_output = output;
                output
            }
        } else {
            self.last_output
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        Self::set_enabled(self, enabled);
    }

    fn write_timer_lo(&mut self, val: u8) {
        Self::write_timer_lo(self, val);
    }

    fn write_timer_hi(&mut self, val: u8) {
        Self::write_timer_hi(self, val);
    }

    fn set_period_hi_soft(&mut self, hi_bits: u8) {
        Self::set_period_hi_soft(self, hi_bits);
    }

    fn on_top_note(&mut self, _reset_phase: bool) {
        self.write_linear_counter(0xFF);
    }

    fn fixup_final_period(&self, period: i32) -> i32 {
        (period - 1).max(0) / 2
    }
}

const TRIANGLE_AMPLITUDE_SCALE: i32 = 3000;

impl Channel for Noise {
    fn reset(&mut self) {
        Self::reset(self);
    }

    fn clock(&mut self) {
        Self::clock(self);
    }

    fn output_delta(&mut self, gate: bool) -> i32 {
        self.clock();
        if gate && !self.is_muted() {
            self.output() as i32 * AMPLITUDE_SCALE
        } else {
            0
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        Self::set_enabled(self, enabled);
    }

    fn on_top_note(&mut self, _reset_phase: bool) {
        self.write_length(0xF8);
    }

    fn phase_reset(&self) -> PhaseReset {
        PhaseReset::Always
    }
}

const VRC6_PERIOD_REGISTER: PeriodRegister = PeriodRegister {
    hi_mask: 0x0F,
    hi_control_bits: 0x80,
    inverted: false,
};

impl Channel for Vrc6Pulse {
    fn reset(&mut self) {
        Self::reset(self);
    }

    fn clock(&mut self) {
        Self::clock(self);
    }

    fn output_delta(&mut self, gate: bool) -> i32 {
        self.clock();
        if gate {
            i32::from(self.volume()) * VRC6_AMPLITUDE_SCALE
        } else {
            0
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        Self::set_enabled(self, enabled);
    }

    fn write_timer_lo(&mut self, val: u8) {
        Self::write_freq_lo(self, val);
    }

    fn write_timer_hi(&mut self, val: u8) {
        Self::write_freq_hi(self, val);
    }

    fn set_period_hi_soft(&mut self, hi_bits: u8) {
        Self::set_period_hi_soft(self, hi_bits);
    }

    fn phase_reset(&self) -> PhaseReset {
        PhaseReset::OnFirstUse
    }

    fn period_register(&self) -> PeriodRegister {
        VRC6_PERIOD_REGISTER
    }
}

impl Channel for Vrc6Saw {
    fn reset(&mut self) {
        Self::reset(self);
    }

    fn clock(&mut self) {
        Self::clock(self);
    }

    fn output_delta(&mut self, gate: bool) -> i32 {
        self.clock();
        if gate {
            (i32::from(self.volume()) * VRC6_AMPLITUDE_SCALE) / 2
        } else {
            0
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        Self::set_enabled(self, enabled);
    }

    fn write_timer_lo(&mut self, val: u8) {
        Self::write_freq_lo(self, val);
    }

    fn write_timer_hi(&mut self, val: u8) {
        Self::write_freq_hi(self, val);
    }

    fn set_period_hi_soft(&mut self, hi_bits: u8) {
        Self::set_period_hi_soft(self, hi_bits);
    }

    fn period_register(&self) -> PeriodRegister {
        VRC6_PERIOD_REGISTER
    }
}

const VRC6_AMPLITUDE_SCALE: i32 = 1500;

// Sunsoft and FDS share the same 12-bit, no-fixed-bits period register
// shape; FDS overrides only `inverted` (its register runs
// frequency-proportional instead of period-proportional).
const S5B_FDS_PERIOD_REGISTER: PeriodRegister = PeriodRegister {
    hi_mask: 0x0F,
    hi_control_bits: 0,
    inverted: false,
};

impl Channel for Sunsoft {
    fn reset(&mut self) {
        Self::reset(self);
    }

    fn clock(&mut self) {
        Self::clock(self);
    }

    fn output_delta(&mut self, gate: bool) -> i32 {
        self.clock();
        if gate {
            i32::from(self.output()) * S5B_AMPLITUDE_SCALE
        } else {
            0
        }
    }

    fn write_timer_lo(&mut self, val: u8) {
        Self::write_timer_lo(self, val);
    }

    fn write_timer_hi(&mut self, val: u8) {
        Self::write_timer_hi(self, val);
    }

    fn set_period_hi_soft(&mut self, hi_bits: u8) {
        Self::write_timer_hi(self, hi_bits);
    }

    fn period_register(&self) -> PeriodRegister {
        S5B_FDS_PERIOD_REGISTER
    }
}

const S5B_AMPLITUDE_SCALE: i32 = 6;

impl Channel for Fds {
    fn reset(&mut self) {
        Self::reset(self);
    }

    fn clock(&mut self) {
        Self::clock(self);
    }

    fn output_delta(&mut self, gate: bool) -> i32 {
        self.clock();
        if gate {
            self.output() * FDS_AMPLITUDE_SCALE
        } else {
            0
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        Self::set_enabled(self, enabled);
    }

    fn write_timer_lo(&mut self, val: u8) {
        Self::write_freq_lo(self, val);
    }

    fn write_timer_hi(&mut self, val: u8) {
        Self::write_freq_hi(self, val);
    }

    fn set_period_hi_soft(&mut self, hi_bits: u8) {
        Self::set_period_hi_soft(self, hi_bits);
    }

    fn period_register(&self) -> PeriodRegister {
        PeriodRegister {
            inverted: true,
            ..S5B_FDS_PERIOD_REGISTER
        }
    }
}

const FDS_AMPLITUDE_SCALE: i32 = 3;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apu_pulse::PulseChannel;

    /// One conformance pass run against every chip: enable/disable, a timer
    /// write, a soft period-hi write, reset idempotence, and output range
    /// all have to hold no matter which `Channel` impl is under test.
    fn assert_conformant<C: Channel + std::fmt::Debug>(mut chip: C) {
        chip.set_enabled(true);
        chip.set_enabled(false);
        chip.set_enabled(true);

        chip.write_timer_lo(0xAB);
        chip.write_timer_hi(0xCD);
        chip.set_period_hi_soft(0x03);

        let silent = chip.output_delta(false);
        assert_eq!(
            silent, 0,
            "a freshly constructed, ungated chip must contribute silence"
        );

        chip.clock();
        let gated = chip.output_delta(true);
        assert!(
            gated.abs() <= 200_000,
            "output_delta({gated}) is out of the sane amplitude-scaled range"
        );

        chip.reset();
        let after_first_reset = format!("{chip:?}");
        chip.reset();
        let after_second_reset = format!("{chip:?}");
        assert_eq!(
            after_first_reset, after_second_reset,
            "reset() must be idempotent"
        );
    }

    #[test]
    fn pulse_is_conformant() {
        assert_conformant(Pulse::new(PulseChannel::One));
    }

    #[test]
    fn triangle_is_conformant() {
        assert_conformant(Triangle::new());
    }

    #[test]
    fn noise_is_conformant() {
        assert_conformant(Noise::new());
    }

    #[test]
    fn vrc6_pulse_is_conformant() {
        assert_conformant(Vrc6Pulse::new());
    }

    #[test]
    fn vrc6_saw_is_conformant() {
        assert_conformant(Vrc6Saw::new());
    }

    #[test]
    fn sunsoft_is_conformant() {
        assert_conformant(Sunsoft::new());
    }

    #[test]
    fn fds_is_conformant() {
        assert_conformant(Fds::new());
    }

    #[test]
    fn period_register_matches_each_chip_s_register_layout() {
        let pulse = Pulse::new(PulseChannel::One).period_register();
        assert_eq!(pulse.hi_mask, 0x07);
        assert_eq!(pulse.hi_control_bits | 0x03, 0xFB);
        assert!(!pulse.inverted);

        assert_eq!(Triangle::new().period_register(), pulse);
        assert_eq!(Noise::new().period_register(), pulse);

        let vrc6 = Vrc6Pulse::new().period_register();
        assert_eq!(vrc6.hi_mask, 0x0F);
        assert_eq!(vrc6.hi_control_bits | 0x0A, 0x8A);
        assert!(!vrc6.inverted);
        assert_eq!(Vrc6Saw::new().period_register(), vrc6);

        let s5b = Sunsoft::new().period_register();
        assert_eq!(s5b.hi_mask, 0x0F);
        assert_eq!(s5b.hi_control_bits | 0x0A, 0x0A);
        assert!(!s5b.inverted);

        let fds = Fds::new().period_register();
        assert_eq!(fds.hi_mask, 0x0F);
        assert_eq!(fds.hi_control_bits | 0x0A, 0x0A);
        assert!(fds.inverted);
    }

    #[test]
    fn only_triangle_halves_its_final_period() {
        assert_eq!(Pulse::new(PulseChannel::One).fixup_final_period(101), 101);
        assert_eq!(Triangle::new().fixup_final_period(101), 50);
        assert_eq!(Triangle::new().fixup_final_period(0), 0);
    }

    #[test]
    fn phase_reset_policy_matches_each_chip_s_hardware_fact() {
        assert_eq!(
            Pulse::new(PulseChannel::One).phase_reset(),
            PhaseReset::OnFirstUse
        );
        assert_eq!(Vrc6Pulse::new().phase_reset(), PhaseReset::OnFirstUse);
        assert_eq!(Noise::new().phase_reset(), PhaseReset::Always);
        assert_eq!(Triangle::new().phase_reset(), PhaseReset::OnRetrigger);
        assert_eq!(Vrc6Saw::new().phase_reset(), PhaseReset::OnRetrigger);
        assert_eq!(Sunsoft::new().phase_reset(), PhaseReset::OnRetrigger);
        assert_eq!(Fds::new().phase_reset(), PhaseReset::OnRetrigger);
    }

    #[test]
    fn triangle_holds_its_last_output_while_ungated() {
        let mut triangle = Triangle::new();
        triangle.set_enabled(true);
        triangle.write_timer_lo(0x50);
        triangle.write_timer_hi(0x07);
        for _ in 0..64 {
            Channel::output_delta(&mut triangle, true);
        }
        let held = triangle.last_output;
        assert_eq!(Channel::output_delta(&mut triangle, false), held);
        assert_eq!(Channel::output_delta(&mut triangle, false), held);
    }
}
