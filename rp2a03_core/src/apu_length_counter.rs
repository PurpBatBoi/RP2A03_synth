//! rp2a03_core\src\apu_length_counter.rs
//!
//! Adapted from TetaNES: https://github.com/lukexor/tetanes/blob/main/tetanes-core/src/apu/length_counter.rs
//! APU Length Counter implementation.
//! See: <https://www.nesdev.org/wiki/APU_Length_Counter>

#[derive(Debug, Clone)]
pub struct LengthCounter {
    enabled: bool,
    halt: bool,
    new_halt: bool,
    counter: u8,
    previous_counter: u8,
    reload: u8,
}

impl LengthCounter {
    const LENGTH_TABLE: [u8; 32] = [
        10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96,
        22, 192, 24, 72, 26, 16, 28, 32, 30,
    ];

    pub fn new() -> Self {
        Self {
            enabled: false,
            halt: false,
            new_halt: false,
            counter: 0,
            previous_counter: 0,
            reload: 0,
        }
    }

    /// Returns the current counter value. 0 means the channel should be silenced.
    #[inline]
    pub fn counter(&self) -> u8 {
        self.counter
    }

    /// Returns true if the counter is greater than zero.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.counter > 0
    }

    /// Called from the channel control register write (D5 = halt/loop flag).
    #[inline]
    pub fn write_ctrl(&mut self, halt: bool) {
        self.new_halt = halt;
    }

    /// Called from the channel's timer-hi register write.
    /// `val` is the 5-bit length counter load index (D7..D3 of the register, already shifted).
    #[inline]
    pub fn write(&mut self, val: u8) {
        if self.enabled {
            self.reload = Self::LENGTH_TABLE[val as usize];
            self.previous_counter = self.counter;
        }
    }

    /// Called to apply a pending reload. Should be called at the start of each
    /// APU cycle before other clocking, matching TetaNES behavior.
    #[inline]
    pub fn reload(&mut self) {
        if self.reload > 0 {
            if self.counter == self.previous_counter {
                self.counter = self.reload;
            }
            self.reload = 0;
        }
        self.halt = self.new_halt;
    }

    /// Enable or disable this length counter (from $4015).
    /// Disabling immediately zeroes the counter.
    #[inline]
    pub fn set_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.counter = 0;
        }
        self.enabled = enabled;
    }

    /// Clock the length counter (called on half-frame).
    /// Decrements the counter if it's > 0 and not halted.
    pub fn clock(&mut self) {
        if self.counter > 0 && !self.halt {
            self.counter -= 1;
        }
    }

    /// Reset all state.
    pub fn reset(&mut self) {
        self.enabled = false;
        self.halt = false;
        self.new_halt = false;
        self.counter = 0;
        self.previous_counter = 0;
        self.reload = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_counter_is_inactive() {
        let lc = LengthCounter::new();
        assert_eq!(lc.counter(), 0);
        assert!(!lc.is_active());
    }

    #[test]
    fn write_loads_from_length_table_when_enabled() {
        let mut lc = LengthCounter::new();
        lc.set_enabled(true);
        lc.write(0); // LENGTH_TABLE[0] = 10
        lc.reload();
        assert_eq!(lc.counter(), 10);
        assert!(lc.is_active());
    }

    #[test]
    fn write_does_nothing_when_disabled() {
        let mut lc = LengthCounter::new();
        // enabled is false by default
        lc.write(0);
        lc.reload();
        assert_eq!(lc.counter(), 0);
    }

    #[test]
    fn clock_decrements_counter() {
        let mut lc = LengthCounter::new();
        lc.set_enabled(true);
        lc.write(0); // loads 10
        lc.reload();

        lc.clock();
        assert_eq!(lc.counter(), 9);
    }

    #[test]
    fn clock_does_not_decrement_when_halted() {
        let mut lc = LengthCounter::new();
        lc.set_enabled(true);
        lc.write(0); // loads 10
        lc.reload();

        lc.write_ctrl(true); // halt = true
        lc.reload(); // apply the new halt
        lc.clock();
        assert_eq!(lc.counter(), 10); // unchanged
    }

    #[test]
    fn clock_stops_at_zero() {
        let mut lc = LengthCounter::new();
        lc.set_enabled(true);
        lc.write(3); // LENGTH_TABLE[3] = 2
        lc.reload();

        lc.clock(); // 2 -> 1
        lc.clock(); // 1 -> 0
        lc.clock(); // stays 0
        assert_eq!(lc.counter(), 0);
        assert!(!lc.is_active());
    }

    #[test]
    fn disable_zeroes_counter_immediately() {
        let mut lc = LengthCounter::new();
        lc.set_enabled(true);
        lc.write(0); // loads 10
        lc.reload();
        assert_eq!(lc.counter(), 10);

        lc.set_enabled(false);
        assert_eq!(lc.counter(), 0);
    }

    #[test]
    fn reset_clears_all_state() {
        let mut lc = LengthCounter::new();
        lc.set_enabled(true);
        lc.write(0);
        lc.reload();
        lc.reset();
        assert_eq!(lc.counter(), 0);
        assert!(!lc.is_active());
    }

    #[test]
    fn length_table_spot_check() {
        let mut lc = LengthCounter::new();
        lc.set_enabled(true);

        // Spot-check several entries from the standard NES length table
        lc.write(1); // LENGTH_TABLE[1] = 254
        lc.reload();
        assert_eq!(lc.counter(), 254);

        lc.write(4); // LENGTH_TABLE[4] = 40
        lc.previous_counter = lc.counter; // simulate fresh write
        lc.reload();
        // reload only applies if counter == previous_counter, so let's do a clean write
        let mut lc2 = LengthCounter::new();
        lc2.set_enabled(true);
        lc2.write(4);
        lc2.reload();
        assert_eq!(lc2.counter(), 40);
    }
}
