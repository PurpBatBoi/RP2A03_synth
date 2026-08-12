//! rp2a03_niceplug\src\editor.rs
//! Editor window construction. The actual sequence-editor drawing lives in
//! `rp2a03_common::render_editor_ui`; this module only owns the egui host
//! window and the write-back of editor results into host parameters.

use nice_plug::prelude::*;
use nice_plug_egui::{EguiSettings, EguiState, create_egui_editor};
use rp2a03_common::{EditorResult, EditorUiState, SEQUENCE_TYPE_COUNT, render_editor_ui, style};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use crate::params::Rp2a03Params;
use crate::sequences::snapshot;

const EDITOR_MARGIN: i8 = 12;
const EDITOR_BORDER: egui::Color32 = egui::Color32::from_rgb(30, 30, 30);
const REPAINT_INTERVAL: Duration = Duration::from_millis(30);

/// Writes a parameter through the host's gesture protocol.
///
/// Every editor-driven parameter change must be wrapped in a begin/end pair so
/// the host can record it as a single automation gesture.
fn set_param<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

pub(crate) fn create(
    params: Arc<Rp2a03Params>,
    playheads: Arc<[AtomicUsize; SEQUENCE_TYPE_COUNT]>,
) -> Option<Box<dyn Editor>> {
    let egui_state: Arc<EguiState> = params.egui_state.clone();

    create_egui_editor(
        egui_state,
        EditorUiState::default(),
        EguiSettings::default(),
        move |ctx, _queue, _ui_state| {
            egui_extras::install_image_loaders(ctx);
            ctx.set_style_of(egui::Theme::Dark, style());
        },
        move |ui, setter, _queue, ui_state| {
            let mut data = params.shared_sequences.lock();
            let sequence_index = data.selected_sequence_index(0);
            let playheads = snapshot(&playheads);
            let step_time_hz = params.step_time.value() as u32;

            ui.ctx().request_repaint_after(REPAINT_INTERVAL);

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(egui::Color32::from_gray(27)))
                .show_inside(ui, |ui| {
                    egui::Frame::new()
                        .stroke(egui::Stroke::new(1.5_f32, EDITOR_BORDER))
                        .inner_margin(egui::Margin::same(EDITOR_MARGIN))
                        .show(ui, |ui| {
                            let result = render_editor_ui(
                                ui,
                                &mut data,
                                sequence_index,
                                &playheads,
                                step_time_hz,
                                ui_state,
                            );

                            // The sequence index is the one control the editor owns
                            // directly as well as through the parameter, so the shared
                            // state is updated before the parameter write.
                            if let Some(new_index) = result.new_sequence_index {
                                data.set_all_selected_sequence_indices(new_index);
                            }

                            apply_result(setter, &params, &result);
                        });
                });
        },
    )
}

/// Pushes everything the editor changed back out as host parameter gestures.
fn apply_result(setter: &ParamSetter, params: &Rp2a03Params, result: &EditorResult) {
    if let Some(new_index) = result.new_sequence_index {
        set_param(setter, &params.sequence_number, new_index as i32);
    }
    if let Some(new_hz) = result.new_step_time_hz {
        set_param(setter, &params.step_time, new_hz);
    }
    if let Some(new_mode) = result.new_channel_mode {
        set_param(setter, &params.waveform, new_mode as i32);
    }
    if let Some(new_polyphony) = result.new_polyphony {
        set_param(setter, &params.polyphony, new_polyphony);
    }
    if let Some(new_max_voices) = result.new_max_voices {
        set_param(setter, &params.max_voices, new_max_voices);
    }
    if let Some(enabled) = result.new_portamento_enabled {
        set_param(setter, &params.portamento_enabled, enabled);
    }
    if let Some(speed) = result.new_portamento_speed {
        set_param(setter, &params.portamento_speed, speed);
    }
}
