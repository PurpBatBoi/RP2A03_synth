//! rp2a03_core\src\apu_envelope.rs
//!
//! Code adapted from TetaNES code: https://github.com/lukexor/tetanes/blob/main/tetanes-core/src/apu/envelope.rs
//!
//! APU Envelope implementation.
//!
//! See: <https://www.nesdev.org/wiki/APU_Envelope>

#[derive(Debug, Clone)]
pub struct Envelope {
    start: bool,
    constant_volume: bool,
    volume: u8,
    divider: u8,
    counter: u8,
    loops: bool,
}

impl Envelope {
    pub fn new() -> Self {
        Self {
            start: false,
            constant_volume: false,
            volume: 0,
            divider: 0,
            counter: 0,
            loops: false,
        }
    }

    #[inline]
    pub fn volume(&self) -> u8 {
        if self.constant_volume {
            self.volume
        } else {
            self.counter
        }
    }

    #[inline]
    pub fn restart(&mut self) {
        self.start = true;
    }

    /// $4000/$4004/$400C Envelope control
    #[inline]
    pub fn write_ctrl(&mut self, val: u8) {
        self.loops = (val & 0x20) == 0x20; // D5
        self.constant_volume = (val & 0x10) == 0x10; // D4
        self.volume = val & 0x0F; // D3..D0
    }

    pub fn clock(&mut self) {
        if self.start {
            self.start = false;
            self.counter = 15;
            self.divider = self.volume;
        } else if self.divider > 0 {
            self.divider -= 1;
        } else {
            self.divider = self.volume;
            if self.counter > 0 {
                self.counter -= 1;
            } else if self.loops {
                self.counter = 15;
            }
        }
    }

    pub fn reset(&mut self) {
        self.start = false;
        self.constant_volume = false;
        self.volume = 0;
        self.divider = 0;
        self.counter = 0;
        self.loops = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_envelope_has_zero_volume() {
        let env = Envelope::new();
        assert_eq!(env.volume(), 0);
    }

    #[test]
    fn constant_volume_returns_volume_field() {
        let mut env = Envelope::new();
        env.write_ctrl(0x10 | 0x0A); // constant_volume = true, volume = 10
        assert_eq!(env.volume(), 0x0A);
    }

    #[test]
    fn non_constant_volume_returns_counter() {
        let mut env = Envelope::new();
        env.write_ctrl(0x05); // constant_volume = false, volume = 5
        // counter starts at 0 before any clocking
        assert_eq!(env.volume(), 0);
    }

    #[test]
    fn restart_then_clock_sets_counter_to_15() {
        let mut env = Envelope::new();
        env.write_ctrl(0x05); // volume/period = 5
        env.restart();
        env.clock();
        // After restart + clock: counter = 15, divider = volume
        assert_eq!(env.volume(), 15);
    }

    #[test]
    fn clock_decrements_divider_then_counter() {
        let mut env = Envelope::new();
        env.write_ctrl(0x02); // volume/period = 2
        env.restart();
        env.clock(); // start: counter=15, divider=2

        env.clock(); // divider 2 -> 1
        assert_eq!(env.volume(), 15);

        env.clock(); // divider 1 -> 0
        assert_eq!(env.volume(), 15);

        env.clock(); // divider=0 -> reload to 2, counter 15 -> 14
        assert_eq!(env.volume(), 14);
    }

    #[test]
    fn looping_wraps_counter_from_zero_to_15() {
        let mut env = Envelope::new();
        env.write_ctrl(0x20 | 0x00); // loops = true, volume/period = 0
        env.restart();
        env.clock(); // start: counter=15, divider=0

        // With period=0, every clock reloads divider and decrements counter
        for _ in 0..15 {
            env.clock(); // counter goes 15 -> 14 -> ... -> 0
        }
        assert_eq!(env.volume(), 0);

        env.clock(); // loops: counter wraps 0 -> 15
        assert_eq!(env.volume(), 15);
    }

    #[test]
    fn non_looping_stops_at_zero() {
        let mut env = Envelope::new();
        env.write_ctrl(0x00); // loops = false, volume/period = 0
        env.restart();
        env.clock(); // start: counter=15, divider=0

        for _ in 0..15 {
            env.clock();
        }
        assert_eq!(env.volume(), 0);

        env.clock(); // should stay at 0
        assert_eq!(env.volume(), 0);
    }

    #[test]
    fn reset_clears_all_state() {
        let mut env = Envelope::new();
        env.write_ctrl(0x3F); // set everything
        env.restart();
        env.clock();
        env.reset();
        assert_eq!(env.volume(), 0);
    }
}
