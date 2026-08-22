//! Criterion benchmark for the 8-voice mixdown accumulate step named in
//! M7 step 45 (`.claude/plans/refactor-plan.md` line 451). Drives
//! `accumulate_voice_samples`, `#[doc(hidden)] pub`-exposed from
//! `rp2a03_niceplug`'s `lib.rs` specifically for this benchmark — see the
//! doc comment there.

use criterion::{Criterion, criterion_group, criterion_main};
use rp2a03_niceplug::accumulate_voice_samples;
use std::hint::black_box;

/// Matches `voice_bank.rs`'s `MAX_VOICES`.
const VOICES: usize = 8;
/// Matches `baseline.rs`'s render block size.
const BLOCK_SAMPLES: usize = 512;

fn mixdown_benchmark(c: &mut Criterion) {
    let voice_samples: Vec<Vec<i16>> = (0..VOICES)
        .map(|voice| {
            (0..BLOCK_SAMPLES)
                .map(|i| {
                    let phase = (i as f32 + voice as f32 * 37.0) * 0.05;
                    (phase.sin() * 12_000.0) as i16
                })
                .collect()
        })
        .collect();
    let mut output = vec![0.0f32; BLOCK_SAMPLES];

    c.bench_function(
        "voice_bank::accumulate_voice_samples (8 voices/block)",
        |b| {
            b.iter(|| {
                output.fill(0.0);
                for samples in &voice_samples {
                    accumulate_voice_samples(black_box(&mut output), black_box(samples));
                }
                black_box(&output);
            });
        },
    );
}

criterion_group!(benches, mixdown_benchmark);
criterion_main!(benches);
