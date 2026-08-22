//! `rp2a03_common\src\midi\modulate.rs`
//!
//! Per-chip modulation: turning the active sequencers/LFO/velocity state on
//! `MidiHandler` into register writes. Unlike `Channel` (duty writes, sweep
//! math, S5B's PSG layout, ...), this is real business logic that needs the
//! concrete chip type — `Modulate: Channel` lets `Channels::select` in
//! `rp2a03_niceplug` hand out one trait object that covers both.

use super::handler::MidiHandler;
use super::types::{ActiveSequences, Lane, midi_note_to_noise_period};
use rp2a03_core::apu_noise::Noise;
use rp2a03_core::apu_pulse::Pulse;
use rp2a03_core::apu_triangle::Triangle;
use rp2a03_core::channel::Channel;
use rp2a03_core::fds_audio::Fds;
use rp2a03_core::s5b_audio::Sunsoft;
use rp2a03_core::sequencer::{
    S5B_MODE_NOISE, S5B_MODE_SQUARE, S5B_PERIOD_MASK, SeqState, VolMode, VolMode5B, s5b_duty_index,
};
use rp2a03_core::vrc6_audio::{Vrc6Pulse, Vrc6Saw};

/// Applies one block's worth of modulation to whichever chip `self` is,
/// given the shared MIDI/sequencer state.
pub trait Modulate: Channel {
    fn apply_modulation(&mut self, handler: &mut MidiHandler, seqs: &ActiveSequences);

    /// This chip's quiet form while the gate is off — the register write(s)
    /// that hold it silent between notes.
    ///
    /// Checked, not boilerplate: `Channel::output_delta` gates every chip
    /// purely on its `gate: bool` parameter, so these writes are inaudible
    /// *during* the silent span for every chip. They still matter for two of
    /// the seven — Triangle's `set_volume(0.0)` hard-snaps both `volume` and
    /// `volume_target`, standing in for the slew-to-zero that only runs
    /// while gated (so a release can leave a nonzero residual as the next
    /// attack's start point without it); `Vrc6Saw`'s `write_rate(0)` freezes
    /// the sawtooth accumulator, which otherwise keeps advancing every clock
    /// regardless of gate and lands the next attack at whatever phase it
    /// drifted to. Don't delete this as a blanket "unused write" cleanup
    /// without re-checking those two.
    fn silence(&mut self, handler: &mut MidiHandler, seqs: &ActiveSequences);
}

/// A chip's volume register range: its ceiling value and what an inactive
/// Vol lane reads as instead.
#[derive(Clone, Copy)]
struct VolumeRange {
    ceiling: u8,
    default: u8,
}

impl VolumeRange {
    const fn full(ceiling: u8) -> Self {
        Self {
            ceiling,
            default: ceiling,
        }
    }

    /// Tremolo deltas are in 4-bit volume units; rescale to this chip's range.
    const fn tremolo_steps(self) -> u8 {
        self.ceiling / 15
    }
}

/// Volume for one block: the sequencer lane (or `range.default` when the
/// lane is off), scaled by hardware volume and velocity, minus tremolo.
///
/// `hardware_volume` is clamped to 0..=15 upstream, so the hardware scale can
/// never push the result above `range.ceiling` on its own.
fn scaled_volume(handler: &MidiHandler, seqs: &ActiveSequences, range: VolumeRange) -> u8 {
    let vol_val = handler
        .lane_or(seqs, Lane::Vol, i16::from(range.default))
        .clamp(0, i16::from(range.ceiling)) as u8;

    let hardware_scaled = u32::from(vol_val) * u32::from(handler.hardware_volume) / 15;
    let vel_scaled_vol = (hardware_scaled * u32::from(handler.current_velocity) / 127) as u8;

    vel_scaled_vol
        .saturating_sub(handler.lfo.tremolo_volume_delta() * range.tremolo_steps())
        .min(range.ceiling)
}

/// Pulse and `Vrc6Pulse` both pack the Duty lane into their top ctrl bits
/// alongside fixed bits that differ only by chip; this is that shared shape.
struct DutyCtrl {
    /// `i16` to match `MidiHandler::lane_or`'s return type — this is a clamp
    /// bound for a raw lane value, not a register-width count.
    max: i16,
    shift: u8,
    fixed_bits: u8,
}

impl DutyCtrl {
    fn ctrl_bits(&self, handler: &MidiHandler, seqs: &ActiveSequences) -> u8 {
        let duty_val = handler.lane_or(seqs, Lane::Duty, 0).clamp(0, self.max) as u8;
        (duty_val << self.shift) | self.fixed_bits
    }
}

/// Drops the gate once a released volume sequence has run out at silence, so
/// the voice can go idle.
fn release_gate_off(handler: &mut MidiHandler, silent: bool) {
    if silent
        && handler.note_stack.is_empty()
        && handler.seq_players[Lane::Vol].is_releasing
        && handler.seq_players[Lane::Vol].state == SeqState::End
    {
        handler.gate = false;
    }
}

const PULSE_DUTY_CTRL: DutyCtrl = DutyCtrl {
    max: 3,
    shift: 6,
    fixed_bits: 0x30,
};

impl Modulate for Pulse {
    fn apply_modulation(&mut self, handler: &mut MidiHandler, seqs: &ActiveSequences) {
        let apu_vol = scaled_volume(handler, seqs, VolumeRange::full(15));
        release_gate_off(handler, apu_vol == 0);

        let ctrl_byte = PULSE_DUTY_CTRL.ctrl_bits(handler, seqs) | apu_vol;
        if handler.ctrl_needs_write(ctrl_byte) {
            self.write_ctrl(ctrl_byte);
        }

        handler.apply_pitch_registers(self);
    }

    fn silence(&mut self, handler: &mut MidiHandler, seqs: &ActiveSequences) {
        let ctrl_byte = PULSE_DUTY_CTRL.ctrl_bits(handler, seqs);
        if handler.ctrl_needs_write(ctrl_byte) {
            self.write_ctrl(ctrl_byte);
        }
    }
}

impl Modulate for Triangle {
    fn apply_modulation(&mut self, handler: &mut MidiHandler, seqs: &ActiveSequences) {
        let vol_val = f32::from(handler.lane_or(seqs, Lane::Vol, 15).clamp(0, 15));

        let hardware_scaled = vol_val * (f32::from(handler.hardware_volume) / 15.0);
        let vel_scaled_vol = hardware_scaled * (f32::from(handler.current_velocity) / 127.0);
        let tremolo_sub = f32::from(handler.lfo.tremolo_volume_delta());
        let apu_vol = (vel_scaled_vol - tremolo_sub).clamp(0.0, 15.0);

        release_gate_off(handler, apu_vol <= 0.0);

        self.set_volume_target(apu_vol);

        handler.apply_pitch_registers(self);
    }

    fn silence(&mut self, _handler: &mut MidiHandler, _seqs: &ActiveSequences) {
        self.set_volume(0.0);
    }
}

impl Modulate for Noise {
    fn apply_modulation(&mut self, handler: &mut MidiHandler, seqs: &ActiveSequences) {
        let apu_vol = scaled_volume(handler, seqs, VolumeRange::full(15));
        release_gate_off(handler, apu_vol == 0);

        let short_mode =
            seqs.lane_active(Lane::Duty) && handler.seq_players[Lane::Duty].value() != 0;
        self.write_ctrl(0x30 | apu_vol);
        let arpeggio = handler.lane_or(seqs, Lane::Arp, 0);
        let note = (i16::from(handler.active_note) + arpeggio).clamp(0, 127) as u8;
        let period = midi_note_to_noise_period(note);
        self.write_timer(period | if short_mode { 0x80 } else { 0 });
    }

    fn silence(&mut self, _handler: &mut MidiHandler, _seqs: &ActiveSequences) {
        self.write_ctrl(0x30);
    }
}

const VRC6_PULSE_DUTY_CTRL: DutyCtrl = DutyCtrl {
    max: 7,
    shift: 4,
    fixed_bits: 0,
};

impl Modulate for Vrc6Pulse {
    fn apply_modulation(&mut self, handler: &mut MidiHandler, seqs: &ActiveSequences) {
        let apu_vol = scaled_volume(handler, seqs, VolumeRange::full(15));
        release_gate_off(handler, apu_vol == 0);

        let ctrl_byte = VRC6_PULSE_DUTY_CTRL.ctrl_bits(handler, seqs) | apu_vol;
        if handler.ctrl_needs_write(ctrl_byte) {
            self.write_ctrl(ctrl_byte);
        }

        handler.apply_pitch_registers(self);
    }

    fn silence(&mut self, handler: &mut MidiHandler, seqs: &ActiveSequences) {
        let ctrl_byte = VRC6_PULSE_DUTY_CTRL.ctrl_bits(handler, seqs);
        if handler.ctrl_needs_write(ctrl_byte) {
            self.write_ctrl(ctrl_byte);
        }
    }
}

impl Modulate for Vrc6Saw {
    fn apply_modulation(&mut self, handler: &mut MidiHandler, seqs: &ActiveSequences) {
        let vol_active = seqs.lane_active(Lane::Vol);
        let duty_active = seqs.lane_active(Lane::Duty);

        let steps_64 = seqs.seq[Lane::Vol].vol_mode == VolMode::Steps64;

        let (level, rate_val) = if steps_64 {
            let level = scaled_volume(handler, seqs, VolumeRange::full(63));

            (level, level)
        } else {
            let level = scaled_volume(handler, seqs, VolumeRange::full(15));

            let duty_val = handler.lane_or(seqs, Lane::Duty, 0).clamp(0, 1) as u8;

            (level, (level << 1) | (duty_val << 5))
        };

        // Duty lane release can also end a note on this chip, unlike the
        // other six (whose release-ends-note check lives entirely in
        // `release_gate_off`, keyed on the Vol lane only) — Vrc6Saw's Duty
        // lane doubles as a square/triangle mode select, so its own release
        // can legitimately be what ends the note instead.
        if handler.note_stack.is_empty() {
            if vol_active && handler.seq_players[Lane::Vol].is_releasing {
                if handler.seq_players[Lane::Vol].state == SeqState::End && level == 0 {
                    handler.gate = false;
                }
            } else if !vol_active
                && duty_active
                && handler.seq_players[Lane::Duty].is_releasing
                && handler.seq_players[Lane::Duty].state == SeqState::End
            {
                handler.gate = false;
            }
        }

        self.write_rate(rate_val);

        handler.apply_pitch_registers(self);
    }

    fn silence(&mut self, _handler: &mut MidiHandler, _seqs: &ActiveSequences) {
        self.write_rate(0);
    }
}

impl Modulate for Sunsoft {
    fn apply_modulation(&mut self, handler: &mut MidiHandler, seqs: &ActiveSequences) {
        let steps_32 = seqs.seq[Lane::Vol].vol_mode_5b == VolMode5B::Steps32;
        let vol_ceiling: u8 = if steps_32 { 31 } else { 15 };
        let level = scaled_volume(handler, seqs, VolumeRange::full(vol_ceiling));
        release_gate_off(handler, level == 0);

        let duty_val = handler.lane_or(seqs, Lane::Duty, S5B_MODE_SQUARE);
        let noise_period = (duty_val & S5B_PERIOD_MASK) as u8;
        let square_flag = duty_val & S5B_MODE_SQUARE != 0;
        let noise_flag = duty_val & S5B_MODE_NOISE != 0;

        if noise_flag {
            self.write_noise_period(noise_period ^ S5B_PERIOD_MASK as u8);
        }

        self.write_duty_index(s5b_duty_index(duty_val) as u8);

        self.set_tone_noise_enable(square_flag, noise_flag);

        let vol_index = if steps_32 {
            level
        } else {
            self.psg().linear_volume_index(level, vol_ceiling)
        };
        self.write_volume_level(vol_index);

        handler.apply_pitch_registers(self);
    }

    fn silence(&mut self, _handler: &mut MidiHandler, _seqs: &ActiveSequences) {
        self.set_tone_noise_enable(false, false);
    }
}

impl Modulate for Fds {
    fn apply_modulation(&mut self, handler: &mut MidiHandler, seqs: &ActiveSequences) {
        const FDS_MAX_VOLUME: u8 = 32;
        const FDS_DEFAULT_VOLUME: u8 = 31;

        let level = scaled_volume(
            handler,
            seqs,
            VolumeRange {
                ceiling: FDS_MAX_VOLUME,
                default: FDS_DEFAULT_VOLUME,
            },
        );
        release_gate_off(handler, level == 0);

        self.set_volume(level);
        handler.apply_fds_chip_settings(self, seqs);
        handler.upload_fds_mod_table(self, seqs);
        handler.upload_fds_wave(self, seqs);
        handler.apply_pitch_registers(self);
    }

    fn silence(&mut self, _handler: &mut MidiHandler, _seqs: &ActiveSequences) {
        self.set_volume(0);
    }
}
