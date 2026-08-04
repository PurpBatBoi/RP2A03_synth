//! rp2a03_niceplug\src\lib.rs
//! RP2A03 Plugin wrapper using nice-plug.
//!
//! Module map — audio-thread code is everything reachable from
//! [`plugin::Rp2a03Plugin::process`]:
//!
//! - [`params`] — host parameter set and the projections the synth reads from it.
//! - [`voice`] — one voice: every emulated channel plus its MIDI/resampler state.
//! - [`voice_bank`] — polyphony: allocation, stealing, event routing, mixdown.
//! - [`sequences`] — envelope data in, playhead positions out.
//! - [`editor`] — egui window hosting `rp2a03_common::render_editor_ui`.
//! - [`plugin`] — the `Plugin` impl tying the above together.

use nice_plug::prelude::*;

mod editor;
mod params;
mod plugin;
mod sequences;
mod voice;
mod voice_bank;

#[cfg(test)]
mod tests;

pub use plugin::Rp2a03Plugin;

nice_export_clap!(Rp2a03Plugin);
nice_export_vst3!(Rp2a03Plugin);
