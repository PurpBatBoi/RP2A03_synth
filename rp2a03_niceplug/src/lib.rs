use nice_plug::prelude::*;
use rp2a03_core::apu_pulse::{Pulse, PulseChannel};
use rp2a03_core::blip_buf::BlipBuf;
use std::sync::Arc;

use rp2a03_core::NTSC_CPU_CLOCK;

const BLIP_BUFFER_SIZE: u32 = 4096;
const AMPLITUDE_SCALE: i32 = 1500;

pub struct Rp2a03Plugin {
    params: Arc<Rp2a03Params>,
    pulse: Pulse,
    blip: BlipBuf,
    sample_rate: f32,
    last_output: i16,
    midi_note_id: u8,
    gate: bool,
}

#[derive(Params)]
struct Rp2a03Params {
    #[id = "volume"]
    pub volume: IntParam,

    #[id = "duty"]
    pub duty: IntParam,
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
            midi_note_id: 0,
            gate: false,
        }
    }
}

impl Default for Rp2a03Params {
    fn default() -> Self {
        Self {
            volume: IntParam::new(
                "Volume",
                15,
                IntRange::Linear { min: 0, max: 15 },
            ),
            duty: IntParam::new(
                "Duty Cycle",
                2,
                IntRange::Linear { min: 0, max: 3 },
            ),
        }
    }
}

impl Rp2a03Plugin {
    fn freq_to_period(freq: f32) -> u16 {
        let t = (NTSC_CPU_CLOCK as f32 / (16.0 * freq)) - 0.5;
        t.round().clamp(0.0, 2047.0) as u16
    }

    fn generate_samples(&mut self, output: &mut [f32]) {
        let sample_count = output.len() as u32;
        if sample_count == 0 {
            return;
        }

        let clocks_needed = self.blip.clocks_needed(sample_count);

        let volume = self.params.volume.value() as u8;
        let duty = self.params.duty.value() as u8;
        let actual_vol = if self.gate { volume } else { 0 };

        // Set Duty (bits 6,7), Halt Length Counter (bit 5), Constant Volume (bit 4)
        self.pulse.write_ctrl((duty << 6) | 0x20 | 0x10 | actual_vol);

        for clock in 0..clocks_needed {
            self.pulse.clock();

            let current_output = if self.gate && !self.pulse.is_muted() {
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
            output[i] = *sample as f32 / 32768.0;
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

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
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
        self.blip = BlipBuf::new(BLIP_BUFFER_SIZE);
        self.blip.set_rates(NTSC_CPU_CLOCK, buffer_config.sample_rate as f64);
        self.pulse.reset();
        self.pulse.set_enabled(true);
        self.pulse.write_sweep(0x08);
        self.last_output = 0;
        true
    }

    fn reset(&mut self) {
        self.pulse.reset();
        self.pulse.set_enabled(true);
        self.pulse.write_sweep(0x08);
        self.blip.clear();
        self.last_output = 0;
        self.midi_note_id = 0;
        self.gate = false;
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
                    NoteEvent::NoteOn { note, velocity: _, .. } => {
                        self.midi_note_id = note;
                        let effective_note = note.saturating_add(12);
                        let freq = util::midi_note_to_freq(effective_note);
                        let period = Self::freq_to_period(freq);

                        self.pulse.set_enabled(true);
                        self.pulse.write_sweep(0x08);
                        self.pulse.write_timer_lo((period & 0xFF) as u8);
                        self.pulse.write_timer_hi(0xF8 | (((period >> 8) & 0x07) as u8));

                        self.gate = true;
                    }
                    NoteEvent::NoteOff { note, .. } if note == self.midi_note_id => {
                        self.gate = false;
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

        // Copy mono buffer to all stereo output channels
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
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Synth,
    ];
}

nice_export_clap!(Rp2a03Plugin);
nice_export_vst3!(Rp2a03Plugin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_note_on_produces_audio() {
        let mut plugin = Rp2a03Plugin::default();
        plugin.pulse.set_enabled(true);
        plugin.pulse.write_sweep(0x08);

        let note = 60; // Middle C
        let freq = util::midi_note_to_freq(note);
        let period = Rp2a03Plugin::freq_to_period(freq);

        plugin.pulse.write_timer_lo((period & 0xFF) as u8);
        plugin.pulse.write_timer_hi(0xF8 | (((period >> 8) & 0x07) as u8));
        plugin.pulse.write_ctrl((2 << 6) | 0x20 | 0x10 | 12); // Duty 50%, Halt, Constant Vol = 12

        let mut non_zero_samples = 0;
        for _ in 0..1000 {
            plugin.pulse.clock();
            if plugin.pulse.output() > 0.0 {
                non_zero_samples += 1;
            }
        }

        assert!(non_zero_samples > 0, "Pulse channel should produce non-zero samples when note is played");
    }
}
