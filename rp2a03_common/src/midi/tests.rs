//! rp2a03_common\src\midi\tests.rs
//! Tests for `MidiHandler` and its sequence/pitch/envelope processing.

use super::handler::AnyChannel;
use super::*;
use nice_plug::prelude::*;
use rp2a03_core::NTSC_CPU_CLOCK;
use rp2a03_core::apu_pulse::{Pulse, PulseChannel};
use rp2a03_core::apu_triangle::Triangle;
use rp2a03_core::sequencer::{ArpMode, PitchMode, SeqState, Sequence};

fn default_seqs() -> ActiveSequences {
    ActiveSequences {
        vol_seq: Sequence::default(),
        vol_enabled: false,
        arp_seq: Sequence::default(),
        arp_enabled: false,
        pitch_seq: Sequence::default(),
        pitch_enabled: false,
        hipitch_seq: Sequence::default(),
        hipitch_enabled: false,
        duty_seq: Sequence::default(),
        duty_enabled: false,
    }
}

/// Period of MIDI 72 (note 60 + the default +12 octave offset) — the dn
/// `TriggerNote` equivalent for these tests.
fn test_base_period() -> i32 {
    freq_to_period(midi_note_to_freq(72)) as i32
}

#[test]
fn host_automation_controls_update_the_matching_synth_controls() {
    let mut handler = MidiHandler::new();

    handler.apply_host_automation(HostAutomationControls {
        vibrato_depth: 7,
        vibrato_speed: 20,
        tremolo_depth: 9,
        tremolo_speed: 30,
        hardware_volume: 11,
        fine_pitch: -24,
        hi_pitch: 5,
        step_time_hz: 120,
    });

    assert_eq!(handler.lfo.vibrato_depth, 7);
    assert_eq!(handler.lfo.vibrato_speed, 20);
    assert_eq!(handler.lfo.tremolo_depth, 9);
    assert_eq!(handler.lfo.tremolo_speed, 30);
    assert_eq!(handler.hardware_volume, 11);
    assert_eq!(handler.fine_pitch, -24);
    assert_eq!(handler.hi_pitch, 5);
    assert_eq!(handler.step_time_hz, 120);
}

#[test]
fn triangle_channel_produces_non_zero_output_on_note_on() {
    let mut handler = MidiHandler::new();
    handler.channel_mode = ChannelMode::Triangle;
    let mut pulse = Pulse::new(PulseChannel::One);
    let mut triangle = Triangle::new();
    let seqs = default_seqs();

    handler.note_on(60, 127, &mut AnyChannel::Triangle(&mut triangle), &seqs);
    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 44100.0, 1);

    // Length counter reload occurs on triangle.clock()
    triangle.clock();

    assert!(!triangle.is_muted());
    assert!(
        triangle.output() > 0.0,
        "Triangle output should be non-zero after note trigger and clock, got {}",
        triangle.output()
    );

    // Clock the triangle timer and verify that sequence steps
    let initial_step = triangle.sequence;
    for _ in 0..1000 {
        triangle.clock();
    }
    assert_ne!(
        triangle.sequence, initial_step,
        "Triangle sequencer step should advance when clocked"
    );
}

#[test]
fn program_change_returns_its_sequence_index() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let mut triangle = Triangle::new();
    let sequences = default_seqs();

    let index = handler.handle_event(
        &NoteEvent::<()>::MidiProgramChange {
            timing: 0,
            channel: 0,
            program: 42,
        },
        &mut pulse,
        &mut triangle,
        &sequences,
    );

    assert_eq!(index, Some(42));
}

#[test]
fn test_relative_and_absolute_pitch_modes() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let mut triangle = Triangle::new();

    let base = test_base_period();

    let mut pitch_seq = Sequence::parse("1 2 3");
    pitch_seq.pitch_mode = PitchMode::Relative;

    let seqs_rel = ActiveSequences {
        pitch_seq: pitch_seq.clone(),
        pitch_enabled: true,
        ..default_seqs()
    };

    // dn RunNote: working period starts at the note period; step 0 folds immediately
    handler.note_on(60, 127, &mut AnyChannel::Pulse(&mut pulse), &seqs_rel);
    assert_eq!(handler.macro_period, base + 1);

    // dn SETTING_PITCH_RELATIVE: SetPeriod(GetPeriod() + Value) per tick
    handler.update_modulation(&mut pulse, &mut triangle, &seqs_rel, 60.0, 1);
    assert_eq!(handler.macro_period, base + 3); // +2

    handler.update_modulation(&mut pulse, &mut triangle, &seqs_rel, 60.0, 1);
    assert_eq!(handler.macro_period, base + 6); // +3

    // Sequence ended: dn END/HALT states process nothing, so it must hold
    handler.update_modulation(&mut pulse, &mut triangle, &seqs_rel, 60.0, 1);
    assert_eq!(handler.macro_period, base + 6);

    pitch_seq.pitch_mode = PitchMode::Absolute;

    let seqs_abs = ActiveSequences {
        pitch_seq,
        ..seqs_rel
    };

    // dn SETTING_PITCH_ABSOLUTE: SetPeriod(TriggerNote(GetNote()) + Value) per tick
    handler.note_on(60, 127, &mut AnyChannel::Pulse(&mut pulse), &seqs_abs);
    assert_eq!(handler.macro_period, base + 1);

    handler.update_modulation(&mut pulse, &mut triangle, &seqs_abs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 2);

    handler.update_modulation(&mut pulse, &mut triangle, &seqs_abs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 3);

    handler.update_modulation(&mut pulse, &mut triangle, &seqs_abs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 3); // ended: sticky
}

#[test]
fn hipitch_always_accumulates_regardless_of_pitch_mode() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let mut triangle = Triangle::new();

    let base = test_base_period();

    // Only a hi-pitch sequence; dn: SetPeriod(GetPeriod() + (Value << 4)) — always
    // relative/accumulating, with no mode setting of its own.
    let seqs = ActiveSequences {
        hipitch_seq: Sequence::parse("1 2 3"),
        hipitch_enabled: true,
        ..default_seqs()
    };

    handler.note_on(60, 127, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(handler.macro_period, base + 16); // step 0: 1 << 4

    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 16 + 32); // step 1: 2 << 4

    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 48 + 48); // step 2: 3 << 4

    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 96); // ended: sticky
}

#[test]
fn absolute_pitch_replaces_arp_and_prior_accumulation_each_tick() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let mut triangle = Triangle::new();

    let base = test_base_period();

    // dn order quirk (UpdateInstrument: pitch runs after arpeggio): an absolute
    // pitch setting does SetPeriod(TriggerNote(GetNote()) + Value), replacing any
    // period the arpeggio sequence set earlier in the same tick.
    let mut pitch_seq = Sequence::parse("0 0 0 0");
    pitch_seq.pitch_mode = PitchMode::Absolute;

    let seqs = ActiveSequences {
        arp_seq: Sequence::parse("4 4 4 4"),
        arp_enabled: true,
        pitch_seq,
        pitch_enabled: true,
        hipitch_seq: Sequence::parse("1 2"),
        hipitch_enabled: true,
        ..default_seqs()
    };

    handler.note_on(60, 127, &mut AnyChannel::Pulse(&mut pulse), &seqs);

    // Trigger folds step 0 of each in order: arp period(76), then absolute pitch
    // replaces with base + 0, then hi-pitch adds 16.
    assert_eq!(handler.macro_period, base + 16);

    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1);
    // tick: arp replaces with period(76), absolute pitch replaces with base + 0,
    // hi-pitch adds 2 << 4
    assert_eq!(handler.macro_period, base + 32);

    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1);
    // hi-pitch sequence ended (2 items): keeps accumulating nothing; absolute pitch
    // still rewrites the period to base + 0 each tick
    assert_eq!(handler.macro_period, base);
}

#[test]
fn macro_period_clamps_to_0x7ff_on_every_tick_like_dn_setperiod() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let mut triangle = Triangle::new();

    let base = test_base_period();
    assert!(
        base + 2 * (127 << 4) > 0x7FF,
        "test requires hitting the upper rail"
    );

    let seqs = ActiveSequences {
        hipitch_seq: Sequence::parse("127 127 -127 -127"),
        hipitch_enabled: true,
        ..default_seqs()
    };

    // dn clamps via LimitPeriod inside every SetPeriod call; overshoot past the
    // rail is discarded instead of being soaked up by an unbounded accumulator.
    handler.note_on(60, 127, &mut AnyChannel::Pulse(&mut pulse), &seqs); // base + 2032 -> clamped
    assert_eq!(handler.macro_period, 0x7FF);

    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1); // +2032 again -> clamped
    assert_eq!(handler.macro_period, 0x7FF);

    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1); // -2032
    assert_eq!(handler.macro_period, 0x7FF - 2032);

    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1); // -2032 again -> clamped at 0
    assert_eq!(handler.macro_period, 0);

    // Ended: holds the clamped value rather than unwinding back toward `base`
    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1);
    assert_eq!(handler.macro_period, 0);
}

#[test]
fn arpeggio_replaces_working_period_each_tick_wiping_relative_pitch_accumulation() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let mut triangle = Triangle::new();

    // dn quirk preserved for 1:1: while an absolute arpeggio sequence runs, its
    // per-tick SetPeriod discards the relative pitch accumulation every frame.
    let mut pitch_seq = Sequence::parse("1 2 3");
    pitch_seq.pitch_mode = PitchMode::Relative;

    let seqs = ActiveSequences {
        arp_seq: Sequence::parse("0 4 7"),
        arp_enabled: true,
        pitch_seq,
        pitch_enabled: true,
        ..default_seqs()
    };

    handler.note_on(60, 127, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(handler.macro_period, test_base_period() + 1); // arp 0, pitch +1

    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1);
    let period_arp4 = freq_to_period(midi_note_to_freq(76)) as i32;
    assert_eq!(handler.macro_period, period_arp4 + 2); // arp replaces, then +2

    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1);
    let period_arp7 = freq_to_period(midi_note_to_freq(79)) as i32;
    assert_eq!(handler.macro_period, period_arp7 + 3); // NOT accumulating across ticks
}

#[test]
fn arpeggio_relative_mode_mutates_active_note_accumulating() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let mut triangle = Triangle::new();

    let mut arp_seq = Sequence::parse("2 3"); // +2 semitones, then +3 semitones
    arp_seq.arp_mode = ArpMode::Relative;

    let seqs = ActiveSequences {
        arp_seq,
        arp_enabled: true,
        ..default_seqs()
    };

    // NoteOn: MIDI note 60 (active_note = 60). Step 0 is +2 -> active_note becomes 62.
    handler.note_on(60, 127, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(handler.active_note, 62);

    // Tick 1: step 1 is +3 -> active_note becomes 65.
    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1);
    assert_eq!(handler.active_note, 65);
}

#[test]
fn test_famitracker_reference_key_pitch_frequencies() {
    let base_freq = 522.71f32;
    let base_period = freq_to_period(base_freq); // 213

    // Value +127: period = 213 + 127 = 340 -> ~328.04 Hz
    let period_127 = (base_period as i32 + 127) as u16;
    let freq_127 = NTSC_CPU_CLOCK as f32 / (16.0 * (period_127 as f32 + 0.5));
    assert!(
        (freq_127 - 328.04).abs() < 1.0,
        "Expected ~328.04 Hz, got {}",
        freq_127
    );

    // Value -128: period = 213 - 128 = 85 -> ~1300.71 Hz
    let period_minus_128 = (base_period as i32 - 128) as u16;
    let freq_minus_128 = NTSC_CPU_CLOCK as f32 / (16.0 * (period_minus_128 as f32 + 0.5));
    assert!(
        (freq_minus_128 - 1300.71).abs() < 10.0,
        "Expected ~1300.71 Hz, got {}",
        freq_minus_128
    );
}

#[test]
fn test_all_envelope_editor_timings_1to1_famitracker() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let mut triangle = Triangle::new();

    let vol_seq = Sequence::parse("15 10 5");
    let arp_seq = Sequence::parse("0 4 7");
    let mut pitch_seq = Sequence::parse("1 2 3");
    pitch_seq.pitch_mode = PitchMode::Relative;
    let hipitch_seq = Sequence::parse("0 1 2");
    let duty_seq = Sequence::parse("0 1 2");

    let active_seqs = ActiveSequences {
        vol_seq,
        vol_enabled: true,
        arp_seq,
        arp_enabled: true,
        pitch_seq,
        pitch_enabled: true,
        hipitch_seq,
        hipitch_enabled: true,
        duty_seq,
        duty_enabled: true,
    };

    // On NoteOn attack (Frame 0): Step 0 is evaluated immediately across all envelope types
    handler.note_on(60, 127, &mut AnyChannel::Pulse(&mut pulse), &active_seqs);

    assert_eq!(handler.vol_seq_player.value(), 15);
    assert_eq!(handler.arp_seq_player.value(), 0);
    assert_eq!(handler.duty_seq_player.value(), 0);
    assert_eq!(handler.pitch_seq_player.value(), 1);
    assert_eq!(handler.hipitch_seq_player.value(), 0);

    // arp step 0 (= 0) replaces, relative pitch +1, hi-pitch step 0 (= 0)
    assert_eq!(handler.macro_period, test_base_period() + 1);

    // Frame 1 tick (16.6ms): Step 1 evaluated
    handler.update_modulation(&mut pulse, &mut triangle, &active_seqs, 60.0, 1);

    assert_eq!(handler.vol_seq_player.value(), 10);
    assert_eq!(handler.arp_seq_player.value(), 4);
    assert_eq!(handler.duty_seq_player.value(), 1);
    assert_eq!(handler.pitch_seq_player.value(), 2);
    assert_eq!(handler.hipitch_seq_player.value(), 1);

    // arp step 4 replaces the working period (dn quirk: accumulated relative
    // pitch is discarded by the arp SetPeriod), then pitch +2, hi-pitch +16
    let period_arp4 = freq_to_period(midi_note_to_freq(76)) as i32;
    assert_eq!(handler.macro_period, period_arp4 + 2 + 16);

    // Frame 2 tick (33.3ms): Step 2 evaluated
    handler.update_modulation(&mut pulse, &mut triangle, &active_seqs, 60.0, 1);

    assert_eq!(handler.vol_seq_player.value(), 5);
    assert_eq!(handler.arp_seq_player.value(), 7);
    assert_eq!(handler.duty_seq_player.value(), 2);
    assert_eq!(handler.pitch_seq_player.value(), 3);
    assert_eq!(handler.hipitch_seq_player.value(), 2);

    let period_arp7 = freq_to_period(midi_note_to_freq(79)) as i32;
    assert_eq!(handler.macro_period, period_arp7 + 3 + 32);

    // Frame 3 tick: all sequences finished (3 items each); macro period holds
    handler.update_modulation(&mut pulse, &mut triangle, &active_seqs, 60.0, 1);
    assert_eq!(handler.macro_period, period_arp7 + 3 + 32);
}

#[test]
fn envelope_ticks_land_on_sample_boundaries_inside_large_host_buffers() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);

    let seqs = ActiveSequences {
        vol_seq: Sequence::parse("15 10 5"),
        vol_enabled: true,
        ..default_seqs()
    };

    handler.note_on(60, 127, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(handler.vol_seq_player.value(), 15);

    assert_eq!(handler.samples_until_next_frame(44_100.0), 735);

    handler.advance_frame_samples(&seqs, 44_100.0, 734);
    assert_eq!(
        handler.vol_seq_player.value(),
        15,
        "step 1 must not be applied early at the start of a large host buffer"
    );

    handler.advance_frame_samples(&seqs, 44_100.0, 1);
    assert_eq!(handler.vol_seq_player.value(), 10);

    assert_eq!(handler.samples_until_next_frame(44_100.0), 735);

    handler.advance_frame_samples(&seqs, 44_100.0, 735);
    assert_eq!(handler.vol_seq_player.value(), 5);
}

#[test]
fn note_on_restarts_frame_phase_for_a_full_attack_step() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);

    let seqs = ActiveSequences {
        vol_seq: Sequence::parse("15 10 5"),
        vol_enabled: true,
        ..default_seqs()
    };

    handler.note_on(60, 127, &mut AnyChannel::Pulse(&mut pulse), &seqs);

    handler.advance_frame_samples(&seqs, 44_100.0, 300);
    assert_eq!(handler.samples_until_next_frame(44_100.0), 435);

    handler.note_on(62, 127, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(handler.vol_seq_player.value(), 15);
    assert_eq!(handler.samples_until_next_frame(44_100.0), 735);

    handler.advance_frame_samples(&seqs, 44_100.0, 734);
    assert_eq!(handler.vol_seq_player.value(), 15);

    handler.advance_frame_samples(&seqs, 44_100.0, 1);
    assert_eq!(handler.vol_seq_player.value(), 10);
}

#[test]
fn note_off_release_processes_release_step_on_the_release_tick() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);

    let seqs = ActiveSequences {
        vol_seq: Sequence::parse("15 14 / 9 0"),
        vol_enabled: true,
        ..default_seqs()
    };

    handler.note_on(60, 127, &mut AnyChannel::Pulse(&mut pulse), &seqs);

    handler.advance_frame_samples(&seqs, 44_100.0, 300);

    handler.note_off(60, &mut AnyChannel::Pulse(&mut pulse), &seqs);

    assert_eq!(
        handler.vol_seq_player.value(),
        9,
        "release step must be applied in the same engine tick as the release"
    );
    assert_eq!(handler.samples_until_next_frame(44_100.0), 735);

    handler.advance_frame_samples(&seqs, 44_100.0, 734);
    assert_eq!(handler.vol_seq_player.value(), 9);

    handler.advance_frame_samples(&seqs, 44_100.0, 1);
    assert_eq!(handler.vol_seq_player.value(), 0);
}

#[test]
fn note_off_release_processes_pitch_release_step_on_the_release_tick() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let mut triangle = Triangle::new();

    let mut pitch_seq = Sequence::parse("1 / 5");
    pitch_seq.pitch_mode = PitchMode::Relative;

    let seqs = ActiveSequences {
        pitch_seq,
        pitch_enabled: true,
        vol_seq: Sequence::parse("15 / 12"),
        vol_enabled: true,
        ..default_seqs()
    };

    let base = test_base_period();

    handler.note_on(60, 127, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(handler.macro_period, base + 1);

    handler.note_off(60, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    // 1:1 with dn ReleaseInstrument: the pointer jumps before UpdateInstrument,
    // so the release step is consumed in the same engine tick.
    assert_eq!(handler.macro_period, base + 1 + 5);
    assert_eq!(handler.pitch_seq_player.state, SeqState::End);

    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 1 + 5);

    // Release tail finished: END processes nothing more
    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 6);
}

#[test]
fn triangle_timer_is_halved_for_octave_parity_with_pulse() {
    let mut handler = MidiHandler::new();
    handler.channel_mode = ChannelMode::Triangle;
    let mut pulse = Pulse::new(PulseChannel::One);
    let mut triangle = Triangle::new();
    let seqs = default_seqs();

    handler.note_on(60, 127, &mut AnyChannel::Triangle(&mut triangle), &seqs);
    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 44100.0, 1);

    // Pulse-domain period for note 72 (C4 + the default +12 octave offset) is 213.
    // The triangle sequencer runs at twice the pulse's clock ratio, so matching
    // pitch needs (213 - 1) / 2 = 106: CPU/32(106+1) == CPU/16(213.5) ≈ 522.6 Hz.
    assert_eq!(handler.prev_timer_lo, ((test_base_period() - 1) / 2) as u8);
    assert_eq!(handler.prev_timer_hi, 0);
}

#[test]
fn waveform_switch_rewrites_registers_even_when_timer_bytes_match_the_cache() {
    // Regression for the "first note after switching sounds wrong, next note
    // corrects it" bug: register caches are handler-level, but each channel keeps
    // its own registers, so a matching byte must not suppress the write after a
    // waveform switch.

    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let mut triangle = Triangle::new();

    handler.channel_mode = ChannelMode::Triangle;
    let seqs = default_seqs();

    // MIDI 48 on the triangle → note 60 → pulse-domain period 427, compensated to
    // (427 - 1) / 2 = 213 → low byte 0xD5 now sits in the cache against TRIANGLE state.
    handler.note_on(48, 127, &mut AnyChannel::Triangle(&mut triangle), &seqs);
    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 44100.0, 1);
    assert_eq!(handler.prev_timer_lo, 213);

    // Switch to pulse and play MIDI 60 → also period 213, low byte 0xD5 — equal to
    // the cache. If the timer-low write were skipped (the old behavior), the Pulse
    // struct's period would stay 0 and the channel would read as muted.
    handler.note_off(48, &mut AnyChannel::Triangle(&mut triangle), &seqs);
    handler.channel_mode = ChannelMode::Pulse;

    handler.note_on(60, 127, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 44100.0, 1);
    assert!(
        !pulse.is_muted(),
        "pulse period must actually be written after a waveform switch"
    );
    pulse.clock(); // applies the length-counter reload from write_timer_hi
    assert_eq!(pulse.volume(), 15);
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests for the note-stack voice-stealing / note-return fix
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn releasing_top_note_returns_to_previously_held_note() {
    // The core scenario: hold C5, press D5, release D5 → should return to C5.
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let mut triangle = Triangle::new();

    let vol_seq = Sequence::parse("15 12 9 6");
    let seqs = ActiveSequences {
        vol_seq,
        vol_enabled: true,
        ..default_seqs()
    };

    // Press C5 (MIDI 72)
    handler.note_on(72, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(handler.active_note, 72);
    assert_eq!(handler.vol_seq_player.value(), 15);

    // Advance a tick to move sequences forward
    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 60.0, 1);
    assert_eq!(handler.vol_seq_player.value(), 12);

    // Press D5 (MIDI 74) while still holding C5
    handler.note_on(74, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(handler.active_note, 74);
    assert_eq!(
        handler.vol_seq_player.value(),
        15,
        "sequences should restart on new note"
    );

    // Release D5 while C5 is still held
    handler.note_off(74, &mut AnyChannel::Pulse(&mut pulse), &seqs);

    // Should switch back to C5 and retrigger sequences
    assert_eq!(
        handler.active_note, 72,
        "active note should return to C5 (72) after releasing D5"
    );
    assert_eq!(
        handler.vol_seq_player.value(),
        15,
        "volume sequence should retrigger for the restored note"
    );
    assert!(handler.gate, "gate should remain on while C5 is still held");
}

#[test]
fn releasing_top_note_recalculates_macro_period_for_held_note() {
    // Verify that macro_period is recalculated when switching back to a held note.
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);

    let seqs = default_seqs();

    let period_c5 = freq_to_period(midi_note_to_freq(72 + 12)) as i32; // C5 + octave offset
    let period_d5 = freq_to_period(midi_note_to_freq(74 + 12)) as i32; // D5 + octave offset

    // Press C5
    handler.note_on(72, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(handler.macro_period, period_c5);

    // Press D5 while holding C5
    handler.note_on(74, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(handler.macro_period, period_d5);

    // Release D5 → should return macro_period to C5
    handler.note_off(74, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(
        handler.macro_period, period_c5,
        "macro_period should return to C5's period after releasing D5"
    );
}

#[test]
fn releasing_top_note_restarts_frame_counter() {
    // The frame_sample_counter should be reset so the restored note gets a full
    // attack step duration before the first envelope tick.
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);

    let seqs = ActiveSequences {
        vol_seq: Sequence::parse("15 10 5"),
        vol_enabled: true,
        ..default_seqs()
    };

    // Press C5
    handler.note_on(72, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs);

    // Advance partway through a frame
    handler.advance_frame_samples(&seqs, 44_100.0, 300);
    assert_eq!(handler.samples_until_next_frame(44_100.0), 435);

    // Press D5 while holding C5
    handler.note_on(74, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs);

    // Advance partway again
    handler.advance_frame_samples(&seqs, 44_100.0, 200);

    // Release D5 → frame counter should reset for a full attack step
    handler.note_off(74, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(
        handler.samples_until_next_frame(44_100.0),
        735,
        "frame_sample_counter should reset so the restored note gets a full attack step"
    );
    assert_eq!(
        handler.vol_seq_player.value(),
        15,
        "volume should be at step 0"
    );
}

#[test]
fn releasing_top_note_restarts_pitch_sequences_for_held_note() {
    // Verify that pitch/arp/hipitch sequences also retrigger when returning
    // to a held note.
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);

    let mut pitch_seq = Sequence::parse("1 2 3");
    pitch_seq.pitch_mode = PitchMode::Relative;

    let seqs = ActiveSequences {
        pitch_seq,
        pitch_enabled: true,
        ..default_seqs()
    };

    let base_c5 = freq_to_period(midi_note_to_freq(72 + 12)) as i32;
    let base_d5 = freq_to_period(midi_note_to_freq(74 + 12)) as i32;

    // Press C5
    handler.note_on(72, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(handler.macro_period, base_c5 + 1); // pitch step 0 = 1

    // Press D5 while holding C5
    handler.note_on(74, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(handler.macro_period, base_d5 + 1); // pitch step 0 = 1

    // Release D5 → should retrigger for C5
    handler.note_off(74, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(
        handler.macro_period,
        base_c5 + 1,
        "macro_period should be C5's base period + pitch step 0"
    );
    assert_eq!(
        handler.pitch_seq_player.value(),
        1,
        "pitch sequence should be at step 0"
    );
}

#[test]
fn three_note_stack_returns_to_second_note_on_release() {
    // Press C5, press D5, press E5, release E5 → should return to D5.
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let seqs = default_seqs();

    handler.note_on(72, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs); // C5
    handler.note_on(74, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs); // D5
    handler.note_on(76, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs); // E5

    assert_eq!(handler.active_note, 76);

    handler.note_off(76, &mut AnyChannel::Pulse(&mut pulse), &seqs); // release E5
    assert_eq!(
        handler.active_note, 74,
        "should return to D5 after releasing E5"
    );

    handler.note_off(74, &mut AnyChannel::Pulse(&mut pulse), &seqs); // release D5
    assert_eq!(
        handler.active_note, 72,
        "should return to C5 after releasing D5"
    );
}

#[test]
fn releasing_all_notes_gates_off() {
    // When the last note is released with no release sequences, gate turns off.
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let seqs = default_seqs();

    handler.note_on(72, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    handler.note_on(74, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs);

    handler.note_off(74, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert!(handler.gate, "gate should be on while C5 is still held");

    handler.note_off(72, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert!(!handler.gate, "gate should be off when all notes released");
}

#[test]
fn note_on_duplicate_note_retrigger_does_not_corrupt_stack() {
    // Pressing the same note twice should not duplicate it in the stack.
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let seqs = default_seqs();

    handler.note_on(60, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    handler.note_on(60, 80, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert_eq!(
        handler.note_stack.len(),
        1,
        "duplicate note should not be duplicated"
    );
    assert_eq!(
        handler.current_velocity, 80,
        "velocity should update to latest"
    );

    handler.note_off(60, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    assert!(
        !handler.gate,
        "gate should be off after releasing the only note"
    );
}

#[test]
fn note_off_for_another_voice_does_not_retrigger_this_voice() {
    let seqs = default_seqs();
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    pulse.set_enabled(true);

    handler.note_on(60, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    handler.advance_frame_samples(&seqs, 44_100.0, 300);
    let frame_counter_before = handler.frame_sample_counter;
    let sequence_position_before = handler.vol_seq_player.pos;

    handler.note_off(62, &mut AnyChannel::Pulse(&mut pulse), &seqs);

    assert_eq!(handler.frame_sample_counter, frame_counter_before);
    assert_eq!(handler.vol_seq_player.pos, sequence_position_before);
    assert_eq!(handler.active_note, 60);
    assert!(handler.gate);
}

#[test]
fn hi_pitch_cc15_and_host_automation_offsets_period() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let mut triangle = Triangle::new();
    let seqs = default_seqs();

    // Default hi_pitch is 0
    assert_eq!(handler.hi_pitch, 0);

    // CC 15 sets hi_pitch (value 66 -> +2)
    handler.handle_control_change(15, 66);
    assert_eq!(handler.hi_pitch, 2);

    // Trigger note and update modulation
    handler.note_on(60, 100, &mut AnyChannel::Pulse(&mut pulse), &seqs);
    let base_period = handler.macro_period;
    handler.update_modulation(&mut pulse, &mut triangle, &seqs, 44100.0, 1);

    // Final timer written should be base_period - (2 << 4) = base_period - 32
    let expected_period = (base_period - 32) as u16;
    let written_period = ((handler.prev_timer_hi as u16) << 8) | (handler.prev_timer_lo as u16);
    assert_eq!(written_period, expected_period);
}
