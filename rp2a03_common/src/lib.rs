//! rp2a03_ui\src\lib.rs
//!
//! `rp2a03_ui` - Dedicated egui User Interface for RP2A03 Synth.

pub mod editor;
pub mod state;
pub mod widgets;

pub use editor::{
    cleanup_tab_sequence, render_editor_ui, sanitize_sequence_text, sequence_to_text,
};
pub use state::{SequenceBank, SequenceSlot, SharedSequences, MAX_SEQUENCES, SEQUENCE_TYPE_COUNT};
pub use widgets::draw_envelope_bar_graph;
