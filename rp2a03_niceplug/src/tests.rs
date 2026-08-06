//! rp2a03_niceplug\src\tests.rs
//! Tests for the plugin wrapper: parameter projections, voice allocation, and
//! the rendered-audio fingerprints that pin the DSP path.
//!
//! The fingerprints are golden values captured from the pre-refactor wrapper.
//! They are not derived from theory — if one changes, the audio changed, and
//! that has to be a deliberate decision rather than a refactoring accident.

use nice_plug::params::InternalParamMut;
use nice_plug::prelude::*;
use rp2a03_common::{ChannelMode, SEQUENCE_TYPE_COUNT};

use crate::params::Rp2a03Params;
use crate::plugin::Rp2a03Plugin;
use crate::sequences::snapshot;
use crate::voice::{DEFAULT_SAMPLE_RATE, Voice};
use crate::voice_bank::MAX_VOICES;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn set_int(param: &IntParam, value: i32) {
    unsafe { param._internal_set_plain_value(value) };
}

fn set_bool(param: &BoolParam, value: bool) {
    unsafe { param._internal_set_plain_value(value) };
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

/// A plugin instance brought up the way `Plugin::initialize` would, without
/// needing a host `BufferConfig`.
struct Harness {
    plugin: Rp2a03Plugin,
    rendered: Vec<f32>,
}

impl Harness {
    fn new(waveform: i32, polyphony: bool, max_voices: i32) -> Self {
        let plugin = Rp2a03Plugin::default();
        set_int(&plugin.params.waveform, waveform);
        set_bool(&plugin.params.polyphony, polyphony);
        set_int(&plugin.params.max_voices, max_voices);

        let mut harness = Self {
            plugin,
            rendered: Vec::new(),
        };
        harness.boot();
        harness
    }

    /// Mirrors `Plugin::initialize` at the default sample rate.
    fn boot(&mut self) {
        self.plugin.sample_rate = DEFAULT_SAMPLE_RATE;
        self.plugin
            .voices
            .set_sample_rate(DEFAULT_SAMPLE_RATE)
            .expect("the default sample rate is representable");
        self.plugin
            .voices
            .reset_all(self.plugin.params.channel_mode());
        self.plugin.playheads.clear();
    }

    /// Mirrors one `Plugin::process` call, driving the real `render_block`.
    fn block(&mut self, events: &[NoteEvent<()>], num_samples: usize) -> &mut Self {
        let host_controls = self.plugin.params.host_automation_controls();
        self.plugin.voices.apply_host_automation(host_controls);
        let channel_mode = self.plugin.params.channel_mode();
        self.plugin.voices.set_channel_mode(channel_mode);
        // `try_lock`, matching `process` — the audio thread never blocks on the
        // editor's mutex, and a harness that used `lock()` here would not be
        // able to exercise that.
        if let Some(mut data) = self.plugin.params.shared_sequences.try_lock() {
            self.plugin.params.publish_to_gui(&mut data, channel_mode);
        }

        let active_voice_count = self.plugin.params.active_voice_count();
        self.plugin.voices.retire_above(active_voice_count);

        let mut stream = events.iter().cloned();
        self.plugin
            .render_block(num_samples, &mut stream, active_voice_count, host_controls);

        self.rendered
            .extend_from_slice(&self.plugin.mono_buf()[..num_samples]);
        self
    }

    fn fingerprint(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for sample in &self.rendered {
            for byte in sample.to_bits().to_le_bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash
    }

    fn peak(&self) -> f32 {
        self.rendered.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
    }

    fn sum(&self) -> f32 {
        self.rendered.iter().sum()
    }
}

/// One note held for three blocks, the standard monophonic shape used by the
/// per-waveform fingerprints.
fn mono_note(waveform: i32) -> Harness {
    let mut harness = Harness::new(waveform, false, 1);
    harness
        .block(&[note_on(0, 60, 1.0)], 512)
        .block(&[], 1024)
        .block(&[note_off(0, 60)], 512);
    harness
}

// ---------------------------------------------------------------------------
// Rendered-audio fingerprints
// ---------------------------------------------------------------------------

#[test]
fn pulse_render_matches_golden_output() {
    let harness = mono_note(0);
    assert_eq!(harness.fingerprint(), 0xb9a9_a9c3_b90c_af18);
    assert_eq!(harness.peak(), 1.0880662e-1);
    assert_eq!(harness.sum(), 2.6333196e0);
}

#[test]
fn triangle_render_matches_golden_output() {
    let harness = mono_note(1);
    assert_eq!(harness.fingerprint(), 0x3637_c8c1_609e_03de);
    assert_eq!(harness.peak(), 1.5848446e-1);
    assert_eq!(harness.sum(), 5.7397194e1);
}

#[test]
fn noise_render_matches_golden_output() {
    let harness = mono_note(2);
    assert_eq!(harness.fingerprint(), 0x231f_413d_a0da_42bb);
    assert_eq!(harness.peak(), 1.0889852e-1);
    assert_eq!(harness.sum(), 1.0782955e1);
}

#[test]
fn vrc6_pulse_render_matches_golden_output() {
    let harness = mono_note(3);
    assert_eq!(harness.fingerprint(), 0xff06_7cc9_6348_21a6);
    assert_eq!(harness.peak(), 1.0909199e-1);
    assert_eq!(harness.sum(), 1.2852213e0);
}

#[test]
fn vrc6_saw_render_matches_golden_output() {
    let harness = mono_note(4);
    assert_eq!(harness.fingerprint(), 0x6a48_108d_47ad_d2d6);
    assert_eq!(harness.peak(), 7.526404e-2);
    assert_eq!(harness.sum(), 7.1145854e0);
}

#[test]
fn polyphonic_chord_render_matches_golden_output() {
    let mut harness = Harness::new(0, true, 4);
    harness
        .block(
            &[
                note_on(0, 60, 1.0),
                note_on(0, 64, 0.8),
                note_on(0, 67, 0.6),
            ],
            512,
        )
        .block(&[], 1024)
        .block(&[note_off(0, 64)], 512);

    assert_eq!(harness.fingerprint(), 0x1558_585e_7a79_758a);
    assert_eq!(harness.peak(), 2.4673024e-1);
    assert_eq!(harness.sum(), 1.282046e1);
}

/// More simultaneous notes than voices: exercises stealing and the triangle
/// allocation ramp that keeps a recycled voice from clicking.
#[test]
fn voice_stealing_render_matches_golden_output() {
    let mut harness = Harness::new(1, true, 2);
    harness
        .block(&[note_on(0, 48, 1.0), note_on(0, 55, 1.0)], 256)
        .block(&[note_on(0, 60, 1.0)], 256)
        .block(&[note_on(0, 64, 1.0)], 512)
        .block(&[note_off(0, 60), note_off(0, 64)], 512);

    assert_eq!(harness.fingerprint(), 0x3326_0cbc_e083_51f2);
    assert_eq!(harness.peak(), 2.728146e-1);
    assert_eq!(harness.sum(), 9.523952e1);
}

/// Two note-ons on the same key, then two note-offs: the offs must pair with
/// the ons oldest-first rather than releasing both voices at once.
#[test]
fn repeated_note_fifo_render_matches_golden_output() {
    let mut harness = Harness::new(0, true, 4);
    harness
        .block(&[note_on(0, 60, 1.0)], 128)
        .block(&[note_on(0, 60, 0.5)], 128)
        .block(&[note_off(0, 60)], 256)
        .block(&[note_off(0, 60)], 256);

    assert_eq!(harness.fingerprint(), 0x7633_103b_a627_301a);
    assert_eq!(harness.peak(), 1.0880662e-1);
    assert_eq!(harness.sum(), 2.1788769e0);
}

// ---------------------------------------------------------------------------
// Block splitting
// ---------------------------------------------------------------------------

#[test]
fn the_reused_render_buffer_never_leaks_the_previous_block() {
    // `mono_buf` persists across blocks instead of being freshly zeroed, so any
    // sample the render path fails to write would replay as stale audio.
    let mut harness = Harness::new(0, false, 1);
    harness.block(&[note_on(0, 60, 1.0)], 1024);
    assert!(
        harness.rendered.iter().any(|s| *s != 0.0),
        "the first block should have produced audio to leak"
    );

    harness.rendered.clear();
    harness.boot();
    // A shorter block, so the tail of the loud block is still sitting in the
    // buffer beyond what this one writes.
    harness.block(&[], 256);
    assert!(
        harness.rendered.iter().all(|s| *s == 0.0),
        "silence after a reset must not replay the previous block"
    );
}

#[test]
fn mid_block_note_on_leaves_the_leading_samples_silent() {
    let mut harness = Harness::new(0, false, 1);
    harness.block(&[note_on(256, 60, 1.0)], 512);

    let (before, after) = harness.rendered.split_at(256);
    assert!(
        before.iter().all(|s| *s == 0.0),
        "audio started before the note-on timing"
    );
    assert!(
        after.iter().any(|s| *s != 0.0),
        "no audio after the note-on timing"
    );
}

#[test]
fn events_past_the_block_end_are_clamped_not_dropped() {
    let mut harness = Harness::new(0, false, 1);
    // A timing at exactly the block length must still leave a fully rendered
    // block behind rather than truncating it.
    harness.block(&[note_on(512, 60, 1.0)], 512);
    assert_eq!(harness.rendered.len(), 512);
    assert!(harness.rendered.iter().all(|s| *s == 0.0));
}

// ---------------------------------------------------------------------------
// Parameter projections
// ---------------------------------------------------------------------------

#[test]
fn host_automation_controls_track_the_parameters() {
    let params = Rp2a03Params::default();
    set_int(&params.vibrato_depth, 7);
    set_int(&params.vibrato_speed, 20);
    set_int(&params.tremolo_depth, 3);
    set_int(&params.hardware_volume, 9);
    set_int(&params.fine_pitch, -12);
    set_int(&params.pitch_slide, -4096);
    set_int(&params.pitch_slide_range, 12);
    set_int(&params.step_time, 120);
    set_bool(&params.portamento_enabled, true);
    set_int(&params.portamento_speed, 64);

    let controls = params.host_automation_controls();
    assert_eq!(controls.vibrato_depth, 7);
    assert_eq!(controls.vibrato_speed, 20);
    assert_eq!(controls.tremolo_depth, 3);
    assert_eq!(controls.hardware_volume, 9);
    assert_eq!(controls.fine_pitch, -12);
    assert_eq!(controls.pitch_slide, -4096);
    assert_eq!(controls.pitch_slide_range, 12);
    assert_eq!(controls.step_time_hz, 120);
    assert!(controls.portamento_enabled);
    assert_eq!(controls.portamento_speed, 64);
}

#[test]
fn monophonic_mode_ignores_max_voices() {
    let params = Rp2a03Params::default();
    set_bool(&params.polyphony, false);
    set_int(&params.max_voices, 8);
    assert_eq!(params.active_voice_count(), 1);
}

#[test]
fn polyphonic_voice_count_is_clamped_to_the_bank_size() {
    let params = Rp2a03Params::default();
    set_bool(&params.polyphony, true);

    for requested in 1..=MAX_VOICES as i32 {
        set_int(&params.max_voices, requested);
        assert_eq!(params.active_voice_count(), requested as usize);
    }

    // The parameter range already caps this, but the clamp is the thing that
    // keeps a bad state recall from indexing past the voice bank.
    set_int(&params.max_voices, 99);
    assert_eq!(params.active_voice_count(), MAX_VOICES);
    set_int(&params.max_voices, 0);
    assert_eq!(params.active_voice_count(), 1);
}

#[test]
fn waveform_is_hidden_from_the_host() {
    let params = Rp2a03Params::default();

    assert!(
        params.waveform.flags().contains(ParamFlags::HIDDEN),
        "Waveform must stay hidden: it is slot-owned instrument data, and any \
         host-side writer of it would not write through to `slot_waveforms`, \
         so its value would silently revert on the next slot transition"
    );
    assert!(
        !params.sequence_number.flags().contains(ParamFlags::HIDDEN),
        "Sequence Index stays automatable — it is the intended way to change \
         waveform over a timeline"
    );
}

#[test]
fn waveform_parameter_maps_onto_every_channel_mode() {
    let params = Rp2a03Params::default();
    for (value, expected) in [
        (0, ChannelMode::Pulse),
        (1, ChannelMode::Triangle),
        (2, ChannelMode::Noise),
        (3, ChannelMode::Vrc6Pulse),
        (4, ChannelMode::Vrc6Saw),
    ] {
        set_int(&params.waveform, value);
        assert_eq!(params.channel_mode(), expected);
    }
}

#[test]
fn gui_state_mirrors_the_parameters() {
    let params = Rp2a03Params::default();
    set_bool(&params.polyphony, true);
    set_int(&params.max_voices, 5);
    set_bool(&params.portamento_enabled, true);
    set_int(&params.portamento_speed, 40);

    let mut shared = params.shared_sequences.lock().clone();
    params.publish_to_gui(&mut shared, ChannelMode::Vrc6Saw);

    assert_eq!(shared.channel_mode, ChannelMode::Vrc6Saw);
    assert!(shared.polyphony);
    assert_eq!(shared.max_voices, 5);
    assert!(shared.portamento_enabled);
    assert_eq!(shared.portamento_speed, 40);
}

// ---------------------------------------------------------------------------
// Voice output ramp
// ---------------------------------------------------------------------------

#[test]
fn output_deltas_sum_to_the_target_without_a_ramp() {
    let mut voice = Voice::new();
    voice.midi_handler.channel_mode = ChannelMode::Pulse;

    // No ramp is active, so the first delta jumps straight to the target.
    assert_eq!(voice.advance_output(1500, 1.0), 1500);
    assert_eq!(voice.advance_output(1500, 1.0), 0);
    assert_eq!(voice.advance_output(0, 1.0), -1500);
}

#[test]
fn allocation_ramp_reaches_the_target_and_only_applies_to_triangle() {
    let mut voice = Voice::new();
    voice.midi_handler.channel_mode = ChannelMode::Triangle;
    voice.reset_for_allocation();

    // The ramp rounds its step up, so the target is reached well inside the
    // ramp length and no delta ever overshoots it.
    let mut level = 0i32;
    for _ in 0..4096 {
        level += voice.advance_output(3000, 1.0);
        assert!(level <= 3000, "ramp overshot the target");
    }
    assert_eq!(level, 3000);

    // A non-triangle channel cancels the ramp outright.
    let mut voice = Voice::new();
    voice.midi_handler.channel_mode = ChannelMode::Triangle;
    voice.reset_for_allocation();
    voice.midi_handler.channel_mode = ChannelMode::Pulse;
    assert_eq!(voice.advance_output(3000, 1.0), 3000);
}

#[test]
fn allocation_reset_preserves_the_held_output_level() {
    let mut voice = Voice::new();
    voice.midi_handler.channel_mode = ChannelMode::Triangle;
    voice.advance_output(2000, 1.0);

    voice.reset_for_allocation();

    // Recycling starts from the level already sitting in BlipBuf, so the first
    // delta is a transition rather than a jump up from silence.
    assert_eq!(voice.advance_output(2000, 1.0), 0);
}

#[test]
fn full_reset_returns_the_output_to_silence() {
    let mut voice = Voice::new();
    voice.midi_handler.channel_mode = ChannelMode::Pulse;
    voice.advance_output(1500, 1.0);

    voice.reset();

    assert_eq!(voice.advance_output(1500, 1.0), 1500);
}

// ---------------------------------------------------------------------------
// Sequence state
// ---------------------------------------------------------------------------

#[test]
fn playheads_start_and_clear_as_unset() {
    let plugin = Rp2a03Plugin::default();
    let steps = plugin.playheads.handle();
    plugin.playheads.clear();

    let published = snapshot(&steps);
    for index in 0..SEQUENCE_TYPE_COUNT {
        assert_eq!(published.step(index), None);
    }
}

#[test]
fn a_running_sequence_publishes_its_playhead_to_the_editor() {
    let mut harness = Harness::new(0, false, 1);
    {
        let mut data = harness.plugin.params.shared_sequences.lock();
        data.selected_sequence_mut(0).1.values = vec![15, 12, 9, 6, 3];
        *data.sequence_enabled_mut(0) = true;
    }

    let steps = harness.plugin.playheads.handle();
    assert_eq!(snapshot(&steps).step(0), None, "idle before any note");

    // Long enough to cross several 60 Hz sequence frames.
    harness.block(&[note_on(0, 60, 1.0)], 2048);

    assert!(
        snapshot(&steps).step(0).is_some(),
        "a running volume sequence must report a playhead"
    );
}

#[test]
fn unchanged_sequence_state_is_not_re_cloned() {
    let mut plugin = Rp2a03Plugin::default();
    let shared = plugin.params.shared_sequences.clone();

    assert!(
        plugin.sequences.refresh(&shared, 0).is_some(),
        "the first refresh is a slot load"
    );
    assert!(
        plugin.sequences.refresh(&shared, 0).is_none(),
        "an unchanged slot must short-circuit"
    );

    // Editing the slot bumps the revision, which forces a re-clone but is not a
    // slot switch, so the sequence players are left running.
    shared.lock().selected_sequence_mut(0).1.values.push(5);
    assert!(
        plugin.sequences.refresh(&shared, 0).is_none(),
        "an in-place edit must not re-point the sequence players"
    );
    assert_eq!(plugin.sequences.active.vol_seq.values, vec![5]);
}

#[test]
fn switching_slots_reports_a_reload() {
    let mut plugin = Rp2a03Plugin::default();
    let shared = plugin.params.shared_sequences.clone();
    plugin.sequences.refresh(&shared, 0);

    shared.lock().set_all_selected_sequence_indices(1);
    shared.lock().selected_sequence_mut(0).1.values.push(7);
    shared.lock().set_all_selected_sequence_indices(0);

    let reload = plugin
        .sequences
        .refresh(&shared, 1)
        .expect("a slot switch must report a reload");
    assert!(reload.vol, "the changed envelope must be flagged");
    assert!(!reload.arp, "an identical envelope must not be flagged");
}

// ---------------------------------------------------------------------------
// Block-size safety
// ---------------------------------------------------------------------------
//
// `BlipBuf` can only resample `BlipBuf::capacity()` samples per frame, and
// nothing about the host's block size is bounded by that. These pin the two
// paths that used to hand a whole oversized block straight to `clocks_needed`
// and panic on the audio thread.

/// Blocks past one blip frame, at every size a host might realistically pick.
const OVERSIZED_BLOCKS: [usize; 3] = [4096, 8192, 16384];

#[test]
fn an_idle_oversized_block_renders_without_panicking() {
    // No gated voice: `render` takes the whole remaining block in one segment.
    for block in OVERSIZED_BLOCKS {
        let mut harness = Harness::new(0, false, 1);
        harness.block(&[], block);
        assert_eq!(harness.rendered.len(), block);
    }
}

#[test]
fn a_gated_oversized_block_renders_without_panicking() {
    for block in OVERSIZED_BLOCKS {
        let mut harness = Harness::new(0, false, 1);
        harness.block(&[note_on(0, 60, 1.0)], block);
        assert_eq!(harness.rendered.len(), block);
    }
}

#[test]
fn the_slowest_step_time_does_not_defeat_the_block_clamp() {
    // Step Time 1 Hz puts the next envelope frame 44100 samples away, so the
    // sequence-frame split alone leaves the block whole.
    for block in OVERSIZED_BLOCKS {
        let mut harness = Harness::new(0, false, 1);
        set_int(&harness.plugin.params.step_time, 1);
        harness.block(&[note_on(0, 60, 1.0)], block);
        assert_eq!(harness.rendered.len(), block);
    }
}

#[test]
fn an_oversized_polyphonic_block_renders_without_panicking() {
    let mut harness = Harness::new(0, true, MAX_VOICES as i32);
    set_int(&harness.plugin.params.step_time, 1);
    harness.block(
        &[
            note_on(0, 60, 1.0),
            note_on(0, 64, 1.0),
            note_on(0, 67, 1.0),
        ],
        8192,
    );
    assert_eq!(harness.rendered.len(), 8192);
}

#[test]
fn a_split_block_matches_the_same_audio_rendered_in_one_call() {
    // Segmenting is an internal detail: 4096 samples as one block must equal
    // 4096 samples as eight 512-sample blocks.
    let mut whole = Harness::new(0, false, 1);
    whole.block(&[note_on(0, 60, 1.0)], 4096);

    let mut split = Harness::new(0, false, 1);
    split.block(&[note_on(0, 60, 1.0)], 512);
    for _ in 0..7 {
        split.block(&[], 512);
    }

    assert_eq!(whole.rendered.len(), split.rendered.len());
    assert_eq!(whole.fingerprint(), split.fingerprint());
}

// ---------------------------------------------------------------------------
// Real-time lock discipline
// ---------------------------------------------------------------------------

#[test]
fn a_block_rendered_while_the_editor_holds_the_lock_keeps_its_cached_envelopes() {
    let mut harness = Harness::new(0, false, 1);
    let shared = harness.plugin.params.shared_sequences.clone();

    // Load an envelope and let the audio thread cache it.
    {
        let mut data = shared.lock();
        *data.sequence_enabled_mut(0) = true;
        data.selected_sequence_mut(0).1.values.push(9);
    }
    harness.block(&[note_on(0, 60, 1.0)], 512);
    assert_eq!(harness.plugin.sequences.active.vol_seq.values, vec![9]);

    // Now hold the lock the way the editor does for a whole repaint. The audio
    // thread must render through it rather than block.
    let held = shared.lock();
    harness.block(&[], 512);
    drop(held);

    assert_eq!(
        harness.plugin.sequences.active.vol_seq.values,
        vec![9],
        "a skipped refresh must leave the cached envelope intact"
    );
    assert_eq!(harness.rendered.len(), 1024, "rendering must still happen");
}
