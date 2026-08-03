//! rp2a03_common\src\gui\editor.rs
//! Layout rendering logic for the reusable sequence editor window.

use super::state::{MAX_SEQUENCES, SequencePlayheads, SharedSequences};
use super::widgets::{draw_envelope_bar_graph, group_box, repeating_button};
use crate::ChannelMode;
use rp2a03_core::sequencer::{ArpMode, PitchMode, Sequence, VolMode};

/// Result returned by [`render_editor_ui`] to communicate parameter changes back
/// to the host plugin wrapper.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditorResult {
    /// If the user changed the shared sequence index via the GUI.
    pub new_sequence_index: Option<usize>,
    /// If the user changed the step time Hz value via the GUI.
    pub new_step_time_hz: Option<i32>,
    /// If the user changed the channel mode via the waveform combobox.
    pub new_channel_mode: Option<ChannelMode>,
    /// If the user changed the polyphony toggle.
    pub new_polyphony: Option<bool>,
    /// If the user changed the maximum voice count.
    pub new_max_voices: Option<i32>,
    pub new_portamento_enabled: Option<bool>,
    pub new_portamento_speed: Option<i32>,
}

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
///
/// `vol_mode` only matters for tab 0 on the VRC6 sawtooth: dn stores the
/// 16-step/64-step setting per volume sequence (`CSequence::GetSetting`), and the
/// bar graph max follows it (SequenceEditor.cpp: `0x0F` vs `0x3F`).
fn sequence_range(tab: usize, channel_mode: ChannelMode, vol_mode: VolMode) -> (i16, i16) {
    match tab {
        0 => {
            if channel_mode == ChannelMode::Vrc6Saw && vol_mode == VolMode::Steps64 {
                (0, 63)
            } else {
                (0, 15)
            }
        }
        1 => (-96, 96),
        2 | 3 => (-128, 127),
        _ => match channel_mode {
            ChannelMode::Vrc6Pulse => (0, 7),
            // dn `CVRC6Sawtooth::MAX_DUTY = 0x01` — the saw's duty is a single bit
            // that becomes bit 5 of the $B000 accumulator rate.
            ChannelMode::Vrc6Saw => (0, 1),
            _ => (0, 3),
        },
    }
}

/// Switches a volume sequence between the 4-bit and 6-bit step ranges.
///
/// dn remaps the existing steps when the setting flips (`SequenceSetting.cpp`:
/// `x * 4` / `x / 4`) so the envelope keeps its shape instead of flattening
/// against the new ceiling. Halving is lossy — the low two bits are gone for good.
fn set_volume_step_mode(text: &mut String, sequence: &mut Sequence, next: VolMode) {
    if sequence.vol_mode == next {
        return;
    }

    let scale_up = next == VolMode::Steps64;
    let (min_val, max_val) = sequence_range(0, ChannelMode::Vrc6Saw, next);

    for value in &mut sequence.values {
        *value = if scale_up { *value * 4 } else { *value / 4 };
        *value = (*value).clamp(min_val, max_val);
    }

    sequence.vol_mode = next;
    *text = sequence_to_text(sequence);
}

/// Brings the selected volume sequence back to the 4-bit range once the editor is
/// no longer on the VRC6 sawtooth.
///
/// 64-step is a saw-only setting, so leaving that waveform has to behave like
/// unticking the toggle — otherwise the stored steps keep their 0..63 values while
/// every other channel reads them through a 0..15 clamp, and the text box disagrees
/// with the graph. Idempotent, so it is safe to run every frame and it catches host
/// automation and state recall as well as the waveform combobox.
fn sync_volume_step_mode_to_channel(data: &mut SharedSequences) {
    if data.channel_mode == ChannelMode::Vrc6Saw {
        return;
    }

    let (text, sequence) = data.selected_sequence_mut(0);
    set_volume_step_mode(text, sequence, VolMode::Steps16);
}

/// Sanitizes the selected numbered sequence for an envelope type.
pub fn cleanup_tab_sequence(data: &mut SharedSequences, tab: usize) {
    let (sanitized, prev_pitch_mode, prev_arp_mode, prev_vol_mode) = {
        let (text, sequence) = data.selected_sequence_mut(tab);
        (
            sanitize_sequence_text(text),
            sequence.pitch_mode,
            sequence.arp_mode,
            sequence.vol_mode,
        )
    };
    let (min_val, max_val) = sequence_range(tab, data.channel_mode, prev_vol_mode);
    let (text, sequence) = data.selected_sequence_mut(tab);

    if sanitized.trim().is_empty() {
        *sequence = Sequence::default();
        sequence.pitch_mode = prev_pitch_mode;
        sequence.arp_mode = prev_arp_mode;
        sequence.vol_mode = prev_vol_mode;
        text.clear();
    } else {
        let (mut parsed, normalized) = Sequence::parse_clamped(&sanitized, min_val, max_val);
        parsed.pitch_mode = prev_pitch_mode;
        parsed.arp_mode = prev_arp_mode;
        parsed.vol_mode = prev_vol_mode;
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

fn draw_header(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    changed_channel_mode: &mut Option<ChannelMode>,
    changed_polyphony: &mut Option<bool>,
    changed_portamento_enabled: &mut Option<bool>,
) {
    let mut portamento = data.portamento_enabled;

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
                let mut waveform_id = data.channel_mode as i32;
                egui::ComboBox::from_id_salt("waveform")
                    .width(180.0)
                    .selected_text(match data.channel_mode {
                        ChannelMode::Pulse => "2A03 | Pulse",
                        ChannelMode::Triangle => "2A03 | Triangle",
                        ChannelMode::Noise => "2A03 | Noise",
                        ChannelMode::Vrc6Pulse => "VRC6 | Pulse",
                        ChannelMode::Vrc6Saw => "VRC6 | Saw",
                    })
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(&mut waveform_id, 0, "2A03 | Pulse")
                            .clicked()
                        {
                            let new_mode = ChannelMode::Pulse;
                            data.channel_mode = new_mode;
                            *changed_channel_mode = Some(new_mode);
                        }
                        if ui
                            .selectable_value(&mut waveform_id, 1, "2A03 | Triangle")
                            .clicked()
                        {
                            let new_mode = ChannelMode::Triangle;
                            // If Duty tab was active, revert to Volume
                            if data.selected_tab == 4 {
                                cleanup_tab_sequence(data, 4);
                                data.selected_tab = 0;
                            }
                            data.channel_mode = new_mode;
                            *changed_channel_mode = Some(new_mode);
                        }
                        if ui
                            .selectable_value(&mut waveform_id, 2, "2A03 | Noise")
                            .clicked()
                        {
                            let new_mode = ChannelMode::Noise;
                            // Fine pitch is not an NES noise-channel control. Keep the
                            // editor on a supported tab when switching into Noise.
                            if matches!(data.selected_tab, 2 | 3 | 4) {
                                cleanup_tab_sequence(data, data.selected_tab);
                                data.selected_tab = 0;
                            }
                            data.channel_mode = new_mode;
                            *changed_channel_mode = Some(new_mode);
                        }
                        if ui
                            .selectable_value(&mut waveform_id, 3, "VRC6 | Pulse")
                            .clicked()
                        {
                            let new_mode = ChannelMode::Vrc6Pulse;
                            data.channel_mode = new_mode;
                            *changed_channel_mode = Some(new_mode);
                        }
                        if ui
                            .selectable_value(&mut waveform_id, 4, "VRC6 | Saw")
                            .clicked()
                        {
                            let new_mode = ChannelMode::Vrc6Saw;
                            // The saw keeps its Duty tab: in 16-step mode duty bit 0
                            // is the $B000 rate MSB, so tab 4 stays selectable.
                            data.channel_mode = new_mode;
                            *changed_channel_mode = Some(new_mode);
                        }
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
                    ui.add_enabled_ui(!data.portamento_enabled, |ui| {
                        if ui.checkbox(&mut data.polyphony, "Polyphony").changed() {
                            *changed_polyphony = Some(data.polyphony);
                        }
                    });
                    if ui.checkbox(&mut portamento, "Portamento").changed() {
                        data.portamento_enabled = portamento;
                        *changed_portamento_enabled = Some(portamento);
                    }
                })
                .response
            },
        );

        let _check_row_w = check_row.inner.rect.width();
    });

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(5.0);
}

fn draw_chip_tabs(ui: &mut egui::Ui, _data: &mut SharedSequences) {
    ui.horizontal(|ui| {
        let _ = ui.selectable_label(true, "Envelope Editors");
    });
    ui.add_space(5.0);
    ui.separator();
}

fn draw_instrument_settings_panel(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    shared_sequence_index: usize,
    changed_sequence_index: &mut Option<usize>,
    step_time_hz: u32,
    changed_step_time_hz: &mut Option<i32>,
    changed_max_voices: &mut Option<i32>,
    changed_portamento_speed: &mut Option<i32>,
) {
    // "Duty / Noise" tab is disabled for Triangle mode.
    let seq_types: &[(&str, usize)] = &[
        ("Volume", 0),
        ("Arpeggio", 1),
        ("Pitch", 2),
        ("Hi-Pitch", 3),
        ("Duty / Noise", 4),
    ];

    const PANEL_WIDTH: f32 = 180.0;
    const GROUP_GAP: f32 = 12.0;

    ui.set_width(PANEL_WIDTH);

    // ---------------------------------------------------------
    // Instrument settings
    // ---------------------------------------------------------

    group_box(ui, "Instrument settings", |ui| {
        // Make the group box fill the width of the left panel.
        ui.set_min_width(ui.available_width());

        // Grid with checkboxes and effect names
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

                // Separator line under header row
                ui.separator();
                ui.separator();
                ui.end_row();

                for &(name, tab) in seq_types {
                    let is_duty = tab == 4;
                    let is_no_duty = is_duty
                        && matches!(
                            data.channel_mode,
                            ChannelMode::Triangle | ChannelMode::Noise
                        );
                    let is_noise_pitch = matches!(tab, 2 | 3)
                        && data.channel_mode == ChannelMode::Noise;
                    let enabled = !is_no_duty && !is_noise_pitch;

                    ui.add_enabled(
                        enabled,
                        egui::Checkbox::new(data.sequence_enabled_mut(tab), ""),
                    );


                    if ui
                        .add_enabled(
                            enabled,
                            egui::Button::new(name).selected(data.selected_tab == tab),
                        )
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

    // ---------------------------------------------------------
    // Time settings
    // ---------------------------------------------------------

    // The current cursor is now immediately below Instrument settings.
    // Move down slightly to create a gap between the group boxes.
    ui.add_space(GROUP_GAP);

    // Everything remaining in the left panel belongs to Time settings.
    let remaining = ui.available_rect_before_wrap();

    ui.scope_builder(egui::UiBuilder::new().max_rect(remaining), |ui| {
        group_box(ui, "Settings", |ui| {
            // Fill the same width as Instrument settings.
            ui.set_min_width(ui.available_width());

            // group_box() adds:
            //
            // top    = PADDING + TITLE_HEIGHT = 20px
            // bottom = PADDING                = 10px
            //
            // Therefore its contents need to be 30px shorter
            // than the outer available rectangle.
            ui.set_min_height(ui.available_height());

            // -------------------------------------------------
            // Time controls
            // -------------------------------------------------
            ui.horizontal(|ui| {
                ui.label("Engine Speed:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut hz = step_time_hz as i32;
                    if ui
                        .add(egui::DragValue::new(&mut hz).range(1..=600).suffix(" Hz"))
                        .changed()
                    {
                        *changed_step_time_hz = Some(hz);
                    }
                });
            });

            ui.horizontal(|ui| {
                ui.label("Polyphony:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(!data.portamento_enabled, |ui| {
                        if ui
                            .add(
                                egui::DragValue::new(&mut data.max_voices)
                                    .range(1..=8)
                                    .suffix(" voices"),
                            )
                            .changed()
                        {
                            *changed_max_voices = Some(data.max_voices);
                        }
                    });
                });
            });

            ui.horizontal(|ui| {
                ui.label("Porta. Speed:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::DragValue::new(&mut data.portamento_speed).range(0..=127))
                        .changed()
                    {
                        *changed_portamento_speed = Some(data.portamento_speed);
                    }
                });
            });
        });
    });
}

fn draw_sequence_editor_panel(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    playheads: &SequencePlayheads,
    step_time_hz: u32,
) {
    let tab = data.selected_tab;

    let title = match tab {
        0 => "Volume",
        1 => "Arpeggio",
        2 => "Pitch",
        3 => "Hi-pitch",
        _ => "Duty / Noise",
    };

    group_box(ui, &format!("Sequence editor - {}", title), |ui| {
        // Controls below graph: add_space(6) + Size row (~22) + add_space(6) + TextEdit (~20) = ~54px.
        const CONTROLS_HEIGHT: f32 = 54.0;
        let graph_height = (ui.available_height() - CONTROLS_HEIGHT).max(150.0);
        let mut auto_enable = false;

        // Both reads must happen before `selected_sequence_mut` takes the mutable
        // borrow of `data` below.
        let channel_mode = data.channel_mode;
        let vol_mode = data.selected_sequence(tab).vol_mode;
        let (min_val, max_val) = sequence_range(tab, channel_mode, vol_mode);

        {
            let (text, sequence) = data.selected_sequence_mut(tab);

            let is_arpeggio = tab == 1;
            if draw_envelope_bar_graph(
                ui,
                sequence,
                min_val,
                max_val,
                is_arpeggio,
                playheads.step(tab),
                graph_height,
            ) {
                *text = sequence_to_text(sequence);
                auto_enable = true;
            }

            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.label("Size:");

                let mut desired_len = sequence.len();

                if repeating_button(ui, "-") {
                    desired_len = desired_len.saturating_sub(1);
                }

                // FamiTracker-style size control: drag the number vertically to
                // grow or shrink the sequence, while retaining the +/- buttons.
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
                    auto_enable = true;
                    sequence.values.resize(desired_len, 0);

                    if sequence.loop_point.is_some_and(|p| p >= desired_len) {
                        sequence.loop_point = None;
                    }

                    if sequence.release_point.is_some_and(|p| p >= desired_len) {
                        sequence.release_point = None;
                    }

                    *text = sequence_to_text(sequence);
                }

                ui.add_space(15.0);

                ui.label(format!(
                    "{} ms",
                    (sequence.len() as u64 * 1000) / (step_time_hz as u64).max(1)
                ));

                if tab == 1 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.radio_value(&mut sequence.arp_mode, ArpMode::Absolute, "Absolute");

                        ui.radio_value(&mut sequence.arp_mode, ArpMode::Relative, "Relative");

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

                // dn exposes 16-step/64-step as a volume-sequence setting and only
                // enables it for VRC6 instruments (SequenceSetting.cpp), so the
                // toggle occupies the same slot as the arpeggio/pitch mode radios.
                if tab == 0 && channel_mode == ChannelMode::Vrc6Saw {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut is_64 = sequence.vol_mode == VolMode::Steps64;

                        if ui.checkbox(&mut is_64, "64-Step").changed() {
                            let next = if is_64 {
                                VolMode::Steps64
                            } else {
                                VolMode::Steps16
                            };
                            set_volume_step_mode(text, sequence, next);
                            auto_enable = true;
                        }
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
                auto_enable = true;
                let sanitized = sanitize_sequence_text(text);
                // `parse_clamped` resets every mode field to its default, so all
                // three have to survive the round trip.
                let prev_pitch_mode = sequence.pitch_mode;
                let prev_arp_mode = sequence.arp_mode;
                let prev_vol_mode = sequence.vol_mode;

                *sequence = if sanitized.trim().is_empty() {
                    Sequence::default()
                } else {
                    Sequence::parse_clamped(&sanitized, min_val, max_val).0
                };

                sequence.pitch_mode = prev_pitch_mode;
                sequence.arp_mode = prev_arp_mode;
                sequence.vol_mode = prev_vol_mode;
            }

            if enter_pressed || edit.lost_focus() {
                *text = sequence_to_text(sequence);
            }
        }

        if auto_enable {
            *data.sequence_enabled_mut(tab) = true;
        }
    });
}

fn draw_main_content(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    shared_sequence_index: usize,
    changed_sequence_index: &mut Option<usize>,
    playheads: &SequencePlayheads,
    step_time_hz: u32,
    changed_step_time_hz: &mut Option<i32>,
    changed_max_voices: &mut Option<i32>,
    changed_portamento_speed: &mut Option<i32>,
) {
    const LEFT_W: f32 = 180.0;
    const GAP: f32 = 8.0;
    const TOP_GAP: f32 = 10.0;

    let available = ui.available_rect_before_wrap();

    // Move the start of BOTH columns down.
    let content_top = available.min.y + TOP_GAP;

    let left_rect = egui::Rect::from_min_max(
        egui::pos2(available.min.x, content_top),
        egui::pos2(available.min.x + LEFT_W, available.max.y),
    );

    let right_rect = egui::Rect::from_min_max(
        egui::pos2(left_rect.max.x + GAP, content_top),
        available.max,
    );

    ui.scope_builder(egui::UiBuilder::new().max_rect(left_rect), |ui| {
        draw_instrument_settings_panel(
            ui,
            data,
            shared_sequence_index,
            changed_sequence_index,
            step_time_hz,
            changed_step_time_hz,
            changed_max_voices,
            changed_portamento_speed,
        );
    });

    ui.scope_builder(egui::UiBuilder::new().max_rect(right_rect), |ui| {
        draw_sequence_editor_panel(ui, data, playheads, step_time_hz);
    });
}

fn draw_footer(ui: &mut egui::Ui) {
    ui.separator();
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("0.0.5-Alpha").weak());
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
    step_time_hz: u32,
) -> EditorResult {
    // Apply non-selectable labels to the global Context style so all child scopes,
    // grids, and group boxes inherit it
    ui.ctx().global_style_mut(|style| {
        style.interaction.selectable_labels = false;
    });

    ui.set_min_height(ui.available_height());

    data.set_all_selected_sequence_indices(shared_sequence_index);

    // Noise has no fine-pitch control. This also handles channel changes made
    // through host automation rather than through the editor combobox.
    if data.channel_mode == ChannelMode::Noise && matches!(data.selected_tab, 2 | 3) {
        data.selected_tab = 0;
    }

    // 64-step volume is saw-only; drop back to the 4-bit range on every other channel.
    sync_volume_step_mode_to_channel(data);

    let mut changed_sequence_index = None;
    let mut changed_step_time_hz = None;
    let mut changed_channel_mode = None;
    let mut changed_polyphony = None;
    let mut changed_max_voices = None;
    let mut changed_portamento_enabled = None;
    let mut changed_portamento_speed = None;

    draw_header(
        ui,
        data,
        &mut changed_channel_mode,
        &mut changed_polyphony,
        &mut changed_portamento_enabled,
    );
    draw_chip_tabs(ui, data);

    // Reserve footer space at the bottom of the window with spacing gap
    const FOOTER_HEIGHT: f32 = 26.0;
    const FOOTER_GAP: f32 = 10.0;
    let available = ui.available_rect_before_wrap();

    // Allocate full available space in parent UI so outer frames don't shrink
    ui.allocate_rect(available, egui::Sense::hover());

    let footer_rect = egui::Rect::from_min_max(
        egui::pos2(available.min.x, available.max.y - FOOTER_HEIGHT),
        available.max,
    );
    let main_rect = egui::Rect::from_min_max(
        available.min,
        egui::pos2(available.max.x, footer_rect.min.y - FOOTER_GAP),
    );

    // Main content fills everything above the footer
    ui.scope_builder(egui::UiBuilder::new().max_rect(main_rect), |ui| {
        draw_main_content(
            ui,
            data,
            shared_sequence_index,
            &mut changed_sequence_index,
            playheads,
            step_time_hz,
            &mut changed_step_time_hz,
            &mut changed_max_voices,
            &mut changed_portamento_speed,
        );
    });

    // Footer is always at the very bottom
    ui.scope_builder(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
        draw_footer(ui);
    });

    EditorResult {
        new_sequence_index: changed_sequence_index,
        new_step_time_hz: changed_step_time_hz,
        new_channel_mode: changed_channel_mode,
        new_polyphony: changed_polyphony,
        new_max_voices: changed_max_voices,
        new_portamento_enabled: changed_portamento_enabled,
        new_portamento_speed: changed_portamento_speed,
    }
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
        assert_eq!(
            sequence_range(2, ChannelMode::Pulse, VolMode::Steps16),
            (-128, 127)
        );
        assert_eq!(
            sequence_range(3, ChannelMode::Pulse, VolMode::Steps16),
            (-128, 127)
        );

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

    #[test]
    fn vrc6_saw_volume_range_follows_the_step_mode() {
        // dn defaults the VRC6 saw volume sequence to SETTING_VOL_16_STEPS; the 6-bit
        // range is only reachable through SETTING_VOL_64_STEPS.
        assert_eq!(
            sequence_range(0, ChannelMode::Vrc6Saw, VolMode::Steps16),
            (0, 15)
        );
        assert_eq!(
            sequence_range(0, ChannelMode::Vrc6Saw, VolMode::Steps64),
            (0, 63)
        );

        // The step mode is saw-only — every other channel stays 4-bit either way.
        assert_eq!(
            sequence_range(0, ChannelMode::Pulse, VolMode::Steps64),
            (0, 15)
        );
        assert_eq!(
            sequence_range(0, ChannelMode::Vrc6Pulse, VolMode::Steps64),
            (0, 15)
        );
    }

    #[test]
    fn vrc6_saw_duty_range_is_one_bit() {
        // dn `CVRC6Sawtooth::MAX_DUTY = 0x01` — the saw's duty is the $B000 rate MSB.
        assert_eq!(
            sequence_range(4, ChannelMode::Vrc6Saw, VolMode::Steps16),
            (0, 1)
        );
        assert_eq!(
            sequence_range(4, ChannelMode::Vrc6Pulse, VolMode::Steps16),
            (0, 7)
        );
        assert_eq!(
            sequence_range(4, ChannelMode::Pulse, VolMode::Steps16),
            (0, 3)
        );
    }

    #[test]
    fn leaving_the_saw_waveform_rescales_a_64_step_volume_sequence() {
        for channel_mode in [
            ChannelMode::Pulse,
            ChannelMode::Triangle,
            ChannelMode::Noise,
            ChannelMode::Vrc6Pulse,
        ] {
            let mut data = SharedSequences::default();
            data.channel_mode = ChannelMode::Vrc6Saw;
            data.selected_sequence_mut(0).0.push_str("63 32 4 0");
            cleanup_tab_sequence(&mut data, 0);

            {
                let (text, sequence) = data.selected_sequence_mut(0);
                set_volume_step_mode(text, sequence, VolMode::Steps64);
            }
            // "63 32 4 0" was authored under 16-step, so cleanup clamped it to
            // [15, 15, 4, 0] before the x4 promotion.
            assert_eq!(data.selected_sequence(0).values, vec![60, 60, 16, 0]);

            data.channel_mode = channel_mode;
            sync_volume_step_mode_to_channel(&mut data);

            assert_eq!(data.selected_sequence(0).vol_mode, VolMode::Steps16);
            assert_eq!(
                data.selected_sequence(0).values,
                vec![15, 15, 4, 0],
                "{channel_mode:?} must see 4-bit steps"
            );
            // The text box has to agree with the graph, not keep showing 0..63.
            assert_eq!(data.selected_sequence_mut(0).0, "15 15 4 0");

            // Idempotent — a second frame on the same channel changes nothing.
            sync_volume_step_mode_to_channel(&mut data);
            assert_eq!(data.selected_sequence(0).values, vec![15, 15, 4, 0]);
        }
    }

    #[test]
    fn staying_on_the_saw_waveform_keeps_64_step_values() {
        let mut data = SharedSequences::default();
        data.channel_mode = ChannelMode::Vrc6Saw;
        data.selected_sequence_mut(0).0.push_str("15 8 0");
        cleanup_tab_sequence(&mut data, 0);

        {
            let (text, sequence) = data.selected_sequence_mut(0);
            set_volume_step_mode(text, sequence, VolMode::Steps64);
        }

        sync_volume_step_mode_to_channel(&mut data);
        assert_eq!(data.selected_sequence(0).vol_mode, VolMode::Steps64);
        assert_eq!(data.selected_sequence(0).values, vec![60, 32, 0]);
    }

    #[test]
    fn cleanup_preserves_the_saw_step_mode_and_clamps_to_it() {
        let mut data = SharedSequences::default();
        data.channel_mode = ChannelMode::Vrc6Saw;
        data.selected_sequence_mut(0).1.vol_mode = VolMode::Steps64;
        data.selected_sequence_mut(0).0.push_str("63 32 0");
        cleanup_tab_sequence(&mut data, 0);
        assert_eq!(data.selected_sequence(0).vol_mode, VolMode::Steps64);
        assert_eq!(data.selected_sequence(0).values, vec![63, 32, 0]);

        // Back to 16-step: the same text now clamps to the 4-bit range.
        data.selected_sequence_mut(0).1.vol_mode = VolMode::Steps16;
        cleanup_tab_sequence(&mut data, 0);
        assert_eq!(data.selected_sequence(0).vol_mode, VolMode::Steps16);
        assert_eq!(data.selected_sequence(0).values, vec![15, 15, 0]);
    }
}
