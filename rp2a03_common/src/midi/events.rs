//! `rp2a03_common\src\midi\events.rs`

use super::handler::{MAX_PITCH_SLIDE_RANGE, MidiHandler, RPN_NULL, RPN_PITCH_BEND_SENSITIVITY};
use super::types::{ActiveSequences, Lane};
use nice_plug::prelude::*;
use rp2a03_core::channel::{Channel, PhaseReset};
use rp2a03_core::sequencer::{ArpMode, PitchMode};

const PITCH_BEND_CENTER: i32 = 1 << 13;

const PITCH_BEND_MAX: f32 = ((1u32 << 14) - 1) as f32;

impl MidiHandler {
    pub fn handle_event<S>(
        &mut self,
        event: &NoteEvent<S>,
        channel: &mut dyn Channel,
        seqs: &ActiveSequences,
    ) -> Option<usize> {
        self.sync_channel_mode();

        match event {
            NoteEvent::MidiPitchBend { value, .. } => {
                self.pitch_bend(*value);
                return None;
            }
            NoteEvent::MidiCC { cc, value, .. } => {
                self.control_change(*cc, *value);
                return None;
            }
            _ => {}
        }

        match event {
            NoteEvent::NoteOn { note, velocity, .. } => {
                let vel_u8 = (velocity * 127.0).clamp(0.0, 127.0) as u8;
                self.note_on(*note, vel_u8, channel, seqs);
                None
            }
            NoteEvent::NoteOff { note, .. } => {
                self.note_off(*note, channel, seqs);
                None
            }
            NoteEvent::MidiProgramChange { program, .. } => Some(*program as usize),
            _ => None,
        }
    }

    pub fn pitch_bend(&mut self, value: f32) {
        let raw = (value.clamp(0.0, 1.0) * PITCH_BEND_MAX).round() as i32;
        let bend = (raw - PITCH_BEND_CENTER).clamp(-8192, 8191) as i16;
        self.midi_pitch_bend = Some(bend);
        self.pitch_slide = bend;
    }

    pub fn control_change(&mut self, cc: u8, value: f32) {
        let raw = (value.clamp(0.0, 1.0) * 127.0).round() as u8;

        match cc {
            control_change::REGISTERED_PARAMETER_NUMBER_MSB => self.selected_rpn.0 = raw,
            control_change::REGISTERED_PARAMETER_NUMBER_LSB => self.selected_rpn.1 = raw,
            control_change::DATA_ENTRY_MSB if self.selected_rpn == RPN_PITCH_BEND_SENSITIVITY => {
                let range = raw.min(MAX_PITCH_SLIDE_RANGE);
                self.midi_pitch_bend_range = Some(range);
                self.pitch_slide_range = range;
            }

            // Data Entry LSB is deliberately unhandled: pitch bend sensitivity
            // (the only RPN this synth reacts to) only ever needs semitone
            // precision, so it falls through to the catch-all below.
            control_change::RESET_ALL_CONTROLLERS => {
                self.selected_rpn = RPN_NULL;

                self.midi_pitch_bend = None;
                self.midi_pitch_bend_range = None;
                let host = self.last_host_controls.unwrap_or_default();
                self.pitch_slide = host.pitch_slide.clamp(-8192, 8191);
                self.pitch_slide_range = host.pitch_slide_range.min(MAX_PITCH_SLIDE_RANGE);
            }
            _ => {}
        }
    }

    pub fn note_on(
        &mut self,
        note: u8,
        velocity: u8,
        channel: &mut dyn Channel,
        seqs: &ActiveSequences,
    ) {
        let previous_period = self.macro_period;
        let was_gated = self.gate;

        let reset_phase = match channel.phase_reset() {
            PhaseReset::OnFirstUse => !self.pulse_phase_initialized,
            PhaseReset::Always => true,
            PhaseReset::OnRetrigger => !self.gate,
        };

        self.note_stack.retain(|(n, _)| *n != note);
        self.note_stack.push((note, velocity));

        if seqs.wavesynth.reset_on_note {
            self.restart_fds_wavesynth();
        }

        self.arm_fds_mod_delay(seqs);

        self.trigger_sequences(seqs);
        if !was_gated {
            self.lfo.retrigger();
        }
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

    pub fn note_off(&mut self, note: u8, channel: &mut dyn Channel, seqs: &ActiveSequences) {
        let had_note = self
            .note_stack
            .iter()
            .any(|(held_note, _)| *held_note == note);
        self.note_stack.retain(|(n, _)| *n != note);

        if !had_note {
            return;
        }

        if self.note_stack.is_empty() {
            let has_vol_rel =
                seqs.lane_active(Lane::Vol) && seqs.seq[Lane::Vol].release_point.is_some();
            let has_duty_rel =
                seqs.lane_active(Lane::Duty) && seqs.seq[Lane::Duty].release_point.is_some();

            if !has_vol_rel && !has_duty_rel {
                self.gate = false;
                for player in &mut self.seq_players {
                    player.reset();
                }
            } else {
                for lane in Lane::ALL {
                    if lane == Lane::HiPitch {
                        if self.hipitch_lane_active(seqs) {
                            self.seq_players[lane].release(&seqs.seq[lane]);
                        }
                    } else if seqs.lane_active(lane) {
                        self.seq_players[lane].release(&seqs.seq[lane]);
                    }
                }

                self.clock_sequences_one_frame(seqs);
                self.frame_sample_counter = 0.0;
            }
        } else {
            let previous_period = self.macro_period;
            self.apply_top_note(channel, false);
            self.recalculate_macro_period(seqs);
            let target_period = self.macro_period;
            self.start_portamento(previous_period, target_period);
            self.frame_sample_counter = 0.0;
        }
    }

    fn trigger_sequences(&mut self, seqs: &ActiveSequences) {
        for lane in Lane::ALL {
            let active = if lane == Lane::HiPitch {
                self.hipitch_lane_active(seqs)
            } else {
                seqs.lane_active(lane)
            };
            if active {
                self.seq_players[lane].trigger(&seqs.seq[lane]);
            }
        }
    }

    fn recalculate_macro_period(&mut self, seqs: &ActiveSequences) {
        self.macro_period = self.note_period(0);

        if seqs.lane_active(Lane::Arp) {
            match seqs.seq[Lane::Arp].arp_mode {
                ArpMode::Absolute => {
                    self.macro_period = self.note_period(self.seq_players[Lane::Arp].value());
                }
                ArpMode::Relative => {
                    let step0 = self.seq_players[Lane::Arp].value();
                    self.active_note = (i16::from(self.active_note) + step0).clamp(0, 127) as u8;
                    self.macro_period = self.note_period(0);
                }
            }
        }

        if seqs.lane_active(Lane::Pitch) {
            let pitch_step = i32::from(self.seq_players[Lane::Pitch].value());
            match seqs.seq[Lane::Pitch].pitch_mode {
                PitchMode::Relative => self.macro_period += pitch_step * self.pitch_lane_sign(),

                PitchMode::Absolute => {
                    self.macro_period = self.note_period(0) + pitch_step * self.pitch_lane_sign();
                }
            }
        }

        if self.hipitch_lane_active(seqs) {
            self.macro_period +=
                (i32::from(self.seq_players[Lane::HiPitch].value()) << 4) * self.pitch_lane_sign();
        }

        self.macro_period = self.macro_period.clamp(0, self.max_macro_period());

        self.period_channel = Some(self.channel_mode);
    }
}
