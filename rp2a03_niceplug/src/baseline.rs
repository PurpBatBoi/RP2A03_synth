//! Rendered-audio baseline: captures what the synth sounds like today so a
//! refactor can prove it did not change the output.
//!
//! Compiled only under the `baseline` feature. It lives inside the crate
//! because it drives `render_block` and the parameter set directly, and it
//! lives outside `#[cfg(test)]` because it has to outlive the test suite.

use nice_plug::params::InternalParamMut;
use nice_plug::prelude::*;
use rp2a03_common::gui::WaveSynthEffect;
use rp2a03_common::{ChannelMode, FDS_MOD_TABLE_LEN, Lane, SharedSequences, WaveSynthParams};
use rp2a03_core::sequencer::Sequence;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::plugin::Rp2a03Plugin;

const BLOCK: usize = 512;
const SECONDS: f32 = 2.0;
const RATES: [f32; 3] = [44_100.0, 48_000.0, 96_000.0];

const CHIPS: [(ChannelMode, &str); 7] = [
    (ChannelMode::Pulse, "pulse"),
    (ChannelMode::Triangle, "triangle"),
    (ChannelMode::Noise, "noise"),
    (ChannelMode::Vrc6Pulse, "vrc6pulse"),
    (ChannelMode::Vrc6Saw, "vrc6saw"),
    (ChannelMode::S5B, "s5b"),
    (ChannelMode::Fds, "fds"),
];

// ---------------------------------------------------------------------------
// The reference instrument
// ---------------------------------------------------------------------------

/// One fixed instrument, authored so every lane, both LFOs, portamento and the
/// FDS modulator/wavesynth all do something audible. Changing any of this
/// invalidates every committed hash, so it is frozen once captured.
fn seed_instrument(data: &mut SharedSequences) {
    const LANES: [(Lane, &str); 5] = [
        (Lane::Vol, "15 13 11 10 | 9 8 7 6 / 4 2 0"),
        (Lane::Arp, "0 4 7 | 12 7 4"),
        (Lane::Pitch, "0 1 2 1 0 -1 -2 -1 |"),
        (Lane::HiPitch, "0 0 1 0 |"),
        (Lane::Duty, "0 1 2 3 |"),
    ];

    for (lane, text) in LANES {
        let sequence = Sequence::parse(text);
        let (slot_text, slot_sequence) = data.selected_sequence_mut(lane);
        *slot_text = text.to_string();
        *slot_sequence = sequence;
        *data.sequence_enabled_mut(lane) = true;
    }

    // Two FDS waves so the Wave Index lane and the wavesynth both have
    // something to point at: a sine and a ramp.
    let sine: Vec<u16> = (0..64u16)
        .map(|i| {
            let phase = f32::from(i) * std::f32::consts::TAU / 64.0;
            (((phase.sin() + 1.0) * 31.5).round() as i32).clamp(0, 63) as u16
        })
        .collect();
    let ramp: Vec<u16> = (0..64u16).collect();
    {
        let slots = data.wave_slots_mut();
        slots.add_slot_from(&sine);
        slots.add_slot_from(&ramp);
        slots.set_current_slot(0);
    }

    let fds = data.fds_settings_mut(0);
    fds.mod_depth = 24;
    fds.mod_speed = 480;
    fds.mod_delay = 4;
    fds.mod_table.values = (0..FDS_MOD_TABLE_LEN)
        .map(|i| [0i16, 1, 2, 3, -3, -2, -1, 0][i % 8])
        .collect();

    data.set_wavesynth(
        0,
        WaveSynthParams {
            wave1: 0,
            wave2: 1,
            rate_divider: 1,
            effect: WaveSynthEffect::Mix,
            enabled: true,
            speed: 3,
            reset_on_note: true,
            param1: 8,
            param2: 4,
        },
    );
}

/// Host parameters other than the chip selection. Fixed so the LFOs and
/// portamento contribute to every case.
fn seed_params(plugin: &Rp2a03Plugin) {
    let p = &plugin.params;
    set_int(&p.vibrato_depth, 6);
    set_int(&p.vibrato_speed, 5);
    set_int(&p.vibrato_delay, 2);
    set_int(&p.tremolo_depth, 4);
    set_int(&p.tremolo_speed, 3);
    set_int(&p.tremolo_delay, 1);
    set_int(&p.hardware_volume, 15);
    set_int(&p.fine_pitch, 3);
    set_int(&p.hi_pitch, -2);
    set_int(&p.pitch_slide_range, 2);
    set_int(&p.step_time, 60);
    set_bool(&p.portamento_enabled, true);
    set_int(&p.portamento_speed, 12);
}

// ---------------------------------------------------------------------------
// The event script
// ---------------------------------------------------------------------------

/// Absolute-sample events for one case. Note-ons overlap so polyphonic cases
/// exercise allocation and stealing; the tail past the last note-off catches
/// the release ramps.
fn script(sample_rate: f32) -> Vec<(usize, NoteEvent<()>)> {
    let at = |seconds: f32| (seconds * sample_rate) as usize;
    vec![
        (at(0.00), note_on(60, 1.0)),
        (at(0.25), note_on(64, 0.8)),
        (at(0.50), note_on(67, 0.6)),
        (at(0.70), pitch_bend(0.75)),
        (at(0.90), note_on(72, 1.0)),
        (at(1.10), note_off(64)),
        (at(1.20), pitch_bend(0.5)),
        (at(1.35), note_off(72)),
        (at(1.50), note_off(60)),
        (at(1.50), note_off(67)),
    ]
}

fn note_on(note: u8, velocity: f32) -> NoteEvent<()> {
    NoteEvent::NoteOn {
        timing: 0,
        voice_id: None,
        channel: 0,
        note,
        velocity,
    }
}

fn note_off(note: u8) -> NoteEvent<()> {
    NoteEvent::NoteOff {
        timing: 0,
        voice_id: None,
        channel: 0,
        note,
        velocity: 0.0,
    }
}

fn pitch_bend(value: f32) -> NoteEvent<()> {
    NoteEvent::MidiPitchBend {
        timing: 0,
        channel: 0,
        value,
    }
}

fn retime(event: NoteEvent<()>, timing: u32) -> NoteEvent<()> {
    match event {
        NoteEvent::NoteOn {
            voice_id,
            channel,
            note,
            velocity,
            ..
        } => NoteEvent::NoteOn {
            timing,
            voice_id,
            channel,
            note,
            velocity,
        },
        NoteEvent::NoteOff {
            voice_id,
            channel,
            note,
            velocity,
            ..
        } => NoteEvent::NoteOff {
            timing,
            voice_id,
            channel,
            note,
            velocity,
        },
        NoteEvent::MidiPitchBend { channel, value, .. } => NoteEvent::MidiPitchBend {
            timing,
            channel,
            value,
        },
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn set_int(param: &IntParam, value: i32) {
    // SAFETY: no host is attached, so this is the only writer.
    unsafe { param._internal_set_plain_value(value) };
}

fn set_bool(param: &BoolParam, value: bool) {
    // SAFETY: no host is attached, so this is the only writer.
    unsafe { param._internal_set_plain_value(value) };
}

pub struct Case {
    pub name: String,
    pub sample_rate: f32,
    pub samples: Vec<f32>,
}

impl Case {
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for sample in &self.samples {
            for byte in sample.to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash
    }

    #[must_use]
    pub fn peak(&self) -> f32 {
        self.samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
    }

    #[must_use]
    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = self
            .samples
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum();
        (sum / self.samples.len() as f64).sqrt() as f32
    }

    fn record(&self) -> String {
        format!(
            "{} rate={} samples={} hash={:#018x} peak={:.9} rms={:.9}",
            self.name,
            self.sample_rate as u32,
            self.samples.len(),
            self.hash(),
            self.peak(),
            self.rms()
        )
    }
}

fn render(mode: ChannelMode, polyphony: bool, sample_rate: f32, name: String) -> Case {
    let mut plugin = Rp2a03Plugin::default();
    set_int(&plugin.params.waveform, mode as i32);
    set_bool(&plugin.params.polyphony, polyphony);
    set_int(&plugin.params.max_voices, if polyphony { 8 } else { 1 });
    seed_params(&plugin);
    seed_instrument(&mut plugin.params.shared_sequences.lock_master());

    // What `Plugin::initialize` does, minus the host `BufferConfig`.
    plugin.sample_rate = sample_rate;
    plugin
        .voices
        .set_sample_rate(sample_rate)
        .expect("baseline rates are representable");
    plugin.voices.reset_all(plugin.params.channel_mode());
    plugin.playheads.clear();

    let total = (SECONDS * sample_rate) as usize;
    let events = script(sample_rate);
    let mut next = 0usize;
    let mut samples = Vec::with_capacity(total);
    let mut block_start = 0usize;

    while block_start < total {
        let len = BLOCK.min(total - block_start);
        let block_end = block_start + len;

        let mut block_events = Vec::new();
        while next < events.len() && events[next].0 < block_end {
            let (absolute, event) = events[next];
            block_events.push(retime(event, (absolute - block_start) as u32));
            next += 1;
        }

        let host_controls = plugin.params.host_automation_snapshot();
        plugin.voices.apply_host_automation(host_controls);
        let channel_mode = plugin.params.channel_mode();
        plugin.voices.set_channel_mode(channel_mode);
        let active_voice_count = plugin.params.active_voice_count();
        plugin.voices.retire_above(active_voice_count);

        let mut stream = block_events.into_iter();
        plugin.render_block(len, &mut stream, active_voice_count, host_controls);
        samples.extend_from_slice(&plugin.mono_buf()[..len]);

        block_start = block_end;
    }

    Case {
        name,
        sample_rate,
        samples,
    }
}

#[must_use]
pub fn render_all() -> Vec<Case> {
    let mut cases = Vec::new();
    for (mode, chip) in CHIPS {
        for polyphony in [false, true] {
            for rate in RATES {
                let voices = if polyphony { "poly" } else { "mono" };
                let name = format!("{chip}_{voices}_{}", rate as u32);
                cases.push(render(mode, polyphony, rate, name));
            }
        }
    }
    cases
}

// ---------------------------------------------------------------------------
// WAV output (16-bit PCM mono) — for listening and diffing, never committed
// ---------------------------------------------------------------------------

fn write_wav(path: &Path, case: &Case) -> std::io::Result<()> {
    let data_len = case.samples.len() * 2;
    let rate = case.sample_rate as u32;
    let mut out = Vec::with_capacity(44 + data_len);

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for sample in &case.samples {
        out.extend_from_slice(&to_pcm(*sample).to_le_bytes());
    }
    std::fs::write(path, out)
}

fn to_pcm(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn baseline_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir has a workspace root above it")
        .join("tests")
        .join("baseline")
}

/// # Panics
///
/// Panics if the baseline output directory can't be created.
#[must_use]
pub fn main() -> std::process::ExitCode {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "capture".into());
    let dir = baseline_dir();
    let manifest = dir.join("manifest.txt");
    let wav_dir = dir.join("wav");

    let cases = render_all();
    let mut rendered = String::new();
    for case in &cases {
        let _ = writeln!(rendered, "{}", case.record());
    }

    match mode.as_str() {
        "capture" => {
            std::fs::create_dir_all(&wav_dir).expect("create baseline dir");
            for case in &cases {
                write_wav(&wav_dir.join(format!("{}.wav", case.name)), case).expect("write wav");
            }
            std::fs::write(&manifest, &rendered).expect("write manifest");
            println!("captured {} cases -> {}", cases.len(), manifest.display());
            std::process::ExitCode::SUCCESS
        }
        "parity" => parity(&manifest, &wav_dir, &cases),
        other => {
            eprintln!("usage: cargo baseline [capture|parity]  (got {other:?})");
            std::process::ExitCode::FAILURE
        }
    }
}

fn parity(manifest: &Path, wav_dir: &Path, cases: &[Case]) -> std::process::ExitCode {
    let Ok(committed) = std::fs::read_to_string(manifest) else {
        eprintln!(
            "no baseline at {} - run `cargo baseline capture` first",
            manifest.display()
        );
        return std::process::ExitCode::FAILURE;
    };

    let want: Vec<&str> = committed.lines().map(str::trim).collect();
    let mut failures = 0usize;

    if want.len() != cases.len() {
        eprintln!(
            "case count changed: baseline has {}, current run has {}",
            want.len(),
            cases.len()
        );
        failures += 1;
    }

    for (want, case) in want.iter().zip(cases) {
        let got = case.record();
        if *want != got {
            failures += 1;
            eprintln!("MISMATCH {}", case.name);
            eprintln!("  baseline: {want}");
            eprintln!("  current:  {got}");
            report_first_diff(wav_dir, case);
        }
    }

    if failures == 0 {
        println!("parity clean: {} cases identical", cases.len());
        std::process::ExitCode::SUCCESS
    } else {
        eprintln!("{failures} case(s) differ from the baseline");
        std::process::ExitCode::FAILURE
    }
}

/// Locate the first differing sample against a locally kept WAV, when one
/// exists. The WAVs are not committed, so this is best-effort diagnostics.
fn report_first_diff(wav_dir: &Path, case: &Case) {
    let path = wav_dir.join(format!("{}.wav", case.name));
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("  (no local wav to diff against)");
        return;
    };
    let Some(pcm) = bytes.get(44..) else {
        eprintln!("  (local wav is truncated)");
        return;
    };

    for (index, chunk) in pcm.as_chunks::<2>().0.iter().enumerate() {
        let want = i16::from_le_bytes([chunk[0], chunk[1]]);
        let Some(sample) = case.samples.get(index) else {
            eprintln!("  current render is shorter, from sample {index}");
            return;
        };
        if want != to_pcm(*sample) {
            let seconds = index as f32 / case.sample_rate;
            eprintln!(
                "  first diff at sample {index} ({seconds:.4}s): baseline {want}, current {}",
                to_pcm(*sample)
            );
            return;
        }
    }
}
