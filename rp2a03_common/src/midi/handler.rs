//! `rp2a03_common\src\midi\handler.rs`

use super::fds_bridge::FdsWaveSynth;
use super::types::{
    ActiveSequences, ChannelMode, HostAutomationControls, Lane, SequenceReload,
    freq_to_fds_frequency, freq_to_period, freq_to_s5b_period, freq_to_triangle_period,
    freq_to_vrc6_pulse_period, freq_to_vrc6_saw_period, midi_note_to_freq,
};
use crate::gui::FDS_MOD_TABLE_LEN;
use rp2a03_core::channel::{Channel, PhaseReset};
use rp2a03_core::sequencer::Sequence;
use rp2a03_core::sequencer::{ArpMode, PitchMode, SeqState, SequencePlayer};
use rp2a03_core::software_lfo::SoftwareLfo;

fn reload_player(player: &mut SequencePlayer, enabled: bool, sequence: &Sequence, changed: bool) {
    if !enabled || sequence.values.is_empty() {
        player.reset();
    } else if changed || player.state == SeqState::Disabled {
        player.setup();
    }
}

pub(super) const RPN_NULL: (u8, u8) = (0x7F, 0x7F);

pub(super) const RPN_PITCH_BEND_SENSITIVITY: (u8, u8) = (0, 0);

pub(super) const MAX_PITCH_SLIDE_RANGE: u8 = 24;

// Each bool is an independent piece of playback state (gate, portamento
// on/off, portamento in-flight), not a disguised state machine.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct MidiHandler {
    pub note_stack: Vec<(u8, u8)>,

    pub octave_offset: i8,

    pub gate: bool,

    pub current_velocity: u8,

    pub fine_pitch: i8,

    pub hi_pitch: i8,

    pub pitch_slide: i16,

    pub pitch_slide_range: u8,

    pub(super) midi_pitch_bend: Option<i16>,

    pub(super) midi_pitch_bend_range: Option<u8>,

    pub(super) selected_rpn: (u8, u8),

    pub hardware_volume: u8,

    pub(super) last_host_controls: Option<HostAutomationControls>,

    pub active_note: u8,

    pub lfo: SoftwareLfo,

    pub frame_sample_counter: f64,

    pub step_time_hz: u16,

    pub seq_players: [SequencePlayer; Lane::COUNT],

    pub macro_period: i32,

    pub portamento_target_period: i32,

    pub portamento_active: bool,

    pub portamento_enabled: bool,
    pub portamento_speed: u8,

    pub prev_ctrl: u8,

    ctrl_channel: Option<ChannelMode>,

    pub prev_timer_lo: u8,

    pub prev_timer_hi: u8,

    pub(super) uploaded_fds_wave: Option<[u8; crate::FDS_WAVE_LEN]>,

    pub(super) uploaded_fds_mod_table: Option<[i8; FDS_MOD_TABLE_LEN]>,

    pub(super) fds_wavesynth: FdsWaveSynth,

    pub(super) fds_mod_delay: u8,

    reg_channel: Option<ChannelMode>,

    pub(super) period_channel: Option<ChannelMode>,

    pub(super) pulse_phase_initialized: bool,

    pub channel_mode: ChannelMode,
}

impl Default for MidiHandler {
    fn default() -> Self {
        Self {
            // `note_on` retains-then-pushes by note number, so at most one
            // entry per MIDI note number can ever be held at once — 128 is
            // the real ceiling, not an estimate, so this never grows on the
            // audio thread.
            note_stack: Vec::with_capacity(128),
            octave_offset: 12,
            gate: false,
            current_velocity: 127,
            fine_pitch: 0,
            hi_pitch: 0,
            pitch_slide: 0,
            pitch_slide_range: 2,
            midi_pitch_bend: None,
            midi_pitch_bend_range: None,
            selected_rpn: RPN_NULL,
            hardware_volume: 15,
            last_host_controls: None,
            active_note: 60,
            lfo: SoftwareLfo::new(),
            frame_sample_counter: 0.0,
            step_time_hz: 60,
            seq_players: std::array::from_fn(|_| SequencePlayer::new()),
            macro_period: 0,
            portamento_target_period: 0,
            portamento_active: false,
            portamento_enabled: false,
            portamento_speed: 0,
            prev_ctrl: 0xFF,
            ctrl_channel: None,
            prev_timer_lo: 0xFF,
            prev_timer_hi: 0xFF,
            uploaded_fds_wave: None,
            uploaded_fds_mod_table: None,
            fds_wavesynth: FdsWaveSynth::default(),
            fds_mod_delay: 0,
            reg_channel: None,
            period_channel: None,
            pulse_phase_initialized: false,
            channel_mode: ChannelMode::Pulse,
        }
    }
}

impl MidiHandler {
    pub(super) fn pitch_lane_sign(&self) -> i32 {
        if self.channel_mode == ChannelMode::Fds {
            -1
        } else {
            1
        }
    }

    pub(super) fn hipitch_lane_active(&self, seqs: &ActiveSequences) -> bool {
        seqs.lane_active(Lane::HiPitch) && self.channel_mode != ChannelMode::Fds
    }

    /// The active lane's current value, or `default` when the lane is off.
    pub(super) fn lane_or(&self, seqs: &ActiveSequences, lane: Lane, default: i16) -> i16 {
        if seqs.lane_active(lane) {
            self.seq_players[lane].value()
        } else {
            default
        }
    }

    pub(super) fn max_macro_period(&self) -> i32 {
        match self.channel_mode {
            ChannelMode::Triangle
            | ChannelMode::Vrc6Pulse
            | ChannelMode::Vrc6Saw
            | ChannelMode::S5B
            | ChannelMode::Fds => 0x0FFF,
            _ => 0x07FF,
        }
    }

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

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
        for player in &mut self.seq_players {
            player.reset();
        }
        self.macro_period = 0;
        self.portamento_target_period = 0;
        self.portamento_active = false;
        self.portamento_enabled = false;
        self.portamento_speed = 0;
        self.prev_ctrl = 0xFF;
        self.ctrl_channel = None;
        self.prev_timer_lo = 0xFF;
        self.prev_timer_hi = 0xFF;

        self.uploaded_fds_wave = None;

        self.uploaded_fds_mod_table = None;

        self.fds_wavesynth.restart();

        self.fds_mod_delay = 0;
        self.reg_channel = None;
        self.period_channel = None;
        self.pulse_phase_initialized = false;
    }

    pub(super) fn apply_top_note(&mut self, channel: &mut dyn Channel, reset_phase: bool) {
        if let Some(&(note, velocity)) = self.note_stack.last() {
            self.active_note = note;
            self.current_velocity = velocity;
            self.gate = true;
            channel.set_enabled(true);
            channel.on_top_note(reset_phase);

            if reset_phase && channel.phase_reset() == PhaseReset::OnFirstUse {
                self.pulse_phase_initialized = true;
            }

            if reset_phase {
                self.prev_timer_hi = 0xFF;
            }
        }
    }

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
        if self.last_host_controls.is_none()
            || controls.vibrato_delay != previous.vibrato_delay
            || controls.tremolo_delay != previous.tremolo_delay
            || controls.delay_speed != previous.delay_speed
        {
            self.lfo.set_delay_params(
                controls.vibrato_delay,
                controls.tremolo_delay,
                controls.delay_speed,
            );
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

        let host_moved_slide = self
            .last_host_controls
            .is_some_and(|previous| controls.pitch_slide != previous.pitch_slide);
        if host_moved_slide {
            self.midi_pitch_bend = None;
        }
        if host_moved_slide || self.last_host_controls.is_none() {
            self.pitch_slide = self
                .midi_pitch_bend
                .unwrap_or(controls.pitch_slide)
                .clamp(-8192, 8191);
        }

        let host_moved_range = self
            .last_host_controls
            .is_some_and(|previous| controls.pitch_slide_range != previous.pitch_slide_range);
        if host_moved_range {
            self.midi_pitch_bend_range = None;
        }
        if host_moved_range || self.last_host_controls.is_none() {
            self.pitch_slide_range = self
                .midi_pitch_bend_range
                .unwrap_or(controls.pitch_slide_range)
                .min(MAX_PITCH_SLIDE_RANGE);
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

    pub(super) fn ctrl_needs_write(&mut self, ctrl_byte: u8) -> bool {
        let channel_switched = self.ctrl_channel != Some(self.channel_mode);
        self.ctrl_channel = Some(self.channel_mode);
        if channel_switched || ctrl_byte != self.prev_ctrl {
            self.prev_ctrl = ctrl_byte;
            true
        } else {
            false
        }
    }

    pub fn sync_channel_mode(&mut self) {
        let Some(previous) = self.period_channel.replace(self.channel_mode) else {
            return;
        };
        if previous == self.channel_mode {
            return;
        }

        let previous_base = self.note_period_in(previous, 0);
        let new_base = self.note_period(0);
        if previous_base == new_base {
            return;
        }

        let max = self.max_macro_period();

        if (previous == ChannelMode::Fds) != (self.channel_mode == ChannelMode::Fds) {
            self.macro_period = new_base.clamp(0, max);
            self.portamento_target_period = new_base.clamp(0, max);
            return;
        }

        self.macro_period = (new_base + (self.macro_period - previous_base)).clamp(0, max);
        self.portamento_target_period =
            (new_base + (self.portamento_target_period - previous_base)).clamp(0, max);
    }

    pub fn reload_sequences(&mut self, seqs: &ActiveSequences, reload: SequenceReload) {
        self.fds_wavesynth.restart();

        for lane in Lane::ALL {
            reload_player(
                &mut self.seq_players[lane],
                seqs.enabled[lane],
                &seqs.seq[lane],
                reload[lane],
            );
        }
    }

    pub(super) fn clock_sequences_one_frame(&mut self, seqs: &ActiveSequences) {
        if seqs.lane_active(Lane::Vol) && self.seq_players[Lane::Vol].state == SeqState::Running {
            self.seq_players[Lane::Vol].clock_tick(&seqs.seq[Lane::Vol]);
        }

        if seqs.lane_active(Lane::Arp) {
            match seqs.seq[Lane::Arp].arp_mode {
                ArpMode::Absolute => {
                    if self.seq_players[Lane::Arp].state == SeqState::Running {
                        let arp_step = self.seq_players[Lane::Arp].clock_tick(&seqs.seq[Lane::Arp]);
                        self.macro_period = self.note_period(arp_step);
                    }
                }
                ArpMode::Relative => {
                    if self.seq_players[Lane::Arp].state == SeqState::Running {
                        let arp_step = self.seq_players[Lane::Arp].clock_tick(&seqs.seq[Lane::Arp]);
                        self.active_note =
                            (i16::from(self.active_note) + arp_step).clamp(0, 127) as u8;
                        self.macro_period = self.note_period(0);
                    }
                }
            }
        }

        if seqs.lane_active(Lane::Pitch) && self.seq_players[Lane::Pitch].state == SeqState::Running
        {
            let pitch_step =
                i32::from(self.seq_players[Lane::Pitch].clock_tick(&seqs.seq[Lane::Pitch]));
            match seqs.seq[Lane::Pitch].pitch_mode {
                PitchMode::Relative => self.macro_period += pitch_step * self.pitch_lane_sign(),

                PitchMode::Absolute => {
                    self.macro_period = self.note_period(0) + pitch_step * self.pitch_lane_sign();
                }
            }
            self.macro_period = self.macro_period.clamp(0, self.max_macro_period());
        }

        if self.hipitch_lane_active(seqs)
            && self.seq_players[Lane::HiPitch].state == SeqState::Running
        {
            let hipitch_step =
                i32::from(self.seq_players[Lane::HiPitch].clock_tick(&seqs.seq[Lane::HiPitch]));
            self.macro_period = (self.macro_period
                + ((hipitch_step << 4) * self.pitch_lane_sign()))
            .clamp(0, self.max_macro_period());
        }

        if seqs.lane_active(Lane::Duty) && self.seq_players[Lane::Duty].state == SeqState::Running {
            self.seq_players[Lane::Duty].clock_tick(&seqs.seq[Lane::Duty]);
        }

        self.tick_fds_wavesynth(seqs);

        self.fds_mod_delay = self.fds_mod_delay.saturating_sub(1);

        self.advance_portamento();
    }

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
        let step = ((distance * i32::from(self.portamento_speed)) + 126) / 127;
        let step = step.max(1);
        self.macro_period = if difference > 0 {
            (self.macro_period + step).min(self.portamento_target_period)
        } else {
            (self.macro_period - step).max(self.portamento_target_period)
        };
        self.portamento_active = self.macro_period != self.portamento_target_period;
    }

    pub(super) fn note_period(&self, arp_semitones: i16) -> i32 {
        self.note_period_in(self.channel_mode, arp_semitones)
    }

    fn note_period_in(&self, mode: ChannelMode, arp_semitones: i16) -> i32 {
        let note = (i16::from(self.active_note) + i16::from(self.octave_offset) + arp_semitones)
            .clamp(0, 127) as u8;
        match mode {
            ChannelMode::Triangle => i32::from(freq_to_triangle_period(midi_note_to_freq(note))),
            ChannelMode::Vrc6Pulse => i32::from(freq_to_vrc6_pulse_period(midi_note_to_freq(note))),
            ChannelMode::Vrc6Saw => i32::from(freq_to_vrc6_saw_period(midi_note_to_freq(note))),
            ChannelMode::S5B => i32::from(freq_to_s5b_period(midi_note_to_freq(note))),

            ChannelMode::Fds => i32::from(freq_to_fds_frequency(midi_note_to_freq(note))),
            _ => i32::from(freq_to_period(midi_note_to_freq(note))),
        }
    }

    #[must_use]
    pub fn samples_until_next_frame(&self, sample_rate: f32) -> usize {
        let samples_per_tick = f64::from(sample_rate) / f64::from(self.step_time_hz);
        (samples_per_tick - self.frame_sample_counter)
            .ceil()
            .max(1.0) as usize
    }

    pub fn advance_frame_samples(
        &mut self,
        seqs: &ActiveSequences,
        sample_rate: f32,
        num_samples: usize,
    ) {
        if !self.gate {
            return;
        }

        let samples_per_tick = f64::from(sample_rate) / f64::from(self.step_time_hz);
        self.frame_sample_counter += num_samples as f64;

        // A `>=` phase-accumulator drain, not an equality test: float error
        // can only cost one fewer/extra tick, never hang the loop.
        #[allow(clippy::while_float)]
        while self.frame_sample_counter >= samples_per_tick {
            self.clock_sequences_one_frame(seqs);
            self.lfo.clock_tick();
            self.frame_sample_counter -= samples_per_tick;
        }
    }

    pub(super) fn apply_pitch_registers(&mut self, channel: &mut dyn Channel) {
        let fine_pitch_offset = i32::from(self.fine_pitch);
        let hi_pitch_offset = i32::from(self.hi_pitch) << 4;
        let vibrato_delta = i32::from(self.lfo.vibrato_pitch_delta());

        let bend_semitones =
            f32::from(self.pitch_slide) / 8192.0 * f32::from(self.pitch_slide_range);
        let bend_ratio = 2.0_f32.powf(bend_semitones / 12.0);

        let register = channel.period_register();

        let final_period = if register.inverted {
            let slide_freq = (self.macro_period as f32 * bend_ratio).round() as i32;
            (slide_freq + fine_pitch_offset + hi_pitch_offset + vibrato_delta)
                .clamp(0, self.max_macro_period())
        } else {
            let slide_period =
                (((self.macro_period as f32 + 0.5) / bend_ratio) - 0.5).round() as i32;
            (slide_period - fine_pitch_offset - hi_pitch_offset - vibrato_delta)
                .clamp(0, self.max_macro_period())
        };

        let final_period = channel.fixup_final_period(final_period) as u16;

        let channel_switched = self.reg_channel != Some(self.channel_mode);
        if channel_switched {
            self.reg_channel = Some(self.channel_mode);
        }

        let timer_lo = (final_period & 0xFF) as u8;
        let timer_hi_bits = ((final_period >> 8) & u16::from(register.hi_mask)) as u8;

        if channel_switched || timer_lo != self.prev_timer_lo {
            channel.write_timer_lo(timer_lo);
            self.prev_timer_lo = timer_lo;
        }

        if channel_switched || timer_hi_bits != self.prev_timer_hi {
            if channel_switched || self.prev_timer_hi == 0xFF {
                channel.write_timer_hi(register.hi_control_bits | timer_hi_bits);
            } else {
                channel.set_period_hi_soft(timer_hi_bits);
            }
            self.prev_timer_hi = timer_hi_bits;
        }
    }

    #[must_use]
    pub fn gate(&self) -> bool {
        self.gate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `note_period_in` must route VRC6 Pulse through its own 12-bit-ranged
    /// formula, not the 2A03-shaped `freq_to_period` (11-bit ceiling) the
    /// catch-all arm uses for the actual 2A03 chips.
    #[test]
    fn vrc6_pulse_period_is_not_truncated_to_the_2a03_elevenbit_ceiling() {
        let mut handler = MidiHandler::new();
        handler.channel_mode = ChannelMode::Vrc6Pulse;
        handler.active_note = 20; // well below the ~54.6 Hz / period-2047 crossover

        let period = handler.note_period_in(ChannelMode::Vrc6Pulse, 0);

        assert!(
            period > 2047,
            "period {period} must exceed the 2A03-shaped ceiling for this low a note"
        );
        assert!(period <= 4095, "must stay inside the real 12-bit register");
    }
}
