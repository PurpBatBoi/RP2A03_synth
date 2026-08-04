//! rp2a03_core\src\apu_triangle.rs
//!
//! Adapted from TetaNES: https://github.com/lukexor/tetanes/blob/main/tetanes-core/src/apu/triangle.rs
//! Implementation based on NESdev APU Triangle documentation.
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
    pub reload: bool,
    pub control: bool,
    pub counter_reload: u8,
    pub counter: u8,
}

impl LinearCounter {
    pub fn new() -> Self {
        Self {
            reload: false,
            control: false,
            counter_reload: 0,
            counter: 0,
        }
    }

    pub fn write(&mut self, val: u8) {
        self.counter_reload = val & 0x7F;
        self.counter = val & 0x7F;
    }

    pub fn counter(&self) -> u8 {
        self.counter
    }

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
    pub real_period: u16,
    pub timer: Timer,
    pub sequence: u8,
    pub length: LengthCounter,
    pub linear: LinearCounter,
    pub force_silent: bool,
    pub volume: f32,
    volume_target: f32,
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
        }
    }

    #[inline]
    pub fn is_muted(&self) -> bool {
        self.real_period < 2 || self.force_silent
    }

    pub fn silent(&self) -> bool {
        self.force_silent
    }

    pub fn set_silent(&mut self, silent: bool) {
        self.force_silent = silent;
    }

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

    /// Clock on quarter frame (240Hz): linear counter only.
    pub fn clock_quarter_frame(&mut self) {
        self.linear.clock();
    }

    /// Clock on half frame (120Hz): linear counter + length counter.
    pub fn clock_half_frame(&mut self) {
        self.clock_quarter_frame();
        self.length.clock();
    }

    /// Final sample output of this triangle channel (0.0 to 15.0 scaled by volume).
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
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_triangle_is_muted_until_period_set() {
        let tri = Triangle::new();
        // Period is 0 (< 2), so muted
        assert!(tri.is_muted());
        assert_eq!(tri.output(), 0.0);
    }

    #[test]
    fn triangle_period_updates_correctly() {
        let mut tri = Triangle::new();
        tri.write_timer_lo(0x80);
        tri.write_timer_hi(0x02);
        // real_period = (0x02 << 8) | 0x80 = 640
        assert_eq!(tri.real_period, 640);
        assert!(!tri.is_muted());
    }

    #[test]
    fn linear_counter_reload_and_clock() {
        let mut lc = LinearCounter::new();
        lc.write(10);
        lc.reload = true;
        lc.clock(); // reload happens
        assert_eq!(lc.counter(), 10);
        assert!(!lc.reload); // control is false, so reload flag is cleared

        lc.clock(); // decrements to 9
        assert_eq!(lc.counter(), 9);
    }

    #[test]
    fn volume_scaling() {
        let mut tri = Triangle::new();
        tri.set_enabled(true);
        tri.write_linear_counter(0xFF); // linear control = true, reload = 127
        tri.linear.reload = true;
        tri.linear.clock(); // counter = 127
        tri.write_timer_lo(0x80);
        tri.write_timer_hi(0x02); // length counter loaded and active
        tri.length.reload();

        tri.sequence = 0; // step 0 -> raw value 15
        tri.set_volume(15.0);
        assert_eq!(tri.output(), 15.0);

        tri.set_volume(0.0);
        assert_eq!(tri.output(), 0.0);

        tri.set_volume(5.0);
        // (15 * 5) / 15 = 5.0
        assert_eq!(tri.output(), 5.0);
    }

    #[test]
    fn test_triangle_volume_no_aliasing_on_intermediate_steps() {
        let mut tri = Triangle::new();
        tri.set_enabled(true);
        tri.write_linear_counter(0xFF);
        tri.linear.reload = true;
        tri.linear.clock();
        tri.write_timer_lo(0x80);
        tri.write_timer_hi(0x02);
        tri.length.reload();

        // Check intermediate volume step 7.0 (a 0-14 step)
        tri.set_volume(7.0);

        // Sequence steps 0 through 15: 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0
        let mut outputs = Vec::new();
        for seq in 0..16 {
            tri.sequence = seq;
            outputs.push(tri.output());
        }

        // Verify that step deltas between consecutive sequence positions are constant
        // (7.0 / 15.0) rather than staircased/aliased integer steps.
        let expected_step_delta = 7.0 / 15.0;
        for i in 0..15 {
            let delta = outputs[i] - outputs[i + 1];
            assert!(
                (delta - expected_step_delta).abs() < 1e-6,
                "Step delta should be smooth linear delta {}, got {}",
                expected_step_delta,
                delta
            );
        }
    }

    #[test]
    fn target_volume_slews_without_changing_immediate_volume_api() {
        let mut tri = Triangle::new();
        tri.set_enabled(true);
        tri.write_linear_counter(0xFF);
        tri.linear.reload = true;
        tri.linear.clock();
        tri.write_timer_lo(0x80);
        tri.write_timer_hi(0x02);
        tri.length.reload();
        tri.sequence = 0;

        tri.set_volume(15.0);
        tri.set_volume_target(0.0);
        assert_eq!(tri.volume(), 15.0);

        tri.clock();
        assert!(tri.volume() < 15.0);
        assert!(tri.volume() > 0.0);

        for _ in 0..2_000 {
            tri.clock();
        }
        assert_eq!(tri.volume(), 0.0);
    }

    #[test]
    fn set_period_hi_soft_does_not_set_linear_reload() {
        let mut tri = Triangle::new();
        tri.write_timer_lo(0x80);
        tri.write_timer_hi(0x02);
        tri.linear.reload = false;

        tri.set_period_hi_soft(0x03);
        assert_eq!(tri.real_period, 0x0380);
        assert!(!tri.linear.reload);
    }

    #[test]
    fn reset_clears_state() {
        let mut tri = Triangle::new();
        tri.write_timer_lo(0xFF);
        tri.write_timer_hi(0x07);
        tri.reset();

        assert_eq!(tri.real_period, 0);
        assert_eq!(tri.output(), 0.0);
    }
}
