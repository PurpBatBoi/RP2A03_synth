//! nes_noise.rs
//! NES 2A03 Noise channel.
//! 
//! Uses a 15-bit Linear Feedback Shift Register (LFSR) driven by a 
//! lookup table of CPU cycle periods to generate pseudo-random noise.

use crate::nes_core::Envelope; // Adjust import path as needed

// NTSC period lookup table. Note: the actual period set to the timer is value - 1.
const NOISE_PERIOD_NTSC: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

pub struct NoiseChannel {
    pub envelope: Envelope,
    timer: u16,
    period: u16,
    
    /// Must be initialized to 1 at power-up/reset to avoid silence.
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
            period: NOISE_PERIOD_NTSC[0] - 1,
            shift_register: 1, // Vital: LFSR initialized to 1
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
        // Muted if bit 0 of shift register is 1, or length counter is exhausted.
        (self.shift_register & 0x01) == 0x01 || !self.envelope.length_counter.status()
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

    /// 0x400C: Envelope init / Length counter halt
    pub fn write_reg0(&mut self, value: u8) {
        self.envelope.init(value);
        self.update_output();
    }

    /// 0x400E: Mode flag + Period index
    pub fn write_reg2(&mut self, value: u8) {
        let period_index = (value & 0x0F) as usize;
        self.period = NOISE_PERIOD_NTSC[period_index] - 1;
        self.mode_flag = (value & 0x80) != 0;
        self.update_output();
    }

    /// 0x400F: Length counter load + Envelope restart
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

    /// Clock the timer by one NES CPU cycle.
    pub fn clock(&mut self) {
        if self.timer == 0 {
            self.timer = self.period;

            // Feedback is the exclusive-OR of bit 0 and one other bit.
            // If mode flag is set, it's bit 6; otherwise, bit 1.
            let feedback_bit = if self.mode_flag { 6 } else { 1 };
            let feedback = (self.shift_register & 0x01) ^ ((self.shift_register >> feedback_bit) & 0x01);
            
            self.shift_register >>= 1;
            self.shift_register |= feedback << 14;

            self.update_output();
        } else {
            self.timer -= 1;
        }
    }
}