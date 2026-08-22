//! `rp2a03_core\src\blip_buf.rs`
//!
//! `blip_buf` is a small waveform synthesis library meant for use in classic video game
//! sound chip emulation. It greatly simplifies sound chip emulation code by handling
//! all the details of resampling. The emulator merely sets the input clock rate and output
//! sample rate, adds waveforms by specifying the clock times where their amplitude changes,
//! then reads the resulting output samples.
//!
//! # Features
//!
//! * Several code examples, including simple sound chip emulator.
//! * Uses fast, high-quality band-limited resampling algorithm ([BLEP]).
//! * Output is low-pass and high-pass filtered and clamped to 16-bit range.
//! * Supports mono, stereo, and multi-channel synthesis.
//!
//! # Based upon
//!
//! This library is a very thin wrapper on the original C library, found here: <https://code.google.com/p/blip-buf>/
//!
//! [BLEP]: http://www.cs.cmu.edu/~eli/L/icmc01/hardsync.html
//!
//! Taken from <https://github.com/mvdnes/blip_buf-rs/tree/main> with some small changes.

use core::fmt;

/// Maximum `clock_rate / sample_rate ratio`. For a given `sample_rate`,
/// `clock_rate` must not be greater than `sample_rate * MAX_RATIO`.
pub const MAX_RATIO: u64 = 1 << 20;

/// Maximum number of samples that can be generated from one time frame.
///
/// This is a hard limit of the fixed-point arithmetic, not of the buffer:
/// [`BlipBuf::clocks_needed`] computes `sample_count * TIME_UNIT` in a [`Fixed`],
/// which overflows past `!1 >> TIME_BITS` samples. A caller rendering a larger
/// block must split it into several frames — see [`BlipBuf::capacity`].
pub const MAX_FRAME: u32 = 4000;

/// Fixed-point time value, `TIME_BITS` of fraction.
type Fixed = u64;

/// Bit counts and buffer paddings. `usize` so they can be used directly as
/// shift amounts and slice indices.
type Bits = usize;

/// One accumulated sample in the delta buffer, before integration.
type Sample = i32;

const PRE_SHIFT: Bits = 32;
const TIME_BITS: Bits = PRE_SHIFT + 20;
const TIME_UNIT: Fixed = (1 as Fixed) << TIME_BITS;
const BASS_SHIFT: Bits = 9;
const END_FRAME_EXTRA: Bits = 2;

const HALF_WIDTH: Bits = 8;
/// Padding past the usable capacity: the 16-tap BLEP kernel one `add_delta`
/// writes, plus the two samples a frame may overshoot by.
const BUF_EXTRA: Bits = HALF_WIDTH * 2 + END_FRAME_EXTRA;
const PHASE_BITS: Bits = 5;
const PHASE_COUNT: Bits = 1 << PHASE_BITS;
const DELTA_BITS: Bits = 15;
const DELTA_UNIT: Bits = 1 << DELTA_BITS;
const FRAC_BITS: Bits = TIME_BITS - PRE_SHIFT;

/// Requested clock/sample rate pair cannot be represented by [`BlipBuf`].
///
/// Both rates must be finite and greater than zero, and `clock_rate` must not
/// exceed `sample_rate * MAX_RATIO` (see [`MAX_RATIO`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InvalidRates {
    /// The rejected input clock rate, in Hz.
    pub clock_rate: f64,
    /// The rejected output sample rate, in Hz.
    pub sample_rate: f64,
}

impl fmt::Display for InvalidRates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "clock rate {} Hz and sample rate {} Hz are out of range: both must be finite and positive, and clock_rate must not exceed sample_rate * {}",
            self.clock_rate, self.sample_rate, MAX_RATIO
        )
    }
}

impl std::error::Error for InvalidRates {}

/// Sample buffer that resamples from input clock rate to output sample rate
pub struct BlipBuf {
    factor: Fixed,
    offset: Fixed,
    integrator: i32,
    avail: usize,
    samples: Vec<Sample>,
}

impl BlipBuf {
    /// Creates new buffer that can hold at most `sample_count` samples. Sets rates
    /// so that there are `MAX_RATIO` clocks per sample.
    #[must_use]
    pub fn new(sample_count: u32) -> Self {
        let sample_count = sample_count as usize;
        const FACTOR: u64 = TIME_UNIT / MAX_RATIO;
        Self {
            factor: FACTOR,
            offset: FACTOR / 2,
            integrator: 0,
            avail: 0,
            samples: vec![0; sample_count + BUF_EXTRA],
        }
    }

    /// Largest number of samples one time frame may produce.
    ///
    /// This is the upper bound on the `sample_count` [`Self::clocks_needed`]
    /// accepts and on the number of samples [`Self::end_frame`] may make
    /// available; exceeding it is a caller bug, not a recoverable condition.
    ///
    /// It is the smaller of the allocated buffer and [`MAX_FRAME`]. Allocating a
    /// buffer larger than `MAX_FRAME` therefore buys nothing — a caller with a
    /// bigger block to render must loop, rendering `capacity()` samples per
    /// frame and draining with [`Self::read_samples`] between frames.
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> u32 {
        ((self.samples.len() - BUF_EXTRA) as u32).min(MAX_FRAME)
    }

    /// Sets approximate input clock rate and output sample rate. For every
    /// `clock_rate` input clocks, approximately `sample_rate` samples are generated.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRates`] when the pair is not representable — a
    /// non-finite or non-positive rate, or a ratio above [`MAX_RATIO`]. The
    /// buffer's existing rates are left untouched in that case.
    pub fn set_rates(&mut self, clock_rate: f64, sample_rate: f64) -> Result<(), InvalidRates> {
        let invalid = InvalidRates {
            clock_rate,
            sample_rate,
        };
        if !clock_rate.is_finite()
            || !sample_rate.is_finite()
            || clock_rate <= 0.0
            || sample_rate <= 0.0
        {
            return Err(invalid);
        }

        if clock_rate > sample_rate * MAX_RATIO as f64 {
            return Err(invalid);
        }

        // `factor` is TIME_UNIT scaled by the sample-per-clock ratio. Rounding up
        // keeps `end_frame` from ever reporting more samples than the caller
        // reserved; rounding down could overshoot by one.
        let factor = (TIME_UNIT as f64) * sample_rate / clock_rate;
        if factor > Fixed::MAX as f64 {
            return Err(invalid);
        }

        self.factor = factor.ceil() as Fixed;
        Ok(())
    }

    /// Clears entire buffer. Afterwards, `samples_avail() == 0`.
    pub fn clear(&mut self) {
        /* We could set offset to 0, factor/2, or factor-1. 0 is suitable if
        factor is rounded up. factor-1 is suitable if factor is rounded down.
        Since we don't know rounding direction, factor/2 accommodates either,
        with the slight loss of showing an error in half the time. Since for
        a 64-bit factor this is years, the halving isn't a problem. */

        self.offset = self.factor / 2;
        self.avail = 0;
        self.integrator = 0;
        self.samples.fill(0);
    }

    /// Fixed-point buffer position for a delta landing at `clock_time`,
    /// shared by [`Self::add_delta`] and [`Self::add_delta_fast`].
    #[inline]
    fn fixed_time(&self, clock_time: u32) -> usize {
        let time = Fixed::from(clock_time);
        ((time * self.factor + self.offset) >> PRE_SHIFT) as usize
    }

    /// Adds positive/negative delta into buffer at specified clock time.
    ///
    /// If `clock_time` lands past the end of the buffer — the caller ran more
    /// clocks than [`Self::clocks_needed`] asked for, or is rendering a block
    /// larger than [`Self::capacity`] — the delta is silently dropped rather
    /// than panicking; a host process must never crash over a caller bug in
    /// this crate. `debug_assert!`s the same condition loudly for tests and
    /// development builds.
    pub fn add_delta(&mut self, clock_time: u32, delta: i32) {
        let fixed = self.fixed_time(clock_time);
        let out_index = self.avail + (fixed >> FRAC_BITS);

        const PHASE_SHIFT: usize = FRAC_BITS - PHASE_BITS;
        let phase = fixed >> PHASE_SHIFT & (PHASE_COUNT - 1);
        let phase_rev = PHASE_COUNT - phase;

        let interp = (fixed >> (PHASE_SHIFT - DELTA_BITS) & (DELTA_UNIT - 1)) as i32;
        let delta2 = (delta * interp) >> DELTA_BITS;
        let delta1 = delta - delta2;

        // One 16-tap kernel is written starting at `out_index`, so that whole
        // window has to be in bounds — the pre-existing check allowed
        // `out_index == samples.len()` and relied on the per-element bounds
        // checks below to catch the overrun.
        let Some(window) = self.samples.get_mut(out_index..out_index + HALF_WIDTH * 2) else {
            debug_assert!(
                false,
                "blip buffer overflow: rendered past capacity() (out_index {out_index}, buffer len {})",
                self.samples.len()
            );
            return;
        };
        let (rise, fall) = window.split_at_mut(HALF_WIDTH);

        // Slicing once hoists the bounds check out of the taps, which matters:
        // this runs once per APU clock per voice.
        for ((out, &step), &next) in rise
            .iter_mut()
            .zip(&BL_STEP[phase])
            .zip(&BL_STEP[phase + 1])
        {
            *out += step * delta1 + next * delta2;
        }
        for ((out, &step), &prev) in fall
            .iter_mut()
            .zip(BL_STEP[phase_rev].iter().rev())
            .zip(BL_STEP[phase_rev - 1].iter().rev())
        {
            *out += step * delta1 + prev * delta2;
        }
    }

    /// Same as `add_delta()`, but uses faster, lower-quality synthesis.
    ///
    /// Saturates under the same conditions as [`Self::add_delta`] rather than
    /// panicking.
    pub fn add_delta_fast(&mut self, clock_time: u32, delta: i32) {
        let fixed = self.fixed_time(clock_time);
        let out_index = self.avail + (fixed >> FRAC_BITS);

        let interp = (fixed >> (FRAC_BITS - DELTA_BITS) & (DELTA_UNIT - 1)) as i32;
        let delta2 = delta * interp;

        let Some(window) = self.samples.get_mut(out_index + 7..out_index + 9) else {
            debug_assert!(
                false,
                "blip buffer overflow: rendered past capacity() (out_index {out_index}, buffer len {})",
                self.samples.len()
            );
            return;
        };
        window[0] += delta * (DELTA_UNIT as i32) - delta2;
        window[1] += delta2;
    }

    /// Length of time frame, in clocks, needed to make `sample_count` additional
    /// samples available.
    ///
    /// `sample_count` is silently clamped so that, added to the samples
    /// already buffered, it never exceeds [`Self::capacity`] — a caller bug,
    /// not a recoverable-by-crashing condition. Callers that take their block
    /// size from a host must still clamp to `capacity()` and render in
    /// several frames; this clamp is a last resort, not license to skip that.
    /// `debug_assert!`s the unclamped invariant loudly for tests and
    /// development builds.
    #[must_use]
    pub fn clocks_needed(&self, sample_count: u32) -> u32 {
        debug_assert!(
            self.avail + sample_count as usize <= self.capacity() as usize,
            "blip buffer overflow: {} buffered + {} requested exceeds capacity {}",
            self.avail,
            sample_count,
            self.capacity()
        );
        let room = (self.capacity() as usize).saturating_sub(self.avail) as u32;
        let sample_count = sample_count.min(room);

        let needed = Fixed::from(sample_count) * TIME_UNIT;
        if needed < self.offset {
            return 0;
        }

        (needed - self.offset).div_ceil(self.factor) as u32
    }

    /// Makes input clocks before `clock_duration` available for reading as output
    /// samples. Also begins new time frame at `clock_duration`, so that clock time 0 in
    /// the new time frame specifies the same clock as `clock_duration` in the old time
    /// frame specified. Deltas can have been added slightly past `clock_duration` (up to
    /// however many clocks there are in two output samples).
    ///
    /// If the frame made more samples available than [`Self::capacity`], the
    /// excess is silently dropped rather than panicking. `debug_assert!`s the
    /// unclamped invariant loudly for tests and development builds.
    pub fn end_frame(&mut self, clock_duration: u32) {
        let off = Fixed::from(clock_duration) * self.factor + self.offset;
        self.avail += (off >> TIME_BITS) as usize;
        self.offset = off & (TIME_UNIT - 1);
        debug_assert!(
            self.avail <= self.capacity() as usize,
            "blip buffer overflow: {} samples available exceeds capacity {}",
            self.avail,
            self.capacity()
        );
        self.avail = self.avail.min(self.capacity() as usize);
    }

    /// Number of buffered samples available for reading.
    #[must_use]
    pub fn samples_avail(&self) -> u32 {
        self.avail as u32
    }

    fn remove_samples(&mut self, count: usize) {
        let remain = (self.avail + BUF_EXTRA).saturating_sub(count);
        self.avail = self.avail.saturating_sub(count);

        // We emulate the following:
        //    memmove( &buf [0], &buf [count], remain * sizeof buf [0] );
        //    memset( &buf [remain], 0, count * sizeof buf [0] );
        self.samples.copy_within(count..count + remain, 0);
        self.samples[remain..remain + count].fill(0);
    }

    /// Reads and removes at most `buf.len()` samples and writes them to `buf`. If
    /// `stereo` is true, writes output to every other element of `buf`, allowing easy
    /// interleaving of two buffers into a stereo sample stream. Outputs 16-bit signed
    /// samples. Returns number of samples actually read.
    pub fn read_samples(&mut self, buf: &mut [i16], stereo: bool) -> usize {
        let count = buf.len().min(self.avail);

        if count > 0 {
            let step = if stereo { 2 } else { 1 };
            let mut sum = self.integrator;

            for i in 0..count {
                let s = clamp_to_i16(sum >> DELTA_BITS);
                buf[i * step] = s as i16;
                sum += self.samples[i];
                sum -= s << (DELTA_BITS - BASS_SHIFT);
            }

            self.integrator = sum;
            self.remove_samples(count);
        }

        count
    }
}

#[inline]
fn clamp_to_i16(n: i32) -> i32 {
    n.clamp(i16::MIN.into(), i16::MAX.into())
}

/// Layout invariants the fixed-point arithmetic above depends on. Checked at
/// compile time rather than in a test, so a bad edit cannot build at all.
///
/// The original C library also asserted that `>>` on a signed integer is an
/// arithmetic shift. Rust defines that in the language — a right shift on a
/// signed type is always arithmetic — so there is nothing left to check.
const _: () = {
    assert!(MAX_RATIO <= TIME_UNIT);
    assert!(MAX_FRAME as Fixed <= !1 >> TIME_BITS);
};

const BL_STEP: &[[i32; 8]] = &[
    [43, -115, 350, -488, 1136, -914, 5861, 21022],
    [44, -118, 348, -473, 1076, -799, 5274, 21001],
    [45, -121, 344, -454, 1011, -677, 4706, 20936],
    [46, -122, 336, -431, 942, -549, 4156, 20829],
    [47, -123, 327, -404, 868, -418, 3629, 20679],
    [47, -122, 316, -375, 792, -285, 3124, 20488],
    [47, -120, 303, -344, 714, -151, 2644, 20256],
    [46, -117, 289, -310, 634, -17, 2188, 19985],
    [46, -114, 273, -275, 553, 117, 1758, 19675],
    [44, -108, 255, -237, 471, 247, 1356, 19327],
    [43, -103, 237, -199, 390, 373, 981, 18944],
    [42, -98, 218, -160, 310, 495, 633, 18527],
    [40, -91, 198, -121, 231, 611, 314, 18078],
    [38, -84, 178, -81, 153, 722, 22, 17599],
    [36, -76, 157, -43, 80, 824, -241, 17092],
    [34, -68, 135, -3, 8, 919, -476, 16558],
    [32, -61, 115, 34, -60, 1006, -683, 16001],
    [29, -52, 94, 70, -123, 1083, -862, 15422],
    [27, -44, 73, 106, -184, 1152, -1015, 14824],
    [25, -36, 53, 139, -239, 1211, -1142, 14210],
    [22, -27, 34, 170, -290, 1261, -1244, 13582],
    [20, -20, 16, 199, -335, 1301, -1322, 12942],
    [18, -12, -3, 226, -375, 1331, -1376, 12293],
    [15, -4, -19, 250, -410, 1351, -1408, 11638],
    [13, 3, -35, 272, -439, 1361, -1419, 10979],
    [11, 9, -49, 292, -464, 1362, -1410, 10319],
    [9, 16, -63, 309, -483, 1354, -1383, 9660],
    [7, 22, -75, 322, -496, 1337, -1339, 9005],
    [6, 26, -85, 333, -504, 1312, -1280, 8355],
    [4, 31, -94, 341, -507, 1278, -1205, 7713],
    [3, 35, -102, 347, -506, 1238, -1119, 7082],
    [1, 40, -110, 350, -499, 1190, -1021, 6464],
    [0, 43, -115, 350, -488, 1136, -914, 5861],
];
