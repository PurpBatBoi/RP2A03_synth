//! `rp2a03_common\src\gui\mod.rs`

mod chip_settings;
mod chip_settings_state;
mod editor;
mod state;
pub mod theme;
mod wavesynth;
mod wavesynth_state;
mod wavetable_editor;
mod wavetable_state;
mod widgets;

pub use theme::style;

pub use editor::{
    EditorResult, EditorUiState, HostParamsView, cleanup_tab_sequence, render_editor_ui,
    sanitize_sequence_text, sequence_to_text, sequence_to_text_for_tab,
};

pub use state::{
    MAX_SEQUENCES, NO_PLAYHEAD_STEP, SequenceBank, SequencePlayheads, SequenceSlot, SharedSequences,
};

pub use chip_settings_state::{
    FDS_MOD_DELAY_MAX, FDS_MOD_DEPTH_MAX, FDS_MOD_SPEED_MAX, FDS_MOD_TABLE_LEN, FDS_MOD_TABLE_MAX,
    FDS_MOD_TABLE_MIN, FdsSettings,
};

pub use widgets::{draw_envelope_bar_graph, group_box, repeating_button};

pub use wavesynth::{FDS_WAVE_LEN, FDS_WAVE_MAX, fds_wave_from_slot, tick as wavesynth_tick};
pub use wavesynth_state::{TickState, WaveSynthEffect, WaveSynthParams};
pub use wavetable_state::{MAX_SLOTS, WaveSlot, WaveSlots};
