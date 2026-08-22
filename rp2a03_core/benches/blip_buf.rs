//! Criterion benchmarks for the two `BlipBuf` hot loops named in M7 step 45
//! (`.claude/plans/refactor-plan.md` line 451): the 16-tap `add_delta`
//! kernel and the `read_samples` integrator. Parameters mirror real
//! production usage (`rp2a03_niceplug/src/voice.rs`,
//! `rp2a03_niceplug/src/voice_bank.rs`) rather than arbitrary round numbers.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use rp2a03_core::NTSC_CPU_CLOCK;
use rp2a03_core::blip_buf::BlipBuf;
use std::hint::black_box;

/// Matches `voice.rs`'s `BLIP_BUFFER_SIZE`.
const BUFFER_SIZE: u32 = 4096;
const SAMPLE_RATE: f64 = 44_100.0;
/// Matches `baseline.rs`'s render block size.
const BLOCK_SAMPLES: u32 = 512;
/// Production only calls `add_delta` on a waveform edge, not every clock
/// (`voice_bank.rs`: `if delta != 0 { blip.add_delta(...) }`) — a handful of
/// active oscillators' duty transitions per block, not one write per clock.
/// Hammering every clock would wildly overweight this candidate's real
/// per-block cost share.
const EDGES_PER_BLOCK: u32 = 64;
/// Representative nonzero delta magnitude — `add_delta`'s cost doesn't
/// depend on the value, only that every write actually happens.
const DELTA: i32 = 3000;

fn fresh_buffer() -> BlipBuf {
    let mut buf = BlipBuf::new(BUFFER_SIZE);
    buf.set_rates(NTSC_CPU_CLOCK, SAMPLE_RATE)
        .expect("NTSC clock / 44.1kHz is representable");
    buf
}

fn fill_with_edges(buf: &mut BlipBuf, clocks: u32) {
    let stride = (clocks / EDGES_PER_BLOCK).max(1);
    for edge in 0..EDGES_PER_BLOCK {
        let clock = (edge * stride).min(clocks.saturating_sub(1));
        // Alternate sign like a real waveform edge (rising, falling,
        // rising, ...) — a run of same-sign deltas has no real-waveform
        // equivalent and compounds without the cancellation neighboring
        // kernel windows normally get, overflowing the integrator.
        let delta = if edge % 2 == 0 { DELTA } else { -DELTA };
        buf.add_delta(black_box(clock), black_box(delta));
    }
    buf.end_frame(clocks);
}

fn add_delta_benchmark(c: &mut Criterion) {
    let clocks = fresh_buffer().clocks_needed(BLOCK_SAMPLES);
    c.bench_function("blip_buf::add_delta (64 edges/block)", |b| {
        b.iter_batched(
            fresh_buffer,
            |mut buf| fill_with_edges(&mut buf, clocks),
            BatchSize::SmallInput,
        );
    });
}

fn read_samples_benchmark(c: &mut Criterion) {
    let clocks = fresh_buffer().clocks_needed(BLOCK_SAMPLES);
    let mut scratch = vec![0i16; BLOCK_SAMPLES as usize];
    c.bench_function("blip_buf::read_samples (512-sample block)", |b| {
        b.iter_batched(
            || {
                let mut buf = fresh_buffer();
                fill_with_edges(&mut buf, clocks);
                buf
            },
            |mut buf| {
                black_box(buf.read_samples(black_box(&mut scratch), false));
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, add_delta_benchmark, read_samples_benchmark);
criterion_main!(benches);
