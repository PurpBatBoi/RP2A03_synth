//! rp2a03_common\src\midi\handler.rs
//! `MidiHandler`: state container for note stack, sequence players, LFO, and
//! per-tick modulation / register-write logic. NoteEvent ingestion (NoteOn,
//! NoteOff, CC dispatch) lives in `events.rs`.

use rp2a03_core::apu_pulse::Pulse;
use rp2a03_core::sequencer::{PitchMode, SeqState, SequencePlayer};
use rp2a03_core::software_lfo::SoftwareLfo;

use super::types::{freq_to_period, midi_note_to_freq, ActiveSequences, HostAutomationControls};

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
    /// Sample accumulator for 60 Hz frame tick timing.
    ///
    /// This is intentionally fractional so envelope frames land sample-accurately
    /// across host buffer sizes whose length is not an exact multiple of 1/60 s.
    pub frame_sample_counter: f64,

    /// 5 FamiTracker sequence players
    pub vol_seq_player: SequencePlayer,
    pub arp_seq_player: SequencePlayer,
    pub pitch_seq_player: SequencePlayer,
    pub hipitch_seq_player: SequencePlayer,
    pub duty_seq_player: SequencePlayer,

    /// The working macro period in raw 11-bit APU period units — the plugin's
    /// equivalent of dnFamiTracker's `CChannelHandler::m_iPeriod`.
    ///
    /// Reset to the triggered note's period on each NoteOn (dn: `RunNote`), then
    /// mutated once per 60 Hz tick by the sequence players in dn's
    /// `CSeqInstHandler::UpdateInstrument` order (arpeggio → pitch → hi-pitch):
    /// arpeggio *replaces* it (`SetPeriod(TriggerNote(...))`), relative pitch and
    /// hi-pitch *add* to it, absolute pitch *replaces* it with note period + value.
    /// Every mutation is clamped to 0..=0x7FF exactly where dn's `SetPeriod` calls
    /// `LimitPeriod`, so per-tick overshoot past the rails is discarded like in dn —
    /// i32 is used so the intermediate signed math matches before clamping.
    /// Fine pitch and vibrato are *not* folded in here; they are composed onto
    /// `macro_period` at register-write time (dn: `CalculatePeriod`).
    pub macro_period: i32,

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
            macro_period: 0,
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
        self.macro_period = 0;

        self.prev_ctrl = 0xFF;
        self.prev_timer_lo = 0xFF;
        self.prev_timer_hi = 0xFF;
    }

    /// Apply top note from monophonic note stack.
    ///
    /// `pub(super)` because it's also called from `note_on`/`note_off` in `events.rs`.
    pub(super) fn apply_top_note(&mut self, pulse: &mut Pulse) {
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

    /// Ticks all sequence players forward by one 60 Hz engine frame and updates the
    /// working macro period, following dnFamiTracker's
    /// `CSeqInstHandler::UpdateInstrument` ordering (arpeggio → pitch → hi-pitch) and
    /// `SetPeriod` semantics:
    ///
    /// - A sequence only advances while `SeqState::Running` (dn: END/HALT process
    ///   nothing more, so the working period simply persists).
    /// - Arpeggio (absolute setting) *replaces* the working period with the arp note's
    ///   period every tick — wiping any accumulated pitch offsets (yes, really; dn
    ///   does this via `SetPeriod(TriggerNote(GetNote() + Value))`).
    /// - Relative pitch *adds* its step; absolute pitch *replaces* with the base note
    ///   period + step (dn: `SetPeriod(GetPeriod() + Value)` vs
    ///   `SetPeriod(TriggerNote(GetNote()) + Value)`).
    /// - Hi-pitch *adds* its step shifted left by 4 and is always accumulating,
    ///   regardless of the pitch mode setting (dn: `SetPeriod(GetPeriod() + (Value << 4))`).
    pub(super) fn clock_sequences_one_frame(&mut self, seqs: &ActiveSequences) {
        if seqs.vol_enabled
            && !seqs.vol_seq.values.is_empty()
            && self.vol_seq_player.state == SeqState::Running
        {
            self.vol_seq_player.clock_tick(&seqs.vol_seq);
        }

        if seqs.arp_enabled
            && !seqs.arp_seq.values.is_empty()
            && self.arp_seq_player.state == SeqState::Running
        {
            let arp_step = self.arp_seq_player.clock_tick(&seqs.arp_seq);
            self.macro_period = self.note_period(arp_step);
        }

        if seqs.pitch_enabled
            && !seqs.pitch_seq.values.is_empty()
            && self.pitch_seq_player.state == SeqState::Running
        {
            let pitch_step = self.pitch_seq_player.clock_tick(&seqs.pitch_seq) as i32;
            match seqs.pitch_seq.pitch_mode {
                PitchMode::Relative => self.macro_period += pitch_step,
                PitchMode::Absolute => self.macro_period = self.note_period(0) + pitch_step,
            }
            self.macro_period = self.macro_period.clamp(0, 0x7FF);
        }

        if seqs.hipitch_enabled
            && !seqs.hipitch_seq.values.is_empty()
            && self.hipitch_seq_player.state == SeqState::Running
        {
            let hipitch_step = self.hipitch_seq_player.clock_tick(&seqs.hipitch_seq) as i32;
            self.macro_period = (self.macro_period + (hipitch_step << 4)).clamp(0, 0x7FF);
        }

        if seqs.duty_enabled
            && !seqs.duty_seq.values.is_empty()
            && self.duty_seq_player.state == SeqState::Running
        {
            self.duty_seq_player.clock_tick(&seqs.duty_seq);
        }
    }

    /// dn `TriggerNote` equivalent: the APU period for the active note with the octave
    /// transposition and an optional arpeggio semitone offset. `midi_note_to_freq` +
    /// `freq_to_period` are this plugin's note lookup table over notes 0..=127.
    ///
    /// `pub(super)` because it's also called from `note_on` in `events.rs`.
    pub(super) fn note_period(&self, arp_semitones: i16) -> i32 {
        let note = (self.active_note as i16 + self.octave_offset as i16 + arp_semitones)
            .clamp(0, 127) as u8;
        freq_to_period(midi_note_to_freq(note)) as i32
    }

    /// Number of samples that can be rendered before the next 60 Hz envelope tick.
    pub fn samples_until_next_frame(&self, sample_rate: f32) -> usize {
        let samples_per_tick = sample_rate as f64 / 60.0;
        (samples_per_tick - self.frame_sample_counter)
            .ceil()
            .max(1.0) as usize
    }

    /// Account for samples that have just been rendered, advancing envelopes at the
    /// exact sample boundary where each 60 Hz frame elapses.
    pub fn advance_frame_samples(
        &mut self,
        seqs: &ActiveSequences,
        sample_rate: f32,
        num_samples: usize,
    ) {
        if !self.gate {
            return;
        }

        let samples_per_tick = sample_rate as f64 / 60.0;
        self.frame_sample_counter += num_samples as f64;
        while self.frame_sample_counter >= samples_per_tick {
            self.clock_sequences_one_frame(seqs);
            self.lfo.clock_tick();
            self.frame_sample_counter -= samples_per_tick;
        }
    }

    /// Write the current sequence/LFO state to the APU pulse channel.
    /// Returns master gain multiplier (CC11 Expression).
    pub fn apply_current_modulation(&mut self, pulse: &mut Pulse, seqs: &ActiveSequences) -> f32 {
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
            if self.vol_seq_player.state == SeqState::End && apu_vol == 0 {
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

        // 3. Pitch application. The macro period already carries the arpeggio /
        // pitch / hi-pitch sequences (folded per engine tick above). Fine pitch and
        // vibrato compose onto it at write time, like dn's CalculatePeriod; both are
        // "up = positive" offsets, so they subtract in period space (higher period
        // = lower pitch on the 2A03).
        let fine_pitch_offset = self.fine_pitch as i32;
        let vibrato_delta = self.lfo.vibrato_pitch_delta() as i32;

        let final_period =
            (self.macro_period - fine_pitch_offset - vibrato_delta).clamp(0, 0x7FF) as u16;

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

    /// Update sequence playback, LFO modulation, and write updated parameters to APU pulse channel.
    /// Returns master gain multiplier (CC11 Expression).
    #[cfg(test)]
    pub fn update_modulation(
        &mut self,
        pulse: &mut Pulse,
        seqs: &ActiveSequences,
        sample_rate: f32,
        num_samples: usize,
    ) -> f32 {
        self.advance_frame_samples(seqs, sample_rate, num_samples);
        self.apply_current_modulation(pulse, seqs)
    }

    /// Check if gate is currently active.
    pub fn gate(&self) -> bool {
        self.gate
    }
}
