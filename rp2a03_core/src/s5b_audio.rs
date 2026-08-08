//! rp2a03_core\src\s5b_audio.rs
//!
//! Adapted from emu2149 (YM2149/AY-3-8910 emulator) by Mitsutaka Okazaki,
//! with envelope/noise fixes by alexmush:
//! <https://github.com/digital-sound-antiques/emu2149>
//!
//! The register-select/write latch and channel-enable masking are adapted
//! from Nes_Sunsoft (Sunsoft 5B wrapper for Nes_Snd_Emu / FamiStudio) by
//! @NesBleuBleu.
//!
//! Sunsoft 5B Expansion Audio implementation.
//!
//! The Sunsoft 5B is a Konami-competitor expansion chip (used by e.g.
//! Gimmick!, Batman: Return of the Joker) built around a YM2149 PSG: three
//! square-wave tone channels, a shared 17-bit LFSR noise generator, and one
//! hardware envelope generator that any channel can opt into in place of its
//! own 5-bit volume register.
//!
//! Unlike the 2A03/VRC6 channels in this crate, the three tone channels are
//! not independent structs — they share one noise generator and one
//! envelope generator, so the chip is modeled as a single [`Sunsoft`] unit
//! clocked once per CPU cycle, exposing per-channel output for the mixer.
//!
//! See: <https://www.nesdev.org/wiki/Sunsoft_5B_audio>

// ─────────────────────────────────────────────
// Volume Tables
// ─────────────────────────────────────────────

/// Selects which chip's logarithmic volume table the PSG uses. The two
/// chips' 5-bit volume registers map to different analog curves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VolumeMode {
    /// YM2149 32-step curve. This is what real Sunsoft 5B hardware uses.
    #[default]
    Ym2149,
    /// AY-3-8910 16-step curve (each step duplicated to fill 32 entries),
    /// provided for compatibility with content authored against that chip.
    Ay8910,
}

impl VolumeMode {
    fn table(self) -> &'static [u32; 32] {
        match self {
            VolumeMode::Ym2149 => &VOLTBL_YM2149,
            VolumeMode::Ay8910 => &VOLTBL_AY8910,
        }
    }
}

/// YM2149 - 32 steps. Source: emu2149 (shifted forward by 1 by alexmush).
const VOLTBL_YM2149: [u32; 32] = [
    0x00, 0x00, 0x01, 0x01, 0x02, 0x02, 0x03, 0x03, 0x04, 0x05, 0x06, 0x07, 0x09, 0x0B, 0x0D,
    0x0F, 0x12, 0x16, 0x1A, 0x1F, 0x25, 0x2D, 0x35, 0x3F, 0x4C, 0x5A, 0x6A, 0x7F, 0x97, 0xB4,
    0xD6, 0xFF,
];

/// AY-3-8910 - 16 steps, each duplicated. Source: emu2149.
const VOLTBL_AY8910: [u32; 32] = [
    0x00, 0x00, 0x03, 0x03, 0x04, 0x04, 0x06, 0x06, 0x09, 0x09, 0x0D, 0x0D, 0x12, 0x12, 0x1D,
    0x1D, 0x22, 0x22, 0x37, 0x37, 0x4D, 0x4D, 0x62, 0x62, 0x82, 0x82, 0xA6, 0xA6, 0xD0, 0xD0,
    0xFF, 0xFF,
];

/// Per-register write masks ($0..$F), matching what real PSG hardware
/// ignores on write. Indexed by register number.
const REG_MASK: [u8; 16] = [
    0xff, 0x0f, 0xff, 0x0f, 0xff, 0x0f, 0x1f, 0x3f, 0x1f, 0x1f, 0x1f, 0xff, 0xff, 0x0f, 0xff,
    0xff,
];

// ─────────────────────────────────────────────
// Envelope Generator
// ─────────────────────────────────────────────

/// The PSG's single shared hardware envelope generator ($0B/$0C period,
/// $0D shape). Any of the three tone channels can select it in place of
/// its own constant volume by setting bit 5 of its volume register.
#[derive(Debug, Clone, Default)]
struct EnvelopeGen {
    /// 6-bit ramp position; only the low 5 bits (0..=0x1F) are ever used to
    /// index the volume table — the generator forces the pointer to 0 or
    /// 0x1F immediately after every carry/borrow, matching hardware.
    ptr: u8,
    /// Ramp direction: true counts up, false counts down.
    face: bool,
    period: u16,
    count: u32,
    pause: bool,
    continue_: bool,
    attack: bool,
    alternate: bool,
    hold: bool,
}

impl EnvelopeGen {
    /// $0D Envelope shape.
    ///   D3: Continue
    ///   D2: Attack
    ///   D1: Alternate
    ///   D0: Hold
    /// Writing this register fully restarts the envelope, matching real
    /// YM2149/YM6630 hardware (alexmush fix; the original emu2149 left the
    /// ramp position untouched here).
    fn write_shape(&mut self, val: u8) {
        self.continue_ = (val >> 3) & 1 != 0;
        self.attack = (val >> 2) & 1 != 0;
        self.alternate = (val >> 1) & 1 != 0;
        self.hold = val & 1 != 0;
        self.face = self.attack;
        self.pause = false;
        self.count = 0;
        self.ptr = if self.face { 0 } else { 0x1f };
    }

    fn set_period(&mut self, period: u16) {
        self.period = period;
    }

    /// Advances the envelope by `incr` sub-sample steps. Returns whether the
    /// ramp position reset to 0 this step while repeating (used by the
    /// caller to build the FamiStudio trigger mask).
    fn advance(&mut self, incr: u32) -> bool {
        let mut trigger = false;
        self.count += incr;
        if self.count >= u32::from(self.period) {
            if !self.pause {
                if self.face {
                    self.ptr = (self.ptr + 1) & 0x3f;
                } else {
                    self.ptr = (self.ptr + 0x3f) & 0x3f;
                }
            }

            if self.ptr & 0x20 != 0 {
                // Carry or borrow out of the 5-bit ramp.
                if self.continue_ {
                    if self.alternate ^ self.hold {
                        self.face = !self.face;
                    }
                    if self.hold {
                        self.pause = true;
                    }
                    self.ptr = if self.face { 0 } else { 0x1f };
                } else {
                    self.pause = true;
                    self.ptr = 0;
                }
            }

            if self.ptr == 0 && !self.hold && self.continue_ && (!self.alternate || !self.face) {
                trigger = true;
            }

            if u32::from(self.period) >= incr {
                self.count -= u32::from(self.period);
            } else {
                self.count = 0;
            }
        }
        trigger
    }

    /// Envelope shape considered "fast enough to trigger", matching the
    /// FamiStudio threshold in `Nes_Sunsoft::run_until`'s trigger logic.
    fn is_fast_repeating(&self) -> bool {
        self.period != 0 && self.period < 200 && !self.hold
    }
}

// ─────────────────────────────────────────────
// PSG Core
// ─────────────────────────────────────────────

/// One-bit trigger flags per channel, packed as `00AAABBB`: bits 3..=5
/// ("A") mark that the channel is able to report a trigger this step, bits
/// 0..=2 ("B") mark that a rising edge actually happened. Consumed by a
/// host (e.g. a piano-roll visualizer) that wants to know when a channel's
/// waveform restarted.
pub type TriggerMask = u8;

/// AY8930 "expanded mode" tone duty-cycle presets, ported from Furnace's
/// `ay8910.cpp` (`duty_cycle[9]`, 3.125% .. 96.875%). Each entry is a 32-bit
/// pattern; a 5-bit phase counter indexes one bit per phase step, so the
/// pattern's population count sets the mark/space ratio over a full cycle.
/// Index 4 (`0xffff0000`, 50%) is bit-for-bit the same alternating pattern a
/// plain half-period toggle produces, so it reproduces this chip's original
/// fixed-50%-duty output exactly.
const DUTY_CYCLE_TABLE: [u32; 9] = [
    0x8000_0000, // 3.125 %
    0xc000_0000, // 6.25 %
    0xf000_0000, // 12.50 %
    0xff00_0000, // 25.00 %
    0xffff_0000, // 50.00 %
    0xffff_ff00, // 75.00 %
    0xffff_fff0, // 87.50 %
    0xffff_fffc, // 93.75 %
    0xffff_fffe, // 96.875 %
];

/// Sunsoft 5B PSG core: three tone generators, one shared noise generator,
/// one shared envelope generator, and the register file that drives them.
///
/// See: <https://www.nesdev.org/wiki/Sunsoft_5B_audio>
//
//                         ┌─> Tone 0 ─┐
// Noise LFSR ──shared───> ├─> Tone 1 ─┼──> per-channel gate ──> (to mixer)
//                         └─> Tone 2 ─┘
//                               ^
//                     Envelope (opt-in per channel)
//
#[derive(Debug, Clone)]
pub struct Psg {
    voltbl: VolumeMode,

    reg: [u8; 16],

    /// Tone period and free-running full-wave position counter, per channel
    /// (`count[i]` wraps every `2*freq[i]` units — see `Psg::clock`).
    freq: [u16; 3],
    count: [u16; 3],
    /// Current tone output level, sampled from `DUTY_CYCLE_TABLE` each
    /// clock via `count[i]`'s position in the wave. Read every clock by the
    /// mixer gate, same as the plain toggle it replaced.
    edge: [bool; 3],
    /// Selected duty preset (0..=8) per channel, indexing `DUTY_CYCLE_TABLE`.
    duty_index: [u8; 3],
    /// Per-channel volume/envelope-select, as derived from $08/$09/$0A.
    /// The CPU write is 5 bits (D4..D0): D4 selects the shared envelope in
    /// place of a constant volume, D3..D0 is the 4-bit constant volume.
    /// It's stored here shifted left one (`(val << 1) | 1`, matching
    /// hardware/emu2149): bit 5 becomes the envelope-select flag and bits
    /// 4..1 hold the volume, which is what the output stage indexes with.
    volume: [u8; 3],
    /// Tone/noise disable flags from $07 (Mixer control), per channel.
    /// `true` means the corresponding source is *disabled* on that channel,
    /// matching the register's active-low polarity.
    tone_disable: [bool; 3],
    noise_disable: [bool; 3],
    /// Per-channel output level (0..=0xFF0, i.e. `voltbl value << 4`).
    ch_out: [i16; 3],

    noise_freq: u8,
    noise_count: u8,
    noise_scaler: bool,
    noise_seed: u32,

    envelope: EnvelopeGen,

    /// External channel mute mask (bit `i` set mutes tone channel `i`),
    /// distinct from the mixer-control tone/noise disable bits.
    mask: u32,

    /// Fixed-point sub-sample accumulator, `1 << GETA_BITS` per full step.
    base_count: u32,
    base_incr: u32,

    trigger_mask: TriggerMask,

    /// Set by a write that moves a tone channel's `edge` without any counter
    /// advancing (a period or duty change). Read only by
    /// [`Psg::clock_channel0`], whose fast path is otherwise entitled to
    /// assume nothing can have changed since the previous step.
    tone_dirty: bool,
}

const GETA_BITS: u32 = 24;

impl Default for Psg {
    fn default() -> Self {
        Self::new()
    }
}

impl Psg {
    pub fn new() -> Self {
        Self {
            voltbl: VolumeMode::Ym2149,
            reg: [0; 16],
            freq: [0; 3],
            count: [0; 3],
            edge: [false; 3],
            duty_index: [4; 3],
            volume: [0; 3],
            tone_disable: [false; 3],
            noise_disable: [false; 3],
            ch_out: [0; 3],
            noise_freq: 0,
            noise_count: 0,
            noise_scaler: false,
            noise_seed: 1,
            envelope: EnvelopeGen::default(),
            mask: 0,
            base_count: 0,
            base_incr: 1 << GETA_BITS,
            trigger_mask: 0,
            tone_dirty: false,
        }
    }

    /// Selects the YM2149 or AY-3-8910 volume curve.
    pub fn set_volume_mode(&mut self, mode: VolumeMode) {
        self.voltbl = mode;
    }

    /// Sets the number of sub-sample steps [`Self::clock`] advances by on
    /// each call. Defaults to `1 << 24` (one full internal step per call);
    /// a caller resampling with its own accumulator can pass a smaller
    /// increment to clock the PSG at sub-CPU-cycle granularity. Most hosts
    /// should leave this untouched and call [`Self::clock`] once per
    /// relevant tick.
    pub fn set_step_increment(&mut self, incr: u32) {
        self.base_incr = incr;
    }

    /// Externally mutes/unmutes tone channel `idx` (0..=2), independent of
    /// the chip's own $07 mixer register. Used for e.g. a UI channel-solo
    /// toggle.
    pub fn set_channel_mask(&mut self, idx: usize, muted: bool) {
        if muted {
            self.mask |= 1 << idx;
        } else {
            self.mask &= !(1 << idx);
        }
    }

    /// Current external mute mask, bit `i` set means tone channel `i` is
    /// muted.
    pub fn mask(&self) -> u32 {
        self.mask
    }

    /// Raw register value last written to `reg` (0..=15).
    pub fn reg(&self, reg: usize) -> u8 {
        self.reg[reg & 0x0f]
    }

    /// FamiStudio-style per-channel trigger flags produced by the most
    /// recent [`Self::clock`] call. See [`TriggerMask`].
    pub fn trigger_mask(&self) -> TriggerMask {
        self.trigger_mask
    }

    /// Per-channel output level (0..=0xFF0) from the most recent
    /// [`Self::clock`] call, before summing for the mixer.
    pub fn channel_output(&self, idx: usize) -> i16 {
        self.ch_out[idx]
    }

    /// Sets tone channel `idx`'s level as a direct index into the selected
    /// volume table (0..=31), clearing the envelope-select flag.
    ///
    /// This deliberately reaches past the register model. A $08/$09/$0A
    /// write is 4-bit and is stored as `(val << 1) | 1`, so the register
    /// path can only ever address the table's *odd* entries — 16 of its 32
    /// analog levels. The even entries are levels the chip really does
    /// produce, just only ever under the hardware envelope generator, which
    /// sweeps the full 0..=31 range. A host that wants that resolution as a
    /// constant volume has no register to write, so it writes here instead.
    /// Faithful register emulation goes through [`Self::write_reg`].
    pub fn set_volume_level(&mut self, idx: usize, level: u8) {
        self.volume[idx] = level & 0x1f;
    }

    /// Tone channel `idx`'s current volume-table index (0..=31), however it
    /// was set.
    pub fn volume_level(&self, idx: usize) -> u8 {
        self.volume[idx] & 0x1f
    }

    /// Sets tone channel `idx`'s duty-cycle preset (0..=8, clamped),
    /// indexing `DUTY_CYCLE_TABLE` (3.125% .. 96.875%; 4 = 50%, this chip's
    /// stock behavior). Ported from the AY8930's expanded-mode duty select,
    /// see `AY8930-TONE-BEHAVIOR-REPORT.md` in `.references/`.
    pub fn set_duty_index(&mut self, idx: usize, duty_index: u8) {
        self.duty_index[idx] = duty_index.min(8);
        self.tone_dirty = true;
    }

    /// Tone channel `idx`'s current duty-cycle preset index (0..=8).
    pub fn duty_index(&self, idx: usize) -> u8 {
        self.duty_index[idx]
    }

    /// Volume-table index whose output level is nearest `num`/`den` of full
    /// scale — a *linear* fader over the chip's logarithmic ladder.
    ///
    /// The 5B's volume table is an analog ladder roughly 1.5 dB per entry, so
    /// scaling the index does not scale the amplitude: index 16 of 31 is
    /// about −23 dB, not −6 dB. A host that wants a linear response (to match
    /// the 2A03 pulse's 4-bit linear DAC, say) has to search the table, which
    /// is what this does. It reads the *selected* table, so it stays right if
    /// the AY-3-8910 curve is in use. Ties resolve to the lower index.
    pub fn linear_volume_index(&self, num: u8, den: u8) -> u8 {
        let table = self.voltbl.table();
        let target = if den == 0 {
            0
        } else {
            u32::from(num) * table[31] / u32::from(den)
        };
        let mut best = 0u8;
        let mut best_err = u32::MAX;
        for (i, &level) in table.iter().enumerate() {
            let err = level.abs_diff(target);
            if err < best_err {
                best_err = err;
                best = i as u8;
            }
        }
        best
    }

    /// Sum of all three tone channels' output, matching the chip's mono
    /// mix.
    pub fn output(&self) -> i16 {
        self.ch_out[0] + self.ch_out[1] + self.ch_out[2]
    }

    // ── Register writes ─────────────────────

    /// Writes `val` to internal register `reg` (0..=15). Out-of-range
    /// register numbers are ignored, matching hardware.
    pub fn write_reg(&mut self, reg: u32, val: u32) {
        if reg > 15 {
            return;
        }
        let reg = reg as usize;
        let val = (val as u8) & REG_MASK[reg];
        self.reg[reg] = val;

        match reg {
            0..=5 => {
                let c = reg >> 1;
                self.freq[c] =
                    (u16::from(self.reg[c * 2 + 1] & 0x0f) << 8) | u16::from(self.reg[c * 2]);
                self.tone_dirty = true;
            }
            6 => {
                self.noise_freq = val & 0x1f;
            }
            7 => {
                self.tone_disable[0] = val & 0x01 != 0;
                self.tone_disable[1] = val & 0x02 != 0;
                self.tone_disable[2] = val & 0x04 != 0;
                self.noise_disable[0] = val & 0x08 != 0;
                self.noise_disable[1] = val & 0x10 != 0;
                self.noise_disable[2] = val & 0x20 != 0;
            }
            8..=10 => {
                // The masked write value is 5 bits (D4..D0: envelope-select
                // + 4-bit volume). It's shifted left one and OR'd with 1
                // before storing, so the stored byte's bit 5 is the
                // envelope-select flag and bits 4..1 hold the volume — this
                // matches the original C exactly (and its `| 1` low bit is
                // simply unused by the read side below).
                self.volume[reg - 8] = (val << 1) | 1;
            }
            11 | 12 => {
                let period = (u16::from(self.reg[12]) << 8) | u16::from(self.reg[11]);
                self.envelope.set_period(period);
            }
            13 => {
                self.envelope.write_shape(val);
            }
            _ => {}
        }
    }

    /// Current value of internal register `reg` (0..=15), or 0 if out of
    /// range.
    pub fn read_reg(&self, reg: u32) -> u8 {
        self.reg.get(reg as usize & 0x1f).copied().unwrap_or(0)
    }

    // ── Clocking ────────────────────────────

    /// Advances the PSG by one internal step (envelope, noise LFSR, and all
    /// three tone generators), recomputing [`Self::channel_output`]/
    /// [`Self::output`] and [`Self::trigger_mask`]. Call once per host
    /// sub-sample tick; see [`Self::set_step_increment`] for finer
    /// granularity.
    pub fn clock(&mut self) {
        self.trigger_mask = 0;
        let incr = self.take_step_increment();
        let (noise, env_trigger) = self.clock_shared(incr);
        for i in 0..3 {
            self.clock_tone(i, incr, noise, env_trigger);
        }
        self.tone_dirty = false;
    }

    /// Advances the PSG by one internal step, computing tone channel 0 only.
    ///
    /// For callers that drive channel 0 alone — this crate's [`Sunsoft`]
    /// voice helpers do, since each polyphonic voice owns a private chip and
    /// only ever addresses its first tone channel. Channels 1 and 2 keep
    /// whatever [`Self::channel_output`] and [`Self::trigger_mask`] bits the
    /// last full [`Self::clock`] left them (silence, on a chip that has never
    /// been given a period or volume for them). Channel 0's output is
    /// bit-identical to what [`Self::clock`] would have produced.
    ///
    /// Worth having because the NES feeds this chip through a divide-by-16
    /// prescaler (see [`Sunsoft::new`]), so 15 of every 16 calls only advance
    /// `base_count` and produce no tone step at all — yet the general path
    /// still runs a tone body costing two integer divisions *per channel* on
    /// every one of them. Both the swallowed steps and the two unused
    /// channels are pure overhead for a polyphonic voice pool, where this is
    /// clocked once per CPU cycle per sounding voice.
    pub fn clock_channel0(&mut self) {
        let incr = self.take_step_increment();

        // The shared generators always run: they are a handful of integer ops
        // with no division, and they are *not* inert on a swallowed step —
        // both `EnvelopeGen::advance` and the noise LFSR guard against a
        // period of 0, which `x >= 0` makes true unconditionally, and 0 is
        // this chip's reset value for both. Only the tone body is skipped.
        self.trigger_mask = 0;
        let (noise, env_trigger) = self.clock_shared(incr);

        if incr == 0 && !self.tone_dirty {
            // Nothing the tone body reads has moved: `count[0]` is below
            // `full_cycle` (the previous `clock_tone` left it there, and
            // `tone_dirty` would be set had a period or duty write happened
            // since), so it would wrap to itself, leaving `phase` and
            // `edge[0]` exactly as they are. The output gate still has to
            // run — `noise` and `envelope.ptr` above may have moved, as may
            // a volume, mute, or mixer register written between two clocks.
            self.update_ch_out(0, noise);
            self.trigger_mask = self.trigger_bits(0, env_trigger, false);
            return;
        }

        self.clock_tone(0, incr, noise, env_trigger);
        self.tone_dirty = false;
    }

    /// Advances the sub-sample accumulator, returning the number of whole
    /// internal steps this call produces — 0 on a call the caller's step
    /// increment swallows.
    #[inline]
    fn take_step_increment(&mut self) -> u32 {
        self.base_count += self.base_incr;
        let incr = self.base_count >> GETA_BITS;
        self.base_count &= (1 << GETA_BITS) - 1;
        incr
    }

    /// Advances the two chip-wide generators by `incr` steps, returning the
    /// current noise gate level and whether the envelope reported a trigger.
    #[inline]
    fn clock_shared(&mut self, incr: u32) -> (bool, bool) {
        let env_trigger = self.envelope.advance(incr);

        // Noise: shared 17-bit LFSR, advances at half the rate of its own
        // period counter (the `noise_scaler` toggle), matching real
        // YM2149/YM6630 hardware.
        self.noise_count = self.noise_count.wrapping_add(incr as u8);
        if u32::from(self.noise_count) >= u32::from(self.noise_freq) {
            self.noise_scaler = !self.noise_scaler;
            if self.noise_scaler {
                if (self.noise_seed ^ (self.noise_seed >> 3)) & 1 != 0 {
                    self.noise_seed |= 1 << 17;
                }
                self.noise_seed >>= 1;
            }

            if u32::from(self.noise_freq) >= incr {
                self.noise_count -= self.noise_freq;
            } else {
                self.noise_count = 0;
            }
        }

        (self.noise_seed & 1 == 0, env_trigger)
    }

    /// Advances tone channel `i` by `incr` steps and folds its new edge into
    /// [`Self::channel_output`] and [`Self::trigger_mask`].
    #[inline]
    fn clock_tone(&mut self, i: usize, incr: u32, noise: bool, env_trigger: bool) {
        // `count[i]` free-runs across a full wave (`2*freq[i]` count units —
        // the register period historically covered a half-cycle, two toggles
        // per wave) and wraps every cycle. Its position within that span maps
        // directly onto the 32-wide `DUTY_CYCLE_TABLE` pattern, so resolution
        // is exact at any tone period instead of stepping in fixed increments
        // that would underflow at low periods.
        let full_cycle = u32::from(self.freq[i]).saturating_mul(2).max(1);
        self.count[i] = ((u32::from(self.count[i]) + incr) % full_cycle) as u16;
        let phase = ((u32::from(self.count[i]) * 32) / full_cycle) as u8 & 0x1f;

        let pattern = DUTY_CYCLE_TABLE[usize::from(self.duty_index[i])];
        let new_edge = pattern & (1 << phase) != 0;
        let tone_trigger = new_edge != self.edge[i];
        self.edge[i] = new_edge;

        self.update_ch_out(i, noise);
        self.trigger_mask |= self.trigger_bits(i, env_trigger, tone_trigger);
    }

    /// Recomputes tone channel `i`'s output level from the current mute,
    /// mixer, edge, and volume/envelope state.
    #[inline]
    fn update_ch_out(&mut self, i: usize, noise: bool) {
        self.ch_out[i] = if self.mask & (1 << i) != 0 {
            0
        // Both terms are *disable* bits, so a disabled source contributes an
        // unconditional `true` and lets the other one gate alone (emu2149's
        // `(tmask||edge) && (nmask||noise)`). Negating them instead swaps the
        // two sources: tone-enabled/noise-disabled would be gated by the
        // noise bit and vice versa.
        } else if (self.tone_disable[i] || self.edge[i]) && (self.noise_disable[i] || noise) {
            let table = self.voltbl.table();
            if self.volume[i] & 0x20 == 0 {
                (table[usize::from(self.volume[i] & 0x1f)] as i16) << 4
            } else {
                (table[usize::from(self.envelope.ptr)] as i16) << 4
            }
        } else {
            0
        };
    }

    /// Tone channel `i`'s contribution to [`Self::trigger_mask`]. If the
    /// channel is gated by a repeating (non-hold) envelope, report the
    /// envelope's trigger; otherwise report the tone edge, provided the tone
    /// isn't disabled.
    #[inline]
    fn trigger_bits(&self, i: usize, env_trigger: bool, tone_trigger: bool) -> TriggerMask {
        if self.envelope.is_fast_repeating() && self.volume[i] & 0x20 != 0 {
            (0x08 << i) | (u8::from(env_trigger) << i)
        } else if !self.tone_disable[i] && self.freq[i] != 0 {
            (0x08 << i) | (u8::from(tone_trigger) << i)
        } else {
            0
        }
    }

    // ── Reset ───────────────────────────────

    /// Resets all state as if freshly constructed, preserving the currently
    /// selected [`VolumeMode`] and step increment (matching the original
    /// `PSG_reset`, which is a soft reset distinct from `PSG_new`).
    pub fn reset(&mut self) {
        let voltbl = self.voltbl;
        let base_incr = self.base_incr;
        *self = Self::new();
        self.voltbl = voltbl;
        self.base_incr = base_incr;
    }
}

// ─────────────────────────────────────────────
// Sunsoft 5B Wrapper
// ─────────────────────────────────────────────

/// Sunsoft 5B mapper register window.
///   $C000..$DFFF: register select (writes the target register 0..=15)
///   $E000..$FFFF: register write (writes the selected register)
const REG_SELECT_BASE: u16 = 0xC000;
const REG_WRITE_BASE: u16 = 0xE000;
const REG_RANGE: u16 = 0x2000;

/// Sunsoft 5B expansion audio: a [`Psg`] plus the mapper's register-select
/// latch, per-channel enable, and the register-shadowing needed to replay
/// state when a host seeks (e.g. rewind/fast-forward in an emulator UI).
///
/// See: <https://www.nesdev.org/wiki/Sunsoft_5B_audio>
#[derive(Debug, Clone)]
pub struct Sunsoft {
    psg: Psg,
    /// Currently selected internal register (0..=15), latched by a write
    /// to the $C000 window.
    selected_reg: u8,
    /// Age, in register writes, since each internal register was last
    /// written — mirrors `Nes_Sunsoft::ages`, used by a debug/visualizer
    /// UI to fade out stale register displays.
    ages: [u8; 16],
    /// Shadow copy of the 16 internal registers used only while seeking;
    /// `None` means "not touched since seeking started" for that register.
    shadow_regs: [Option<u8>; 16],
}

impl Default for Sunsoft {
    fn default() -> Self {
        Self::new()
    }
}

impl Sunsoft {
    pub fn new() -> Self {
        let mut sunsoft = Self {
            psg: Psg::new(),
            selected_reg: 0,
            ages: [0; 16],
            shadow_regs: [None; 16],
        };
        sunsoft.reset();
        // On the NES the 5B's PSG is fed the CPU clock through a divide-by-2
        // prescaler and its own divide-by-8 tone prescaler, so one internal
        // PSG step is 16 CPU cycles — that is what makes the tone register
        // mean `f = clk / (32 * TP)`. Callers clock us once per CPU cycle
        // (`Sunsoft::clock`), so the /16 lives here rather than in `Psg`,
        // which stays a rate-agnostic chip model. FamiStudio does the same
        // split: `PSG_setRate(psg, psg_clock / 16)` plus `t += 16` in
        // `Nes_Sunsoft::run_until`.
        sunsoft.psg.set_step_increment((1 << GETA_BITS) / 16);
        sunsoft
    }

    /// Shared PSG core, for direct access to per-sample output/trigger
    /// state.
    pub fn psg(&self) -> &Psg {
        &self.psg
    }

    /// Advances the PSG by one internal step. See [`Psg::clock_channel0`] —
    /// only tone channel 0 is computed, because only tone channel 0 is ever
    /// driven (see the channel-0-scoped helpers below). Channels 1 and 2 stay
    /// silent, which is what the full three-channel path produces for them
    /// anyway: nothing gives them a period or a volume.
    pub fn clock(&mut self) {
        self.psg.clock_channel0();
    }

    /// Sum of all three tone channels, matching the chip's mono mix.
    pub fn output(&self) -> i16 {
        self.psg.output()
    }

    /// Per-channel trigger flags from the most recent [`Self::clock`]
    /// call. See [`TriggerMask`].
    pub fn trigger_mask(&self) -> TriggerMask {
        self.psg.trigger_mask()
    }

    /// Mutes or unmutes tone channel `idx` (0..=2) from a host UI, leaving
    /// the chip's own mixer register untouched.
    pub fn enable_channel(&mut self, idx: usize, enabled: bool) {
        self.psg.set_channel_mask(idx, !enabled);
    }

    // ── Channel-0-scoped voice helpers ───────
    //
    // This project drives one polyphonic voice pool, each voice owning a
    // private `Sunsoft` that only ever uses tone channel 0 (see
    // `.references/Implement-Plans-S5B/CoreHook.md`, "Architecture
    // mismatch"). These are thin wrappers around `write_reg` for that one
    // channel — `Psg` itself stays a faithful 3-channel/16-register model.

    /// Channel 0 fine period (reg 0).
    pub fn write_timer_lo(&mut self, val: u8) {
        self.psg.write_reg(0, u32::from(val));
    }

    /// Channel 0 coarse period (reg 1, masked to 4 bits by hardware).
    pub fn write_timer_hi(&mut self, val: u8) {
        self.psg.write_reg(1, u32::from(val));
    }

    /// Channel 0 output level, as a direct 0..=31 index into the volume
    /// table. See [`Psg::set_volume_level`] — this reaches all 32 analog
    /// levels rather than the 16 the 4-bit constant-volume register can
    /// address. Pass `(v << 1) | 1` to reproduce a register write of the
    /// 4-bit volume `v`.
    pub fn write_volume_level(&mut self, level: u8) {
        self.psg.set_volume_level(0, level);
    }

    /// Channel 0 tone duty-cycle preset (0..=8). See [`Psg::set_duty_index`].
    pub fn write_duty_index(&mut self, duty_index: u8) {
        self.psg.set_duty_index(0, duty_index);
    }

    /// Chip-global noise generator period (reg 6, masked to 5 bits).
    pub fn write_noise_period(&mut self, period: u8) {
        self.psg.write_reg(6, u32::from(period & 0x1F));
    }

    /// Channel 0's tone/noise enable bits in the mixer register (reg 7),
    /// read-modify-write so channels 1/2 stay disabled (their bits forced
    /// to `1`, matching the register's active-low polarity) since nothing
    /// drives them.
    pub fn set_tone_noise_enable(&mut self, tone: bool, noise: bool) {
        let mut val = self.psg.reg(7) | 0b0011_0110; // channels 1/2 tone+noise disabled
        if tone {
            val &= !0x01;
        } else {
            val |= 0x01;
        }
        if noise {
            val &= !0x08;
        } else {
            val |= 0x08;
        }
        self.psg.write_reg(7, u32::from(val));
    }

    // ── Register access ──────────────────────

    /// Handles a CPU write in the mapper's register window: `addr` in
    /// `$C000..$DFFF` selects a register, `addr` in `$E000..$FFFF` writes
    /// the currently selected register.
    pub fn write_register(&mut self, addr: u16, data: u8) {
        // Widen to u32 before adding REG_RANGE: `$E000 + $2000` overflows a
        // u16 (it lands exactly at $10000), so the range check has to run
        // in a type that can represent that.
        let addr = u32::from(addr);
        if addr >= u32::from(REG_SELECT_BASE) && addr < u32::from(REG_SELECT_BASE) + u32::from(REG_RANGE) {
            self.selected_reg = data;
        } else if addr >= u32::from(REG_WRITE_BASE) && addr < u32::from(REG_WRITE_BASE) + u32::from(REG_RANGE) {
            self.psg.write_reg(u32::from(self.selected_reg), u32::from(data));
            if let Some(slot) = self.ages.get_mut(self.selected_reg as usize & 0x0f) {
                *slot = 0;
            }
        }
    }

    /// Copies out the 16 internal register values and their write-ages, and
    /// advances every age by one (saturating). Intended for a periodic
    /// debug/visualizer snapshot, matching `Nes_Sunsoft::get_register_values`.
    pub fn register_values(&mut self, regs: &mut [u8; 16], ages: &mut [u8; 16]) {
        for i in 0..16 {
            regs[i] = self.psg.reg(i);
            ages[i] = self.ages[i];
            self.ages[i] = self.ages[i].saturating_add(1);
        }
    }

    // ── Seeking ──────────────────────────────

    /// Begins tracking register writes for a later [`Self::stop_seeking`]
    /// replay, discarding any previously tracked shadow state.
    pub fn start_seeking(&mut self) {
        self.shadow_regs = [None; 16];
    }

    /// Replays every register touched since [`Self::start_seeking`] through
    /// the real register-select/write sequence, so the chip ends up in the
    /// exact state a save state (or fast-forward) landed on, without
    /// replaying every intermediate write.
    pub fn stop_seeking(&mut self) {
        for i in 0..16 {
            if let Some(val) = self.shadow_regs[i] {
                self.write_register(REG_SELECT_BASE, i as u8);
                self.write_register(REG_WRITE_BASE, val);
            }
        }
    }

    /// Records a register write into the shadow copy instead of the live
    /// PSG, for use only between [`Self::start_seeking`] and
    /// [`Self::stop_seeking`].
    pub fn write_shadow_register(&mut self, addr: u16, data: u8) {
        let addr = u32::from(addr);
        if addr >= u32::from(REG_SELECT_BASE) && addr < u32::from(REG_SELECT_BASE) + u32::from(REG_RANGE) {
            self.selected_reg = data;
        } else if addr >= u32::from(REG_WRITE_BASE)
            && addr < u32::from(REG_WRITE_BASE) + u32::from(REG_RANGE)
            && let Some(slot) = self.shadow_regs.get_mut(self.selected_reg as usize & 0x0f)
        {
            *slot = Some(data);
        }
    }

    // ── Reset ───────────────────────────────

    /// Resets the PSG and all wrapper state (register select, ages, shadow
    /// registers).
    pub fn reset(&mut self) {
        self.psg.reset();
        self.selected_reg = 0;
        self.ages = [0; 16];
        self.shadow_regs = [None; 16];
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── PSG core tests ──

    #[test]
    fn new_psg_outputs_silence() {
        let psg = Psg::new();
        assert_eq!(psg.output(), 0);
    }

    #[test]
    fn tone_period_write_updates_frequency() {
        let mut psg = Psg::new();
        psg.write_reg(0, 0xFD); // channel A period lo
        psg.write_reg(1, 0x02); // channel A period hi (masked to 4 bits)
        assert_eq!(psg.freq[0], 0x02FD);
    }

    #[test]
    fn noise_period_write_masks_to_five_bits() {
        let mut psg = Psg::new();
        psg.write_reg(6, 0xFF);
        assert_eq!(psg.noise_freq, 0x1f);
    }

    #[test]
    fn out_of_range_register_is_ignored() {
        let mut psg = Psg::new();
        psg.write_reg(16, 0xFF);
        assert_eq!(psg.reg(0), 0);
    }

    #[test]
    fn mixer_control_sets_tone_and_noise_disable_flags() {
        let mut psg = Psg::new();
        // D0..D2 = tone disable (channels A,B,C), D3..D5 = noise disable.
        // bit0=1 (A tone), bit2=1 (C tone), bit3=1 (A noise), bit4=1 (B noise)
        psg.write_reg(7, 0b0001_1101);
        assert!(psg.tone_disable[0]);
        assert!(!psg.tone_disable[1]);
        assert!(psg.tone_disable[2]);
        assert!(psg.noise_disable[0]);
        assert!(psg.noise_disable[1]);
        assert!(!psg.noise_disable[2]);
    }

    #[test]
    fn channel_a_constant_volume_produces_tone_when_enabled() {
        let mut psg = Psg::new();
        // Enable tone A, disable noise A (so only the tone edge gates it).
        psg.write_reg(7, 0b0000_1110); // tone A enabled (bit0=0), B/C tone disabled, noise all disabled
        psg.write_reg(0, 4); // short period so it clocks quickly
        psg.write_reg(1, 0);
        psg.write_reg(8, 0x0F); // channel A constant volume = max, not envelope

        // Step until the tone edge goes high.
        let mut saw_output = false;
        for _ in 0..64 {
            psg.clock();
            if psg.channel_output(0) != 0 {
                saw_output = true;
                break;
            }
        }
        assert!(saw_output, "channel A should produce nonzero output once its edge goes high");
    }

    #[test]
    fn masked_channel_is_always_silent() {
        let mut psg = Psg::new();
        psg.write_reg(7, 0b0000_1110); // tone A enabled, rest disabled
        psg.write_reg(0, 4);
        psg.write_reg(1, 0);
        psg.write_reg(8, 0x0F);
        psg.set_channel_mask(0, true);

        for _ in 0..64 {
            psg.clock();
            assert_eq!(psg.channel_output(0), 0);
        }
    }

    #[test]
    fn envelope_select_bit_uses_envelope_ptr_instead_of_constant_volume() {
        let mut psg = Psg::new();
        psg.write_reg(7, 0b0000_1110); // tone A enabled, rest disabled
        psg.write_reg(0, 4);
        psg.write_reg(1, 0);
        psg.write_reg(8, 0x10); // D4 set -> select shared envelope, volume bits ignored
        psg.write_reg(11, 0x10); // envelope period lo
        psg.write_reg(12, 0x00); // envelope period hi
        psg.write_reg(13, 0b0000_1100); // continue + attack -> ramps up from 0

        // Just verify it doesn't panic and eventually produces nonzero
        // output as the envelope ramps up from 0.
        let mut saw_output = false;
        for _ in 0..2000 {
            psg.clock();
            if psg.channel_output(0) != 0 {
                saw_output = true;
                break;
            }
        }
        assert!(saw_output);
    }

    #[test]
    fn envelope_shape_write_resets_ramp_position() {
        let mut psg = Psg::new();
        psg.write_reg(11, 0x01); // short envelope period
        psg.write_reg(12, 0x00);
        psg.write_reg(13, 0b0000_1000); // continue only, attack=0 -> starts at 0x1f, ramps down

        for _ in 0..200 {
            psg.clock();
        }
        assert_ne!(psg.envelope.ptr, 0x1f, "envelope should have ramped away from its reset position");

        // Rewriting the shape register should fully restart the ramp.
        psg.write_reg(13, 0b0000_1000);
        assert_eq!(psg.envelope.ptr, 0x1f);
    }

    #[test]
    fn noise_lfsr_changes_seed_over_time() {
        let mut psg = Psg::new();
        psg.write_reg(6, 1); // fast noise period
        let initial_seed = psg.noise_seed;
        for _ in 0..64 {
            psg.clock();
        }
        assert_ne!(psg.noise_seed, initial_seed);
    }

    #[test]
    fn volume_mode_selects_correct_table() {
        let mut psg = Psg::new();
        psg.set_volume_mode(VolumeMode::Ay8910);
        assert_eq!(psg.voltbl.table(), &VOLTBL_AY8910);
        psg.set_volume_mode(VolumeMode::Ym2149);
        assert_eq!(psg.voltbl.table(), &VOLTBL_YM2149);
    }

    #[test]
    fn reset_clears_registers_but_keeps_volume_mode() {
        let mut psg = Psg::new();
        psg.set_volume_mode(VolumeMode::Ay8910);
        psg.write_reg(0, 0xFF);
        psg.write_reg(8, 0x1F);

        psg.reset();

        assert_eq!(psg.reg(0), 0);
        assert_eq!(psg.output(), 0);
        assert_eq!(psg.voltbl, VolumeMode::Ay8910);
    }

    /// `clock_channel0` is an optimization of `clock`, so the only thing that
    /// keeps it honest is agreeing with `clock` clock-for-clock. Both chips
    /// get the /16 prescaler `Sunsoft` uses (the ratio the fast path exists
    /// to exploit) and the same register script, mixing in every write that
    /// can change channel 0's output without a counter advancing: period,
    /// duty, volume, mute, mixer, and the shared envelope — including the
    /// degenerate zero-period envelope/noise states the fast path has to
    /// decline to take.
    #[test]
    fn clock_channel0_matches_full_clock_on_channel_zero() {
        let mut reference = Psg::new();
        let mut fast = Psg::new();
        let step = (1 << GETA_BITS) / 16;
        reference.set_step_increment(step);
        fast.set_step_increment(step);

        // (clock at which to apply, register, value); `None` register means a
        // non-register setter, applied via the closure below.
        let script: [(u32, u32, u32); 14] = [
            (0, 7, 0b0011_1110),  // channel 0 tone on, noise off
            (0, 0, 0x40),         // period lo
            (0, 1, 0x01),         // period hi
            (0, 8, 0x0F),         // constant volume, max
            (300, 8, 0x07),       // volume change mid-wave
            (700, 0, 0x11),       // period change mid-wave
            (1100, 6, 0x0B),      // noise period
            (1100, 7, 0b0011_0110), // noise on alongside tone
            (1900, 6, 0x00),      // degenerate: noise period 0
            (2600, 11, 0x40),     // envelope period lo
            (2600, 12, 0x00),     // envelope period hi
            (2600, 13, 0b0000_1100), // continue+attack shape
            (2600, 8, 0x10),      // channel 0 now gated by the envelope
            (3800, 11, 0x00),     // degenerate: envelope period 0
        ];

        for clock in 0..6000u32 {
            for &(at, reg, val) in &script {
                if at == clock {
                    reference.write_reg(reg, val);
                    fast.write_reg(reg, val);
                }
            }
            // Setters that bypass the register file, on their own schedule.
            if clock == 500 {
                reference.set_duty_index(0, 2);
                fast.set_duty_index(0, 2);
            }
            if clock == 1500 {
                reference.set_volume_level(0, 22);
                fast.set_volume_level(0, 22);
            }
            if clock == 2000 {
                reference.set_channel_mask(0, true);
                fast.set_channel_mask(0, true);
            }
            if clock == 2200 {
                reference.set_channel_mask(0, false);
                fast.set_channel_mask(0, false);
            }

            reference.clock();
            fast.clock_channel0();

            assert_eq!(
                fast.channel_output(0),
                reference.channel_output(0),
                "channel 0 output diverged at clock {clock}"
            );
            assert_eq!(
                fast.trigger_mask() & 0b0100_1001,
                reference.trigger_mask() & 0b0100_1001,
                "channel 0 trigger bits diverged at clock {clock}"
            );
        }
    }

    /// The scripted equivalence test above fixes *when* each write lands, and
    /// the fast path's whole premise is a 16-clock prescaler — so a bug in it
    /// would most plausibly hide at one particular write phase relative to
    /// that boundary. This walks the same comparison over 400 pseudo-random
    /// scripts (fixed seed, so a failure is reproducible), scattering writes
    /// across every phase and interleaving with the degenerate zero-period
    /// states the two chips start in.
    #[test]
    fn clock_channel0_matches_full_clock_under_random_register_traffic() {
        // xorshift32: deterministic, no dev-dependency.
        let mut seed = 0x5B_5B_5B_5Bu32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };

        for script in 0..400 {
            let mut reference = Psg::new();
            let mut fast = Psg::new();
            let step = (1 << GETA_BITS) / 16;
            reference.set_step_increment(step);
            fast.set_step_increment(step);

            for clock in 0..400u32 {
                // ~1 write every 8 clocks, landing on arbitrary phases.
                if next() % 8 == 0 {
                    let val = next() & 0xFF;
                    match next() % 8 {
                        0..=4 => {
                            // Registers that matter here: periods, noise
                            // period, mixer, volume, envelope period/shape.
                            let reg = [0, 1, 6, 7, 8, 11, 12, 13][(next() % 8) as usize];
                            reference.write_reg(reg, val);
                            fast.write_reg(reg, val);
                        }
                        5 => {
                            let level = (val & 0x1f) as u8;
                            reference.set_volume_level(0, level);
                            fast.set_volume_level(0, level);
                        }
                        6 => {
                            let duty = (val % 9) as u8;
                            reference.set_duty_index(0, duty);
                            fast.set_duty_index(0, duty);
                        }
                        _ => {
                            let muted = val & 1 != 0;
                            reference.set_channel_mask(0, muted);
                            fast.set_channel_mask(0, muted);
                        }
                    }
                }

                reference.clock();
                fast.clock_channel0();

                assert_eq!(
                    fast.channel_output(0),
                    reference.channel_output(0),
                    "script {script}: channel 0 output diverged at clock {clock}"
                );
                assert_eq!(
                    fast.trigger_mask() & 0b0100_1001,
                    reference.trigger_mask() & 0b0100_1001,
                    "script {script}: channel 0 trigger bits diverged at clock {clock}"
                );
            }
        }
    }

    /// `Sunsoft::clock` computes tone channel 0 alone, so `output()` — the
    /// only thing the synth's audio path reads — is only unchanged if
    /// channels 1 and 2 really do contribute nothing. They are never given a
    /// period, volume, or duty, and `set_tone_noise_enable` forces their
    /// mixer bits off, so both routes through the output gate land on
    /// `table[0]`, which is 0 in both volume curves. Driven here through the
    /// same helpers `apply_s5b_modulation` calls, including a gate-off.
    #[test]
    fn sunsoft_output_comes_entirely_from_channel_zero() {
        for mode in [VolumeMode::Ym2149, VolumeMode::Ay8910] {
            let mut sunsoft = Sunsoft::new();
            sunsoft.psg.set_volume_mode(mode);

            for clock in 0..8000u32 {
                match clock {
                    0 => {
                        sunsoft.write_timer_lo(0x40);
                        sunsoft.write_timer_hi(0x01);
                        sunsoft.write_volume_level(31);
                        sunsoft.set_tone_noise_enable(true, false);
                    }
                    2000 => {
                        // Noise on alongside tone, as a noise-flagged duty
                        // step does.
                        sunsoft.write_noise_period(0x0B);
                        sunsoft.set_tone_noise_enable(true, true);
                    }
                    4000 => {
                        sunsoft.write_duty_index(7);
                        sunsoft.write_volume_level(18);
                    }
                    6000 => sunsoft.set_tone_noise_enable(false, false), // gate off
                    _ => {}
                }

                sunsoft.clock();

                assert_eq!(
                    sunsoft.psg.channel_output(1),
                    0,
                    "{mode:?}: channel 1 spoke at clock {clock}"
                );
                assert_eq!(
                    sunsoft.psg.channel_output(2),
                    0,
                    "{mode:?}: channel 2 spoke at clock {clock}"
                );
                assert_eq!(
                    sunsoft.output(),
                    sunsoft.psg.channel_output(0),
                    "{mode:?}: mix diverged from channel 0 at clock {clock}"
                );
            }
        }
    }

    /// Not an assertion — a stopwatch. Run with
    /// `cargo test --release -p rp2a03_core -- --ignored --nocapture`
    /// to compare the two paths at the clock rate a sounding voice uses.
    /// Alternates the two and reports the best of several rounds, so a cold
    /// cache or a clock-speed ramp in the first round cannot decide the
    /// result.
    #[test]
    #[ignore = "timing measurement, not a correctness check"]
    fn clock_channel0_throughput_report() {
        use std::time::{Duration, Instant};

        const CLOCKS: u32 = 20_000_000;
        const ROUNDS: usize = 3;

        // Set up as `apply_s5b_modulation` leaves a sounding voice: a tone
        // period, a constant volume, tone on / noise off, and the envelope
        // and noise periods left at their reset value of 0 (nothing in this
        // project writes them).
        fn armed() -> Psg {
            let mut psg = Psg::new();
            psg.set_step_increment((1 << GETA_BITS) / 16);
            psg.write_reg(7, 0b0011_1110);
            psg.write_reg(0, 0x40);
            psg.write_reg(1, 0x01);
            psg.write_reg(8, 0x0F);
            psg
        }

        fn time(mut step: impl FnMut(&mut Psg)) -> Duration {
            let mut psg = armed();
            let start = Instant::now();
            for _ in 0..CLOCKS {
                step(&mut psg);
                std::hint::black_box(psg.output());
            }
            start.elapsed()
        }

        // Reference point: the 2A03 pulse, clocked and read exactly as
        // `Voice::clock_channel_output` does. Everything wrapped around the
        // chip model per clock is identical between the two channel modes, so
        // this is what "S5B costs the same as any other waveform" means.
        fn time_pulse() -> Duration {
            use crate::apu_pulse::{Pulse, PulseChannel};
            let mut pulse = Pulse::new(PulseChannel::One);
            pulse.set_enabled(true);
            pulse.write_sweep(0x08);
            pulse.write_timer_lo(0x40);
            pulse.write_timer_hi(0x01);
            pulse.write_ctrl(0x9F);
            let start = Instant::now();
            for _ in 0..CLOCKS {
                pulse.clock();
                std::hint::black_box(pulse.output());
            }
            start.elapsed()
        }

        let mut full = Duration::MAX;
        let mut fast = Duration::MAX;
        let mut pulse = Duration::MAX;
        for _ in 0..ROUNDS {
            full = full.min(time(Psg::clock));
            fast = fast.min(time(Psg::clock_channel0));
            pulse = pulse.min(time_pulse());
        }

        println!("clock():          {full:?} for {CLOCKS} clocks");
        println!("clock_channel0(): {fast:?} for {CLOCKS} clocks");
        println!("Pulse::clock():   {pulse:?} for {CLOCKS} clocks (reference)");
        println!(
            "speedup: {:.2}x, now {:.2}x the cost of a pulse channel",
            full.as_secs_f64() / fast.as_secs_f64().max(f64::EPSILON),
            fast.as_secs_f64() / pulse.as_secs_f64().max(f64::EPSILON),
        );
    }

    // ── Sunsoft wrapper tests ──

    #[test]
    fn new_sunsoft_is_silent() {
        let sunsoft = Sunsoft::new();
        assert_eq!(sunsoft.output(), 0);
    }

    #[test]
    fn write_register_selects_then_writes() {
        let mut sunsoft = Sunsoft::new();
        sunsoft.write_register(REG_SELECT_BASE, 6); // select noise period register
        sunsoft.write_register(REG_WRITE_BASE, 0x1F);
        assert_eq!(sunsoft.psg.noise_freq, 0x1f);
    }

    #[test]
    fn writing_a_register_resets_its_age() {
        let mut sunsoft = Sunsoft::new();
        sunsoft.write_register(REG_SELECT_BASE, 0);
        sunsoft.write_register(REG_WRITE_BASE, 0x42);

        let mut regs = [0u8; 16];
        let mut ages = [0u8; 16];
        // Advance the age a few times before checking.
        sunsoft.register_values(&mut regs, &mut ages);
        sunsoft.register_values(&mut regs, &mut ages);
        assert_eq!(regs[0], 0x42);
        // ages[0] reflects the age *before* this call's increment, so after
        // two snapshot calls following one write, it should read 1.
        assert_eq!(ages[0], 1);
    }

    #[test]
    fn enable_channel_maps_to_psg_mask() {
        let mut sunsoft = Sunsoft::new();
        sunsoft.enable_channel(1, false);
        assert_eq!(sunsoft.psg.mask(), 0b010);
        sunsoft.enable_channel(1, true);
        assert_eq!(sunsoft.psg.mask(), 0);
    }

    #[test]
    fn seeking_replays_only_shadowed_registers() {
        let mut sunsoft = Sunsoft::new();
        // Give register 0 a known live value first.
        sunsoft.write_register(REG_SELECT_BASE, 0);
        sunsoft.write_register(REG_WRITE_BASE, 0x11);

        sunsoft.start_seeking();
        sunsoft.write_shadow_register(REG_SELECT_BASE, 6);
        sunsoft.write_shadow_register(REG_WRITE_BASE, 0x1F); // shadow-writes noise period

        // Live PSG state must be untouched while seeking.
        assert_eq!(sunsoft.psg.noise_freq, 0);

        sunsoft.stop_seeking();

        assert_eq!(sunsoft.psg.noise_freq, 0x1f);
        // Register 0 wasn't touched during seeking, so it should be
        // unaffected by stop_seeking.
        assert_eq!(sunsoft.psg.reg(0), 0x11);
    }

    #[test]
    fn channel0_helpers_round_trip_through_psg_registers() {
        let mut sunsoft = Sunsoft::new();
        sunsoft.write_timer_lo(0xFD);
        sunsoft.write_timer_hi(0x02);
        assert_eq!(sunsoft.psg.freq[0], 0x02FD);

        sunsoft.write_noise_period(0xFF);
        assert_eq!(sunsoft.psg.noise_freq, 0x1F);

        sunsoft.write_volume_level(31);
        assert_eq!(sunsoft.psg.volume_level(0), 31);
        sunsoft.write_volume_level(16);
        assert_eq!(sunsoft.psg.volume_level(0), 16);
    }

    #[test]
    fn direct_volume_level_reaches_table_entries_the_register_cannot() {
        // A $08 write is 4-bit and stored as `(val << 1) | 1`, so it can only
        // land on odd table indices; the even ones are reachable only through
        // the envelope generator, or through `set_volume_level`.
        let mut register = Psg::new();
        for v in 0..16u32 {
            register.write_reg(8, v);
            assert_eq!(register.volume_level(0) % 2, 1, "register path is odd-only");
        }

        let mut direct = Psg::new();
        for level in 0..32u8 {
            direct.set_volume_level(0, level);
            assert_eq!(direct.volume_level(0), level);
        }

        // The two agree wherever they overlap: `(v << 1) | 1` is the packing.
        for v in 0..16u32 {
            register.write_reg(8, v);
            direct.set_volume_level(0, ((v as u8) << 1) | 1);
            assert_eq!(register.volume_level(0), direct.volume_level(0));
        }
    }

    #[test]
    fn set_tone_noise_enable_only_touches_channel_zero_bits() {
        let mut sunsoft = Sunsoft::new();
        sunsoft.set_tone_noise_enable(true, false);
        assert!(!sunsoft.psg.tone_disable[0]);
        assert!(sunsoft.psg.noise_disable[0]);
        // Channels 1/2 always stay disabled — nothing drives them.
        assert!(sunsoft.psg.tone_disable[1]);
        assert!(sunsoft.psg.tone_disable[2]);
        assert!(sunsoft.psg.noise_disable[1]);
        assert!(sunsoft.psg.noise_disable[2]);

        sunsoft.set_tone_noise_enable(false, true);
        assert!(sunsoft.psg.tone_disable[0]);
        assert!(!sunsoft.psg.noise_disable[0]);
        assert!(sunsoft.psg.tone_disable[1]);
        assert!(sunsoft.psg.tone_disable[2]);
        assert!(sunsoft.psg.noise_disable[1]);
        assert!(sunsoft.psg.noise_disable[2]);
    }

    #[test]
    fn tone_and_noise_flags_gate_their_own_source() {
        // Tone enabled, noise disabled: the slowest possible noise period must
        // not stall the square. (With the mixer's disable bits negated, this
        // channel would be gated by the noise LFSR instead.)
        let mut tone = Sunsoft::new();
        tone.write_timer_lo(8);
        tone.write_timer_hi(0);
        tone.write_noise_period(0x1F);
        tone.write_volume_level(31);
        tone.set_tone_noise_enable(true, false);

        let mut toggled = false;
        for _ in 0..512 {
            tone.clock();
            if tone.output() != 0 {
                toggled = true;
                break;
            }
        }
        assert!(toggled, "tone-only must follow the tone edge");

        // Noise enabled, tone disabled, tone period parked at its maximum so
        // the square edge cannot toggle inside the window — any output at all
        // therefore comes from the LFSR.
        let mut noise = Sunsoft::new();
        noise.write_timer_lo(0xFF);
        noise.write_timer_hi(0x0F);
        noise.write_noise_period(1);
        noise.write_volume_level(31);
        noise.set_tone_noise_enable(false, true);

        let mut changed = false;
        for _ in 0..4096 {
            noise.clock();
            if noise.output() != 0 {
                changed = true;
                break;
            }
        }
        assert!(changed, "noise-only must follow the noise LFSR");
    }

    #[test]
    fn one_psg_step_is_sixteen_cpu_cycles() {
        let mut sunsoft = Sunsoft::new();
        sunsoft.write_timer_lo(8); // TP = 8
        sunsoft.write_timer_hi(0);
        sunsoft.write_volume_level(31);
        sunsoft.set_tone_noise_enable(true, false);

        // TP=8 toggles the square edge every 8 internal steps. At 16 CPU
        // cycles per step that is one toggle per 128 CPU cycles, i.e. a
        // 256-cycle wave — `f = clk / (32 * TP)`.
        let mut transitions = 0;
        let mut prev = sunsoft.output();
        for _ in 0..1024 {
            sunsoft.clock();
            let now = sunsoft.output();
            if now != prev {
                transitions += 1;
            }
            prev = now;
        }
        assert_eq!(transitions, 1024 / 128);
    }

    #[test]
    fn duty_index_defaults_to_fifty_percent() {
        let sunsoft = Sunsoft::new();
        assert_eq!(sunsoft.psg().duty_index(0), 4);
    }

    #[test]
    fn duty_presets_produce_expected_mark_space_ratio() {
        // One preset per table entry; measures the fraction of clocks the
        // tone output is high over several full waves and checks it lands
        // near that preset's documented percentage (`DUTY_CYCLE_TABLE`'s
        // comment in the source). Generous tolerance: this counts raw
        // clocks, not phase samples, so period/incr rounding shifts the
        // measured edge by up to a couple of percent either way.
        const EXPECTED_PERCENT: [f64; 9] = [
            3.125, 6.25, 12.5, 25.0, 50.0, 75.0, 87.5, 93.75, 96.875,
        ];

        for (index, &expected) in EXPECTED_PERCENT.iter().enumerate() {
            let mut sunsoft = Sunsoft::new();
            sunsoft.write_timer_lo(64);
            sunsoft.write_timer_hi(0);
            sunsoft.write_volume_level(31);
            sunsoft.write_duty_index(index as u8);
            sunsoft.set_tone_noise_enable(true, false);

            let mut high = 0u32;
            let mut total = 0u32;
            // A few full waves' worth of CPU-cycle-granularity clocks
            // (16 clock() calls per internal step, TP=64 -> one full wave
            // is `2*64*16` clocks).
            let wave_clocks = 2 * 64 * 16;
            for _ in 0..(wave_clocks * 4) {
                sunsoft.clock();
                if sunsoft.output() != 0 {
                    high += 1;
                }
                total += 1;
            }

            let measured_percent = f64::from(high) / f64::from(total) * 100.0;
            assert!(
                (measured_percent - expected).abs() < 4.0,
                "duty index {index}: expected ~{expected}%, measured {measured_percent}%"
            );
        }
    }

    #[test]
    fn set_duty_index_clamps_to_table_range() {
        let mut psg = Psg::new();
        psg.set_duty_index(0, 255);
        assert_eq!(psg.duty_index(0), 8);
    }

    #[test]
    fn reset_clears_wrapper_and_psg_state() {
        let mut sunsoft = Sunsoft::new();
        sunsoft.write_register(REG_SELECT_BASE, 0);
        sunsoft.write_register(REG_WRITE_BASE, 0xFF);
        sunsoft.enable_channel(0, false);

        sunsoft.reset();

        assert_eq!(sunsoft.psg.reg(0), 0);
        assert_eq!(sunsoft.psg.mask(), 0);
        assert_eq!(sunsoft.output(), 0);
    }
}
