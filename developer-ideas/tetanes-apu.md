"apu\dmc.rs"
```rust
//! APU DMC (Delta Modulation Channel) implementation.
//!
//! See: <https://www.nesdev.org/wiki/APU_DMC>

use crate::{
    apu::timer::{Timer, TimerCycle},
    common::{Clock, NesRegion, Regional, Reset, ResetKind, Sample},
};
use serde::{Deserialize, Serialize};
use tracing::trace;

/// APU DMC (Delta Modulation Channel) provides sample playback.
///
/// See: <https://www.nesdev.org/wiki/APU_DMC>
#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct Dmc {
    pub region: NesRegion,
    pub timer: Timer,
    pub force_silent: bool,
    pub irq_enabled: bool,
    pub irq_pending: bool,
    pub dma_pending: bool,
    pub loops: bool,
    pub addr: u16,
    pub sample_addr: u16,
    pub bytes_remaining: u16,
    pub sample_length: u16,
    pub sample_buffer: u8,
    pub buffer_empty: bool,
    pub init: u8,
    pub output_level: u8,
    pub bits_remaining: u8,
    pub shift: u8,
    pub silence: bool,
    pub should_clock: bool,
}

impl Default for Dmc {
    fn default() -> Self {
        Self::new(NesRegion::default())
    }
}

impl Dmc {
    const PERIOD_TABLE_NTSC: [u16; 16] = [
        428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54,
    ];
    const PERIOD_TABLE_PAL: [u16; 16] = [
        398, 354, 316, 298, 276, 236, 210, 198, 176, 148, 132, 118, 98, 78, 66, 50,
    ];

    pub const fn new(region: NesRegion) -> Self {
        Self {
            region,
            timer: Timer::preload(Self::period(region, 0)),
            force_silent: false,
            irq_enabled: false,
            irq_pending: false,
            dma_pending: false,
            loops: false,
            addr: 0xC000,
            sample_addr: 0x0000,
            bytes_remaining: 0x0000,
            sample_length: 0x0001,
            sample_buffer: 0x00,
            buffer_empty: true,
            init: 0,
            output_level: 0x00,
            bits_remaining: 0x08,
            shift: 0x00,
            silence: true,
            should_clock: false,
        }
    }

    #[must_use]
    pub const fn silent(&self) -> bool {
        self.force_silent
    }

    pub const fn set_silent(&mut self, silent: bool) {
        self.force_silent = silent;
    }

    #[cold]
    #[must_use]
    pub fn irq_pending_in(&self, cycles_to_run: u32) -> bool {
        if self.irq_enabled && self.bytes_remaining > 0 {
            let cycles_to_empty = (u16::from(self.bits_remaining) + (self.bytes_remaining - 1) * 8)
                * self.timer.period;
            cycles_to_run >= u32::from(cycles_to_empty)
        } else {
            false
        }
    }

    #[must_use]
    pub const fn dma_addr(&self) -> u16 {
        self.addr
    }

    fn init_sample(&mut self) {
        self.addr = self.sample_addr;
        self.bytes_remaining = self.sample_length;
        trace!(
            "APU DMC sample started. bytes remaining: {}",
            self.bytes_remaining
        );
        self.should_clock = self.bytes_remaining > 0;
    }

    /// Load a sample into the DMC buffer - returns `true` if an IRQ is triggered.
    pub fn load_buffer(&mut self, val: u8) {
        if self.bytes_remaining > 0 {
            self.sample_buffer = val;
            self.buffer_empty = false;
            if self.addr == 0xFFFF {
                self.addr = 0x8000;
            } else {
                self.addr += 1;
            }
            self.bytes_remaining -= 1;
            trace!("APU DMC bytes remaining: {}", self.bytes_remaining);
            if self.bytes_remaining == 0 {
                self.should_clock = false;
                if self.loops {
                    self.init_sample();
                } else if self.irq_enabled {
                    self.irq_pending = true;
                }
            }
        }
    }

    const fn period(region: NesRegion, val: u8) -> u16 {
        let index = (val & 0x0F) as usize;
        match region {
            NesRegion::Auto | NesRegion::Ntsc | NesRegion::Dendy => {
                Self::PERIOD_TABLE_NTSC[index] - 1
            }
            NesRegion::Pal => Self::PERIOD_TABLE_PAL[index] - 1,
        }
    }

    /// $4010 DMC timer
    pub const fn write_timer(&mut self, val: u8) {
        self.irq_enabled = val & 0x80 == 0x80;
        self.loops = val & 0x40 == 0x40;
        self.timer.period = Self::period(self.region, val);
        if !self.irq_enabled {
            self.irq_pending = false;
        }
    }

    /// $4011 DMC output
    pub const fn write_output(&mut self, val: u8) {
        self.output_level = val & 0x7F;
    }

    /// $4012 DMC addr load
    pub fn write_addr(&mut self, val: u8) {
        self.sample_addr = 0xC000 | (u16::from(val) << 6);
    }

    /// $4013 DMC length
    pub fn write_length(&mut self, val: u8) {
        self.sample_length = (u16::from(val) << 4) | 1;
    }

    /// $4015 WRITE
    pub fn set_enabled(&mut self, enabled: bool, cycle: u32) {
        if !enabled {
            self.bytes_remaining = 0;
            self.should_clock = false;
        } else if self.bytes_remaining == 0 {
            self.init_sample();
            // Delay a number of cycles based on even/odd cycle
            self.init = if cycle & 0x01 == 0x00 { 2 } else { 3 };
        }
    }

    #[inline(always)]
    pub fn should_clock(&mut self) -> bool {
        if self.init > 0 {
            self.init -= 1;
            if self.init == 0 && self.buffer_empty && self.bytes_remaining > 0 {
                trace!("APU DMC DMA pending");
                self.dma_pending = true;
            }
        }
        self.should_clock
    }
}

impl Sample for Dmc {
    fn output(&self) -> f32 {
        if self.silent() {
            0.0
        } else {
            f32::from(self.output_level)
        }
    }
}

impl TimerCycle for Dmc {
    fn cycle(&self) -> u32 {
        self.timer.cycle
    }
}

impl Clock for Dmc {
    //                          Timer
    //                            |
    //                            v
    // Reader ---> Buffer ---> Shifter ---> Output level ---> (to the mixer)
    fn clock(&mut self) {
        if self.timer.tick() {
            if !self.silence {
                // Update output level but clamp to 0..=127 range
                if self.shift & 0x01 == 0x01 {
                    if self.output_level <= 125 {
                        self.output_level += 2;
                    }
                } else if self.output_level >= 2 {
                    self.output_level -= 2;
                }
                self.shift >>= 1;
            }

            if self.bits_remaining > 0 {
                self.bits_remaining -= 1;
            }
            trace!("APU DMC bits remaining: {}", self.bits_remaining);

            if self.bits_remaining == 0 {
                self.bits_remaining = 8;
                self.silence = self.buffer_empty;
                if !self.buffer_empty {
                    self.shift = self.sample_buffer;
                    self.buffer_empty = true;
                    if self.bytes_remaining > 0 {
                        trace!("APU DMC DMA pending");
                        self.dma_pending = true;
                    }
                }
            }
        }
    }
}

impl Regional for Dmc {
    fn region(&self) -> NesRegion {
        self.region
    }

    fn set_region(&mut self, region: NesRegion) {
        self.region = region;
        self.timer.period = Self::period(region, 0);
    }
}

impl Reset for Dmc {
    fn reset(&mut self, kind: ResetKind) {
        self.timer.reset(kind);
        self.timer.period = Self::period(self.region, 0);
        self.timer.reload();
        self.timer.cycle += 1; // FIXME: Startup timing is slightly wrong, DMA tests fail with the
        // default
        if let ResetKind::Hard = kind {
            self.sample_addr = 0xC000;
            self.sample_length = 1;
        }
        self.irq_enabled = false;
        self.irq_pending = false;
        self.dma_pending = false;
        self.loops = false;
        self.addr = 0x0000;
        self.bytes_remaining = 0;
        self.sample_buffer = 0x00;
        self.buffer_empty = true;
        self.output_level = 0x00;
        self.bits_remaining = 0x08;
        self.shift = 0x00;
        self.silence = true;
        self.should_clock = false;
    }
}
```

"envelope.rs"
```rust
//! APU Envelope implementation.
//!
//! See: <https://www.nesdev.org/wiki/APU_Envelope>

use crate::common::{Clock, Reset, ResetKind};
use serde::{Deserialize, Serialize};

/// APU Envelope provides volume control for APU waveform channels.
///
/// See: <https://www.nesdev.org/wiki/APU_Envelope>
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct Envelope {
    pub start: bool,
    pub constant_volume: bool,
    pub volume: u8,
    pub divider: u8,
    pub counter: u8,
    pub loops: bool,
}

impl Envelope {
    pub const fn new() -> Self {
        Self {
            start: false,
            constant_volume: false,
            volume: 0,
            divider: 0,
            counter: 0,
            loops: false,
        }
    }

    #[inline]
    #[must_use]
    pub const fn volume(&self) -> u8 {
        if self.constant_volume {
            self.volume
        } else {
            self.counter
        }
    }

    #[inline]
    pub const fn restart(&mut self) {
        self.start = true;
    }

    /// $4000/$4004/$400C Envelope control
    #[inline]
    pub const fn write_ctrl(&mut self, val: u8) {
        self.loops = (val & 0x20) == 0x20; // D5
        self.constant_volume = (val & 0x10) == 0x10; // D4
        self.volume = val & 0x0F; // D3..D0
    }
}

impl Clock for Envelope {
    fn clock(&mut self) {
        if self.start {
            self.start = false;
            self.counter = 15;
            self.divider = self.volume;
        } else if self.divider > 0 {
            self.divider -= 1;
        } else {
            self.divider = self.volume;
            if self.counter > 0 {
                self.counter -= 1;
            } else if self.loops {
                self.counter = 15;
            }
        }
    }
}

impl Reset for Envelope {
    fn reset(&mut self, _kind: ResetKind) {
        self.start = false;
        self.constant_volume = false;
        self.volume = 0;
        self.divider = 0;
        self.counter = 0;
    }
}
```

"filter.rs"
```rust
//! Digital filters for the [`Apu`](crate::apu::Apu).
//!
//! See <https://www.nesdev.org/wiki/APU_Mixer>

use crate::{
    common::{NesRegion, Sample},
    cpu::Cpu,
};
use serde::{Deserialize, Serialize};
use std::f32::consts::{PI, TAU};

/// A trait for audio processing that consumes samples.
pub trait Consume {
    fn consume(&mut self, sample: f32);
}

/// Represents a digital filter with certain characteristics.
#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
#[must_use]
pub enum FilterKind {
    Identity,
    HighPass,
    LowPass,
}

/// An infinite impulse response (IIR) filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct Iir {
    pub alpha: f32,
    pub prev_output: f32,
    pub prev_input: f32,
    pub delta: f32,
    pub kind: FilterKind,
}

impl Iir {
    pub const fn identity() -> Self {
        Self {
            alpha: 0.0,
            prev_output: 0.0,
            prev_input: 0.0,
            delta: 0.0,
            kind: FilterKind::Identity,
        }
    }

    pub fn high_pass(sample_rate: f32, cutoff: f32) -> Self {
        let period = 1.0 / sample_rate;
        let cutoff_period = 1.0 / cutoff;
        let alpha = cutoff_period / (cutoff_period + period);
        Self {
            alpha,
            prev_output: 0.0,
            prev_input: 0.0,
            delta: 0.0,
            kind: FilterKind::HighPass,
        }
    }

    pub fn low_pass(sample_rate: f32, cutoff: f32) -> Self {
        let period = 1.0 / sample_rate;
        let cutoff_period = 1.0 / (TAU * cutoff);
        let alpha = cutoff_period / (cutoff_period + period);
        Self {
            alpha,
            prev_output: 0.0,
            prev_input: 0.0,
            delta: 0.0,
            kind: FilterKind::LowPass,
        }
    }
}

impl Consume for Iir {
    fn consume(&mut self, sample: f32) {
        self.prev_output = self.output();
        self.delta = sample - self.prev_input;
        self.prev_input = sample;
    }
}

impl Sample for Iir {
    fn output(&self) -> f32 {
        match self.kind {
            FilterKind::Identity => self.prev_input,
            FilterKind::HighPass => self.alpha * self.prev_output + self.alpha * self.delta,
            FilterKind::LowPass => self.prev_output + self.alpha * self.delta,
        }
    }
}

/// A finite impulse response (FIR) filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct Fir {
    pub kernel: Box<[f32]>,
    pub inputs: Box<[f32]>,
    pub input_index: usize,
    pub kind: FilterKind,
}

impl Fir {
    pub fn low_pass(sample_rate: f32, cutoff: f32, window_size: usize) -> Self {
        Self {
            kernel: windowed_sinc_kernel(sample_rate, cutoff, window_size),
            inputs: vec![0.0; window_size + 1].into(),
            input_index: 0,
            kind: FilterKind::LowPass,
        }
    }
}

impl Consume for Fir {
    fn consume(&mut self, sample: f32) {
        self.inputs[self.input_index] = sample;
        self.input_index += 1;
        if self.input_index >= self.inputs.len() {
            self.input_index = 0;
        }
    }
}

impl Sample for Fir {
    fn output(&self) -> f32 {
        let kernel = &self.kernel[..];
        let inputs = &self.inputs[..];
        let idx = self.input_index;

        let mut sum = 0f32;

        // input_index..inputs.len()
        let end = (inputs.len() - idx).min(kernel.len());
        for i in 0..end {
            sum = kernel[i].mul_add(inputs[i + idx], sum);
        }

        // 0..input_index
        for i in 0..idx {
            sum = kernel[end + i].mul_add(inputs[i], sum);
        }

        sum
    }
}

/// Generate a windowed sinc kernel.
pub fn windowed_sinc_kernel(sample_rate: f32, cutoff: f32, window_size: usize) -> Box<[f32]> {
    fn blackman_window(index: usize, window_size: usize) -> f32 {
        let i = index as f32;
        let m = window_size as f32;
        0.42 - 0.5 * ((TAU * i) / m).cos() + 0.08 * ((2.0 * TAU * i) / m).cos()
    }

    fn sinc(index: usize, fc: f32, window_size: usize) -> f32 {
        let i = index as f32;
        let m = window_size as f32;
        let shifted_index = i - (m / 2.0);
        if index == (window_size / 2) {
            TAU * fc
        } else {
            (TAU * fc * shifted_index).sin() / shifted_index
        }
    }

    fn normalize(input: Box<[f32]>) -> Box<[f32]> {
        let sum: f32 = input.iter().sum();
        input.into_iter().map(|x| x / sum).collect()
    }

    let fc = cutoff / sample_rate;
    let mut kernel = Vec::with_capacity(window_size);
    for i in 0..=window_size {
        kernel.push(sinc(i, fc, window_size) * blackman_window(i, window_size));
    }
    normalize(kernel.into())
}

/// Represents a digital audio filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub enum Filter {
    Iir(Iir),
    Fir(Fir),
}

impl Consume for Filter {
    fn consume(&mut self, sample: f32) {
        match self {
            Filter::Iir(iir) => iir.consume(sample),
            Filter::Fir(fir) => fir.consume(sample),
        }
    }
}

impl Sample for Filter {
    fn output(&self) -> f32 {
        match self {
            Filter::Iir(iir) => iir.output(),
            Filter::Fir(fir) => fir.output(),
        }
    }
}

impl From<Iir> for Filter {
    fn from(filter: Iir) -> Self {
        Self::Iir(filter)
    }
}

impl From<Fir> for Filter {
    fn from(filter: Fir) -> Self {
        Self::Fir(filter)
    }
}

/// Represents a filter with a given sampling period.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct SampledFilter {
    pub filter: Filter,
    pub sample_period: f32,
    pub period_counter: f32,
}

impl SampledFilter {
    pub fn new(filter: impl Into<Filter>, sample_rate: f32) -> Self {
        Self {
            filter: filter.into(),
            sample_period: 1.0 / sample_rate,
            period_counter: 0.0,
        }
    }
}

/// Represents a chain of filters for a given [`NesRegion`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterChain {
    pub region: NesRegion,
    pub dt: f32,
    pub filters: [SampledFilter; 6],
}

impl FilterChain {
    pub fn new(region: NesRegion, output_rate: f32) -> Self {
        let clock_rate = Cpu::region_clock_rate(region);
        let intermediate_sample_rate = output_rate * 2.0 + (PI / 32.0);
        let intermediate_cutoff = output_rate * 0.4;

        let filters = [
            SampledFilter::new(Iir::identity(), 1.0),
            SampledFilter::new(Iir::low_pass(clock_rate, intermediate_cutoff), clock_rate),
            // first-order high-pass filter at 90 Hz
            SampledFilter::new(
                Iir::high_pass(intermediate_sample_rate, 90.0),
                intermediate_sample_rate,
            ),
            // first-order high-pass filter at 440 Hz
            SampledFilter::new(
                Iir::high_pass(intermediate_sample_rate, 440.0),
                intermediate_sample_rate,
            ),
            // first-order low-pass filter at 14 kHz
            SampledFilter::new(
                Iir::low_pass(intermediate_sample_rate, 14000.0),
                intermediate_sample_rate,
            ),
            // TODO: Support famicom filter selection
            // // first-order high-pass filter at 37 Hz
            // filters.push(SampledFilter::new(
            //     Iir::high_pass(intermediate_sample_rate, 37.0),
            //     intermediate_sample_rate,
            // ));
            // high-quality low-pass filter
            {
                let window_size = 160;
                let intermediate_cutoff = output_rate * 0.45;
                SampledFilter::new(
                    Fir::low_pass(intermediate_sample_rate, intermediate_cutoff, window_size),
                    intermediate_sample_rate,
                )
            },
        ];

        Self {
            region,
            dt: 1.0 / clock_rate,
            filters,
        }
    }
}

impl Consume for FilterChain {
    fn consume(&mut self, sample: f32) {
        // Add sample to identity filter
        self.filters[0].filter.consume(sample);
        for i in 1..self.filters.len() {
            let prev = i - 1;
            let current = i;
            while self.filters[current].period_counter >= self.filters[current].sample_period {
                self.filters[current].period_counter -= self.filters[current].sample_period;
                let prev_output = self.filters[prev].filter.output();
                self.filters[current].filter.consume(prev_output);
            }
            self.filters[current].period_counter += self.dt;
        }
    }
}

impl Sample for FilterChain {
    fn output(&self) -> f32 {
        self.filters.last().map_or(0.0, |f| f.filter.output())
    }
}
```

"frame_counter.rs"
```rust
//! The APU Frame Counter implementation.
//!
//! See: <https://www.nesdev.org/wiki/APU_Frame_Counter>

use crate::common::{NesRegion, Reset, ResetKind};
use serde::{Deserialize, Serialize};
use tracing::trace;

/// The APU Frame Counter generates a low-frequency clock for each APU channel.
///
/// See: <https://www.nesdev.org/wiki/APU_Frame_Counter>
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct FrameCounter {
    pub region: NesRegion,
    pub step_cycles: [u32; 6],
    pub step: usize,
    pub mode: u8,
    pub write_buffer: Option<u8>,
    pub write_delay: u8,
    pub block_counter: u8,
    pub cycle: u32,
    pub inhibit_irq: bool, // Set by $4017 D6
    pub irq_pending: bool,
}

/// The Frame Counter clock type.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameType {
    #[default]
    None,
    Quarter,
    Half,
}

impl FrameCounter {
    const STEP4_CYCLES_NTSC: [u32; 6] = [7457, 14913, 22371, 29828, 29829, 29830];
    const STEP5_CYCLES_NTSC: [u32; 6] = [7457, 14913, 22371, 29829, 37281, 37282];
    const STEP4_CYCLES_PAL: [u32; 6] = [8313, 16627, 24939, 33252, 33253, 33254];
    const STEP5_CYCLES_PAL: [u32; 6] = [8313, 16627, 24939, 33253, 41565, 41566];

    const FRAME_TYPE: [FrameType; 6] = [
        FrameType::Quarter,
        FrameType::Half,
        FrameType::Quarter,
        FrameType::None,
        FrameType::Half,
        FrameType::None,
    ];

    pub const fn new(region: NesRegion) -> Self {
        let mode = 0;
        let step_cycles = Self::step_cycles(mode, region);
        Self {
            region,
            step_cycles,
            step: 0,
            mode,
            write_buffer: None,
            write_delay: 0,
            block_counter: 0,
            cycle: 0,
            inhibit_irq: false,
            irq_pending: false,
        }
    }

    pub const fn set_region(&mut self, region: NesRegion) {
        self.region = region;
        self.step_cycles = Self::step_cycles(self.mode, region);
    }

    const fn step_cycles(mode: u8, region: NesRegion) -> [u32; 6] {
        match (mode, region) {
            (0, NesRegion::Auto | NesRegion::Ntsc | NesRegion::Dendy) => Self::STEP4_CYCLES_NTSC,
            (0, NesRegion::Pal) => Self::STEP4_CYCLES_PAL,
            (_, NesRegion::Auto | NesRegion::Ntsc | NesRegion::Dendy) => Self::STEP5_CYCLES_NTSC,
            (_, NesRegion::Pal) => Self::STEP5_CYCLES_PAL,
        }
    }

    /// On write to $4017
    pub fn write(&mut self, val: u8, cycle: u32) {
        self.write_buffer = Some(val);
        // Writes occurring on odd clocks are delayed
        self.write_delay = if cycle & 0x01 == 0x01 { 4 } else { 3 };
        trace!("APU $4017 write delay cycles: {}", self.write_delay);
        self.inhibit_irq = val & 0x40 == 0x40; // D6
        if self.inhibit_irq {
            trace!("APU Frame Counter IRQ inhibit");
            self.irq_pending = false;
        }
    }

    #[inline(always)]
    pub const fn should_clock(&mut self, cycles: u32) -> bool {
        self.block_counter > 0
            || self.write_buffer.is_some()
            || (self.cycle + cycles) >= (self.step_cycles[self.step] - 1)
    }

    // mode 0: 4-step  effective rate (approx)
    // ---------------------------------------
    // - - - f f f      60 Hz
    // - l - - l -     120 Hz
    // e e e - e -     240 Hz
    //
    // mode 1: 5-step  effective rate (approx)
    // ---------------------------------------
    // - - - - - -     (interrupt flag never set)
    // - l - - l -     96 Hz
    // e e e - e -     192 Hz
    pub fn clock_with(&mut self, cycles: u32, mut on_clock: impl FnMut(FrameType)) -> u32 {
        let mut cycles_ran = 0;
        let step_cycles = self.step_cycles[self.step];
        if self.cycle + cycles >= step_cycles {
            if !self.inhibit_irq && self.mode == 0 && self.step >= 3 {
                trace!(
                    "APU Frame Counter IRQ pending - cycles: {} >= {step_cycles}",
                    self.cycle + cycles
                );
                self.irq_pending = true;
            }

            let ty = Self::FRAME_TYPE[self.step];
            if ty != FrameType::None && self.block_counter == 0 {
                on_clock(ty);
                // Do not allow writes to $4017 to clock for the next cycle (odd + following even
                // cycle)
                self.block_counter = 2;
            }

            if step_cycles >= self.cycle {
                cycles_ran = step_cycles - self.cycle;
            }

            self.step += 1;
            if self.step == 6 {
                trace!(
                    "APU Frame Counter total cycles: {}",
                    self.cycle + cycles_ran
                );
                self.step = 0;
                self.cycle = 0;
            } else {
                self.cycle += cycles_ran;
            }
        } else {
            cycles_ran = cycles;
            self.cycle += cycles_ran;
        }

        if let Some(val) = self.write_buffer {
            self.write_delay -= 1;
            if self.write_delay == 0 {
                self.mode = if val & 0x80 == 0x80 { 1 } else { 0 };
                self.step_cycles = Self::step_cycles(self.mode, self.region);
                self.step = 0;
                self.cycle = 0;
                self.write_buffer = None;
                if self.mode == 1 && self.block_counter == 0 {
                    // Writing to $4017 with bit 7 set will immediately generate a quarter/half frame
                    on_clock(FrameType::Half);
                    self.block_counter = 2;
                }
            }
        }

        if self.block_counter > 0 {
            self.block_counter -= 1;
        }

        cycles_ran
    }
}

impl Reset for FrameCounter {
    fn reset(&mut self, kind: ResetKind) {
        self.cycle = 0;
        if kind == ResetKind::Hard {
            self.mode = 0;
            self.step_cycles = Self::step_cycles(self.mode, self.region);
            // After reset, APU acts as if $4017 was written 9-12 clocks before first instruction,
            // Reset acts as if $00 was written to $4017
            self.write(0x00, 0);
            self.write_delay -= 1; // FIXME: Startup timing is slightly wrong, reset_timing fails
            // with the default
        }
        self.step = 0;
        self.block_counter = 0;
        self.irq_pending = false;
    }
}
```

"length_counter.rs"
```rust
//! APU Length Counter implementation.
//!
//! See: <https://www.nesdev.org/wiki/APU_Length_Counter>

use crate::{
    apu::Channel,
    common::{Clock, Reset, ResetKind},
};
use serde::{Deserialize, Serialize};

/// APU Length Counter provides duration control for APU waveform channels.
///
/// See: <https://www.nesdev.org/wiki/APU_Length_Counter>
#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
#[must_use]
pub struct LengthCounter {
    pub enabled: bool,
    pub channel: Channel,
    pub halt: bool,
    pub new_halt: bool,
    pub counter: u8, // Entry into LENGTH_TABLE
    pub previous_counter: u8,
    pub reload: u8,
}

impl LengthCounter {
    const LENGTH_TABLE: [u8; 32] = [
        10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96,
        22, 192, 24, 72, 26, 16, 28, 32, 30,
    ];

    pub const fn new(channel: Channel) -> Self {
        Self {
            enabled: false,
            channel,
            halt: false,
            new_halt: false,
            counter: 0,
            previous_counter: 0,
            reload: 0,
        }
    }

    #[inline]
    pub const fn write(&mut self, val: u8) {
        if self.enabled {
            self.reload = Self::LENGTH_TABLE[val as usize]; // D7..D3
            self.previous_counter = self.counter;
        }
    }

    #[inline]
    pub const fn set_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.counter = 0;
        }
        self.enabled = enabled;
    }

    #[inline]
    pub const fn reload(&mut self) {
        if self.reload > 0 {
            if self.counter == self.previous_counter {
                self.counter = self.reload;
            }
            self.reload = 0;
        }
        self.halt = self.new_halt;
    }

    #[inline]
    pub const fn write_ctrl(&mut self, halt: bool) {
        self.new_halt = halt;
    }
}

impl Clock for LengthCounter {
    fn clock(&mut self) {
        if self.counter > 0 && !self.halt {
            self.counter -= 1;
        }
    }
}

impl Reset for LengthCounter {
    fn reset(&mut self, kind: ResetKind) {
        self.enabled = false;
        match kind {
            ResetKind::Soft => {
                if self.channel != Channel::Triangle {
                    self.halt = false;
                    self.new_halt = false;
                    self.counter = 0;
                    self.reload = 0;
                    self.previous_counter = 0;
                }
            }
            ResetKind::Hard => {
                self.halt = false;
                self.new_halt = false;
                self.counter = 0;
                self.reload = 0;
                self.previous_counter = 0;
            }
        }
    }
}
```

"noise.rs"
```rust
//! APU Noise Channel implementation.
//!
//! See: <https://www.nesdev.org/wiki/APU_Noise>

use crate::{
    apu::{
        Channel,
        envelope::Envelope,
        length_counter::LengthCounter,
        timer::{Timer, TimerCycle},
    },
    common::{Clock, NesRegion, Regional, Reset, ResetKind, Sample},
};
use serde::{Deserialize, Serialize};

/// Noise shift mode.
#[derive(Debug, PartialEq, Eq, Copy, Clone, Serialize, Deserialize)]
pub enum ShiftMode {
    /// Zero (XOR bits 0 and 1)
    Zero,
    /// One (XOR bits 0 and 6)
    One,
}

/// APU Noise Channel provides pseudo-random noise generation.
///
/// See: <https://www.nesdev.org/wiki/APU_Noise>
#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct Noise {
    pub region: NesRegion,
    pub timer: Timer,
    pub shift: u16,
    pub shift_mode: ShiftMode,
    pub length: LengthCounter,
    pub envelope: Envelope,
    pub force_silent: bool,
}

impl Default for Noise {
    fn default() -> Self {
        Self::new(NesRegion::default())
    }
}

impl Noise {
    const PERIOD_TABLE_NTSC: [u16; 16] = [
        4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
    ];
    const PERIOD_TABLE_PAL: [u16; 16] = [
        4, 8, 14, 30, 60, 88, 118, 148, 188, 236, 354, 472, 708, 944, 1890, 3778,
    ];

    pub const fn new(region: NesRegion) -> Self {
        Self {
            region,
            timer: Timer::new(Self::period(region, 0)),
            shift: 1, // defaults to 1 on power up
            shift_mode: ShiftMode::Zero,
            length: LengthCounter::new(Channel::Noise),
            envelope: Envelope::new(),
            force_silent: false,
        }
    }

    #[must_use]
    pub const fn is_muted(&self) -> bool {
        (self.shift & 0x01) == 0x01 || self.silent()
    }

    #[must_use]
    pub const fn silent(&self) -> bool {
        self.force_silent
    }

    pub const fn set_silent(&mut self, silent: bool) {
        self.force_silent = silent;
    }

    const fn period(region: NesRegion, val: u8) -> u16 {
        let index = (val & 0x0F) as usize;
        match region {
            NesRegion::Auto | NesRegion::Ntsc | NesRegion::Dendy => {
                Self::PERIOD_TABLE_NTSC[index] - 1
            }
            NesRegion::Pal => Self::PERIOD_TABLE_PAL[index] - 1,
        }
    }

    pub fn clock_quarter_frame(&mut self) {
        self.envelope.clock();
    }

    pub fn clock_half_frame(&mut self) {
        self.clock_quarter_frame();
        self.length.clock();
    }

    /// $400C Noise control
    pub const fn write_ctrl(&mut self, val: u8) {
        self.length.write_ctrl((val & 0x20) == 0x20); // !D5
        self.envelope.write_ctrl(val);
    }

    /// $400E Noise timer
    pub const fn write_timer(&mut self, val: u8) {
        self.timer.period = Self::period(self.region, val);
        self.shift_mode = if (val & 0x80) == 0x80 {
            ShiftMode::One
        } else {
            ShiftMode::Zero
        };
    }

    /// $400F Length counter
    pub const fn write_length(&mut self, val: u8) {
        self.length.write(val >> 3);
        self.envelope.restart();
    }

    pub const fn set_enabled(&mut self, enabled: bool) {
        self.length.set_enabled(enabled);
    }

    pub const fn volume(&self) -> u8 {
        if self.length.counter > 0 {
            self.envelope.volume()
        } else {
            0
        }
    }
}

impl Sample for Noise {
    fn output(&self) -> f32 {
        if self.is_muted() {
            0f32
        } else {
            f32::from(self.volume())
        }
    }
}

impl TimerCycle for Noise {
    fn cycle(&self) -> u32 {
        self.timer.cycle
    }
}

impl Clock for Noise {
    //    Timer --> Shift Register   Length Counter
    //                    |                |
    //                    v                v
    // Envelope -------> Gate ----------> Gate --> (to mixer)
    fn clock(&mut self) {
        if self.timer.tick() {
            let shift_by = if self.shift_mode == ShiftMode::One {
                6
            } else {
                1
            };
            let feedback = (self.shift & 0x01) ^ ((self.shift >> shift_by) & 0x01);
            self.shift >>= 1;
            self.shift |= feedback << 14;
        }
    }
}

impl Regional for Noise {
    fn region(&self) -> NesRegion {
        self.region
    }

    fn set_region(&mut self, region: NesRegion) {
        self.region = region;
    }
}

impl Reset for Noise {
    fn reset(&mut self, kind: ResetKind) {
        self.timer.reset(kind);
        self.timer.period = Self::period(self.region, 0);
        self.length.reset(kind);
        self.envelope.reset(kind);
        self.shift = 1;
        self.shift_mode = ShiftMode::Zero;
    }
}
```

"pulse.rs"
```rust
//! APU Pulse Channel implementation.
//!
//! See: <https://www.nesdev.org/wiki/APU_Pulse>

use crate::{
    apu::{
        Channel,
        envelope::Envelope,
        length_counter::LengthCounter,
        timer::{Timer, TimerCycle},
    },
    common::{Clock, Reset, ResetKind, Sample},
};
use serde::{Deserialize, Serialize};

/// Pulse Channel output frequency. Supports MMC5 being able to pulse at ultrasonic frequencies.
#[derive(Debug, PartialEq, Eq, Copy, Clone, Serialize, Deserialize)]
pub enum OutputFreq {
    Default,
    Ultrasonic,
}

/// Pulse Channel selection.
#[derive(Debug, PartialEq, Eq, Copy, Clone, Serialize, Deserialize)]
pub enum PulseChannel {
    One,
    Two,
}

/// APU Pulse Channel provides square wave generation.
///
/// See: <https://www.nesdev.org/wiki/APU_Pulse>
#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct Pulse {
    pub channel: PulseChannel,
    pub real_period: u16,
    pub timer: Timer,
    pub duty: u8,       // Select row in DUTY_TABLE
    pub duty_cycle: u8, // Select column in DUTY_TABLE
    pub length: LengthCounter,
    pub envelope: Envelope,
    pub sweep: Sweep,
    pub force_silent: bool,
    pub output_freq: OutputFreq,
}

impl Default for Pulse {
    fn default() -> Self {
        Self::new(PulseChannel::One, OutputFreq::Default)
    }
}

impl Pulse {
    const DUTY_TABLE: [[u8; 8]; 4] = [
        [0, 0, 0, 0, 0, 0, 0, 1],
        [0, 0, 0, 0, 0, 0, 1, 1],
        [0, 0, 0, 0, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1, 0, 0],
    ];

    pub const fn new(channel: PulseChannel, output_freq: OutputFreq) -> Self {
        Self {
            channel,
            real_period: 0,
            timer: Timer::new(0),
            duty: 0u8,
            duty_cycle: 0,
            length: LengthCounter::new(match channel {
                PulseChannel::One => Channel::Pulse1,
                PulseChannel::Two => Channel::Pulse2,
            }),
            envelope: Envelope::new(),
            sweep: Sweep::new(channel),
            force_silent: false,
            output_freq,
        }
    }

    #[inline]
    pub fn is_muted(&self) -> bool {
        // MMC5 doesn't mute at ultasonic frequencies
        self.output_freq == OutputFreq::Default
            && (self.real_period < 8 || (!self.sweep.negate && self.sweep.target_period > 0x7FF))
            || self.silent()
    }

    #[must_use]
    pub const fn silent(&self) -> bool {
        self.force_silent
    }

    pub const fn set_silent(&mut self, silent: bool) {
        self.force_silent = silent;
    }

    const fn update_target_period(&mut self) {
        let delta = self.real_period >> self.sweep.shift;
        if self.sweep.negate {
            self.sweep.target_period = self.real_period - delta;
            if let PulseChannel::One = self.channel {
                self.sweep.target_period = self.sweep.target_period.wrapping_sub(1);
            }
        } else {
            self.sweep.target_period = self.real_period + delta;
        }
    }

    const fn set_period(&mut self, period: u16) {
        self.real_period = period;
        self.timer.period = (period * 2) + 1;
        self.update_target_period();
    }

    const fn clock_sweep(&mut self) {
        self.sweep.divider = self.sweep.divider.wrapping_sub(1);
        if self.sweep.divider == 0 {
            if self.sweep.shift > 0
                && self.sweep.enabled
                && self.real_period >= 8
                && self.sweep.target_period <= 0x7FF
            {
                self.set_period(self.sweep.target_period);
            }
            self.sweep.divider = self.sweep.period;
        }

        if self.sweep.reload {
            self.sweep.divider = self.sweep.period;
            self.sweep.reload = false;
        }
    }

    pub fn clock_quarter_frame(&mut self) {
        self.envelope.clock();
    }

    pub fn clock_half_frame(&mut self) {
        self.clock_quarter_frame();
        self.length.clock();
        self.clock_sweep();
    }

    /// $4000/$4004 Pulse control
    pub const fn write_ctrl(&mut self, val: u8) {
        self.length.write_ctrl((val & 0x20) == 0x20); // !D5
        self.envelope.write_ctrl(val);
        self.duty = (val & 0xC0) >> 6;
    }

    /// $4001/$4005 Pulse sweep
    pub const fn write_sweep(&mut self, val: u8) {
        self.sweep.enabled = (val & 0x80) == 0x80;
        self.sweep.negate = (val & 0x08) == 0x08;
        self.sweep.period = ((val & 0x70) >> 4) + 1;
        self.sweep.shift = val & 0x07;
        self.update_target_period();
        self.sweep.reload = true;
    }

    /// $4002/$4006 Pulse timer lo
    pub fn write_timer_lo(&mut self, val: u8) {
        self.set_period(self.real_period & 0x0700 | u16::from(val));
    }

    /// $4003/$4007 Pulse timer hi
    pub fn write_timer_hi(&mut self, val: u8) {
        self.length.write(val >> 3);
        self.set_period(self.real_period & 0xFF | (u16::from(val & 0x07) << 8));
        self.duty_cycle = 0;
        self.envelope.restart();
    }

    pub const fn set_enabled(&mut self, enabled: bool) {
        self.length.set_enabled(enabled);
    }

    pub const fn volume(&self) -> u8 {
        if self.length.counter > 0 {
            self.envelope.volume()
        } else {
            0
        }
    }
}

impl Sample for Pulse {
    fn output(&self) -> f32 {
        if self.is_muted() {
            0.0
        } else {
            f32::from(
                Self::DUTY_TABLE[self.duty as usize][self.duty_cycle as usize] * self.volume(),
            )
        }
    }
}

impl TimerCycle for Pulse {
    fn cycle(&self) -> u32 {
        self.timer.cycle
    }
}

impl Clock for Pulse {
    //                  Sweep -----> Timer
    //                    |            |
    //                    |            |
    //                    |            v
    //                    |        Sequencer   Length Counter
    //                    |            |             |
    //                    |            |             |
    //                    v            v             v
    // Envelope -------> Gate -----> Gate -------> Gate --->(to mixer)
    fn clock(&mut self) {
        if self.timer.tick() {
            self.duty_cycle = self.duty_cycle.wrapping_sub(1) & 0x07;
        }
    }
}

impl Reset for Pulse {
    fn reset(&mut self, kind: ResetKind) {
        self.timer.reset(kind);
        self.length.reset(kind);
        self.envelope.reset(kind);
        self.sweep.reset(kind);
        self.update_target_period();
        self.duty = 0;
        self.duty_cycle = 0;
    }
}

/// APU Sweep provides frequency sweeping for the APU pulse channels.
///
/// See: <https://www.nesdev.org/wiki/APU_Sweep>
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sweep {
    pub enabled: bool,
    pub channel: PulseChannel,
    pub negate: bool, // Treats PulseChannel 1 differently than PulseChannel 2
    pub reload: bool,
    pub shift: u8,
    pub timer: u16,
    pub divider: u8,
    pub period: u8,
    pub target_period: u16,
}

impl Sweep {
    pub const fn new(channel: PulseChannel) -> Self {
        Self {
            enabled: false,
            channel,
            negate: false,
            reload: false,
            shift: 0,
            timer: 0,
            divider: 0,
            period: 0,
            target_period: 0,
        }
    }
}

impl Reset for Sweep {
    fn reset(&mut self, _kind: ResetKind) {
        self.enabled = false;
        self.period = 0;
        self.negate = false;
        self.reload = false;
        self.shift = 0;
        self.divider = 0;
        self.target_period = 0;
    }
}
```

"timer.rs"
```rust
//! Timer abstraction for the [`Apu`](crate::apu::Apu).

use crate::common::{Reset, ResetKind};
use serde::{Deserialize, Serialize};

/// Trait for types that have timers.
pub trait TimerCycle {
    fn cycle(&self) -> u32;
}

/// A timer that generates a clock signal based on a divider and a period. The timer is clocked
/// every (period + 1) * divider cycles.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct Timer {
    pub cycle: u32,
    pub counter: u16,
    pub period: u16,
}

impl Timer {
    pub const fn new(period: u16) -> Self {
        Self {
            cycle: 0,
            counter: 0,
            period,
        }
    }

    pub const fn preload(period: u16) -> Self {
        let mut timer = Self::new(period);
        timer.counter = timer.period;
        timer
    }

    pub const fn reload(&mut self) {
        self.counter = self.period;
    }

    pub const fn tick(&mut self) -> bool {
        self.cycle += 1;
        if self.counter == 0 {
            self.counter = self.period;
            return true;
        }
        self.counter -= 1;
        false
    }
}

impl Reset for Timer {
    fn reset(&mut self, _kind: ResetKind) {
        self.counter = 0;
        self.period = 0;
        self.cycle = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer() {
        // Period (10 + 1) == 11 + initial clock
        let mut timer = Timer::new(10);
        let mut expected = [false; 23];
        expected[0] = true;
        expected[11] = true;
        expected[22] = true;
        assert_eq!(expected, [(); 23].map(|_| timer.tick()));
        assert_eq!(23, timer.cycle);

        // Period (10 + 1) == 11
        let mut timer = Timer::preload(10);
        let mut expected = [false; 22];
        expected[10] = true;
        expected[21] = true;
        assert_eq!(expected, [(); 22].map(|_| timer.tick()));
        assert_eq!(22, timer.cycle);

        // Period (10 * 2) + 1 == 22 + initial clock
        let mut timer = Timer::new((10 * 2) + 1);
        let mut expected = [false; 45];
        expected[0] = true;
        expected[22] = true;
        expected[44] = true;
        assert_eq!(expected, [(); 45].map(|_| timer.tick()));
        assert_eq!(45, timer.cycle);

        // Period (10 * 2) + 1 == 22
        let mut timer = Timer::preload((10 * 2) + 1);
        let mut expected = [false; 44];
        expected[21] = true;
        expected[43] = true;
        assert_eq!(expected, [(); 44].map(|_| timer.tick()));
        assert_eq!(44, timer.cycle);
    }
}

"triangle.rs"
```rust
//! APU Triangle Channel implementation.
//!
//! See: <https://www.nesdev.org/wiki/APU_Triangle>

use crate::{
    apu::{
        Channel,
        length_counter::LengthCounter,
        timer::{Timer, TimerCycle},
    },
    common::{Clock, Reset, ResetKind, Sample},
};
use serde::{Deserialize, Serialize};

/// APU Triangle Channel provides triangle wave generation.
///
/// See: <https://www.nesdev.org/wiki/APU_Triangle>
#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct Triangle {
    pub timer: Timer,
    pub sequence: u8,
    pub length: LengthCounter,
    pub linear: LinearCounter,
    pub force_silent: bool,
}

impl Default for Triangle {
    fn default() -> Self {
        Self::new()
    }
}

impl Triangle {
    const SEQUENCE: [u8; 32] = [
        15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11,
        12, 13, 14, 15,
    ];

    pub const fn new() -> Self {
        Self {
            timer: Timer::new(0),
            sequence: 0,
            length: LengthCounter::new(Channel::Triangle),
            linear: LinearCounter::new(),
            force_silent: false,
        }
    }

    #[must_use]
    pub const fn silent(&self) -> bool {
        self.force_silent
    }

    pub const fn set_silent(&mut self, silent: bool) {
        self.force_silent = silent;
    }

    pub fn clock_quarter_frame(&mut self) {
        self.linear.clock();
    }

    pub fn clock_half_frame(&mut self) {
        self.clock_quarter_frame();
        self.length.clock();
    }

    /// $4008 Linear counter control
    pub const fn write_linear_counter(&mut self, val: u8) {
        self.linear.control = (val & 0x80) == 0x80; // D7
        self.linear.write(val & 0x7F); // D6..D0;
        self.length.write_ctrl(self.linear.control); // !D7
    }

    /// $400A Triangle timer lo
    pub fn write_timer_lo(&mut self, val: u8) {
        self.timer.period = (self.timer.period & 0xFF00) | u16::from(val); // D7..D0
    }

    /// $400B Triangle timer high
    pub fn write_timer_hi(&mut self, val: u8) {
        self.length.write(val >> 3);
        self.timer.period = (self.timer.period & 0x00FF) | (u16::from(val & 0x07) << 8); // D2..D0
        self.linear.reload = true;
    }

    pub const fn set_enabled(&mut self, enabled: bool) {
        self.length.set_enabled(enabled);
    }
}

impl Sample for Triangle {
    fn output(&self) -> f32 {
        if self.silent() {
            0.0
        } else if self.timer.period < 2 {
            // This is normally silenced by a lowpass filter on real hardware
            // See: https://forums.nesdev.org/viewtopic.php?t=10658
            7.5
        } else {
            f32::from(Self::SEQUENCE[self.sequence as usize])
        }
    }
}

impl TimerCycle for Triangle {
    fn cycle(&self) -> u32 {
        self.timer.cycle
    }
}

impl Clock for Triangle {
    //       Linear Counter   Length Counter
    //             |                |
    //             v                v
    // Timer ---> Gate ----------> Gate ---> Sequencer ---> (to mixer)
    fn clock(&mut self) {
        if self.timer.tick() && self.length.counter > 0 && self.linear.counter > 0 {
            self.sequence = (self.sequence + 1) & 0x1F;
        }
    }
}

impl Reset for Triangle {
    fn reset(&mut self, kind: ResetKind) {
        self.length.reset(kind);
        self.linear.reset(kind);
        self.sequence = 0;
    }
}

/// APU Linear Counter provides duration control for the APU triangle channel.
///
/// See: <https://www.nesdev.org/wiki/APU_Triangle>
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct LinearCounter {
    pub reload: bool,
    pub control: bool,
    pub counter_reload: u8,
    pub counter: u8,
}

impl LinearCounter {
    pub const fn new() -> Self {
        Self {
            reload: false,
            control: false,
            counter_reload: 0u8,
            counter: 0u8,
        }
    }

    pub const fn write(&mut self, val: u8) {
        self.counter_reload = val;
    }
}

impl Clock for LinearCounter {
    fn clock(&mut self) {
        if self.reload {
            self.counter = self.counter_reload;
        } else if self.counter > 0 {
            self.counter -= 1;
        }
        if !self.control {
            self.reload = false;
        }
    }
}

impl Reset for LinearCounter {
    fn reset(&mut self, _kind: ResetKind) {
        self.counter = 0;
        self.counter_reload = 0;
        self.reload = false;
        self.control = false;
    }
}
```