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

/// Points nice-plug's crash logger at a fixed file before the host makes its first call into
/// this library (nice-plug's own `setup_logger()` runs on that first call and locks in whatever
/// `NICE_LOG` says then). Without this, panics log to STDERR or WinDbg, which no host user can
/// find — this way a crash always leaves a file the user can attach to a bug report. Leaves
/// `NICE_LOG` alone if the user already set it.
#[ctor::ctor(unsafe)]
fn point_crash_log_at_a_file() {
    if std::env::var_os("NICE_LOG").is_some() {
        return;
    }
    let Some(mut log_dir) = dirs::data_local_dir() else {
        return;
    };
    log_dir.push("rp2a03_synth");
    if std::fs::create_dir_all(&log_dir).is_err() {
        return;
    }
    log_dir.push("crash.log");
    // SAFETY: ctors run at library load time, before the host creates any thread that could
    // race on the environment.
    unsafe { std::env::set_var("NICE_LOG", log_dir) };
}
