//! rp2a03_niceplug\src\midi.rs
//! Incoming MIDI handling and CC mapping for RP2A03 plugin.

use nice_plug::prelude::*;
use rp2a03_core::apu_pulse::Pulse;
use rp2a03_core::lfo::{SoftwareLfo, DEFAULT_LFO_SPEED};
use rp2a03_core::sequence::{PitchMode, SeqState, Sequence, SequencePlayer};
use rp2a03_core::NTSC_CPU_CLOCK;

/// Converts MIDI note number to frequency in Hz.
pub fn midi_note_to_freq(note: u8) -> f32 {
    440.0 * 2.0f32.powf((note as f32 - 69.0) / 12.0)
}

/// Converts frequency in Hz to NES APU 11-bit timer period value.
pub fn freq_to_period(freq: f32) -> u16 {
    if freq <= 0.0 {
        return 2047;
    }
    let t = (NTSC_CPU_CLOCK as f32 / (16.0 * freq)) - 0.5;
    t.round().clamp(0.0, 2047.0) as u16
}

/// Container holding owned copies of all 5 active sequences and their enable statuses.
#[derive(Debug, Clone)]
pub struct ActiveSequences {
    pub vol_seq: Sequence,
    pub vol_enabled: bool,

    pub arp_seq: Sequence,
    pub arp_enabled: bool,

    pub pitch_seq: Sequence,
    pub pitch_enabled: bool,

    pub hipitch_seq: Sequence,
    pub hipitch_enabled: bool,

    pub duty_seq: Sequence,
    pub duty_enabled: bool,
}

/// Host-automatable controls that mirror the corresponding MIDI CC functions.
///
/// They are synchronized only when their parameter value changes, allowing MIDI
/// CC messages to continue controlling a value until host automation changes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostAutomationControls {
    pub vibrato_depth: u8,
    pub vibrato_speed: u8,
    pub tremolo_depth: u8,
    pub tremolo_speed: u8,
    pub hardware_volume: u8,
    pub fine_pitch: i8,
}

impl Default for HostAutomationControls {
    fn default() -> Self {
        Self {
            vibrato_depth: 0,
            vibrato_speed: DEFAULT_LFO_SPEED,
            tremolo_depth: 0,
            tremolo_speed: DEFAULT_LFO_SPEED,
            hardware_volume: 15,
            fine_pitch: 0,
        }
    }
}

/// Manages incoming MIDI events, note stack, velocity, CCs, and modulation.
#[derive(Debug, Clone)]
pub struct MidiHandler {
    /// Monophonic note stack storing `(note_number, velocity_u8)`
    pub note_stack: Vec<(u8, u8)>,
    /// Semitone transpose offset (default +12 semitones / 1 octave)
    pub octave_offset: i8,
    /// Active gate status
    pub gate: bool,
    /// Current active note velocity (0..127)
    pub current_velocity: u8,
    /// MIDI CC 07 (Volume MSB, 0..127, default 127) — scales APU 15-value volume
    pub cc_volume: u8,
    /// MIDI CC 11 (Expression MSB, 0..127, default 127) — plugin-level gain
    pub cc_expression: u8,
    /// MIDI CC 14 (Pitch offset, -64..+63 semitone cents offset)
    pub fine_pitch: i8,
    /// Host-controlled 4-bit APU volume, applied before MIDI CC 7 and velocity.
    pub hardware_volume: u8,
    /// Last host parameter values applied to this handler.
    last_host_controls: Option<HostAutomationControls>,
    /// Active base MIDI note
    pub active_note: u8,
    /// Software LFO engine from `rp2a03_core`
    pub lfo: SoftwareLfo,
    /// Sample accumulator for 60 Hz frame tick timing
    pub frame_sample_counter: f32,

    /// 5 FamiTracker sequence players
    pub vol_seq_player: SequencePlayer,
    pub arp_seq_player: SequencePlayer,
    pub pitch_seq_player: SequencePlayer,
    pub hipitch_seq_player: SequencePlayer,
    pub duty_seq_player: SequencePlayer,

    /// Accumulated pitch offset for Relative pitch mode
    pub pitch_accum: i16,

    /// Cache of last written control register byte to avoid redundant register writes
    pub prev_ctrl: u8,
    /// Cache of last written timer low byte to avoid redundant register writes
    pub prev_timer_lo: u8,
    /// Cache of last written timer high 3 bits to avoid resetting duty sequencer phase needlessly
    pub prev_timer_hi: u8,
}

impl Default for MidiHandler {
    fn default() -> Self {
        Self {
            note_stack: Vec::with_capacity(16),
            octave_offset: 12,
            gate: false,
            current_velocity: 127,
            cc_volume: 127,
            cc_expression: 127,
            fine_pitch: 0,
            hardware_volume: 15,
            last_host_controls: None,
            active_note: 60,
            lfo: SoftwareLfo::new(),
            frame_sample_counter: 0.0,
            vol_seq_player: SequencePlayer::new(),
            arp_seq_player: SequencePlayer::new(),
            pitch_seq_player: SequencePlayer::new(),
            hipitch_seq_player: SequencePlayer::new(),
            duty_seq_player: SequencePlayer::new(),
            pitch_accum: 0,
            prev_ctrl: 0xFF,
            prev_timer_lo: 0xFF,
            prev_timer_hi: 0xFF,
        }
    }
}

impl MidiHandler {
    /// Create a new `MidiHandler`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset MIDI handler state and held notes.
    pub fn reset(&mut self) {
        self.note_stack.clear();
        self.gate = false;
        self.current_velocity = 127;
        self.cc_volume = 127;
        self.cc_expression = 127;
        self.fine_pitch = 0;
        self.hardware_volume = 15;
        self.last_host_controls = None;
        self.active_note = 60;
        self.lfo.reset();
        self.frame_sample_counter = 0.0;

        self.vol_seq_player.reset();
        self.arp_seq_player.reset();
        self.pitch_seq_player.reset();
        self.hipitch_seq_player.reset();
        self.duty_seq_player.reset();
        self.pitch_accum = 0;

        self.prev_ctrl = 0xFF;
        self.prev_timer_lo = 0xFF;
        self.prev_timer_hi = 0xFF;
    }

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

        self.pitch_accum = 0;
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

    /// Apply top note from monophonic note stack.
    fn apply_top_note(&mut self, pulse: &mut Pulse) {
        if let Some(&(note, velocity)) = self.note_stack.last() {
            self.active_note = note;
            self.current_velocity = velocity;
            self.gate = true;

            pulse.set_enabled(true);
            pulse.write_sweep(0x08);

            // Reset the sentinel so the next update_modulation frame is guaranteed to call
            // write_timer_hi (full phase reset) regardless of whether the new note shares
            // the same high-period bits as the previous note. 0xFF is never a valid 3-bit
            // timer_hi value (valid range 0–7), so this safely signals "note just triggered".
            self.prev_timer_hi = 0xFF;
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

    /// Applies changed host parameter values without continuously overriding MIDI CC values.
    pub fn apply_host_automation(&mut self, controls: HostAutomationControls) {
        let previous = self.last_host_controls.unwrap_or_default();

        if self.last_host_controls.is_none() || controls.vibrato_depth != previous.vibrato_depth {
            self.lfo
                .set_vibrato(controls.vibrato_depth, self.lfo.vibrato_speed);
        }
        if self.last_host_controls.is_none() || controls.vibrato_speed != previous.vibrato_speed {
            self.lfo
                .set_vibrato(self.lfo.vibrato_depth, controls.vibrato_speed);
        }
        if self.last_host_controls.is_none() || controls.tremolo_depth != previous.tremolo_depth {
            self.lfo
                .set_tremolo(controls.tremolo_depth, self.lfo.tremolo_speed);
        }
        if self.last_host_controls.is_none() || controls.tremolo_speed != previous.tremolo_speed {
            self.lfo
                .set_tremolo(self.lfo.tremolo_depth, controls.tremolo_speed);
        }
        if self.last_host_controls.is_none() || controls.hardware_volume != previous.hardware_volume
        {
            self.hardware_volume = controls.hardware_volume.min(15);
        }
        if self.last_host_controls.is_none() || controls.fine_pitch != previous.fine_pitch {
            self.fine_pitch = controls.fine_pitch.clamp(-64, 63);
        }

        self.last_host_controls = Some(controls);
    }

    /// Update sequence playback, LFO modulation, and write updated parameters to APU pulse channel.
    /// Returns master gain multiplier (CC11 Expression).
    pub fn update_modulation(
        &mut self,
        pulse: &mut Pulse,
        seqs: &ActiveSequences,
        sample_rate: f32,
        num_samples: usize,
    ) -> f32 {
        let master_gain = self.cc_expression as f32 / 127.0;

        if !self.gate {
            let duty_val = if seqs.duty_enabled && !seqs.duty_seq.values.is_empty() {
                self.duty_seq_player.value().clamp(0, 3) as u8
            } else {
                0
            };
            let ctrl_byte = (duty_val << 6) | 0x30;
            if ctrl_byte != self.prev_ctrl {
                pulse.write_ctrl(ctrl_byte);
                self.prev_ctrl = ctrl_byte;
            }
            return master_gain;
        }

        // Advance 60 Hz frame ticks for active non-empty sequences
        let samples_per_tick = sample_rate / 60.0;
        self.frame_sample_counter += num_samples as f32;
        while self.frame_sample_counter >= samples_per_tick {
            if seqs.vol_enabled && !seqs.vol_seq.values.is_empty() {
                self.vol_seq_player.clock_tick(&seqs.vol_seq);
            }
            if seqs.arp_enabled && !seqs.arp_seq.values.is_empty() {
                self.arp_seq_player.clock_tick(&seqs.arp_seq);
            }
            let pitch_was_running = seqs.pitch_enabled
                && !seqs.pitch_seq.values.is_empty()
                && self.pitch_seq_player.state == SeqState::Running;
            let pitch_step = if seqs.pitch_enabled && !seqs.pitch_seq.values.is_empty() {
                self.pitch_seq_player.clock_tick(&seqs.pitch_seq)
            } else {
                0
            };

            let hipitch_was_running = seqs.hipitch_enabled
                && !seqs.hipitch_seq.values.is_empty()
                && self.hipitch_seq_player.state == SeqState::Running;
            let hipitch_step = if seqs.hipitch_enabled && !seqs.hipitch_seq.values.is_empty() {
                self.hipitch_seq_player.clock_tick(&seqs.hipitch_seq) << 4
            } else {
                0
            };

            if seqs.pitch_enabled && seqs.pitch_seq.pitch_mode == PitchMode::Relative {
                let p_add = if pitch_was_running { pitch_step } else { 0 };
                let hp_add = if hipitch_was_running { hipitch_step } else { 0 };
                self.pitch_accum = self.pitch_accum.saturating_add(p_add + hp_add);
            }

            if seqs.duty_enabled && !seqs.duty_seq.values.is_empty() {
                self.duty_seq_player.clock_tick(&seqs.duty_seq);
            }

            self.lfo.clock_tick();
            self.frame_sample_counter -= samples_per_tick;
        }

        // 1. Volume Sequence & Tremolo LFO (Fallback to 15 if sequence is empty)
        let vol_val = if seqs.vol_enabled && !seqs.vol_seq.values.is_empty() {
            self.vol_seq_player.value().clamp(0, 15) as u8
        } else {
            15
        };
        let hardware_scaled = (vol_val as u32 * self.hardware_volume as u32 / 15) as u32;
        let cc7_scaled = (hardware_scaled * self.cc_volume as u32 / 127) as u32;
        let vel_scaled_vol = (cc7_scaled * self.current_velocity as u32 / 127) as u8;
        let tremolo_sub = self.lfo.tremolo_volume_delta();
        let apu_vol = vel_scaled_vol.saturating_sub(tremolo_sub).clamp(0, 15);

        // Turn off gate when release tail completes and volume reaches 0
        if self.note_stack.is_empty() && self.vol_seq_player.is_releasing {
            if self.vol_seq_player.state == SeqState::Held && apu_vol == 0 {
                self.gate = false;
            }
        }

        // 2. Duty Sequence (Fallback to 0 [12.5% square] if sequence is empty)
        let duty_val = if seqs.duty_enabled && !seqs.duty_seq.values.is_empty() {
            self.duty_seq_player.value().clamp(0, 3) as u8
        } else {
            0
        };
        let ctrl_byte = (duty_val << 6) | 0x30 | apu_vol;

        if ctrl_byte != self.prev_ctrl {
            pulse.write_ctrl(ctrl_byte);
            self.prev_ctrl = ctrl_byte;
        }

        // 3. Arpeggio Sequence (Fallback to 0 if sequence is empty)
        let arp_semitones = if seqs.arp_enabled && !seqs.arp_seq.values.is_empty() {
            self.arp_seq_player.value()
        } else {
            0
        };
        let effective_note = (self.active_note as i16 + self.octave_offset as i16 + arp_semitones)
            .clamp(0, 127) as u8;

        let base_freq = midi_note_to_freq(effective_note);
        let base_period = freq_to_period(base_freq);

        // 4. Pitch & Hi-Pitch Sequences (Fallback to 0 if sequence is empty)
        let pitch_step = if seqs.pitch_enabled && !seqs.pitch_seq.values.is_empty() {
            self.pitch_seq_player.value()
        } else {
            0
        };
        let hipitch_step = if seqs.hipitch_enabled && !seqs.hipitch_seq.values.is_empty() {
            self.hipitch_seq_player.value() << 4
        } else {
            0
        };
        let total_step = pitch_step + hipitch_step;

        let pitch_delta = if seqs.pitch_enabled && seqs.pitch_seq.pitch_mode == PitchMode::Relative {
            self.pitch_accum
        } else {
            total_step
        };

        let fine_pitch_offset = self.fine_pitch as i16;
        let vibrato_delta = self.lfo.vibrato_pitch_delta();

        // Calculate final timer period (Dn-FamiTracker: Period = BasePeriod + PitchValue)
        // Positive values (+127) increase period -> lower pitch (pitch down, ~328 Hz for ref note 522 Hz)
        // Negative values (-128) decrease period -> higher pitch (pitch up, ~1300 Hz for ref note 522 Hz)
        let final_period = (base_period as i32 + pitch_delta as i32
            - fine_pitch_offset as i32
            - vibrato_delta as i32)
            .clamp(0, 2047) as u16;

        let timer_lo = (final_period & 0xFF) as u8;
        let timer_hi_bits = ((final_period >> 8) & 0x07) as u8;

        if timer_lo != self.prev_timer_lo {
            pulse.write_timer_lo(timer_lo);
            self.prev_timer_lo = timer_lo;
        }

        if timer_hi_bits != self.prev_timer_hi {
            if self.prev_timer_hi == 0xFF {
                // Note attack (sentinel set by apply_top_note): use write_timer_hi so the
                // duty sequencer and envelope reset cleanly for the new note's attack.
                pulse.write_timer_hi(0xF8 | timer_hi_bits);
            } else {
                // Sustain: soft high-period update — skips duty.reset_step() and
                // envelope.restart(), eliminating the click at period byte boundaries
                // (e.g. 0x00FF ↔ 0x0100) during vibrato / LFO modulation.
                // Mirrors Blaarg's smooth vibrato technique used in FamiStudio.
                pulse.set_period_hi_soft(timer_hi_bits);
            }
            self.prev_timer_hi = timer_hi_bits;
        }

        master_gain
    }

    /// Check if gate is currently active.
    pub fn gate(&self) -> bool {
        self.gate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_automation_controls_update_the_matching_synth_controls() {
        let mut handler = MidiHandler::new();
        handler.apply_host_automation(HostAutomationControls {
            vibrato_depth: 7,
            vibrato_speed: 20,
            tremolo_depth: 9,
            tremolo_speed: 30,
            hardware_volume: 11,
            fine_pitch: -24,
        });

        assert_eq!(handler.lfo.vibrato_depth, 7);
        assert_eq!(handler.lfo.vibrato_speed, 20);
        assert_eq!(handler.lfo.tremolo_depth, 9);
        assert_eq!(handler.lfo.tremolo_speed, 30);
        assert_eq!(handler.hardware_volume, 11);
        assert_eq!(handler.fine_pitch, -24);
    }

    #[test]
    fn program_change_returns_its_sequence_index() {
        let mut handler = MidiHandler::new();
        let mut pulse = Pulse::new(rp2a03_core::apu_pulse::PulseChannel::One);
        let sequences = ActiveSequences {
            vol_seq: Sequence::default(),
            vol_enabled: false,
            arp_seq: Sequence::default(),
            arp_enabled: false,
            pitch_seq: Sequence::default(),
            pitch_enabled: false,
            hipitch_seq: Sequence::default(),
            hipitch_enabled: false,
            duty_seq: Sequence::default(),
            duty_enabled: false,
        };

        let index = handler.handle_event(
            &NoteEvent::<()>::MidiProgramChange {
                timing: 0,
                channel: 0,
                program: 42,
            },
            &mut pulse,
            &sequences,
        );

        assert_eq!(index, Some(42));
    }

    #[test]
    fn test_relative_and_absolute_pitch_modes() {
        let mut handler = MidiHandler::new();
        let mut pulse = Pulse::new(rp2a03_core::apu_pulse::PulseChannel::One);

        let mut pitch_seq = Sequence::parse("1 2 3");
        pitch_seq.pitch_mode = PitchMode::Relative;

        let seqs_rel = ActiveSequences {
            vol_seq: Sequence::default(),
            vol_enabled: false,
            arp_seq: Sequence::default(),
            arp_enabled: false,
            pitch_seq: pitch_seq.clone(),
            pitch_enabled: true,
            hipitch_seq: Sequence::default(),
            hipitch_enabled: false,
            duty_seq: Sequence::default(),
            duty_enabled: false,
        };

        // Note trigger (pitch_accum starts at 0)
        handler.note_on(60, 127, &mut pulse, &seqs_rel);
        assert_eq!(handler.pitch_accum, 0);

        // Advance 1 tick (step 0, value 1)
        handler.update_modulation(&mut pulse, &seqs_rel, 60.0, 1);
        assert_eq!(handler.pitch_accum, 1); // 0 + 1

        // Advance 1 tick (step 1, value 2)
        handler.update_modulation(&mut pulse, &seqs_rel, 60.0, 1);
        assert_eq!(handler.pitch_accum, 3); // 1 + 2

        // Advance 1 tick (step 2, value 3)
        handler.update_modulation(&mut pulse, &seqs_rel, 60.0, 1);
        assert_eq!(handler.pitch_accum, 6); // 3 + 3

        // Advance 1 more tick (sequence finished, state is now Held)
        handler.update_modulation(&mut pulse, &seqs_rel, 60.0, 1);
        assert_eq!(handler.pitch_accum, 6); // Must hold constant at 6!

        // Switch to Absolute Mode
        pitch_seq.pitch_mode = PitchMode::Absolute;
        let seqs_abs = ActiveSequences {
            pitch_seq,
            ..seqs_rel
        };

        handler.note_on(60, 127, &mut pulse, &seqs_abs);
        handler.update_modulation(&mut pulse, &seqs_abs, 60.0, 1);
        assert_eq!(handler.pitch_seq_player.value(), 1);
    }

    #[test]
    fn test_famitracker_reference_key_pitch_frequencies() {
        let base_freq = 522.71f32;
        let base_period = freq_to_period(base_freq); // 213

        // Value +127: period = 213 + 127 = 340 -> ~328.04 Hz
        let period_127 = (base_period as i32 + 127) as u16;
        let freq_127 = NTSC_CPU_CLOCK as f32 / (16.0 * (period_127 as f32 + 0.5));
        assert!((freq_127 - 328.04).abs() < 1.0, "Expected ~328.04 Hz, got {}", freq_127);

        // Value -128: period = 213 - 128 = 85 -> ~1300.71 Hz
        let period_minus_128 = (base_period as i32 - 128) as u16;
        let freq_minus_128 = NTSC_CPU_CLOCK as f32 / (16.0 * (period_minus_128 as f32 + 0.5));
        assert!((freq_minus_128 - 1300.71).abs() < 10.0, "Expected ~1300.71 Hz, got {}", freq_minus_128);
    }
}
