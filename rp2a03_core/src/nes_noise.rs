//! rp2a03_core\src\nes_noise.rs
//! NES 2A03 Noise channel (faithful to MesenCE behavior).

use crate::nes_core::{ApuTimer, Envelope};

const NOISE_PERIOD_NTSC: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

pub struct NoiseChannel {
    pub envelope: Envelope,
    timer: ApuTimer,
    shift_register: u16,
    mode_flag: bool,
    pub output: u8,
    pending_delta: i16,
}

impl Default for NoiseChannel {
    fn default() -> Self {
        let mut timer = ApuTimer::new();
        timer.set_period(NOISE_PERIOD_NTSC[0].saturating_sub(1));
        
        Self {
            envelope: Envelope::default(),
            timer,
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
        (self.shift_register & 0x01) == 0x01 || !self.envelope.length_counter.status()
    }

    fn add_output(&mut self, output: u8) {
        if output != self.output {
            self.pending_delta += output as i16 - self.output as i16;
            self.output = output;
        }
    }

    fn update_output(&mut self) {
        if self.is_muted() {
            self.add_output(0);
        } else {
            self.add_output(self.envelope.volume());
        }
    }

    // --- Register writes ---

    pub fn write_reg0(&mut self, value: u8) {
        self.envelope.init(value);
    }

    pub fn write_reg2(&mut self, value: u8) {
        let period_index = (value & 0x0F) as usize;
        self.timer.set_period(NOISE_PERIOD_NTSC[period_index].saturating_sub(1));
        self.mode_flag = (value & 0x80) != 0;
    }

    pub fn set_mode_flag(&mut self, metallic: bool) {
        self.mode_flag = metallic;
    }

    pub fn mode_flag(&self) -> bool {
        self.mode_flag
    }

    pub fn write_reg3(&mut self, value: u8) {
        self.envelope.length_counter.load(value >> 3);
        self.envelope.restart();
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.envelope.length_counter.set_enabled(enabled);
    }

    pub fn status(&self) -> bool {
        self.envelope.length_counter.status()
    }

    // --- Frame sequencer callbacks ---

    pub fn tick_envelope(&mut self) {
        self.envelope.tick();
    }

    pub fn tick_length_counter(&mut self) {
        self.envelope.length_counter.tick();
    }

    pub fn reload_length_counter(&mut self) {
        self.envelope.length_counter.reload();
    }

    pub fn end_frame(&mut self) {
        // No-op: the per-cycle tick() timer has no cross-buffer state to reset.
    }

    /// Advance the noise channel by exactly one CPU cycle.
    pub fn clock(&mut self) {
        if self.timer.tick() {
            // Perform the LFSR feedback
            let feedback_bit = if self.mode_flag { 6 } else { 1 };
            let feedback = (self.shift_register & 0x01)
                ^ ((self.shift_register >> feedback_bit) & 0x01);

            self.shift_register >>= 1;
            self.shift_register |= feedback << 14;

            self.update_output();
        }
    }
}