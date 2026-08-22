//! `rp2a03_common\src\gui\editor\instrument_panel.rs`
//! The left sidebar on the Envelope Editors tab: which lane is enabled and
//! selected, the sequence index, and the per-instrument engine/voice
//! settings.

use super::sequence_panel::cleanup_tab_sequence;
use super::{EditorResult, HostParamsView};
use crate::Lane;
use crate::gui::state::{MAX_SEQUENCES, SharedSequences};
use crate::gui::widgets::group_box;

pub(super) fn draw_instrument_settings_panel(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    host: &mut HostParamsView,
    shared_sequence_index: usize,
    step_time_hz: u32,
    result: &mut EditorResult,
) {
    const PANEL_WIDTH: f32 = 180.0;
    const GROUP_GAP: f32 = 12.0;

    ui.set_width(PANEL_WIDTH);

    draw_lane_selector(ui, data, host, shared_sequence_index, result);

    ui.add_space(GROUP_GAP);

    let remaining = ui.available_rect_before_wrap();
    ui.scope_builder(egui::UiBuilder::new().max_rect(remaining), |ui| {
        draw_engine_settings(ui, host, step_time_hz, result);
    });
}

/// The "Instrument settings" group: which lane is enabled/selected per row,
/// plus the sequence index.
fn draw_lane_selector(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    host: &HostParamsView,
    shared_sequence_index: usize,
    result: &mut EditorResult,
) {
    group_box(ui, "Instrument settings", |ui| {
        ui.set_min_width(ui.available_width());

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

                ui.separator();
                ui.separator();
                ui.end_row();

                for lane in Lane::ALL {
                    let enabled = lane.available_for(host.channel_mode);
                    let name = lane.label(host.channel_mode);

                    let mut lane_enabled = data.sequence_enabled(lane);
                    if ui
                        .add_enabled(enabled, egui::Checkbox::new(&mut lane_enabled, ""))
                        .changed()
                    {
                        *data.sequence_enabled_mut(lane) = lane_enabled;
                    }

                    if ui
                        .add_enabled(
                            enabled,
                            egui::Button::new(name).selected(data.selected_tab == lane),
                        )
                        .clicked()
                        && data.selected_tab != lane
                    {
                        cleanup_tab_sequence(data, host.channel_mode, data.selected_tab);
                        data.selected_tab = lane;
                        cleanup_tab_sequence(data, host.channel_mode, lane);
                    }

                    ui.end_row();
                }
            });

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
}

/// The "Settings" group: engine speed, polyphony voice count, portamento speed.
fn draw_engine_settings(
    ui: &mut egui::Ui,
    host: &mut HostParamsView,
    step_time_hz: u32,
    result: &mut EditorResult,
) {
    group_box(ui, "Settings", |ui| {
        ui.set_min_width(ui.available_width());

        ui.set_min_height(ui.available_height());

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
                ui.add_enabled_ui(!host.portamento_enabled, |ui| {
                    if ui
                        .add(
                            egui::DragValue::new(&mut host.max_voices)
                                .range(1..=8)
                                .suffix(" voices"),
                        )
                        .changed()
                    {
                        result.new_max_voices = Some(host.max_voices);
                    }
                });
            });
        });

        ui.horizontal(|ui| {
            ui.label("Porta. Speed:");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::DragValue::new(&mut host.portamento_speed).range(0..=127))
                    .changed()
                {
                    result.new_portamento_speed = Some(host.portamento_speed);
                }
            });
        });
    });
}
