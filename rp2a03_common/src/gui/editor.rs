//! rp2a03_common\src\gui\editor.rs
//! Layout rendering logic for the reusable sequence editor window.

use super::state::{MAX_SEQUENCES, SequencePlayheads, SharedSequences};
use super::widgets::{draw_envelope_bar_graph, group_box, repeating_button};
use crate::ChannelMode;
use rp2a03_core::sequencer::{ArpMode, PitchMode, Sequence, VolMode};

/// Result returned by [`render_editor_ui`] to communicate parameter changes back
/// to the host plugin wrapper.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditorResult {
    /// If the user changed the shared sequence index via the GUI.
    pub new_sequence_index: Option<usize>,
    /// If the user changed the step time Hz value via the GUI.
    pub new_step_time_hz: Option<i32>,
    /// If the user changed the channel mode via the waveform combobox.
    pub new_channel_mode: Option<ChannelMode>,
    /// If the user changed the polyphony toggle.
    pub new_polyphony: Option<bool>,
    /// If the user changed the maximum voice count.
    pub new_max_voices: Option<i32>,
    pub new_portamento_enabled: Option<bool>,
    pub new_portamento_speed: Option<i32>,
}

/// How long a footer status message stays visible after being set.
const STATUS_DISPLAY_DURATION: std::time::Duration = std::time::Duration::from_secs(4);

/// Transient, UI-only display state for the editor — e.g. the footer status
/// message after a Save/Load. Never touches [`SharedSequences`] (audio-relevant
/// state) or [`EditorResult`] (host-parameter signals).
#[derive(Default)]
pub struct EditorUiState {
    status: Option<(String, std::time::Instant)>,
    /// Set while a Save dialog is showing on its own thread; see
    /// `handle_save_click`'s doc comment for why this can't run on the GUI
    /// thread directly. Polled once per frame in `poll_pending_dialogs`.
    pending_save: Option<std::sync::mpsc::Receiver<Option<String>>>,
    pending_load: Option<std::sync::mpsc::Receiver<Option<std::path::PathBuf>>>,
}

impl EditorUiState {
    /// Sets the footer status message, restarting its display timer.
    pub fn set(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), std::time::Instant::now()));
    }

    /// Returns the status message if one is set and still within its display
    /// window; `None` otherwise (including once it has expired).
    pub fn status_text(&self) -> Option<&str> {
        self.status
            .as_ref()
            .filter(|(_, set_at)| set_at.elapsed() < STATUS_DISPLAY_DURATION)
            .map(|(msg, _)| msg.as_str())
    }
}

/// Converts a Sequence engine instance back to FamiTracker formatted text.
pub fn sequence_to_text(seq: &Sequence) -> String {
    if seq.values.is_empty() {
        return String::new();
    }

    let mut tokens = Vec::with_capacity(seq.values.len() * 2);
    for (i, value) in seq.values.iter().enumerate() {
        if seq.loop_point == Some(i) {
            tokens.push("|".to_string());
        }
        if seq.release_point == Some(i) {
            tokens.push("/".to_string());
        }
        tokens.push(value.to_string());
    }
    if seq.loop_point == Some(seq.values.len()) {
        tokens.push("|".to_string());
    }
    if seq.release_point == Some(seq.values.len()) {
        tokens.push("/".to_string());
    }
    tokens.join(" ")
}

/// Strips non-sequence characters from raw text input.
pub fn sanitize_sequence_text(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_digit() || matches!(*c, '|' | '/' | '-') || c.is_ascii_whitespace())
        .collect()
}

/// dnFamiTracker keeps all sequence items as `signed char`; the editor clamps each
/// envelope type to its documented range. Pitch and hi-pitch share one graph editor
/// (`CPitchGraphEditor`) clamped to [-128, 127] (GraphEditor.cpp: DrawRange(127, -128)).
///
/// `vol_mode` only matters for tab 0 on the VRC6 sawtooth: dn stores the
/// 16-step/64-step setting per volume sequence (`CSequence::GetSetting`), and the
/// bar graph max follows it (SequenceEditor.cpp: `0x0F` vs `0x3F`).
fn sequence_range(tab: usize, channel_mode: ChannelMode, vol_mode: VolMode) -> (i16, i16) {
    match tab {
        0 => {
            if channel_mode == ChannelMode::Vrc6Saw && vol_mode == VolMode::Steps64 {
                (0, 63)
            } else {
                (0, 15)
            }
        }
        1 => (-96, 96),
        2 | 3 => (-128, 127),
        _ => match channel_mode {
            ChannelMode::Vrc6Pulse => (0, 7),
            // dn `CVRC6Sawtooth::MAX_DUTY = 0x01` — the saw's duty is a single bit
            // that becomes bit 5 of the $B000 accumulator rate.
            ChannelMode::Vrc6Saw => (0, 1),
            _ => (0, 3),
        },
    }
}

/// Switches a volume sequence between the 4-bit and 6-bit step ranges.
///
/// dn remaps the existing steps when the setting flips (`SequenceSetting.cpp`:
/// `x * 4` / `x / 4`) so the envelope keeps its shape instead of flattening
/// against the new ceiling. Halving is lossy — the low two bits are gone for good.
fn set_volume_step_mode(text: &mut String, sequence: &mut Sequence, next: VolMode) {
    if sequence.vol_mode == next {
        return;
    }

    let scale_up = next == VolMode::Steps64;
    let (min_val, max_val) = sequence_range(0, ChannelMode::Vrc6Saw, next);

    for value in &mut sequence.values {
        *value = if scale_up { *value * 4 } else { *value / 4 };
        *value = (*value).clamp(min_val, max_val);
    }

    sequence.vol_mode = next;
    *text = sequence_to_text(sequence);
}

/// Brings the selected volume sequence back to the 4-bit range once the editor is
/// no longer on the VRC6 sawtooth.
///
/// 64-step is a saw-only setting, so leaving that waveform has to behave like
/// unticking the toggle — otherwise the stored steps keep their 0..63 values while
/// every other channel reads them through a 0..15 clamp, and the text box disagrees
/// with the graph. Idempotent, so it is safe to run every frame and it catches host
/// automation and state recall as well as the waveform combobox.
fn sync_volume_step_mode_to_channel(data: &mut SharedSequences) {
    if data.channel_mode == ChannelMode::Vrc6Saw {
        return;
    }

    let (text, sequence) = data.selected_sequence_mut(0);
    set_volume_step_mode(text, sequence, VolMode::Steps16);
}

/// Sanitizes the selected numbered sequence for an envelope type.
pub fn cleanup_tab_sequence(data: &mut SharedSequences, tab: usize) {
    let (sanitized, prev_pitch_mode, prev_arp_mode, prev_vol_mode) = {
        let (text, sequence) = data.selected_sequence_mut(tab);
        (
            sanitize_sequence_text(text),
            sequence.pitch_mode,
            sequence.arp_mode,
            sequence.vol_mode,
        )
    };
    let (min_val, max_val) = sequence_range(tab, data.channel_mode, prev_vol_mode);
    let (text, sequence) = data.selected_sequence_mut(tab);

    if sanitized.trim().is_empty() {
        *sequence = Sequence::default();
        sequence.pitch_mode = prev_pitch_mode;
        sequence.arp_mode = prev_arp_mode;
        sequence.vol_mode = prev_vol_mode;
        text.clear();
    } else {
        let (mut parsed, normalized) = Sequence::parse_clamped(&sanitized, min_val, max_val);
        parsed.pitch_mode = prev_pitch_mode;
        parsed.arp_mode = prev_arp_mode;
        parsed.vol_mode = prev_vol_mode;
        let len = parsed.len();
        if parsed.loop_point.is_some_and(|point| point >= len) {
            parsed.loop_point = None;
        }
        if parsed.release_point.is_some_and(|point| point >= len) {
            parsed.release_point = None;
        }
        *sequence = parsed;
        *text = normalized;
    }
}

/// Whether an envelope tab means anything for `channel_mode`.
///
/// The instrument settings panel greys out unavailable tabs; `.rp2a03patch`
/// load reads the same rule to move off a tab the loaded waveform doesn't
/// support, so the two can't drift apart. Triangle and noise have no duty
/// envelope, and noise has no pitch/hi-pitch.
fn tab_is_available(tab: usize, channel_mode: ChannelMode) -> bool {
    let is_no_duty = tab == 4 && matches!(channel_mode, ChannelMode::Triangle | ChannelMode::Noise);
    let is_noise_pitch = matches!(tab, 2 | 3) && channel_mode == ChannelMode::Noise;
    !is_no_duty && !is_noise_pitch
}

/// Opens the Save dialog and writes the file on a dedicated OS thread.
///
/// `rfd`'s blocking dialog runs its own nested Win32 message loop. If that
/// loop runs on the same thread as the editor's baseview window, Windows
/// dispatches cross-window messages back into baseview's WndProc while it is
/// already mid-callback, and baseview's window state (a `RefCell`) is not
/// reentrant — the nested call panics with "already borrowed", which nothing
/// upstream catches, so the panic reaches the top of the thread and aborts
/// the whole host process. Spawning a plain thread with no baseview window on
/// it sidesteps this entirely: that thread's message queue only ever serves
/// the dialog's own windows. This is a bare `std::thread`, not an async
/// runtime — the "no async runtime" constraint from this feature's spec is
/// about not pulling in an executor, not about avoiding threads.
fn handle_save_click(data: &SharedSequences, step_time_hz: u32, ui_state: &mut EditorUiState) {
    if ui_state.pending_save.is_some() {
        return;
    }

    let patch = crate::Patch::from_shared_sequences(data, data.channel_mode, step_time_hz as u16);
    let (tx, rx) = std::sync::mpsc::channel();
    ui_state.pending_save = Some(rx);

    std::thread::spawn(move || {
        let status = save_dialog_and_write(&patch);
        let _ = tx.send(status);
    });
}

/// Returns `None` when the user cancels the dialog (no status to report).
fn save_dialog_and_write(patch: &crate::Patch) -> Option<String> {
    let mut path = rfd::FileDialog::new()
        .add_filter("RP2A03 Patch", &["rp2a03patch"])
        .set_file_name("User_Instrument.rp2a03patch")
        .save_file()?;

    if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rp2a03patch") {
        let mut file_name = path.into_os_string();
        file_name.push(".rp2a03patch");
        path = file_name.into();
    }

    Some(match crate::save_patch_to_path(&path, patch) {
        Ok(()) => format!("Saved {}", path.display()),
        Err(e) => e.to_string(),
    })
}

/// Opens the Load dialog on a dedicated OS thread — see `handle_save_click`'s
/// doc comment for why the dialog itself can never run on the GUI thread.
/// The picked path (if any) is applied later, from `poll_pending_dialogs`,
/// since that needs `&mut SharedSequences`/`&mut EditorResult` from whatever
/// frame the pick actually lands on.
fn handle_load_click(ui_state: &mut EditorUiState) {
    if ui_state.pending_load.is_some() {
        return;
    }

    let (tx, rx) = std::sync::mpsc::channel();
    ui_state.pending_load = Some(rx);

    std::thread::spawn(move || {
        let path = rfd::FileDialog::new()
            .add_filter("RP2A03 Patch", &["rp2a03patch"])
            .pick_file();
        let _ = tx.send(path);
    });
}

/// Applies whichever background dialog (Save, Load, or neither) has finished
/// since the last frame. Must run every frame regardless of whether a dialog
/// is pending — that's what actually advances a finished one.
fn poll_pending_dialogs(
    data: &mut SharedSequences,
    result: &mut EditorResult,
    ui_state: &mut EditorUiState,
) {
    use std::sync::mpsc::TryRecvError;

    if let Some(rx) = &ui_state.pending_save {
        match rx.try_recv() {
            Ok(status) => {
                ui_state.pending_save = None;
                if let Some(status) = status {
                    ui_state.set(status);
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => ui_state.pending_save = None,
        }
    }

    if let Some(rx) = &ui_state.pending_load {
        match rx.try_recv() {
            Ok(Some(path)) => {
                ui_state.pending_load = None;
                // Nothing has been written yet on `Err`, so a rejected file
                // leaves the current instrument exactly as it was — never a
                // partial application.
                match crate::load_patch_from_path(&path) {
                    Ok(patch) => {
                        apply_loaded_patch(data, result, &patch);
                        ui_state.set(format!("Loaded {}", path.display()));
                    }
                    Err(e) => ui_state.set(e.to_string()),
                }
            }
            Ok(None) => ui_state.pending_load = None,
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => ui_state.pending_load = None,
        }
    }
}

/// Installs a decoded patch into the live editor state.
///
/// Split out from the dialog half of Load so the ordering below is reachable
/// from tests — the step order here is load-bearing, not stylistic.
fn apply_loaded_patch(data: &mut SharedSequences, result: &mut EditorResult, patch: &crate::Patch) {
    // `channel_mode` must be updated BEFORE the sequences land.
    // `sync_volume_step_mode_to_channel` runs unconditionally at the top of
    // every frame, and rescales tab 0's volume sequence back to the 4-bit
    // range whenever the channel is not the VRC6 saw. Writing the sequences
    // first would leave a freshly loaded saw patch's 64-step volume data
    // sitting under the previous channel mode, and the next frame's sync
    // would silently clamp it away. Setting the mode here means that sync
    // sees the loaded channel and takes its early return instead.
    data.channel_mode = patch.waveform;
    result.new_channel_mode = Some(patch.waveform);

    patch.apply_to_shared_sequences(data);

    // The loaded waveform may not have the tab the editor is sitting on.
    if !tab_is_available(data.selected_tab, data.channel_mode) {
        data.selected_tab = 0;
    }

    result.new_step_time_hz = Some(patch.step_time_hz as i32);
    // The slot selection has to travel out as a parameter gesture too, not
    // just into `data`: the audio thread re-asserts `sequence_number` over
    // every envelope's selected index on each `process` block
    // (`SequenceCache::refresh`), so leaving the parameter stale would slam
    // the freshly loaded selection back to the old slot within one block.
    // A `Patch` carries five independent indices but this build only has the
    // one shared parameter, so tab 0's is the one that can be honored — every
    // file this build writes has all five equal anyway, since the plugin
    // keeps them in lockstep.
    result.new_sequence_index = Some(patch.active_indices.vol);
}

fn draw_header(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    result: &mut EditorResult,
    ui_state: &mut EditorUiState,
    step_time_hz: u32,
) {
    poll_pending_dialogs(data, result, ui_state);

    let mut portamento = data.portamento_enabled;

    const HEADER_H: f32 = 92.0;
    const LOGO_W: f32 = 312.0;
    const LOGO_H: f32 = 92.0;
    const CENTER_Y: f32 = LOGO_H / 2.0; // 46.0 — everything centers on this

    ui.allocate_ui(egui::vec2(ui.available_width(), HEADER_H), |ui| {
        let origin = ui.min_rect().min;

        //------------------------------------------------------
        // Logo
        //------------------------------------------------------

        ui.scope_builder(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                origin,
                egui::vec2(LOGO_W, LOGO_H),
            )),
            |ui| {
                ui.add(
                    egui::Image::new(egui::include_image!("logo.png"))
                        .fit_to_exact_size(egui::vec2(LOGO_W, LOGO_H))
                        .tint(egui::Color32::from_white_alpha(128)),
                );
            },
        );

        //------------------------------------------------------
        // Layout constants
        //------------------------------------------------------

        let controls_x = LOGO_W + 28.0;

        // Dropdown + checkbox row treated as ONE block, then that
        // block is centered on CENTER_Y.
        const COMBO_H: f32 = 24.0;
        const CHECK_H: f32 = 22.0;
        const ROW_GAP: f32 = 6.0;
        const BLOCK_H: f32 = COMBO_H + ROW_GAP + CHECK_H;

        let block_top = CENTER_Y - BLOCK_H / 2.0;
        let combo_y = block_top;
        let check_y = block_top + COMBO_H + ROW_GAP;

        //------------------------------------------------------
        // Dropdown
        //------------------------------------------------------

        ui.scope_builder(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::pos2(origin.x + controls_x, origin.y + combo_y),
                egui::vec2(190.0, COMBO_H),
            )),
            |ui| {
                let mut waveform_id = data.channel_mode as i32;
                egui::ComboBox::from_id_salt("waveform")
                    .width(180.0)
                    .selected_text(match data.channel_mode {
                        ChannelMode::Pulse => "2A03 | Pulse",
                        ChannelMode::Triangle => "2A03 | Triangle",
                        ChannelMode::Noise => "2A03 | Noise",
                        ChannelMode::Vrc6Pulse => "VRC6 | Pulse",
                        ChannelMode::Vrc6Saw => "VRC6 | Saw",
                    })
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(&mut waveform_id, 0, "2A03 | Pulse")
                            .clicked()
                        {
                            let new_mode = ChannelMode::Pulse;
                            data.channel_mode = new_mode;
                            result.new_channel_mode = Some(new_mode);
                        }
                        if ui
                            .selectable_value(&mut waveform_id, 1, "2A03 | Triangle")
                            .clicked()
                        {
                            let new_mode = ChannelMode::Triangle;
                            // If Duty tab was active, revert to Volume
                            if data.selected_tab == 4 {
                                cleanup_tab_sequence(data, 4);
                                data.selected_tab = 0;
                            }
                            data.channel_mode = new_mode;
                            result.new_channel_mode = Some(new_mode);
                        }
                        if ui
                            .selectable_value(&mut waveform_id, 2, "2A03 | Noise")
                            .clicked()
                        {
                            let new_mode = ChannelMode::Noise;
                            // Fine pitch is not an NES noise-channel control. Keep the
                            // editor on a supported tab when switching into Noise.
                            if matches!(data.selected_tab, 2..=4) {
                                cleanup_tab_sequence(data, data.selected_tab);
                                data.selected_tab = 0;
                            }
                            data.channel_mode = new_mode;
                            result.new_channel_mode = Some(new_mode);
                        }
                        if ui
                            .selectable_value(&mut waveform_id, 3, "VRC6 | Pulse")
                            .clicked()
                        {
                            let new_mode = ChannelMode::Vrc6Pulse;
                            data.channel_mode = new_mode;
                            result.new_channel_mode = Some(new_mode);
                        }
                        if ui
                            .selectable_value(&mut waveform_id, 4, "VRC6 | Saw")
                            .clicked()
                        {
                            let new_mode = ChannelMode::Vrc6Saw;
                            // The saw keeps its Duty tab: in 16-step mode duty bit 0
                            // is the $B000 rate MSB, so tab 4 stays selectable.
                            data.channel_mode = new_mode;
                            result.new_channel_mode = Some(new_mode);
                        }
                    });
            },
        );

        //------------------------------------------------------
        // Checkboxes
        //------------------------------------------------------

        // Capture the row's response so we can measure its actual width,
        // rather than guessing a constant.
        let check_row = ui.scope_builder(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::pos2(origin.x + controls_x, origin.y + check_y),
                egui::vec2(340.0, CHECK_H),
            )),
            |ui| {
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!data.portamento_enabled, |ui| {
                        if ui.checkbox(&mut data.polyphony, "Polyphony").changed() {
                            result.new_polyphony = Some(data.polyphony);
                        }
                    });
                    if ui.checkbox(&mut portamento, "Portamento").changed() {
                        data.portamento_enabled = portamento;
                        result.new_portamento_enabled = Some(portamento);
                    }
                })
                .response
            },
        );

        let _check_row_w = check_row.inner.rect.width();

        //------------------------------------------------------
        // Save / Load
        //------------------------------------------------------

        const BUTTON_W: f32 = 64.0;
        const BUTTON_H: f32 = 24.0;
        const BUTTON_GAP: f32 = 8.0;
        let block_w = BUTTON_W * 2.0 + BUTTON_GAP;
        let block_x = origin.x + ui.available_width() - block_w;

        ui.scope_builder(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::pos2(block_x, origin.y + CENTER_Y - BUTTON_H / 2.0),
                egui::vec2(block_w, BUTTON_H),
            )),
            |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled_ui(ui_state.pending_save.is_none(), |ui| {
                            ui.add_sized([BUTTON_W, BUTTON_H], egui::Button::new("Save"))
                        })
                        .inner
                        .clicked()
                    {
                        handle_save_click(data, step_time_hz, ui_state);
                    }
                    if ui
                        .add_enabled_ui(ui_state.pending_load.is_none(), |ui| {
                            ui.add_sized([BUTTON_W, BUTTON_H], egui::Button::new("Load"))
                        })
                        .inner
                        .clicked()
                    {
                        handle_load_click(ui_state);
                    }
                });
            },
        );
    });

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(5.0);
}

fn draw_chip_tabs(ui: &mut egui::Ui, _data: &mut SharedSequences) {
    ui.horizontal(|ui| {
        let _ = ui.selectable_label(true, "Envelope Editors");
    });
    ui.add_space(5.0);
    ui.separator();
}

fn draw_instrument_settings_panel(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    shared_sequence_index: usize,
    step_time_hz: u32,
    result: &mut EditorResult,
) {
    // "Duty / Noise" tab is disabled for Triangle mode.
    let seq_types: &[(&str, usize)] = &[
        ("Volume", 0),
        ("Arpeggio", 1),
        ("Pitch", 2),
        ("Hi-Pitch", 3),
        ("Duty / Noise", 4),
    ];

    const PANEL_WIDTH: f32 = 180.0;
    const GROUP_GAP: f32 = 12.0;

    ui.set_width(PANEL_WIDTH);

    // ---------------------------------------------------------
    // Instrument settings
    // ---------------------------------------------------------

    group_box(ui, "Instrument settings", |ui| {
        // Make the group box fill the width of the left panel.
        ui.set_min_width(ui.available_width());

        // Grid with checkboxes and effect names
        egui::Grid::new("seq_type_grid")
            .num_columns(2)
            .spacing([6.0, 6.0])
            .show(ui, |ui| {
                ui.label("");
                ui.label(
                    egui::RichText::new("Effect name")
                        .color(egui::Color32::from_rgb(130, 130, 130)),
                );
                ui.end_row();

                // Separator line under header row
                ui.separator();
                ui.separator();
                ui.end_row();

                for &(name, tab) in seq_types {
                    let enabled = tab_is_available(tab, data.channel_mode);

                    ui.add_enabled(
                        enabled,
                        egui::Checkbox::new(data.sequence_enabled_mut(tab), ""),
                    );

                    if ui
                        .add_enabled(
                            enabled,
                            egui::Button::new(name).selected(data.selected_tab == tab),
                        )
                        .clicked()
                        && data.selected_tab != tab
                    {
                        cleanup_tab_sequence(data, data.selected_tab);
                        data.selected_tab = tab;
                        cleanup_tab_sequence(data, tab);
                    }

                    ui.end_row();
                }
            });

        // Sequence Index control inside the frame, below the grid
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label("Sequence Index:");

            let mut index = shared_sequence_index;

            if ui
                .add(egui::DragValue::new(&mut index).range(0..=MAX_SEQUENCES - 1))
                .changed()
            {
                result.new_sequence_index = Some(index);
            }
        });
    });

    // ---------------------------------------------------------
    // Time settings
    // ---------------------------------------------------------

    // The current cursor is now immediately below Instrument settings.
    // Move down slightly to create a gap between the group boxes.
    ui.add_space(GROUP_GAP);

    // Everything remaining in the left panel belongs to Time settings.
    let remaining = ui.available_rect_before_wrap();

    ui.scope_builder(egui::UiBuilder::new().max_rect(remaining), |ui| {
        group_box(ui, "Settings", |ui| {
            // Fill the same width as Instrument settings.
            ui.set_min_width(ui.available_width());

            // group_box() adds:
            //
            // top    = PADDING + TITLE_HEIGHT = 20px
            // bottom = PADDING                = 10px
            //
            // Therefore its contents need to be 30px shorter
            // than the outer available rectangle.
            ui.set_min_height(ui.available_height());

            // -------------------------------------------------
            // Time controls
            // -------------------------------------------------
            ui.horizontal(|ui| {
                ui.label("Engine Speed:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut hz = step_time_hz as i32;
                    if ui
                        .add(egui::DragValue::new(&mut hz).range(1..=600).suffix(" Hz"))
                        .changed()
                    {
                        result.new_step_time_hz = Some(hz);
                    }
                });
            });

            ui.horizontal(|ui| {
                ui.label("Polyphony:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(!data.portamento_enabled, |ui| {
                        if ui
                            .add(
                                egui::DragValue::new(&mut data.max_voices)
                                    .range(1..=8)
                                    .suffix(" voices"),
                            )
                            .changed()
                        {
                            result.new_max_voices = Some(data.max_voices);
                        }
                    });
                });
            });

            ui.horizontal(|ui| {
                ui.label("Porta. Speed:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::DragValue::new(&mut data.portamento_speed).range(0..=127))
                        .changed()
                    {
                        result.new_portamento_speed = Some(data.portamento_speed);
                    }
                });
            });
        });
    });
}

fn draw_sequence_editor_panel(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    playheads: &SequencePlayheads,
    step_time_hz: u32,
) {
    let tab = data.selected_tab;

    let title = match tab {
        0 => "Volume",
        1 => "Arpeggio",
        2 => "Pitch",
        3 => "Hi-pitch",
        _ => "Duty / Noise",
    };

    group_box(ui, &format!("Sequence editor - {}", title), |ui| {
        // Controls below graph: add_space(6) + Size row (~22) + add_space(6) + TextEdit (~20) = ~54px.
        const CONTROLS_HEIGHT: f32 = 54.0;
        let graph_height = (ui.available_height() - CONTROLS_HEIGHT).max(150.0);
        let mut auto_enable = false;

        // Both reads must happen before `selected_sequence_mut` takes the mutable
        // borrow of `data` below.
        let channel_mode = data.channel_mode;
        let vol_mode = data.selected_sequence(tab).vol_mode;
        let (min_val, max_val) = sequence_range(tab, channel_mode, vol_mode);

        {
            let (text, sequence) = data.selected_sequence_mut(tab);

            let is_arpeggio = tab == 1;
            if draw_envelope_bar_graph(
                ui,
                sequence,
                min_val,
                max_val,
                is_arpeggio,
                playheads.step(tab),
                graph_height,
            ) {
                *text = sequence_to_text(sequence);
                auto_enable = true;
            }

            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.label("Size:");

                let mut desired_len = sequence.len();

                if repeating_button(ui, "-") {
                    desired_len = desired_len.saturating_sub(1);
                }

                // FamiTracker-style size control: drag the number vertically to
                // grow or shrink the sequence, while retaining the +/- buttons.
                ui.add_sized(
                    [28.0, 18.0],
                    egui::DragValue::new(&mut desired_len)
                        .speed(1.0)
                        .range(0..=256),
                );

                if repeating_button(ui, "+") {
                    desired_len = desired_len.saturating_add(1).min(256);
                }

                if desired_len != sequence.len() {
                    auto_enable = true;
                    sequence.values.resize(desired_len, 0);

                    if sequence.loop_point.is_some_and(|p| p >= desired_len) {
                        sequence.loop_point = None;
                    }

                    if sequence.release_point.is_some_and(|p| p >= desired_len) {
                        sequence.release_point = None;
                    }

                    *text = sequence_to_text(sequence);
                }

                ui.add_space(15.0);

                ui.label(format!(
                    "{} ms",
                    (sequence.len() as u64 * 1000) / (step_time_hz as u64).max(1)
                ));

                if tab == 1 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.radio_value(&mut sequence.arp_mode, ArpMode::Absolute, "Absolute");

                        ui.radio_value(&mut sequence.arp_mode, ArpMode::Relative, "Relative");

                        ui.label(egui::RichText::new("Mode:").weak());
                    });
                }

                if tab == 2 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.radio_value(&mut sequence.pitch_mode, PitchMode::Absolute, "Absolute");

                        ui.radio_value(&mut sequence.pitch_mode, PitchMode::Relative, "Relative");

                        ui.label(egui::RichText::new("Mode:").weak());
                    });
                }

                // dn exposes 16-step/64-step as a volume-sequence setting and only
                // enables it for VRC6 instruments (SequenceSetting.cpp), so the
                // toggle occupies the same slot as the arpeggio/pitch mode radios.
                if tab == 0 && channel_mode == ChannelMode::Vrc6Saw {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut is_64 = sequence.vol_mode == VolMode::Steps64;

                        if ui.checkbox(&mut is_64, "64-Step").changed() {
                            let next = if is_64 {
                                VolMode::Steps64
                            } else {
                                VolMode::Steps16
                            };
                            set_volume_step_mode(text, sequence, next);
                            auto_enable = true;
                        }
                    });
                }
            });

            ui.add_space(6.0);

            let edit = ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(ui.available_width())
                    .font(egui::TextStyle::Monospace),
            );

            let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));

            if edit.changed() {
                auto_enable = true;
                let sanitized = sanitize_sequence_text(text);
                // `parse_clamped` resets every mode field to its default, so all
                // three have to survive the round trip.
                let prev_pitch_mode = sequence.pitch_mode;
                let prev_arp_mode = sequence.arp_mode;
                let prev_vol_mode = sequence.vol_mode;

                *sequence = if sanitized.trim().is_empty() {
                    Sequence::default()
                } else {
                    Sequence::parse_clamped(&sanitized, min_val, max_val).0
                };

                sequence.pitch_mode = prev_pitch_mode;
                sequence.arp_mode = prev_arp_mode;
                sequence.vol_mode = prev_vol_mode;
            }

            if enter_pressed || edit.lost_focus() {
                *text = sequence_to_text(sequence);
            }
        }

        if auto_enable {
            *data.sequence_enabled_mut(tab) = true;
        }
    });
}

fn draw_main_content(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    shared_sequence_index: usize,
    playheads: &SequencePlayheads,
    step_time_hz: u32,
    result: &mut EditorResult,
) {
    const LEFT_W: f32 = 180.0;
    const GAP: f32 = 8.0;
    const TOP_GAP: f32 = 10.0;

    let available = ui.available_rect_before_wrap();

    // Move the start of BOTH columns down.
    let content_top = available.min.y + TOP_GAP;

    let left_rect = egui::Rect::from_min_max(
        egui::pos2(available.min.x, content_top),
        egui::pos2(available.min.x + LEFT_W, available.max.y),
    );

    let right_rect = egui::Rect::from_min_max(
        egui::pos2(left_rect.max.x + GAP, content_top),
        available.max,
    );

    ui.scope_builder(egui::UiBuilder::new().max_rect(left_rect), |ui| {
        draw_instrument_settings_panel(ui, data, shared_sequence_index, step_time_hz, result);
    });

    ui.scope_builder(egui::UiBuilder::new().max_rect(right_rect), |ui| {
        draw_sequence_editor_panel(ui, data, playheads, step_time_hz);
    });
}

fn draw_footer(ui: &mut egui::Ui, ui_state: &EditorUiState) {
    ui.separator();
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(env!("CARGO_PKG_VERSION")).weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(status) = ui_state.status_text() {
                ui.label(status);
            }
        });
    });
}

pub fn render_editor_ui(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    shared_sequence_index: usize,
    playheads: &SequencePlayheads,
    step_time_hz: u32,
    ui_state: &mut EditorUiState,
) -> EditorResult {
    // Apply non-selectable labels to the global Context style so all child scopes,
    // grids, and group boxes inherit it
    ui.ctx().global_style_mut(|style| {
        style.interaction.selectable_labels = false;
    });

    ui.set_min_height(ui.available_height());

    data.set_all_selected_sequence_indices(shared_sequence_index);

    // Noise has no fine-pitch control. This also handles channel changes made
    // through host automation rather than through the editor combobox.
    if data.channel_mode == ChannelMode::Noise && matches!(data.selected_tab, 2 | 3) {
        data.selected_tab = 0;
    }

    // 64-step volume is saw-only; drop back to the 4-bit range on every other channel.
    sync_volume_step_mode_to_channel(data);

    // Each panel records what the user touched straight into this, so the change
    // set travels as one value instead of a fan of `&mut Option<_>` out-params.
    let mut result = EditorResult::default();

    draw_header(ui, data, &mut result, ui_state, step_time_hz);
    draw_chip_tabs(ui, data);

    // Reserve footer space at the bottom of the window with spacing gap
    const FOOTER_HEIGHT: f32 = 26.0;
    const FOOTER_GAP: f32 = 10.0;
    let available = ui.available_rect_before_wrap();

    // Allocate full available space in parent UI so outer frames don't shrink
    ui.allocate_rect(available, egui::Sense::hover());

    let footer_rect = egui::Rect::from_min_max(
        egui::pos2(available.min.x, available.max.y - FOOTER_HEIGHT),
        available.max,
    );
    let main_rect = egui::Rect::from_min_max(
        available.min,
        egui::pos2(available.max.x, footer_rect.min.y - FOOTER_GAP),
    );

    // Main content fills everything above the footer
    ui.scope_builder(egui::UiBuilder::new().max_rect(main_rect), |ui| {
        draw_main_content(
            ui,
            data,
            shared_sequence_index,
            playheads,
            step_time_hz,
            &mut result,
        );
    });

    // Footer is always at the very bottom
    ui.scope_builder(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
        draw_footer(ui, ui_state);
    });

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A saw instrument whose volume envelope uses the 64-step range — the
    /// only data shape the load-ordering hazard can destroy.
    fn saw_patch_with_64_step_volume() -> crate::Patch {
        let mut source = SharedSequences::default();
        source.channel_mode = ChannelMode::Vrc6Saw;
        source.set_selected_sequence_index(0, 7);
        {
            let (text, sequence) = source.selected_sequence_mut(0);
            sequence.values = vec![63, 40, 20];
            sequence.vol_mode = VolMode::Steps64;
            *text = sequence_to_text(sequence);
        }
        crate::Patch::from_shared_sequences(&source, ChannelMode::Vrc6Saw, 60)
    }

    #[test]
    fn loading_a_saw_patch_survives_the_next_frames_volume_sync() {
        let patch = saw_patch_with_64_step_volume();

        // The editor is on a non-saw channel, which is what arms the hazard.
        let mut data = SharedSequences::default();
        data.channel_mode = ChannelMode::Pulse;
        let mut result = EditorResult::default();

        apply_loaded_patch(&mut data, &mut result, &patch);

        // `render_editor_ui` runs this unconditionally at the top of every
        // frame, so the very next repaint after the click does exactly this.
        sync_volume_step_mode_to_channel(&mut data);

        assert_eq!(
            data.selected_sequence(0).vol_mode,
            VolMode::Steps64,
            "the loaded saw patch must still be in 64-step mode"
        );
        assert_eq!(
            data.selected_sequence(0).values,
            vec![63, 40, 20],
            "64-step volume data must survive the next frame's sync"
        );
    }

    #[test]
    fn writing_sequences_before_the_channel_mode_would_clamp_a_saw_patch() {
        // The inverse of the test above: proves that test is actually load
        // bearing rather than passing for unrelated reasons. This reproduces
        // the wrong ordering by hand — sequences first, channel mode second —
        // and shows the next frame's sync halves every step. If a refactor
        // ever makes this ordering safe, delete both tests together.
        let patch = saw_patch_with_64_step_volume();

        let mut data = SharedSequences::default();
        data.channel_mode = ChannelMode::Pulse;

        patch.apply_to_shared_sequences(&mut data);
        sync_volume_step_mode_to_channel(&mut data);
        data.channel_mode = patch.waveform;

        assert_eq!(
            data.selected_sequence(0).values,
            vec![15, 10, 5],
            "the wrong ordering is expected to scale the steps down by 4"
        );
    }

    #[test]
    fn loading_moves_off_a_tab_the_loaded_waveform_does_not_have() {
        let mut source = SharedSequences::default();
        source.set_selected_sequence_index(4, 1);
        source.selected_sequence_mut(4).1.values = vec![1, 2];
        let patch = crate::Patch::from_shared_sequences(&source, ChannelMode::Triangle, 60);

        let mut data = SharedSequences::default();
        data.selected_tab = 4; // Duty / Noise — triangle has no duty envelope
        let mut result = EditorResult::default();

        apply_loaded_patch(&mut data, &mut result, &patch);

        assert_eq!(
            data.selected_tab, 0,
            "loading a triangle patch must move off the duty tab"
        );
    }

    #[test]
    fn loading_reports_every_host_parameter_the_patch_carries() {
        // Without these the shared state and the host parameters disagree, and
        // the audio thread re-asserts the stale parameter over the loaded slot
        // selection on its next block (see `apply_loaded_patch`).
        let mut source = SharedSequences::default();
        source.set_selected_sequence_index(0, 9);
        source.selected_sequence_mut(0).1.values = vec![15, 7];
        let patch = crate::Patch::from_shared_sequences(&source, ChannelMode::Vrc6Pulse, 120);

        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();

        apply_loaded_patch(&mut data, &mut result, &patch);

        assert_eq!(result.new_channel_mode, Some(ChannelMode::Vrc6Pulse));
        assert_eq!(result.new_step_time_hz, Some(120));
        assert_eq!(result.new_sequence_index, Some(9));
        assert_eq!(
            data.channel_mode,
            ChannelMode::Vrc6Pulse,
            "the shared state must be updated alongside the parameter signal"
        );
    }

    #[test]
    fn unavailable_tabs_match_the_channels_missing_envelopes() {
        for tab in 0..crate::SEQUENCE_TYPE_COUNT {
            assert!(tab_is_available(tab, ChannelMode::Pulse));
            assert!(tab_is_available(tab, ChannelMode::Vrc6Pulse));
            assert!(tab_is_available(tab, ChannelMode::Vrc6Saw));
        }

        assert!(!tab_is_available(4, ChannelMode::Triangle));
        assert!(tab_is_available(0, ChannelMode::Triangle));

        for tab in [2, 3, 4] {
            assert!(!tab_is_available(tab, ChannelMode::Noise));
        }
        assert!(tab_is_available(0, ChannelMode::Noise));
        assert!(tab_is_available(1, ChannelMode::Noise));
    }

    #[test]
    fn status_text_expires_after_its_display_window() {
        let mut ui_state = EditorUiState::default();
        assert_eq!(ui_state.status_text(), None);

        ui_state.set("Saved");
        assert_eq!(ui_state.status_text(), Some("Saved"));

        ui_state.status = Some((
            "Saved".to_string(),
            std::time::Instant::now() - STATUS_DISPLAY_DURATION,
        ));
        assert_eq!(
            ui_state.status_text(),
            None,
            "a message older than the display window must stop rendering"
        );
    }

    #[test]
    fn polling_an_empty_pending_save_channel_does_not_block_or_clear_it() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut ui_state = EditorUiState {
            pending_save: Some(rx),
            ..Default::default()
        };
        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();

        poll_pending_dialogs(&mut data, &mut result, &mut ui_state);

        assert!(
            ui_state.pending_save.is_some(),
            "an unfinished dialog must stay pending, not silently drop"
        );
    }

    #[test]
    fn polling_a_finished_save_channel_reports_the_status_and_clears_pending() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Some("Saved test.rp2a03patch".to_string())).unwrap();
        let mut ui_state = EditorUiState {
            pending_save: Some(rx),
            ..Default::default()
        };
        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();

        poll_pending_dialogs(&mut data, &mut result, &mut ui_state);

        assert!(ui_state.pending_save.is_none());
        assert_eq!(ui_state.status_text(), Some("Saved test.rp2a03patch"));
    }

    #[test]
    fn polling_a_cancelled_save_dialog_clears_pending_without_a_status() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(None).unwrap();
        let mut ui_state = EditorUiState {
            pending_save: Some(rx),
            ..Default::default()
        };
        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();

        poll_pending_dialogs(&mut data, &mut result, &mut ui_state);

        assert!(
            ui_state.pending_save.is_none(),
            "the Save button must re-enable after a cancelled dialog"
        );
        assert_eq!(ui_state.status_text(), None);
    }

    #[test]
    fn polling_a_cancelled_load_dialog_clears_pending_without_touching_state() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(None).unwrap();
        let mut ui_state = EditorUiState {
            pending_load: Some(rx),
            ..Default::default()
        };
        let mut data = SharedSequences::default();
        data.channel_mode = ChannelMode::Vrc6Saw;
        let mut result = EditorResult::default();

        poll_pending_dialogs(&mut data, &mut result, &mut ui_state);

        assert!(
            ui_state.pending_load.is_none(),
            "the Load button must re-enable after a cancelled dialog"
        );
        assert_eq!(data.channel_mode, ChannelMode::Vrc6Saw);
        assert_eq!(result.new_channel_mode, None);
    }

    #[test]
    fn polling_a_finished_load_dialog_loads_the_file_from_the_picked_path() {
        let mut source = SharedSequences::default();
        source.set_selected_sequence_index(0, 3);
        source.selected_sequence_mut(0).1.values = vec![10, 5];
        let patch = crate::Patch::from_shared_sequences(&source, ChannelMode::Pulse, 60);

        let path = std::env::temp_dir().join(format!(
            "rp2a03_editor_test_{}_{}.rp2a03patch",
            std::process::id(),
            line!()
        ));
        crate::save_patch_to_path(&path, &patch).expect("setup save must succeed");

        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Some(path.clone())).unwrap();
        let mut ui_state = EditorUiState {
            pending_load: Some(rx),
            ..Default::default()
        };
        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();

        poll_pending_dialogs(&mut data, &mut result, &mut ui_state);
        std::fs::remove_file(&path).expect("cleanup must succeed");

        assert!(ui_state.pending_load.is_none());
        assert_eq!(data.selected_sequence(0).values, vec![10, 5]);
        assert_eq!(result.new_channel_mode, Some(ChannelMode::Pulse));
    }

    #[test]
    fn polling_a_disconnected_channel_clears_pending_instead_of_wedging_the_button() {
        // Simulates the dialog thread panicking before it can send anything —
        // dropping the sender is exactly what that looks like from here.
        let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
        drop(tx);
        let mut ui_state = EditorUiState {
            pending_save: Some(rx),
            ..Default::default()
        };
        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();

        poll_pending_dialogs(&mut data, &mut result, &mut ui_state);

        assert!(
            ui_state.pending_save.is_none(),
            "a dead sender must not leave the Save button permanently disabled"
        );
    }

    #[test]
    fn sequence_text_round_trips_markers() {
        let sequence = Sequence {
            values: vec![15, 12, 10],
            loop_point: Some(1),
            release_point: Some(1),
            ..Sequence::default()
        };

        let text = sequence_to_text(&sequence);
        assert_eq!(text, "15 | / 12 10");
        let (reparsed, _) = Sequence::parse_clamped(&text, 0, 15);
        assert_eq!(reparsed, sequence);
    }

    #[test]
    fn empty_sequence_has_empty_text() {
        assert!(sequence_to_text(&Sequence::default()).is_empty());
    }

    #[test]
    fn selected_sequence_cleanup_does_not_modify_another_slot() {
        let mut data = SharedSequences::default();
        data.selected_sequence_mut(0).0.push_str("15 12");
        cleanup_tab_sequence(&mut data, 0);
        data.set_selected_sequence_index(0, 1);
        cleanup_tab_sequence(&mut data, 0);
        assert_eq!(data.selected_sequence(0).len(), 0);
        data.set_selected_sequence_index(0, 0);
        assert_eq!(data.selected_sequence(0).values, vec![15, 12]);
    }

    #[test]
    fn hi_pitch_and_pitch_accept_the_full_dn_signed_char_range() {
        // dnFamiTracker's CPitchGraphEditor serves both tabs with a 127..-128 axis, and
        // both the graph and the text box must agree on it.
        assert_eq!(
            sequence_range(2, ChannelMode::Pulse, VolMode::Steps16),
            (-128, 127)
        );
        assert_eq!(
            sequence_range(3, ChannelMode::Pulse, VolMode::Steps16),
            (-128, 127)
        );

        let mut data = SharedSequences::default();
        data.selected_sequence_mut(3)
            .0
            .push_str("127 -128 64 -64 200 -200");
        cleanup_tab_sequence(&mut data, 3);
        assert_eq!(
            data.selected_sequence(3).values,
            vec![127, -128, 64, -64, 127, -128]
        );
    }

    #[test]
    fn vrc6_saw_volume_range_follows_the_step_mode() {
        // dn defaults the VRC6 saw volume sequence to SETTING_VOL_16_STEPS; the 6-bit
        // range is only reachable through SETTING_VOL_64_STEPS.
        assert_eq!(
            sequence_range(0, ChannelMode::Vrc6Saw, VolMode::Steps16),
            (0, 15)
        );
        assert_eq!(
            sequence_range(0, ChannelMode::Vrc6Saw, VolMode::Steps64),
            (0, 63)
        );

        // The step mode is saw-only — every other channel stays 4-bit either way.
        assert_eq!(
            sequence_range(0, ChannelMode::Pulse, VolMode::Steps64),
            (0, 15)
        );
        assert_eq!(
            sequence_range(0, ChannelMode::Vrc6Pulse, VolMode::Steps64),
            (0, 15)
        );
    }

    #[test]
    fn vrc6_saw_duty_range_is_one_bit() {
        // dn `CVRC6Sawtooth::MAX_DUTY = 0x01` — the saw's duty is the $B000 rate MSB.
        assert_eq!(
            sequence_range(4, ChannelMode::Vrc6Saw, VolMode::Steps16),
            (0, 1)
        );
        assert_eq!(
            sequence_range(4, ChannelMode::Vrc6Pulse, VolMode::Steps16),
            (0, 7)
        );
        assert_eq!(
            sequence_range(4, ChannelMode::Pulse, VolMode::Steps16),
            (0, 3)
        );
    }

    #[test]
    fn leaving_the_saw_waveform_rescales_a_64_step_volume_sequence() {
        for channel_mode in [
            ChannelMode::Pulse,
            ChannelMode::Triangle,
            ChannelMode::Noise,
            ChannelMode::Vrc6Pulse,
        ] {
            let mut data = SharedSequences::default();
            data.channel_mode = ChannelMode::Vrc6Saw;
            data.selected_sequence_mut(0).0.push_str("63 32 4 0");
            cleanup_tab_sequence(&mut data, 0);

            {
                let (text, sequence) = data.selected_sequence_mut(0);
                set_volume_step_mode(text, sequence, VolMode::Steps64);
            }
            // "63 32 4 0" was authored under 16-step, so cleanup clamped it to
            // [15, 15, 4, 0] before the x4 promotion.
            assert_eq!(data.selected_sequence(0).values, vec![60, 60, 16, 0]);

            data.channel_mode = channel_mode;
            sync_volume_step_mode_to_channel(&mut data);

            assert_eq!(data.selected_sequence(0).vol_mode, VolMode::Steps16);
            assert_eq!(
                data.selected_sequence(0).values,
                vec![15, 15, 4, 0],
                "{channel_mode:?} must see 4-bit steps"
            );
            // The text box has to agree with the graph, not keep showing 0..63.
            assert_eq!(data.selected_sequence_mut(0).0, "15 15 4 0");

            // Idempotent — a second frame on the same channel changes nothing.
            sync_volume_step_mode_to_channel(&mut data);
            assert_eq!(data.selected_sequence(0).values, vec![15, 15, 4, 0]);
        }
    }

    #[test]
    fn staying_on_the_saw_waveform_keeps_64_step_values() {
        let mut data = SharedSequences::default();
        data.channel_mode = ChannelMode::Vrc6Saw;
        data.selected_sequence_mut(0).0.push_str("15 8 0");
        cleanup_tab_sequence(&mut data, 0);

        {
            let (text, sequence) = data.selected_sequence_mut(0);
            set_volume_step_mode(text, sequence, VolMode::Steps64);
        }

        sync_volume_step_mode_to_channel(&mut data);
        assert_eq!(data.selected_sequence(0).vol_mode, VolMode::Steps64);
        assert_eq!(data.selected_sequence(0).values, vec![60, 32, 0]);
    }

    #[test]
    fn cleanup_preserves_the_saw_step_mode_and_clamps_to_it() {
        let mut data = SharedSequences::default();
        data.channel_mode = ChannelMode::Vrc6Saw;
        data.selected_sequence_mut(0).1.vol_mode = VolMode::Steps64;
        data.selected_sequence_mut(0).0.push_str("63 32 0");
        cleanup_tab_sequence(&mut data, 0);
        assert_eq!(data.selected_sequence(0).vol_mode, VolMode::Steps64);
        assert_eq!(data.selected_sequence(0).values, vec![63, 32, 0]);

        // Back to 16-step: the same text now clamps to the 4-bit range.
        data.selected_sequence_mut(0).1.vol_mode = VolMode::Steps16;
        cleanup_tab_sequence(&mut data, 0);
        assert_eq!(data.selected_sequence(0).vol_mode, VolMode::Steps16);
        assert_eq!(data.selected_sequence(0).values, vec![15, 15, 0]);
    }
}
