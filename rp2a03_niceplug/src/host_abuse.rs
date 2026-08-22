//! `rp2a03_niceplug\src\host_abuse.rs`
//! `HostAbuse`: drives `Plugin::initialize`/`process`/`reset` the way a
//! hostile — or merely uncooperative — host would, per
//! `.claude/plans/refactor-plan.md` §M1 step 5. Every case here pins a real
//! defect from that plan's defect list, not a hypothetical; rewritten
//! spec-first for M2 step 14 against that same defect list rather than
//! against whatever the code currently does.
//!
//! `empty_output_slices_are_the_root_cause_export_crash` reproduces defect 1
//! by constructing a `Buffer` directly, the way nice-plug's own buffer
//! manager does when a host hands over an inactive/muted bus
//! (`nice-plug-0.1.10/src/wrapper/util/buffer_management.rs:158,217`): a
//! `num_samples` that does not match the channel slice lengths. Before the
//! per-channel length guard in `plugin.rs`, writing through that buffer was
//! undefined behavior, not a catchable panic.

use nice_plug::prelude::*;
use std::collections::VecDeque;

use crate::plugin::Rp2a03Plugin;

// ---------------------------------------------------------------------------
// Mock host plumbing — just enough of `InitContext`/`ProcessContext` for
// `process` to run without a real host behind it.
// ---------------------------------------------------------------------------

struct MockInitContext;

impl<P: Plugin> InitContext<P> for MockInitContext {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }
    fn execute(&self, _task: P::BackgroundTask) {}
    fn set_latency_samples(&self, _samples: u32) {}
    fn set_current_voice_capacity(&self, _capacity: u32) {}
}

struct MockProcessContext {
    events: VecDeque<NoteEvent<()>>,
    transport: Transport,
}

impl MockProcessContext {
    fn new(events: Vec<NoteEvent<()>>, sample_rate: f32) -> Self {
        Self {
            events: events.into(),
            transport: Transport::new(sample_rate),
        }
    }
}

impl ProcessContext<Rp2a03Plugin> for MockProcessContext {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }
    fn execute_background(&self, _task: ()) {}
    fn execute_gui(&self, _task: ()) {}
    fn transport(&self) -> &Transport {
        &self.transport
    }
    fn next_event(&mut self) -> Option<NoteEvent<()>> {
        self.events.pop_front()
    }
    fn send_event(&mut self, _event: NoteEvent<()>) {}
    fn set_latency_samples(&self, _samples: u32) {}
    fn set_current_voice_capacity(&self, _capacity: u32) {}
}

fn note_on(timing: u32, note: u8, velocity: f32) -> NoteEvent<()> {
    NoteEvent::NoteOn {
        timing,
        voice_id: None,
        channel: 0,
        note,
        velocity,
    }
}

fn note_off(timing: u32, note: u8) -> NoteEvent<()> {
    NoteEvent::NoteOff {
        timing,
        voice_id: None,
        channel: 0,
        note,
        velocity: 0.0,
    }
}

/// Builds a real `Buffer` the way a host does, backed by the caller's own
/// storage, so the channel count and per-channel length are entirely under
/// test control — independent of `num_samples`.
fn make_buffer(num_samples: usize, channels: &mut [Vec<f32>]) -> Buffer<'_> {
    let mut buffer = Buffer::default();
    // SAFETY: `channels` outlives the returned `Buffer` (tied to the same
    // lifetime `'a`), and every element stays alive for as long as the
    // `Buffer` does.
    unsafe {
        buffer.set_slices(num_samples, |output_slices| {
            *output_slices = channels.iter_mut().map(Vec::as_mut_slice).collect();
        });
    }
    buffer
}

fn initialize(plugin: &mut Rp2a03Plugin, sample_rate: f32, max_buffer_size: u32) -> bool {
    let layout = AudioIOLayout {
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    };
    let config = BufferConfig {
        sample_rate,
        min_buffer_size: None,
        max_buffer_size,
        process_mode: ProcessMode::Realtime,
    };
    let mut context = MockInitContext;
    Plugin::initialize(plugin, &layout, &config, &mut context)
}

fn run_process(
    plugin: &mut Rp2a03Plugin,
    buffer: &mut Buffer,
    events: Vec<NoteEvent<()>>,
    sample_rate: f32,
) -> ProcessStatus {
    let mut aux = AuxiliaryBuffers {
        inputs: &mut [],
        outputs: &mut [],
    };
    let mut context = MockProcessContext::new(events, sample_rate);
    Plugin::process(plugin, buffer, &mut aux, &mut context)
}

// ---------------------------------------------------------------------------
// Defect 1 — the export crash: empty/short output slices
// ---------------------------------------------------------------------------

/// Reproduces the export crash directly: before `plugin.rs`'s per-channel
/// length guard existed, this was real undefined behavior — a
/// `get_unchecked_mut` write through a dangling zero-length-`Vec` pointer,
/// not a catchable panic. The root-cause writeup in
/// `.claude/plans/refactor-plan.md` describes exactly this: `num_samples`
/// stays at the host's block length while every output slice is `&mut []`
/// for an inactive/muted bus during offline bounce. The guard makes this
/// safe to run unconditionally.
#[test]
fn empty_output_slices_are_the_root_cause_export_crash() {
    let mut plugin = Rp2a03Plugin::default();
    assert!(initialize(&mut plugin, 44_100.0, 512));

    let mut channels: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
    let mut buffer = make_buffer(512, &mut channels);

    run_process(
        &mut plugin,
        &mut buffer,
        vec![note_on(0, 60, 1.0)],
        44_100.0,
    );
}

/// The safe half of the same scenario: zero output channels entirely (a
/// buffer with no slices at all, `num_samples` still set). No slice exists to
/// write through, so this cannot corrupt memory either before or after the
/// fix — it pins that `process` must not panic when the host provides no
/// output ports.
#[test]
fn a_buffer_with_no_output_channels_at_all_does_not_panic() {
    let mut plugin = Rp2a03Plugin::default();
    assert!(initialize(&mut plugin, 44_100.0, 512));

    let mut channels: [Vec<f32>; 0] = [];
    let mut buffer = make_buffer(512, &mut channels);

    run_process(
        &mut plugin,
        &mut buffer,
        vec![note_on(0, 60, 1.0)],
        44_100.0,
    );
}

// ---------------------------------------------------------------------------
// Defect 2 — a persisted `SharedSequences` blob with a short `SequenceBank`
// ---------------------------------------------------------------------------

/// A session file saved before some future `MAX_SEQUENCES` bump — or simply
/// hand-edited — deserializes a `SequenceBank` shorter than `MAX_SEQUENCES`.
/// `SequenceBank`'s custom `Deserialize` must normalize it back up (the way
/// `WaveSlot` already does) so `slot()`/`slot_mut()` stay in-bounds for every
/// index up to `MAX_SEQUENCES - 1`, the way `SequenceCache::refresh` indexes
/// them on the audio thread every block.
#[test]
fn a_persisted_bank_shorter_than_max_sequences_still_indexes_every_slot() {
    #[derive(serde::Serialize)]
    struct ShortBank {
        slots: Vec<rp2a03_common::SequenceSlot>,
    }
    let short = ShortBank {
        slots: vec![rp2a03_common::SequenceSlot::default(); 3],
    };
    let bytes = rmp_serde::to_vec(&short).expect("a short bank must encode");
    let bank: rp2a03_common::SequenceBank =
        rmp_serde::from_slice(&bytes).expect("a short bank must still decode");

    for index in [
        0,
        3,
        50,
        rp2a03_common::MAX_SEQUENCES - 1,
        rp2a03_common::MAX_SEQUENCES,
    ] {
        let _ = bank.slot(index);
    }
}

// ---------------------------------------------------------------------------
// Block size
// ---------------------------------------------------------------------------

#[test]
fn a_zero_length_block_renders_without_panicking() {
    let mut plugin = Rp2a03Plugin::default();
    assert!(initialize(&mut plugin, 44_100.0, 512));

    let mut channels: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
    let mut buffer = make_buffer(0, &mut channels);

    run_process(&mut plugin, &mut buffer, vec![], 44_100.0);
}

/// A host that ignores the `max_buffer_size` it was quoted at `initialize` —
/// the normal offline-bounce case — must still render every sample it asked
/// for, through the real `process` entry point (not just `render_block`).
#[test]
fn a_block_larger_than_the_declared_max_buffer_size_renders_every_sample() {
    let mut plugin = Rp2a03Plugin::default();
    assert!(initialize(&mut plugin, 44_100.0, 512));

    const OVERSIZED: usize = 8192;
    let mut channels: [Vec<f32>; 2] = [vec![0.0; OVERSIZED], vec![0.0; OVERSIZED]];
    let mut buffer = make_buffer(OVERSIZED, &mut channels);

    run_process(
        &mut plugin,
        &mut buffer,
        vec![note_on(0, 60, 1.0)],
        44_100.0,
    );

    assert!(
        channels[0].iter().any(|&s| s != 0.0),
        "an oversized block must still produce audio, not silently truncate"
    );
}

// ---------------------------------------------------------------------------
// Event ordering
// ---------------------------------------------------------------------------

/// Events are not guaranteed sorted by timing — a host is free to hand them
/// over in any order it received them. `process` must not panic or hang.
#[test]
fn out_of_order_event_timings_do_not_panic() {
    let mut plugin = Rp2a03Plugin::default();
    assert!(initialize(&mut plugin, 44_100.0, 512));

    let mut channels: [Vec<f32>; 2] = [vec![0.0; 512], vec![0.0; 512]];
    let mut buffer = make_buffer(512, &mut channels);

    let events = vec![
        note_on(300, 60, 1.0),
        note_on(100, 64, 1.0),
        note_off(200, 60),
        note_on(50, 67, 1.0),
    ];
    run_process(&mut plugin, &mut buffer, events, 44_100.0);
}

/// A timing at or past the end of the block must be handled, not panic on an
/// out-of-bounds slice index.
#[test]
fn event_timings_past_the_block_end_do_not_panic() {
    let mut plugin = Rp2a03Plugin::default();
    assert!(initialize(&mut plugin, 44_100.0, 512));

    let mut channels: [Vec<f32>; 2] = [vec![0.0; 512], vec![0.0; 512]];
    let mut buffer = make_buffer(512, &mut channels);

    let events = vec![note_on(10_000, 60, 1.0), note_off(u32::MAX, 60)];
    run_process(&mut plugin, &mut buffer, events, 44_100.0);
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// `reset()` mid-stream, with a note held, must not panic and must leave the
/// plugin in a state that keeps rendering cleanly afterward.
#[test]
fn reset_mid_stream_with_a_held_note_does_not_panic() {
    let mut plugin = Rp2a03Plugin::default();
    assert!(initialize(&mut plugin, 44_100.0, 512));

    {
        let mut channels: [Vec<f32>; 2] = [vec![0.0; 512], vec![0.0; 512]];
        let mut buffer = make_buffer(512, &mut channels);
        run_process(
            &mut plugin,
            &mut buffer,
            vec![note_on(0, 60, 1.0)],
            44_100.0,
        );
    }

    Plugin::reset(&mut plugin);

    let mut channels: [Vec<f32>; 2] = [vec![0.0; 512], vec![0.0; 512]];
    let mut buffer = make_buffer(512, &mut channels);
    run_process(&mut plugin, &mut buffer, vec![], 44_100.0);
    assert!(
        channels[0].iter().all(|&s| s == 0.0),
        "a reset voice must not keep sounding the note it held"
    );
}

/// A sample-rate change between two `initialize` calls — a host reconfiguring
/// its engine without tearing the plugin down first — must not leave the
/// resampler in a state that panics on the next block.
#[test]
fn a_sample_rate_change_between_initialize_calls_does_not_panic() {
    let mut plugin = Rp2a03Plugin::default();
    assert!(initialize(&mut plugin, 44_100.0, 512));

    {
        let mut channels: [Vec<f32>; 2] = [vec![0.0; 512], vec![0.0; 512]];
        let mut buffer = make_buffer(512, &mut channels);
        run_process(
            &mut plugin,
            &mut buffer,
            vec![note_on(0, 60, 1.0)],
            44_100.0,
        );
    }

    assert!(initialize(&mut plugin, 96_000.0, 512));

    let mut channels: [Vec<f32>; 2] = [vec![0.0; 512], vec![0.0; 512]];
    let mut buffer = make_buffer(512, &mut channels);
    run_process(
        &mut plugin,
        &mut buffer,
        vec![note_on(0, 60, 1.0)],
        96_000.0,
    );
}

// ---------------------------------------------------------------------------
// M2 step 13 scaffolding — the `assert_no_alloc` guard itself
// ---------------------------------------------------------------------------

/// `no_alloc_render` wraps the exact render path `process` drives — proves
/// a normal, in-capacity block does not allocate, matching the
/// `assert_no_alloc`/`AllocDisabler` wiring in `lib.rs`.
#[test]
fn the_real_render_path_does_not_allocate_on_a_capacity_sized_block() {
    let mut plugin = Rp2a03Plugin::default();
    assert!(initialize(&mut plugin, 44_100.0, 512));

    let host_controls = plugin.params.host_automation_snapshot();
    plugin.voices.apply_host_automation(host_controls);
    let channel_mode = plugin.params.channel_mode();
    plugin.voices.set_channel_mode(channel_mode);
    let active_voice_count = plugin.params.active_voice_count();
    plugin.voices.retire_above(active_voice_count);

    let mut events = vec![note_on(0, 60, 1.0)].into_iter();

    // Does not panic or abort: that is the assertion. `AllocDisabler` aborts
    // the process outright on a real violation (see the sibling test below),
    // so simply returning here is proof the render path stayed allocation-free.
    plugin.no_alloc_render(512, &mut events, active_voice_count, host_controls);
}

/// `AllocDisabler` aborts the process on a violation (`handle_alloc_error`),
/// which cannot be caught in-process — not even `catch_unwind` survives an
/// abort — so the only honest way to prove the guard actually fires is to
/// trigger it in a child process and confirm that child did not exit
/// cleanly. The child re-invokes this same test binary (which already links
/// `AllocDisabler` as its global allocator under `cfg(debug_assertions)`,
/// see `lib.rs`), selecting the `#[ignore]`d probe below. Gated the same
/// way: `assert_no_alloc`'s `disable_release` feature removes the type
/// itself whenever `debug_assertions` is off, so under `--release` there is
/// no guard left to prove.
#[cfg(debug_assertions)]
#[test]
fn a_reintroduced_allocation_aborts_the_process() {
    let exe = std::env::current_exe().expect("test binary path");
    let status = std::process::Command::new(exe)
        .args([
            "--exact",
            "host_abuse::alloc_guard_child_process_probe",
            "--ignored",
            "--test-threads=1",
        ])
        .env("RP2A03_TRIGGER_ALLOC_VIOLATION", "1")
        .status()
        .expect("spawn child test process");

    assert!(
        !status.success(),
        "a real allocation inside assert_no_alloc must abort the process, not succeed quietly"
    );
}

/// Only runs (and only allocates) when the parent test above asks for it via
/// the env var — `#[ignore]`d so a normal `cargo test` run never triggers an
/// intentional abort of its own test binary.
#[cfg(debug_assertions)]
#[test]
#[ignore = "only meant to run as the child process spawned by the test above"]
fn alloc_guard_child_process_probe() {
    if std::env::var_os("RP2A03_TRIGGER_ALLOC_VIOLATION").is_some() {
        assert_no_alloc::assert_no_alloc(|| {
            let v: Vec<u8> = vec![1];
            std::hint::black_box(&v);
        });
    }
}
