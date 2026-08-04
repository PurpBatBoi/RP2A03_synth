//! rp2a03_common\src\midi\mod.rs
//! Incoming MIDI handling and CC mapping for RP2A03 plugin.

mod events;
mod handler;
#[cfg(test)]
mod tests;
mod types;

pub use handler::{AnyChannel, MidiHandler};
pub use types::{
    ActiveSequences, ChannelMode, HostAutomationControls, SequenceReload, freq_to_period,
    freq_to_triangle_period, midi_note_to_freq,
};
