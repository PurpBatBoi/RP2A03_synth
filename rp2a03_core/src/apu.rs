//! rp2a03_core\src\apu.rs
//!
//! Combined APU implementations: Envelope, Frame Counter, Length Counter, and Timer.
//! Adapted from TetaNES: https://github.com/lukexor/tetanes/tree/main/tetanes-core/src/apu
//!
//! See: <https://www.nesdev.org/wiki/APU>

// =============================================================================
// Envelope
// =============================================================================
//
// $4000/$4004/$400C Envelope control
// See: <https://www.nesdev.org/wiki/APU_Envelope>
pub mod envelope {
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
}

// =============================================================================
// Frame Counter
// =============================================================================
//
// APU Frame Counter generates low-frequency clocks for APU channel units (NTSC only).
// See: <https://www.nesdev.org/wiki/APU_Frame_Counter>
pub mod frame_counter {
    /// The type of frame event generated by the frame counter.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum FrameType {
        None,
        Quarter,
        Half,
    }

    /// APU Frame Counter generates low-frequency clocks for APU channel units.
    ///
    /// See: <https://www.nesdev.org/wiki/APU_Frame_Counter>
    #[derive(Debug, Clone)]
    pub struct FrameCounter {
        step_cycles: [u32; 6],
        step: usize,
        mode: u8,
        write_buffer: Option<u8>,
        write_delay: u8,
        block_counter: u8,
        cycle: u32,
        inhibit_irq: bool,
        irq_pending: bool,
    }

    impl FrameCounter {
        // NTSC cycle counts for 4-step and 5-step modes
        const STEP4_CYCLES: [u32; 6] = [7457, 14913, 22371, 29828, 29829, 29830];
        const STEP5_CYCLES: [u32; 6] = [7457, 14913, 22371, 29829, 37281, 37282];

        const FRAME_TYPE: [FrameType; 6] = [
            FrameType::Quarter,
            FrameType::Half,
            FrameType::Quarter,
            FrameType::None,
            FrameType::Half,
            FrameType::None,
        ];

        pub fn new() -> Self {
            Self {
                step_cycles: Self::STEP4_CYCLES,
                step: 0,
                mode: 0,
                write_buffer: None,
                write_delay: 0,
                block_counter: 0,
                cycle: 0,
                inhibit_irq: false,
                irq_pending: false,
            }
        }

        /// Returns whether an IRQ is pending.
        #[inline]
        pub fn irq_pending(&self) -> bool {
            self.irq_pending
        }

        /// Acknowledges (clears) a pending IRQ.
        #[inline]
        pub fn acknowledge_irq(&mut self) {
            self.irq_pending = false;
        }

        fn step_cycles_for_mode(mode: u8) -> [u32; 6] {
            if mode == 0 {
                Self::STEP4_CYCLES
            } else {
                Self::STEP5_CYCLES
            }
        }

        /// On write to $4017
        pub fn write(&mut self, val: u8, cycle: u32) {
            self.write_buffer = Some(val);
            // Writes occurring on odd clocks are delayed
            self.write_delay = if cycle & 0x01 == 0x01 { 4 } else { 3 };
            self.inhibit_irq = val & 0x40 == 0x40; // D6
            if self.inhibit_irq {
                self.irq_pending = false;
            }
        }

        /// Returns true if the frame counter needs to be clocked this cycle.
        #[inline]
        pub fn should_clock(&self, cycles: u32) -> bool {
            self.block_counter > 0
                || self.write_buffer.is_some()
                || (self.cycle + cycles) >= (self.step_cycles[self.step] - 1)
        }

        // mode 0: 4-step  effective rate (approx)
        // ---------------------------------------
        // - - - f f f      60 Hz
        // - l - - l -     120 Hz
        // e e e - e -     240 Hz
        //
        // mode 1: 5-step  effective rate (approx)
        // ---------------------------------------
        // - - - - - -     (interrupt flag never set)
        // - l - - l -     96 Hz
        // e e e - e -     192 Hz
        pub fn clock_with(&mut self, cycles: u32, mut on_clock: impl FnMut(FrameType)) -> u32 {
            let mut cycles_ran = 0;
            let step_cycles = self.step_cycles[self.step];

            if self.cycle + cycles >= step_cycles {
                if !self.inhibit_irq && self.mode == 0 && self.step >= 3 {
                    self.irq_pending = true;
                }

                let ty = Self::FRAME_TYPE[self.step];
                if ty != FrameType::None && self.block_counter == 0 {
                    on_clock(ty);
                    // Do not allow writes to $4017 to clock for the next cycle
                    // (odd + following even cycle)
                    self.block_counter = 2;
                }

                if step_cycles >= self.cycle {
                    cycles_ran = step_cycles - self.cycle;
                }

                self.step += 1;
                if self.step == 6 {
                    self.step = 0;
                    self.cycle = 0;
                } else {
                    self.cycle += cycles_ran;
                }
            } else {
                cycles_ran = cycles;
                self.cycle += cycles_ran;
            }

            if let Some(val) = self.write_buffer {
                self.write_delay -= 1;
                if self.write_delay == 0 {
                    self.mode = if val & 0x80 == 0x80 { 1 } else { 0 };
                    self.step_cycles = Self::step_cycles_for_mode(self.mode);
                    self.step = 0;
                    self.cycle = 0;
                    self.write_buffer = None;
                    if self.mode == 1 && self.block_counter == 0 {
                        // Writing to $4017 with bit 7 set will immediately
                        // generate a quarter/half frame
                        on_clock(FrameType::Half);
                        self.block_counter = 2;
                    }
                }
            }

            if self.block_counter > 0 {
                self.block_counter -= 1;
            }

            cycles_ran
        }

        /// Reset all state.
        pub fn reset(&mut self) {
            self.step_cycles = Self::STEP4_CYCLES;
            self.step = 0;
            self.mode = 0;
            self.write_buffer = None;
            self.write_delay = 0;
            self.block_counter = 0;
            self.cycle = 0;
            self.inhibit_irq = false;
            self.irq_pending = false;
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn new_frame_counter_defaults() {
            let fc = FrameCounter::new();
            assert!(!fc.irq_pending());
            assert_eq!(fc.mode, 0);
            assert_eq!(fc.step, 0);
            assert_eq!(fc.cycle, 0);
        }

        #[test]
        fn write_sets_irq_inhibit() {
            let mut fc = FrameCounter::new();
            fc.write(0x40, 0); // D6 = inhibit IRQ
            assert!(fc.inhibit_irq);
        }

        #[test]
        fn write_with_mode_1_triggers_immediate_half_frame() {
            let mut fc = FrameCounter::new();
            fc.write(0x80, 0); // D7 = mode 1

            let mut frame_type_received = FrameType::None;
            // Clock enough times for the write delay to expire
            for _ in 0..4 {
                fc.clock_with(1, |ft| {
                    frame_type_received = ft;
                });
            }
            assert_eq!(frame_type_received, FrameType::Half);
        }

        #[test]
        fn quarter_frame_fires_at_correct_cycle() {
            let mut fc = FrameCounter::new();
            let mut received = Vec::new();

            // Clock up to just past the first step boundary (7457 cycles)
            let mut total = 0;
            while total <= 7457 {
                fc.clock_with(1, |ft| {
                    received.push(ft);
                });
                total += 1;
            }

            assert!(received.contains(&FrameType::Quarter));
        }

        #[test]
        fn mode_0_generates_irq() {
            let mut fc = FrameCounter::new();
            // Don't inhibit IRQ
            assert!(!fc.inhibit_irq);

            // Clock through all steps of mode 0 until step >= 3
            let mut total = 0u32;
            while total < 30000 {
                fc.clock_with(1, |_| {});
                total += 1;
            }

            assert!(fc.irq_pending());
        }

        #[test]
        fn inhibit_irq_prevents_irq() {
            let mut fc = FrameCounter::new();
            fc.write(0x40, 0); // inhibit IRQ

            // Process write delay
            for _ in 0..4 {
                fc.clock_with(1, |_| {});
            }

            // Clock through entire mode 0 sequence
            let mut total = 0u32;
            while total < 30000 {
                fc.clock_with(1, |_| {});
                total += 1;
            }

            assert!(!fc.irq_pending());
        }

        #[test]
        fn reset_clears_state() {
            let mut fc = FrameCounter::new();
            fc.write(0xC0, 0);
            fc.reset();
            assert_eq!(fc.mode, 0);
            assert!(!fc.irq_pending());
            assert!(!fc.inhibit_irq);
        }
    }
}

// =============================================================================
// Length Counter
// =============================================================================
//
// APU Length Counter implementation.
// See: <https://www.nesdev.org/wiki/APU_Length_Counter>
pub mod length_counter {
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
            10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20,
            96, 22, 192, 24, 72, 26, 16, 28, 32, 30,
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
}

// =============================================================================
// Timer
// =============================================================================
//
// Timer abstraction for APU units. The timer is clocked every (period + 1)
// cycles.
pub mod timer {
    /// Trait for types that have timers.
    pub trait TimerCycle {
        fn cycle(&self) -> u32;
    }

    /// A timer that generates a clock signal based on a divider and a period. The timer is clocked
    /// every (period + 1) * divider cycles.
    #[derive(Default, Debug, Clone)]
    #[must_use]
    pub struct Timer {
        pub cycle: u32,
        pub counter: u16,
        pub period: u16,
    }

    impl Timer {
        pub const fn new(period: u16) -> Self {
            Self {
                cycle: 0,
                counter: 0,
                period,
            }
        }

        pub const fn preload(period: u16) -> Self {
            let mut timer = Self::new(period);
            timer.counter = timer.period;
            timer
        }

        pub const fn reload(&mut self) {
            self.counter = self.period;
        }

        pub const fn tick(&mut self) -> bool {
            self.cycle += 1;
            if self.counter == 0 {
                self.counter = self.period;
                return true;
            }
            self.counter -= 1;
            false
        }
    }

    impl Timer {
        pub fn reset(&mut self) {
            self.counter = 0;
            self.period = 0;
            self.cycle = 0;
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn timer() {
            // Period (10 + 1) == 11 + initial clock
            let mut timer = Timer::new(10);
            let mut expected = [false; 23];
            expected[0] = true;
            expected[11] = true;
            expected[22] = true;
            assert_eq!(expected, [(); 23].map(|_| timer.tick()));
            assert_eq!(23, timer.cycle);

            // Period (10 + 1) == 11
            let mut timer = Timer::preload(10);
            let mut expected = [false; 22];
            expected[10] = true;
            expected[21] = true;
            assert_eq!(expected, [(); 22].map(|_| timer.tick()));
            assert_eq!(22, timer.cycle);

            // Period (10 * 2) + 1 == 22 + initial clock
            let mut timer = Timer::new((10 * 2) + 1);
            let mut expected = [false; 45];
            expected[0] = true;
            expected[22] = true;
            expected[44] = true;
            assert_eq!(expected, [(); 45].map(|_| timer.tick()));
            assert_eq!(45, timer.cycle);

            // Period (10 * 2) + 1 == 22
            let mut timer = Timer::preload((10 * 2) + 1);
            let mut expected = [false; 44];
            expected[21] = true;
            expected[43] = true;
            assert_eq!(expected, [(); 44].map(|_| timer.tick()));
            assert_eq!(44, timer.cycle);
        }
    }
}

// Re-exports for convenient access, e.g. `apu::Envelope`, `apu::Timer`, etc.
pub use envelope::Envelope;
pub use frame_counter::{FrameCounter, FrameType};
pub use length_counter::LengthCounter;
pub use timer::{Timer, TimerCycle};
