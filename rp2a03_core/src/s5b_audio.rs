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

    /// Tone period, current down-counter, and square edge, per channel.
    freq: [u16; 3],
    count: [u16; 3],
    edge: [bool; 3],
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
            0 | 1 | 2 | 3 | 4 | 5 => {
                let c = reg >> 1;
                self.freq[c] =
                    (u16::from(self.reg[c * 2 + 1] & 0x0f) << 8) | u16::from(self.reg[c * 2]);
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
            8 | 9 | 10 => {
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

        self.base_count += self.base_incr;
        let incr = self.base_count >> GETA_BITS;
        self.base_count &= (1 << GETA_BITS) - 1;

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
        let noise = self.noise_seed & 1 == 0;

        let table = self.voltbl.table();

        for i in 0..3 {
            let mut tone_trigger = false;

            self.count[i] = self.count[i].wrapping_add(incr as u16);
            if self.count[i] >= self.freq[i] {
                self.edge[i] = !self.edge[i];

                if self.freq[i] >= incr as u16 {
                    self.count[i] -= self.freq[i];
                } else {
                    self.count[i] = 0;
                }

                if self.edge[i] {
                    tone_trigger = true;
                }
            }

            if self.mask & (1 << i) != 0 {
                self.ch_out[i] = 0;
            } else if (!self.tone_disable[i] || self.edge[i])
                && (!self.noise_disable[i] || noise)
            {
                self.ch_out[i] = if self.volume[i] & 0x20 == 0 {
                    (table[usize::from(self.volume[i] & 0x1f)] as i16) << 4
                } else {
                    (table[usize::from(self.envelope.ptr)] as i16) << 4
                };
            } else {
                self.ch_out[i] = 0;
            }

            // If this channel is gated by a repeating (non-hold) envelope,
            // report the envelope's trigger; otherwise report the tone
            // edge, provided the tone isn't disabled.
            if self.envelope.is_fast_repeating() && self.volume[i] & 0x20 != 0 {
                self.trigger_mask |= (0x08 << i) | (u8::from(env_trigger) << i);
            } else if !self.tone_disable[i] && self.freq[i] != 0 {
                self.trigger_mask |= (0x08 << i) | (u8::from(tone_trigger) << i);
            }
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
        sunsoft
    }

    /// Shared PSG core, for direct access to per-sample output/trigger
    /// state.
    pub fn psg(&self) -> &Psg {
        &self.psg
    }

    /// Advances the PSG by one internal step. See [`Psg::clock`].
    pub fn clock(&mut self) {
        self.psg.clock();
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

    /// Channel 0 volume/envelope-select (reg 8): `vol` is the 4-bit constant
    /// volume, `envelope_enabled` selects the shared envelope generator
    /// instead (bit 4).
    pub fn write_volume_envelope(&mut self, vol: u8, envelope_enabled: bool) {
        let val = (vol & 0x0F) | if envelope_enabled { 0x10 } else { 0 };
        self.psg.write_reg(8, u32::from(val));
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
        } else if addr >= u32::from(REG_WRITE_BASE) && addr < u32::from(REG_WRITE_BASE) + u32::from(REG_RANGE) {
            if let Some(slot) = self.shadow_regs.get_mut(self.selected_reg as usize & 0x0f) {
                *slot = Some(data);
            }
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

        sunsoft.write_volume_envelope(0x0F, false);
        assert_eq!(sunsoft.psg.volume[0], (0x0F << 1) | 1);
        sunsoft.write_volume_envelope(0x00, true);
        assert_eq!(sunsoft.psg.volume[0], (0x10 << 1) | 1);
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
