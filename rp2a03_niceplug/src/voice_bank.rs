//! `rp2a03_niceplug\src\voice_bank.rs`
//! Polyphony: voice allocation and stealing, per-voice event routing, and the
//! per-clock render loop that mixes every active voice down to one mono bus.
//!
//! Everything here runs on the audio thread. All scratch buffers are owned by
//! the bank and reused in place so no render path allocates.

use nice_plug::prelude::*;
use rp2a03_common::{ActiveSequences, ChannelMode, HostAutomationSnapshot, SequenceReload};
use rp2a03_core::blip_buf::InvalidRates;

use crate::voice::Voice;

pub const MAX_VOICES: usize = 8;

const MASTER_OUTPUT_GAIN: f32 = 0.158_489_3;

pub struct VoiceBank {
    voices: Vec<Voice>,

    sample_scratch: Vec<i16>,
    alloc_counter: u64,
    last_active_voice_count: usize,
}

impl VoiceBank {
    pub(crate) fn new() -> Self {
        let voices: Vec<Voice> = (0..MAX_VOICES).map(|_| Voice::new()).collect();
        // Every voice shares one `frame_capacity()`, so sizing scratch once
        // here up front means `render_segment` never grows it on the audio
        // thread — a segment is never longer than that capacity.
        let frame_capacity = voices[0].frame_capacity();
        Self {
            voices,
            sample_scratch: vec![0; frame_capacity],
            alloc_counter: 0,
            last_active_voice_count: 1,
        }
    }

    pub(crate) fn primary(&self) -> &Voice {
        &self.voices[0]
    }

    pub(crate) fn set_sample_rate(&mut self, sample_rate: f32) -> Result<(), InvalidRates> {
        for voice in &mut self.voices {
            voice.set_sample_rate(sample_rate)?;
        }
        Ok(())
    }

    fn frame_capacity(&self) -> usize {
        self.voices[0].frame_capacity()
    }

    pub(crate) fn reset_all(&mut self, channel_mode: ChannelMode) {
        for voice in &mut self.voices {
            voice.reset();
            voice.midi_handler.channel_mode = channel_mode;
        }
        self.alloc_counter = 0;
        self.last_active_voice_count = 1;
    }

    pub(crate) fn apply_host_automation(&mut self, controls: HostAutomationSnapshot) {
        for voice in &mut self.voices {
            voice.midi_handler.apply_host_automation(controls);
        }
    }

    pub(crate) fn set_channel_mode(&mut self, channel_mode: ChannelMode) {
        for voice in &mut self.voices {
            voice.midi_handler.channel_mode = channel_mode;
        }
    }

    pub(crate) fn retire_above(&mut self, active_voice_count: usize) {
        if active_voice_count < self.last_active_voice_count {
            for voice in &mut self.voices[active_voice_count..self.last_active_voice_count] {
                voice.reset();
            }
        }
        self.last_active_voice_count = active_voice_count;
    }

    pub(crate) fn reload_sequences(&mut self, seqs: &ActiveSequences, reload: SequenceReload) {
        for voice in &mut self.voices {
            voice.midi_handler.reload_sequences(seqs, reload);
        }
    }

    fn select_voice(
        &mut self,
        active_voice_count: usize,
        controls: HostAutomationSnapshot,
    ) -> usize {
        if active_voice_count == 1 || self.voices[0].midi_handler.channel_mode == ChannelMode::Noise
        {
            return 0;
        }

        let index = self.voices[..active_voice_count]
            .iter()
            .position(|voice| !voice.gate())
            .unwrap_or_else(|| {
                self.voices[..active_voice_count]
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, voice)| voice.alloc_id)
                    .map_or(0, |(index, _)| index)
            });

        self.voices[index].reset_for_allocation();

        self.voices[index]
            .midi_handler
            .apply_host_automation(controls);
        self.alloc_counter = self.alloc_counter.wrapping_add(1);
        self.voices[index].alloc_id = self.alloc_counter;
        index
    }

    fn find_note_off_target(&self, note: u8, active_voice_count: usize) -> Option<usize> {
        self.voices[..active_voice_count]
            .iter()
            .enumerate()
            .filter(|(_, voice)| voice.holds_note(note))
            .min_by_key(|(_, voice)| voice.alloc_id)
            .map(|(index, _)| index)
    }

    pub(crate) fn handle_event(
        &mut self,
        event: &NoteEvent<()>,
        seqs: &ActiveSequences,
        active_voice_count: usize,
        controls: HostAutomationSnapshot,
    ) -> Option<usize> {
        match event {
            NoteEvent::NoteOn { .. } => {
                let index = self.select_voice(active_voice_count, controls);
                let voice = &mut self.voices[index];
                voice.begin_triangle_attack_ramp();
                voice.handle_event(event, seqs)
            }
            NoteEvent::NoteOff { note, .. } => self
                .find_note_off_target(*note, active_voice_count)
                .and_then(|index| self.voices[index].handle_event(event, seqs)),

            NoteEvent::Choke { .. }
            | NoteEvent::MidiCC { .. }
            | NoteEvent::MidiPitchBend { .. } => {
                let mut program = None;
                for voice in &mut self.voices {
                    program = voice.handle_event(event, seqs).or(program);
                }
                program
            }
            NoteEvent::MidiProgramChange { .. } => self.voices[0].handle_event(event, seqs),
            _ => None,
        }
    }

    pub(crate) fn render(
        &mut self,
        output: &mut [f32],
        seqs: &ActiveSequences,
        active_voice_count: usize,
        sample_rate: f32,
    ) {
        let mut rendered = 0;

        while rendered < output.len() {
            let mut segment_len = output.len() - rendered;
            let mut any_gated = false;

            for voice in &mut self.voices[..active_voice_count] {
                voice.apply_current_modulation(seqs);
                if voice.gate() {
                    any_gated = true;
                    segment_len =
                        segment_len.min(voice.midi_handler.samples_until_next_frame(sample_rate));
                }
            }

            if !any_gated {
                segment_len = output.len() - rendered;
            }

            segment_len = segment_len.min(self.frame_capacity());

            self.render_segment(
                &mut output[rendered..rendered + segment_len],
                active_voice_count,
            );
            rendered += segment_len;

            for voice in &mut self.voices[..active_voice_count] {
                voice
                    .midi_handler
                    .advance_frame_samples(seqs, sample_rate, segment_len);
            }
        }
    }

    fn render_segment(&mut self, output: &mut [f32], active_voice_count: usize) {
        let sample_count = output.len() as u32;
        if sample_count == 0 {
            return;
        }

        let clocks_needed = self.voices[0].blip.clocks_needed(sample_count);

        for voice in &mut self.voices[..active_voice_count] {
            if !voice.is_idle() {
                for clock in 0..clocks_needed {
                    let channel_output = voice.clock_channel_output();
                    let delta = voice.advance_output(channel_output);
                    if delta != 0 {
                        voice.blip.add_delta(clock, delta);
                    }
                }
            }
            voice.blip.end_frame(clocks_needed);
        }

        self.sample_scratch.resize(output.len(), 0);
        output.fill(0.0);
        for voice in &mut self.voices[..active_voice_count] {
            self.sample_scratch.fill(0);
            let samples_read = voice.blip.read_samples(&mut self.sample_scratch, false);
            accumulate_voice_samples(
                &mut output[..samples_read],
                &self.sample_scratch[..samples_read],
            );
        }
        for sample in output.iter_mut() {
            *sample = (*sample * MASTER_OUTPUT_GAIN).clamp(-1.0, 1.0);
        }
    }
}

/// Converts one voice's rendered samples to `f32` (scaled to `[-1.0, 1.0]`)
/// and sums them into `output`. Normally private to `render_segment`'s
/// per-voice mixdown loop — `pub` (and re-exported `#[doc(hidden)]` from
/// `lib.rs`) only so the M7 step-45 criterion benchmark
/// (`benches/mixdown.rs`) can drive the exact same hot loop production runs.
pub fn accumulate_voice_samples(output: &mut [f32], samples: &[i16]) {
    for (out, &sample) in output.iter_mut().zip(samples) {
        *out += f32::from(sample) / 32768.0;
    }
}
