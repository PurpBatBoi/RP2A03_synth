//! `rp2a03_common\src\gui\wavetable_editor\mod.rs`
//! The Wavetable Editor tab: the wave graph plus its numeric readout row,
//! with a `generator` sidebar (`Shapes`/`WaveTools` sub-tabs) alongside it.

mod generator;
mod tools;

use crate::gui::wavetable_state::{
    DisplayMode, DrawStyle, FDS_WAVE_LEN, FDS_WAVE_MAX, ShapesGenState, WaveSlots,
    WavetableEditorState,
};
use crate::gui::widgets::draw_wavetable_graph;
use generator::{draw_generator_panel, generate_shape};

const READOUT_RESERVED_HEIGHT: f32 = 24.0;

const READOUT_GAP: f32 = 6.0;

const MIN_GRAPH_HEIGHT: f32 = 120.0;

const GENERATOR_WIDTH: f32 = 250.0;

const COLUMN_GAP: f32 = 8.0;

const MIN_GRAPH_WIDTH: f32 = 200.0;

fn new_slot_shape() -> Vec<u16> {
    let mut values = vec![0u16; FDS_WAVE_LEN];
    generate_shape(&mut values, FDS_WAVE_MAX, &ShapesGenState::default());
    values
}

pub fn draw_wavetable_editor_panel(
    ui: &mut egui::Ui,
    state: &mut WavetableEditorState,
    slots: &mut WaveSlots,
) {
    ui.add_space(10.0);

    ui.horizontal(|ui| {
        ui.label("Wavetable:");

        let slot_count = slots.slot_count();
        let mut shown = slots.current_slot();

        let highest = slot_count.saturating_sub(1);

        if ui
            .add_enabled(!slots.is_empty(), egui::Button::new("−"))
            .on_hover_text("Remove this wavetable slot")
            .clicked()
        {
            slots.remove_slot();
        }

        let picker = egui::DragValue::new(&mut shown).range(0..=highest);

        let picker = if slots.is_empty() {
            picker.custom_formatter(|_, _| "-".to_string())
        } else {
            picker
        };
        if ui.add_enabled(slot_count > 1, picker).changed() {
            slots.set_current_slot(shown);
        }

        let hover = if slots.is_full() {
            "All 256 wavetable slots are in use"
        } else if slots.is_empty() {
            "Create the first wavetable"
        } else {
            "Add a copy of this wavetable as a new slot"
        };
        if ui
            .add_enabled(!slots.is_full(), egui::Button::new("+"))
            .on_hover_text(hover)
            .clicked()
        {
            if slots.is_empty() {
                slots.add_slot_from(&new_slot_shape());
            } else {
                slots.add_slot();
            }
        }

        let slot_count = slots.slot_count();
        ui.label(
            egui::RichText::new(format!(
                "{} slot{}",
                slot_count,
                if slot_count == 1 { "" } else { "s" }
            ))
            .color(crate::gui::theme::TEXT_DIM),
        );

        ui.separator();

        ui.radio_value(&mut state.draw_style, DrawStyle::Steps, "Steps");
        ui.radio_value(&mut state.draw_style, DrawStyle::Lines, "Lines");

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (glyph, tip) = if state.generator_visible {
                ("▶", "Hide the generator panel")
            } else {
                ("◀", "Show the generator panel")
            };
            if ui.button(glyph).on_hover_text(tip).clicked() {
                state.generator_visible = !state.generator_visible;
            }
        });
    });

    ui.add_space(8.0);

    let body = ui.available_rect_before_wrap();
    ui.allocate_rect(body, egui::Sense::hover());

    let readout_rect = egui::Rect::from_min_max(
        egui::pos2(body.min.x, body.max.y - READOUT_RESERVED_HEIGHT),
        body.max,
    );
    let graph_rect = egui::Rect::from_min_max(
        body.min,
        egui::pos2(body.max.x, readout_rect.min.y - READOUT_GAP),
    );
    let graph_height = graph_rect.height().max(MIN_GRAPH_HEIGHT);

    ui.scope_builder(egui::UiBuilder::new().max_rect(graph_rect), |ui| {
        draw_graph_row(ui, state, slots, graph_height);
    });

    ui.scope_builder(egui::UiBuilder::new().max_rect(readout_rect), |ui| {
        draw_readout_row(ui, state, slots);
    });
}

fn draw_readout_row(ui: &mut egui::Ui, state: &mut WavetableEditorState, slots: &mut WaveSlots) {
    ui.horizontal(|ui| {
        let mode_changed = ui
            .radio_value(&mut state.display, DisplayMode::Dec, "Dec")
            .changed()
            | ui.radio_value(&mut state.display, DisplayMode::Hex, "Hex")
                .changed();

        let sign_changed = ui
            .add_enabled(
                state.display == DisplayMode::Dec,
                egui::Button::selectable(state.signed, if state.signed { "±" } else { "+" }),
            )
            .on_hover_text("Signed/Unsigned")
            .clicked();
        if sign_changed {
            state.signed = !state.signed;
        }

        if slots.current().is_none() {
            state.readout_text.clear();
            ui.add_enabled(
                false,
                egui::TextEdit::singleline(&mut state.readout_text)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
            return;
        }
        let max = FDS_WAVE_MAX;

        let edit = ui.add(
            egui::TextEdit::singleline(&mut state.readout_text)
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace),
        );

        if edit.changed() {
            let parsed = decode_readout(&state.readout_text, max, state.display, state.signed);

            if !parsed.is_empty() {
                slots.current_mut().unwrap().set_data(&parsed);
            }
        }

        if !edit.has_focus() || mode_changed || sign_changed {
            state.readout_text = encode_readout(
                slots.current().unwrap().data(),
                max,
                state.display,
                state.signed,
            );
        }
    });
}

fn draw_graph_row(
    ui: &mut egui::Ui,
    state: &mut WavetableEditorState,
    slots: &mut WaveSlots,
    graph_height: f32,
) {
    ui.horizontal_top(|ui| {
        let graph_width = if state.generator_visible {
            (ui.available_width() - GENERATOR_WIDTH - COLUMN_GAP).max(MIN_GRAPH_WIDTH)
        } else {
            ui.available_width()
        };

        let style = state.draw_style;

        let mut empty = [];
        let data: &mut [u16] = match slots.current_mut() {
            Some(slot) => slot.data_mut(),
            None => &mut empty,
        };

        ui.scope(|ui| {
            ui.set_width(graph_width);
            draw_wavetable_graph(ui, data, FDS_WAVE_MAX, style, graph_height, true);
        });

        if state.generator_visible {
            ui.add_space(COLUMN_GAP);
            draw_generator_panel(ui, state, slots, graph_height);
        }
    });
}

fn signed_bias(max: u16, mode: DisplayMode, signed: bool) -> i32 {
    if signed && mode == DisplayMode::Dec {
        i32::from(max.div_ceil(2))
    } else {
        0
    }
}

fn encode_readout(data: &[u16], max: u16, mode: DisplayMode, signed: bool) -> String {
    let bias = signed_bias(max, mode, signed);

    data.iter()
        .map(|&v| match mode {
            DisplayMode::Dec => (i32::from(v) - bias).to_string(),
            DisplayMode::Hex => format!("{v:02X}"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_readout(text: &str, max: u16, mode: DisplayMode, signed: bool) -> Vec<u16> {
    let bias = signed_bias(max, mode, signed);

    let (lo, hi) = if bias > 0 {
        (-bias, i32::from(max) / 2)
    } else {
        (0, i32::from(max))
    };

    text.split_whitespace()
        .filter_map(|token| match mode {
            DisplayMode::Dec => token.parse::<i32>().ok(),

            DisplayMode::Hex => i32::from_str_radix(token, 16).ok(),
        })
        .map(|v| (v.clamp(lo, hi) + bias) as u16)
        .take(FDS_WAVE_LEN)
        .collect()
}
