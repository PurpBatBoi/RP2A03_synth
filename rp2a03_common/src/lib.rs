//! rp2a03_common\src\lib.rs
//! `rp2a03_common` - Shared MIDI and GUI logic for RP2A03 Synth.

pub mod gui;
pub mod midi;

pub use gui::{
    cleanup_tab_sequence, draw_envelope_bar_graph, render_editor_ui, sanitize_sequence_text,
    sequence_to_text, style, SequenceBank, SequencePlayheads, SequenceSlot, SharedSequences,
    MAX_SEQUENCES, NO_PLAYHEAD_STEP, SEQUENCE_TYPE_COUNT,
};
pub use midi::{
    freq_to_period, midi_note_to_freq, ActiveSequences, HostAutomationControls, MidiHandler,
};
