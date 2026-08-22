//! `rp2a03_core\src\fds_audio.rs`

/// One FDS hardware envelope ($4080 volume, or the modulator's $4084 depth):
/// a speed-scaled up/down gain counter, or a direct gain set in constant mode.
#[derive(Debug, Clone, Default)]
pub struct FdsEnvelope {
    /// 6-bit reload value for `counter`; smaller is faster.
    pub speed: u8,

    /// Current envelope level, 0..=32.
    pub gain: u8,

    /// When set, `gain` tracks `speed` directly every tick instead of ramping.
    pub constant_mode: bool,

    /// Direction the envelope ramps: true toward 32, false toward 0.
    pub increase: bool,
    /// Master-speed-scaled cycles remaining until the next gain step.
    pub counter: u32,
}

impl FdsEnvelope {
    /// A silent, zeroed envelope.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `$4080`/`$4084`: speed (D5..D0), direction (D6), constant mode (D7).
    pub fn write_reg(&mut self, value: u8) {
        self.speed = value & 0x3F;
        self.increase = value & 0x40 != 0;
        self.constant_mode = value & 0x80 != 0;
    }

    /// Advances one master clock. Returns true on a gain step (constant mode
    /// never reports a step, since it has no timing of its own).
    pub fn tick(&mut self, master_speed: u8, envelopes_disabled: bool) -> bool {
        if self.constant_mode {
            self.gain = self.speed;
            return false;
        }
        if envelopes_disabled || master_speed == 0 {
            return false;
        }
        if self.counter != 0 {
            self.counter -= 1;
            return false;
        }
        self.counter = u32::from(master_speed) * 8 * (u32::from(self.speed) + 1);
        if self.increase {
            if self.gain < 32 {
                self.gain += 1;
            }
        } else if self.gain != 0 {
            self.gain -= 1;
        }
        true
    }
}

const MOD_LUT: [i8; 8] = [0, 1, 2, 3, -4, -3, -2, -1];

/// The FDS pitch modulator: a 64-step wavetable ($4088) whose entries nudge a
/// running `bias`, which an envelope-scaled multiply turns into a frequency
/// delta applied to the wave generator.
#[derive(Debug, Clone)]
pub struct FdsModulator {
    /// The `$4084` depth envelope; its gain scales `bias` into `output`.
    pub envelope: FdsEnvelope,

    /// The modulation table, stored doubled (each `$4088` write shifts in one
    /// value at two adjacent slots — see `write_table`).
    pub table: [i8; 64],

    /// Current read position into `table`.
    pub index: u8,

    /// 12-bit modulator frequency (`$4086`/`$4087`), phase-accumulated by `counter`.
    pub frequency: u16,

    /// Phase accumulator driven by `frequency`; a table step fires on underflow.
    pub counter: i32,

    /// Accumulated pitch-modulation bias, updated by each table step.
    pub bias: i8,

    /// `$4087` bit 7: disables the modulator entirely.
    pub disabled: bool,

    /// Last computed frequency delta, added to the wave generator's frequency.
    pub output: i32,
}

impl Default for FdsModulator {
    fn default() -> Self {
        Self::new()
    }
}

impl FdsModulator {
    /// A disabled, zeroed modulator.
    #[must_use]
    pub fn new() -> Self {
        Self {
            envelope: FdsEnvelope::new(),
            table: [0; 64],
            index: 0,
            frequency: 0,
            counter: 0,
            bias: 0,
            disabled: false,
            output: 0,
        }
    }

    /// `$4084`-`$4087`: depth envelope, bias reset, frequency low/high + disable.
    pub fn write_reg(&mut self, addr: u16, value: u8) {
        match addr {
            0x4084 => self.envelope.write_reg(value),
            0x4085 => {
                self.bias = ((value << 1) as i8) / 2;
                self.index = 0;
            }
            0x4086 => self.frequency = (self.frequency & 0xFF00) | u16::from(value),
            0x4087 => {
                self.frequency = ((u16::from(value) & 0x0F) << 8) | (self.frequency & 0x00FF);
                self.disabled = value & 0x80 != 0;
            }
            _ => {}
        }
    }

    /// `$4088`: shifts one 3-bit modulation value into the table (each write
    /// advances by one logical step; stored doubled — see the `table` field).
    pub fn write_table(&mut self, value: u8) {
        for i in 0..32 {
            let a = i << 1;
            self.table[a] = if i < 31 {
                self.table[a + 2]
            } else {
                MOD_LUT[(value & 0x07) as usize]
            };
            self.table[a + 1] = self.table[a];
        }
    }

    /// Advances the phase accumulator. Returns true and steps `bias` by the
    /// current table entry on underflow (a table value of -4 resets `bias` to 0
    /// instead of accumulating, per hardware).
    pub fn tick(&mut self) -> bool {
        if self.disabled || self.frequency == 0 {
            return false;
        }
        self.counter -= i32::from(self.frequency);
        if self.counter >= 0 {
            return false;
        }
        self.counter += 65536;
        let adj = self.table[self.index as usize];
        self.index = (self.index + 1) % 64;
        if adj == -4 {
            self.bias = 0;
        } else {
            self.bias = self.bias.wrapping_add(adj);
        }
        true
    }

    /// Recomputes `output` (the frequency delta applied to the wave
    /// generator) from the current `bias` and envelope gain.
    pub fn update_output(&mut self, wave_frequency: i32) {
        let temp = i32::from(self.bias) * i32::from(self.envelope.gain.min(32));

        let mut a = 64i32;
        let mut d = 0i32;

        if temp <= 0 {
            d = 15;
        } else if temp < 3040 {
            a = 66;
            d = -31;
        }

        let temp2 = a + i32::from(((temp - d) / 16 - a) as i8);

        self.output = wave_frequency * temp2 / 64;
    }
}

const VOL_TABLE: [i32; 4] = [39, 26, 19, 15];

/// The Famicom Disk System's wavetable channel: an arbitrary 64-step
/// user-defined waveform, its own volume envelope, and a pitch modulator.
// Each bool is an independent hardware register flag (see their doc
// comments), not a disguised state machine — an enum wouldn't fit.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct Fds {
    /// `$4023` bit 1: gates every other FDS register write while clear.
    pub io_enabled: bool,

    /// The 64-step, 6-bit wavetable ($4040-$407F).
    pub wave: [u8; 64],

    /// `$4089` bit 7: while set, the wavetable is CPU-writable and playback halts.
    pub wave_write_enabled: bool,

    /// `$4089` bits 0-1: index into `VOL_TABLE`, the coarse output gain.
    pub master_volume: u8,

    /// The `$4080` volume envelope.
    pub volume: FdsEnvelope,

    /// The pitch modulator (its own envelope, table, and phase state).
    pub modulator: FdsModulator,

    /// `$4083` bit 6: halts both the volume and modulator envelopes' timing.
    pub envelopes_disabled: bool,

    /// `$408A`: global envelope speed multiplier shared by both envelopes.
    pub master_env_speed: u8,

    /// `$4083` bit 7: mutes wave output entirely (independent of the envelope).
    pub silence: bool,

    /// 12-bit wave frequency (`$4082`/`$4083`).
    pub frequency: u16,
    /// Phase accumulator driving wavetable playback, stepped by `frequency`.
    pub wave_counter: i32,
    /// Current read position into `wave`.
    pub wave_index: u8,

    output: i32,
}

impl Default for Fds {
    fn default() -> Self {
        Self::new()
    }
}

impl Fds {
    /// A silenced FDS channel with I/O enabled and playback halted, matching
    /// the power-on `$4023` sequence real hardware/software goes through.
    #[must_use]
    pub fn new() -> Self {
        let mut fds = Self {
            io_enabled: false,
            wave: [0; 64],
            wave_write_enabled: false,
            master_volume: 0,
            volume: FdsEnvelope::new(),
            modulator: FdsModulator::new(),
            envelopes_disabled: false,
            master_env_speed: 0,
            silence: false,
            frequency: 0,
            wave_counter: 0,
            wave_index: 0,
            output: 0,
        };
        fds.write_reg(0x4023, 0x00);
        fds.write_reg(0x4023, 0x83);
        fds
    }

    /// Resets to power-on state.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Raw register write, gated on `io_enabled` (except `$4023` itself, which sets it).
    pub fn write_reg(&mut self, addr: u16, value: u8) {
        if addr == 0x4023 {
            self.io_enabled = value & 0x02 != 0;
        }
        if !self.io_enabled || !(0x4040..=0x408A).contains(&addr) {
            return;
        }
        match addr {
            0x4040..=0x407F => self.wave[(addr & 0x3F) as usize] = value & 0x3F,
            0x4080 => self.volume.write_reg(value),
            0x4082 => self.frequency = (self.frequency & 0xFF00) | u16::from(value),
            0x4083 => {
                self.frequency = ((u16::from(value) & 0x0F) << 8) | (self.frequency & 0x00FF);
                self.envelopes_disabled = value & 0x40 != 0;
                self.silence = value & 0x80 != 0;
            }
            0x4084..=0x4087 => self.modulator.write_reg(addr, value),
            0x4088 => self.modulator.write_table(value),
            0x4089 => {
                self.wave_write_enabled = value & 0x80 != 0;
                self.master_volume = value & 0x03;
            }
            0x408A => self.master_env_speed = value,

            _ => {}
        }
    }

    /// Advances both envelopes, the modulator, and (unless silenced or in
    /// write mode) the wavetable playhead by one clock.
    pub fn clock(&mut self) {
        self.volume
            .tick(self.master_env_speed, self.envelopes_disabled);
        self.modulator
            .envelope
            .tick(self.master_env_speed, self.envelopes_disabled);

        let mut freq = i32::from(self.frequency);

        if !self.modulator.disabled && self.modulator.frequency != 0 {
            if self.modulator.tick() {
                self.modulator.update_output(freq);
            }
            if freq != 0 {
                freq += self.modulator.output;
            }
        }

        if self.silence {
            self.output = 0;
            return;
        }

        if freq != 0 && !self.wave_write_enabled {
            self.wave_counter -= freq;
            if self.wave_counter < 0 {
                self.wave_counter += 65536;
                let level =
                    i32::from(self.volume.gain.min(32)) * VOL_TABLE[self.master_volume as usize];
                self.output = (i32::from(self.wave[self.wave_index as usize]) * level) >> 3;
                self.wave_index = (self.wave_index + 1) % 64;
            }
        }
    }

    /// The current sample, already gain- and envelope-scaled.
    #[must_use]
    pub fn output(&self) -> i32 {
        self.output
    }

    /// `$4082`: wave frequency low byte.
    pub fn write_freq_lo(&mut self, value: u8) {
        self.write_reg(0x4082, value);
    }

    /// `$4083`: wave frequency high nibble, plus its halt/silence bits.
    pub fn write_freq_hi(&mut self, value: u8) {
        self.write_reg(0x4083, value);
    }

    /// Updates only the frequency register's high nibble, preserving the
    /// current halt/silence bits rather than the raw value `write_freq_hi` takes.
    pub fn set_period_hi_soft(&mut self, bits: u8) {
        self.write_reg(0x4083, (bits & 0x0F) | self.halt_bits());
    }

    /// Enable or disable playback via `$4083`'s silence bit (D7), preserving
    /// frequency and the envelope-halt bit.
    pub fn set_enabled(&mut self, enabled: bool) {
        let d7 = if enabled { 0x00 } else { 0x80 };
        let d6 = self.halt_bits() & 0x40;
        self.write_reg(0x4083, ((self.frequency >> 8) as u8 & 0x0F) | d6 | d7);
    }

    /// The current envelope-halt (D6) and silence (D7) bits, as they appear
    /// packed into `$4083`.
    fn halt_bits(&self) -> u8 {
        (if self.envelopes_disabled { 0x40 } else { 0 }) | (if self.silence { 0x80 } else { 0 })
    }

    /// Sets the volume envelope to constant mode at `gain` (`$4080`).
    pub fn set_volume(&mut self, gain: u8) {
        self.write_reg(0x4080, 0x80 | (gain & 0x3F));
    }

    /// Loads a new 64-step wavetable, entering and leaving write mode around it.
    pub fn load_wave(&mut self, wave: &[u8; 64]) {
        self.write_reg(0x4089, 0x80 | self.master_volume);
        for (i, &v) in wave.iter().enumerate() {
            self.write_reg(0x4040 + i as u16, v);
        }
        self.write_reg(0x4089, self.master_volume);
    }

    /// Sets the modulator's depth envelope to constant mode at `depth` (`$4084`).
    pub fn set_mod_depth(&mut self, depth: u8) {
        self.write_reg(0x4084, 0x80 | (depth & 0x3F));
    }

    /// Sets the modulator's 12-bit frequency (`$4086`/`$4087`), preserving its enabled state.
    pub fn set_mod_speed(&mut self, speed: u16) {
        self.write_reg(0x4086, (speed & 0xFF) as u8);
        self.write_reg(0x4087, ((speed >> 8) & 0x0F) as u8);
    }

    /// Disables the pitch modulator (`$4087` bit 7).
    pub fn disable_modulator(&mut self) {
        self.write_reg(0x4087, 0x80);
    }

    /// Loads a new 32-value modulation table, restoring the modulator's
    /// enabled state and frequency around the write sequence `$4088` needs.
    pub fn load_mod_table(&mut self, table: &[i8; 32]) {
        let speed_hi = (self.modulator.frequency >> 8) as u8 & 0x0F;
        let disable_bit = if self.modulator.disabled { 0x80 } else { 0x00 };
        self.write_reg(0x4087, 0x80 | speed_hi);
        for &v in table {
            self.write_reg(0x4088, (v as u8) & 0x07);
        }
        self.write_reg(0x4087, disable_bit | speed_hi);
    }
}
