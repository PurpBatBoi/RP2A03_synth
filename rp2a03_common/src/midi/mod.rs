//! rp2a03_common\src\midi\mod.rs
//! Incoming MIDI handling and CC mapping for RP2A03 plugin.

mod events;
mod handler;
mod types;

#[cfg(test)]
mod tests;

pub use handler::MidiHandler;
pub use types::{freq_to_period, midi_note_to_freq, ActiveSequences, HostAutomationControls};