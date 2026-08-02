//! rp2a03_common\src\midi\events.rs
//! MIDI/NoteEvent ingestion: NoteOn/NoteOff handling, CC dispatch, and the
//! trigger-time macro-period computation performed on NoteOn.

use super::handler::{AnyChannel, MidiHandler};
use super::types::{ActiveSequences, ChannelMode};
use nice_plug::prelude::*;
use rp2a03_core::apu_pulse::Pulse;
use rp2a03_core::apu_triangle::Triangle;
use rp2a03_core::sequencer::{ArpMode, PitchMode};
use rp2a03_core::software_lfo::DEFAULT_LFO_SPEED;

impl MidiHandler {
    /// Process an incoming MIDI / Note event.
    pub fn handle_event<S>(
        &mut self,
        event: &NoteEvent<S>,
        pulse: &mut Pulse,
        triangle: &mut Triangle,
        seqs: &ActiveSequences,
    ) -> Option<usize> {
        let mut channel = match self.channel_mode {
            ChannelMode::Pulse | ChannelMode::Noise => AnyChannel::Pulse(pulse),
            ChannelMode::Triangle => AnyChannel::Triangle(triangle),
        };

        match event {
            NoteEvent::NoteOn { note, velocity, .. } => {
                let vel_u8 = (velocity * 127.0).clamp(0.0, 127.0) as u8;
                self.note_on(*note, vel_u8, &mut channel, seqs);
            }
            NoteEvent::NoteOff { note, .. } => {
                self.note_off(*note, &mut channel, seqs);
            }
            NoteEvent::MidiCC { cc, value, .. } => {
                let value_u8 = (value * 127.0).clamp(0.0, 127.0) as u8;
                self.handle_control_change(*cc, value_u8);
            }
            NoteEvent::MidiProgramChange { program, .. } => {
                return Some(*program as usize);
            }
            _ => {}
        }
        None
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
            // Retrigger sequences and recalculate macro_period so the sound
            // returns to the held note, just like dnFamiTracker's RunNote does
            // when the top note is released and a lower note is still held.
            let previous_period = self.macro_period;
            self.trigger_sequences(seqs);
            self.apply_top_note(channel, false);
            self.recalculate_macro_period(seqs);
            let target_period = self.macro_period;
            self.start_portamento(previous_period, target_period);
            self.frame_sample_counter = 0.0;
        }
    }

    /// Trigger all enabled sequence players (restart from step 0).
    ///
    /// Called on NoteOn and when restoring a held note after the top note is released.
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

        self.macro_period = self.macro_period.clamp(0, 0x7FF);
    }

    /// Handle MIDI Control Change messages.
    pub fn handle_control_change(&mut self, controller: u8, value: u8) {
        match controller {
            1 => {
                let depth = value >> 3;
                let speed = if self.lfo.vibrato_speed == 0 {
                    DEFAULT_LFO_SPEED
                } else {
                    self.lfo.vibrato_speed
                };
                self.lfo.set_vibrato(depth, speed);
            }
            2 => {
                let speed = value >> 1;
                self.lfo.set_vibrato(self.lfo.vibrato_depth, speed);
            }
            3 => {
                let depth = value >> 3;
                let speed = if self.lfo.tremolo_speed == 0 {
                    DEFAULT_LFO_SPEED
                } else {
                    self.lfo.tremolo_speed
                };
                self.lfo.set_tremolo(depth, speed);
            }
            4 => {
                let speed = value >> 1;
                self.lfo.set_tremolo(self.lfo.tremolo_depth, speed);
            }
            7 => {
                self.cc_volume = value;
            }
            11 => {
                self.cc_expression = value;
            }
            14 => {
                self.fine_pitch = value as i8 - 64;
            }
            15 => {
                self.hi_pitch = value as i8 - 64;
            }
            _ => {}
        }
    }
}
