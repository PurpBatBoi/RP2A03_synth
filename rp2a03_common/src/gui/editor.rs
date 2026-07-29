//! rp2a03_common\src\gui\editor.rs
//! Layout rendering logic for the reusable sequence editor window.

use super::state::{SequencePlayheads, SharedSequences, MAX_SEQUENCES};
use super::widgets::{draw_envelope_bar_graph, group_box, repeating_button};
use rp2a03_core::sequencer::{ArpMode, PitchMode, Sequence};

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
/// For arpeggio, `arp_mode` changes the range: Fixed uses 0..=95, others use -96..=96.
fn sequence_range(tab: usize, arp_mode: ArpMode) -> (i16, i16) {
    match tab {
        0 => (0, 15),
        1 => match arp_mode {
            ArpMode::Fixed => (0, 95),
            _ => (-96, 96),
        },
        2 | 3 => (-128, 127),
        _ => (0, 3),
    }
}

/// Sanitizes the selected numbered sequence for an envelope type.
pub fn cleanup_tab_sequence(data: &mut SharedSequences, tab: usize) {
    let (sanitized, prev_pitch_mode, prev_arp_mode) = {
        let (text, sequence) = data.selected_sequence_mut(tab);
        (
            sanitize_sequence_text(text),
            sequence.pitch_mode,
            sequence.arp_mode,
        )
    };
    let (min_val, max_val) = sequence_range(tab, prev_arp_mode);
    let (text, sequence) = data.selected_sequence_mut(tab);
    if sanitized.trim().is_empty() {
        *sequence = Sequence::default();
        sequence.pitch_mode = prev_pitch_mode;
        sequence.arp_mode = prev_arp_mode;
        text.clear();
    } else {
        let (mut parsed, normalized) = Sequence::parse_clamped(&sanitized, min_val, max_val);
        parsed.pitch_mode = prev_pitch_mode;
        parsed.arp_mode = prev_arp_mode;
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

fn draw_header(ui: &mut egui::Ui) {
    let mut waveform = 0;
    let mut polyphony = false;
    let mut legato = false;
    let mut portamento = false;
    let mut portamento_amount = 24u8;

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
                egui::ComboBox::from_id_salt("waveform")
                    .width(180.0)
                    .selected_text(match waveform {
                        0 => "2A03 | Pulse",
                        1 => "2A03 | Triangle",
                        _ => "2A03 | Noise",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut waveform, 0, "2A03 | Pulse");
                        ui.selectable_value(&mut waveform, 1, "2A03 | Triangle");
                        ui.selectable_value(&mut waveform, 2, "2A03 | Noise");
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
                    ui.checkbox(&mut polyphony, "Polyphony");
                    // ui.checkbox(&mut legato, "Legato");
                    // ui.checkbox(&mut portamento, "Portamento");
                })
                .response
            },
        );

        let check_row_w = check_row.inner.rect.width();

        //------------------------------------------------------
        // Portamento knob
        //------------------------------------------------------

        // const CONTROL_W: f32 = 100.0;
        // const CONTROL_H: f32 = 50.0;
        // const CONTROL_GAP_FROM_ROW: f32 = 20.0;

        // let control_x = controls_x + check_row_w + CONTROL_GAP_FROM_ROW;
        // let control_top = CENTER_Y - CONTROL_H / 2.0;

        // ui.scope_builder(
        //     egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
        //         egui::pos2(origin.x + control_x, origin.y + control_top),
        //         egui::vec2(CONTROL_W, CONTROL_H),
        //     )),
        //     |ui| {
        //         ui.vertical_centered(|ui| {
        //             ui.label("Portamento Amount");
        //             ui.add(
        //                 egui::DragValue::new(&mut portamento_amount)
        //                     .range(0..=127)
        //                     .speed(1.0),
        //             );
        //         });
        //     },
        // );
    });

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(5.0);
}

fn draw_chip_tabs(ui: &mut egui::Ui, _data: &mut SharedSequences) {
    ui.horizontal(|ui| {
        let _ = ui.selectable_label(true, "Envelopes");
    });

    ui.separator();
}

fn draw_instrument_settings_panel(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    shared_sequence_index: usize,
    changed_sequence_index: &mut Option<usize>,
) {
    const SEQ_TYPES: [(&str, usize); 5] = [
        ("Volume", 0),
        ("Arpeggio", 1),
        ("Pitch", 2),
        ("Hi-Pitch", 3),
        ("Duty / Noise", 4),
    ];

    ui.vertical(|ui| {
        ui.set_width(180.0);
        ui.add_space(16.0);
        group_box(ui, "Instrument settings", |ui| {
            // Grid with checkboxes and effect names
            egui::Grid::new("seq_type_grid")
                .num_columns(2)
                .spacing([6.0, 6.0])
                .show(ui, |ui| {
                    ui.label("");
                    ui.label(
                        egui::RichText::new("Effect name")
                            .color(egui::Color32::from_rgb(130, 130, 130)), // Darker text color
                    );
                    ui.end_row();

                    // Separator line under header row
                    ui.separator();
                    ui.separator();
                    ui.end_row();

                    for (name, tab) in SEQ_TYPES {
                        ui.checkbox(data.sequence_enabled_mut(tab), "");
                        if ui
                            .selectable_label(data.selected_tab == tab, name)
                            .clicked()
                        {
                            if data.selected_tab != tab {
                                cleanup_tab_sequence(data, data.selected_tab);
                                data.selected_tab = tab;
                                cleanup_tab_sequence(data, tab);
                            }
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
                    *changed_sequence_index = Some(index);
                }
            });
        });
    });
}

fn draw_sequence_editor_panel(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    playheads: &SequencePlayheads,
) {
    ui.vertical(|ui| {
        let tab = data.selected_tab;

        let title = match tab {
            0 => "Volume",
            1 => "Arpeggio",
            2 => "Pitch",
            3 => "Hi-pitch",
            _ => "Duty / Noise",
        };

        ui.add_space(16.0);
        group_box(ui, &format!("Sequence editor - {}", title), |ui| {
            let (text, sequence) = data.selected_sequence_mut(tab);

            let (min_val, max_val) = sequence_range(tab, sequence.arp_mode);

            let is_arpeggio = tab == 1;
            if draw_envelope_bar_graph(
                ui,
                sequence,
                min_val,
                max_val,
                is_arpeggio,
                playheads.step(tab),
            ) {
                *text = sequence_to_text(sequence);
            }

            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.label("Size:");

                let cur_len = sequence.len();

                if repeating_button(ui, "-") && cur_len > 0 {
                    sequence.values.pop();

                    let new_len = sequence.len();

                    if sequence.loop_point.is_some_and(|p| p >= new_len) {
                        sequence.loop_point = None;
                    }

                    if sequence.release_point.is_some_and(|p| p >= new_len) {
                        sequence.release_point = None;
                    }

                    *text = sequence_to_text(sequence);
                }

                ui.add_sized(
                    [28.0, 18.0],
                    egui::Label::new(egui::RichText::new(cur_len.to_string()).strong()),
                );

                if repeating_button(ui, "+") {
                    sequence.values.push(0);
                    *text = sequence_to_text(sequence);
                }

                ui.add_space(15.0);

                ui.label(format!("{} ms", (cur_len * 1000) / 60));

                if tab == 1 {
                    // Arpeggio mode ComboBox (Absolute / Relative / Fixed)
                    // placed right-to-left, mirroring the Pitch mode radio buttons.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let prev_arp_mode = sequence.arp_mode;
                        let mode_label = match sequence.arp_mode {
                            ArpMode::Absolute => "Absolute",
                            ArpMode::Relative => "Relative",
                            ArpMode::Fixed    => "Fixed",
                        };
                        egui::ComboBox::from_id_salt("arp_mode_combo")
                            .selected_text(mode_label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut sequence.arp_mode,
                                    ArpMode::Absolute,
                                    "Absolute",
                                );
                                ui.selectable_value(
                                    &mut sequence.arp_mode,
                                    ArpMode::Relative,
                                    "Relative",
                                );
                                ui.selectable_value(
                                    &mut sequence.arp_mode,
                                    ArpMode::Fixed,
                                    "Fixed",
                                );
                            });
                        if sequence.arp_mode != prev_arp_mode {
                            // Re-clamp all step values to the new mode's valid range
                            // (e.g. Fixed 0..=95 vs Absolute/Relative -96..=96).
                            let (new_min, new_max) = sequence_range(1, sequence.arp_mode);
                            for v in &mut sequence.values {
                                *v = (*v).clamp(new_min, new_max);
                            }
                            *text = sequence_to_text(sequence);
                        }
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
            });

            ui.add_space(6.0);

            let edit = ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(ui.available_width())
                    .font(egui::TextStyle::Monospace),
            );

            let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));

            if edit.changed() {
                let sanitized = sanitize_sequence_text(text);
                let prev_mode = sequence.pitch_mode;

                *sequence = if sanitized.trim().is_empty() {
                    Sequence::default()
                } else {
                    Sequence::parse_clamped(&sanitized, min_val, max_val).0
                };

                sequence.pitch_mode = prev_mode;
            }

            if enter_pressed || edit.lost_focus() {
                *text = sequence_to_text(sequence);
            }
        });
    });
}

fn draw_main_content(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    shared_sequence_index: usize,
    changed_sequence_index: &mut Option<usize>,
    playheads: &SequencePlayheads,
) {
    ui.horizontal(|ui| {
        draw_instrument_settings_panel(ui, data, shared_sequence_index, changed_sequence_index);

        ui.add_space(10.0);

        ui.vertical(|ui| {
            ui.set_min_width(ui.available_width());

            draw_sequence_editor_panel(ui, data, playheads);
        });
    });
}

fn draw_footer(ui: &mut egui::Ui) {
    ui.separator();
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("0.0.05").weak());
        // ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        //     ui.button("Settings")
        // });
    });
}

pub fn render_editor_ui(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    shared_sequence_index: usize,
    playheads: &SequencePlayheads,
) -> Option<usize> {
    // Apply non-selectable labels to the global Context style so all child scopes,
    // grids, and group boxes inherit it
    ui.ctx().global_style_mut(|style| {
        style.interaction.selectable_labels = false;
    });

    ui.set_min_height(ui.available_height());

    data.set_all_selected_sequence_indices(shared_sequence_index);

    let mut changed_sequence_index = None;

    draw_header(ui);
    draw_chip_tabs(ui, data);
    draw_main_content(
        ui,
        data,
        shared_sequence_index,
        &mut changed_sequence_index,
        playheads,
    );

    // Calculate remaining vertical space and push footer to the bottom edge
    const FOOTER_HEIGHT: f32 = 30.0;
    let space_to_bottom = (ui.available_height() - FOOTER_HEIGHT).max(0.0);
    ui.add_space(space_to_bottom);

    draw_footer(ui);

    changed_sequence_index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_text_round_trips_markers() {
        let mut sequence = Sequence::default();
        sequence.values = vec![15, 12, 10];
        sequence.loop_point = Some(1);
        sequence.release_point = Some(1);

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
        assert_eq!(sequence_range(2, ArpMode::Absolute), (-128, 127));
        assert_eq!(sequence_range(3, ArpMode::Absolute), (-128, 127));

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
}
