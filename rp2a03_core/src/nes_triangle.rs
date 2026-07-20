//! rp2a03_core\src\nes_triangle.rs
//! NES 2A03 Triangle channel.

use crate::nes_core::{ApuTimer, LengthCounter};

const TRIANGLE_SEQUENCE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];

#[derive(Default)]
pub struct TriangleChannel {
    pub length_counter: LengthCounter,
    timer: ApuTimer,

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

    pub fn write_reg0(&mut self, value: u8) {
        self.linear_control_flag = (value & 0x80) != 0;
        self.linear_counter_reload = value & 0x7F;
        self.length_counter.set_halt_pending(self.linear_control_flag);
        self.update_output();
    }

    pub fn write_reg2(&mut self, value: u8) {
        let period = (self.timer.get_period() & 0xFF00) | (value as u16);
        self.timer.set_period(period);
        self.update_output();
    }

    pub fn write_reg3(&mut self, value: u8) {
        self.length_counter.load(value >> 3);
        let period = (self.timer.get_period() & 0x00FF) | (((value & 0x07) as u16) << 8);
        self.timer.set_period(period);
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

    pub fn end_frame(&mut self) {
        // No-op: the per-cycle tick() timer has no cross-buffer state to reset.
    }

    /// Advance the triangle channel by exactly one CPU cycle.
    pub fn clock(&mut self) {
        // Sequencer is clocked if both length and linear counters are non-zero.
        // `tick()` runs first unconditionally (short-circuit `&&` still evaluates
        // the left side), so the timer always advances even when gated.
        if self.timer.tick() && self.length_counter.status() && self.linear_counter > 0 {
            self.sequence_position = self.sequence_position.wrapping_add(1) & 0x1F;
            self.update_output();
        }
    }
}