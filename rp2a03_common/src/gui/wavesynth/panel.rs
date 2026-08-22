//! `rp2a03_common\src\gui\wavesynth\panel.rs`
//! The Wave Synthesizer tab: algorithm picker, wave-slot pickers, live
//! preview, and per-effect parameters. Painting only — the actual
//! synthesis runs in `engine::tick`.

use super::engine::tick;
use super::{FDS_WAVE_LEN, FDS_WAVE_MAX};
use crate::gui::theme;
use crate::gui::wavesynth_state::{TickState, WaveSynthEffect, WaveSynthParams, WaveSynthPreview};
use crate::gui::wavetable_state::{DrawStyle, WaveSlots};
use crate::gui::widgets::draw_wavetable_graph;

const PREVIEW_HEIGHT: f32 = 120.0;

pub fn draw_wavesynth_panel(
    ui: &mut egui::Ui,
    params: &mut WaveSynthParams,
    preview: &mut WaveSynthPreview,
    slots: &mut WaveSlots,
    wave_index_lane_active: bool,
) {
    ui.add_space(10.0);

    ui.checkbox(&mut params.enabled, "Enable synthesizer");
    ui.add_space(6.0);

    if !params.enabled {
        ui.label(
            egui::RichText::new(
                "Synthesizer disabled — select a slot in the Wavetable Editor tab \
                 to preview its raw wave.",
            )
            .weak(),
        );
        return;
    }

    if slots.is_empty() {
        ui.label(
            egui::RichText::new(
                "No wavetables yet — add one in the Wavetable Editor tab to morph it.",
            )
            .weak(),
        );
        return;
    }

    let slot_count = slots.slot_count();
    params.wave1 = params.wave1.min(slot_count - 1);
    params.wave2 = params.wave2.min(slot_count - 1);

    let (width, height) = (FDS_WAVE_LEN, FDS_WAVE_MAX);
    let src1 = slots.slots()[params.wave1].data().to_vec();
    let src2 = slots.slots()[params.wave2].data().to_vec();

    reseed_and_tick_preview(ui, params, preview, &src1, &src2, width, height);

    let dual = params.effect.is_dual();

    draw_algorithm_picker(ui, params);

    ui.add_space(6.0);

    let mut views: Vec<(&str, Vec<u16>)> = vec![("Wave 1", src1.clone())];
    if dual {
        views.push(("Wave 2", src2.clone()));
    }
    views.push(("Result", preview.result.clone()));

    ui.columns(views.len(), |columns| {
        for (col, (label, data)) in columns.iter_mut().zip(views.iter_mut()) {
            col.vertical_centered(|ui| {
                ui.label(egui::RichText::new(*label).weak());
            });
            draw_wavetable_graph(col, data, height, DrawStyle::Lines, PREVIEW_HEIGHT, false);
        }
    });

    ui.add_space(6.0);

    draw_wave_picker_row(
        ui,
        params,
        preview,
        slots,
        wave_index_lane_active,
        slot_count,
        dual,
        width,
        height,
    );

    ui.add_space(6.0);

    egui::Grid::new("wavesynth_params")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Update Rate");
            ui.add(egui::DragValue::new(&mut params.rate_divider).range(1..=255))
                .on_hover_text("Ticks between effect passes");
            ui.end_row();

            ui.label("Speed");
            ui.add(egui::DragValue::new(&mut params.speed).range(0..=255))
                .on_hover_text("Extra output positions advanced per pass");
            ui.end_row();

            ui.label("Amount");
            ui.add(egui::DragValue::new(&mut params.param1).range(0..=255));
            ui.end_row();

            if params.effect == WaveSynthEffect::PhaseModulation {
                ui.label("Power");
                ui.add(egui::DragValue::new(&mut params.param2).range(0..=255));
                ui.end_row();
            }

            ui.label("Reset on new note");
            ui.checkbox(&mut params.reset_on_note, "").on_hover_text(
                "Restart the synthesizer from step 0 on each note-on. \
                 Off: it keeps running across notes.\n\n\
                 An instrument change restarts it either way.",
            );
            ui.end_row();
        });
}

/// The Algorithm combo box, grouped into single- and dual-waveform effects.
fn draw_algorithm_picker(ui: &mut egui::Ui, params: &mut WaveSynthParams) {
    ui.horizontal(|ui| {
        ui.label("Algorithm:");
        egui::ComboBox::from_id_salt("wavesynth_effect")
            .selected_text(params.effect.label())
            .show_ui(ui, |ui| {
                ui.label(egui::RichText::new("Single-waveform").weak());
                for (effect, label) in WaveSynthEffect::SINGLE {
                    ui.selectable_value(&mut params.effect, effect, label);
                }
                ui.separator();
                ui.label(egui::RichText::new("Dual-waveform").weak());
                for (effect, label) in WaveSynthEffect::DUAL {
                    ui.selectable_value(&mut params.effect, effect, label);
                }
            });
    });
}

/// Re-seeds `preview` from `src1` whenever its dimensions/effect changed (or,
/// for an accumulating effect, whenever the source wave itself changed), then
/// advances one tick unless paused. State-only — no painting.
fn reseed_and_tick_preview(
    ui: &egui::Ui,
    params: &WaveSynthParams,
    preview: &mut WaveSynthPreview,
    src1: &[u16],
    src2: &[u16],
    width: usize,
    height: u16,
) {
    if preview.result.len() != width
        || preview.seeded_dims != (width, height)
        || preview.seeded_effect != Some(params.effect)
    {
        preview.result.clear();
        preview.result.extend_from_slice(src1);
        preview.seeded_from.clear();
        preview.seeded_from.extend_from_slice(src1);
        preview.seeded_dims = (width, height);
        preview.seeded_effect = Some(params.effect);
        preview.tick_state = TickState::default();
    } else if params.effect.accumulates() && preview.seeded_from != src1 {
        preview.result.clear();
        preview.result.extend_from_slice(src1);
        preview.seeded_from.clear();
        preview.seeded_from.extend_from_slice(src1);
    }

    if !preview.paused {
        tick(
            params,
            &mut preview.tick_state,
            &mut preview.result,
            src1,
            src2,
            height,
        );
        ui.ctx().request_repaint();
    }
}

/// Wave 1/Wave 2 slot pickers, transport controls (pause/restart/copy-to-slot),
/// and the dimensions readout.
#[allow(clippy::too_many_arguments)]
fn draw_wave_picker_row(
    ui: &mut egui::Ui,
    params: &mut WaveSynthParams,
    preview: &mut WaveSynthPreview,
    slots: &mut WaveSlots,
    wave_index_lane_active: bool,
    slot_count: usize,
    dual: bool,
    width: usize,
    height: u16,
) {
    ui.horizontal(|ui| {
        const WAVE1_OVERRIDDEN_TOOLTIP: &str =
            "Wave Index envelope is controlling Wave 1 — this value will be ineffective.";

        if wave_index_lane_active {
            ui.label(egui::RichText::new("⚠ Wave 1:").color(theme::WARNING))
                .on_hover_text(WAVE1_OVERRIDDEN_TOOLTIP);
        } else {
            ui.label("Wave 1:");
        }

        let highest = slot_count.saturating_sub(1);
        let mut shown = params.wave1;
        let picker = wave_slot_picker(&mut shown, highest, slot_count);
        let picker = ui.add_enabled(slot_count > 1, picker);
        let picker = if wave_index_lane_active {
            picker.on_hover_text(WAVE1_OVERRIDDEN_TOOLTIP)
        } else {
            picker
        };
        if picker.changed() {
            params.wave1 = shown;
        }

        if dual {
            ui.label("Wave 2:");
            let mut shown = params.wave2;
            let picker = wave_slot_picker(&mut shown, highest, slot_count);
            if ui.add_enabled(slot_count > 1, picker).changed() {
                params.wave2 = shown;
            }
        }

        ui.separator();

        if ui
            .button(if preview.paused { "▶" } else { "⏸" })
            .on_hover_text(if preview.paused { "Resume" } else { "Pause" })
            .clicked()
        {
            preview.paused = !preview.paused;
        }
        if ui.button("⟳").on_hover_text("Restart").clicked() {
            preview.restart();
        }
        if ui
            .add_enabled(!slots.is_full(), egui::Button::new("⏏"))
            .on_hover_text(if slots.is_full() {
                "All 256 wavetable slots are in use"
            } else {
                "Copy the Result to a new wavetable slot"
            })
            .clicked()
        {
            let values = preview.result.clone();
            slots.add_slot_from(&values);
        }

        ui.separator();
        ui.label(
            egui::RichText::new(format!("{} × {}", width, height as usize + 1))
                .color(theme::TEXT_DIM),
        );
    });
}

fn wave_slot_picker(shown: &mut usize, highest: usize, slot_count: usize) -> egui::DragValue<'_> {
    let picker = egui::DragValue::new(shown).range(0..=highest);
    if slot_count == 0 {
        picker.custom_formatter(|_, _| "-".to_string())
    } else {
        picker
    }
}
