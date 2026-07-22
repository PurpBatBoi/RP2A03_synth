mod midi;

use midi::MidiHandler;
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

/// Shared thread-safe container for parsed volume and duty sequences.
#[derive(Debug, Clone)]
pub struct SharedSequences {
    pub vol_seq: Sequence,
    pub duty_seq: Sequence,
    pub vol_text: String,
    pub duty_text: String,
}

impl Default for SharedSequences {
    fn default() -> Self {
        let vol_text = "15".to_string();
        let duty_text = "2".to_string();
        Self {
            vol_seq: Sequence::parse(&vol_text),
            duty_seq: Sequence::parse(&duty_text),
            vol_text,
            duty_text,
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
            egui_state: EguiState::from_size(520, 480),
        }
    }
}

impl Rp2a03Plugin {
    fn generate_samples(&mut self, output: &mut [f32], vol_seq: &Sequence, duty_seq: &Sequence) {
        let sample_count = output.len() as u32;
        if sample_count == 0 {
            return;
        }

        let clocks_needed = self.blip.clocks_needed(sample_count);

        let master_gain = self.midi_handler.update_modulation(
            &mut self.pulse,
            vol_seq,
            duty_seq,
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

                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    ui.heading("RP2A03 Sequence Editor");
                    ui.add_space(8.0);

                    // --- Volume Sequence Section ---
                    ui.group(|ui| {
                        ui.label(egui::RichText::new("Volume Sequence").strong());
                        ui.add_space(4.0);

                        // Render Bar Graph Visualization for Volume
                        draw_envelope_bar_graph(ui, &data.vol_seq, 15, "Volume");

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label("Sequence:");
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut data.vol_text)
                                    .desired_width(360.0)
                                    .font(egui::TextStyle::Monospace),
                            );
                            if edit.changed() {
                                data.vol_seq = Sequence::parse(&data.vol_text);
                            }
                        });
                    });

                    ui.add_space(12.0);

                    // --- Duty Sequence Section ---
                    ui.group(|ui| {
                        ui.label(egui::RichText::new("Duty Cycle Sequence").strong());
                        ui.add_space(4.0);

                        // Render Bar Graph Visualization for Duty
                        draw_envelope_bar_graph(ui, &data.duty_seq, 3, "Duty");

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label("Sequence:");
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut data.duty_text)
                                    .desired_width(360.0)
                                    .font(egui::TextStyle::Monospace),
                            );
                            if edit.changed() {
                                data.duty_seq = Sequence::parse(&data.duty_text);
                            }
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
        let (vol_seq, duty_seq) = {
            let data = self.shared_sequences.lock();
            (data.vol_seq.clone(), data.duty_seq.clone())
        };

        loop {
            let chunk_end = if let Some(ref event) = next_event {
                (event.timing() as usize).min(num_samples)
            } else {
                num_samples
            };

            if chunk_end > sample_pos {
                self.generate_samples(&mut mono_buf[sample_pos..chunk_end], &vol_seq, &duty_seq);
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
                self.midi_handler.handle_event(&event, &mut self.pulse, &vol_seq, &duty_seq);
                next_event = context.next_event();
            }

            if next_event.is_none() && sample_pos < num_samples {
                self.generate_samples(&mut mono_buf[sample_pos..num_samples], &vol_seq, &duty_seq);
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

/// Renders a FamiTracker-style envelope bar graph with step bars,
/// cyan Loop region header/bars (`|`), and purple Release region header/bars (`/`).
fn draw_envelope_bar_graph(ui: &mut egui::Ui, seq: &Sequence, max_val: u8, _label: &str) {
    let desired_size = Vec2::new(480.0, 120.0);
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

    // Render bars
    for i in 0..num_steps {
        let val = seq.values[i].min(max_val);
        let norm_val = val as f32 / max_val.max(1) as f32;

        let bar_x_min = graph_rect.min.x + i as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0;
        let bar_y_max = graph_rect.max.y;
        let bar_y_min = graph_rect.max.y - (norm_val * graph_rect.height());

        let bar_rect = Rect::from_min_max(
            Pos2::new(bar_x_min, bar_y_min),
            Pos2::new(bar_x_max, bar_y_max),
        );

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

        // Bar color based on region (normal white/gray, cyan loop, purple release)
        let bar_color = if i >= rel_idx {
            Color32::from_rgb(200, 120, 220) // Purple release region
        } else if i >= loop_idx {
            Color32::from_rgb(100, 200, 220) // Cyan loop region
        } else {
            Color32::from_rgb(220, 220, 220) // Normal region
        };

        if norm_val > 0.0 {
            painter.rect_filled(bar_rect, 1.0, bar_color);
        }
    }

    // Render Header markers for Loop & Release
    painter.rect_filled(header_rect, 0.0, Color32::from_rgb(25, 25, 25));

    let loop_end = if rel_idx < usize::MAX {
        rel_idx
    } else {
        num_steps
    };

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

    // Print step count and duration info below
    let duration_ms = (num_steps * 1000) / 60;
    let info_str = format!("Size: {} steps  ({} ms)", num_steps, duration_ms);
    painter.text(
        Pos2::new(rect.min.x + 6.0, rect.min.y + 4.0),
        egui::Align2::LEFT_TOP,
        info_str,
        egui::FontId::proportional(11.0),
        Color32::from_rgb(180, 180, 180),
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
