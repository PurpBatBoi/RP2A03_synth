//! `rp2a03_core\src\vrc6_audio.rs`
//!
//! VRC6 Sound Expansion implementation (pulse 1, pulse 2, saw).
//!
//! Adapted from Mesen:
//! <https://github.com/nesdev-org/MesenCE/blob/master/Core/NES/Mappers/Audio/Vrc6Pulse.h>
//! <https://github.com/nesdev-org/MesenCE/blob/master/Core/NES/Mappers/Audio/Vrc6Saw.h>
//!
//! See: <https://www.nesdev.org/wiki/VRC6_audio>

// ─────────────────────────────────────────────
// Frequency Divider
// ─────────────────────────────────────────────

/// VRC6's free-running divider, clocked once per CPU cycle (there's no
/// "every other cycle" halving like the 2A03 timer). The reload value is
/// `(frequency >> frequency_shift) + 1`, recomputed by the caller on every
/// tick rather than cached, matching the original hardware/Mesen behavior.
#[derive(Debug, Clone)]
pub struct Divider {
    counter: i32,
}

impl Default for Divider {
    fn default() -> Self {
        Self::new()
    }
}

impl Divider {
    /// A divider primed to fire on its very first tick.
    #[must_use]
    pub fn new() -> Self {
        Self { counter: 1 }
    }

    /// Ticks the divider down by one. Returns true (and reloads) when it
    /// reaches zero.
    pub fn tick(&mut self, reload: i32) -> bool {
        self.counter -= 1;
        if self.counter == 0 {
            self.counter = reload;
            true
        } else {
            false
        }
    }
}

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
    pub(crate) duty_cycle: u8,
    ignore_duty: bool,
    pub(crate) current_step: u8,
}

impl Default for DutySequencer {
    fn default() -> Self {
        Self::new()
    }
}

impl DutySequencer {
    /// A duty sequencer at step 0, threshold 0, not in DDA/PCM mode.
    #[must_use]
    pub fn new() -> Self {
        Self {
            duty_cycle: 0,
            ignore_duty: false,
            current_step: 0,
        }
    }

    pub(crate) fn set_duty_cycle(&mut self, duty_cycle: u8) {
        self.duty_cycle = duty_cycle;
    }

    pub(crate) fn set_ignore_duty(&mut self, ignore_duty: bool) {
        self.ignore_duty = ignore_duty;
    }

    /// Whether the channel is "on" this step: always true in DDA/PCM mode,
    /// otherwise true while `current_step <= duty_cycle`.
    #[must_use]
    pub fn output(&self) -> bool {
        self.ignore_duty || self.current_step <= self.duty_cycle
    }

    /// Advances the 4-bit step counter, wrapping 15 back to 0.
    pub fn clock(&mut self) {
        self.current_step = (self.current_step + 1) & 0x0F;
    }

    /// Resets the step counter to 0 without touching the duty threshold.
    pub fn reset_step(&mut self) {
        self.current_step = 0;
    }
}

// ─────────────────────────────────────────────
// Pulse Channel
// ─────────────────────────────────────────────

/// VRC6 Pulse Channel provides square wave generation for the VRC6 sound
/// expansion chip (Konami, used by e.g. Akumajou Densetsu / Castlevania III).
///
/// Unlike the 2A03 pulse channels, the VRC6 pulse has no envelope, no length
/// counter, and no sweep unit — just a constant 4-bit volume gated by a
/// duty-cycle comparator. The frequency shift (used for VRC6's "halve"/
/// "quarter" pitch modes) is broadcast from the audio controller ($9003)
/// to all three VRC6 channels, so it's set externally rather than owned here.
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
    pub(crate) duty: DutySequencer,
    pub(crate) frequency: u16,
    pub(crate) frequency_shift: u8,
    pub(crate) enabled: bool,
    divider: Divider,
}

impl Default for Vrc6Pulse {
    fn default() -> Self {
        Self::new()
    }
}

impl Vrc6Pulse {
    /// A disabled, silent VRC6 pulse channel.
    #[must_use]
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

    /// Sets channel enable status without changing duty step or frequency bits.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !self.enabled {
            self.duty.reset_step();
        }
    }

    /// Soft update of frequency high 4 bits without resetting duty phase step.
    pub fn set_period_hi_soft(&mut self, hi_bits: u8) {
        self.frequency = (self.frequency & 0x00FF) | (u16::from(hi_bits & 0x0F) << 8);
    }

    // ── Clocking ────────────────────────────

    /// Clock the pulse divider. When it expires, the duty step advances.
    /// No-op while the channel is disabled, matching hardware (the divider
    /// simply doesn't run).
    pub fn clock(&mut self) {
        if self.enabled {
            let reload = i32::from(self.frequency >> self.frequency_shift) + 1;
            if self.divider.tick(reload) {
                self.duty.clock();
            }
        }
    }

    // ── Output ──────────────────────────────

    /// Current volume contribution of this channel (0 if disabled or duty
    /// gate is closed).
    #[must_use]
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

    /// Resets to power-on state.
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

// ─────────────────────────────────────────────
// Saw Channel
// ─────────────────────────────────────────────

/// VRC6 Saw Channel provides sawtooth wave generation for the VRC6 sound
/// expansion chip (Konami, used by e.g. Akumajou Densetsu / Castlevania III).
///
/// The saw channel has no volume register or duty cycle; instead an
/// `accumulator` is incremented by `accumulator_rate` on alternating steps
/// of a 14-step cycle, then reset to zero at the start of the next cycle,
/// producing a sawtooth ramp. Only the high 5 bits of the accumulator are
/// output. Like the pulse channels, the frequency shift is broadcast from
/// the audio controller ($9003) to all three VRC6 channels.
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
    pub(crate) accumulator_rate: u8,
    pub(crate) accumulator: u8,
    pub(crate) frequency: u16,
    pub(crate) frequency_shift: u8,
    pub(crate) enabled: bool,
    pub(crate) step: u8,
    divider: Divider,
}

impl Default for Vrc6Saw {
    fn default() -> Self {
        Self::new()
    }
}

impl Vrc6Saw {
    /// A disabled, silent VRC6 saw channel.
    #[must_use]
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
    #[must_use]
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
            let reload = i32::from(self.frequency >> self.frequency_shift) + 1;
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
    #[must_use]
    pub fn volume(&self) -> u8 {
        if self.enabled {
            self.accumulator >> 3
        } else {
            0
        }
    }

    // ── Reset ───────────────────────────────

    /// Resets to power-on state.
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}
