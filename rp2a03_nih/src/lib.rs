use nih_plug::prelude::*;
use std::num::NonZeroU32;
use std::sync::Arc;

use rp2a03_core::nes_square::{midi_note_to_freq, period_for_frequency, FrameSequencer, FrameTick, SquareChannel, NTSC_CPU_CLOCK};
use rp2a03_core::blip::{BlepKernel, BlipLine};

struct NesSynth {
    params: Arc<NesSynthParams>,
    square: SquareChannel,
    frame_seq: FrameSequencer,
    /// Fractional NES-cycle accumulator, since the host sample rate and the
    /// NTSC APU clock don't divide evenly.
    cycle_accum: f64,
    sample_rate: f32,

    blep_kernel: BlepKernel,
    blip: BlipLine,
}

#[derive(Params)]
struct NesSynthParams {
    #[id = "duty"]
    pub duty: IntParam,
    #[id = "volume"]
    pub volume: IntParam,
}

impl Default for NesSynthParams {
    fn default() -> Self {
        Self {
            duty: IntParam::new("Duty", 2, IntRange::Linear { min: 0, max: 3 }),
            volume: IntParam::new("Volume", 15, IntRange::Linear { min: 0, max: 15 }),
        }
    }
}

impl Default for NesSynth {
    fn default() -> Self {
        Self {
            params: Arc::new(NesSynthParams::default()),
            // `true` = this is pulse 1 (matters for the sweep negate quirk;
            // doesn't matter at all if you never touch the sweep register).
            square: SquareChannel::new(true),
            frame_seq: FrameSequencer::default(),
            cycle_accum: 0.0,
            sample_rate: 44100.0,

            // Generating the kernel does a bit of trig work but it's a
            // one-time cost at plugin load, not per-sample.
            blep_kernel: BlepKernel::generate(),
            blip: BlipLine::new(),
        }
    }
}

impl NesSynth {
    fn note_on(&mut self, note: u8, velocity: f32) {
        let period = period_for_frequency(midi_note_to_freq(note));
        let duty = self.params.duty.value() as u8;
        let volume = ((self.params.volume.value() as f32) * velocity).round() as u8;

        self.square.set_enabled(true);
        // reg0: duty (bits 6-7) | constant-volume flag (bit 4) | volume (bits 0-3)
        self.square.write_reg0((duty << 6) | 0x10 | (volume & 0x0F));
        self.square.write_reg2((period & 0xFF) as u8);
        // reg3: length-counter load (bits 3-7, use max = 0x1F so it never
        // silences itself) | period high bits (bits 0-2)
        self.square
            .write_reg3((0x1Fu8 << 3) | ((period >> 8) as u8 & 0x07));
    }

    fn note_off(&mut self) {
        self.square.set_enabled(false);
    }

    /// Advances the chip by however many NES cycles correspond to one host
    /// sample, running the frame sequencer alongside it, and returns the
    /// current output normalized to [-1.0, 1.0].
    ///
    /// Every NES cycle, we check whether the channel's output level changed
    /// (from a duty step, a register write, or the sweep unit) and deposit
    /// it into the BLEP line at its exact fractional position within this
    /// sample -- that's what actually does the anti-aliasing. There's a
    /// small (HALF_WIDTH-sample) fixed latency baked into the BLEP line;
    /// that's normal for this technique and not audible.
    fn next_sample(&mut self) -> f32 {
        self.cycle_accum += NTSC_CPU_CLOCK / self.sample_rate as f64;
        let total_cycles = self.cycle_accum; // snapshot: cycles landing in this sample
        let mut cycle_index = 0f64;

        while self.cycle_accum >= 1.0 {
            self.cycle_accum -= 1.0;
            match self.frame_seq.clock() {
                FrameTick::Quarter => self.square.tick_envelope(),
                FrameTick::Half => {
                    self.square.tick_length_counter();
                    self.square.tick_sweep();
                }
                FrameTick::None => {}
            }
            self.square.reload_length_counter();
            self.square.clock();

            let delta = self.square.take_delta();
            if delta != 0 {
                let frac = (cycle_index / total_cycles).clamp(0.0, 1.0);
                self.blip.add_delta(&self.blep_kernel, frac, delta as f32);
            }
            cycle_index += 1.0;
        }

        let level = self.blip.end_sample(); // still in 0-15 chip units
        (level / 15.0) * 2.0 - 1.0
    }
}

impl Plugin for NesSynth {
    const NAME: &'static str = "NES Square Synth";
    const VENDOR: &'static str = "Your Name";
    const URL: &'static str = "https://example.com";
    const EMAIL: &'static str = "you@example.com";
    const VERSION: &'static str = "0.1.0";

    // No audio inputs -- this is a synth. Stereo out (we'll just duplicate
    // the mono chip output to both channels).
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

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        true
    }

    fn reset(&mut self) {
        self.cycle_accum = 0.0;
        self.frame_seq = FrameSequencer::default();
        self.square = SquareChannel::new(true);
        self.blip = BlipLine::new();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut next_event = context.next_event();

        for (sample_id, mut channel_samples) in buffer.iter_samples().enumerate() {
            // Dispatch any MIDI events timed to land on this sample.
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
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

            let sample_value = self.next_sample();
            for out_sample in channel_samples.iter_mut() {
                *out_sample = sample_value;
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for NesSynth {
    const CLAP_ID: &'static str = "com.example.nes-square-synth";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("NES square channel synth");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::Instrument, ClapFeature::Synthesizer, ClapFeature::Mono];
}

impl Vst3Plugin for NesSynth {
    const VST3_CLASS_ID: [u8; 16] = *b"NesSquareSynth01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nih_export_clap!(NesSynth);
nih_export_vst3!(NesSynth);