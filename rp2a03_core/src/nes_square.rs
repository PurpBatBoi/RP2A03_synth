//! rp2a03_core\src\nes_square.rs
//! NES 2A03 Square/Pulse channel.

use crate::nes_core::{ApuTimer, Envelope};

const DUTY_SEQUENCES: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1],
    [0, 0, 0, 0, 0, 0, 1, 1],
    [0, 0, 0, 0, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 0, 0],
];

#[derive(Default, Clone, Debug)]
pub struct SquareChannel {
    pub envelope: Envelope,
    is_channel1: bool,
    timer: ApuTimer,
    duty: u8,
    duty_pos: u8,

    // Sweep unit
    sweep_enabled: bool,
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    reload_sweep: bool,
    sweep_divider: u8,
    sweep_target_period: u16,
    real_period: u16,

    pub output: u8,
    pending_delta: i16,
}

impl SquareChannel {
    pub fn new(is_channel1: bool) -> Self {
        Self {
            envelope: Envelope::default(),
            is_channel1,
            timer: ApuTimer::new(),
            duty: 0,
            duty_pos: 0,
            sweep_enabled: false,
            sweep_period: 0,
            sweep_negate: false,
            sweep_shift: 0,
            reload_sweep: false,
            sweep_divider: 0,
            sweep_target_period: 0,
            real_period: 0,
            output: 0,
            pending_delta: 0,
        }
    }

    pub fn take_delta(&mut self) -> i16 {
        let d = self.pending_delta;
        self.pending_delta = 0;
        d
    }

    fn is_muted(&self) -> bool {
        self.real_period < 8 || (!self.sweep_negate && self.sweep_target_period > 0x7FF)
    }

    fn update_target_period(&mut self) {
        let shift_result = self.real_period >> self.sweep_shift;
        if self.sweep_negate {
            self.sweep_target_period = self.real_period.wrapping_sub(shift_result);
            if self.is_channel1 {
                self.sweep_target_period = self.sweep_target_period.wrapping_sub(1);
            }
        } else {
            self.sweep_target_period = self.real_period + shift_result;
        }
    }

    fn set_period(&mut self, new_period: u16) {
        self.real_period = new_period;
        self.timer.set_period((self.real_period * 2) + 1);
        self.update_target_period();
    }

    fn update_output(&mut self) {
        let new_output = if self.is_muted() {
            0
        } else {
            DUTY_SEQUENCES[self.duty as usize][self.duty_pos as usize] * self.envelope.volume()
        };
        if new_output != self.output {
            self.pending_delta += new_output as i16 - self.output as i16;
            self.output = new_output;
        }
    }

    // --- Register writes ---

    pub fn write_reg0(&mut self, value: u8) {
        self.envelope.init(value);
        self.duty = (value & 0xC0) >> 6;
        self.update_output();
    }

    pub fn write_reg1(&mut self, value: u8) {
        self.sweep_enabled = (value & 0x80) != 0;
        self.sweep_negate = (value & 0x08) != 0;
        self.sweep_period = ((value & 0x70) >> 4) + 1;
        self.sweep_shift = value & 0x07;
        self.update_target_period();
        self.reload_sweep = true;
        self.update_output();
    }

    pub fn write_reg2(&mut self, value: u8) {
        self.set_period((self.real_period & 0x0700) | value as u16);
        self.update_output();
    }

    pub fn write_reg3(&mut self, value: u8) {
        self.envelope.length_counter.load(value >> 3);
        self.set_period((self.real_period & 0xFF) | (((value & 0x07) as u16) << 8));
        self.duty_pos = 0;
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

    pub fn tick_sweep(&mut self) {
        self.sweep_divider = self.sweep_divider.wrapping_sub(1);
        if self.sweep_divider == 0 {
            if self.sweep_shift > 0
                && self.sweep_enabled
                && self.real_period >= 8
                && self.sweep_target_period <= 0x7FF
            {
                let target = self.sweep_target_period;
                self.set_period(target);
            }
            self.sweep_divider = self.sweep_period;
        }

        if self.reload_sweep {
            self.sweep_divider = self.sweep_period;
            self.reload_sweep = false;
        }
        self.update_output();
    }

    pub fn end_frame(&mut self) {
        self.timer.end_frame();
    }

    /// Run the square channel up to target_cycle
    pub fn run(&mut self, target_cycle: u32) {
        // The closure passed to `timer.run` can't borrow `self` while `self.timer`
        // is already mutably borrowed, so we mutate a local copy of `duty_pos`
        // and write it back (and update the output) once the timer call returns.
        let mut duty_pos = self.duty_pos;
        let mut expired = false;

        self.timer.run(target_cycle, || {
            duty_pos = duty_pos.wrapping_sub(1) & 0x07;
            expired = true;
        });

        self.duty_pos = duty_pos;
        if expired {
            self.update_output();
        }
    }

    /// Simplified single-cycle clock
    pub fn clock(&mut self) {
        let prev = self.timer.get_timer();
        self.run(prev as u32 + 1);
    }
}