//! nes_square.rs
//! NES 2A03 Square/Pulse channel.
//!
//! Direct translation of the chip logic from MesenCE's SquareChannel.h,
//! adapted for standalone cycle-by-cycle audio synthesis.

use crate::nes_core::{Envelope, NTSC_CPU_CLOCK};

// ---------------------------------------------------------------------
// Square channel
// ---------------------------------------------------------------------

const DUTY_SEQUENCES: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1],
    [0, 0, 0, 0, 0, 0, 1, 1],
    [0, 0, 0, 0, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 0, 0],
];

#[derive(Default, Clone, Debug)]
pub struct SquareChannel {
    pub envelope: Envelope,
    is_channel1: bool,

    // Timer
    timer: u16,
    period: u16, // = real_period * 2 + 1, set via set_period()

    duty: u8,
    duty_pos: u8,

    // Sweep unit
    sweep_enabled: bool,
    sweep_period: u8, // already stored as P+1
    sweep_negate: bool,
    sweep_shift: u8,
    reload_sweep: bool,
    sweep_divider: u8,
    sweep_target_period: u16,
    real_period: u16,

    /// Current instantaneous output, 0-15.
    pub output: u8,

    /// Accumulates output level changes since the last `take_delta()` call.
    /// This is what feeds the BLEP line -- it's set here in `update_output`
    /// (the single place all level changes flow through: duty steps,
    /// register writes, and sweep-driven mute/unmute) rather than only in
    /// `clock()`, so nothing gets missed.
    pending_delta: i16,
}

impl SquareChannel {
    pub fn new(is_channel1: bool) -> Self {
        Self {
            envelope: Envelope::default(),
            is_channel1,
            timer: 0,
            period: 0,
            duty: 0,
            duty_pos: 0,
            sweep_enabled: false,
            sweep_period: 0,
            sweep_negate: false,
            sweep_shift: 0,
            reload_sweep: false,
            sweep_divider: 0,
            sweep_target_period: 0,
            real_period: 0,
            output: 0,
            pending_delta: 0,
        }
    }

    /// Returns and clears any accumulated output-level change since the
    /// last call. Call this once per NES cycle from your audio loop and
    /// feed nonzero results into a `BlipLine::add_delta`.
    pub fn take_delta(&mut self) -> i16 {
        let d = self.pending_delta;
        self.pending_delta = 0;
        d
    }

    fn is_muted(&self) -> bool {
        self.real_period < 8 || (!self.sweep_negate && self.sweep_target_period > 0x7FF)
    }

    fn update_target_period(&mut self) {
        let shift_result = self.real_period >> self.sweep_shift;
        if self.sweep_negate {
            self.sweep_target_period = self.real_period.wrapping_sub(shift_result);
            if self.is_channel1 {
                // Pulse 1's negate subtracts one extra (one's-complement quirk)
                self.sweep_target_period = self.sweep_target_period.wrapping_sub(1);
            }
        } else {
            self.sweep_target_period = self.real_period + shift_result;
        }
    }

    fn set_period(&mut self, new_period: u16) {
        self.real_period = new_period;
        self.period = (self.real_period * 2) + 1;
        self.update_target_period();
    }

    fn update_output(&mut self) {
        let new_output = if self.is_muted() {
            0
        } else {
            DUTY_SEQUENCES[self.duty as usize][self.duty_pos as usize] * self.envelope.volume()
        };
        if new_output != self.output {
            self.pending_delta += new_output as i16 - self.output as i16;
            self.output = new_output;
        }
    }

    // --- Register writes (call these instead of a bus WriteRam) ---

    /// 0x4000 / 0x4004
    pub fn write_reg0(&mut self, value: u8) {
        self.envelope.init(value);
        self.duty = (value & 0xC0) >> 6;
        self.update_output();
    }

    /// 0x4001 / 0x4005 (sweep)
    pub fn write_reg1(&mut self, value: u8) {
        self.sweep_enabled = (value & 0x80) != 0;
        self.sweep_negate = (value & 0x08) != 0;
        self.sweep_period = ((value & 0x70) >> 4) + 1;
        self.sweep_shift = value & 0x07;
        self.update_target_period();
        self.reload_sweep = true;
        self.update_output();
    }

    /// 0x4002 / 0x4006 (period low byte)
    pub fn write_reg2(&mut self, value: u8) {
        self.set_period((self.real_period & 0x0700) | value as u16);
        self.update_output();
    }

    /// 0x4003 / 0x4007 (length counter load + period high bits)
    pub fn write_reg3(&mut self, value: u8) {
        self.envelope.length_counter.load(value >> 3);
        self.set_period((self.real_period & 0xFF) | (((value & 0x07) as u16) << 8));
        self.duty_pos = 0;
        self.envelope.restart();
        self.update_output();
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.envelope.length_counter.set_enabled(enabled);
        self.update_output();
    }

    pub fn status(&self) -> bool {
        self.envelope.length_counter.status()
    }

    // --- Frame sequencer callbacks ---
    // Call these from your frame sequencer at the standard NTSC timing
    // (see FrameSequencer in nes_core.rs): tick_envelope on every quarter frame,
    // tick_length_counter + tick_sweep on every half frame. reload_length_counter
    // must run right after tick_length_counter, per-frame, before the audio
    // channels are clocked.

    pub fn tick_envelope(&mut self) {
        self.envelope.tick();
        self.update_output();
    }

    pub fn tick_length_counter(&mut self) {
        self.envelope.length_counter.tick();
        self.update_output();
    }

    pub fn reload_length_counter(&mut self) {
        self.envelope.length_counter.reload();
        self.update_output();
    }

    pub fn tick_sweep(&mut self) {
        self.sweep_divider = self.sweep_divider.wrapping_sub(1);
        if self.sweep_divider == 0 {
            if self.sweep_shift > 0
                && self.sweep_enabled
                && self.real_period >= 8
                && self.sweep_target_period <= 0x7FF
            {
                let target = self.sweep_target_period;
                self.set_period(target);
            }
            self.sweep_divider = self.sweep_period;
        }

        if self.reload_sweep {
            self.sweep_divider = self.sweep_period;
            self.reload_sweep = false;
        }
        self.update_output();
    }

    /// Clock the timer by one NES CPU cycle. Returns true if the duty
    /// sequencer stepped this cycle (you don't need this for anything other
    /// than debugging — `output` is already updated).
    pub fn clock(&mut self) -> bool {
        if self.timer == 0 {
            self.timer = self.period;
            self.duty_pos = self.duty_pos.wrapping_sub(1) & 0x07;
            self.update_output();
            true
        } else {
            self.timer -= 1;
            false
        }
    }
}

// ---------------------------------------------------------------------
// Helper: MIDI note -> NES period
// ---------------------------------------------------------------------

/// NES pulse frequency = CPU_CLOCK / (16 * (real_period + 1)).
/// Solve for real_period given a target frequency in Hz.
pub fn period_for_frequency(freq_hz: f64) -> u16 {
    let p = (NTSC_CPU_CLOCK / (16.0 * freq_hz)) - 1.0;
    p.round().clamp(0.0, 0x7FF as f64) as u16
}

pub fn midi_note_to_freq(note: u8) -> f64 {
    440.0 * 2f64.powf((note as f64 - 69.0) / 12.0)
}