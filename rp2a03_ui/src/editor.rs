//! rp2a03_ui\src\editor.rs
//! 
//! Layout rendering logic for the main sequence editor window.

use super::state::SharedSequences;
use super::widgets::draw_envelope_bar_graph;
use rp2a03_core::sequence::Sequence;

/// Strips non-sequence characters from raw text input.
/// Retains digits, minus sign (-), loop marker (|), release marker (/), and whitespace.
pub fn sanitize_sequence_text(text: &str) -> String {
    text.chars()
        .filter(|c| {
            c.is_ascii_digit() || *c == '|' || *c == '/' || *c == '-' || c.is_ascii_whitespace()
        })
        .collect()
}

/// Sanitizes text input and updates sequence state for a specific tab index.
pub fn cleanup_tab_sequence(data: &mut SharedSequences, tab: usize) {
    let (min_val, max_val, default_str) = match tab {
        0 => (0i16, 15i16, "15"),
        1 => (-96i16, 96i16, "0"),
        2 => (-128i16, 127i16, "0"),
        3 => (-64i16, 63i16, "0"),
        _ => (0i16, 3i16, "2"),
    };

    let (text_ptr, seq_ptr) = match tab {
        0 => (&mut data.vol_text, &mut data.vol_seq),
        1 => (&mut data.arp_text, &mut data.arp_seq),
        2 => (&mut data.pitch_text, &mut data.pitch_seq),
        3 => (&mut data.hipitch_text, &mut data.hipitch_seq),
        _ => (&mut data.duty_text, &mut data.duty_seq),
    };

    let sanitized = sanitize_sequence_text(text_ptr);
    if sanitized.trim().is_empty() {
        let (parsed, _) = Sequence::parse_clamped(default_str, min_val, max_val);
        *seq_ptr = parsed;
        *text_ptr = String::new();
    } else {
        let (parsed, norm) = Sequence::parse_clamped(&sanitized, min_val, max_val);
        *seq_ptr = parsed;
        *text_ptr = norm;
    }
}

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
                                if data.selected_tab != idx {
                                    cleanup_tab_sequence(data, data.selected_tab);
                                    data.selected_tab = idx;
                                    cleanup_tab_sequence(data, idx);
                                }
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
            let (title, min_val, max_val, default_str) = match tab {
                0 => ("Volume", 0i16, 15i16, "15"),
                1 => ("Arpeggio", -96i16, 96i16, "0"),
                2 => ("Pitch", -128i16, 127i16, "0"),
                3 => ("Hi-pitch", -64i16, 63i16, "0"),
                _ => ("Duty / Noise", 0i16, 3i16, "2"),
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
                        let sanitized = sanitize_sequence_text(text_ptr);
                        if sanitized.trim().is_empty() {
                            let (parsed, _) = Sequence::parse_clamped(default_str, min_val, max_val);
                            *seq_ptr = parsed;
                            *text_ptr = String::new();
                        } else {
                            let (parsed, norm) = Sequence::parse_clamped(&sanitized, min_val, max_val);
                            *seq_ptr = parsed;
                            *text_ptr = norm;
                        }
                    }

                    ui.label(egui::RichText::new(format!("{}", cur_len)).strong());

                    if ui.button("+").clicked() {
                        if text_ptr.trim().is_empty() {
                            *text_ptr = format!("{} 0", default_str);
                        } else {
                            text_ptr.push_str(" 0");
                        }
                        let sanitized = sanitize_sequence_text(text_ptr);
                        let (parsed, norm) = Sequence::parse_clamped(&sanitized, min_val, max_val);
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

                    let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));

                    if edit.changed() {
                        let sanitized = sanitize_sequence_text(text_ptr);
                        let parse_input = if sanitized.trim().is_empty() {
                            default_str
                        } else {
                            &sanitized
                        };
                        let (parsed, _) = Sequence::parse_clamped(parse_input, min_val, max_val);
                        *seq_ptr = parsed;
                    }

                    if enter_pressed || edit.lost_focus() {
                        let sanitized = sanitize_sequence_text(text_ptr);
                        if sanitized.trim().is_empty() {
                            let (parsed, _) = Sequence::parse_clamped(default_str, min_val, max_val);
                            *seq_ptr = parsed;
                            *text_ptr = String::new();
                        } else {
                            let (parsed, norm) = Sequence::parse_clamped(&sanitized, min_val, max_val);
                            *seq_ptr = parsed;
                            *text_ptr = norm;
                        }
                    }
                });
            });
        });
    });
}