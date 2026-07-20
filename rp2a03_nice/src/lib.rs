use nice_plug::prelude::*;
use std::num::NonZeroU32;
use std::sync::Arc;

use nice_plug_egui::{create_egui_editor, EguiState};
use egui;

use rp2a03_core::blip_buf::BlipBuf;
use rp2a03_core::nes_core::{FrameSequencer, FrameTick, NTSC_CPU_CLOCK};
use rp2a03_core::nes_noise::NoiseChannel;
use rp2a03_core::nes_square::SquareChannel;
use rp2a03_core::nes_triangle::TriangleChannel;

mod midi_helper;
const BLIP_BUFFER_SIZE: i32 = 4096;
/// Scale factor so that a full 0–15 swing maps to the full i16 range.
/// 32767 / 15 ≈ 2184
const AMPLITUDE_SCALE: i32 = 2184;

#[derive(Enum, PartialEq, Clone, Copy)]
enum ChannelMode {
    #[name = "2A03 Square"]
    Square,
    #[name = "2A03 Triangle"]
    Triangle,
    #[name = "2A03 Noise"]
    Noise,
}

#[derive(Params)]
struct NesSynthParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,
    #[id = "mode"]
    pub mode: EnumParam<ChannelMode>,
    #[id = "duty"]
    pub duty: IntParam,
    #[id = "volume"]
    pub volume: IntParam,
    #[id = "noise_mode"]
    pub noise_mode: BoolParam,
}

impl Default for NesSynthParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(400, 300),
            mode: EnumParam::new("Mode", ChannelMode::Square),
            duty: IntParam::new("Duty", 2, IntRange::Linear { min: 0, max: 3 }),
            volume: IntParam::new("Volume", 15, IntRange::Linear { min: 0, max: 15 }),
            noise_mode: BoolParam::new("Metallic", false),
        }
    }
}

struct NesSynth {
    params: Arc<NesSynthParams>,
    square: SquareChannel,
    triangle: TriangleChannel,
    noise: NoiseChannel,
    frame_seq: FrameSequencer,
    sample_rate: f32,
    blip: BlipBuf,
}

impl Default for NesSynth {
    fn default() -> Self {
        let mut blip = BlipBuf::new(BLIP_BUFFER_SIZE).expect("failed to create BlipBuf");
        blip.set_rates(NTSC_CPU_CLOCK, 44100.0);

        let mut synth = Self {
            params: Arc::new(NesSynthParams::default()),
            square: SquareChannel::new(true),
            triangle: TriangleChannel::new(),
            noise: NoiseChannel::new(),
            frame_seq: FrameSequencer::default(),
            sample_rate: 44100.0,
            blip,
        };
        synth.apply_default_state();
        synth
    }
}

impl NesSynth {
    fn apply_default_state(&mut self) {
        // Square: volume 0, length counter halt, no sweep
        self.square.write_reg0(0x30);
        self.square.write_reg1(0x08);
        
        // Triangle: linear counter halt (disables length counter)
        self.triangle.write_reg0(0x80);
        
        // Noise: volume 0, length counter halt, load noise length 0
        self.noise.write_reg0(0x30);
        self.noise.write_reg3(0x00);
    }

    fn note_on(&mut self, note: u8, velocity: f32) {
        let mode = self.params.mode.value();
        let volume = ((self.params.volume.value() as f32) * velocity).round() as u8;

        // Ensure all are muted before we activate the selected one
        self.square.set_enabled(false);
        self.triangle.set_enabled(false);
        self.noise.set_enabled(false);

        match mode {
            ChannelMode::Square => {
                let period = midi_helper::period_for_frequency(midi_helper::midi_note_to_freq(note));
                let duty = self.params.duty.value() as u8;
                self.square.set_enabled(true);
                // 0x30 = Constant volume (0x10) + Length counter halt (0x20)
                self.square.write_reg0((duty << 6) | 0x30 | (volume & 0x0F));
                // 0x08 = Sweep negate true. This is required to prevent the APU from muting 
                // the channel when the period is > 1023 (i.e. low notes).
                self.square.write_reg1(0x08);
                self.square.write_reg2((period & 0xFF) as u8);
                self.square.write_reg3((0x1Fu8 << 3) | ((period >> 8) as u8 & 0x07));
            }
            ChannelMode::Triangle => {
                let freq = midi_helper::midi_note_to_freq(note);
                let period = midi_helper::period_for_frequency(freq);
                
                self.triangle.set_enabled(true);
                // 0x80 = Control flag (halts length counter) + 0x7F = max linear counter reload
                self.triangle.write_reg0(0x80 | 0x7F);
                self.triangle.write_reg2((period & 0xFF) as u8);
                self.triangle.write_reg3((0x1Fu8 << 3) | ((period >> 8) as u8 & 0x07));
            }
            ChannelMode::Noise => {
                let period_idx = midi_helper::noise_period_for_midi_note(note);
                let mode_bit = if self.params.noise_mode.value() { 0x80 } else { 0x00 };

                self.noise.set_enabled(true);
                // 0x30 = Constant volume (0x10) + Length counter halt (0x20)
                self.noise.write_reg0(0x30 | (volume & 0x0F));
                self.noise.write_reg2(mode_bit | period_idx);
                self.noise.write_reg3(0x1F << 3);
            }
        }
    }

    fn note_off(&mut self) {
        self.square.set_enabled(false);
        self.triangle.set_enabled(false);
        self.noise.set_enabled(false);
    }

    /// Run enough NES CPU clocks to produce `sample_count` output samples,
    /// then read them out of the BlipBuf into the provided slice.
    fn generate_samples(&mut self, output: &mut [f32]) {
        let sample_count = output.len() as i32;
        let clocks_needed = self.blip.clocks_needed(sample_count) as u32;
        let mode = self.params.mode.value();

        for clock in 0..clocks_needed {
            // 1. Tick frame sequencer (envelopes, length counters, sweep)
            match self.frame_seq.clock() {
                FrameTick::Quarter => {
                    self.square.tick_envelope();
                    self.triangle.tick_linear_counter();
                    self.noise.tick_envelope();
                }
                FrameTick::Half => {
                    self.square.tick_length_counter();
                    self.square.tick_sweep();
                    self.triangle.tick_length_counter();
                    self.noise.tick_length_counter();
                }
                FrameTick::None => {}
            }
            
            // 2. Reload length counters
            self.square.reload_length_counter();
            self.triangle.reload_length_counter();
            self.noise.reload_length_counter();

            // 3. Clock the channel timers
            self.square.clock();
            self.triangle.clock();
            self.noise.clock();

            // 4. Consume deltas from ALL channels (prevents massive DC buildup in unused channels)
            let sq_delta = self.square.take_delta();
            let tri_delta = self.triangle.take_delta();
            let noise_delta = self.noise.take_delta();

            // 5. Submit only the active channel's delta to the BLEP synth
            let active_delta = match mode {
                ChannelMode::Square => sq_delta,
                ChannelMode::Triangle => tri_delta,
                ChannelMode::Noise => noise_delta,
            };

            if active_delta != 0 {
                self.blip.add_delta(clock, active_delta as i32 * AMPLITUDE_SCALE);
            }
        }

        self.blip.end_frame(clocks_needed);

        let mut buf_i16 = vec![0i16; sample_count as usize];
        self.blip.read_samples(&mut buf_i16, sample_count, false);

        for (i, sample) in buf_i16.iter().enumerate() {
            output[i] = *sample as f32 / 32768.0;
        }
    }
}

impl Plugin for NesSynth {
    const NAME: &'static str = "RP2A03 Synth";
    const VENDOR: &'static str = "PurpBatBoi";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = "0.1.0";

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let params = self.params.clone();
        let egui_state = params.editor_state.clone();

        create_egui_editor(
            egui_state.clone(),
            (),
            nice_plug_egui::EguiSettings::default(),
            |egui_ctx, _queue, _state| {
                egui_ctx.set_theme(egui::Theme::Dark);
                egui_ctx.set_style_of(egui::Theme::Dark, rp2a03_common::gui::rp2a03_style());
            },
            move |ui, setter, _queue, _state| {
                nice_plug_egui::resizable_window::ResizableWindow::new("NES Synth Editor")
                    .min_size(egui::Vec2::new(300.0, 200.0))
                    .show(ui, egui_state.as_ref(), |ui| {
                        egui::Frame::new()
                            .inner_margin(egui::Margin::same(20))
                            .show(ui, |ui| {
let mut selected_mode = params.mode.value() as usize;
                                let prev_duty = params.duty.value() as f32;
                                let prev_vol = params.volume.value() as f32;
                                let mut noise_mode = params.noise_mode.value();

                                let mut new_duty = prev_duty;
                                let mut new_vol = prev_vol;

                                let responses = rp2a03_common::gui::draw_synth_ui(
                                    ui,
                                    &mut selected_mode,
                                    &mut new_duty,
                                    &mut new_vol,
                                    &mut noise_mode,
                                );

                                if responses.mode.changed {
                                    let new_mode = match selected_mode {
                                        0 => ChannelMode::Square,
                                        1 => ChannelMode::Triangle,
                                        2 => ChannelMode::Noise,
                                        _ => ChannelMode::Square,
                                    };
                                    setter.begin_set_parameter(&params.mode);
                                    setter.set_parameter(&params.mode, new_mode);
                                    setter.end_set_parameter(&params.mode);
                                }

                                if responses.duty.drag_started {
                                    setter.begin_set_parameter(&params.duty);
                                }
                                if new_duty != prev_duty {
                                    setter.set_parameter(&params.duty, new_duty as i32);
                                }
                                if responses.duty.drag_stopped {
                                    setter.end_set_parameter(&params.duty);
                                }

                                if responses.volume.drag_started {
                                    setter.begin_set_parameter(&params.volume);
                                }
                                if new_vol != prev_vol {
                                    setter.set_parameter(&params.volume, new_vol as i32);
                                }
                                if responses.volume.drag_stopped {
                                    setter.end_set_parameter(&params.volume);
                                }

                                if responses.noise_mode.changed {
                                    setter.begin_set_parameter(&params.noise_mode);
                                    setter.set_parameter(&params.noise_mode, noise_mode);
                                    setter.end_set_parameter(&params.noise_mode);
                                }
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
        self.blip = BlipBuf::new(BLIP_BUFFER_SIZE).expect("failed to create BlipBuf");
        self.blip
            .set_rates(NTSC_CPU_CLOCK, buffer_config.sample_rate as f64);
        true
    }

    fn reset(&mut self) {
        self.frame_seq = FrameSequencer::default();
        self.square = SquareChannel::new(true);
        self.triangle = TriangleChannel::new();
        self.noise = NoiseChannel::new();
        self.blip.clear();
        self.apply_default_state();
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

        loop {
            // Find where the next MIDI event lands (or end of buffer)
            let chunk_end = if let Some(ref event) = next_event {
                (event.timing() as usize).min(num_samples)
            } else {
                num_samples
            };

            // Generate audio up to that point
            if chunk_end > sample_pos {
                self.generate_samples(&mut mono_buf[sample_pos..chunk_end]);
                sample_pos = chunk_end;
            }

            if sample_pos >= num_samples {
                break;
            }

            // Dispatch all MIDI events at this timing
            while let Some(event) = next_event {
                if event.timing() as usize > sample_pos {
                    next_event = Some(event);
                    break;
                }
                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => {
                        self.note_on(note, velocity);
                    }
                    NoteEvent::NoteOff { .. } => {
                        self.note_off();
                    }
                    _ => {}
                }
                next_event = context.next_event();
            }

            if next_event.is_none() && sample_pos < num_samples {
                self.generate_samples(&mut mono_buf[sample_pos..num_samples]);
                break;
            }
        }

        // Copy mono to all output channels
        for (sample_id, mut channel_samples) in buffer.iter_samples().enumerate() {
            for out_sample in channel_samples.iter_mut() {
                *out_sample = mono_buf[sample_id];
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for NesSynth {
    const CLAP_ID: &'static str = "com.example.nes-synth";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("NES multi-channel synth");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Mono,
    ];
}

impl Vst3Plugin for NesSynth {
    const VST3_CLASS_ID: [u8; 16] = *b"NesSynth00000001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nice_export_clap!(NesSynth);
nice_export_vst3!(NesSynth);