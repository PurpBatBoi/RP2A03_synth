//! `rp2a03_niceplug\src\voice.rs`
//! A single polyphonic voice: one instance of every emulated channel, the MIDI
//! state driving them, and the band-limited resampler they feed.
//!
//! `ChannelMode` picks which channel is audible; the others still exist so a
//! waveform switch does not have to rebuild anything mid-block.

use nice_plug::prelude::*;
use rp2a03_common::{ActiveSequences, ChannelMode, MidiHandler, Modulate};
use rp2a03_core::NTSC_CPU_CLOCK;
use rp2a03_core::apu_noise::Noise;
use rp2a03_core::apu_pulse::{Pulse, PulseChannel};
use rp2a03_core::apu_triangle::Triangle;
use rp2a03_core::blip_buf::{BlipBuf, InvalidRates};
use rp2a03_core::fds_audio::Fds;
use rp2a03_core::s5b_audio::Sunsoft;
use rp2a03_core::vrc6_audio::{Vrc6Pulse, Vrc6Saw};

pub const DEFAULT_SAMPLE_RATE: f32 = 44100.0;

const BLIP_BUFFER_SIZE: u32 = 4096;
const ALLOCATION_RAMP_CLOCKS: u32 = 2048;

const RELEASE_RAMP_CLOCKS: u32 = 2048;

fn ramp_step(difference: i32, remaining: u32) -> i32 {
    if difference == 0 {
        return 0;
    }
    let magnitude = (difference.unsigned_abs() / remaining)
        + u32::from(!difference.unsigned_abs().is_multiple_of(remaining));
    difference.signum() * magnitude.min(i32::MAX as u32) as i32
}

pub struct Channels {
    pulse: Pulse,
    triangle: Triangle,
    noise: Noise,
    vrc6_pulse: Vrc6Pulse,
    vrc6_saw: Vrc6Saw,
    sunsoft: Sunsoft,
    fds: Fds,
}

impl Channels {
    /// The single per-chip selector: everything that needs "whichever chip
    /// `mode` names" — register I/O (`Channel`) and modulation (`Modulate`)
    /// alike — goes through this one match. An 8th chip needs one new field
    /// here and one new arm; nothing else in the crate re-enumerates chips.
    fn select(&mut self, mode: ChannelMode) -> &mut dyn Modulate {
        match mode {
            ChannelMode::Pulse => &mut self.pulse,
            ChannelMode::Triangle => &mut self.triangle,
            ChannelMode::Noise => &mut self.noise,
            ChannelMode::Vrc6Pulse => &mut self.vrc6_pulse,
            ChannelMode::Vrc6Saw => &mut self.vrc6_saw,
            ChannelMode::S5B => &mut self.sunsoft,
            ChannelMode::Fds => &mut self.fds,
        }
    }
}

pub struct Voice {
    pub(crate) midi_handler: MidiHandler,
    pub(crate) blip: BlipBuf,

    pub(crate) alloc_id: u64,
    channels: Channels,
    last_output: i32,
    allocation_ramp_clocks: u32,
    release_ramp_clocks: u32,

    previous_gate: bool,
}

impl Voice {
    pub(crate) fn new() -> Self {
        let mut pulse = Pulse::new(PulseChannel::One);
        pulse.set_enabled(true);
        let mut triangle = Triangle::new();
        triangle.set_enabled(true);
        let mut noise = Noise::new();
        noise.set_enabled(true);
        let mut vrc6_pulse = Vrc6Pulse::new();
        vrc6_pulse.set_enabled(true);
        let mut vrc6_saw = Vrc6Saw::new();
        vrc6_saw.set_enabled(true);
        let sunsoft = Sunsoft::new();
        let fds = Fds::new();
        let mut blip = BlipBuf::new(BLIP_BUFFER_SIZE);
        blip.set_rates(NTSC_CPU_CLOCK, f64::from(DEFAULT_SAMPLE_RATE))
            .expect("the NTSC clock at the default sample rate is a valid rate pair");
        Self {
            midi_handler: MidiHandler::new(),
            blip,
            alloc_id: 0,
            channels: Channels {
                pulse,
                triangle,
                noise,
                vrc6_pulse,
                vrc6_saw,
                sunsoft,
                fds,
            },
            last_output: 0,
            allocation_ramp_clocks: 0,
            release_ramp_clocks: 0,
            previous_gate: false,
        }
    }

    pub(crate) fn set_sample_rate(&mut self, sample_rate: f32) -> Result<(), InvalidRates> {
        self.blip.set_rates(NTSC_CPU_CLOCK, f64::from(sample_rate))
    }

    pub(crate) fn frame_capacity(&self) -> usize {
        self.blip.capacity() as usize
    }

    fn reset_channels(&mut self) {
        self.channels.pulse.reset();
        self.channels.pulse.set_enabled(true);
        self.channels.pulse.write_sweep(0x08);
        self.channels.triangle.reset();
        self.channels.triangle.set_enabled(true);
        self.channels.noise.reset();
        self.channels.noise.set_enabled(true);
        self.channels.vrc6_pulse.reset();
        self.channels.vrc6_pulse.set_enabled(true);
        self.channels.vrc6_saw.reset();
        self.channels.vrc6_saw.set_enabled(true);
        self.channels.sunsoft.reset();

        self.channels.fds.reset();
    }

    pub(crate) fn reset(&mut self) {
        self.reset_channels();
        self.midi_handler.reset();
        self.blip.clear();
        self.last_output = 0;
        self.allocation_ramp_clocks = 0;
        self.release_ramp_clocks = 0;
        self.previous_gate = false;
    }

    pub(crate) fn reset_for_allocation(&mut self) {
        let previous_output = self.last_output;
        let previous_triangle_output = self.channels.triangle.last_output;
        let triangle_sequence = self.channels.triangle.sequence;
        let noise_shift = self.channels.noise.shift;
        let noise_timer_counter = self.channels.noise.timer.counter;

        let fds_wave = self.channels.fds.wave;
        let fds_wave_index = self.channels.fds.wave_index;
        let fds_wave_counter = self.channels.fds.wave_counter;
        self.reset_channels();
        self.channels.fds.wave = fds_wave;
        self.channels.fds.wave_index = fds_wave_index;
        self.channels.fds.wave_counter = fds_wave_counter;

        self.channels.triangle.sequence = triangle_sequence;
        self.channels.triangle.last_output = previous_triangle_output;

        self.channels.noise.shift = noise_shift;
        self.channels.noise.timer.counter = noise_timer_counter;
        self.midi_handler.reset();
        self.last_output = previous_output;
        self.allocation_ramp_clocks = ALLOCATION_RAMP_CLOCKS;

        self.release_ramp_clocks = 0;
        self.previous_gate = false;
    }

    pub(crate) fn begin_triangle_attack_ramp(&mut self) {
        if self.midi_handler.channel_mode == ChannelMode::Triangle && !self.midi_handler.gate() {
            self.allocation_ramp_clocks = ALLOCATION_RAMP_CLOCKS;
        }
    }

    pub(crate) fn gate(&self) -> bool {
        self.midi_handler.gate()
    }

    pub(crate) fn holds_note(&self, note: u8) -> bool {
        self.midi_handler
            .note_stack
            .iter()
            .any(|(held_note, _)| *held_note == note)
    }

    pub(crate) fn handle_event(
        &mut self,
        event: &NoteEvent<()>,
        seqs: &ActiveSequences,
    ) -> Option<usize> {
        let Self {
            midi_handler,
            channels,
            ..
        } = self;

        let channel = channels.select(midi_handler.channel_mode);
        midi_handler.handle_event(event, channel, seqs)
    }

    pub(crate) fn apply_current_modulation(&mut self, seqs: &ActiveSequences) {
        let Self {
            midi_handler,
            channels,
            ..
        } = self;
        midi_handler.sync_channel_mode();
        let chip = channels.select(midi_handler.channel_mode);
        if midi_handler.gate {
            chip.apply_modulation(midi_handler, seqs);
        } else {
            chip.silence(midi_handler, seqs);
        }
    }

    pub(crate) fn is_idle(&self) -> bool {
        !self.midi_handler.gate()
            && self.last_output == 0
            && self.allocation_ramp_clocks == 0
            && self.release_ramp_clocks == 0
    }

    pub(crate) fn clock_channel_output(&mut self) -> i32 {
        let gate = self.midi_handler.gate();
        let mode = self.midi_handler.channel_mode;
        self.channels.select(mode).output_delta(gate)
    }

    pub(crate) fn advance_output(&mut self, channel_output: i32) -> i32 {
        let previous_output = self.last_output;

        let gated = self.midi_handler.gate();
        if self.previous_gate && !gated && self.midi_handler.channel_mode == ChannelMode::Fds {
            self.release_ramp_clocks = RELEASE_RAMP_CLOCKS;
        }
        self.previous_gate = gated;

        if !matches!(
            self.midi_handler.channel_mode,
            ChannelMode::Triangle | ChannelMode::Fds
        ) {
            self.allocation_ramp_clocks = 0;
        }

        if self.release_ramp_clocks > 0 {
            self.last_output = previous_output
                + ramp_step(channel_output - previous_output, self.release_ramp_clocks);
            self.release_ramp_clocks -= 1;
        } else if self.allocation_ramp_clocks > 0 {
            self.last_output = previous_output
                + ramp_step(
                    channel_output - previous_output,
                    self.allocation_ramp_clocks,
                );
            self.allocation_ramp_clocks -= 1;
        } else {
            self.last_output = channel_output;
        }

        self.last_output - previous_output
    }
}
