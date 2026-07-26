//! rp2a03_common\src\midi\tests.rs
//! Tests for `MidiHandler` and its sequence/pitch/envelope processing.

use super::*;
use nice_plug::prelude::*;
use rp2a03_core::apu_pulse::{Pulse, PulseChannel};
use rp2a03_core::sequencer::{PitchMode, SeqState, Sequence};
use rp2a03_core::NTSC_CPU_CLOCK;

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
    });

    assert_eq!(handler.lfo.vibrato_depth, 7);
    assert_eq!(handler.lfo.vibrato_speed, 20);
    assert_eq!(handler.lfo.tremolo_depth, 9);
    assert_eq!(handler.lfo.tremolo_speed, 30);
    assert_eq!(handler.hardware_volume, 11);
    assert_eq!(handler.fine_pitch, -24);
}

#[test]
fn program_change_returns_its_sequence_index() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let sequences = default_seqs();

    let index = handler.handle_event(
        &NoteEvent::<()>::MidiProgramChange {
            timing: 0,
            channel: 0,
            program: 42,
        },
        &mut pulse,
        &sequences,
    );

    assert_eq!(index, Some(42));
}

#[test]
fn test_relative_and_absolute_pitch_modes() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let base = test_base_period();

    let mut pitch_seq = Sequence::parse("1 2 3");
    pitch_seq.pitch_mode = PitchMode::Relative;

    let seqs_rel = ActiveSequences {
        pitch_seq: pitch_seq.clone(),
        pitch_enabled: true,
        ..default_seqs()
    };

    // dn RunNote: working period starts at the note period; step 0 folds immediately
    handler.note_on(60, 127, &mut pulse, &seqs_rel);
    assert_eq!(handler.macro_period, base + 1);

    // dn SETTING_PITCH_RELATIVE: SetPeriod(GetPeriod() + Value) per tick
    handler.update_modulation(&mut pulse, &seqs_rel, 60.0, 1);
    assert_eq!(handler.macro_period, base + 3); // +2
    handler.update_modulation(&mut pulse, &seqs_rel, 60.0, 1);
    assert_eq!(handler.macro_period, base + 6); // +3
                                                // Sequence ended: dn END/HALT states process nothing, so it must hold
    handler.update_modulation(&mut pulse, &seqs_rel, 60.0, 1);
    assert_eq!(handler.macro_period, base + 6);

    pitch_seq.pitch_mode = PitchMode::Absolute;
    let seqs_abs = ActiveSequences {
        pitch_seq,
        ..seqs_rel
    };

    // dn SETTING_PITCH_ABSOLUTE: SetPeriod(TriggerNote(GetNote()) + Value) per tick
    handler.note_on(60, 127, &mut pulse, &seqs_abs);
    assert_eq!(handler.macro_period, base + 1);
    handler.update_modulation(&mut pulse, &seqs_abs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 2);
    handler.update_modulation(&mut pulse, &seqs_abs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 3);
    handler.update_modulation(&mut pulse, &seqs_abs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 3); // ended: sticky
}

#[test]
fn hipitch_always_accumulates_regardless_of_pitch_mode() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
    let base = test_base_period();

    // Only a hi-pitch sequence; dn: SetPeriod(GetPeriod() + (Value << 4)) — always
    // relative/accumulating, with no mode setting of its own.
    let seqs = ActiveSequences {
        hipitch_seq: Sequence::parse("1 2 3"),
        hipitch_enabled: true,
        ..default_seqs()
    };

    handler.note_on(60, 127, &mut pulse, &seqs);
    assert_eq!(handler.macro_period, base + 16); // step 0: 1 << 4
    handler.update_modulation(&mut pulse, &seqs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 16 + 32); // step 1: 2 << 4
    handler.update_modulation(&mut pulse, &seqs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 48 + 48); // step 2: 3 << 4
    handler.update_modulation(&mut pulse, &seqs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 96); // ended: sticky
}

#[test]
fn absolute_pitch_replaces_arp_and_prior_accumulation_each_tick() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
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

    handler.note_on(60, 127, &mut pulse, &seqs);
    // Trigger folds step 0 of each in order: arp period(76), then absolute pitch
    // replaces with base + 0, then hi-pitch adds 16.
    assert_eq!(handler.macro_period, base + 16);

    handler.update_modulation(&mut pulse, &seqs, 60.0, 1);
    // tick: arp replaces with period(76), absolute pitch replaces with base + 0,
    // hi-pitch adds 2 << 4
    assert_eq!(handler.macro_period, base + 32);

    handler.update_modulation(&mut pulse, &seqs, 60.0, 1);
    // hi-pitch sequence ended (2 items): keeps accumulating nothing; absolute pitch
    // still rewrites the period to base + 0 each tick
    assert_eq!(handler.macro_period, base);
}

#[test]
fn macro_period_clamps_to_0x7ff_on_every_tick_like_dn_setperiod() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);
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
    handler.note_on(60, 127, &mut pulse, &seqs); // base + 2032 -> clamped
    assert_eq!(handler.macro_period, 0x7FF);
    handler.update_modulation(&mut pulse, &seqs, 60.0, 1); // +2032 again -> clamped
    assert_eq!(handler.macro_period, 0x7FF);
    handler.update_modulation(&mut pulse, &seqs, 60.0, 1); // -2032
    assert_eq!(handler.macro_period, 0x7FF - 2032);
    handler.update_modulation(&mut pulse, &seqs, 60.0, 1); // -2032 again -> clamped at 0
    assert_eq!(handler.macro_period, 0);
    // Ended: holds the clamped value rather than unwinding back toward `base`
    handler.update_modulation(&mut pulse, &seqs, 60.0, 1);
    assert_eq!(handler.macro_period, 0);
}

#[test]
fn arpeggio_replaces_working_period_each_tick_wiping_relative_pitch_accumulation() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);

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

    handler.note_on(60, 127, &mut pulse, &seqs);
    assert_eq!(handler.macro_period, test_base_period() + 1); // arp 0, pitch +1

    handler.update_modulation(&mut pulse, &seqs, 60.0, 1);
    let period_arp4 = freq_to_period(midi_note_to_freq(76)) as i32;
    assert_eq!(handler.macro_period, period_arp4 + 2); // arp replaces, then +2

    handler.update_modulation(&mut pulse, &seqs, 60.0, 1);
    let period_arp7 = freq_to_period(midi_note_to_freq(79)) as i32;
    assert_eq!(handler.macro_period, period_arp7 + 3); // NOT accumulating across ticks
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
    handler.note_on(60, 127, &mut pulse, &active_seqs);
    assert_eq!(handler.vol_seq_player.value(), 15);
    assert_eq!(handler.arp_seq_player.value(), 0);
    assert_eq!(handler.duty_seq_player.value(), 0);
    assert_eq!(handler.pitch_seq_player.value(), 1);
    assert_eq!(handler.hipitch_seq_player.value(), 0);
    // arp step 0 (= 0) replaces, relative pitch +1, hi-pitch step 0 (= 0)
    assert_eq!(handler.macro_period, test_base_period() + 1);

    // Frame 1 tick (16.6ms): Step 1 evaluated
    handler.update_modulation(&mut pulse, &active_seqs, 60.0, 1);
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
    handler.update_modulation(&mut pulse, &active_seqs, 60.0, 1);
    assert_eq!(handler.vol_seq_player.value(), 5);
    assert_eq!(handler.arp_seq_player.value(), 7);
    assert_eq!(handler.duty_seq_player.value(), 2);
    assert_eq!(handler.pitch_seq_player.value(), 3);
    assert_eq!(handler.hipitch_seq_player.value(), 2);
    let period_arp7 = freq_to_period(midi_note_to_freq(79)) as i32;
    assert_eq!(handler.macro_period, period_arp7 + 3 + 32);

    // Frame 3 tick: all sequences finished (3 items each); macro period holds
    handler.update_modulation(&mut pulse, &active_seqs, 60.0, 1);
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

    handler.note_on(60, 127, &mut pulse, &seqs);
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
fn note_off_release_does_not_process_the_release_step_early() {
    let mut handler = MidiHandler::new();
    let mut pulse = Pulse::new(PulseChannel::One);

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
    handler.note_on(60, 127, &mut pulse, &seqs);
    assert_eq!(handler.macro_period, base + 1);

    handler.note_off(60, &mut pulse, &seqs);
    // 1:1 with dn ReleaseInstrument: the pointer jumps now but the release step's
    // value is only applied on the next 60 Hz engine tick
    assert_eq!(handler.macro_period, base + 1);
    assert_eq!(handler.pitch_seq_player.state, SeqState::Running);

    handler.update_modulation(&mut pulse, &seqs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 1 + 5);

    // Release tail finished: END processes nothing more
    handler.update_modulation(&mut pulse, &seqs, 60.0, 1);
    assert_eq!(handler.macro_period, base + 6);
}