//! `rp2a03_core\src\apu.rs`
//!
//! Combined APU implementations: Envelope, Length Counter, and Timer.
//! Adapted from `TetaNES`: <https://github.com/lukexor/tetanes/tree/main/tetanes-core/src/apu>
//!
//! See: <https://www.nesdev.org/wiki/APU>

// =============================================================================
// Envelope
// =============================================================================
//
// $4000/$4004/$400C Envelope control
// See: <https://www.nesdev.org/wiki/APU_Envelope>
/// $4000/$4004/$400C envelope generator, clocked on every quarter-frame.
pub mod envelope {
    /// Decaying/constant-volume generator shared by Pulse and Noise.
    #[derive(Debug, Clone)]
    pub struct Envelope {
        start: bool,
        constant_volume: bool,
        volume: u8,
        divider: u8,
        counter: u8,
        loops: bool,
    }

    impl Default for Envelope {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Envelope {
        /// A silent, zeroed envelope.
        #[must_use]
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

        /// Current output level: the decay counter, or the fixed volume when
        /// `constant_volume` is set.
        #[inline]
        #[must_use]
        pub fn volume(&self) -> u8 {
            if self.constant_volume {
                self.volume
            } else {
                self.counter
            }
        }

        /// Flags the envelope to reload on its next `clock()`.
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

        /// Advances one quarter-frame: reloads on `start`, otherwise decrements the
        /// divider and, on underflow, the decay counter (looping if `loops` is set).
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

        /// Resets to power-on state.
        pub fn reset(&mut self) {
            self.start = false;
            self.constant_volume = false;
            self.volume = 0;
            self.divider = 0;
            self.counter = 0;
            self.loops = false;
        }
    }
}

// =============================================================================
// Length Counter
// =============================================================================
//
// APU Length Counter implementation.
// See: <https://www.nesdev.org/wiki/APU_Length_Counter>
/// APU length counter, clocked on every half-frame.
pub mod length_counter {
    /// Silences a channel automatically after a hardware-defined number of frames.
    #[derive(Debug, Clone)]
    pub struct LengthCounter {
        enabled: bool,
        halt: bool,
        new_halt: bool,
        pub(crate) counter: u8,
        pub(crate) previous_counter: u8,
        reload: u8,
    }

    impl Default for LengthCounter {
        fn default() -> Self {
            Self::new()
        }
    }

    impl LengthCounter {
        const LENGTH_TABLE: [u8; 32] = [
            10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20,
            96, 22, 192, 24, 72, 26, 16, 28, 32, 30,
        ];

        /// A disabled, zeroed length counter.
        #[must_use]
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
        #[must_use]
        pub fn counter(&self) -> u8 {
            self.counter
        }

        /// Returns true if the counter is greater than zero.
        #[inline]
        #[must_use]
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
        /// APU cycle before other clocking, matching `TetaNES` behavior.
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
}

// =============================================================================
// Timer
// =============================================================================
//
// Timer abstraction for APU units. The timer is clocked every (period + 1)
// cycles.
/// A divider that generates a clock signal every `(period + 1)` cycles.
pub mod timer {
    /// Trait for types that have timers.
    pub trait TimerCycle {
        /// The type's current cycle count, used to phase-align timers that share a divider.
        fn cycle(&self) -> u32;
    }

    /// A timer that generates a clock signal based on a divider and a period. The timer is clocked
    /// every (period + 1) * divider cycles.
    #[derive(Default, Debug, Clone)]
    #[must_use]
    pub struct Timer {
        /// Running cycle count since the timer was created or last reset.
        pub cycle: u32,
        /// Cycles remaining until the timer reloads and fires.
        pub counter: u16,
        /// Reload value: the timer fires every `period + 1` cycles.
        pub period: u16,
    }

    impl Timer {
        /// A timer with `period` set and the counter starting at zero.
        pub const fn new(period: u16) -> Self {
            Self {
                cycle: 0,
                counter: 0,
                period,
            }
        }

        /// A timer with `period` set and the counter preloaded to `period`, so it
        /// does not fire on its very first tick.
        pub const fn preload(period: u16) -> Self {
            let mut timer = Self::new(period);
            timer.counter = timer.period;
            timer
        }

        /// Resets the counter to `period`.
        pub const fn reload(&mut self) {
            self.counter = self.period;
        }

        /// Advances one cycle. Returns true and reloads the counter on underflow.
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
        /// Resets to power-on state (period zeroed too, unlike `reload`).
        pub fn reset(&mut self) {
            self.counter = 0;
            self.period = 0;
            self.cycle = 0;
        }
    }
}

// Re-exports for convenient access, e.g. `apu::Envelope`, `apu::Timer`, etc.
pub use envelope::Envelope;
pub use length_counter::LengthCounter;
pub use timer::{Timer, TimerCycle};
