//! rp2a03_niceplug\src\voice.rs
//! A single polyphonic voice: one instance of every emulated channel, the MIDI
//! state driving them, and the band-limited resampler they feed.
//!
//! `ChannelMode` picks which channel is audible; the others still exist so a
//! waveform switch does not have to rebuild anything mid-block.

use nice_plug::prelude::*;
use rp2a03_common::{ActiveSequences, ChannelMode, MidiHandler};
use rp2a03_core::NTSC_CPU_CLOCK;
use rp2a03_core::apu_noise::Noise;
use rp2a03_core::apu_pulse::{Pulse, PulseChannel};
use rp2a03_core::apu_triangle::Triangle;
use rp2a03_core::blip_buf::BlipBuf;
use rp2a03_core::vrc6_pulse::Vrc6Pulse;
use rp2a03_core::vrc6_saw::Vrc6Saw;

/// Sample rate assumed until the host reports the real one in `initialize`.
pub(crate) const DEFAULT_SAMPLE_RATE: f32 = 44100.0;

const BLIP_BUFFER_SIZE: u32 = 4096;
const AMPLITUDE_SCALE: i32 = 1500;
const TRIANGLE_AMPLITUDE_SCALE: i32 = 3000;
const ALLOCATION_RAMP_CLOCKS: u32 = 2048;

pub(crate) struct Voice {
    pub(crate) midi_handler: MidiHandler,
    pub(crate) blip: BlipBuf,
    /// Monotonic allocation stamp; the lowest value is the oldest voice and so
    /// the first candidate for stealing and for FIFO note-off pairing.
    pub(crate) alloc_id: u64,
    pulse: Pulse,
    triangle: Triangle,
    noise: Noise,
    vrc6_pulse: Vrc6Pulse,
    vrc6_saw: Vrc6Saw,
    last_output: i32,
    last_triangle_output: i32,
    allocation_ramp_clocks: u32,
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
        let mut blip = BlipBuf::new(BLIP_BUFFER_SIZE);
        blip.set_rates(NTSC_CPU_CLOCK, DEFAULT_SAMPLE_RATE as f64);
        Self {
            midi_handler: MidiHandler::new(),
            blip,
            alloc_id: 0,
            pulse,
            triangle,
            noise,
            vrc6_pulse,
            vrc6_saw,
            last_output: 0,
            last_triangle_output: 0,
            allocation_ramp_clocks: 0,
        }
    }

    pub(crate) fn set_sample_rate(&mut self, sample_rate: f32) {
        self.blip.set_rates(NTSC_CPU_CLOCK, sample_rate as f64);
    }

    /// Re-enable every channel after a reset. The channels are always enabled;
    /// `ChannelMode` — not the enable flag — decides which one is heard.
    fn reset_channels(&mut self) {
        self.pulse.reset();
        self.pulse.set_enabled(true);
        self.pulse.write_sweep(0x08);
        self.triangle.reset();
        self.triangle.set_enabled(true);
        self.noise.reset();
        self.noise.set_enabled(true);
        self.vrc6_pulse.reset();
        self.vrc6_pulse.set_enabled(true);
        self.vrc6_saw.reset();
        self.vrc6_saw.set_enabled(true);
    }

    pub(crate) fn reset(&mut self) {
        self.reset_channels();
        self.midi_handler.reset();
        self.blip.clear();
        self.last_output = 0;
        self.last_triangle_output = 0;
        self.allocation_ramp_clocks = 0;
    }

    /// Reset synthesis state for voice allocation while preserving the previous
    /// output level. The next BlipBuf delta then transitions from the old note
    /// to the new note instead of introducing an artificial zero-level click.
    pub(crate) fn reset_for_allocation(&mut self) {
        let previous_output = self.last_output;
        let previous_triangle_output = self.last_triangle_output;
        let triangle_sequence = self.triangle.sequence;
        self.reset_channels();
        // A triangle timer/register write does not reset the waveform's 32-step
        // phase. Preserve it when recycling a polyphonic voice so the DAC does
        // not jump from the held release level back to sequence step zero.
        self.triangle.sequence = triangle_sequence;
        self.midi_handler.reset();
        self.last_output = previous_output;
        self.last_triangle_output = previous_triangle_output;
        self.allocation_ramp_clocks = ALLOCATION_RAMP_CLOCKS;
    }

    /// Smooth a fresh triangle attack when the voice is reused in monophonic
    /// mode. Polyphonic voice recycling sets this as part of allocation reset,
    /// but the single voice is intentionally not reset between notes.
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

    /// Routes a MIDI event to every channel this voice owns, returning a
    /// Program Change sequence index when the event carried one.
    pub(crate) fn handle_event(
        &mut self,
        event: &NoteEvent<()>,
        seqs: &ActiveSequences,
    ) -> Option<usize> {
        self.midi_handler.handle_event_all(
            event,
            &mut self.pulse,
            &mut self.triangle,
            Some(&mut self.noise),
            Some(&mut self.vrc6_pulse),
            Some(&mut self.vrc6_saw),
            seqs,
        )
    }

    /// Folds the current envelope/LFO state into the channel registers and
    /// returns the master gain to apply to this voice for the coming segment.
    pub(crate) fn apply_current_modulation(&mut self, seqs: &ActiveSequences) -> f32 {
        self.midi_handler.apply_current_modulation_all(
            &mut self.pulse,
            &mut self.triangle,
            Some(&mut self.noise),
            Some(&mut self.vrc6_pulse),
            Some(&mut self.vrc6_saw),
            seqs,
        )
    }

    /// Advances the currently selected channel by one APU clock and returns its
    /// raw (pre-gain) DAC level.
    pub(crate) fn clock_channel_output(&mut self) -> i32 {
        match self.midi_handler.channel_mode {
            ChannelMode::Pulse => {
                self.pulse.clock();
                if self.midi_handler.gate() && !self.pulse.is_muted() {
                    self.pulse.output() as i32 * AMPLITUDE_SCALE
                } else {
                    0
                }
            }
            ChannelMode::Noise => {
                self.noise.clock();
                if self.midi_handler.gate() && !self.noise.is_muted() {
                    self.noise.output() as i32 * AMPLITUDE_SCALE
                } else {
                    0
                }
            }
            ChannelMode::Triangle => {
                // The triangle DAC holds its last level when ungated rather than
                // snapping to zero, so the release tail stays click-free.
                if self.midi_handler.gate() {
                    self.triangle.clock();
                    if !self.triangle.is_muted() {
                        let output =
                            (self.triangle.output() * TRIANGLE_AMPLITUDE_SCALE as f32) as i32;
                        self.last_triangle_output = output;
                        output
                    } else {
                        0
                    }
                } else {
                    self.last_triangle_output
                }
            }
            ChannelMode::Vrc6Pulse => {
                self.vrc6_pulse.clock();
                if self.midi_handler.gate() {
                    self.vrc6_pulse.volume() as i32 * AMPLITUDE_SCALE
                } else {
                    0
                }
            }
            ChannelMode::Vrc6Saw => {
                self.vrc6_saw.clock();
                if self.midi_handler.gate() {
                    (self.vrc6_saw.volume() as i32 * AMPLITUDE_SCALE) / 2
                } else {
                    0
                }
            }
        }
    }

    /// Moves the voice's output level towards `channel_output * gain` and
    /// returns the BlipBuf delta for this clock.
    ///
    /// A recycled voice may still be holding the previous note's level in
    /// BlipBuf. While `allocation_ramp_clocks` is counting down, the new target
    /// is chased over a short APU-clock ramp instead of submitting one large
    /// allocation delta at clock zero. Only the triangle needs this — every
    /// other channel restarts from silence, so the ramp is cancelled for them.
    pub(crate) fn advance_output(&mut self, channel_output: i32, gain: f32) -> i32 {
        let previous_output = self.last_output;
        let target_output = (channel_output as f32 * gain) as i32;

        if self.midi_handler.channel_mode != ChannelMode::Triangle {
            self.allocation_ramp_clocks = 0;
        }

        if self.allocation_ramp_clocks > 0 {
            let remaining = self.allocation_ramp_clocks;
            let difference = target_output - previous_output;
            let step = if difference == 0 {
                0
            } else {
                // Round the per-clock step up so the ramp always lands on the
                // target within `remaining` clocks rather than undershooting.
                let magnitude = (difference.unsigned_abs() / remaining)
                    + u32::from(!difference.unsigned_abs().is_multiple_of(remaining));
                difference.signum() * magnitude.min(i32::MAX as u32) as i32
            };
            self.last_output = previous_output + step;
            self.allocation_ramp_clocks -= 1;
        } else {
            self.last_output = target_output;
        }

        self.last_output - previous_output
    }
}
