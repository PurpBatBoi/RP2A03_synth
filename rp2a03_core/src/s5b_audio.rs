//! `rp2a03_core\src\s5b_audio.rs`

/// Which logarithmic volume table the PSG's 5-bit volume registers index
/// into — the two real silicon variants disagree on its curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VolumeMode {
    /// YM2149's volume table (the common Sunsoft 5B clone).
    #[default]
    Ym2149,

    /// AY-3-8910's volume table.
    Ay8910,
}

impl VolumeMode {
    pub(crate) fn table(self) -> &'static [u32; 32] {
        match self {
            Self::Ym2149 => &VOLTBL_YM2149,
            Self::Ay8910 => &VOLTBL_AY8910,
        }
    }
}

pub(crate) const VOLTBL_YM2149: [u32; 32] = [
    0x00, 0x00, 0x01, 0x01, 0x02, 0x02, 0x03, 0x03, 0x04, 0x05, 0x06, 0x07, 0x09, 0x0B, 0x0D, 0x0F,
    0x12, 0x16, 0x1A, 0x1F, 0x25, 0x2D, 0x35, 0x3F, 0x4C, 0x5A, 0x6A, 0x7F, 0x97, 0xB4, 0xD6, 0xFF,
];

pub(crate) const VOLTBL_AY8910: [u32; 32] = [
    0x00, 0x00, 0x03, 0x03, 0x04, 0x04, 0x06, 0x06, 0x09, 0x09, 0x0D, 0x0D, 0x12, 0x12, 0x1D, 0x1D,
    0x22, 0x22, 0x37, 0x37, 0x4D, 0x4D, 0x62, 0x62, 0x82, 0x82, 0xA6, 0xA6, 0xD0, 0xD0, 0xFF, 0xFF,
];

const REG_MASK: [u8; 16] = [
    0xff, 0x0f, 0xff, 0x0f, 0xff, 0x0f, 0x1f, 0x3f, 0x1f, 0x1f, 0x1f, 0xff, 0xff, 0x0f, 0xff, 0xff,
];

const DUTY_CYCLE_TABLE: [u32; 9] = [
    0x8000_0000,
    0xc000_0000,
    0xf000_0000,
    0xff00_0000,
    0xffff_0000,
    0xffff_ff00,
    0xffff_fff0,
    0xffff_fffc,
    0xffff_fffe,
];

/// A 3-channel AY-3-8910/YM2149-compatible PSG core, driven one channel
/// (channel 0) at a time via `clock_channel0` — see the type's callers for why.
#[derive(Debug, Clone)]
pub struct Psg {
    pub(crate) voltbl: VolumeMode,

    reg: [u8; 16],

    pub(crate) freq: [u16; 3],
    count: [u16; 3],

    edge: [bool; 3],

    duty_index: [u8; 3],

    volume: [u8; 3],

    pub(crate) tone_disable: [bool; 3],
    pub(crate) noise_disable: [bool; 3],

    ch_out: [i16; 3],

    pub(crate) noise_freq: u8,
    noise_count: u8,
    noise_scaler: bool,
    pub(crate) noise_seed: u32,

    mask: u32,

    base_count: u32,
    base_incr: u32,

    tone_dirty: bool,
}

pub(crate) const GETA_BITS: u32 = 24;

impl Default for Psg {
    fn default() -> Self {
        Self::new()
    }
}

impl Psg {
    /// A silent PSG at the default 1/16 step increment.
    #[must_use]
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
            mask: 0,
            base_count: 0,
            base_incr: 1 << GETA_BITS,
            tone_dirty: false,
        }
    }

    /// Selects the YM2149 or AY-3-8910 volume-table variant.
    pub fn set_volume_mode(&mut self, mode: VolumeMode) {
        self.voltbl = mode;
    }

    /// Sets how far the fixed-point phase accumulator advances per `clock_channel0`.
    pub fn set_step_increment(&mut self, incr: u32) {
        self.base_incr = incr;
    }

    /// Mutes (or unmutes) channel `idx` independent of its tone/noise/volume registers.
    pub fn set_channel_mask(&mut self, idx: usize, muted: bool) {
        if muted {
            self.mask |= 1 << idx;
        } else {
            self.mask &= !(1 << idx);
        }
    }

    /// Reads back a raw PSG register (`$00`-`$0F`), already register-masked.
    #[must_use]
    pub fn reg(&self, reg: usize) -> u8 {
        self.reg[reg & 0x0f]
    }

    /// Sets channel `idx`'s 5-bit volume-table index directly.
    pub fn set_volume_level(&mut self, idx: usize, level: u8) {
        self.volume[idx] = level & 0x1f;
    }

    /// Sets channel `idx`'s duty-cycle table index (an extension beyond real
    /// PSG hardware, which is always 50% duty — see `DUTY_CYCLE_TABLE`).
    pub fn set_duty_index(&mut self, idx: usize, duty_index: u8) {
        self.duty_index[idx] = duty_index.min(8);
        self.tone_dirty = true;
    }

    /// The volume-table index whose level best approximates the linear ratio `num/den`.
    #[must_use]
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

    /// Sum of all three channels' last-computed output.
    #[must_use]
    pub fn output(&self) -> i16 {
        self.ch_out[0] + self.ch_out[1] + self.ch_out[2]
    }

    /// Raw register write (`$00`-`$0F`), register-masked and decoded into
    /// the live tone/noise/volume/mixer state it drives.
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
                self.volume[reg - 8] = (val << 1) | 1;
            }

            _ => {}
        }
    }

    /// Advances channel 0 (tone + shared noise generator) one step.
    pub fn clock_channel0(&mut self) {
        let incr = self.take_step_increment();
        let noise = self.clock_shared(incr);

        if incr == 0 && !self.tone_dirty {
            self.update_ch_out(0, noise);
            return;
        }

        self.clock_tone(0, incr, noise);
        self.tone_dirty = false;
    }

    #[inline]
    fn take_step_increment(&mut self) -> u32 {
        self.base_count += self.base_incr;
        let incr = self.base_count >> GETA_BITS;
        self.base_count &= (1 << GETA_BITS) - 1;
        incr
    }

    #[inline]
    fn clock_shared(&mut self, incr: u32) -> bool {
        self.clock_noise(incr);
        self.noise_seed & 1 == 0
    }

    #[inline]
    fn clock_noise(&mut self, incr: u32) {
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
    }

    #[inline]
    fn clock_tone(&mut self, i: usize, incr: u32, noise: bool) {
        let full_cycle = u32::from(self.freq[i]).saturating_mul(2).max(1);
        self.count[i] = ((u32::from(self.count[i]) + incr) % full_cycle) as u16;
        let phase = ((u32::from(self.count[i]) * 32) / full_cycle) as u8 & 0x1f;

        let pattern = DUTY_CYCLE_TABLE[usize::from(self.duty_index[i])];
        let new_edge = pattern & (1 << phase) != 0;
        self.edge[i] = new_edge;

        self.update_ch_out(i, noise);
    }

    #[inline]
    fn update_ch_out(&mut self, i: usize, noise: bool) {
        self.ch_out[i] = if self.mask & (1 << i) != 0 {
            0
        } else if (self.tone_disable[i] || self.edge[i]) && (self.noise_disable[i] || noise) {
            let table = self.voltbl.table();
            (table[usize::from(self.volume[i] & 0x1f)] as i16) << 4
        } else {
            0
        };
    }

    /// Resets to power-on state, preserving `voltbl` and `base_incr` — the
    /// two host-configured settings, as opposed to per-note register state.
    pub fn reset(&mut self) {
        let voltbl = self.voltbl;
        let base_incr = self.base_incr;
        *self = Self::new();
        self.voltbl = voltbl;
        self.base_incr = base_incr;
    }
}

/// The Sunsoft 5B expansion chip: one PSG channel, addressed the same way
/// the 2A03's own channels are (`write_timer_lo`/`write_timer_hi`/...).
#[derive(Debug, Clone)]
pub struct Sunsoft {
    pub(crate) psg: Psg,
}

impl Default for Sunsoft {
    fn default() -> Self {
        Self::new()
    }
}

impl Sunsoft {
    /// A silent Sunsoft 5B channel.
    #[must_use]
    pub fn new() -> Self {
        let mut sunsoft = Self { psg: Psg::new() };
        sunsoft.reset();

        sunsoft.psg.set_step_increment((1 << GETA_BITS) / 16);
        sunsoft
    }

    /// The underlying PSG core, for reads the 5B's own API doesn't cover.
    #[must_use]
    pub fn psg(&self) -> &Psg {
        &self.psg
    }

    /// Advances the PSG one step.
    pub fn clock(&mut self) {
        self.psg.clock_channel0();
    }

    /// The PSG's last-computed output.
    #[must_use]
    pub fn output(&self) -> i16 {
        self.psg.output()
    }

    /// Tone period low byte (PSG register `$00`).
    pub fn write_timer_lo(&mut self, val: u8) {
        self.psg.write_reg(0, u32::from(val));
    }

    /// Tone period high nibble (PSG register `$01`).
    pub fn write_timer_hi(&mut self, val: u8) {
        self.psg.write_reg(1, u32::from(val));
    }

    /// Sets the 5-bit volume level directly (PSG volume register).
    pub fn write_volume_level(&mut self, level: u8) {
        self.psg.set_volume_level(0, level);
    }

    /// Sets the duty-cycle table index (an extension beyond real PSG hardware).
    pub fn write_duty_index(&mut self, duty_index: u8) {
        self.psg.set_duty_index(0, duty_index);
    }

    /// Noise period (PSG register `$06`).
    pub fn write_noise_period(&mut self, period: u8) {
        self.psg.write_reg(6, u32::from(period & 0x1F));
    }

    /// Sets the tone/noise enable bits in the PSG mixer register (`$07`),
    /// preserving the other channels' and the I/O-port bits already there.
    pub fn set_tone_noise_enable(&mut self, tone: bool, noise: bool) {
        let mut val = self.psg.reg(7) | 0b0011_0110;
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

    /// Resets to power-on state.
    pub fn reset(&mut self) {
        self.psg.reset();
    }
}
