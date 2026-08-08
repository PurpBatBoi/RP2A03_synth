//! rp2a03_common\src\lib.rs
//! `rp2a03_common` - Shared MIDI and GUI logic for RP2A03 Synth.

pub mod format;
pub mod gui;
pub mod midi;

pub use format::patch::{
    ActiveIndices, CURRENT_FORMAT_VERSION, PATCH_MAGIC, Patch, PatchError, PatchFileError,
    PatchSequenceEntry, PatchSequences, load_from_path as load_patch_from_path,
    save_to_path as save_patch_to_path,
};
pub use gui::{
    EditorResult, EditorUiState, MAX_SEQUENCES, NO_PLAYHEAD_STEP, SEQUENCE_TYPE_COUNT,
    SequenceBank, SequencePlayheads, SequenceSlot, SharedSequences, cleanup_tab_sequence,
    draw_envelope_bar_graph, render_editor_ui, sanitize_sequence_text, sequence_to_text,
    sequence_to_text_for_tab, style,
};
pub use midi::{
    ActiveSequences, AnyChannel, ChannelMode, HostAutomationControls, MidiHandler, SequenceReload,
    freq_to_period, freq_to_triangle_period, midi_note_to_freq,
};
