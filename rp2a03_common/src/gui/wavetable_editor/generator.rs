//! `rp2a03_common\src\gui\wavetable_editor\generator.rs`
//! The generator sidebar's "Shapes" tab: additive-synthesis wave generation
//! from a base shape plus up to 16 partials.

use super::tools::draw_wavetools_tab;
use crate::gui::wavetable_state::{
    BaseShape, FDS_WAVE_MAX, GeneratorTab, PARTIAL_COUNT, ShapesGenState, WaveSlots,
    WavetableEditorState,
};

pub(super) fn generate_shape(data: &mut [u16], max: u16, params: &ShapesGenState) {
    let len = data.len();
    if len < 2 {
        return;
    }

    let len_f = len as f64;

    let mut result = vec![0.0f64; len];

    for (i, out) in result.iter_mut().enumerate() {
        for j in 0..PARTIAL_COUNT {
            let pos =
                ((f64::from(params.phase[j]) * len_f) + (i * (j + 1)) as f64).rem_euclid(len_f);

            let partial = match params.shape {
                BaseShape::Sine => ((0.5 + pos) * 2.0 * std::f64::consts::PI / len_f).sin(),
                BaseShape::Triangle => 4.0 * (0.5 - (0.5 - (pos / (len_f - 1.0))).abs()) - 1.0,
                BaseShape::Saw => ((2.0 * pos) / (len_f - 1.0)) - 1.0,
                BaseShape::Pulse => {
                    if pos >= f64::from(params.duty) * len_f {
                        1.0
                    } else {
                        -1.0
                    }
                }
            };

            *out += partial.powi(params.exponent) * f64::from(params.amp[j]);
        }
    }

    let invert_from = (f64::from(params.invert_point) * len_f) as usize;
    for v in result.iter_mut().skip(invert_from) {
        *v = -*v;
    }

    for (out, v) in data.iter_mut().zip(result) {
        let normalized = f64::midpoint(1.0, v).clamp(0.0, 1.0);
        *out = (normalized * f64::from(max)).round() as u16;
    }
}

pub(super) fn draw_generator_panel(
    ui: &mut egui::Ui,
    state: &mut WavetableEditorState,
    slots: &mut WaveSlots,
    height: f32,
) {
    ui.allocate_ui(egui::vec2(super::GENERATOR_WIDTH, height), |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut state.generator_tab, GeneratorTab::Shapes, "Shapes");
                ui.selectable_value(
                    &mut state.generator_tab,
                    GeneratorTab::WaveTools,
                    "WaveTools",
                );
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match state.generator_tab {
                    GeneratorTab::Shapes => {
                        ui.add_enabled_ui(!slots.is_empty(), |ui| {
                            if draw_shapes_tab(ui, &mut state.shapes) {
                                let params = state.shapes.clone();
                                if let Some(slot) = slots.current_mut() {
                                    generate_shape(slot.data_mut(), FDS_WAVE_MAX, &params);
                                }
                            }
                        });
                    }
                    GeneratorTab::WaveTools => {
                        ui.add_enabled_ui(!slots.is_empty(), |ui| {
                            draw_wavetools_tab(ui, state, slots);
                        });
                    }
                });
        });
    });
}

fn draw_shapes_tab(ui: &mut egui::Ui, params: &mut ShapesGenState) -> bool {
    let mut changed = false;

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (shape, label) in BaseShape::ALL {
            changed |= ui
                .selectable_value(&mut params.shape, shape, label)
                .changed();
        }
    });

    ui.add_space(6.0);

    egui::Grid::new("wavetable_shapes_params")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Duty");
            changed |= ui
                .add(egui::Slider::new(&mut params.duty, 0.0..=1.0).show_value(true))
                .changed();
            ui.end_row();

            ui.label("Exponent");
            changed |= ui
                .add(egui::Slider::new(&mut params.exponent, 1..=8))
                .changed();
            ui.end_row();

            ui.label("XOR Point");
            changed |= ui
                .add(egui::Slider::new(&mut params.invert_point, 0.0..=1.0).show_value(true))
                .changed();
            ui.end_row();
        });

    ui.add_space(4.0);

    egui::CollapsingHeader::new("Amplitude/Phase")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("wavetable_partials")
                .num_columns(3)
                .spacing([6.0, 2.0])
                .show(ui, |ui| {
                    for i in 0..PARTIAL_COUNT {
                        ui.label(format!("{}", i + 1));
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut params.amp[i])
                                    .speed(0.01)
                                    .range(-1.0..=1.0),
                            )
                            .on_hover_text("Amplitude")
                            .changed();
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut params.phase[i])
                                    .speed(0.01)
                                    .range(0.0..=1.0),
                            )
                            .on_hover_text("Phase")
                            .changed();
                        ui.end_row();
                    }
                });
        });

    changed
}
