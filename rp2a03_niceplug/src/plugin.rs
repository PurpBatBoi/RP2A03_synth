//! `rp2a03_niceplug\src\plugin.rs`
//! The `nice_plug::Plugin` implementation: lifecycle, parameter/GUI sync, and
//! the block-splitting event loop that drives the voice bank.

use nice_plug::prelude::*;
use rp2a03_common::{HostAutomationSnapshot, MAX_SEQUENCES, SharedSequences};
use std::sync::Arc;
use triple_buffer::Output;

use crate::editor;
use crate::params::Rp2a03Params;
use crate::sequences::{
    PlayheadPublisher, ResolvedFdsWaves, SequenceCache, SequenceIndexPublisher,
    SharedSequencesHandle,
};
use crate::voice::DEFAULT_SAMPLE_RATE;
use crate::voice_bank::VoiceBank;

pub struct Rp2a03Plugin {
    pub(crate) params: Arc<Rp2a03Params>,
    pub(crate) voices: VoiceBank,
    pub(crate) sequences: SequenceCache,
    sequences_output: Output<SharedSequences>,
    fds_waves_output: Output<ResolvedFdsWaves>,
    pub(crate) playheads: PlayheadPublisher,
    pub(crate) active_sequence_index: SequenceIndexPublisher,
    pub(crate) sample_rate: f32,
    /// Mono render bus, reused across blocks so `process` never allocates.
    mono_buf: Vec<f32>,
    /// Program Change selection remains active until the host changes the Index parameter.
    midi_program_index: Option<usize>,
    last_sequence_parameter: i32,
    /// TEMPORARY diagnostic for an open, believed-CLAP-specific FL Studio
    /// "replace file" export crash — remove alongside the logging in
    /// `process_inner` once it's confirmed and fixed.
    first_process_logged: bool,
}

impl Default for Rp2a03Plugin {
    fn default() -> Self {
        let (shared_sequences, sequences_output, fds_waves_output) = SharedSequencesHandle::new();
        Self {
            params: Arc::new(Rp2a03Params::new(shared_sequences)),
            voices: VoiceBank::new(),
            sequences: SequenceCache::default(),
            sequences_output,
            fds_waves_output,
            playheads: PlayheadPublisher::new(),
            active_sequence_index: SequenceIndexPublisher::new(),
            sample_rate: DEFAULT_SAMPLE_RATE,
            mono_buf: Vec::new(),
            midi_program_index: None,
            last_sequence_parameter: 0,
            first_process_logged: false,
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
        self.active_sequence_index.clear();
        self.first_process_logged = false;
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
        if let Some(reload) = self.sequences.refresh(
            &mut self.sequences_output,
            &mut self.fds_waves_output,
            sequence_index,
        ) {
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
        host_controls: HostAutomationSnapshot,
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
        self.active_sequence_index.publish(sequence_index);
    }

    #[cfg(feature = "baseline")]
    pub(crate) fn mono_buf(&self) -> &[f32] {
        &self.mono_buf
    }

    /// `render_block`, wrapped so a test can assert the real render path
    /// never allocates. Not used by `process` itself, and only ever called
    /// from tests — the release allocator is never `AllocDisabler` (see
    /// `lib.rs`) regardless.
    #[cfg(test)]
    pub(crate) fn no_alloc_render<E>(
        &mut self,
        num_samples: usize,
        events: &mut E,
        active_voice_count: usize,
        host_controls: HostAutomationSnapshot,
    ) where
        E: Iterator<Item = NoteEvent<()>>,
    {
        assert_no_alloc::assert_no_alloc(|| {
            self.render_block(num_samples, events, active_voice_count, host_controls);
        });
    }

    fn process_inner(
        &mut self,
        buffer: &mut Buffer,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // TEMPORARY diagnostic for an open, believed-CLAP-specific export
        // crash: logs only the first `process` call after each
        // `initialize`/`reset` (the flag is cleared in `reset_state`), with
        // the instance pointer and thread id, to line up against
        // `initialize`/`reset`'s own instance+thread logging and confirm the
        // FL Studio "replace file" export crash is one instance being
        // reactivated on a thread other than the one still calling
        // `process` on it. Remove once confirmed.
        if !self.first_process_logged {
            self.first_process_logged = true;
            nice_log!(
                "first process since last activate/reset on instance {:p}, thread {:?}, {} samples",
                self,
                std::thread::current().id(),
                buffer.samples()
            );
        }

        let host_controls = self.params.host_automation_snapshot();
        self.voices.apply_host_automation(host_controls);
        let channel_mode = self.params.channel_mode();
        self.voices.set_channel_mode(channel_mode);

        let num_samples = buffer.samples();
        let active_voice_count = self.params.active_voice_count();
        self.voices.retire_above(active_voice_count);

        let mut events = std::iter::from_fn(|| context.next_event());
        self.render_block(num_samples, &mut events, active_voice_count, host_controls);

        // The host is a trust boundary: nice-plug fills any output port it
        // could not resolve with an empty slice while leaving `num_samples`
        // at the full block length (offline bounce with a muted/inactive
        // track is the common case), so a channel's real length must be
        // checked before every write rather than trusted to match.
        for channel in buffer.as_slice() {
            if channel.len() < num_samples {
                continue;
            }
            channel[..num_samples].copy_from_slice(&self.mono_buf[..num_samples]);
        }

        ProcessStatus::KeepAlive
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
        editor::create(
            self.params.clone(),
            self.playheads.handle(),
            self.active_sequence_index.handle(),
        )
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
        // TEMPORARY diagnostic for an open, believed-CLAP-specific export
        // crash: instance pointer + thread id, to confirm the FL Studio
        // "replace file"
        // export crash is one instance being reactivated on a different
        // thread than the one still running `process` on it. Remove once
        // confirmed.
        nice_log!(
            "activated at {} Hz, max buffer {} samples (instance {:p}, thread {:?})",
            self.sample_rate,
            buffer_config.max_buffer_size,
            self,
            std::thread::current().id()
        );
        true
    }

    fn reset(&mut self) {
        nice_log!(
            "voice state reset (instance {:p}, thread {:?})",
            self,
            std::thread::current().id()
        );
        self.reset_state();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Last-resort insurance for a `cdylib`: a panic that reaches the host
        // across the FFI boundary is undefined behavior, not a clean abort.
        // `AssertUnwindSafe` is warranted here specifically because a caught
        // panic is followed by silence, not by continued use of whatever
        // partial state `self`/`buffer` were left in.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.process_inner(buffer, context)
        }));

        outcome.unwrap_or_else(|_| {
            for channel in buffer.as_slice() {
                channel.fill(0.0);
            }
            ProcessStatus::Error("panic during processing; see crash.log")
        })
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
