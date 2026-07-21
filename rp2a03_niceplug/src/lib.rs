use nice_plug::prelude::*;
use rp2a03_core::apu_pulse::{Pulse, PulseChannel};
use std::sync::Arc;

pub struct Rp2a03Plugin {
    params: Arc<Rp2a03Params>,
    pulse: Pulse,
    sample_rate: f32,
    
    apu_cycle_accumulator: f32,
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
        Self {
            params: Arc::new(Rp2a03Params::default()),
            pulse,
            sample_rate: 44100.0,
            
            apu_cycle_accumulator: 0.0,
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
                12,
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
        let cpu_freq = 1789773.0;
        let t = (cpu_freq / (16.0 * freq)) - 0.5;
        t.round().clamp(0.0, 2047.0) as u16
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
        self.pulse.reset();
        self.pulse.set_enabled(true);
        self.pulse.write_sweep(0x08);
        true
    }

    fn reset(&mut self) {
        self.pulse.reset();
        self.pulse.set_enabled(true);
        self.pulse.write_sweep(0x08);
        self.apu_cycle_accumulator = 0.0;
        self.midi_note_id = 0;
        self.gate = false;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut next_event = context.next_event();
        let apu_freq = 1789773.0 / 2.0;
        let cycles_per_sample = apu_freq / self.sample_rate;

        for (sample_id, channel_samples) in buffer.iter_samples().enumerate() {
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }

                match event {
                    NoteEvent::NoteOn { note, velocity: _, .. } => {
                        self.midi_note_id = note;
                        let freq = util::midi_note_to_freq(note);
                        let period = Self::freq_to_period(freq);
                        
                        self.pulse.set_enabled(true);
                        self.pulse.write_sweep(0x08); // Negate true to prevent period > 1023 muting
                        self.pulse.write_timer_lo((period & 0xFF) as u8);
                        // D7..D3 = length counter load index (0xF8 = index 31), D2..D0 = period hi
                        self.pulse.write_timer_hi(0xF8 | (((period >> 8) & 0x07) as u8));
                        
                        self.gate = true;
                    }
                    NoteEvent::NoteOff { note, .. } if note == self.midi_note_id => {
                        self.gate = false;
                    }
                    _ => (),
                }
                next_event = context.next_event();
            }

            let volume = self.params.volume.value() as u8;
            let duty = self.params.duty.value() as u8;
            let actual_vol = if self.gate { volume } else { 0 };
            
            // Set Duty (bits 6,7), Halt Length Counter (bit 5), Constant Volume (bit 4)
            self.pulse.write_ctrl((duty << 6) | 0x20 | 0x10 | actual_vol);

            self.apu_cycle_accumulator += cycles_per_sample;
            while self.apu_cycle_accumulator >= 1.0 {
                self.pulse.clock();
                self.apu_cycle_accumulator -= 1.0;
            }

            // Output clean centered bipolar audio when gate is on, 0.0 when gate is off
            let output_scaled = if self.gate && !self.pulse.is_muted() {
                let amp = (volume as f32 / 15.0) * 0.25;
                if self.pulse.output() > 0.0 {
                    amp
                } else {
                    -amp
                }
            } else {
                0.0
            };

            for sample in channel_samples {
                *sample = output_scaled;
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

