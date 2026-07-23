//! rp2a03_niceplug\src\lib.rs
//! RP2A03 Plugin wrapper using nice-plug.

mod midi;

use egui::Color32;
use midi::{ActiveSequences, MidiHandler};
use nice_plug::prelude::*;
use nice_plug_egui::{create_egui_editor, EguiSettings, EguiState};
use parking_lot::Mutex;
use rp2a03_core::apu_pulse::{Pulse, PulseChannel};
use rp2a03_core::blip_buf::BlipBuf;
use rp2a03_core::NTSC_CPU_CLOCK;
use rp2a03_ui::{render_editor_ui, SharedSequences, MAX_SEQUENCES};
use std::sync::Arc;

const BLIP_BUFFER_SIZE: u32 = 4096;
const AMPLITUDE_SCALE: i32 = 1500;

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

    /// Selects the same numbered sequence slot for all five pulse envelopes.
    #[id = "sequence_number"]
    pub sequence_number: IntParam,
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
            sequence_number: IntParam::new(
                "Sequence Number",
                0,
                IntRange::Linear {
                    min: 0,
                    max: (MAX_SEQUENCES - 1) as i32,
                },
            ),
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

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let shared = self.shared_sequences.clone();
        let params = self.params.clone();

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
            move |ui, setter, _queue, _state| {
                let mut data = shared.lock();
                let sequence_index = params.sequence_number.value() as usize;
                if let Some(new_index) = render_editor_ui(ui, &mut data, sequence_index) {
                    data.set_all_selected_sequence_indices(new_index);
                    let new_index = new_index as i32;
                    setter.begin_set_parameter(&params.sequence_number);
                    setter.set_parameter(&params.sequence_number, new_index);
                    setter.end_set_parameter(&params.sequence_number);
                }
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
        self.blip
            .set_rates(NTSC_CPU_CLOCK, buffer_config.sample_rate as f64);
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

        let active_seqs = {
            let sequence_index = self.params.sequence_number.value() as usize;
            let mut data = self.shared_sequences.lock();
            data.set_all_selected_sequence_indices(sequence_index);
            ActiveSequences {
                vol_seq: data.selected_sequence(0).clone(),
                vol_enabled: data.sequence_enabled(0),
                arp_seq: data.selected_sequence(1).clone(),
                arp_enabled: data.sequence_enabled(1),
                pitch_seq: data.selected_sequence(2).clone(),
                pitch_enabled: data.sequence_enabled(2),
                hipitch_seq: data.selected_sequence(3).clone(),
                hipitch_enabled: data.sequence_enabled(3),
                duty_seq: data.selected_sequence(4).clone(),
                duty_enabled: data.sequence_enabled(4),
            }
        };

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
                self.midi_handler
                    .handle_event(&event, &mut self.pulse, &active_seqs);
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
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nice_export_clap!(Rp2a03Plugin);
nice_export_vst3!(Rp2a03Plugin);
