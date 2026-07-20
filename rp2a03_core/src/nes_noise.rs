//! nes_noise.rs
//! NES 2A03 Noise channel (faithful to puNES / hardware behavior).

use crate::nes_core::Envelope;

const NOISE_PERIOD_NTSC: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

pub struct NoiseChannel {
    pub envelope: Envelope,
    timer: u16,
    period: u16,
    shift_register: u16,
    mode_flag: bool,
    pub output: u8,
    pending_delta: i16,
}

impl Default for NoiseChannel {
    fn default() -> Self {
        Self {
            envelope: Envelope::default(),
            timer: 0,
            period: NOISE_PERIOD_NTSC[0],        // ← no -1
            shift_register: 1,
            mode_flag: false,
            output: 0,
            pending_delta: 0,
        }
    }
}

impl NoiseChannel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn take_delta(&mut self) -> i16 {
        let d = self.pending_delta;
        self.pending_delta = 0;
        d
    }

    fn is_muted(&self) -> bool {
        (self.shift_register & 0x01) != 0 || !self.envelope.length_counter.status()
    }

    fn update_output(&mut self) {
        let new_output = if self.is_muted() {
            0
        } else {
            self.envelope.volume()
        };
        if new_output != self.output {
            self.pending_delta += new_output as i16 - self.output as i16;
            self.output = new_output;
        }
    }

    // --- Register writes ---

    pub fn write_reg0(&mut self, value: u8) {
        self.envelope.init(value);
        self.update_output();
    }

    pub fn write_reg2(&mut self, value: u8) {
        let period_index = (value & 0x0F) as usize;
        self.period = NOISE_PERIOD_NTSC[period_index];   // ← direct table value
        self.mode_flag = (value & 0x80) != 0;
        self.update_output();
    }

    pub fn set_mode_flag(&mut self, metallic: bool) {
        self.mode_flag = metallic;
        self.update_output();
    }

    pub fn mode_flag(&self) -> bool {
        self.mode_flag
    }

    pub fn write_reg3(&mut self, value: u8) {
        self.envelope.length_counter.load(value >> 3);
        self.envelope.restart();
        self.update_output();
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.envelope.length_counter.set_enabled(enabled);
        self.update_output();
    }

    pub fn status(&self) -> bool {
        self.envelope.length_counter.status()
    }

    // --- Frame sequencer callbacks ---

    pub fn tick_envelope(&mut self) {
        self.envelope.tick();
        self.update_output();
    }

    pub fn tick_length_counter(&mut self) {
        self.envelope.length_counter.tick();
        self.update_output();
    }

    pub fn reload_length_counter(&mut self) {
        self.envelope.length_counter.reload();
        self.update_output();
    }

    /// Clock the noise timer (called every CPU cycle).
    /// Matches puNES noise_tick() behavior exactly.
    pub fn clock(&mut self) {
        if self.timer == 0 {
            // Perform LFSR feedback (metallic uses bit 6, normal uses bit 1)
            let feedback_bit = if self.mode_flag { 6 } else { 1 };
            let feedback = (self.shift_register & 0x01)
                ^ ((self.shift_register >> feedback_bit) & 0x01);

            self.shift_register = ((self.shift_register >> 1) | (feedback << 14)) & 0x7FFF;

            self.update_output();

            // Reload period *after* the shift (matches puNES)
            self.timer = self.period;
        }
        self.timer -= 1;
    }
}