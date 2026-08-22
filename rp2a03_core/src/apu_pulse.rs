//! `rp2a03_core\src\apu_pulse.rs`
//!
//! Adapted from `TetaNES`: <https://github.com/lukexor/tetanes/blob/main/tetanes-core/src/apu/pulse.rs>
//! Implementation based on the info from `NESdev`, Blaarg's "`apu_ref`" and Brad Taylor's "2A03 technical reference"
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

// ─────────────────────────────────────────────
// Pulse Channel Selection
// ─────────────────────────────────────────────

/// Identifies which of the two pulse channels this is.
/// Pulse 1 and Pulse 2 differ only in sweep negate behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PulseChannel {
    /// $4000-$4003 — sweep negate uses one's-complement (`-x-1`).
    One,
    /// $4004-$4007 — sweep negate uses two's-complement (`-x`).
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
    pub(crate) negate: bool,
    reload: bool,
    shift: u8,
    divider: u8,
    period: u8,
    pub(crate) target_period: u16,
}

impl Default for Sweep {
    fn default() -> Self {
        Self::new()
    }
}

impl Sweep {
    /// A disabled sweep unit with zero shift/period.
    #[must_use]
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

    /// Resets to power-on state.
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
    pub(crate) real_period: u16,
    timer: Timer,
    duty: u8,      // row: DUTY_PATTERNS index, 0..=3
    duty_step: u8, // column: sequencer phase, 0..=7, counts down wrapping
    pub(crate) length: LengthCounter,
    pub(crate) envelope: Envelope,
    pub(crate) sweep: Sweep,
}

impl Pulse {
    /// A silenced pulse channel at 50% duty, bound to `channel`.
    #[must_use]
    pub fn new(channel: PulseChannel) -> Self {
        Self {
            channel,
            real_period: 0,
            timer: Timer::new(0),
            duty: 2, // matches the pre-flatten DutySequencer default (Duty50)
            duty_step: 0,
            length: LengthCounter::new(),
            envelope: Envelope::new(),
            sweep: Sweep::new(),
        }
    }

    // ── Muting ──────────────────────────────

    /// The pulse channel is muted when the period is < 8 (ultrasonic),
    /// or when the sweep unit's target period exceeds $7FF.
    #[inline]
    #[must_use]
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

    pub(crate) fn set_period(&mut self, period: u16) {
        self.real_period = period;
        // The pulse timer is clocked every other CPU cycle,
        // so the effective timer period is (period * 2) + 1.
        self.timer.period = (period * 2) + 1;
        self.update_target_period();
    }

    // ── Register writes ─────────────────────

    /// $4000/$4004 Pulse control
    ///   D7..D6: Duty cycle
    ///   D5:     Length counter halt / Envelope loop
    ///   D4:     Constant volume flag
    ///   D3..D0: Volume / Envelope period
    pub fn write_ctrl(&mut self, val: u8) {
        self.duty = (val & 0xC0) >> 6;
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
        self.duty_step = 0;
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
    /// This is the software-equivalent of Blaarg's smooth vibrato trick used by `FamiStudio`
    /// (ChannelStateSquare.cs / `famistudio_ca65.s`) — adapted for a VST where we can update
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
    #[must_use]
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
            // The duty sequencer counts downward (wrapping), matching real hardware.
            self.duty_step = self.duty_step.wrapping_sub(1) & 0x07;
        }
    }

    // ── Output ──────────────────────────────

    /// Final sample output of this pulse channel.
    #[must_use]
    pub fn output(&self) -> f32 {
        if self.is_muted() {
            0.0
        } else {
            f32::from(DUTY_PATTERNS[self.duty as usize][self.duty_step as usize] * self.volume())
        }
    }

    // ── Reset ───────────────────────────────

    /// Reset all sub-components.
    pub fn reset(&mut self) {
        self.real_period = 0;
        self.timer = Timer::new(0);
        self.duty = 2; // matches the pre-flatten DutySequencer default (Duty50)
        self.duty_step = 0;
        self.length.reset();
        self.envelope.reset();
        self.sweep.reset();
        self.update_target_period();
    }
}
