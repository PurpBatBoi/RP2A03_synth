use nih_plug::prelude::*;
use std::num::NonZeroU32;
use std::sync::Arc;

use rp2a03_core::blip_buf::BlipBuf;
use rp2a03_core::nes_square::{
    midi_note_to_freq, period_for_frequency, FrameSequencer, FrameTick, SquareChannel,
    NTSC_CPU_CLOCK,
};

const BLIP_BUFFER_SIZE: i32 = 4096;
/// Scale factor so that a full 0–15 swing maps to the full i16 range.
/// 32767 / 15 ≈ 2184
const AMPLITUDE_SCALE: i32 = 2184;

struct NesSynth {
    params: Arc<NesSynthParams>,
    square: SquareChannel,
    frame_seq: FrameSequencer,
    sample_rate: f32,
    blip: BlipBuf,
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
        let mut blip = BlipBuf::new(BLIP_BUFFER_SIZE).expect("failed to create BlipBuf");
        blip.set_rates(NTSC_CPU_CLOCK, 44100.0);

        Self {
            params: Arc::new(NesSynthParams::default()),
            square: SquareChannel::new(true),
            frame_seq: FrameSequencer::default(),
            sample_rate: 44100.0,
            blip,
        }
    }
}

impl NesSynth {
    fn note_on(&mut self, note: u8, velocity: f32) {
        let period = period_for_frequency(midi_note_to_freq(note));
        let duty = self.params.duty.value() as u8;
        let volume = ((self.params.volume.value() as f32) * velocity).round() as u8;

        self.square.set_enabled(true);
        self.square
            .write_reg0((duty << 6) | 0x10 | (volume & 0x0F));
        self.square.write_reg2((period & 0xFF) as u8);
        self.square
            .write_reg3((0x1Fu8 << 3) | ((period >> 8) as u8 & 0x07));
    }

    fn note_off(&mut self) {
        self.square.set_enabled(false);
    }

    /// Run enough NES CPU clocks to produce `sample_count` output samples,
    /// then read them out of the BlipBuf into the provided slice.
    fn generate_samples(&mut self, output: &mut [f32]) {
        let sample_count = output.len() as i32;
        let clocks_needed = self.blip.clocks_needed(sample_count) as u32;

        for clock in 0..clocks_needed {
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

            // take_delta() returns the change in output level since last call.
            // That's exactly what BlipBuf wants.
            let delta = self.square.take_delta();
            if delta != 0 {
                self.blip
                    .add_delta(clock, delta as i32 * AMPLITUDE_SCALE);
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
    const NAME: &'static str = "NES Square Synth";
    const VENDOR: &'static str = "Your Name";
    const URL: &'static str = "https://example.com";
    const EMAIL: &'static str = "you@example.com";
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
        self.blip.clear();
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
    const CLAP_ID: &'static str = "com.example.nes-square-synth";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("NES square channel synth");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Mono,
    ];
}

impl Vst3Plugin for NesSynth {
    const VST3_CLASS_ID: [u8; 16] = *b"NesSquareSynth01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nih_export_clap!(NesSynth);
nih_export_vst3!(NesSynth);