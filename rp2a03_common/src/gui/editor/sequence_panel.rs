//! `rp2a03_common\src\gui\editor\sequence_panel.rs`
//! The sequence editor panel (the envelope/duty graph plus its text
//! readout and size controls) and the FamiTracker-style text format it
//! reads and writes — `sequence_to_text`/`sanitize_sequence_text` are also
//! `patch.rs`'s wire-format text encoding, not just this panel's.

use super::commit_if_changed;
use crate::gui::state::{SequencePlayheads, SharedSequences};
use crate::gui::widgets::{
    GraphStyle, draw_envelope_bar_graph, draw_s5b_duty_noise_graph, group_box, repeating_button,
};
use crate::{ChannelMode, Lane};
use rp2a03_core::sequencer::{ArpMode, PitchMode, Sequence, VolMode, VolMode5B};

#[must_use]
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

#[must_use]
pub fn sequence_to_text_for_tab(lane: Lane, channel_mode: ChannelMode, seq: &Sequence) -> String {
    if lane != Lane::Duty || channel_mode != ChannelMode::S5B {
        return sequence_to_text(seq);
    }

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

        let period = value & rp2a03_core::sequencer::S5B_PERIOD_MASK;
        let mut token = period.to_string();
        if value & rp2a03_core::sequencer::S5B_MODE_SQUARE != 0 {
            token.push('t');
        }
        if value & rp2a03_core::sequencer::S5B_MODE_NOISE != 0 {
            token.push('n');
        }

        let duty = rp2a03_core::sequencer::s5b_duty_index(*value);
        if duty != rp2a03_core::sequencer::S5B_DUTY_DEFAULT_INDEX {
            token.push('w');
            token.push_str(&duty.to_string());
        }
        tokens.push(token);
    }
    if seq.loop_point == Some(seq.values.len()) {
        tokens.push("|".to_string());
    }
    if seq.release_point == Some(seq.values.len()) {
        tokens.push("/".to_string());
    }
    tokens.join(" ")
}

#[must_use]
pub fn sanitize_sequence_text(text: &str) -> String {
    text.chars()
        .filter(|c| {
            c.is_ascii_digit()
                || matches!(*c, '|' | '/' | '-')
                || matches!(c.to_ascii_lowercase(), 't' | 'n' | 'w')
                || c.is_ascii_whitespace()
        })
        .collect()
}

fn parse_s5b_duty_text(input: &str) -> (Sequence, String) {
    let mut values = Vec::new();
    let mut loop_point = None;
    let mut release_point = None;
    let mut text_tokens = Vec::new();

    for token in input.split_whitespace() {
        match token {
            "|" | "L" | "l" => {
                loop_point = Some(values.len());
                text_tokens.push("|".to_string());
            }
            "/" | "R" | "r" => {
                release_point = Some(values.len());
                text_tokens.push("/".to_string());
            }
            _ => {
                let digits_end = token
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(token.len());
                let (digits, flags) = token.split_at(digits_end);
                if digits.is_empty() {
                    continue;
                }
                let Ok(period) = digits.parse::<i16>() else {
                    continue;
                };
                let period = period.clamp(0, 31);

                let mut value = period;
                let mut out_token = period.to_string();
                if flags.contains(['t', 'T']) {
                    value |= rp2a03_core::sequencer::S5B_MODE_SQUARE;
                    out_token.push('t');
                }
                if flags.contains(['n', 'N']) {
                    value |= rp2a03_core::sequencer::S5B_MODE_NOISE;
                    out_token.push('n');
                }

                if let Some(rest) = flags
                    .split(['w', 'W'])
                    .nth(1)
                    .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
                {
                    let digits_end = rest
                        .find(|c: char| !c.is_ascii_digit())
                        .unwrap_or(rest.len());
                    if let Ok(duty) = rest[..digits_end].parse::<i16>() {
                        let duty = duty.clamp(0, 8);
                        value = rp2a03_core::sequencer::s5b_set_duty_index(value, duty);
                        if duty != rp2a03_core::sequencer::S5B_DUTY_DEFAULT_INDEX {
                            out_token.push('w');
                            out_token.push_str(&duty.to_string());
                        }
                    }
                }

                values.push(value);
                text_tokens.push(out_token);
            }
        }
    }

    if values.is_empty() {
        values.push(0);
        text_tokens.push("0".to_string());
    }

    let normalized_text = text_tokens.join(" ");
    let sequence = Sequence {
        values,
        loop_point,
        release_point,
        ..Sequence::default()
    };
    (sequence, normalized_text)
}

fn set_volume_step_mode(text: &mut String, sequence: &mut Sequence, next: VolMode) {
    if sequence.vol_mode == next {
        return;
    }

    let scale_up = next == VolMode::Steps64;

    let (min_val, max_val) =
        Lane::Vol.value_range(ChannelMode::Vrc6Saw, next, sequence.vol_mode_5b, 0);

    for value in &mut sequence.values {
        *value = if scale_up { *value * 4 } else { *value / 4 };
        *value = (*value).clamp(min_val, max_val);
    }

    sequence.vol_mode = next;
    *text = sequence_to_text(sequence);
}

fn set_volume_step_mode_5b(text: &mut String, sequence: &mut Sequence, next: VolMode5B) {
    if sequence.vol_mode_5b == next {
        return;
    }

    let scale_up = next == VolMode5B::Steps32;

    let (min_val, max_val) = Lane::Vol.value_range(ChannelMode::S5B, sequence.vol_mode, next, 0);

    for value in &mut sequence.values {
        *value = if scale_up { *value * 2 } else { *value / 2 };
        *value = (*value).clamp(min_val, max_val);
    }

    sequence.vol_mode_5b = next;
    *text = sequence_to_text(sequence);
}

pub(super) fn sync_volume_step_mode_to_channel(
    data: &mut SharedSequences,
    channel_mode: ChannelMode,
) {
    if channel_mode == ChannelMode::Vrc6Saw {
        return;
    }
    if data.selected_sequence(Lane::Vol).vol_mode == VolMode::Steps16 {
        return;
    }

    let (text, sequence) = data.selected_sequence_mut(Lane::Vol);
    set_volume_step_mode(text, sequence, VolMode::Steps16);
}

pub(super) fn sync_volume_step_mode_5b_to_channel(
    data: &mut SharedSequences,
    channel_mode: ChannelMode,
) {
    if channel_mode == ChannelMode::S5B {
        return;
    }
    if data.selected_sequence(Lane::Vol).vol_mode_5b == VolMode5B::Steps16 {
        return;
    }

    let (text, sequence) = data.selected_sequence_mut(Lane::Vol);
    set_volume_step_mode_5b(text, sequence, VolMode5B::Steps16);
}

pub fn cleanup_tab_sequence(data: &mut SharedSequences, channel_mode: ChannelMode, lane: Lane) {
    let (sanitized, prev_pitch_mode, prev_arp_mode, prev_vol_mode, prev_vol_mode_5b) = {
        let (text, sequence) = data.selected_sequence_mut(lane);
        (
            sanitize_sequence_text(text),
            sequence.pitch_mode,
            sequence.arp_mode,
            sequence.vol_mode,
            sequence.vol_mode_5b,
        )
    };
    let slot_count = data.wave_slots().slot_count();
    let (min_val, max_val) =
        lane.value_range(channel_mode, prev_vol_mode, prev_vol_mode_5b, slot_count);
    let (text, sequence) = data.selected_sequence_mut(lane);

    if sanitized.trim().is_empty() {
        *sequence = Sequence::default();
        sequence.pitch_mode = prev_pitch_mode;
        sequence.arp_mode = prev_arp_mode;
        sequence.vol_mode = prev_vol_mode;
        sequence.vol_mode_5b = prev_vol_mode_5b;
        text.clear();
    } else {
        let (mut parsed, normalized) = if lane == Lane::Duty && channel_mode == ChannelMode::S5B {
            parse_s5b_duty_text(&sanitized)
        } else {
            Sequence::parse_clamped(&sanitized, min_val, max_val)
        };
        parsed.pitch_mode = prev_pitch_mode;
        parsed.arp_mode = prev_arp_mode;
        parsed.vol_mode = prev_vol_mode;
        parsed.vol_mode_5b = prev_vol_mode_5b;
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

/// The row under the graph: step count, playback length in ms, and whichever
/// lane/chip-specific mode toggle (arp/pitch mode, VRC6 64-step, S5B 32-step)
/// applies to the current lane.
#[allow(clippy::too_many_arguments)]
fn draw_sequence_size_row(
    ui: &mut egui::Ui,
    lane: Lane,
    channel_mode: ChannelMode,
    step_time_hz: u32,
    text: &mut String,
    sequence: &mut Sequence,
    auto_enable: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.label("Size:");

        let mut desired_len = sequence.len();

        if repeating_button(ui, "-") {
            desired_len = desired_len.saturating_sub(1);
        }

        ui.add_sized(
            [28.0, 18.0],
            egui::DragValue::new(&mut desired_len)
                .speed(1.0)
                .range(0..=256),
        );

        if repeating_button(ui, "+") {
            desired_len = desired_len.saturating_add(1).min(256);
        }

        if desired_len != sequence.len() {
            *auto_enable = true;
            sequence.values.resize(desired_len, 0);

            if sequence.loop_point.is_some_and(|p| p >= desired_len) {
                sequence.loop_point = None;
            }

            if sequence.release_point.is_some_and(|p| p >= desired_len) {
                sequence.release_point = None;
            }

            *text = sequence_to_text_for_tab(lane, channel_mode, sequence);
        }

        ui.add_space(15.0);

        ui.label(format!(
            "{} ms",
            (sequence.len() as u64 * 1000) / u64::from(step_time_hz).max(1)
        ));

        if lane == Lane::Arp {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.radio_value(&mut sequence.arp_mode, ArpMode::Absolute, "Absolute");

                ui.radio_value(&mut sequence.arp_mode, ArpMode::Relative, "Relative");

                ui.label(egui::RichText::new("Mode:").weak());
            });
        }

        if lane == Lane::Pitch {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.radio_value(&mut sequence.pitch_mode, PitchMode::Absolute, "Absolute");

                ui.radio_value(&mut sequence.pitch_mode, PitchMode::Relative, "Relative");

                ui.label(egui::RichText::new("Mode:").weak());
            });
        }

        if lane == Lane::Vol && channel_mode == ChannelMode::Vrc6Saw {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut is_64 = sequence.vol_mode == VolMode::Steps64;

                if ui.checkbox(&mut is_64, "64-Step").changed() {
                    let next = if is_64 {
                        VolMode::Steps64
                    } else {
                        VolMode::Steps16
                    };
                    set_volume_step_mode(text, sequence, next);
                    *auto_enable = true;
                }
            });
        }

        if lane == Lane::Vol && channel_mode == ChannelMode::S5B {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut is_32 = sequence.vol_mode_5b == VolMode5B::Steps32;

                if ui.checkbox(&mut is_32, "32-Step").changed() {
                    let next = if is_32 {
                        VolMode5B::Steps32
                    } else {
                        VolMode5B::Steps16
                    };
                    set_volume_step_mode_5b(text, sequence, next);
                    *auto_enable = true;
                }
            });
        }
    });
}

pub(super) fn draw_sequence_editor_panel(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    channel_mode: ChannelMode,
    playheads: &SequencePlayheads,
    step_time_hz: u32,
) {
    let lane = data.selected_tab;

    let title = lane.label(channel_mode);

    group_box(ui, &format!("Sequence editor - {title}"), |ui| {
        const CONTROLS_HEIGHT: f32 = 54.0;
        let graph_height = (ui.available_height() - CONTROLS_HEIGHT).max(150.0);
        let mut auto_enable = false;

        let vol_mode = data.selected_sequence(lane).vol_mode;
        let vol_mode_5b = data.selected_sequence(lane).vol_mode_5b;
        let slot_count = data.wave_slots().slot_count();
        let (min_val, max_val) = lane.value_range(channel_mode, vol_mode, vol_mode_5b, slot_count);

        let original = (
            data.selected_sequence_text(lane).to_string(),
            data.selected_sequence(lane).clone(),
        );

        let committed = commit_if_changed(&original, |(text, sequence)| {
            let style = if lane == Lane::Arp {
                GraphStyle::Arpeggio
            } else {
                GraphStyle::Bars
            };
            let graph_changed = if lane == Lane::Duty && channel_mode == ChannelMode::S5B {
                draw_s5b_duty_noise_graph(ui, sequence, playheads.step(lane), graph_height)
            } else {
                draw_envelope_bar_graph(
                    ui,
                    sequence,
                    min_val,
                    max_val,
                    style,
                    playheads.step(lane),
                    graph_height,
                )
            };
            if graph_changed {
                *text = sequence_to_text_for_tab(lane, channel_mode, sequence);
                auto_enable = true;
            }

            ui.add_space(6.0);

            draw_sequence_size_row(
                ui,
                lane,
                channel_mode,
                step_time_hz,
                text,
                sequence,
                &mut auto_enable,
            );

            ui.add_space(6.0);

            let edit = ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(ui.available_width())
                    .font(egui::TextStyle::Monospace),
            );

            let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));

            if edit.changed() {
                auto_enable = true;
                let sanitized = sanitize_sequence_text(text);

                let prev_pitch_mode = sequence.pitch_mode;
                let prev_arp_mode = sequence.arp_mode;
                let prev_vol_mode = sequence.vol_mode;
                let prev_vol_mode_5b = sequence.vol_mode_5b;

                *sequence = if sanitized.trim().is_empty() {
                    Sequence::default()
                } else if lane == Lane::Duty && channel_mode == ChannelMode::S5B {
                    parse_s5b_duty_text(&sanitized).0
                } else {
                    Sequence::parse_clamped(&sanitized, min_val, max_val).0
                };

                sequence.pitch_mode = prev_pitch_mode;
                sequence.arp_mode = prev_arp_mode;
                sequence.vol_mode = prev_vol_mode;
                sequence.vol_mode_5b = prev_vol_mode_5b;
            }

            if enter_pressed || edit.lost_focus() {
                *text = sequence_to_text_for_tab(lane, channel_mode, sequence);
            }
        });

        if let Some((text, sequence)) = committed {
            let (dst_text, dst_sequence) = data.selected_sequence_mut(lane);
            *dst_text = text;
            *dst_sequence = sequence;
        }

        if auto_enable {
            *data.sequence_enabled_mut(lane) = true;
        }
    });
}
