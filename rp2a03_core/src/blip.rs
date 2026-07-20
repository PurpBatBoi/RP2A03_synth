//! Minimal band-limited step synthesis (BLEP-style), used to turn the chip's
//! instantaneous level changes into properly anti-aliased audio instead of
//! naive sample-and-hold.
//!
//! Background: a raw square/pulse wave has harmonics extending well past
//! Nyquist. Sampling it directly (take whatever level it's at, at each
//! output sample time) folds those harmonics back down into the audible
//! range as harsh aliasing -- worse the higher the note. The standard fix
//! (used by blargg's Blip_Buffer, and almost certainly by MesenCE's own
//! `NesSoundMixer::AddDelta`, given the matching "delta" API shape) is:
//! instead of writing a sharp step directly into the output, spread a small
//! windowed-sinc-shaped correction across a handful of neighboring samples,
//! weighted by exactly where between two samples the transition happened.
//! Summing those contributions as you go reconstructs a smooth,
//! band-limited version of the step with no added filtering needed.
//!
//! This introduces `HALF_WIDTH` samples of fixed latency (a few hundred
//! microseconds at typical sample rates) so that a transition landing near
//! the very start of "the current sample" still has room to deposit its
//! kernel on both sides. That's normal for this technique and inaudible.

const HALF_WIDTH: usize = 8;
const KERNEL_WIDTH: usize = HALF_WIDTH * 2; // 16 taps
const PHASE_COUNT: usize = 64; // sub-sample timing resolution

/// Precomputed windowed-sinc kernel, one row per sub-sample phase. Build
/// once (it's a bit of math) and share it across every channel/voice.
pub struct BlepKernel {
    table: Vec<[f32; KERNEL_WIDTH]>,
}

impl BlepKernel {
    pub fn generate() -> Self {
        let mut table = vec![[0f32; KERNEL_WIDTH]; PHASE_COUNT];
        for (phase, row) in table.iter_mut().enumerate() {
            // How far *past* the reference tap (index HALF_WIDTH) the
            // transition actually sits, in samples: 0.0..1.0.
            let frac = phase as f64 / PHASE_COUNT as f64;

            let mut raw = [0f64; KERNEL_WIDTH];
            let mut sum = 0f64;
            for (i, v) in raw.iter_mut().enumerate() {
                let x = (i as f64 - HALF_WIDTH as f64) - frac;
                let val = sinc(x) * blackman(x, HALF_WIDTH as f64);
                *v = val;
                sum += val;
            }
            // Normalize so the taps sum to 1 -- this is what guarantees a
            // full transition still settles on the correct final level
            // rather than drifting off due to windowing losses.
            for (i, v) in raw.iter().enumerate() {
                row[i] = (v / sum) as f32;
            }
        }
        Self { table }
    }

    fn taps(&self, frac: f64) -> &[f32; KERNEL_WIDTH] {
        let phase = ((frac * PHASE_COUNT as f64) as usize).min(PHASE_COUNT - 1);
        &self.table[phase]
    }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-8 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

fn blackman(x: f64, half_width: f64) -> f64 {
    if x.abs() >= half_width {
        return 0.0;
    }
    let n = (x + half_width) / (2.0 * half_width); // 0..1 across the window
    0.42 - 0.5 * (2.0 * std::f64::consts::PI * n).cos() + 0.08 * (4.0 * std::f64::consts::PI * n).cos()
}

/// A single-channel streaming band-limited accumulator. Call `add_delta`
/// whenever the raw chip output changes level, with the fractional position
/// (0.0..1.0) of that change within the sample currently being built. Call
/// `end_sample()` once per output sample to pop the finished value.
pub struct BlipLine {
    buf: [f32; KERNEL_WIDTH],
    integrator: f32,
}

impl BlipLine {
    pub fn new() -> Self {
        Self { buf: [0.0; KERNEL_WIDTH], integrator: 0.0 }
    }

    pub fn add_delta(&mut self, kernel: &BlepKernel, frac: f64, delta: f32) {
        if delta == 0.0 {
            return;
        }
        let taps = kernel.taps(frac);
        for i in 0..KERNEL_WIDTH {
            self.buf[i] += delta * taps[i];
        }
    }

    /// Finalizes and returns the current sample, then advances the delay line.
    pub fn end_sample(&mut self) -> f32 {
        self.integrator += self.buf[0];
        let out = self.integrator;
        self.buf.copy_within(1.., 0);
        *self.buf.last_mut().unwrap() = 0.0;
        out
    }
}