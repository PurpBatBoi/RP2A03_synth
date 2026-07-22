//! Incoming MIDI handling and CC mapping for RP2A03 plugin.
//!
//! Encapsulates monophonic note priority, MIDI velocity scaling,
//! fine pitch tuning, CC mappings (CC 01, 02, 03, 04, 07, 11, 14),
//! FamiTracker-style volume/duty sequence playback,
//! and invokes `rp2a03_core::lfo::SoftwareLfo` for APU modulation.

use nice_plug::prelude::*;
use rp2a03_core::apu_pulse::Pulse;
use rp2a03_core::lfo::{SoftwareLfo, DEFAULT_LFO_SPEED};
use rp2a03_core::sequence::{SeqState, Sequence, SequencePlayer};
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
    /// MIDI CC 14 (Fine pitch offset, -64..+63 semitone cents offset)
    pub fine_pitch: i8,
    /// Unmodulated base APU timer period
    pub base_period: u16,
    /// Software LFO engine from `rp2a03_core`
    pub lfo: SoftwareLfo,
    /// Sample accumulator for 60 Hz frame tick timing
    pub frame_sample_counter: f32,

    /// Volume sequence player (advances at 60 Hz)
    pub vol_seq_player: SequencePlayer,
    /// Duty cycle sequence player (advances at 60 Hz)
    pub duty_seq_player: SequencePlayer,

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
            base_period: 0,
            lfo: SoftwareLfo::new(),
            frame_sample_counter: 0.0,
            vol_seq_player: SequencePlayer::new(),
            duty_seq_player: SequencePlayer::new(),
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
        self.base_period = 0;
        self.lfo.reset();
        self.frame_sample_counter = 0.0;
        self.vol_seq_player.reset();
        self.duty_seq_player.reset();
        self.prev_ctrl = 0xFF;
        self.prev_timer_lo = 0xFF;
        self.prev_timer_hi = 0xFF;
    }

    /// Process an incoming MIDI / Note event.
    pub fn handle_event<S>(
        &mut self,
        event: &NoteEvent<S>,
        pulse: &mut Pulse,
        vol_seq: &Sequence,
        duty_seq: &Sequence,
    ) {
        match event {
            NoteEvent::NoteOn { note, velocity, .. } => {
                let vel_u8 = (velocity * 127.0).clamp(0.0, 127.0) as u8;
                self.note_on(*note, vel_u8, pulse, vol_seq, duty_seq);
            }
            NoteEvent::NoteOff { note, .. } => {
                self.note_off(*note, pulse, vol_seq, duty_seq);
            }
            NoteEvent::MidiCC { cc, value, .. } => {
                let value_u8 = (value * 127.0).clamp(0.0, 127.0) as u8;
                self.handle_control_change(*cc, value_u8);
            }
            _ => {}
        }
    }

    /// Handle NoteOn event with monophonic last-note priority.
    pub fn note_on(
        &mut self,
        note: u8,
        velocity: u8,
        pulse: &mut Pulse,
        vol_seq: &Sequence,
        duty_seq: &Sequence,
    ) {
        // Remove existing entry for this note if present to re-push to top
        self.note_stack.retain(|(n, _)| *n != note);
        self.note_stack.push((note, velocity));

        // Trigger sequence players on NoteOn
        self.vol_seq_player.trigger(vol_seq);
        self.duty_seq_player.trigger(duty_seq);

        self.apply_top_note(pulse);
    }

    /// Handle NoteOff event.
    pub fn note_off(
        &mut self,
        note: u8,
        pulse: &mut Pulse,
        vol_seq: &Sequence,
        duty_seq: &Sequence,
    ) {
        self.note_stack.retain(|(n, _)| *n != note);

        if self.note_stack.is_empty() {
            if vol_seq.release_point.is_none() && duty_seq.release_point.is_none() {
                // No release point in envelope: stop sound immediately on key release
                self.gate = false;
                self.vol_seq_player.reset();
                self.duty_seq_player.reset();
            } else {
                // Has release point: trigger release tail
                self.vol_seq_player.release(vol_seq);
                self.duty_seq_player.release(duty_seq);
            }
        } else {
            self.apply_top_note(pulse);
        }
    }

    /// Apply the top note from the monophonic note stack to the APU pulse channel.
    fn apply_top_note(&mut self, pulse: &mut Pulse) {
        if let Some(&(note, velocity)) = self.note_stack.last() {
            let effective_note = (note as i16 + self.octave_offset as i16).clamp(0, 127) as u8;
            let freq = midi_note_to_freq(effective_note);
            let period = freq_to_period(freq);

            self.base_period = period;
            self.current_velocity = velocity;
            self.gate = true;

            let timer_lo = (period & 0xFF) as u8;
            let hi_bits = ((period >> 8) & 0x07) as u8;

            self.prev_timer_lo = timer_lo;
            self.prev_timer_hi = hi_bits;

            pulse.set_enabled(true);
            pulse.write_sweep(0x08);
            pulse.write_timer_lo(timer_lo);
            pulse.write_timer_hi(0xF8 | hi_bits);
        }
    }

    /// Handle MIDI Control Change messages.
    pub fn handle_control_change(&mut self, controller: u8, value: u8) {
        match controller {
            // CC 01: Vibrato Depth (Modulation Wheel)
            1 => {
                let depth = value >> 3; // 0..127 -> 0..15
                let speed = if self.lfo.vibrato_speed == 0 { DEFAULT_LFO_SPEED } else { self.lfo.vibrato_speed };
                self.lfo.set_vibrato(depth, speed);
            }
            // CC 02: Vibrato Speed (Breath Controller)
            2 => {
                let speed = value >> 1; // 0..127 -> 0..63
                self.lfo.set_vibrato(self.lfo.vibrato_depth, speed);
            }
            // CC 03: Tremolo Depth
            3 => {
                let depth = value >> 3; // 0..127 -> 0..15
                let speed = if self.lfo.tremolo_speed == 0 { DEFAULT_LFO_SPEED } else { self.lfo.tremolo_speed };
                self.lfo.set_tremolo(depth, speed);
            }
            // CC 04: Tremolo Speed
            4 => {
                let speed = value >> 1; // 0..127 -> 0..63
                self.lfo.set_tremolo(self.lfo.tremolo_depth, speed);
            }
            // CC 07: Volume MSB — scales APU 15-value volume system
            7 => {
                self.cc_volume = value;
            }
            // CC 11: Expression MSB — plugin-level continuous gain multiplier
            11 => {
                self.cc_expression = value;
            }
            // CC 14: Fine Pitch
            14 => {
                self.fine_pitch = value as i8 - 64;
            }
            _ => {}
        }
    }

    /// Update sequence playback, LFO modulation, and write updated parameters to APU pulse channel.
    /// Returns the overall master gain multiplier (CC11 Expression).
    pub fn update_modulation(
        &mut self,
        pulse: &mut Pulse,
        vol_seq: &Sequence,
        duty_seq: &Sequence,
        sample_rate: f32,
        num_samples: usize,
    ) -> f32 {
        let master_gain = self.cc_expression as f32 / 127.0;

        if !self.gate {
            // When gate is off, write silence with the current duty sequence value
            let duty_val = self.duty_seq_player.value().min(3);
            let ctrl_byte = (duty_val << 6) | 0x30;
            if ctrl_byte != self.prev_ctrl {
                pulse.write_ctrl(ctrl_byte);
                self.prev_ctrl = ctrl_byte;
            }
            return master_gain;
        }

        // Advance 60 Hz frame ticks for sequences and LFO
        let samples_per_tick = sample_rate / 60.0;
        self.frame_sample_counter += num_samples as f32;
        while self.frame_sample_counter >= samples_per_tick {
            self.vol_seq_player.clock_tick(vol_seq);
            self.duty_seq_player.clock_tick(duty_seq);
            self.lfo.clock_tick();
            self.frame_sample_counter -= samples_per_tick;
        }

        // 1. Calculate APU Volume from sequence, CC7, velocity, and Tremolo
        let seq_vol = self.vol_seq_player.value().min(15);
        let cc7_scaled = (seq_vol as u32 * self.cc_volume as u32 / 127) as u32;
        let vel_scaled_vol = (cc7_scaled * self.current_velocity as u32 / 127) as u8;
        let tremolo_sub = self.lfo.tremolo_volume_delta();
        let apu_vol = vel_scaled_vol.saturating_sub(tremolo_sub).clamp(0, 15);

        // Turn off gate when release tail has completed and volume reached zero
        if self.note_stack.is_empty() && self.vol_seq_player.is_releasing {
            if self.vol_seq_player.state == SeqState::Held && apu_vol == 0 {
                self.gate = false;
            }
        }

        // 2. Get duty from sequence
        let duty_val = self.duty_seq_player.value().min(3);
        let ctrl_byte = (duty_val << 6) | 0x30 | apu_vol;

        if ctrl_byte != self.prev_ctrl {
            pulse.write_ctrl(ctrl_byte);
            self.prev_ctrl = ctrl_byte;
        }

        // 3. Calculate Modulated Period (base period - fine pitch - Vibrato pitch delta)
        let fine_pitch_offset = self.fine_pitch as i16;
        let vibrato_delta = self.lfo.vibrato_pitch_delta();

        let final_period = (self.base_period as i32 - fine_pitch_offset as i32 - vibrato_delta as i32)
            .clamp(0, 2047) as u16;

        let timer_lo = (final_period & 0xFF) as u8;
        let timer_hi_bits = ((final_period >> 8) & 0x07) as u8;

        if timer_lo != self.prev_timer_lo {
            pulse.write_timer_lo(timer_lo);
            self.prev_timer_lo = timer_lo;
        }

        if timer_hi_bits != self.prev_timer_hi {
            pulse.write_timer_hi(0xF8 | timer_hi_bits);
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
    use rp2a03_core::apu_pulse::PulseChannel;

    #[test]
    fn test_monophonic_note_stack_last_note_priority() {
        let mut pulse = Pulse::new(PulseChannel::One);
        let mut handler = MidiHandler::new();
        let vol_seq = Sequence::single(15);
        let duty_seq = Sequence::single(2);

        // Play Note A (60)
        handler.note_on(60, 127, &mut pulse, &vol_seq, &duty_seq);
        assert!(handler.gate());
        assert_eq!(handler.note_stack.last(), Some(&(60, 127)));

        // Play Note B (64) while holding Note A
        handler.note_on(64, 100, &mut pulse, &vol_seq, &duty_seq);
        assert_eq!(handler.note_stack.last(), Some(&(64, 100)));

        // Release Note B -> should return to Note A (60)
        handler.note_off(64, &mut pulse, &vol_seq, &duty_seq);
        assert!(handler.gate());
        assert_eq!(handler.note_stack.last(), Some(&(60, 127)));

        // Release Note A -> gate turns false
        handler.note_off(60, &mut pulse, &vol_seq, &duty_seq);
        assert!(!handler.gate());
    }

    #[test]
    fn test_control_change_mappings() {
        let mut handler = MidiHandler::new();

        handler.handle_control_change(1, 120); // Vibrato Depth
        assert_eq!(handler.lfo.vibrato_depth, 15);

        handler.handle_control_change(2, 60); // Vibrato Speed
        assert_eq!(handler.lfo.vibrato_speed, 30);

        handler.handle_control_change(7, 100); // CC Volume
        assert_eq!(handler.cc_volume, 100);

        handler.handle_control_change(14, 64); // Fine pitch center
        assert_eq!(handler.fine_pitch, 0);
    }
}
