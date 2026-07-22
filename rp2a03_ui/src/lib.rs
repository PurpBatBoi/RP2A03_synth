//! rp2a03_ui\src\lib.rs
//! 
//! `rp2a03_ui` - Dedicated egui User Interface for RP2A03 Synth.

pub mod editor;
pub mod state;
pub mod widgets;

pub use editor::{cleanup_tab_sequence, render_editor_ui, sanitize_sequence_text};
pub use state::SharedSequences;
pub use widgets::draw_envelope_bar_graph;