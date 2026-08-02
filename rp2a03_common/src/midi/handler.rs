//! rp2a03_common\src\midi\handler.rs
//! `MidiHandler`: state container for note stack, sequence players, LFO, and
//! per-tick modulation / register-write logic. NoteEvent ingestion (NoteOn,
//! NoteOff, CC dispatch) lives in `events.rs`.

use super::types::{
    ActiveSequences, ChannelMode, HostAutomationControls, freq_to_period, freq_to_triangle_period,
    midi_note_to_freq,
    midi_note_to_noise_period,
};
use rp2a03_core::apu_noise::Noise;
use rp2a03_core::apu_pulse::Pulse;
use rp2a03_core::apu_triangle::Triangle;
use rp2a03_core::sequencer::{ArpMode, PitchMode, SeqState, SequencePlayer};
use rp2a03_core::software_lfo::SoftwareLfo;

// ─────────────────────────────────────────────
// AnyChannel — zero-cost dispatch shim
// ─────────────────────────────────────────────

/// A thin wrapper that gives uniform access to an APU channel.
/// channel, used inside `MidiHandler` methods so they don't need to be
/// duplicated for each channel type.
pub enum AnyChannel<'a> {
    Pulse(&'a mut Pulse),
    Triangle(&'a mut Triangle),
    Noise(&'a mut Noise),
}

impl<'a> AnyChannel<'a> {
    pub fn set_enabled(&mut self, enabled: bool) {
        match self {
            AnyChannel::Pulse(p) => p.set_enabled(enabled),
            AnyChannel::Triangle(t) => t.set_enabled(enabled),
            AnyChannel::Noise(n) => n.set_enabled(enabled),
        }
    }

    pub fn write_timer_lo(&mut self, val: u8) {
        match self {
            AnyChannel::Pulse(p) => p.write_timer_lo(val),
            AnyChannel::Triangle(t) => t.write_timer_lo(val),
            AnyChannel::Noise(_) => {}
        }
    }

    pub fn write_timer_hi(&mut self, val: u8) {
        match self {
            AnyChannel::Pulse(p) => p.write_timer_hi(val),
            AnyChannel::Triangle(t) => t.write_timer_hi(val),
            AnyChannel::Noise(_) => {}
        }
    }

    pub fn set_period_hi_soft(&mut self, hi_bits: u8) {
        match self {
            AnyChannel::Pulse(p) => p.set_period_hi_soft(hi_bits),
            AnyChannel::Triangle(t) => t.set_period_hi_soft(hi_bits),
            AnyChannel::Noise(_) => {}
        }
    }

    pub fn write_noise_timer(&mut self, period_index: u8, short_mode: bool) {
        if let AnyChannel::Noise(n) = self {
            n.write_timer((period_index & 0x0F) | if short_mode { 0x80 } else { 0 });
        }
    }

    pub fn write_noise_ctrl(&mut self, val: u8) {
        if let AnyChannel::Noise(n) = self {
            n.write_ctrl(val);
        }
    }

    pub fn write_noise_length(&mut self) {
        if let AnyChannel::Noise(n) = self {
            n.write_length(0xF8);
        }
    }
}

// ─────────────────────────────────────────────
// MidiHandler
// ─────────────────────────────────────────────

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
    /// MIDI CC 14 (Pitch offset, -64..+63 semitone cents offset)
    pub fine_pitch: i8,
    /// MIDI CC 15 (Hi-pitch offset, -64..+63 coarse high-period offset)
    pub hi_pitch: i8,
    /// Host-automated MIDI-style pitch bend (-8192..=8191).
    pub pitch_slide: i16,
    /// Host-automated pitch-bend range in semitones (0..=24).
    pub pitch_slide_range: u8,
    /// Host-controlled 4-bit APU volume, applied before MIDI CC 7 and velocity.
    pub hardware_volume: u8,
    /// Last host parameter values applied to this handler.
    last_host_controls: Option<HostAutomationControls>,
    /// Active base MIDI note
    pub active_note: u8,
    /// Software LFO engine from `rp2a03_core`
    pub lfo: SoftwareLfo,
    /// Sample accumulator for frame tick timing.
    ///
    /// This is intentionally fractional so envelope frames land sample-accurately
    /// across host buffer sizes whose length is not an exact multiple of one step period.
    pub frame_sample_counter: f64,
    /// Sequence step tick rate in Hz (default 60 = NTSC frame rate).
    pub step_time_hz: u16,
    /// 5 FamiTracker sequence players
    pub vol_seq_player: SequencePlayer,
    pub arp_seq_player: SequencePlayer,
    pub pitch_seq_player: SequencePlayer,
    pub hipitch_seq_player: SequencePlayer,
    pub duty_seq_player: SequencePlayer,
    /// The working macro period in raw 11-bit APU period units (pulse domain —
    /// the triangle halves it at register-write time). This is the plugin's
    /// equivalent of dnFamiTracker's `CChannelHandler::m_iPeriod`.
    ///
    /// Reset to the triggered note's period on each NoteOn (dn: `RunNote`), then
    /// mutated once per 60 Hz tick by the sequence players in dn's
    /// `CSeqInstHandler::UpdateInstrument` order (arpeggio → pitch → hi-pitch):
    /// arpeggio *replaces* it `SetPeriod(TriggerNote(...))`), relative pitch and
    /// hi-pitch *add* to it, absolute pitch *replaces* it with note period + value.
    /// Every mutation is clamped to 0..=0x7FF exactly where dn's `SetPeriod` calls
    /// `LimitPeriod`, so per-tick overshoot past the rails is discarded like in dn —
    /// i32 is used so the intermediate signed math matches before clamping.
    /// Fine pitch and vibrato are *not* folded in here; they are composed onto
    /// `macro_period` at register-write time (dn: `CalculatePeriod`).
    pub macro_period: i32,
    /// Target period for the current legato transition.
    pub portamento_target_period: i32,
    /// Whether the current note transition is being glided.
    pub portamento_active: bool,
    /// Host-controlled portamento state.
    pub portamento_enabled: bool,
    pub portamento_speed: u8,
    /// Cache of last written control register byte to avoid redundant register writes.
    ///
    /// Only ever compared against bytes written to the Pulse struct — the triangle
    /// path drives its volume through `set_volume` instead — so it needs no
    /// waveform-switch invalidation (unlike `prev_timer_lo` / `prev_timer_hi`,
    /// which are guarded by `reg_channel`).
    pub prev_ctrl: u8,
    /// Cache of last written timer low byte to avoid redundant register writes.
    ///
    /// Handler-level cache over *per-channel* register state: only valid for the
    /// channel recorded in `reg_channel` — after a `ChannelMode` switch the first
    /// write is forced through regardless of this value.
    pub prev_timer_lo: u8,
    /// Cache of last written timer high 3 bits to avoid resetting duty sequencer
    /// phase needlessly. See `prev_timer_lo` / `reg_channel` for cache validity.
    pub prev_timer_hi: u8,
    /// Channel whose registers the `prev_timer_lo` / `prev_timer_hi` caches
    /// currently describe.
    ///
    /// Pulse and Triangle keep independent register state, so after a
    /// `ChannelMode` switch the caches are stale for the new channel and its first
    /// period write must be forced through — otherwise the new channel keeps a
    /// stale timer-low byte (wrong pitch) until a note with a different low byte
    /// re-triggers the write (symptom: play C4 on pulse → switch to triangle →
    /// play C4 sounds wrong; playing D4 then "fixes" it).
    ///
    /// `None` = nothing written yet (first gated write always forced, which also
    /// covers `reset()`, since the APU channel structs are reset alongside).
    reg_channel: Option<ChannelMode>,
    /// Whether the pulse duty sequencer has received its initial attack setup.
    /// This remains true through NoteOff so an adjacent NoteOn does not reset
    /// phase merely because MIDI delivered NoteOff first.
    pub(super) pulse_phase_initialized: bool,
    /// Active channel mode (Pulse / Triangle / Noise).
    pub channel_mode: ChannelMode,
}

impl Default for MidiHandler {
    fn default() -> Self {
        Self {
            note_stack: Vec::with_capacity(16),
            octave_offset: 12,
            gate: false,
            current_velocity: 127,
            fine_pitch: 0,
            hi_pitch: 0,
            pitch_slide: 0,
            pitch_slide_range: 2,
            hardware_volume: 15,
            last_host_controls: None,
            active_note: 60,
            lfo: SoftwareLfo::new(),
            frame_sample_counter: 0.0,
            step_time_hz: 60,
            vol_seq_player: SequencePlayer::new(),
            arp_seq_player: SequencePlayer::new(),
            pitch_seq_player: SequencePlayer::new(),
            hipitch_seq_player: SequencePlayer::new(),
            duty_seq_player: SequencePlayer::new(),
            macro_period: 0,
            portamento_target_period: 0,
            portamento_active: false,
            portamento_enabled: false,
            portamento_speed: 0,
            prev_ctrl: 0xFF,
            prev_timer_lo: 0xFF,
            prev_timer_hi: 0xFF,
            reg_channel: None,
            pulse_phase_initialized: false,
            channel_mode: ChannelMode::Pulse,
        }
    }
}

impl MidiHandler {
    pub(super) fn max_macro_period(&self) -> i32 {
        if self.channel_mode == ChannelMode::Triangle {
            0x0FFF
        } else {
            0x07FF
        }
    }
    /// Create a new `MidiHandler`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset MIDI handler state and held notes.
    pub fn reset(&mut self) {
        self.note_stack.clear();
        self.gate = false;
        self.current_velocity = 127;
        self.fine_pitch = 0;
        self.hi_pitch = 0;
        self.hardware_volume = 15;
        self.last_host_controls = None;
        self.active_note = 60;
        self.lfo.reset();
        self.frame_sample_counter = 0.0;
        self.step_time_hz = 60;
        self.vol_seq_player.reset();
        self.arp_seq_player.reset();
        self.pitch_seq_player.reset();
        self.hipitch_seq_player.reset();
        self.duty_seq_player.reset();
        self.macro_period = 0;
        self.portamento_target_period = 0;
        self.portamento_active = false;
        self.portamento_enabled = false;
        self.portamento_speed = 0;
        self.prev_ctrl = 0xFF;
        self.prev_timer_lo = 0xFF;
        self.prev_timer_hi = 0xFF;
        self.reg_channel = None;
        self.pulse_phase_initialized = false;
        // channel_mode is intentionally NOT reset — it's a persistent host parameter.
    }

    /// Apply top note from monophonic note stack.
    ///
    /// `pub(super)` because it's also called from `note_on` / `note_off` in `events.rs`.
    /// `reset_phase` is true only for a fresh attack. Legato note changes
    /// preserve the pulse duty phase so they do not emulate an unnecessary
    /// `$4003/$4007` write. Noise attacks reset to a deterministic phase so
    /// every MIDI-track note uses the same locked metallic timbre.
    pub(super) fn apply_top_note(&mut self, channel: &mut AnyChannel, reset_phase: bool) {
        if let Some(&(note, velocity)) = self.note_stack.last() {
            self.active_note = note;
            self.current_velocity = velocity;
            self.gate = true;
            channel.set_enabled(true);

            match channel {
                AnyChannel::Pulse(p) => {
                    if reset_phase {
                        p.write_sweep(0x08);
                        self.pulse_phase_initialized = true;
                    }
                }
                AnyChannel::Triangle(t) => t.write_linear_counter(0xFF),
                AnyChannel::Noise(n) => {
                    if reset_phase {
                        n.retrigger();
                    }
                    n.write_length(0xF8);
                }
            }

            if reset_phase {
                // Reset the sentinel so the next update_modulation frame is guaranteed to call
                // write_timer_hi (full phase reset) regardless of whether the new note shares
                // the same high-period bits as the previous note. 0xFF is never a valid 3-bit
                // timer_hi value (valid range 0–7), so this safely signals "note just triggered".
                self.prev_timer_hi = 0xFF;
            }
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
        if self.last_host_controls.is_none() || controls.hi_pitch != previous.hi_pitch {
            self.hi_pitch = controls.hi_pitch.clamp(-64, 63);
        }
        if self.last_host_controls.is_none() || controls.pitch_slide != previous.pitch_slide {
            self.pitch_slide = controls.pitch_slide.clamp(-8192, 8191);
        }
        if self.last_host_controls.is_none()
            || controls.pitch_slide_range != previous.pitch_slide_range
        {
            self.pitch_slide_range = controls.pitch_slide_range.min(24);
        }
        if self.last_host_controls.is_none() || controls.step_time_hz != previous.step_time_hz {
            self.step_time_hz = controls.step_time_hz.clamp(1, 600);
        }
        if self.last_host_controls.is_none()
            || controls.portamento_enabled != previous.portamento_enabled
        {
            self.portamento_enabled = controls.portamento_enabled;
            if !self.portamento_enabled {
                self.macro_period = self.portamento_target_period;
                self.portamento_active = false;
            }
        }
        if self.last_host_controls.is_none()
            || controls.portamento_speed != previous.portamento_speed
        {
            self.portamento_speed = controls.portamento_speed;
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
    /// - Arpeggio computes the period from TriggerNote (semitone lookup) so it
    ///   never accumulates pitch offsets (yes, really; dn
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

        if seqs.arp_enabled && !seqs.arp_seq.values.is_empty() {
            match seqs.arp_seq.arp_mode {
                ArpMode::Absolute => {
                    // dn: SetPeriod(TriggerNote(GetNote() + Value))
                    if self.arp_seq_player.state == SeqState::Running {
                        let arp_step = self.arp_seq_player.clock_tick(&seqs.arp_seq);
                        self.macro_period = self.note_period(arp_step);
                    }
                }
                ArpMode::Relative => {
                    // dn: SetNote(GetNote() + Value); SetPeriod(TriggerNote(GetNote()))
                    // Each step permanently shifts the active base note (accumulating).
                    if self.arp_seq_player.state == SeqState::Running {
                        let arp_step = self.arp_seq_player.clock_tick(&seqs.arp_seq);
                        self.active_note = (self.active_note as i16 + arp_step).clamp(0, 127) as u8;
                        self.macro_period = self.note_period(0);
                    }
                }
            }
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
            self.macro_period = self.macro_period.clamp(0, self.max_macro_period());
        }

        if seqs.hipitch_enabled
            && !seqs.hipitch_seq.values.is_empty()
            && self.hipitch_seq_player.state == SeqState::Running
        {
            let hipitch_step = self.hipitch_seq_player.clock_tick(&seqs.hipitch_seq) as i32;
            self.macro_period =
                (self.macro_period + (hipitch_step << 4)).clamp(0, self.max_macro_period());
        }

        if seqs.duty_enabled
            && !seqs.duty_seq.values.is_empty()
            && self.duty_seq_player.state == SeqState::Running
        {
            self.duty_seq_player.clock_tick(&seqs.duty_seq);
        }

        self.advance_portamento();
    }

    /// Starts a glide from the currently sounding period to a newly triggered
    /// legato target. Periods are interpolated in the same pulse-domain units
    /// used by all existing pitch modulation.
    pub(super) fn start_portamento(&mut self, from: i32, target: i32) {
        self.portamento_target_period = target.clamp(0, self.max_macro_period());
        if self.portamento_enabled && self.portamento_speed > 0 && from != target {
            self.macro_period = from.clamp(0, self.max_macro_period());
            self.portamento_active = true;
        } else {
            self.macro_period = self.portamento_target_period;
            self.portamento_active = false;
        }
    }

    fn advance_portamento(&mut self) {
        if !self.portamento_active {
            return;
        }
        let difference = self.portamento_target_period - self.macro_period;
        if difference == 0 {
            self.portamento_active = false;
            return;
        }
        let distance = difference.unsigned_abs() as i32;
        let step = ((distance * self.portamento_speed as i32) + 126) / 127;
        let step = step.max(1);
        self.macro_period = if difference > 0 {
            (self.macro_period + step).min(self.portamento_target_period)
        } else {
            (self.macro_period - step).max(self.portamento_target_period)
        };
        self.portamento_active = self.macro_period != self.portamento_target_period;
    }

    /// dn `TriggerNote` equivalent: the APU period for the active note with the octave
    /// transposition and an optional arpeggio semitone offset. `midi_note_to_freq` +
    /// `freq_to_period` are this plugin's note lookup table over notes 0..=127.
    ///
    /// Returns a period in the *pulse domain*; the triangle halves it at
    /// register-write time for octave parity (see `apply_pitch_registers`).
    ///
    /// `pub(super)` because it's also called from `note_on` in `events.rs`.
    pub(super) fn note_period(&self, arp_semitones: i16) -> i32 {
        let note = (self.active_note as i16 + self.octave_offset as i16 + arp_semitones)
            .clamp(0, 127) as u8;
        if self.channel_mode == ChannelMode::Triangle {
            freq_to_triangle_period(midi_note_to_freq(note)) as i32
        } else {
            freq_to_period(midi_note_to_freq(note)) as i32
        }
    }

    /// Number of samples that can be rendered before the next envelope tick.
    pub fn samples_until_next_frame(&self, sample_rate: f32) -> usize {
        let samples_per_tick = sample_rate as f64 / self.step_time_hz as f64;
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

        let samples_per_tick = sample_rate as f64 / self.step_time_hz as f64;
        self.frame_sample_counter += num_samples as f64;

        while self.frame_sample_counter >= samples_per_tick {
            self.clock_sequences_one_frame(seqs);
            self.lfo.clock_tick();
            self.frame_sample_counter -= samples_per_tick;
        }
    }

    /// Write the current sequence/LFO state to the active APU channel.
    /// Returns master gain multiplier (CC11 Expression).
    pub fn apply_current_modulation(
        &mut self,
        pulse: &mut Pulse,
        triangle: &mut Triangle,
        seqs: &ActiveSequences,
    ) -> f32 {
        self.apply_current_modulation_with_noise(pulse, triangle, None, seqs)
    }

    pub fn apply_current_modulation_with_noise(
        &mut self,
        pulse: &mut Pulse,
        triangle: &mut Triangle,
        mut noise: Option<&mut Noise>,
        seqs: &ActiveSequences,
    ) -> f32 {
        let master_gain = 1.0;

        match self.channel_mode {
            ChannelMode::Pulse => self.apply_pulse_modulation(pulse, seqs, master_gain),
            ChannelMode::Triangle => self.apply_triangle_modulation(triangle, seqs, master_gain),
            ChannelMode::Noise => {
                if let Some(noise) = noise.as_deref_mut() {
                    self.apply_noise_modulation(noise, seqs);
                }
                master_gain
            }
        }
    }

    fn apply_noise_modulation(&mut self, noise: &mut Noise, seqs: &ActiveSequences) {
        let vol_val = if seqs.vol_enabled && !seqs.vol_seq.values.is_empty() {
            self.vol_seq_player.value().clamp(0, 15) as u32
        } else {
            15
        };
        let hardware_scaled = vol_val * self.hardware_volume as u32 / 15;
        let apu_vol = (hardware_scaled * self.current_velocity as u32 / 127) as u8;
        let short_mode = seqs.duty_enabled
            && !seqs.duty_seq.values.is_empty()
            && self.duty_seq_player.value() != 0;
        noise.write_ctrl(0x30 | apu_vol);
        let note =
            (self.active_note as i16 + self.current_noise_arpeggio(seqs)).clamp(0, 127) as u8;
        let period = midi_note_to_noise_period(note);
        noise.write_timer(period | if short_mode { 0x80 } else { 0 });
    }

    fn current_noise_arpeggio(&self, seqs: &ActiveSequences) -> i16 {
        if seqs.arp_enabled && !seqs.arp_seq.values.is_empty() {
            self.arp_seq_player.value() as i16
        } else {
            0
        }
    }

    fn apply_pulse_modulation(
        &mut self,
        pulse: &mut Pulse,
        seqs: &ActiveSequences,
        master_gain: f32,
    ) -> f32 {
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
        let vel_scaled_vol = (hardware_scaled * self.current_velocity as u32 / 127) as u8;
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

        // 3. Pitch application.
        self.apply_pitch_registers(&mut AnyChannel::Pulse(pulse));

        master_gain
    }

    fn apply_triangle_modulation(
        &mut self,
        triangle: &mut Triangle,
        seqs: &ActiveSequences,
        master_gain: f32,
    ) -> f32 {
        if !self.gate {
            triangle.set_volume(0.0);
            return master_gain;
        }

        // 1. Software Volume — continuous floating-point pipeline for Triangle
        //    to avoid integer step quantization aliasing.
        let vol_val = if seqs.vol_enabled && !seqs.vol_seq.values.is_empty() {
            self.vol_seq_player.value().clamp(0, 15) as f32
        } else {
            15.0
        };

        let hardware_scaled = vol_val * (self.hardware_volume as f32 / 15.0);
        let vel_scaled_vol = hardware_scaled * (self.current_velocity as f32 / 127.0);
        let tremolo_sub = self.lfo.tremolo_volume_delta() as f32;
        let apu_vol = (vel_scaled_vol - tremolo_sub).clamp(0.0, 15.0);

        // Turn off gate when release tail completes and volume reaches 0
        if self.note_stack.is_empty() && self.vol_seq_player.is_releasing {
            if self.vol_seq_player.state == SeqState::End && apu_vol <= 0.0 {
                self.gate = false;
            }
        }

        triangle.set_volume_target(apu_vol);

        // 2. Pitch application (same logic, AnyChannel dispatch).
        self.apply_pitch_registers(&mut AnyChannel::Triangle(triangle));

        master_gain
    }

    /// Shared pitch register write path for both Pulse and Triangle.
    ///
    /// Fine pitch and vibrato compose onto `macro_period` at write time,
    /// like dn's `CalculatePeriod`. Uses `set_period_hi_soft` on sustain
    /// to avoid the click at period-byte boundaries during LFO modulation.
    ///
    /// Two channel adjustments happen here, at the register boundary:
    ///
    /// - Triangle octave parity: the triangle sequencer advances once per CPU cycle
    ///   while the pulse's duty sequencer advances every other cycle
    ///   (f = CPU/32(p+1) vs f = CPU/16(p+1)), so an uncompensated triangle sounds
    ///   one octave lower for the same timer value. The composed period is halved
    ///   for the triangle `p_tri = (p_pulse - 1) / 2`) so both waveforms sound the
    ///   same pitch for the same note and sequence modulation. `macro_period` stays
    ///   in the pulse (dn-parity) domain for all sequence math; the halving
    ///   preserves frequency ratios, so relative modulation (vibrato, pitch
    ///   sequences, fine pitch) keeps its perceived depth on the triangle.
    ///
    /// - Waveform-switch cache invalidation: `prev_timer_lo` / `prev_timer_hi` are
    ///   handler-level caches, but each APU channel keeps its own register state,
    ///   so after a `ChannelMode` switch the first write must go through even if
    ///   the bytes happen to match the cache (which still describes the *other*
    ///   channel's registers). Otherwise the new channel keeps a stale period
    ///   until a note with a different low byte forces a write. A switch forces the
    ///   full attack write `write_timer_hi`, not the soft path) so the new channel
    ///   also gets its sequencer/envelope/linear-counter reset — a mid-note
    ///   triangle switch needs that linear-counter reload to sound at all.
    fn apply_pitch_registers(&mut self, channel: &mut AnyChannel) {
        let fine_pitch_offset = self.fine_pitch as i32;
        let hi_pitch_offset = (self.hi_pitch as i32) << 4;
        let vibrato_delta = self.lfo.vibrato_pitch_delta() as i32;

        // Pitch Slide is deliberately separate from Pitch and Hi-Pitch. Those
        // controls are raw APU-period offsets used by the existing sequence and
        // MIDI-CC behavior; this control is a conventional musical pitch bend.
        // The APU period is proportional to 1/frequency, so apply the bend in
        // frequency space rather than adding a fixed period offset.
        let bend_semitones =
            self.pitch_slide as f32 / 8192.0 * self.pitch_slide_range as f32;
        let bend_ratio = 2.0_f32.powf(bend_semitones / 12.0);
        let slide_period = (((self.macro_period as f32 + 0.5) / bend_ratio) - 0.5).round()
            as i32;

        let final_period = (slide_period - fine_pitch_offset - hi_pitch_offset - vibrato_delta)
            .clamp(0, self.max_macro_period());

        // Triangle octave parity — see fn docs. Halving the composed period keeps
        // `macro_period` in the pulse domain while the triangle plays the
        // matching frequency: CPU/32((p-1)/2 + 1) == CPU/16(p+1).
        let final_period = if self.channel_mode == ChannelMode::Triangle {
            (final_period - 1).max(0) / 2
        } else {
            final_period
        };

        let final_period = final_period as u16;

        // Force the first write after a waveform switch (or reset) through the
        // caches — see fn docs.
        let channel_switched = self.reg_channel != Some(self.channel_mode);
        if channel_switched {
            self.reg_channel = Some(self.channel_mode);
        }

        let timer_lo = (final_period & 0xFF) as u8;
        let timer_hi_bits = ((final_period >> 8) & 0x07) as u8;

        if channel_switched || timer_lo != self.prev_timer_lo {
            channel.write_timer_lo(timer_lo);
            self.prev_timer_lo = timer_lo;
        }

        if channel_switched || timer_hi_bits != self.prev_timer_hi {
            if channel_switched || self.prev_timer_hi == 0xFF {
                // Fresh channel / note attack (sentinel set by apply_top_note):
                // use write_timer_hi so the sequencer and linear counter reset
                // cleanly for the new note's attack.
                channel.write_timer_hi(0xF8 | timer_hi_bits);
            } else {
                // Sustain: soft high-period update — skips duty.reset_step() and
                // envelope.restart()/linear.reload, eliminating the click at period
                // byte boundaries (e.g. 0x00FF ↔ 0x0100) during vibrato / LFO
                // modulation. Mirrors Blaarg's smooth vibrato technique used in
                // FamiStudio.
                channel.set_period_hi_soft(timer_hi_bits);
            }
            self.prev_timer_hi = timer_hi_bits;
        }
    }

    /// Update sequence playback, LFO modulation, and write updated parameters to APU channels.
    /// Returns master gain multiplier (CC11 Expression).
    #[cfg(test)]
    pub fn update_modulation(
        &mut self,
        pulse: &mut Pulse,
        triangle: &mut Triangle,
        seqs: &ActiveSequences,
        sample_rate: f32,
        num_samples: usize,
    ) -> f32 {
        self.advance_frame_samples(seqs, sample_rate, num_samples);
        self.apply_current_modulation(pulse, triangle, seqs)
    }

    /// Check if gate is currently active.
    pub fn gate(&self) -> bool {
        self.gate
    }
}
