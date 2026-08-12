//! Software LFO implementation based on DN-FamiTracker.

pub static FT_VIBRATO_TABLE: [u8; 256] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02,
    0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x02, 0x02, 0x02, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
    0x00, 0x00, 0x00, 0x01, 0x01, 0x02, 0x02, 0x03, 0x03, 0x03, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    0x00, 0x00, 0x01, 0x02, 0x02, 0x03, 0x03, 0x04, 0x04, 0x05, 0x05, 0x06, 0x06, 0x06, 0x06, 0x06,
    0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x07, 0x08, 0x08, 0x09, 0x09, 0x09, 0x09,
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x09, 0x0A, 0x0B, 0x0B, 0x0B, 0x0B,
    0x00, 0x01, 0x02, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0C, 0x0D, 0x0D, 0x0D,
    0x00, 0x01, 0x03, 0x04, 0x06, 0x08, 0x09, 0x0A, 0x0C, 0x0D, 0x0E, 0x0E, 0x0F, 0x10, 0x10, 0x10,
    0x00, 0x02, 0x04, 0x06, 0x08, 0x0A, 0x0C, 0x0D, 0x0F, 0x11, 0x12, 0x13, 0x14, 0x15, 0x15, 0x15,
    0x00, 0x02, 0x05, 0x08, 0x0B, 0x0E, 0x10, 0x13, 0x15, 0x17, 0x18, 0x1A, 0x1B, 0x1C, 0x1D, 0x1D,
    0x00, 0x04, 0x08, 0x0C, 0x10, 0x14, 0x18, 0x1B, 0x1F, 0x22, 0x24, 0x26, 0x28, 0x2A, 0x2B, 0x2B,
    0x00, 0x06, 0x0C, 0x12, 0x18, 0x1E, 0x23, 0x28, 0x2D, 0x31, 0x35, 0x38, 0x3B, 0x3D, 0x3E, 0x3F,
    0x00, 0x09, 0x12, 0x1B, 0x24, 0x2D, 0x35, 0x3C, 0x43, 0x4A, 0x4F, 0x54, 0x58, 0x5B, 0x5E, 0x5F,
    0x00, 0x0C, 0x18, 0x25, 0x30, 0x3C, 0x47, 0x51, 0x5A, 0x62, 0x6A, 0x70, 0x76, 0x7A, 0x7D, 0x7F,
];

/// Default active LFO speed (4 ticks step per frame = ~3.75 Hz modulation rate).
pub const DEFAULT_LFO_SPEED: u8 = 4;
/// Largest host-controlled delay or fade duration, in engine frames.
pub const MAX_DELAY_FRAMES: u8 = 127;

/// Software LFO state for Vibrato (pitch) and Tremolo (volume) modulation.
#[derive(Debug, Clone)]
pub struct SoftwareLfo {
    /// Vibrato depth (0..15 index)
    pub vibrato_depth: u8,
    /// Vibrato speed (0..63 tick step)
    pub vibrato_speed: u8,
    /// Vibrato position accumulator (0..63 cycle)
    pub vibrato_pos: u8,
    /// Vibrato delay before modulation starts, in engine frames (0..=127).
    pub vibrato_delay: u8,
    vibrato_delay_remaining: u8,
    vibrato_fade_elapsed: u8,

    /// Tremolo depth (0..15 index)
    pub tremolo_depth: u8,
    /// Tremolo speed (0..63 tick step)
    pub tremolo_speed: u8,
    /// Tremolo position accumulator (0..63 cycle)
    pub tremolo_pos: u8,
    /// Tremolo delay before modulation starts, in engine frames (0..=127).
    pub tremolo_delay: u8,
    tremolo_delay_remaining: u8,
    tremolo_fade_elapsed: u8,
    /// Shared linear fade duration, in engine frames (0..=127).
    pub delay_speed: u8,
    vibrato_fade_speed: u8,
    tremolo_fade_speed: u8,
}

impl Default for SoftwareLfo {
    fn default() -> Self {
        Self {
            vibrato_depth: 0,
            vibrato_speed: DEFAULT_LFO_SPEED,
            vibrato_pos: 0,
            vibrato_delay: 0,
            vibrato_delay_remaining: 0,
            vibrato_fade_elapsed: 0,
            tremolo_depth: 0,
            tremolo_speed: DEFAULT_LFO_SPEED,
            tremolo_pos: 0,
            tremolo_delay: 0,
            tremolo_delay_remaining: 0,
            tremolo_fade_elapsed: 0,
            delay_speed: 0,
            vibrato_fade_speed: 0,
            tremolo_fade_speed: 0,
        }
    }
}

impl SoftwareLfo {
    /// Create a new `SoftwareLfo`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset LFO phases and depth/speed parameters.
    pub fn reset(&mut self) {
        self.vibrato_depth = 0;
        self.vibrato_speed = DEFAULT_LFO_SPEED;
        self.vibrato_pos = 0;
        self.vibrato_delay = 0;
        self.vibrato_delay_remaining = 0;
        self.vibrato_fade_elapsed = 0;

        self.tremolo_depth = 0;
        self.tremolo_speed = DEFAULT_LFO_SPEED;
        self.tremolo_pos = 0;
        self.tremolo_delay = 0;
        self.tremolo_delay_remaining = 0;
        self.tremolo_fade_elapsed = 0;
        self.delay_speed = 0;
        self.vibrato_fade_speed = 0;
        self.tremolo_fade_speed = 0;
    }

    /// Set Vibrato depth (0..15) and speed (0..63).
    pub fn set_vibrato(&mut self, depth: u8, speed: u8) {
        self.vibrato_depth = depth.min(15);
        self.vibrato_speed = speed.min(63);
    }

    /// Set Tremolo depth (0..15) and speed (0..63).
    pub fn set_tremolo(&mut self, depth: u8, speed: u8) {
        self.tremolo_depth = depth.min(15);
        self.tremolo_speed = speed.min(63);
    }

    /// Set delays and the shared linear fade duration. These settings apply to
    /// the next retrigger; an active delay/fade keeps its current timing.
    pub fn set_delay_params(&mut self, vibrato_delay: u8, tremolo_delay: u8, delay_speed: u8) {
        self.vibrato_delay = vibrato_delay.min(MAX_DELAY_FRAMES);
        self.tremolo_delay = tremolo_delay.min(MAX_DELAY_FRAMES);
        self.delay_speed = delay_speed.min(MAX_DELAY_FRAMES);
    }

    /// Retrigger both modulation effects for a fresh note.
    pub fn retrigger(&mut self) {
        self.vibrato_delay_remaining = self.vibrato_delay;
        self.tremolo_delay_remaining = self.tremolo_delay;
        self.vibrato_fade_elapsed = 0;
        self.tremolo_fade_elapsed = 0;
        self.vibrato_fade_speed = self.delay_speed;
        self.tremolo_fade_speed = self.delay_speed;
    }

    /// Clock the LFO accumulators by one tick (e.g. at 60 Hz frame rate).
    pub fn clock_tick(&mut self) {
        if self.vibrato_delay_remaining > 0 {
            self.vibrato_delay_remaining -= 1;
        } else {
            self.vibrato_fade_elapsed = self
                .vibrato_fade_elapsed
                .saturating_add(1)
                .min(self.vibrato_fade_speed);
        }
        if self.vibrato_delay_remaining == 0 && self.vibrato_speed > 0 {
            self.vibrato_pos = self.vibrato_pos.wrapping_add(self.vibrato_speed) & 0x3F;
        }

        if self.tremolo_delay_remaining > 0 {
            self.tremolo_delay_remaining -= 1;
        } else {
            self.tremolo_fade_elapsed = self
                .tremolo_fade_elapsed
                .saturating_add(1)
                .min(self.tremolo_fade_speed);
        }
        if self.tremolo_delay_remaining == 0 && self.tremolo_speed > 0 {
            self.tremolo_pos = self.tremolo_pos.wrapping_add(self.tremolo_speed) & 0x3F;
        }
    }

    fn vibrato_fade_scale(&self) -> u16 {
        if self.vibrato_delay_remaining > 0 || self.vibrato_fade_speed == 0 {
            256
        } else {
            (u16::from(self.vibrato_fade_elapsed) * 256 / u16::from(self.vibrato_fade_speed))
                .min(256)
        }
    }

    fn tremolo_fade_scale(&self) -> u16 {
        if self.tremolo_delay_remaining > 0 || self.tremolo_fade_speed == 0 {
            256
        } else {
            (u16::from(self.tremolo_fade_elapsed) * 256 / u16::from(self.tremolo_fade_speed))
                .min(256)
        }
    }

    /// Calculate the current Vibrato pitch period delta.
    /// Returns signed period shift (subtracting period increases pitch).
    pub fn vibrato_pitch_delta(&self) -> i16 {
        if self.vibrato_speed == 0
            || self.vibrato_depth == 0
            || self.vibrato_delay_remaining > 0
        {
            return 0;
        }

        let pos = self.vibrato_pos & 0x3F;
        let phase = pos & 0x0F;

        let (idx, negate) = match pos {
            0..=15 => (phase, false),
            16..=31 => (phase ^ 0x0F, false),
            32..=47 => (phase, true),
            _ => (phase ^ 0x0F, true),
        };

        // Table is 16 rows of 16 bytes: index = (depth << 4) | idx
        let table_idx = ((self.vibrato_depth as usize) << 4) | (idx as usize);
        let magnitude = FT_VIBRATO_TABLE[table_idx.min(255)] as i16;

        let magnitude = magnitude * self.vibrato_fade_scale() as i16 / 256;
        if negate { -magnitude } else { magnitude }
    }

    /// Calculate the current Tremolo volume reduction delta (0..15).
    pub fn tremolo_volume_delta(&self) -> u8 {
        if self.tremolo_speed == 0
            || self.tremolo_depth == 0
            || self.tremolo_delay_remaining > 0
        {
            return 0;
        }

        let pos = (self.tremolo_pos & 0x3F) >> 1; // Range 0..31
        let phase = if pos < 16 { pos } else { 31 - pos }; // Phase 0..15..0

        // Table is 16 rows of 16 bytes: index = (depth << 4) | phase
        let table_idx = ((self.tremolo_depth as usize) << 4) | (phase as usize);
        let raw_val = FT_VIBRATO_TABLE[table_idx.min(255)];

        ((u16::from(raw_val >> 1) * self.tremolo_fade_scale()) / 256) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vibrato_zero_depth_returns_zero() {
        let mut lfo = SoftwareLfo::new();
        assert_eq!(lfo.vibrato_pitch_delta(), 0);

        lfo.set_vibrato(0, 10);
        assert_eq!(lfo.vibrato_pitch_delta(), 0);
    }

    #[test]
    fn test_vibrato_4_phase_cycle() {
        let mut lfo = SoftwareLfo::new();
        lfo.set_vibrato(15, 1);

        // Phase 1 (0..15): positive delta
        lfo.vibrato_pos = 8;
        let delta_p1 = lfo.vibrato_pitch_delta();
        assert!(
            delta_p1 > 0,
            "Phase 1 should yield positive pitch delta, got {}",
            delta_p1
        );

        // Phase 3 (32..47): negative delta
        lfo.vibrato_pos = 40;
        let delta_p3 = lfo.vibrato_pitch_delta();
        assert!(
            delta_p3 < 0,
            "Phase 3 should yield negative pitch delta, got {}",
            delta_p3
        );
        assert_eq!(delta_p3, -delta_p1);
    }

    #[test]
    fn test_tremolo_volume_delta_range() {
        let mut lfo = SoftwareLfo::new();
        lfo.set_tremolo(15, 1);

        for pos in 0..64 {
            lfo.tremolo_pos = pos;
            let vol_delta = lfo.tremolo_volume_delta();
            assert!(
                vol_delta <= 63,
                "Tremolo volume delta should be within 0..63, got {}",
                vol_delta
            );
        }
    }

    #[test]
    fn test_delay_and_linear_fade() {
        let mut lfo = SoftwareLfo::new();
        lfo.set_vibrato(15, 1);
        lfo.set_delay_params(2, 0, 2);
        lfo.retrigger();
        lfo.vibrato_pos = 8;

        assert_eq!(lfo.vibrato_pitch_delta(), 0);
        lfo.clock_tick();
        assert_eq!(lfo.vibrato_pitch_delta(), 0);
        lfo.clock_tick();
        assert_eq!(lfo.vibrato_pitch_delta(), 0);
        lfo.clock_tick();
        let half = lfo.vibrato_pitch_delta();
        assert!(half > 0);
        lfo.clock_tick();
        assert!(lfo.vibrato_pitch_delta() >= half);
    }

    #[test]
    fn test_tremolo_delay_and_fade_are_independent() {
        let mut lfo = SoftwareLfo::new();
        lfo.set_tremolo(15, 1);
        lfo.set_delay_params(0, 2, 2);
        lfo.retrigger();

        assert_eq!(lfo.tremolo_volume_delta(), 0);
        lfo.clock_tick();
        assert_eq!(lfo.tremolo_volume_delta(), 0);
        lfo.clock_tick();
        assert_eq!(lfo.tremolo_volume_delta(), 0);
        lfo.clock_tick();
        let partial = lfo.tremolo_volume_delta();
        lfo.clock_tick();
        assert!(lfo.tremolo_volume_delta() >= partial);
    }

    #[test]
    fn test_delay_speed_is_captured_on_retrigger() {
        let mut lfo = SoftwareLfo::new();
        lfo.set_vibrato(15, 1);
        lfo.set_delay_params(0, 0, 4);
        lfo.retrigger();
        lfo.set_delay_params(0, 0, 1);

        lfo.vibrato_pos = 8;
        lfo.clock_tick();
        let first = lfo.vibrato_pitch_delta();
        lfo.clock_tick();
        let second = lfo.vibrato_pitch_delta();
        assert!(second > first);

        lfo.retrigger();
        lfo.vibrato_pos = 8;
        lfo.clock_tick();
        assert!(lfo.vibrato_pitch_delta() > first);
    }
}
