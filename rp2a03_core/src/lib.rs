//! rp2a03_core\src\lib.rs

/// NTSC CPU clock frequency in Hz (≈1.789773 MHz).
pub const NTSC_CPU_CLOCK: f64 = 1_789_773.0;

pub mod apu;
pub mod apu_noise;
pub mod apu_pulse;
pub mod apu_triangle;
pub mod blip_buf;
pub mod s5b_audio;
pub mod sequencer;
pub mod software_lfo;
pub mod vrc6_common;
pub mod vrc6_pulse;
pub mod vrc6_saw;
