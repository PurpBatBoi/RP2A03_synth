//! nes_core.rs
//! Core shared components for NES 2A03 APU channels.

pub const NTSC_CPU_CLOCK: f64 = 1_789_773.0;

pub const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

#[derive(Default, Clone, Debug)]
pub struct LengthCounter {
    enabled: bool,
    halt: bool,
    counter: u8,
    reload_value: u8,
    previous_value: u8,
    new_halt_value: bool,
}

impl LengthCounter {
    pub fn set_halt_pending(&mut self, halt: bool) {
        self.new_halt_value = halt;
    }

    pub fn load(&mut self, index: u8) {
        if self.enabled {
            self.reload_value = LENGTH_TABLE[index as usize];
            self.previous_value = self.counter;
        }
    }

    pub fn reload(&mut self) {
        if self.reload_value != 0 {
            if self.counter == self.previous_value {
                self.counter = self.reload_value;
            }
            self.reload_value = 0;
        }
        self.halt = self.new_halt_value;
    }

    pub fn tick(&mut self) {
        if self.counter > 0 && !self.halt {
            self.counter -= 1;
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.counter = 0;
        }
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_halted(&self) -> bool {
        self.halt
    }

    pub fn status(&self) -> bool {
        self.counter > 0
    }
}

#[derive(Default, Clone, Debug)]
pub struct Envelope {
    constant_volume: bool,
    volume: u8,
    start: bool,
    divider: i8,
    counter: u8,
    pub length_counter: LengthCounter,
}

impl Envelope {
    pub fn init(&mut self, reg_value: u8) {
        self.length_counter.set_halt_pending((reg_value & 0x20) != 0);
        self.constant_volume = (reg_value & 0x10) != 0;
        self.volume = reg_value & 0x0F;
    }

    pub fn restart(&mut self) {
        self.start = true;
    }

    pub fn tick(&mut self) {
        if !self.start {
            self.divider -= 1;
            if self.divider < 0 {
                self.divider = self.volume as i8;
                if self.counter > 0 {
                    self.counter -= 1;
                } else if self.length_counter.is_halted() {
                    self.counter = 15;
                }
            }
        } else {
            self.start = false;
            self.counter = 15;
            self.divider = self.volume as i8;
        }
    }

    pub fn volume(&self) -> u8 {
        if self.length_counter.status() {
            if self.constant_volume {
                self.volume
            } else {
                self.counter
            }
        } else {
            0
        }
    }
}

const STEP_CYCLES_NTSC: [u32; 4] = [7457, 14913, 22371, 29830];

pub enum FrameTick {
    None,
    Quarter,
    Half,
}

#[derive(Default, Clone, Debug)]
pub struct FrameSequencer {
    cycle: u32,
    step: usize,
}

impl FrameSequencer {
    pub fn clock(&mut self) -> FrameTick {
        self.cycle += 1;
        if self.cycle >= STEP_CYCLES_NTSC[self.step] {
            let tick = match self.step {
                0 | 2 => FrameTick::Quarter,
                1 | 3 => FrameTick::Half,
                _ => unreachable!(),
            };
            self.step += 1;
            if self.step == 4 {
                self.step = 0;
                self.cycle = 0;
            }
            tick
        } else {
            FrameTick::None
        }
    }
}