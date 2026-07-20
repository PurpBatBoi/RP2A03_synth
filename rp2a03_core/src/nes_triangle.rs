//! nes_triangle.rs
//! NES 2A03 Triangle channel.
//! 
//! The triangle channel produces a 32-step pseudo-triangle wave.
//! Unlike pulse channels, it operates at CPU speed (not CPU/2), 
//! and volume is not envelope-controlled (it is purely on or off based on sequence).

use crate::nes_core::LengthCounter; // Adjust import path as needed

const TRIANGLE_SEQUENCE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];

#[derive(Default)]
pub struct TriangleChannel {
    pub length_counter: LengthCounter,
    timer: u16,
    period: u16,

    linear_counter: u8,
    linear_counter_reload: u8,
    linear_reload_flag: bool,
    linear_control_flag: bool,

    sequence_position: u8,
    pub output: u8,
    pending_delta: i16,
}

impl TriangleChannel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn take_delta(&mut self) -> i16 {
        let d = self.pending_delta;
        self.pending_delta = 0;
        d
    }

    fn update_output(&mut self) {
        let new_output = TRIANGLE_SEQUENCE[self.sequence_position as usize];
        if new_output != self.output {
            self.pending_delta += new_output as i16 - self.output as i16;
            self.output = new_output;
        }
    }

    // --- Register writes ---

    /// 0x4008: Linear counter load + control flag (which maps to length counter halt)
    pub fn write_reg0(&mut self, value: u8) {
        self.linear_control_flag = (value & 0x80) != 0;
        self.linear_counter_reload = value & 0x7F;
        self.length_counter.set_halt_pending(self.linear_control_flag);
        self.update_output();
    }

    /// 0x400A: Timer low 8 bits
    pub fn write_reg2(&mut self, value: u8) {
        self.period = (self.period & 0xFF00) | (value as u16);
        self.update_output();
    }

    /// 0x400B: Length counter load + Timer high 3 bits
    pub fn write_reg3(&mut self, value: u8) {
        self.length_counter.load(value >> 3);
        self.period = (self.period & 0x00FF) | (((value & 0x07) as u16) << 8);
        self.linear_reload_flag = true;
        self.update_output();
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.length_counter.set_enabled(enabled);
    }

    pub fn status(&self) -> bool {
        self.length_counter.status()
    }

    // --- Frame sequencer callbacks ---

    pub fn tick_linear_counter(&mut self) {
        if self.linear_reload_flag {
            self.linear_counter = self.linear_counter_reload;
        } else if self.linear_counter > 0 {
            self.linear_counter -= 1;
        }

        if !self.linear_control_flag {
            self.linear_reload_flag = false;
        }
        self.update_output();
    }

    pub fn tick_length_counter(&mut self) {
        self.length_counter.tick();
        self.update_output();
    }

    pub fn reload_length_counter(&mut self) {
        self.length_counter.reload();
        self.update_output();
    }

    /// Clock the timer by one NES CPU cycle.
    pub fn clock(&mut self) {
        if self.timer == 0 {
            self.timer = self.period;

            // Sequencer is clocked if both length and linear counters are non-zero.
            if self.length_counter.status() && self.linear_counter > 0 {
                self.sequence_position = self.sequence_position.wrapping_add(1) & 0x1F;
                self.update_output();
            }
        } else {
            self.timer -= 1;
        }
    }
}