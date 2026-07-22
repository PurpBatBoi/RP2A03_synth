//! rp2a03_ui\src\editor.rs
//! 
//! Layout rendering logic for the main sequence editor window.

use super::state::SharedSequences;
use super::widgets::draw_envelope_bar_graph;
use rp2a03_core::sequence::Sequence;

/// Renders the main editor layout.
pub fn render_editor_ui(ui: &mut egui::Ui, data: &mut SharedSequences) {
    ui.horizontal(|ui| {
        // Left Column: Instrument Settings Selector
        ui.vertical(|ui| {
            ui.set_width(180.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("Instrument settings").strong());
                ui.add_space(6.0);

                egui::Grid::new("seq_type_grid")
                    .num_columns(3)
                    .spacing([6.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("");
                        ui.label("#");
                        ui.label("Effect name");
                        ui.end_row();

                        let seq_types = [
                            ("Volume", 0),
                            ("Arpeggio", 1),
                            ("Pitch", 2),
                            ("Hi-pitch", 3),
                            ("Duty / Noise", 4),
                        ];

                        for (name, idx) in seq_types {
                            let enabled = match idx {
                                0 => &mut data.vol_enabled,
                                1 => &mut data.arp_enabled,
                                2 => &mut data.pitch_enabled,
                                3 => &mut data.hipitch_enabled,
                                _ => &mut data.duty_enabled,
                            };

                            ui.checkbox(enabled, "");
                            ui.label("0");

                            let is_selected = data.selected_tab == idx;
                            if ui.selectable_label(is_selected, name).clicked() {
                                data.selected_tab = idx;
                            }
                            ui.end_row();
                        }
                    });
            });
        });

        ui.add_space(10.0);

        // Right Column: Graphical Sequence Editor
        ui.vertical(|ui| {
            let tab = data.selected_tab;
            let (title, min_val, max_val) = match tab {
                0 => ("Volume", 0i16, 15i16),
                1 => ("Arpeggio", -96i16, 96i16),
                2 => ("Pitch", -128i16, 127i16),
                3 => ("Hi-pitch", -64i16, 63i16),
                _ => ("Duty / Noise", 0i16, 3i16),
            };

            let (text_ptr, seq_ptr) = match tab {
                0 => (&mut data.vol_text, &mut data.vol_seq),
                1 => (&mut data.arp_text, &mut data.arp_seq),
                2 => (&mut data.pitch_text, &mut data.pitch_seq),
                3 => (&mut data.hipitch_text, &mut data.hipitch_seq),
                _ => (&mut data.duty_text, &mut data.duty_seq),
            };

            ui.group(|ui| {
                ui.label(
                    egui::RichText::new(format!("Sequence editor - {}", title)).strong(),
                );
                ui.add_space(4.0);

                // Painter canvas
                draw_envelope_bar_graph(ui, seq_ptr, min_val, max_val);

                ui.add_space(6.0);

                // Step count modifier controls
                ui.horizontal(|ui| {
                    ui.label("Size:");

                    let cur_len = seq_ptr.len();
                    if ui.button("-").clicked() && cur_len > 1 {
                        let mut tokens: Vec<&str> = text_ptr.split_whitespace().collect();
                        for i in (0..tokens.len()).rev() {
                            if tokens[i].parse::<i16>().is_ok() {
                                tokens.remove(i);
                                break;
                            }
                        }
                        *text_ptr = tokens.join(" ");
                        let (parsed, norm) =
                            Sequence::parse_clamped(text_ptr, min_val, max_val);
                        *seq_ptr = parsed;
                        *text_ptr = norm;
                    }

                    ui.label(egui::RichText::new(format!("{}", cur_len)).strong());

                    if ui.button("+").clicked() {
                        text_ptr.push_str(" 0");
                        let (parsed, norm) =
                            Sequence::parse_clamped(text_ptr, min_val, max_val);
                        *seq_ptr = parsed;
                        *text_ptr = norm;
                    }

                    ui.add_space(15.0);
                    let duration_ms = (cur_len * 1000) / 60;
                    ui.label(format!("{} ms", duration_ms));
                });

                ui.add_space(6.0);

                // String editor
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(text_ptr)
                            .desired_width(420.0)
                            .font(egui::TextStyle::Monospace),
                    );

                    if edit.changed() {
                        let (parsed, norm) =
                            Sequence::parse_clamped(text_ptr, min_val, max_val);
                        *seq_ptr = parsed;
                        if *text_ptr != norm && !edit.has_focus() {
                            *text_ptr = norm;
                        }
                    }
                });
            });
        });
    });
}