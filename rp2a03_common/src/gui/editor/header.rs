//! `rp2a03_common\src\gui\editor\header.rs`
//! The top bar: logo, waveform selector, polyphony/portamento toggles, and
//! the Save/Load buttons — including the background-thread file dialog and
//! `.rp2a03patch` I/O they kick off.

use super::{EditorResult, EditorUiState, HostParamsView, apply_channel_mode_change};
use crate::gui::state::SharedSequences;
use crate::{ChannelMode, Lane};

fn handle_save_click(data: &SharedSequences, step_time_hz: u32, ui_state: &mut EditorUiState) {
    if ui_state.pending_save.is_some() {
        return;
    }

    let patch = crate::Patch::from_shared_sequences(data, step_time_hz as u16);
    let (tx, rx) = std::sync::mpsc::channel();
    ui_state.pending_save = Some(rx);

    std::thread::spawn(move || {
        let status = save_dialog_and_write(&patch);
        let _ = tx.send(status);
    });
}

fn save_dialog_and_write(patch: &crate::Patch) -> Option<String> {
    let mut out_path = rfd::FileDialog::new()
        .add_filter("RP2A03 Patch", &["rp2a03patch"])
        .set_file_name("User_Instrument.rp2a03patch")
        .save_file()?;

    if out_path.extension().and_then(std::ffi::OsStr::to_str) != Some("rp2a03patch") {
        let mut file_name = out_path.into_os_string();
        file_name.push(".rp2a03patch");
        out_path = file_name.into();
    }

    Some(match crate::save_patch_to_path(&out_path, patch) {
        Ok(()) => format!(
            "{} saved",
            out_path.file_name().map_or_else(
                || out_path.display().to_string(),
                |n| n.to_string_lossy().into_owned()
            )
        ),
        Err(e) => e.to_string(),
    })
}

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

fn poll_pending_dialogs(
    data: &mut SharedSequences,
    host: &mut HostParamsView,
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

                match crate::load_patch_from_path(&path) {
                    Ok(loaded) => {
                        apply_loaded_patch(data, host, result, &loaded);
                        let file_name = path.file_name().map_or_else(
                            || path.display().to_string(),
                            |n| n.to_string_lossy().into_owned(),
                        );
                        ui_state.set(format!("{file_name} loaded"));
                    }
                    Err(e) => ui_state.set(e.to_string()),
                }
            }
            Ok(None) | Err(TryRecvError::Disconnected) => ui_state.pending_load = None,
            Err(TryRecvError::Empty) => {}
        }
    }
}

fn apply_loaded_patch(
    data: &mut SharedSequences,
    host: &mut HostParamsView,
    result: &mut EditorResult,
    patch: &crate::Patch,
) {
    let waveform = patch.active_waveform();

    host.channel_mode = waveform;
    result.new_channel_mode = Some(waveform);

    patch.apply_to_shared_sequences(data);

    if !data.selected_tab.available_for(host.channel_mode) {
        data.selected_tab = Lane::Vol;
    }

    result.new_step_time_hz = Some(i32::from(patch.step_time_hz));

    result.new_sequence_index = Some(patch.active_indices.vol);
}

pub(super) fn draw_header(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    host: &mut HostParamsView,
    result: &mut EditorResult,
    ui_state: &mut EditorUiState,
    step_time_hz: u32,
    shared_sequence_index: usize,
) {
    poll_pending_dialogs(data, host, result, ui_state);

    let mut portamento = host.portamento_enabled;

    const HEADER_H: f32 = 92.0;
    const LOGO_W: f32 = 312.0;
    const LOGO_H: f32 = 92.0;
    const CENTER_Y: f32 = LOGO_H / 2.0;

    ui.allocate_ui(egui::vec2(ui.available_width(), HEADER_H), |ui| {
        let origin = ui.min_rect().min;

        ui.scope_builder(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                origin,
                egui::vec2(LOGO_W, LOGO_H),
            )),
            |ui| {
                ui.add(
                    egui::Image::new(egui::include_image!("../logo.png"))
                        .fit_to_exact_size(egui::vec2(LOGO_W, LOGO_H))
                        .tint(egui::Color32::from_white_alpha(128)),
                );
            },
        );

        let controls_x = LOGO_W + 28.0;
        draw_waveform_and_toggles(
            ui,
            data,
            host,
            result,
            ui_state,
            &mut portamento,
            origin,
            controls_x,
            CENTER_Y,
            shared_sequence_index,
        );
        draw_save_load_buttons(ui, data, ui_state, step_time_hz, origin, CENTER_Y);
    });

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(5.0);
}

/// The waveform combo box and the Polyphony/Portamento checkbox row beneath it.
#[allow(clippy::too_many_arguments)]
fn draw_waveform_and_toggles(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    host: &mut HostParamsView,
    result: &mut EditorResult,
    ui_state: &mut EditorUiState,
    portamento: &mut bool,
    origin: egui::Pos2,
    controls_x: f32,
    center_y: f32,
    shared_sequence_index: usize,
) {
    const COMBO_H: f32 = 24.0;
    const CHECK_H: f32 = 22.0;
    const ROW_GAP: f32 = 6.0;
    const BLOCK_H: f32 = COMBO_H + ROW_GAP + CHECK_H;

    let block_top = center_y - BLOCK_H / 2.0;
    let combo_y = block_top;
    let check_y = block_top + COMBO_H + ROW_GAP;

    ui.scope_builder(
        egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
            egui::pos2(origin.x + controls_x, origin.y + combo_y),
            egui::vec2(190.0, COMBO_H),
        )),
        |ui| {
            let mut waveform_id = host.channel_mode as i32;
            egui::ComboBox::from_id_salt("waveform")
                .width(180.0)
                .selected_text(match host.channel_mode {
                    ChannelMode::Pulse => "2A03 | Pulse",
                    ChannelMode::Triangle => "2A03 | Triangle",
                    ChannelMode::Noise => "2A03 | Noise",
                    ChannelMode::Vrc6Pulse => "VRC6 | Pulse",
                    ChannelMode::Vrc6Saw => "VRC6 | Saw",
                    ChannelMode::S5B => "S5B | PSG",
                    ChannelMode::Fds => "FDS | Wavetable",
                })
                .show_ui(ui, |ui| {
                    const WAVEFORMS: [(i32, ChannelMode, &str); 7] = [
                        (0, ChannelMode::Pulse, "2A03 | Pulse"),
                        (1, ChannelMode::Triangle, "2A03 | Triangle"),
                        (2, ChannelMode::Noise, "2A03 | Noise"),
                        (3, ChannelMode::Vrc6Pulse, "VRC6 | Pulse"),
                        (4, ChannelMode::Vrc6Saw, "VRC6 | Saw"),
                        (5, ChannelMode::S5B, "S5B | PSG"),
                        (6, ChannelMode::Fds, "FDS | Wavetable"),
                    ];
                    for (id, new_mode, label) in WAVEFORMS {
                        if ui.selectable_value(&mut waveform_id, id, label).clicked() {
                            apply_channel_mode_change(data, host, new_mode, result);

                            data.set_slot_waveform(shared_sequence_index, new_mode);
                        }
                    }
                });
        },
    );

    ui.scope_builder(
        egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
            egui::pos2(origin.x + controls_x, origin.y + check_y),
            egui::vec2(340.0, CHECK_H),
        )),
        |ui| {
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!host.portamento_enabled, |ui| {
                    if ui.checkbox(&mut host.polyphony, "Polyphony").changed() {
                        result.new_polyphony = Some(host.polyphony);
                    }
                });
                if ui.checkbox(portamento, "Portamento").changed() {
                    host.portamento_enabled = *portamento;
                    result.new_portamento_enabled = Some(*portamento);
                    // Portamento is monophonic; turning it on while
                    // Polyphony is checked would otherwise leave the
                    // checkbox stuck on (just greyed out) instead of
                    // reflecting the engine's actual voice count. Remember
                    // Polyphony's prior value so turning Portamento back off
                    // restores it, rather than silently losing the setting.
                    if *portamento {
                        if host.polyphony {
                            ui_state.polyphony_before_portamento = Some(true);
                            host.polyphony = false;
                            result.new_polyphony = Some(false);
                        }
                    } else if ui_state.polyphony_before_portamento.take() == Some(true) {
                        host.polyphony = true;
                        result.new_polyphony = Some(true);
                    }
                }
            });
        },
    );
}

/// The Save/Load button pair, right-aligned in the header.
fn draw_save_load_buttons(
    ui: &mut egui::Ui,
    data: &SharedSequences,
    ui_state: &mut EditorUiState,
    step_time_hz: u32,
    origin: egui::Pos2,
    center_y: f32,
) {
    const BUTTON_W: f32 = 64.0;
    const BUTTON_H: f32 = 24.0;
    const BUTTON_GAP: f32 = 8.0;
    let block_w = BUTTON_W * 2.0 + BUTTON_GAP;
    let block_x = origin.x + ui.available_width() - block_w;

    ui.scope_builder(
        egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
            egui::pos2(block_x, origin.y + center_y - BUTTON_H / 2.0),
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
}
