// blip_buf 1.1.0 - Rust port
// Original: http://www.slack.net/~ant/

// Library Copyright (C) 2003-2009 Shay Green. This library is free software;
// you can redistribute it and/or modify it under the terms of the GNU Lesser
// General Public License as published by the Free Software Foundation; either
// version 2.1 of the License, or (at your option) any later version. This
// library is distributed in the hope that it will be useful, but WITHOUT ANY
// WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR
// A PARTICULAR PURPOSE. See the GNU Lesser General Public License for more
// details. You should have received a copy of the GNU Lesser General Public
// License along with this module; if not, write to the Free Software Foundation,
// Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA

type FixedT = u64;
const PRE_SHIFT: u32 = 32;

const TIME_BITS: u32 = PRE_SHIFT + 20;
const TIME_UNIT: FixedT = 1u64 << TIME_BITS;

const BASS_SHIFT: i32 = 9; // affects high-pass filter breakpoint frequency
const END_FRAME_EXTRA: usize = 2; // allows deltas slightly after frame length

const HALF_WIDTH: usize = 8;
const BUF_EXTRA: usize = HALF_WIDTH * 2 + END_FRAME_EXTRA;
const PHASE_BITS: u32 = 5;
const PHASE_COUNT: usize = 1 << PHASE_BITS;
const DELTA_BITS: i32 = 15;
const DELTA_UNIT: i32 = 1 << DELTA_BITS;
const FRAC_BITS: u32 = TIME_BITS - PRE_SHIFT;

/// Maximum clock_rate/sample_rate ratio. For a given sample_rate,
/// clock_rate must not be greater than sample_rate * blip_max_ratio.
pub const BLIP_MAX_RATIO: u32 = 1 << 20;

/// Maximum number of samples that can be generated from one time frame.
pub const BLIP_MAX_FRAME: u32 = 4000;

const MAX_SAMPLE: i32 = 32767;
const MIN_SAMPLE: i32 = -32768;

#[inline]
fn arith_shift(n: i32, shift: i32) -> i32 {
    n >> shift
}

#[inline]
fn clamp(n: &mut i32) {
    if *n != (*n as i16) as i32 {
        *n = arith_shift(*n, 16) ^ MAX_SAMPLE;
    }
}

/// Sample buffer that resamples to output rate and accumulates samples
/// until they're read out.
pub struct BlipBuf {
    factor: FixedT,
    offset: FixedT,
    avail: i32,
    size: i32,
    integrator: i32,
    samples: Vec<i32>,
}

// Sinc_Generator( 0.9, 0.55, 4.5 )
#[rustfmt::skip]
static BL_STEP: [[i16; HALF_WIDTH]; PHASE_COUNT + 1] = [
    [   43, -115,  350, -488, 1136, -914, 5861,21022],
    [   44, -118,  348, -473, 1076, -799, 5274,21001],
    [   45, -121,  344, -454, 1011, -677, 4706,20936],
    [   46, -122,  336, -431,  942, -549, 4156,20829],
    [   47, -123,  327, -404,  868, -418, 3629,20679],
    [   47, -122,  316, -375,  792, -285, 3124,20488],
    [   47, -120,  303, -344,  714, -151, 2644,20256],
    [   46, -117,  289, -310,  634,  -17, 2188,19985],
    [   46, -114,  273, -275,  553,  117, 1758,19675],
    [   44, -108,  255, -237,  471,  247, 1356,19327],
    [   43, -103,  237, -199,  390,  373,  981,18944],
    [   42,  -98,  218, -160,  310,  495,  633,18527],
    [   40,  -91,  198, -121,  231,  611,  314,18078],
    [   38,  -84,  178,  -81,  153,  722,   22,17599],
    [   36,  -76,  157,  -43,   80,  824, -241,17092],
    [   34,  -68,  135,   -3,    8,  919, -476,16558],
    [   32,  -61,  115,   34,  -60, 1006, -683,16001],
    [   29,  -52,   94,   70, -123, 1083, -862,15422],
    [   27,  -44,   73,  106, -184, 1152,-1015,14824],
    [   25,  -36,   53,  139, -239, 1211,-1142,14210],
    [   22,  -27,   34,  170, -290, 1261,-1244,13582],
    [   20,  -20,   16,  199, -335, 1301,-1322,12942],
    [   18,  -12,   -3,  226, -375, 1331,-1376,12293],
    [   15,   -4,  -19,  250, -410, 1351,-1408,11638],
    [   13,    3,  -35,  272, -439, 1361,-1419,10979],
    [   11,    9,  -49,  292, -464, 1362,-1410,10319],
    [    9,   16,  -63,  309, -483, 1354,-1383, 9660],
    [    7,   22,  -75,  322, -496, 1337,-1339, 9005],
    [    6,   26,  -85,  333, -504, 1312,-1280, 8355],
    [    4,   31,  -94,  341, -507, 1278,-1205, 7713],
    [    3,   35, -102,  347, -506, 1238,-1119, 7082],
    [    1,   40, -110,  350, -499, 1190,-1021, 6464],
    [    0,   43, -115,  350, -488, 1136, -914, 5861],
];

impl BlipBuf {
    /// Creates new buffer that can hold at most `sample_count` samples. Sets rates
    /// so that there are `BLIP_MAX_RATIO` clocks per sample. Returns `None` if
    /// `sample_count` is negative.
    pub fn new(size: i32) -> Option<BlipBuf> {
        assert!(size >= 0);

        let buf_len = (size as usize) + BUF_EXTRA;
        let mut m = BlipBuf {
            factor: TIME_UNIT / (BLIP_MAX_RATIO as FixedT),
            offset: 0,
            avail: 0,
            size,
            integrator: 0,
            samples: vec![0i32; buf_len],
        };
        m.clear();
        check_assumptions();
        Some(m)
    }

    /// Sets approximate input clock rate and output sample rate. For every
    /// `clock_rate` input clocks, approximately `sample_rate` samples are generated.
    pub fn set_rates(&mut self, clock_rate: f64, sample_rate: f64) {
        let factor = TIME_UNIT as f64 * sample_rate / clock_rate;
        self.factor = factor as FixedT;

        // Fails if clock_rate exceeds maximum, relative to sample_rate
        let factor_diff = factor - self.factor as f64;
        assert!(0.0 <= factor_diff && factor_diff < 1.0);

        // Equivalent to m->factor = (int) ceil( factor )
        if (self.factor as f64) < factor {
            self.factor += 1;
        }
    }

    /// Clears entire buffer. Afterwards, `samples_avail() == 0`.
    pub fn clear(&mut self) {
        // We could set offset to 0, factor/2, or factor-1. 0 is suitable if
        // factor is rounded up. factor-1 is suitable if factor is rounded down.
        // Since we don't know rounding direction, factor/2 accommodates either,
        // with the slight loss of showing an error in half the time. Since for
        // a 64-bit factor this is years, the halving isn't a problem.
        self.offset = self.factor / 2;
        self.avail = 0;
        self.integrator = 0;
        for s in self.samples.iter_mut() {
            *s = 0;
        }
    }

    /// Length of time frame, in clocks, needed to make `sample_count` additional
    /// samples available.
    pub fn clocks_needed(&self, samples: i32) -> i32 {
        // Fails if buffer can't hold that many more samples
        assert!(samples >= 0 && self.avail + samples <= self.size);

        let needed: FixedT = (samples as FixedT) * TIME_UNIT;
        if needed < self.offset {
            return 0;
        }

        ((needed - self.offset + self.factor - 1) / self.factor) as i32
    }

    /// Makes input clocks before `clock_duration` available for reading as output
    /// samples. Also begins new time frame at `clock_duration`, so that clock time 0 in
    /// the new time frame specifies the same clock as `clock_duration` in the old time
    /// frame. Deltas can have been added slightly past `clock_duration` (up to
    /// however many clocks there are in two output samples).
    pub fn end_frame(&mut self, t: u32) {
        let off: FixedT = (t as FixedT) * self.factor + self.offset;
        self.avail += (off >> TIME_BITS) as i32;
        self.offset = off & (TIME_UNIT - 1);

        // Fails if buffer size was exceeded
        assert!(self.avail <= self.size);
    }

    /// Number of buffered samples available for reading.
    pub fn samples_avail(&self) -> i32 {
        self.avail
    }

    fn remove_samples(&mut self, count: i32) {
        let count = count as usize;
        let remain = self.avail as usize + BUF_EXTRA - count;
        self.avail -= count as i32;

        self.samples.copy_within(count..count + remain, 0);
        for i in remain..remain + count {
            self.samples[i] = 0;
        }
    }

    /// Reads and removes at most `count` samples and writes them to `out`. If
    /// `stereo` is true, writes output to every other element of `out`, allowing easy
    /// interleaving of two buffers into a stereo sample stream. Outputs 16-bit signed
    /// samples. Returns number of samples actually read.
    pub fn read_samples(&mut self, out: &mut [i16], count: i32, stereo: bool) -> i32 {
        assert!(count >= 0);

        let mut count = count;
        if count > self.avail {
            count = self.avail;
        }

        if count > 0 {
            let step: usize = if stereo { 2 } else { 1 };
            let mut sum = self.integrator;
            let mut out_idx: usize = 0;

            for i in 0..count as usize {
                // Eliminate fraction
                let mut s = arith_shift(sum, DELTA_BITS);

                sum += self.samples[i];

                clamp(&mut s);

                out[out_idx] = s as i16;
                out_idx += step;

                // High-pass filter
                sum -= s << (DELTA_BITS - BASS_SHIFT);
            }
            self.integrator = sum;

            self.remove_samples(count);
        }

        count
    }

    /// Adds positive/negative delta into buffer at specified clock time.
    pub fn add_delta(&mut self, time: u32, delta: i32) {
        let fixed =
            ((time as FixedT * self.factor + self.offset) >> PRE_SHIFT) as u32;
        let out_idx = self.avail as usize + (fixed >> FRAC_BITS) as usize;

        let phase_shift = FRAC_BITS - PHASE_BITS;
        let phase = ((fixed >> phase_shift) & (PHASE_COUNT as u32 - 1)) as usize;
        let in_phase = &BL_STEP[phase];
        let rev = &BL_STEP[PHASE_COUNT - phase];

        let interp =
            ((fixed >> (phase_shift - DELTA_BITS as u32)) & (DELTA_UNIT as u32 - 1)) as i32;
        let delta2 = (delta * interp) >> DELTA_BITS;
        let delta = delta - delta2;

        // Fails if buffer size was exceeded
        assert!(out_idx + 15 < self.samples.len());
        assert!(out_idx <= self.size as usize + END_FRAME_EXTRA);

        // The original code accesses in[half_width+i] which means it reads into
        // the next row of bl_step. We need to get the "next phase" row.
        let in_next = &BL_STEP[phase + 1];

        self.samples[out_idx + 0] +=
            in_phase[0] as i32 * delta + in_next[0] as i32 * delta2;
        self.samples[out_idx + 1] +=
            in_phase[1] as i32 * delta + in_next[1] as i32 * delta2;
        self.samples[out_idx + 2] +=
            in_phase[2] as i32 * delta + in_next[2] as i32 * delta2;
        self.samples[out_idx + 3] +=
            in_phase[3] as i32 * delta + in_next[3] as i32 * delta2;
        self.samples[out_idx + 4] +=
            in_phase[4] as i32 * delta + in_next[4] as i32 * delta2;
        self.samples[out_idx + 5] +=
            in_phase[5] as i32 * delta + in_next[5] as i32 * delta2;
        self.samples[out_idx + 6] +=
            in_phase[6] as i32 * delta + in_next[6] as i32 * delta2;
        self.samples[out_idx + 7] +=
            in_phase[7] as i32 * delta + in_next[7] as i32 * delta2;

        // rev = bl_step[phase_count - phase]
        // rev_prev = bl_step[phase_count - phase - 1] (for the negative offsets)
        let rev_prev = &BL_STEP[PHASE_COUNT - phase - 1];

        self.samples[out_idx + 8] +=
            rev[7] as i32 * delta + rev_prev[7] as i32 * delta2;
        self.samples[out_idx + 9] +=
            rev[6] as i32 * delta + rev_prev[6] as i32 * delta2;
        self.samples[out_idx + 10] +=
            rev[5] as i32 * delta + rev_prev[5] as i32 * delta2;
        self.samples[out_idx + 11] +=
            rev[4] as i32 * delta + rev_prev[4] as i32 * delta2;
        self.samples[out_idx + 12] +=
            rev[3] as i32 * delta + rev_prev[3] as i32 * delta2;
        self.samples[out_idx + 13] +=
            rev[2] as i32 * delta + rev_prev[2] as i32 * delta2;
        self.samples[out_idx + 14] +=
            rev[1] as i32 * delta + rev_prev[1] as i32 * delta2;
        self.samples[out_idx + 15] +=
            rev[0] as i32 * delta + rev_prev[0] as i32 * delta2;
    }

    /// Same as `add_delta()`, but uses faster, lower-quality synthesis.
    pub fn add_delta_fast(&mut self, time: u32, delta: i32) {
        let fixed =
            ((time as FixedT * self.factor + self.offset) >> PRE_SHIFT) as u32;
        let out_idx = self.avail as usize + (fixed >> FRAC_BITS) as usize;

        let interp =
            ((fixed >> (FRAC_BITS - DELTA_BITS as u32)) & (DELTA_UNIT as u32 - 1)) as i32;
        let delta2 = delta * interp;

        // Fails if buffer size was exceeded
        assert!(out_idx <= self.size as usize + END_FRAME_EXTRA);

        self.samples[out_idx + 7] += delta * DELTA_UNIT - delta2;
        self.samples[out_idx + 8] += delta2;
    }
}

fn check_assumptions() {
    // Right shift must preserve sign (Rust guarantees this for signed integers)
    debug_assert_eq!((-3i32) >> 1, -2);

    let mut n: i32 = MAX_SAMPLE * 2;
    clamp(&mut n);
    debug_assert_eq!(n, MAX_SAMPLE);

    n = MIN_SAMPLE * 2;
    clamp(&mut n);
    debug_assert_eq!(n, MIN_SAMPLE);

    debug_assert!((BLIP_MAX_RATIO as FixedT) <= TIME_UNIT);
    debug_assert!((BLIP_MAX_FRAME as FixedT) <= (FixedT::MAX >> TIME_BITS));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_clear() {
        let buf = BlipBuf::new(1024).unwrap();
        assert_eq!(buf.samples_avail(), 0);
    }

    #[test]
    fn test_set_rates_and_generate() {
        let mut buf = BlipBuf::new(4096).unwrap();
        buf.set_rates(1_000_000.0, 44_100.0);
        buf.add_delta(0, 1000);
        buf.add_delta(500, -1000);
        buf.end_frame(1000);
        assert!(buf.samples_avail() > 0);
    }

    #[test]
    fn test_read_samples() {
        let mut buf = BlipBuf::new(4096).unwrap();
        buf.set_rates(1_000_000.0, 44_100.0);
        buf.add_delta(0, 5000);
        buf.end_frame(10000);
        let avail = buf.samples_avail();
        let mut out = vec![0i16; avail as usize];
        let read = buf.read_samples(&mut out, avail, false);
        assert_eq!(read, avail);
        assert_eq!(buf.samples_avail(), 0);
    }

    #[test]
    fn test_add_delta_fast() {
        let mut buf = BlipBuf::new(4096).unwrap();
        buf.set_rates(1_000_000.0, 44_100.0);
        buf.add_delta_fast(0, 1000);
        buf.end_frame(1000);
        assert!(buf.samples_avail() > 0);
    }

    #[test]
    fn test_clocks_needed() {
        let mut buf = BlipBuf::new(4096).unwrap();
        buf.set_rates(1_000_000.0, 44_100.0);
        let clocks = buf.clocks_needed(100);
        assert!(clocks > 0);
    }

    #[test]
    fn test_stereo_read() {
        let mut buf = BlipBuf::new(4096).unwrap();
        buf.set_rates(1_000_000.0, 44_100.0);
        buf.add_delta(0, 5000);
        buf.end_frame(10000);
        let avail = buf.samples_avail();
        let mut out = vec![0i16; avail as usize * 2];
        let read = buf.read_samples(&mut out, avail, true);
        assert_eq!(read, avail);
    }
}