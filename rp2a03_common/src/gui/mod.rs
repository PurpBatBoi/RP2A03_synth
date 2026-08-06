//! rp2a03_common\src\gui\mod.rs
//! Reusable egui-based UI for editing FamiTracker-style sequences.

mod editor;
mod state;
pub mod theme;
mod widgets;

pub use theme::style;

pub use editor::{
    EditorResult, EditorUiState, cleanup_tab_sequence, render_editor_ui, sanitize_sequence_text,
    sequence_to_text,
};

pub use state::{
    MAX_SEQUENCES, NO_PLAYHEAD_STEP, SEQUENCE_TYPE_COUNT, SequenceBank, SequencePlayheads,
    SequenceSlot, SharedSequences,
};

pub use widgets::{draw_envelope_bar_graph, group_box, repeating_button};
