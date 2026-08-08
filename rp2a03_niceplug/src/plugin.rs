//! rp2a03_niceplug\src\plugin.rs
//! The `nice_plug::Plugin` implementation: lifecycle, parameter/GUI sync, and
//! the block-splitting event loop that drives the voice bank.

use nice_plug::prelude::*;
use rp2a03_common::{HostAutomationControls, MAX_SEQUENCES};
use std::sync::Arc;

use crate::editor;
use crate::params::Rp2a03Params;
use crate::sequences::{PlayheadPublisher, SequenceCache};
use crate::voice::DEFAULT_SAMPLE_RATE;
use crate::voice_bank::VoiceBank;

pub struct Rp2a03Plugin {
    pub(crate) params: Arc<Rp2a03Params>,
    pub(crate) voices: VoiceBank,
    pub(crate) sequences: SequenceCache,
    pub(crate) playheads: PlayheadPublisher,
    pub(crate) sample_rate: f32,
    /// Mono render bus, reused across blocks so `process` never allocates.
    mono_buf: Vec<f32>,
    /// Program Change selection remains active until the host changes the Index parameter.
    midi_program_index: Option<usize>,
    last_sequence_parameter: i32,
}

impl Default for Rp2a03Plugin {
    fn default() -> Self {
        Self {
            params: Arc::new(Rp2a03Params::default()),
            voices: VoiceBank::new(),
            sequences: SequenceCache::default(),
            playheads: PlayheadPublisher::new(),
            sample_rate: DEFAULT_SAMPLE_RATE,
            mono_buf: Vec::new(),
            midi_program_index: None,
            last_sequence_parameter: 0,
        }
    }
}

impl Rp2a03Plugin {
    /// Shared tail of `initialize` and `reset`.
    fn reset_state(&mut self) {
        self.voices.reset_all(self.params.channel_mode());
        self.midi_program_index = None;
        self.last_sequence_parameter = self.params.sequence_number.value();
        self.playheads.clear();
    }

    /// The sequence slot to play this block. A Program Change overrides the
    /// Index parameter until the host moves that parameter again.
    fn resolve_sequence_index(&mut self) -> usize {
        let sequence_parameter = self.params.sequence_number.value();
        if sequence_parameter != self.last_sequence_parameter {
            self.last_sequence_parameter = sequence_parameter;
            self.midi_program_index = None;
        }
        self.midi_program_index
            .unwrap_or(sequence_parameter as usize)
            .min(MAX_SEQUENCES - 1)
    }

    fn refresh_sequences(&mut self, sequence_index: usize) {
        if let Some(reload) = self
            .sequences
            .refresh(&self.params.shared_sequences, sequence_index)
        {
            self.voices.reload_sequences(&self.sequences.active, reload);
        }
    }

    fn render(&mut self, from: usize, to: usize, active_voice_count: usize) {
        self.voices.render(
            &mut self.mono_buf[from..to],
            &self.sequences.active,
            active_voice_count,
            self.sample_rate,
        );
    }

    /// Renders `num_samples` into `self.mono_buf`, splitting the block at each
    /// event so note timing stays sample-accurate.
    ///
    /// Shared by `process` and the render tests, which supply their event
    /// stream from a plain iterator instead of a host `ProcessContext`.
    pub(crate) fn render_block<E>(
        &mut self,
        num_samples: usize,
        events: &mut E,
        active_voice_count: usize,
        host_controls: HostAutomationControls,
    ) where
        E: Iterator<Item = NoteEvent<()>>,
    {
        let mut next_event = events.next();
        self.mono_buf.resize(num_samples, 0.0);

        let mut sequence_index = self.resolve_sequence_index();
        self.refresh_sequences(sequence_index);

        let mut sample_pos: usize = 0;

        loop {
            let chunk_end = next_event.as_ref().map_or(num_samples, |event| {
                (event.timing() as usize).min(num_samples)
            });

            if chunk_end > sample_pos {
                self.render(sample_pos, chunk_end, active_voice_count);
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
                let program_index = self.voices.handle_event(
                    &event,
                    &self.sequences.active,
                    active_voice_count,
                    host_controls,
                );
                if let Some(program_index) = program_index {
                    sequence_index = program_index.min(MAX_SEQUENCES - 1);
                    self.midi_program_index = Some(sequence_index);
                    self.refresh_sequences(sequence_index);
                }
                next_event = events.next();
            }

            if next_event.is_none() && sample_pos < num_samples {
                self.render(sample_pos, num_samples, active_voice_count);
                break;
            }
        }

        self.playheads
            .publish(&self.voices.primary().midi_handler, &self.sequences.active);
    }

    #[cfg(test)]
    pub(crate) fn mono_buf(&self) -> &[f32] {
        &self.mono_buf
    }
}

impl Plugin for Rp2a03Plugin {
    const NAME: &'static str = "RP2A03_Synth";
    const VENDOR: &'static str = "PurpBatBoi";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    // `Basic` delivers note events only. Pitch bend (which drives Pitch Slide),
    // control changes (RPN 0, which drives Pitch Slide Range), and program change
    // all require this level. For VST3 that means the wrapper registers 130*16
    // hidden CC-binding parameters; that is the standard cost of MIDI CC input.
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.playheads.handle())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        if let Err(error) = self.voices.set_sample_rate(buffer_config.sample_rate) {
            // Refusing activation is the only honest answer: rendering with a
            // stale resampler factor would detune every voice silently.
            nice_error!("cannot activate at this sample rate: {error}");
            return false;
        }
        self.sample_rate = buffer_config.sample_rate;
        // Sized here so `process` only ever resizes within existing capacity.
        self.mono_buf
            .resize(buffer_config.max_buffer_size as usize, 0.0);
        self.reset_state();
        nice_log!(
            "activated at {} Hz, max buffer {} samples",
            self.sample_rate,
            buffer_config.max_buffer_size
        );
        true
    }

    fn reset(&mut self) {
        nice_log!("voice state reset");
        self.reset_state();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let host_controls = self.params.host_automation_controls();
        self.voices.apply_host_automation(host_controls);

        // Sync channel mode from host parameter (handles DAW automation / state recall).
        let channel_mode = self.params.channel_mode();
        self.voices.set_channel_mode(channel_mode);
        // Sync editor-visible state so the GUI widgets reflect the current values.
        //
        // `try_lock`, never `lock`: the editor holds this same mutex for the
        // whole egui frame it spends drawing the sequence editor, so blocking
        // here would stall the audio thread behind a UI repaint. Skipping is
        // free — this is a one-way mirror of parameters the GUI re-reads every
        // repaint anyway, so the next block publishes the same values.
        if let Some(mut data) = self.params.shared_sequences.try_lock() {
            self.params.publish_to_gui(&mut data, channel_mode);
        }

        let num_samples = buffer.samples();
        let active_voice_count = self.params.active_voice_count();
        self.voices.retire_above(active_voice_count);

        let mut events = std::iter::from_fn(|| context.next_event());
        self.render_block(num_samples, &mut events, active_voice_count, host_controls);

        for (sample_id, channel_samples) in buffer.iter_samples().enumerate() {
            for out_sample in channel_samples {
                *out_sample = self.mono_buf[sample_id];
            }
        }

        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for Rp2a03Plugin {
    const CLAP_ID: &'static str = "com.rp2a03.synth";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("NES APU+MAPPERS Synthesizer");
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
