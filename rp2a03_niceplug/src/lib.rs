//! rp2a03_niceplug\src\lib.rs
mod midi;

use midi::{ActiveSequences, MidiHandler};
use nice_plug::prelude::*;
use egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use nice_plug_egui::{create_egui_editor, EguiSettings, EguiState};
use parking_lot::Mutex;
use rp2a03_core::apu_pulse::{Pulse, PulseChannel};
use rp2a03_core::blip_buf::BlipBuf;
use rp2a03_core::sequence::Sequence;
use std::sync::Arc;

use rp2a03_core::NTSC_CPU_CLOCK;

const BLIP_BUFFER_SIZE: u32 = 4096;
const AMPLITUDE_SCALE: i32 = 1500;

/// Shared thread-safe state holding sequence definitions and editor configuration.
#[derive(Debug, Clone)]
pub struct SharedSequences {
    pub selected_tab: usize, // 0=Volume, 1=Arpeggio, 2=Pitch, 3=Hi-Pitch, 4=Duty

    // Text representation of each sequence
    pub vol_text: String,
    pub arp_text: String,
    pub pitch_text: String,
    pub hipitch_text: String,
    pub duty_text: String,

    // Parsed Sequence objects
    pub vol_seq: Sequence,
    pub arp_seq: Sequence,
    pub pitch_seq: Sequence,
    pub hipitch_seq: Sequence,
    pub duty_seq: Sequence,

    // Enabled flags
    pub vol_enabled: bool,
    pub arp_enabled: bool,
    pub pitch_enabled: bool,
    pub hipitch_enabled: bool,
    pub duty_enabled: bool,
}

impl Default for SharedSequences {
    fn default() -> Self {
        let vol_text = "15".to_string();
        let arp_text = "0".to_string();
        let pitch_text = "0".to_string();
        let hipitch_text = "0".to_string();
        let duty_text = "2".to_string();

        let (vol_seq, _) = Sequence::parse_clamped(&vol_text, 0, 15);
        let (arp_seq, _) = Sequence::parse_clamped(&arp_text, -96, 96);
        let (pitch_seq, _) = Sequence::parse_clamped(&pitch_text, -128, 127);
        let (hipitch_seq, _) = Sequence::parse_clamped(&hipitch_text, -64, 63);
        let (duty_seq, _) = Sequence::parse_clamped(&duty_text, 0, 3);

        Self {
            selected_tab: 0,
            vol_text,
            arp_text,
            pitch_text,
            hipitch_text,
            duty_text,
            vol_seq,
            arp_seq,
            pitch_seq,
            hipitch_seq,
            duty_seq,
            vol_enabled: true,
            arp_enabled: false,
            pitch_enabled: false,
            hipitch_enabled: false,
            duty_enabled: true,
        }
    }
}

pub struct Rp2a03Plugin {
    params: Arc<Rp2a03Params>,
    pulse: Pulse,
    blip: BlipBuf,
    sample_rate: f32,
    last_output: i16,
    midi_handler: MidiHandler,
    shared_sequences: Arc<Mutex<SharedSequences>>,
}

#[derive(Params)]
struct Rp2a03Params {
    #[persist = "editor_state"]
    pub egui_state: Arc<EguiState>,
}

impl Default for Rp2a03Plugin {
    fn default() -> Self {
        let mut pulse = Pulse::new(PulseChannel::One);
        pulse.set_enabled(true);
        let mut blip = BlipBuf::new(BLIP_BUFFER_SIZE);
        blip.set_rates(NTSC_CPU_CLOCK, 44100.0);

        Self {
            params: Arc::new(Rp2a03Params::default()),
            pulse,
            blip,
            sample_rate: 44100.0,
            last_output: 0,
            midi_handler: MidiHandler::new(),
            shared_sequences: Arc::new(Mutex::new(SharedSequences::default())),
        }
    }
}

impl Default for Rp2a03Params {
    fn default() -> Self {
        Self {
            egui_state: EguiState::from_size(680, 420),
        }
    }
}

impl Rp2a03Plugin {
    fn generate_samples(&mut self, output: &mut [f32], seqs: &ActiveSequences) {
        let sample_count = output.len() as u32;
        if sample_count == 0 {
            return;
        }

        let clocks_needed = self.blip.clocks_needed(sample_count);

        let master_gain = self.midi_handler.update_modulation(
            &mut self.pulse,
            seqs,
            self.sample_rate,
            sample_count as usize,
        );

        for clock in 0..clocks_needed {
            self.pulse.clock();

            let current_output = if self.midi_handler.gate() && !self.pulse.is_muted() {
                self.pulse.output() as i16
            } else {
                0
            };

            let delta = current_output as i32 - self.last_output as i32;
            if delta != 0 {
                self.blip.add_delta(clock, delta * AMPLITUDE_SCALE);
                self.last_output = current_output;
            }
        }

        self.blip.end_frame(clocks_needed);

        let mut buf_i16 = vec![0i16; sample_count as usize];
        self.blip.read_samples(&mut buf_i16, false);

        for (i, sample) in buf_i16.iter().enumerate() {
            output[i] = (*sample as f32 / 32768.0) * master_gain;
        }
    }
}

impl Plugin for Rp2a03Plugin {
    const NAME: &'static str = "RP2A03 Synth";
    const VENDOR: &'static str = "RP2A03 Project";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: None,
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let shared = self.shared_sequences.clone();

        create_egui_editor(
            self.params.egui_state.clone(),
            (),
            EguiSettings::default(),
            move |ctx, _queue, _state| {
                let mut visuals = egui::Visuals::dark();
                visuals.panel_fill = Color32::from_rgb(14, 14, 14);
                visuals.extreme_bg_color = Color32::from_rgb(10, 10, 10);
                ctx.set_visuals(visuals);
            },
            move |ui, _setter, _queue, _state| {
                let mut data = shared.lock();

                ui.horizontal(|ui| {
                    // --- Left Column: Instrument Settings (Tab Selector) ---
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

                    // --- Right Column: Active Sequence Editor ---
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
                            ui.label(egui::RichText::new(format!("Sequence editor - {}", title)).strong());
                            ui.add_space(4.0);

                            // Draw Envelope Bar Graph
                            draw_envelope_bar_graph(ui, seq_ptr, min_val, max_val);

                            ui.add_space(6.0);

                            // Size +/- Controls
                            ui.horizontal(|ui| {
                                ui.label("Size:");

                                let cur_len = seq_ptr.len();
                                if ui.button("-").clicked() && cur_len > 1 {
                                    let mut tokens: Vec<&str> = text_ptr.split_whitespace().collect();
                                    // Remove last numeric token
                                    for i in (0..tokens.len()).rev() {
                                        if tokens[i].parse::<i16>().is_ok() {
                                            tokens.remove(i);
                                            break;
                                        }
                                    }
                                    *text_ptr = tokens.join(" ");
                                    let (parsed, norm) = Sequence::parse_clamped(text_ptr, min_val, max_val);
                                    *seq_ptr = parsed;
                                    *text_ptr = norm;
                                }

                                ui.label(egui::RichText::new(format!("{}", cur_len)).strong());

                                if ui.button("+").clicked() {
                                    text_ptr.push_str(" 0");
                                    let (parsed, norm) = Sequence::parse_clamped(text_ptr, min_val, max_val);
                                    *seq_ptr = parsed;
                                    *text_ptr = norm;
                                }

                                ui.add_space(15.0);
                                let duration_ms = (cur_len * 1000) / 60;
                                ui.label(format!("{} ms", duration_ms));
                            });

                            ui.add_space(6.0);

                            // Text Editor Input Box with Auto-Clamping
                            ui.horizontal(|ui| {
                                let edit = ui.add(
                                    egui::TextEdit::singleline(text_ptr)
                                        .desired_width(420.0)
                                        .font(egui::TextStyle::Monospace),
                                );

                                if edit.changed() {
                                    let (parsed, norm) = Sequence::parse_clamped(text_ptr, min_val, max_val);
                                    *seq_ptr = parsed;
                                    // Update text if clamped
                                    if *text_ptr != norm && !edit.has_focus() {
                                        *text_ptr = norm;
                                    }
                                }
                            });
                        });
                    });
                });
            },
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.blip = BlipBuf::new(BLIP_BUFFER_SIZE);
        self.blip.set_rates(NTSC_CPU_CLOCK, buffer_config.sample_rate as f64);
        self.pulse.reset();
        self.pulse.set_enabled(true);
        self.pulse.write_sweep(0x08);
        self.last_output = 0;
        self.midi_handler.reset();
        true
    }

    fn reset(&mut self) {
        self.pulse.reset();
        self.pulse.set_enabled(true);
        self.pulse.write_sweep(0x08);
        self.blip.clear();
        self.last_output = 0;
        self.midi_handler.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let num_samples = buffer.samples();
        let mut next_event = context.next_event();
        let mut sample_pos: usize = 0;
        let mut mono_buf = vec![0.0f32; num_samples];

        // Fetch current active sequences from GUI thread state
        let active_seqs = {
            let data = self.shared_sequences.lock();
            ActiveSequences {
                vol_seq: data.vol_seq.clone(),
                vol_enabled: data.vol_enabled,
                arp_seq: data.arp_seq.clone(),
                arp_enabled: data.arp_enabled,
                pitch_seq: data.pitch_seq.clone(),
                pitch_enabled: data.pitch_enabled,
                hipitch_seq: data.hipitch_seq.clone(),
                hipitch_enabled: data.hipitch_enabled,
                duty_seq: data.duty_seq.clone(),
                duty_enabled: data.duty_enabled,
            }
        }; // MutexGuard is dropped here

        loop {
            let chunk_end = if let Some(ref event) = next_event {
                (event.timing() as usize).min(num_samples)
            } else {
                num_samples
            };

            if chunk_end > sample_pos {
                self.generate_samples(&mut mono_buf[sample_pos..chunk_end], &active_seqs);
                sample_pos = chunk_end;
            }

            if sample_pos >= num_samples {
                break;
            }

            while let Some(event) = next_event {
                if event.timing() as usize > sample_pos {
                    next_event = Some(event);
                    break;
                }
                self.midi_handler.handle_event(&event, &mut self.pulse, &active_seqs);
                next_event = context.next_event();
            }

            if next_event.is_none() && sample_pos < num_samples {
                self.generate_samples(&mut mono_buf[sample_pos..num_samples], &active_seqs);
                break;
            }
        }

        for (sample_id, channel_samples) in buffer.iter_samples().enumerate() {
            for out_sample in channel_samples {
                *out_sample = mono_buf[sample_id];
            }
        }

        ProcessStatus::KeepAlive
    }
}

/// Renders a FamiTracker-style envelope bar graph with unipolar and bipolar support.
fn draw_envelope_bar_graph(ui: &mut egui::Ui, seq: &Sequence, min_val: i16, max_val: i16) {
    let desired_size = Vec2::new(450.0, 220.0);
    let (rect, _response) = ui.allocate_at_least(desired_size, Sense::hover());

    let painter = ui.painter_at(rect);

    // Background
    painter.rect_filled(rect, 2.0, Color32::from_rgb(8, 8, 8));
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0f32, Color32::from_rgb(35, 35, 35)), egui::StrokeKind::Outside);

    let num_steps = seq.len();
    if num_steps == 0 {
        return;
    }

    let header_height = 20.0;
    let graph_rect = Rect::from_min_max(
        rect.min,
        Pos2::new(rect.max.x, rect.max.y - header_height),
    );
    let header_rect = Rect::from_min_max(
        Pos2::new(rect.min.x, rect.max.y - header_height),
        rect.max,
    );

    let step_width = graph_rect.width() / num_steps as f32;
    let loop_idx = seq.loop_point.unwrap_or(usize::MAX);
    let rel_idx = seq.release_point.unwrap_or(usize::MAX);

    let is_bipolar = min_val < 0;

    // Calculate zero axis Y position
    let zero_y = if is_bipolar {
        let range = (max_val - min_val) as f32;
        let norm_zero = (0 - min_val) as f32 / range.max(1.0);
        graph_rect.max.y - (norm_zero * graph_rect.height())
    } else {
        graph_rect.max.y
    };

    // Draw zero line for bipolar graphs
    if is_bipolar {
        painter.line_segment(
            [Pos2::new(graph_rect.min.x, zero_y), Pos2::new(graph_rect.max.x, zero_y)],
            Stroke::new(1.0f32, Color32::from_rgb(60, 60, 60)),
        );
    }

    // Render step columns & bars
    for i in 0..num_steps {
        let val = seq.values[i].clamp(min_val, max_val);

        let bar_x_min = graph_rect.min.x + i as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0;

        // Step grid column background
        let col_rect = Rect::from_min_max(
            Pos2::new(bar_x_min, graph_rect.min.y),
            Pos2::new(bar_x_max, graph_rect.max.y),
        );
        let bg_color = if i % 2 == 0 {
            Color32::from_rgb(14, 14, 14)
        } else {
            Color32::from_rgb(20, 20, 20)
        };
        painter.rect_filled(col_rect, 0.0, bg_color);

        // Calculate bar geometry relative to zero axis
        let bar_rect = if is_bipolar {
            let range = (max_val - min_val) as f32;
            let norm_val = (val - min_val) as f32 / range.max(1.0);
            let bar_y = graph_rect.max.y - (norm_val * graph_rect.height());
            if val >= 0 {
                Rect::from_min_max(Pos2::new(bar_x_min, bar_y), Pos2::new(bar_x_max, zero_y))
            } else {
                Rect::from_min_max(Pos2::new(bar_x_min, zero_y), Pos2::new(bar_x_max, bar_y))
            }
        } else {
            let norm_val = val as f32 / max_val.max(1) as f32;
            let bar_y_min = graph_rect.max.y - (norm_val * graph_rect.height());
            Rect::from_min_max(Pos2::new(bar_x_min, bar_y_min), Pos2::new(bar_x_max, graph_rect.max.y))
        };

        let is_loop_release_mode = loop_idx < num_steps && rel_idx == loop_idx;

        // Bar color based on region
        let bar_color = if is_loop_release_mode && i >= loop_idx {
            Color32::from_rgb(230, 190, 40) // Yellow for Loop, Release mode
        } else if i >= rel_idx {
            Color32::from_rgb(200, 120, 220) // Purple release region
        } else if i >= loop_idx {
            Color32::from_rgb(100, 200, 220) // Cyan loop region
        } else {
            Color32::from_rgb(220, 220, 220) // Normal region
        };

        if val != 0 || !is_bipolar {
            painter.rect_filled(bar_rect, 1.0, bar_color);
        }
    }

    // Render Header markers for Loop & Release
    painter.rect_filled(header_rect, 0.0, Color32::from_rgb(25, 25, 25));

    let is_loop_release_mode = loop_idx < num_steps && rel_idx == loop_idx;

    if is_loop_release_mode {
        let x_min = header_rect.min.x + loop_idx as f32 * step_width;
        let x_max = header_rect.max.x;
        let lr_rect = Rect::from_min_max(
            Pos2::new(x_min, header_rect.min.y),
            Pos2::new(x_max, header_rect.max.y),
        );
        painter.rect_filled(lr_rect, 0.0, Color32::from_rgb(180, 140, 20));
        painter.text(
            Pos2::new(x_min + 4.0, header_rect.min.y + 2.0),
            egui::Align2::LEFT_TOP,
            "Loop, Release",
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
    } else {
        let loop_end = if rel_idx < usize::MAX { rel_idx } else { num_steps };

        if loop_idx < num_steps {
            let x_min = header_rect.min.x + loop_idx as f32 * step_width;
            let x_max = header_rect.min.x + loop_end as f32 * step_width;
            let l_rect = Rect::from_min_max(
                Pos2::new(x_min, header_rect.min.y),
                Pos2::new(x_max, header_rect.max.y),
            );
            painter.rect_filled(l_rect, 0.0, Color32::from_rgb(0, 120, 130));
            painter.text(
                Pos2::new(x_min + 4.0, header_rect.min.y + 2.0),
                egui::Align2::LEFT_TOP,
                "Loop",
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
        }

        if rel_idx < num_steps {
            let x_min = header_rect.min.x + rel_idx as f32 * step_width;
            let x_max = header_rect.max.x;
            let r_rect = Rect::from_min_max(
                Pos2::new(x_min, header_rect.min.y),
                Pos2::new(x_max, header_rect.max.y),
            );
            painter.rect_filled(r_rect, 0.0, Color32::from_rgb(120, 0, 130));
            painter.text(
                Pos2::new(x_min + 4.0, header_rect.min.y + 2.0),
                egui::Align2::LEFT_TOP,
                "Release",
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
        }
    }

    // Min / Max labels
    painter.text(
        Pos2::new(graph_rect.min.x + 6.0, graph_rect.min.y + 2.0),
        egui::Align2::LEFT_TOP,
        format!("{}", max_val),
        egui::FontId::proportional(11.0),
        Color32::from_rgb(160, 160, 160),
    );
    painter.text(
        Pos2::new(graph_rect.min.x + 6.0, graph_rect.max.y - 14.0),
        egui::Align2::LEFT_TOP,
        format!("{}", min_val),
        egui::FontId::proportional(11.0),
        Color32::from_rgb(160, 160, 160),
    );
}

impl ClapPlugin for Rp2a03Plugin {
    const CLAP_ID: &'static str = "com.rp2a03.synth";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("NES APU Pulse Channel Synth");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for Rp2a03Plugin {
    const VST3_CLASS_ID: [u8; 16] = *b"Rp2a03SynthPlugX";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Synth,
    ];
}

nice_export_clap!(Rp2a03Plugin);
nice_export_vst3!(Rp2a03Plugin);
