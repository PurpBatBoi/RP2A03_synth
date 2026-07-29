//! rp2a03_niceplug\src\lib.rs
//! RP2A03 Plugin wrapper using nice-plug.

use nice_plug::prelude::*;
use nice_plug_egui::{create_egui_editor, EguiSettings, EguiState};
use parking_lot::Mutex;
use rp2a03_common::{
    render_editor_ui, style, ActiveSequences, HostAutomationControls, MidiHandler,
    SequencePlayheads, SharedSequences, MAX_SEQUENCES, NO_PLAYHEAD_STEP, SEQUENCE_TYPE_COUNT,
};
use rp2a03_core::apu_pulse::{Pulse, PulseChannel};
use rp2a03_core::blip_buf::BlipBuf;
use rp2a03_core::sequencer::{SeqState, Sequence, SequencePlayer};
use rp2a03_core::NTSC_CPU_CLOCK;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const BLIP_BUFFER_SIZE: u32 = 4096;
const AMPLITUDE_SCALE: i32 = 1500;

pub struct Rp2a03Plugin {
    params: Arc<Rp2a03Params>,
    pulse: Pulse,
    blip: BlipBuf,
    sample_rate: f32,
    last_output: i16,
    midi_handler: MidiHandler,
    /// Program Change selection remains active until the host changes the Index parameter.
    midi_program_index: Option<usize>,
    last_sequence_parameter: i32,
    shared_sequences: Arc<Mutex<SharedSequences>>,
    sequence_playheads: Arc<[AtomicUsize; SEQUENCE_TYPE_COUNT]>,
}

#[derive(Params)]
struct Rp2a03Params {
    #[persist = "editor_state"]
    pub egui_state: Arc<EguiState>,

    /// Selects the same numbered sequence slot for all five pulse envelopes.
    #[id = "sequence_number"]
    pub sequence_number: IntParam,

    #[id = "vibrato_depth"]
    pub vibrato_depth: IntParam,
    #[id = "vibrato_speed"]
    pub vibrato_speed: IntParam,
    #[id = "tremolo_depth"]
    pub tremolo_depth: IntParam,
    #[id = "tremolo_speed"]
    pub tremolo_speed: IntParam,
    #[id = "hardware_volume"]
    pub hardware_volume: IntParam,
    #[id = "fine_pitch"]
    pub fine_pitch: IntParam,
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
            midi_program_index: None,
            last_sequence_parameter: 0,
            shared_sequences: Arc::new(Mutex::new(SharedSequences::default())),
            sequence_playheads: Arc::new(std::array::from_fn(|_| {
                AtomicUsize::new(NO_PLAYHEAD_STEP)
            })),
        }
    }
}

impl Default for Rp2a03Params {
    fn default() -> Self {
        Self {
            egui_state: EguiState::from_size(758, 590),
            sequence_number: IntParam::new(
                "Sequence Index",
                0,
                IntRange::Linear {
                    min: 0,
                    max: (MAX_SEQUENCES - 1) as i32,
                },
            ),
            vibrato_depth: IntParam::new("Vibrato Depth", 0, IntRange::Linear { min: 0, max: 15 }),
            vibrato_speed: IntParam::new("Vibrato Speed", 4, IntRange::Linear { min: 0, max: 63 }),
            tremolo_depth: IntParam::new("Tremolo Depth", 0, IntRange::Linear { min: 0, max: 15 }),
            tremolo_speed: IntParam::new("Tremolo Speed", 4, IntRange::Linear { min: 0, max: 63 }),
            hardware_volume: IntParam::new("HW Volume", 15, IntRange::Linear { min: 0, max: 15 }),
            fine_pitch: IntParam::new("Pitch", 0, IntRange::Linear { min: -64, max: 63 }),
        }
    }
}

impl Rp2a03Plugin {
    fn render_samples_with_current_modulation(&mut self, output: &mut [f32], master_gain: f32) {
        let sample_count = output.len() as u32;
        if sample_count == 0 {
            return;
        }

        let clocks_needed = self.blip.clocks_needed(sample_count);

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

    fn generate_samples(&mut self, output: &mut [f32], seqs: &ActiveSequences) {
        let mut rendered = 0;

        while rendered < output.len() {
            let master_gain = self
                .midi_handler
                .apply_current_modulation(&mut self.pulse, seqs);

            let segment_len = if self.midi_handler.gate() {
                self.midi_handler
                    .samples_until_next_frame(self.sample_rate)
                    .min(output.len() - rendered)
            } else {
                output.len() - rendered
            };

            self.render_samples_with_current_modulation(
                &mut output[rendered..rendered + segment_len],
                master_gain,
            );

            rendered += segment_len;
            self.midi_handler
                .advance_frame_samples(seqs, self.sample_rate, segment_len);
        }
    }

    fn host_automation_controls(&self) -> HostAutomationControls {
        HostAutomationControls {
            vibrato_depth: self.params.vibrato_depth.value() as u8,
            vibrato_speed: self.params.vibrato_speed.value() as u8,
            tremolo_depth: self.params.tremolo_depth.value() as u8,
            tremolo_speed: self.params.tremolo_speed.value() as u8,
            hardware_volume: self.params.hardware_volume.value() as u8,
            fine_pitch: self.params.fine_pitch.value() as i8,
        }
    }

    fn active_sequences(&self, sequence_index: usize) -> ActiveSequences {
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
    }

    fn publish_sequence_playheads(&self, seqs: &ActiveSequences) {
        let steps = [
            sequence_play_step(
                &self.midi_handler.vol_seq_player,
                &seqs.vol_seq,
                seqs.vol_enabled,
            ),
            sequence_play_step(
                &self.midi_handler.arp_seq_player,
                &seqs.arp_seq,
                seqs.arp_enabled,
            ),
            sequence_play_step(
                &self.midi_handler.pitch_seq_player,
                &seqs.pitch_seq,
                seqs.pitch_enabled,
            ),
            sequence_play_step(
                &self.midi_handler.hipitch_seq_player,
                &seqs.hipitch_seq,
                seqs.hipitch_enabled,
            ),
            sequence_play_step(
                &self.midi_handler.duty_seq_player,
                &seqs.duty_seq,
                seqs.duty_enabled,
            ),
        ];

        for (playhead, step) in self.sequence_playheads.iter().zip(steps) {
            playhead.store(step.unwrap_or(NO_PLAYHEAD_STEP), Ordering::Relaxed);
        }
    }

    fn clear_sequence_playheads(&self) {
        for playhead in self.sequence_playheads.iter() {
            playhead.store(NO_PLAYHEAD_STEP, Ordering::Relaxed);
        }
    }
}

fn sequence_play_step(
    player: &SequencePlayer,
    sequence: &Sequence,
    enabled: bool,
) -> Option<usize> {
    if enabled
        && !sequence.is_empty()
        && player.state == SeqState::Running
        && player.pos < sequence.len()
    {
        Some(player.pos)
    } else {
        None
    }
}

fn load_sequence_playheads(playheads: &[AtomicUsize; SEQUENCE_TYPE_COUNT]) -> SequencePlayheads {
    SequencePlayheads::from_steps(std::array::from_fn(|index| {
        let step = playheads[index].load(Ordering::Relaxed);
        (step != NO_PLAYHEAD_STEP).then_some(step)
    }))
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
        let playheads = self.sequence_playheads.clone();

        create_egui_editor(
            self.params.egui_state.clone(),
            (),
            EguiSettings::default(),
            move |ctx, _queue, _state| {
                egui_extras::install_image_loaders(ctx);
                ctx.set_style_of(egui::Theme::Dark, style());
            },
            move |ui, setter, _queue, _state| {
                let mut data = shared.lock();
                let sequence_index = data.selected_sequence_index(0);
                let playheads = load_sequence_playheads(&playheads);

                ui.ctx().request_repaint_after(Duration::from_millis(30));

                egui::Frame::NONE
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        if let Some(new_index) =
                            render_editor_ui(ui, &mut data, sequence_index, &playheads)
                        {
                            data.set_all_selected_sequence_indices(new_index);

                            let new_index = new_index as i32;
                            setter.begin_set_parameter(&params.sequence_number);
                            setter.set_parameter(&params.sequence_number, new_index);
                            setter.end_set_parameter(&params.sequence_number);
                        }
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
        self.blip
            .set_rates(NTSC_CPU_CLOCK, buffer_config.sample_rate as f64);
        self.pulse.reset();
        self.pulse.set_enabled(true);
        self.pulse.write_sweep(0x08);
        self.last_output = 0;
        self.midi_handler.reset();
        self.midi_program_index = None;
        self.last_sequence_parameter = self.params.sequence_number.value();
        self.clear_sequence_playheads();
        true
    }

    fn reset(&mut self) {
        self.pulse.reset();
        self.pulse.set_enabled(true);
        self.pulse.write_sweep(0x08);
        self.blip.clear();
        self.last_output = 0;
        self.midi_handler.reset();
        self.midi_program_index = None;
        self.last_sequence_parameter = self.params.sequence_number.value();
        self.clear_sequence_playheads();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.midi_handler
            .apply_host_automation(self.host_automation_controls());
        let num_samples = buffer.samples();
        let mut next_event = context.next_event();
        let mut sample_pos: usize = 0;
        let mut mono_buf = vec![0.0f32; num_samples];

        let sequence_parameter = self.params.sequence_number.value();
        if sequence_parameter != self.last_sequence_parameter {
            self.last_sequence_parameter = sequence_parameter;
            self.midi_program_index = None;
        }
        let mut sequence_index = self
            .midi_program_index
            .unwrap_or(sequence_parameter as usize)
            .min(MAX_SEQUENCES - 1);
        let mut active_seqs = self.active_sequences(sequence_index);

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
                if let Some(program_index) =
                    self.midi_handler
                        .handle_event(&event, &mut self.pulse, &active_seqs)
                {
                    sequence_index = program_index.min(MAX_SEQUENCES - 1);
                    self.midi_program_index = Some(sequence_index);
                    active_seqs = self.active_sequences(sequence_index);
                }
                next_event = context.next_event();
            }

            if next_event.is_none() && sample_pos < num_samples {
                self.generate_samples(&mut mono_buf[sample_pos..num_samples], &active_seqs);
                break;
            }
        }

        self.publish_sequence_playheads(&active_seqs);

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
