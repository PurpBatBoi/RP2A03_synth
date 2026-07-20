//! NES 2A03 Square/Pulse channel.
//!
//! Direct translation of the chip logic from MesenCE's SquareChannel.h /
//! ApuEnvelope.h / ApuLengthCounter.h — with all the emulator-only plumbing
//! (NesConsole bus access, mixer delta events, cycle-batched `Run(targetCycle)`
//! catch-up) stripped out and replaced with a simple "clock one NES cycle at a
//! time" model, which is what you actually want when driving this from a
//! plugin's audio callback instead of a CPU bus.
//!
//! NTSC CPU clock: 1_789_773 Hz. The square timer ticks once per CPU cycle.

pub const NTSC_CPU_CLOCK: f64 = 1_789_773.0;

// ---------------------------------------------------------------------
// Length counter
// ---------------------------------------------------------------------

const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

#[derive(Default)]
pub struct LengthCounter {
    enabled: bool,
    halt: bool,
    counter: u8,
    reload_value: u8,
    previous_value: u8,
    new_halt_value: bool,
}

impl LengthCounter {
    /// Called from the 0x4000/0x4004 write (bit 5 = halt/loop flag).
    /// Note: the real hardware doesn't apply the new halt flag immediately —
    /// it's staged and applied in `reload()`, which the frame sequencer calls
    /// right after ticking the length counter. This ordering matters (it's
    /// what MesenCE's comment about "len_reload_timing" tests is about).
    pub fn set_halt_pending(&mut self, halt: bool) {
        self.new_halt_value = halt;
    }

    /// 0x4003/0x4007 write, value = reg >> 3 (5-bit index into the table).
    pub fn load(&mut self, index: u8) {
        if self.enabled {
            self.reload_value = LENGTH_TABLE[index as usize];
            self.previous_value = self.counter;
        }
    }

    /// Call once per CPU cycle-batch, after the frame sequencer's tick for
    /// this step (mirrors NesApu::Run's ordering: reload happens after tick).
    pub fn reload(&mut self) {
        if self.reload_value != 0 {
            if self.counter == self.previous_value {
                self.counter = self.reload_value;
            }
            self.reload_value = 0;
        }
        self.halt = self.new_halt_value;
    }

    pub fn tick(&mut self) {
        if self.counter > 0 && !self.halt {
            self.counter -= 1;
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.counter = 0;
        }
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_halted(&self) -> bool {
        self.halt
    }

    /// True = channel audible (nonzero counter).
    pub fn status(&self) -> bool {
        self.counter > 0
    }
}

// ---------------------------------------------------------------------
// Envelope
// ---------------------------------------------------------------------

#[derive(Default)]
pub struct Envelope {
    constant_volume: bool,
    volume: u8,
    start: bool,
    divider: i8,
    counter: u8,
    pub length_counter: LengthCounter,
}

impl Envelope {
    /// 0x4000/0x4004 write.
    pub fn init(&mut self, reg_value: u8) {
        self.length_counter.set_halt_pending((reg_value & 0x20) != 0);
        self.constant_volume = (reg_value & 0x10) != 0;
        self.volume = reg_value & 0x0F;
    }

    /// Called on 0x4003/0x4007 write — restarts the envelope.
    pub fn restart(&mut self) {
        self.start = true;
    }

    pub fn tick(&mut self) {
        if !self.start {
            self.divider -= 1;
            if self.divider < 0 {
                self.divider = self.volume as i8;
                if self.counter > 0 {
                    self.counter -= 1;
                } else if self.length_counter.is_halted() {
                    self.counter = 15;
                }
            }
        } else {
            self.start = false;
            self.counter = 15;
            self.divider = self.volume as i8;
        }
    }

    pub fn volume(&self) -> u8 {
        if self.length_counter.status() {
            if self.constant_volume {
                self.volume
            } else {
                self.counter
            }
        } else {
            0
        }
    }
}

// ---------------------------------------------------------------------
// Square channel
// ---------------------------------------------------------------------

const DUTY_SEQUENCES: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1],
    [0, 0, 0, 0, 0, 0, 1, 1],
    [0, 0, 0, 0, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 0, 0],
];

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
    }

    pub fn status(&self) -> bool {
        self.envelope.length_counter.status()
    }

    // --- Frame sequencer callbacks ---
    // Call these from your frame sequencer at the standard NTSC timing
    // (see FrameSequencer below): tick_envelope on every quarter frame,
    // tick_length_counter + tick_sweep on every half frame. reload_length_counter
    // must run right after tick_length_counter, per-frame, before the audio
    // channels are clocked (mirrors NesApu::Run's ordering).

    pub fn tick_envelope(&mut self) {
        self.envelope.tick();
    }

    pub fn tick_length_counter(&mut self) {
        self.envelope.length_counter.tick();
    }

    pub fn reload_length_counter(&mut self) {
        self.envelope.length_counter.reload();
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
// Frame sequencer (drives envelope/length/sweep timing)
// ---------------------------------------------------------------------

/// NTSC 4-step frame sequencer cycle counts (from ApuFrameCounter.h,
/// _stepCyclesNtsc[0]). 5-step mode and IRQ generation are omitted here —
/// you don't need APU frame IRQs in a synth plugin, just the quarter/half
/// frame ticks that drive envelope/sweep/length timing.
const STEP_CYCLES_NTSC: [u32; 4] = [7457, 14913, 22371, 29830];

#[derive(Default)]
pub struct FrameSequencer {
    cycle: u32,
    step: usize,
}

pub enum FrameTick {
    None,
    Quarter,
    Half,
}

impl FrameSequencer {
    /// Call once per CPU cycle. Returns which kind of tick (if any) landed
    /// on this cycle so you can dispatch to your channels.
    pub fn clock(&mut self) -> FrameTick {
        self.cycle += 1;
        if self.cycle >= STEP_CYCLES_NTSC[self.step] {
            let tick = match self.step {
                0 | 2 => FrameTick::Quarter,
                1 | 3 => FrameTick::Half,
                _ => unreachable!(),
            };
            self.step += 1;
            if self.step == 4 {
                self.step = 0;
                self.cycle = 0;
            }
            tick
        } else {
            FrameTick::None
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

// ---------------------------------------------------------------------
// Example: wiring it up in a plugin process() loop
// ---------------------------------------------------------------------
//
// struct PulseOscillator {
//     square: SquareChannel,
//     frame_seq: FrameSequencer,
//     cycle_accum: f64,
// }
//
// impl PulseOscillator {
//     fn note_on(&mut self, note: u8, duty: u8, volume: u8) {
//         let period = period_for_frequency(midi_note_to_freq(note));
//         self.square.set_enabled(true);
//         self.square.write_reg0((duty << 6) | 0x10 | volume); // constant volume
//         self.square.write_reg2((period & 0xFF) as u8);
//         self.square.write_reg3((0x1F << 3) | ((period >> 8) as u8 & 0x07)); // max length
//     }
//
//     fn note_off(&mut self) {
//         self.square.set_enabled(false);
//     }
//
//     /// Renders one host audio sample by running the correct number of NES
//     /// CPU cycles and taking the last output as the sample value (naive
//     /// nearest-neighbor — you'll want a proper band-limiting/oversampling
//     /// step here later to kill aliasing on the harsh square edges).
//     fn next_sample(&mut self, host_sample_rate: f64) -> f32 {
//         self.cycle_accum += NTSC_CPU_CLOCK / host_sample_rate;
//         while self.cycle_accum >= 1.0 {
//             self.cycle_accum -= 1.0;
//             match self.frame_seq.clock() {
//                 FrameTick::Quarter => self.square.tick_envelope(),
//                 FrameTick::Half => {
//                     self.square.tick_length_counter();
//                     self.square.tick_sweep();
//                 }
//                 FrameTick::None => {}
//             }
//             self.square.reload_length_counter();
//             self.square.clock();
//         }
//         // output is 0-15, envelope volume is also 0-15 -> normalize to +-1.0
//         (self.square.output as f32 / 15.0) * 2.0 - 1.0
//     }
// }