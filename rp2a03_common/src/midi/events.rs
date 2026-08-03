//! rp2a03_common\src\midi\events.rs
//! MIDI/NoteEvent ingestion: NoteOn/NoteOff handling, CC dispatch, and the
//! trigger-time macro-period computation performed on NoteOn.

use super::handler::{AnyChannel, MidiHandler};
use super::types::{ActiveSequences, ChannelMode};
use nice_plug::prelude::*;
use rp2a03_core::apu_noise::Noise;
use rp2a03_core::apu_pulse::Pulse;
use rp2a03_core::apu_triangle::Triangle;
use rp2a03_core::vrc6_pulse::Vrc6Pulse;
use rp2a03_core::vrc6_saw::Vrc6Saw;
use rp2a03_core::sequencer::{ArpMode, PitchMode};

impl MidiHandler {
    /// Process an incoming MIDI / Note event.
    pub fn handle_event<S>(
        &mut self,
        event: &NoteEvent<S>,
        pulse: &mut Pulse,
        triangle: &mut Triangle,
        seqs: &ActiveSequences,
    ) -> Option<usize> {
        self.handle_event_with_noise(event, pulse, triangle, None, seqs)
    }

    pub fn handle_event_with_noise<S>(
        &mut self,
        event: &NoteEvent<S>,
        pulse: &mut Pulse,
        triangle: &mut Triangle,
        noise: Option<&mut Noise>,
        seqs: &ActiveSequences,
    ) -> Option<usize> {
        self.handle_event_all(event, pulse, triangle, noise, None, None, seqs)
    }

    pub fn handle_event_all<S>(
        &mut self,
        event: &NoteEvent<S>,
        pulse: &mut Pulse,
        triangle: &mut Triangle,
        noise: Option<&mut Noise>,
        vrc6_pulse: Option<&mut Vrc6Pulse>,
        vrc6_saw: Option<&mut Vrc6Saw>,
        seqs: &ActiveSequences,
    ) -> Option<usize> {
        let mode = self.channel_mode;
        let dispatch = |handler: &mut MidiHandler, mut channel: AnyChannel| match event {
            NoteEvent::NoteOn { note, velocity, .. } => {
                let vel_u8 = (velocity * 127.0).clamp(0.0, 127.0) as u8;
                handler.note_on(*note, vel_u8, &mut channel, seqs);
                None
            }
            NoteEvent::NoteOff { note, .. } => {
                handler.note_off(*note, &mut channel, seqs);
                None
            }
            NoteEvent::MidiCC { .. } => None,
            NoteEvent::MidiProgramChange { program, .. } => Some(*program as usize),
            _ => None,
        };

        match (mode, noise, vrc6_pulse, vrc6_saw) {
            (ChannelMode::Pulse, _, _, _) => dispatch(self, AnyChannel::Pulse(pulse)),
            (ChannelMode::Triangle, _, _, _) => dispatch(self, AnyChannel::Triangle(triangle)),
            (ChannelMode::Noise, Some(n), _, _) => dispatch(self, AnyChannel::Noise(n)),
            (ChannelMode::Vrc6Pulse, _, Some(vp), _) => dispatch(self, AnyChannel::Vrc6Pulse(vp)),
            (ChannelMode::Vrc6Pulse, _, None, _) => {
                let mut vp = self.vrc6_pulse.clone();
                let res = dispatch(self, AnyChannel::Vrc6Pulse(&mut vp));
                self.vrc6_pulse = vp;
                res
            }
            (ChannelMode::Vrc6Saw, _, _, Some(vs)) => dispatch(self, AnyChannel::Vrc6Saw(vs)),
            (ChannelMode::Vrc6Saw, _, _, None) => {
                let mut vs = self.vrc6_saw.clone();
                let res = dispatch(self, AnyChannel::Vrc6Saw(&mut vs));
                self.vrc6_saw = vs;
                res
            }
            _ => None,
        }
    }


    /// Handle NoteOn event with monophonic last-note priority.
    pub fn note_on(
        &mut self,
        note: u8,
        velocity: u8,
        channel: &mut AnyChannel,
        seqs: &ActiveSequences,
    ) {
        let previous_period = self.macro_period;
        let was_gated = self.gate;
        // A note arriving while another note is sounding is a legato change:
        // preserve the pulse duty phase and let the normal soft timer path
        // update pitch if needed.
        let reset_phase = match channel {
            AnyChannel::Pulse(_) => !self.pulse_phase_initialized,
            AnyChannel::Triangle(_) => !self.gate,
            // Each MIDI-track noise note starts from the same deterministic
            // phase so its metallic timbre remains locked across notes.
            AnyChannel::Noise(_) => true,
            AnyChannel::Vrc6Pulse(_) => !self.pulse_phase_initialized,
            AnyChannel::Vrc6Saw(_) => !self.gate,
        };

        self.note_stack.retain(|(n, _)| *n != note);
        self.note_stack.push((note, velocity));

        self.trigger_sequences(seqs);
        self.apply_top_note(channel, reset_phase);
        self.recalculate_macro_period(seqs);
        let target_period = self.macro_period;
        if was_gated {
            self.start_portamento(previous_period, target_period);
        } else {
            self.portamento_target_period = target_period;
            self.portamento_active = false;
        }

        self.frame_sample_counter = 0.0;
    }

    /// Handle NoteOff event.
    pub fn note_off(&mut self, note: u8, channel: &mut AnyChannel, seqs: &ActiveSequences) {
        let had_note = self
            .note_stack
            .iter()
            .any(|(held_note, _)| *held_note == note);
        self.note_stack.retain(|(n, _)| *n != note);

        // Polyphonic dispatch broadcasts NoteOff events to every voice. A voice
        // that was not playing this note must remain completely unchanged;
        // otherwise its remaining note would be treated as a restored mono note
        // and its envelopes/LFO would be retriggered.
        if !had_note {
            return;
        }

        if self.note_stack.is_empty() {
            let has_vol_rel = seqs.vol_enabled
                && !seqs.vol_seq.values.is_empty()
                && seqs.vol_seq.release_point.is_some();

            let has_duty_rel = seqs.duty_enabled
                && !seqs.duty_seq.values.is_empty()
                && seqs.duty_seq.release_point.is_some();

            if !has_vol_rel && !has_duty_rel {
                self.gate = false;
                self.vol_seq_player.reset();
                self.duty_seq_player.reset();
                self.arp_seq_player.reset();
                self.pitch_seq_player.reset();
                self.hipitch_seq_player.reset();
            } else {
                if seqs.vol_enabled && !seqs.vol_seq.values.is_empty() {
                    self.vol_seq_player.release(&seqs.vol_seq);
                }
                if seqs.duty_enabled && !seqs.duty_seq.values.is_empty() {
                    self.duty_seq_player.release(&seqs.duty_seq);
                }
                if seqs.arp_enabled && !seqs.arp_seq.values.is_empty() {
                    self.arp_seq_player.release(&seqs.arp_seq);
                }
                if seqs.pitch_enabled && !seqs.pitch_seq.values.is_empty() {
                    self.pitch_seq_player.release(&seqs.pitch_seq);
                }
                if seqs.hipitch_enabled && !seqs.hipitch_seq.values.is_empty() {
                    self.hipitch_seq_player.release(&seqs.hipitch_seq);
                }
                // dn release notes run before CSeqInstHandler::UpdateInstrument in
                // the same engine pass, so the release-point value reaches the APU
                // immediately and then gets a full frame before the next step.
                self.clock_sequences_one_frame(seqs);
                self.frame_sample_counter = 0.0;
            }
        } else {
            // Still notes in the stack — switch back to the previous note.
            // Keep sequence players at their current positions while
            // recalculating macro_period for the restored note. Returning to a
            // held note is a legato change, not a new envelope trigger.
            let previous_period = self.macro_period;
            self.apply_top_note(channel, false);
            self.recalculate_macro_period(seqs);
            let target_period = self.macro_period;
            self.start_portamento(previous_period, target_period);
            self.frame_sample_counter = 0.0;
        }
    }

    /// Trigger all enabled sequence players (restart from step 0).
    ///
    /// Called when a genuinely new note is pressed.
    fn trigger_sequences(&mut self, seqs: &ActiveSequences) {
        if seqs.vol_enabled && !seqs.vol_seq.values.is_empty() {
            self.vol_seq_player.trigger(&seqs.vol_seq);
        }
        if seqs.arp_enabled && !seqs.arp_seq.values.is_empty() {
            self.arp_seq_player.trigger(&seqs.arp_seq);
        }
        if seqs.pitch_enabled && !seqs.pitch_seq.values.is_empty() {
            self.pitch_seq_player.trigger(&seqs.pitch_seq);
        }
        if seqs.hipitch_enabled && !seqs.hipitch_seq.values.is_empty() {
            self.hipitch_seq_player.trigger(&seqs.hipitch_seq);
        }
        if seqs.duty_enabled && !seqs.duty_seq.values.is_empty() {
            self.duty_seq_player.trigger(&seqs.duty_seq);
        }
    }

    /// Recalculate `macro_period` from the current `active_note` and step 0 of all
    /// enabled sequences, following dnFamiTracker's `RunNote` → `UpdateInstrument`
    /// order (arpeggio → pitch → hi-pitch).
    ///
    /// Called on NoteOn and when restoring a held note after the top note is released.
    fn recalculate_macro_period(&mut self, seqs: &ActiveSequences) {
        // dn RunNote: m_iPeriod = TriggerNote(...). Sequence step 0 was already read
        // into the players by trigger() above, so fold it into the working period now.
        self.macro_period = self.note_period(0);

        if seqs.arp_enabled && !seqs.arp_seq.values.is_empty() {
            match seqs.arp_seq.arp_mode {
                ArpMode::Absolute => {
                    // dn: initial period = TriggerNote(BaseNote + step0)
                    self.macro_period = self.note_period(self.arp_seq_player.value());
                }
                ArpMode::Relative => {
                    // dn: SetNote(BaseNote + step0) then SetPeriod(TriggerNote(BaseNote))
                    let step0 = self.arp_seq_player.value();
                    self.active_note = (self.active_note as i16 + step0).clamp(0, 127) as u8;
                    self.macro_period = self.note_period(0);
                }
            }
        }

        if seqs.pitch_enabled && !seqs.pitch_seq.values.is_empty() {
            let pitch_step = self.pitch_seq_player.value() as i32;
            match seqs.pitch_seq.pitch_mode {
                PitchMode::Relative => self.macro_period += pitch_step,
                PitchMode::Absolute => self.macro_period = self.note_period(0) + pitch_step,
            }
        }

        if seqs.hipitch_enabled && !seqs.hipitch_seq.values.is_empty() {
            self.macro_period += (self.hipitch_seq_player.value() as i32) << 4;
        }

        self.macro_period = self.macro_period.clamp(0, self.max_macro_period());
    }

}
