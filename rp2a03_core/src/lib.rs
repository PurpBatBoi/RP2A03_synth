//! rp2a03_core\src\lib.rs

/// NTSC CPU clock frequency in Hz (≈1.789773 MHz).
pub const NTSC_CPU_CLOCK: f64 = 1_789_773.0;

pub mod apu;
pub mod apu_pulse;
pub mod blip_buf;
pub mod software_lfo;
pub mod sequencer;
