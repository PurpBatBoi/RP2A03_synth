//! rp2a03_core\src\lib.rs

/// NTSC CPU clock frequency in Hz (≈1.789773 MHz).
pub const NTSC_CPU_CLOCK: f64 = 1_789_773.0;

pub mod apu_timer;
pub mod apu_envelope;
pub mod apu_length_counter;
pub mod apu_frame_counter;
pub mod apu_pulse;
pub mod blip_buf;
pub mod lfo;