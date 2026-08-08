//! rp2a03_niceplug\src\voice_bank.rs
//! Polyphony: voice allocation and stealing, per-voice event routing, and the
//! per-clock render loop that mixes every active voice down to one mono bus.
//!
//! Everything here runs on the audio thread. All scratch buffers are owned by
//! the bank and reused in place so no render path allocates.

use nice_plug::prelude::*;
use rp2a03_common::{ActiveSequences, ChannelMode, HostAutomationControls, SequenceReload};
use rp2a03_core::blip_buf::InvalidRates;

use crate::voice::Voice;

pub(crate) const MAX_VOICES: usize = 8;

// Nominal synth-instance output calibration: 20*log10(0.1584893) = -16 dBFS.
// This trims the final bus without changing per-voice, polyphony, or MIDI gain.
// Voices sum freely, so this is the headroom budget for the whole mix: a single
// voice at max velocity peaks near -10 dBFS raw, and MAX_VOICES of those summing
// in phase reach ~2.53, so -16 dBFS keeps a full chord well under the output clamp
// while landing a single test note at a comfortable -26 dBFS.
const MASTER_OUTPUT_GAIN: f32 = 0.158_489_3;

pub(crate) struct VoiceBank {
    voices: Vec<Voice>,
    /// Per-voice master gain for the segment currently being rendered.
    master_gains: Vec<f32>,
    /// Landing buffer for `BlipBuf::read_samples` before summing into the bus.
    sample_scratch: Vec<i16>,
    alloc_counter: u64,
    last_active_voice_count: usize,
}

impl VoiceBank {
    pub(crate) fn new() -> Self {
        Self {
            voices: (0..MAX_VOICES).map(|_| Voice::new()).collect(),
            master_gains: Vec::with_capacity(MAX_VOICES),
            sample_scratch: Vec::new(),
            alloc_counter: 0,
            last_active_voice_count: 1,
        }
    }

    /// The voice whose state the editor's playhead display follows.
    pub(crate) fn primary(&self) -> &Voice {
        &self.voices[0]
    }

    /// Retunes every voice's resampler.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRates`] if the host's sample rate is not representable.
    /// Voices retuned before the failure keep the new rate; the caller is
    /// expected to refuse activation, so the bank is not left in use.
    pub(crate) fn set_sample_rate(&mut self, sample_rate: f32) -> Result<(), InvalidRates> {
        for voice in &mut self.voices {
            voice.set_sample_rate(sample_rate)?;
        }
        Ok(())
    }

    /// Whether voice `index` would be skipped by `render_segment` this block.
    /// Exposed for tests that pin the idle-skip behavior.
    #[cfg(test)]
    pub(crate) fn voice_is_idle(&self, index: usize) -> bool {
        self.voices[index].is_idle()
    }

    /// Largest block one `render_segment` call may cover.
    ///
    /// Every voice shares the same resampler configuration, so voice 0 speaks
    /// for the bank — the same voice `render_segment` reads `clocks_needed` from.
    fn frame_capacity(&self) -> usize {
        self.voices[0].frame_capacity()
    }

    /// Full reset of every voice, as performed by `initialize` and `reset`.
    pub(crate) fn reset_all(&mut self, channel_mode: ChannelMode) {
        for voice in &mut self.voices {
            voice.reset();
            voice.midi_handler.channel_mode = channel_mode;
        }
        self.alloc_counter = 0;
        self.last_active_voice_count = 1;
    }

    pub(crate) fn apply_host_automation(&mut self, controls: HostAutomationControls) {
        for voice in &mut self.voices {
            voice.midi_handler.apply_host_automation(controls);
        }
    }

    pub(crate) fn set_channel_mode(&mut self, channel_mode: ChannelMode) {
        for voice in &mut self.voices {
            voice.midi_handler.channel_mode = channel_mode;
        }
    }

    /// Silences voices that fall outside the voice count after a Max Voices
    /// change, so a reduced polyphony setting does not leave notes hanging.
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

    /// Picks the voice a new note should sound on: the first idle voice,
    /// otherwise the oldest allocation (voice stealing).
    fn select_voice(
        &mut self,
        active_voice_count: usize,
        controls: HostAutomationControls,
    ) -> usize {
        if active_voice_count == 1 {
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
                    .map(|(index, _)| index)
                    .unwrap_or(0)
            });

        self.voices[index].reset_for_allocation();
        // `reset_for_allocation` clears the handler's `last_host_controls`, and with
        // it every host-automated control back to its default (full hardware volume,
        // no vibrato/tremolo, 60 Hz step time, no portamento). `process` only applies
        // host automation once per block, before the event loop, so without this the
        // freshly allocated voice would sound the rest of the block on defaults
        // instead of the automated values — an audible level jump on the attack of
        // every polyphonic note.
        self.voices[index]
            .midi_handler
            .apply_host_automation(controls);
        self.alloc_counter = self.alloc_counter.wrapping_add(1);
        self.voices[index].alloc_id = self.alloc_counter;
        index
    }

    /// Finds the voice a NoteOff should release.
    ///
    /// Broadcasting by pitch would release every overlapping note with the same
    /// MIDI key, so the oldest matching allocation is released first — that
    /// gives repeated notes FIFO pairing.
    fn find_note_off_target(&self, note: u8, active_voice_count: usize) -> Option<usize> {
        self.voices[..active_voice_count]
            .iter()
            .enumerate()
            .filter(|(_, voice)| voice.holds_note(note))
            .min_by_key(|(_, voice)| voice.alloc_id)
            .map(|(index, _)| index)
    }

    /// Routes one MIDI event to the voices it applies to, returning a Program
    /// Change sequence index when the event carried one.
    pub(crate) fn handle_event(
        &mut self,
        event: &NoteEvent<()>,
        seqs: &ActiveSequences,
        active_voice_count: usize,
        controls: HostAutomationControls,
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
            // Pitch bend and CC are channel-wide, so every voice gets them —
            // otherwise a chord would bend one note at a time.
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

    /// Renders `output` in segments, re-evaluating modulation at every 60 Hz
    /// sequence frame boundary so envelope steps land on the exact sample.
    pub(crate) fn render(
        &mut self,
        output: &mut [f32],
        seqs: &ActiveSequences,
        active_voice_count: usize,
        sample_rate: f32,
    ) {
        let mut rendered = 0;

        while rendered < output.len() {
            self.master_gains.clear();
            let mut segment_len = output.len() - rendered;
            let mut any_gated = false;

            for voice in &mut self.voices[..active_voice_count] {
                self.master_gains.push(voice.apply_current_modulation(seqs));
                if voice.gate() {
                    any_gated = true;
                    segment_len =
                        segment_len.min(voice.midi_handler.samples_until_next_frame(sample_rate));
                }
            }

            if !any_gated {
                segment_len = output.len() - rendered;
            }

            // A BlipBuf frame is capped by `BlipBuf::capacity()`, and nothing
            // above bounds the segment by it: an idle bank takes the whole
            // block, and a low Step Time makes even a gated voice's frame
            // interval longer than any realistic buffer. Hosts are free to hand
            // us blocks past that cap (8192 is a normal ASIO size, and offline
            // bounces go higher), so clamp last — this is what kept
            // `clocks_needed` from asserting on the audio thread.
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

    /// Renders one modulation-stable segment: clock every active voice at APU
    /// rate into its BlipBuf, then resample and sum to the mono bus.
    fn render_segment(&mut self, output: &mut [f32], active_voice_count: usize) {
        let sample_count = output.len() as u32;
        if sample_count == 0 {
            return;
        }

        let clocks_needed = self.voices[0].blip.clocks_needed(sample_count);

        // Voice-major, not clock-major: voices never interact within a segment,
        // so this is bit-identical output while keeping one voice's channel
        // state and BlipBuf resident instead of cycling all eight per clock.
        for (voice, gain) in self.voices[..active_voice_count]
            .iter_mut()
            .zip(self.master_gains.iter().copied())
        {
            // An idle voice would clock its chip model `clocks_needed` times
            // (~40 per output sample) only to add a delta of zero every time,
            // so the whole loop is skipped. `end_frame` still runs: it is what
            // advances the buffer's `avail`/`offset`, and skipping it would
            // desynchronize this voice's timeline from the others that did
            // render. See `Voice::is_idle` for why this is bit-identical.
            if !voice.is_idle() {
                for clock in 0..clocks_needed {
                    let channel_output = voice.clock_channel_output();
                    let delta = voice.advance_output(channel_output, gain);
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
            for (out, sample) in output[..samples_read]
                .iter_mut()
                .zip(self.sample_scratch[..samples_read].iter())
            {
                *out += *sample as f32 / 32768.0;
            }
        }
        for sample in output.iter_mut() {
            *sample = (*sample * MASTER_OUTPUT_GAIN).clamp(-1.0, 1.0);
        }
    }
}
