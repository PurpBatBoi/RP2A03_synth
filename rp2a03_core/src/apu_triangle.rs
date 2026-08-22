//! `rp2a03_core\src\apu_triangle.rs`
//!
//! Adapted from `TetaNES`: <https://github.com/lukexor/tetanes/blob/main/tetanes-core/src/apu/triangle.rs>
//! Implementation based on `NESdev` APU Triangle documentation.
//!
//! APU Triangle Channel implementation.
//!
//! See: <https://www.nesdev.org/wiki/APU_Triangle>

use crate::apu::{LengthCounter, Timer};

// ─────────────────────────────────────────────
// Linear Counter
// ─────────────────────────────────────────────

/// APU Linear Counter provides duration control for the APU triangle channel.
///
/// See: <https://www.nesdev.org/wiki/APU_Triangle>
#[derive(Debug, Clone, Default)]
pub struct LinearCounter {
    /// Set by `$400B` (a fresh timer-hi write); applied and cleared by the next `clock()`.
    pub reload: bool,
    /// Mirrors the triangle's length-counter halt flag; keeps `reload` sticky when set.
    pub control: bool,
    /// The 7-bit value the counter reloads to.
    pub counter_reload: u8,
    /// Frames remaining before the triangle silences.
    pub counter: u8,
}

impl LinearCounter {
    /// A zeroed linear counter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            reload: false,
            control: false,
            counter_reload: 0,
            counter: 0,
        }
    }

    /// `$4008`'s low 7 bits: sets both the live counter and its reload value.
    pub fn write(&mut self, val: u8) {
        self.counter_reload = val & 0x7F;
        self.counter = val & 0x7F;
    }

    /// Frames remaining before the triangle silences.
    #[must_use]
    pub fn counter(&self) -> u8 {
        self.counter
    }

    /// Advances one quarter-frame: reloads on a pending `reload`, else decrements.
    pub fn clock(&mut self) {
        if self.reload {
            self.counter = self.counter_reload;
        } else if self.counter > 0 {
            self.counter -= 1;
        }
        if !self.control {
            self.reload = false;
        }
    }

    /// Resets to power-on state.
    pub fn reset(&mut self) {
        self.reload = false;
        self.control = false;
        self.counter_reload = 0;
        self.counter = 0;
    }
}

// ─────────────────────────────────────────────
// Triangle Channel
// ─────────────────────────────────────────────

/// APU Triangle Channel provides triangle wave generation.
///
/// See: <https://www.nesdev.org/wiki/APU_Triangle>
#[derive(Debug, Clone)]
pub struct Triangle {
    /// The raw 11-bit period from `$400A`/`$400B`, before hardware's `*2+1` timer doubling.
    pub real_period: u16,
    /// Drives the 32-step sequence at the doubled rate real triangle hardware uses.
    pub timer: Timer,
    /// Current index into the 32-step triangle waveform.
    pub sequence: u8,
    /// Silences the channel a hardware-defined number of frames after `$400B`.
    pub length: LengthCounter,
    /// Duration control gating the sequence advance, separate from `length`.
    pub linear: LinearCounter,
    /// Voice-allocation mute, independent of the hardware envelope/length state.
    pub force_silent: bool,
    /// Current slewed output scale (0.0..=15.0); chases `volume_target` in `clock()`.
    pub volume: f32,
    volume_target: f32,
    /// The last amplitude-scaled sample this channel produced while gated,
    /// held and replayed while ungated — real triangle hardware simply
    /// stops clocking rather than fading out. Owned here (not by the voice
    /// mixdown) so `Channel::output_delta` can honor the hold on its own.
    pub last_output: i32,
}

impl Default for Triangle {
    fn default() -> Self {
        Self::new()
    }
}

impl Triangle {
    const SEQUENCE: [u8; 32] = [
        15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11,
        12, 13, 14, 15,
    ];

    /// A silenced triangle channel at full volume, period zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            real_period: 0,
            timer: Timer::new(0),
            sequence: 0,
            length: LengthCounter::new(),
            linear: LinearCounter::new(),
            force_silent: false,
            volume: 15.0,
            volume_target: 15.0,
            last_output: 0,
        }
    }

    /// True for an ultrasonic period (real hardware DC-offsets rather than
    /// audibly muting there) or when `force_silent` is set.
    #[inline]
    #[must_use]
    pub fn is_muted(&self) -> bool {
        self.real_period < 2 || self.force_silent
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

    /// Sets both the current and target volume immediately, with no slew.
    pub fn set_volume(&mut self, volume: f32) {
        let volume = volume.clamp(0.0, 15.0);
        self.volume = volume;
        self.volume_target = volume;
    }

    /// Set the audible volume target without introducing a discontinuity.
    ///
    /// The target is reached by `clock()`, which runs at the APU clock rate.
    /// This keeps envelope timing sample-accurate while preventing a volume
    /// sequence step from appearing as an instantaneous DAC jump.
    pub fn set_volume_target(&mut self, volume: f32) {
        self.volume_target = volume.clamp(0.0, 15.0);
    }

    #[inline]
    fn advance_volume(&mut self) {
        // At the NTSC CPU clock this reaches a full-scale change in about
        // 1 ms, short enough to retain envelope articulation but long enough
        // to suppress clicks from frame-rate volume changes.
        const SLEW_PER_CLOCK: f32 = 1.0 / 128.0;
        let delta = self.volume_target - self.volume;
        if delta.abs() <= SLEW_PER_CLOCK {
            self.volume = self.volume_target;
        } else {
            self.volume += delta.signum() * SLEW_PER_CLOCK;
        }
    }

    /// Current output scale (0.0..=15.0), or 0.0 while either counter is expired.
    #[must_use]
    pub fn volume(&self) -> f32 {
        if self.length.counter() > 0 && self.linear.counter() > 0 {
            self.volume
        } else {
            0.0
        }
    }

    fn set_period(&mut self, period: u16) {
        self.real_period = period;
        self.timer.period = period;
    }

    /// $4008 Triangle linear counter control
    ///   D7:     Control flag (linear counter halt / length counter halt)
    ///   D6..D0: Linear counter reload value
    pub fn write_linear_counter(&mut self, val: u8) {
        self.linear.control = (val & 0x80) == 0x80;
        self.linear.write(val & 0x7F);
        self.length.write_ctrl(self.linear.control);
    }

    /// $400A Triangle timer lo
    ///   D7..D0: Timer low 8 bits
    pub fn write_timer_lo(&mut self, val: u8) {
        self.set_period((self.real_period & 0x0700) | u16::from(val));
    }

    /// $400B Triangle timer hi
    ///   D7..D3: Length counter load
    ///   D2..D0: Timer high 3 bits
    pub fn write_timer_hi(&mut self, val: u8) {
        self.length.write(val >> 3);
        self.set_period((self.real_period & 0x00FF) | (u16::from(val & 0x07) << 8));
        self.linear.reload = true;
        self.linear.counter = self.linear.counter_reload;
    }

    /// Soft high-period update: updates only the high 3 bits of the timer period
    /// **without** triggering linear counter reload.
    pub fn set_period_hi_soft(&mut self, hi_bits: u8) {
        self.set_period((self.real_period & 0x00FF) | (u16::from(hi_bits & 0x07) << 8));
    }

    /// Enable or disable length counter from $4015.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.length.set_enabled(enabled);
    }

    /// Clock the triangle timer. When the timer expires, if length > 0 and linear > 0,
    /// the 32-step sequence advances.
    /// Also applies any pending length counter reload (must happen each cycle).
    pub fn clock(&mut self) {
        self.advance_volume();
        self.length.reload();
        if self.timer.tick() && self.length.counter() > 0 && self.linear.counter() > 0 {
            self.sequence = (self.sequence + 1) & 0x1F;
        }
    }

    /// Final sample output of this triangle channel (0.0 to 15.0 scaled by volume).
    #[must_use]
    pub fn output(&self) -> f32 {
        if self.is_muted() {
            0.0
        } else {
            let raw_step = f32::from(Self::SEQUENCE[self.sequence as usize]);
            (raw_step * self.volume()) / 15.0
        }
    }

    /// Reset all sub-components.
    pub fn reset(&mut self) {
        self.real_period = 0;
        self.timer = Timer::new(0);
        self.sequence = 0;
        self.length.reset();
        self.linear.reset();
        self.force_silent = false;
        self.volume = 15.0;
        self.volume_target = 15.0;
        self.last_output = 0;
    }
}
