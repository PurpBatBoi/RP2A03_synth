//! `rp2a03_common\src\midi\mod.rs`
//! Incoming MIDI handling and CC mapping for RP2A03 plugin.

mod events;
mod fds_bridge;
mod handler;
mod modulate;
mod types;

pub use handler::MidiHandler;
pub use modulate::Modulate;
pub use types::{
    ActiveSequences, ChannelMode, HostAutomationControls, HostAutomationSnapshot, Lane,
    SequenceReload, freq_to_period, freq_to_triangle_period, midi_note_to_freq,
};
