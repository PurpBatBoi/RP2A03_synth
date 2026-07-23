//! Layout rendering logic for the reusable sequence editor window.

use super::state::{SharedSequences, MAX_SEQUENCES};
use super::widgets::draw_envelope_bar_graph;
use rp2a03_core::sequence::Sequence;

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

fn sequence_range(tab: usize) -> (i16, i16) {
    match tab {
        0 => (0, 15),
        1 => (-96, 96),
        2 => (-128, 127),
        3 => (-64, 63),
        _ => (0, 3),
    }
}

/// Sanitizes the selected numbered sequence for an envelope type.
pub fn cleanup_tab_sequence(data: &mut SharedSequences, tab: usize) {
    let (min_val, max_val) = sequence_range(tab);
    let sanitized = {
        let (text, _) = data.selected_sequence_mut(tab);
        sanitize_sequence_text(text)
    };
    let (text, sequence) = data.selected_sequence_mut(tab);
    if sanitized.trim().is_empty() {
        *sequence = Sequence::default();
        text.clear();
    } else {
        let (mut parsed, normalized) = Sequence::parse_clamped(&sanitized, min_val, max_val);
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

/// Renders the pulse instrument settings and a generic selected-sequence editor.
///
/// Returns a new shared sequence number when the user changes the spinbox. The
/// plugin wrapper owns the automatable parameter and commits this change to the
/// host through its parameter setter.
pub fn render_editor_ui(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    shared_sequence_index: usize,
) -> Option<usize> {
    const SEQ_TYPES: [(&str, usize); 5] = [
        ("Volume", 0),
        ("Arpeggio", 1),
        ("Pitch", 2),
        ("Hi-pitch", 3),
        ("Duty / Noise", 4),
    ];

    data.set_all_selected_sequence_indices(shared_sequence_index);
    let mut changed_sequence_index = None;

    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(180.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("Instrument settings").strong());
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Sequence #:");
                    let mut index = shared_sequence_index;
                    if ui
                        .add(egui::DragValue::new(&mut index).range(0..=MAX_SEQUENCES - 1))
                        .changed()
                    {
                        changed_sequence_index = Some(index);
                    }
                });
                ui.add_space(6.0);
                egui::Grid::new("seq_type_grid")
                    .num_columns(2)
                    .spacing([6.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("");
                        ui.label("Effect name");
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
            });
        });
        ui.add_space(10.0);
        ui.vertical(|ui| {
            let tab = data.selected_tab;
            let (title, min_val, max_val) = match tab {
                0 => ("Volume", 0, 15),
                1 => ("Arpeggio", -96, 96),
                2 => ("Pitch", -128, 127),
                3 => ("Hi-pitch", -64, 63),
                _ => ("Duty / Noise", 0, 3),
            };
            ui.group(|ui| {
                ui.label(egui::RichText::new(format!("Sequence editor - {}", title)).strong());
                ui.add_space(4.0);
                let (text, sequence) = data.selected_sequence_mut(tab);
                if draw_envelope_bar_graph(ui, sequence, min_val, max_val) {
                    *text = sequence_to_text(sequence);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Size:");
                    let cur_len = sequence.len();
                    if ui.button("-").clicked() && cur_len > 0 {
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
                    ui.label(egui::RichText::new(cur_len.to_string()).strong());
                    if ui.button("+").clicked() {
                        sequence.values.push(0);
                        *text = sequence_to_text(sequence);
                    }
                    ui.add_space(15.0);
                    ui.label(format!("{} ms", (cur_len * 1000) / 60));
                });
                ui.add_space(6.0);
                let edit = ui.add(
                    egui::TextEdit::singleline(text)
                        .desired_width(420.0)
                        .font(egui::TextStyle::Monospace),
                );
                let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if edit.changed() {
                    let sanitized = sanitize_sequence_text(text);
                    *sequence = if sanitized.trim().is_empty() {
                        Sequence::default()
                    } else {
                        Sequence::parse_clamped(&sanitized, min_val, max_val).0
                    };
                }
                if enter_pressed || edit.lost_focus() {
                    *text = sequence_to_text(sequence);
                }
            });
        });
    });

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
}
