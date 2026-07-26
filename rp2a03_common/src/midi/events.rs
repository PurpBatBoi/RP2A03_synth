//! rp2a03_common\src\midi\events.rs
//! MIDI/NoteEvent ingestion: NoteOn/NoteOff handling, CC dispatch, and the
//! trigger-time macro-period computation performed on NoteOn.

use nice_plug::prelude::*;
use rp2a03_core::apu_pulse::Pulse;
use rp2a03_core::sequencer::PitchMode;
use rp2a03_core::software_lfo::DEFAULT_LFO_SPEED;

use super::handler::MidiHandler;
use super::types::ActiveSequences;

impl MidiHandler {
    /// Process an incoming MIDI / Note event.
    pub fn handle_event<S>(
        &mut self,
        event: &NoteEvent<S>,
        pulse: &mut Pulse,
        seqs: &ActiveSequences,
    ) -> Option<usize> {
        match event {
            NoteEvent::NoteOn { note, velocity, .. } => {
                let vel_u8 = (velocity * 127.0).clamp(0.0, 127.0) as u8;
                self.note_on(*note, vel_u8, pulse, seqs);
            }
            NoteEvent::NoteOff { note, .. } => {
                self.note_off(*note, pulse, seqs);
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
    pub fn note_on(&mut self, note: u8, velocity: u8, pulse: &mut Pulse, seqs: &ActiveSequences) {
        self.note_stack.retain(|(n, _)| *n != note);
        self.note_stack.push((note, velocity));

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

        self.apply_top_note(pulse);

        // dn RunNote: m_iPeriod = TriggerNote(...). Sequence step 0 was already read
        // into the players by trigger() above (dn processes step 0 in the same engine
        // frame via UpdateInstrument), so fold it into the working period now, in dn's
        // sequence order (arpeggio → pitch → hi-pitch).
        self.macro_period = self.note_period(0);
        if seqs.arp_enabled && !seqs.arp_seq.values.is_empty() {
            self.macro_period = self.note_period(self.arp_seq_player.value());
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

    /// Handle NoteOff event.
    pub fn note_off(&mut self, note: u8, pulse: &mut Pulse, seqs: &ActiveSequences) {
        self.note_stack.retain(|(n, _)| *n != note);

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
            }
        } else {
            self.apply_top_note(pulse);
        }
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
            _ => {}
        }
    }
}