//! rp2a03_common\src\gui\mod.rs
//! Reusable egui-based UI for editing FamiTracker-style sequences.

mod editor;
mod state;
pub mod theme;
mod widgets;

pub use theme::style;

pub use editor::{
    cleanup_tab_sequence,
    render_editor_ui,
    sanitize_sequence_text,
    sequence_to_text,
};

pub use state::{
    SequenceBank,
    SequenceSlot,
    SharedSequences,
    MAX_SEQUENCES,
    SEQUENCE_TYPE_COUNT,
};

pub use widgets::draw_envelope_bar_graph;