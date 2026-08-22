//! `rp2a03_common\src\gui\wavesynth\mod.rs`
//! The Wave Synthesizer: `engine` is the pure tick algorithm (also driven
//! from `midi/fds_bridge.rs`, off the GUI thread), `panel` is the tab that
//! paints it and lets the user pick an algorithm and its parameters.

mod engine;
mod panel;

pub use super::wavetable_state::{FDS_WAVE_LEN, FDS_WAVE_MAX};
pub use engine::{fds_wave_from_slot, tick};
pub use panel::draw_wavesynth_panel;
