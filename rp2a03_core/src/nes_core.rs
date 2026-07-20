//! rp2a03_core\src\nes_core.rs
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

/// APU Timer - matches C++ ApuTimer::Run() behavior exactly
#[derive(Default, Clone, Debug)]
pub struct ApuTimer {
    timer: u16,
    period: u16,
    previous_cycle: u32,
}

impl ApuTimer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_period(&mut self, period: u16) {
        self.period = period;
    }

    pub fn get_period(&self) -> u16 {
        self.period
    }

    pub fn get_timer(&self) -> u16 {
        self.timer
    }

    pub fn set_timer(&mut self, timer: u16) {
        self.timer = timer;
    }

    pub fn end_frame(&mut self) {
        self.previous_cycle = 0;
    }

    pub fn reset(&mut self) {
        self.timer = 0;
        self.period = 0;
        self.previous_cycle = 0;
    }

    /// Run the timer up to target_cycle, calling the provided closure
    /// each time the timer expires. Returns whether any expirations occurred.
    ///
    /// Matches C++ ApuTimer::Run() behavior:
    /// ```cpp
    /// while(cyclesToRun > _timer) {
    ///     _previousCycle += _timer + 1;
    ///     _timer = _period;
    ///     // [action happens here]
    ///     return true;
    /// }
    /// _timer -= cyclesToRun;
    /// _previousCycle = targetCycle;
    /// return false;
    /// ```
    pub fn run<F>(&mut self, target_cycle: u32, mut on_expire: F) -> bool
    where
        F: FnMut(),
    {
        let mut cycles_to_run = (target_cycle - self.previous_cycle) as i32;
        let mut expired = false;

        while cycles_to_run > 0 {
            if cycles_to_run > self.timer as i32 {
                // Timer expired
                self.previous_cycle += (self.timer + 1) as u32;
                self.timer = self.period;
                
                on_expire();
                expired = true;

                cycles_to_run = (target_cycle - self.previous_cycle) as i32;
            } else {
                // Timer didn't expire
                self.timer -= cycles_to_run as u16;
                self.previous_cycle = target_cycle;
                cycles_to_run = 0;
            }
        }

        expired
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