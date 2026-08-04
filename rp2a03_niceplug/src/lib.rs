//! rp2a03_niceplug\src\lib.rs
//! RP2A03 Plugin wrapper using nice-plug.

use nice_plug::prelude::*;
use nice_plug_egui::{EguiSettings, EguiState, create_egui_editor};
use parking_lot::Mutex;
use rp2a03_common::{
    ActiveSequences, ChannelMode, HostAutomationControls, MAX_SEQUENCES, MidiHandler,
    NO_PLAYHEAD_STEP, SEQUENCE_TYPE_COUNT, SequencePlayheads, SequenceReload, SharedSequences,
    render_editor_ui, style,
};
use rp2a03_core::NTSC_CPU_CLOCK;
use rp2a03_core::apu_noise::Noise;
use rp2a03_core::apu_pulse::{Pulse, PulseChannel};
use rp2a03_core::apu_triangle::Triangle;
use rp2a03_core::vrc6_pulse::Vrc6Pulse;
use rp2a03_core::vrc6_saw::Vrc6Saw;
use rp2a03_core::blip_buf::BlipBuf;
use rp2a03_core::sequencer::{SeqState, Sequence, SequencePlayer};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const BLIP_BUFFER_SIZE: u32 = 4096;
const AMPLITUDE_SCALE: i32 = 1500;
const TRIANGLE_AMPLITUDE_SCALE: i32 = 3000;
// Nominal synth-instance output calibration: 20*log10(0.1584893) = -16 dBFS.
// This trims the final bus without changing per-voice, polyphony, or MIDI gain.
// Voices sum freely, so this is the headroom budget for the whole mix: a single
// voice at max velocity peaks near -10 dBFS raw, and MAX_VOICES of those summing
// in phase reach ~2.53, so -16 dBFS keeps a full chord well under the output clamp
// while landing a single test note at a comfortable -26 dBFS.
const MASTER_OUTPUT_GAIN: f32 = 0.158_489_3;
const MAX_VOICES: usize = 8;
const ALLOCATION_RAMP_CLOCKS: u32 = 2048;

struct Voice {
    pulse: Pulse,
    triangle: Triangle,
    noise: Noise,
    vrc6_pulse: Vrc6Pulse,
    vrc6_saw: Vrc6Saw,
    midi_handler: MidiHandler,
    blip: BlipBuf,
    last_output: i32,
    last_triangle_output: i32,
    allocation_ramp_clocks: u32,
    alloc_id: u64,
}

impl Voice {
    fn new() -> Self {
        let mut pulse = Pulse::new(PulseChannel::One);
        pulse.set_enabled(true);
        let mut triangle = Triangle::new();
        triangle.set_enabled(true);
        let mut noise = Noise::new();
        noise.set_enabled(true);
        let mut vrc6_pulse = Vrc6Pulse::new();
        vrc6_pulse.set_enabled(true);
        let mut vrc6_saw = Vrc6Saw::new();
        vrc6_saw.set_enabled(true);
        let mut blip = BlipBuf::new(BLIP_BUFFER_SIZE);
        blip.set_rates(NTSC_CPU_CLOCK, 44100.0);
        Self {
            pulse,
            triangle,
            noise,
            vrc6_pulse,
            vrc6_saw,
            midi_handler: MidiHandler::new(),
            blip,
            last_output: 0,
            last_triangle_output: 0,
            allocation_ramp_clocks: 0,
            alloc_id: 0,
        }
    }

    fn reset(&mut self) {
        self.pulse.reset();
        self.pulse.set_enabled(true);
        self.pulse.write_sweep(0x08);
        self.triangle.reset();
        self.triangle.set_enabled(true);
        self.noise.reset();
        self.noise.set_enabled(true);
        self.vrc6_pulse.reset();
        self.vrc6_pulse.set_enabled(true);
        self.vrc6_saw.reset();
        self.vrc6_saw.set_enabled(true);
        self.midi_handler.reset();
        self.blip.clear();
        self.last_output = 0;
        self.last_triangle_output = 0;
        self.allocation_ramp_clocks = 0;
    }

    /// Reset synthesis state for voice allocation while preserving the previous
    /// output level. The next BlipBuf delta then transitions from the old note
    /// to the new note instead of introducing an artificial zero-level click.
    fn reset_for_allocation(&mut self) {
        let previous_output = self.last_output;
        let previous_triangle_output = self.last_triangle_output;
        let triangle_sequence = self.triangle.sequence;
        self.pulse.reset();
        self.pulse.set_enabled(true);
        self.pulse.write_sweep(0x08);
        self.triangle.reset();
        self.triangle.set_enabled(true);
        self.noise.reset();
        self.noise.set_enabled(true);
        self.vrc6_pulse.reset();
        self.vrc6_pulse.set_enabled(true);
        self.vrc6_saw.reset();
        self.vrc6_saw.set_enabled(true);
        // A triangle timer/register write does not reset the waveform's 32-step
        // phase. Preserve it when recycling a polyphonic voice so the DAC does
        // not jump from the held release level back to sequence step zero.
        self.triangle.sequence = triangle_sequence;
        self.midi_handler.reset();
        self.last_output = previous_output;
        self.last_triangle_output = previous_triangle_output;
        self.allocation_ramp_clocks = ALLOCATION_RAMP_CLOCKS;
    }

    /// Smooth a fresh triangle attack when the voice is reused in monophonic
    /// mode. Polyphonic voice recycling sets this as part of allocation reset,
    /// but the single voice is intentionally not reset between notes.
    fn begin_triangle_attack_ramp(&mut self) {
        if self.midi_handler.channel_mode == ChannelMode::Triangle
            && !self.midi_handler.gate()
        {
            self.allocation_ramp_clocks = ALLOCATION_RAMP_CLOCKS;
        }
    }
}


pub struct Rp2a03Plugin {
    params: Arc<Rp2a03Params>,
    voices: Vec<Voice>,
    sample_rate: f32,
    alloc_counter: u64,
    last_active_voice_count: usize,
    sample_scratch: Vec<i16>,
    /// Program Change selection remains active until the host changes the Index parameter.
    midi_program_index: Option<usize>,
    last_sequence_parameter: i32,
    sequence_playheads: Arc<[AtomicUsize; SEQUENCE_TYPE_COUNT]>,
    /// Audio-thread-owned scratch buffer for the currently active sequences.
    /// Reused in place (see `refresh_active_sequences`) so a normal `process()`
    /// call — where nothing changed since the last block — never locks
    /// `shared_sequences` for a clone or allocates.
    active_seqs: ActiveSequences,
    /// (sequence_index, shared_sequences revision) as of the last refresh.
    cached_seq_key: Option<(usize, u64)>,
}

#[derive(Params)]
struct Rp2a03Params {
    #[persist = "editor_state"]
    pub egui_state: Arc<EguiState>,

    #[persist = "envelope_data"]
    pub shared_sequences: Arc<Mutex<SharedSequences>>,

    /// Selects the same numbered sequence slot for all five pulse envelopes.
    #[id = "sequence_number"]
    pub sequence_number: IntParam,

    #[id = "vibrato_depth"]
    pub vibrato_depth: IntParam,
    #[id = "vibrato_speed"]
    pub vibrato_speed: IntParam,
    #[id = "tremolo_depth"]
    pub tremolo_depth: IntParam,
    #[id = "tremolo_speed"]
    pub tremolo_speed: IntParam,
    #[id = "hardware_volume"]
    pub hardware_volume: IntParam,
    #[id = "fine_pitch"]
    pub fine_pitch: IntParam,
    #[id = "hi_pitch"]
    pub hi_pitch: IntParam,
    #[id = "pitch_slide"]
    pub pitch_slide: IntParam,
    #[id = "pitch_slide_range"]
    pub pitch_slide_range: IntParam,
    #[id = "step_time"]
    pub step_time: IntParam,
    #[id = "waveform"]
    pub waveform: IntParam,
    #[id = "polyphony"]
    pub polyphony: BoolParam,
    #[id = "max_voices"]
    pub max_voices: IntParam,
    #[id = "portamento_enabled"]
    pub portamento_enabled: BoolParam,
    #[id = "portamento_speed"]
    pub portamento_speed: IntParam,
}

impl Default for Rp2a03Plugin {
    fn default() -> Self {
        Self {
            params: Arc::new(Rp2a03Params::default()),
            voices: (0..MAX_VOICES).map(|_| Voice::new()).collect(),
            sample_rate: 44100.0,
            alloc_counter: 0,
            last_active_voice_count: 1,
            sample_scratch: Vec::new(),
            midi_program_index: None,
            last_sequence_parameter: 0,
            sequence_playheads: Arc::new(std::array::from_fn(|_| {
                AtomicUsize::new(NO_PLAYHEAD_STEP)
            })),
            active_seqs: ActiveSequences::default(),
            cached_seq_key: None,
        }
    }
}

impl Default for Rp2a03Params {
    fn default() -> Self {
        Self {
            egui_state: EguiState::from_size(758, 590),
            shared_sequences: Arc::new(Mutex::new(SharedSequences::default())),
            sequence_number: IntParam::new(
                "Sequence Index",
                0,
                IntRange::Linear {
                    min: 0,
                    max: (MAX_SEQUENCES - 1) as i32,
                },
            ),
            vibrato_depth: IntParam::new("Vibrato Depth", 0, IntRange::Linear { min: 0, max: 15 }),
            vibrato_speed: IntParam::new("Vibrato Speed", 4, IntRange::Linear { min: 0, max: 63 }),
            tremolo_depth: IntParam::new("Tremolo Depth", 0, IntRange::Linear { min: 0, max: 15 }),
            tremolo_speed: IntParam::new("Tremolo Speed", 4, IntRange::Linear { min: 0, max: 63 }),
            hardware_volume: IntParam::new("HW Volume", 15, IntRange::Linear { min: 0, max: 15 }),
            fine_pitch: IntParam::new("Pitch", 0, IntRange::Linear { min: -64, max: 63 }),
            hi_pitch: IntParam::new("Hi-Pitch", 0, IntRange::Linear { min: -64, max: 63 }),
            pitch_slide: IntParam::new(
                "Pitch Slide",
                0,
                IntRange::Linear {
                    min: -8192,
                    max: 8191,
                },
            ),
            pitch_slide_range: IntParam::new(
                "Pitch Slide Range",
                2,
                IntRange::Linear { min: 0, max: 24 },
            ),
            step_time: IntParam::new("Step Time", 60, IntRange::Linear { min: 1, max: 600 }),
            waveform: IntParam::new("Waveform", 0, IntRange::Linear { min: 0, max: 4 }),
            polyphony: BoolParam::new("Polyphony", false),
            max_voices: IntParam::new("Max Voices", 8, IntRange::Linear { min: 1, max: 8 }),
            portamento_enabled: BoolParam::new("Portamento", false),
            portamento_speed: IntParam::new(
                "Portamento Speed",
                0,
                IntRange::Linear { min: 0, max: 127 },
            ),
        }
    }
}

impl Rp2a03Plugin {
    fn active_voice_count(&self) -> usize {
        if self.params.polyphony.value() {
            (self.params.max_voices.value() as usize).clamp(1, MAX_VOICES)
        } else {
            1
        }
    }

    fn render_samples_with_current_modulation(
        &mut self,
        output: &mut [f32],
        master_gains: &[f32],
        active_voice_count: usize,
    ) {
        let sample_count = output.len() as u32;
        if sample_count == 0 {
            return;
        }

        let clocks_needed = self.voices[0].blip.clocks_needed(sample_count);

        for clock in 0..clocks_needed {
            for (voice, gain) in self.voices[..active_voice_count]
                .iter_mut()
                .zip(master_gains.iter().copied())
            {
                let voice_output = match voice.midi_handler.channel_mode {
                    ChannelMode::Pulse => {
                        voice.pulse.clock();
                        if voice.midi_handler.gate() && !voice.pulse.is_muted() {
                            voice.pulse.output() as i32 * AMPLITUDE_SCALE
                        } else {
                            0
                        }
                    }
                    ChannelMode::Noise => {
                        voice.noise.clock();
                        if voice.midi_handler.gate() && !voice.noise.is_muted() {
                            voice.noise.output() as i32 * AMPLITUDE_SCALE
                        } else {
                            0
                        }
                    }
                    ChannelMode::Triangle => {
                        if voice.midi_handler.gate() {
                            voice.triangle.clock();
                            if !voice.triangle.is_muted() {
                                let output =
                                    (voice.triangle.output() * TRIANGLE_AMPLITUDE_SCALE as f32) as i32;
                                voice.last_triangle_output = output;
                                output
                            } else {
                                0
                            }
                        } else {
                            voice.last_triangle_output
                        }
                    }
                    ChannelMode::Vrc6Pulse => {
                        voice.vrc6_pulse.clock();
                        if voice.midi_handler.gate() {
                            voice.vrc6_pulse.volume() as i32 * AMPLITUDE_SCALE
                        } else {
                            0
                        }
                    }
                    ChannelMode::Vrc6Saw => {
                        voice.vrc6_saw.clock();
                        if voice.midi_handler.gate() {
                            (voice.vrc6_saw.volume() as i32 * AMPLITUDE_SCALE) / 2
                        } else {
                            0
                        }
                    }
                };
                let previous_output = voice.last_output;
                let target_output = (voice_output as f32 * gain) as i32;
                if voice.midi_handler.channel_mode != ChannelMode::Triangle {
                    voice.allocation_ramp_clocks = 0;
                }
                if voice.allocation_ramp_clocks > 0 {
                    // A recycled voice may still be holding the previous note's
                    // level in BlipBuf. Chase the new triangle output over a short
                    // APU-clock ramp instead of submitting one large allocation
                    // delta at clock zero.
                    let remaining = voice.allocation_ramp_clocks;
                    let difference = target_output - previous_output;
                    let step = if difference == 0 {
                        0
                    } else {
                        let magnitude = (difference.unsigned_abs() / remaining)
                            + u32::from(!difference.unsigned_abs().is_multiple_of(remaining));
                        difference.signum() * magnitude.min(i32::MAX as u32) as i32
                    };
                    voice.last_output = previous_output + step;
                    voice.allocation_ramp_clocks -= 1;
                } else {
                    voice.last_output = target_output;
                }
                let delta = voice.last_output - previous_output;
                if delta != 0 {
                    voice.blip.add_delta(clock, delta);
                }
            }
        }

        for voice in &mut self.voices[..active_voice_count] {
            voice.blip.end_frame(clocks_needed);
        }

        self.sample_scratch.resize(output.len(), 0);
        output.fill(0.0);
        for voice in &mut self.voices[..active_voice_count] {
            self.sample_scratch.fill(0);
            let samples_read = voice.blip.read_samples(&mut self.sample_scratch, false);
            for (out, sample) in output[..samples_read]
                .iter_mut()
                .zip(self.sample_scratch[..samples_read].iter())
            {
                *out += *sample as f32 / 32768.0;
            }
        }
        for sample in output.iter_mut() {
            *sample = (*sample * MASTER_OUTPUT_GAIN).clamp(-1.0, 1.0);
        }
    }

    fn generate_samples(&mut self, output: &mut [f32], seqs: &ActiveSequences) {
        let active_voice_count = self.active_voice_count();
        let mut rendered = 0;

        while rendered < output.len() {
            let mut master_gains = Vec::with_capacity(active_voice_count);
            let mut segment_len = output.len() - rendered;
            let mut any_gated = false;

            for voice in &mut self.voices[..active_voice_count] {
                master_gains.push(voice.midi_handler.apply_current_modulation_all(
                    &mut voice.pulse,
                    &mut voice.triangle,
                    Some(&mut voice.noise),
                    Some(&mut voice.vrc6_pulse),
                    Some(&mut voice.vrc6_saw),
                    seqs,
                ));
                if voice.midi_handler.gate() {
                    any_gated = true;
                    segment_len = segment_len.min(
                        voice
                            .midi_handler
                            .samples_until_next_frame(self.sample_rate),
                    );
                }
            }

            if !any_gated {
                segment_len = output.len() - rendered;
            }

            self.render_samples_with_current_modulation(
                &mut output[rendered..rendered + segment_len],
                &master_gains,
                active_voice_count,
            );
            rendered += segment_len;

            for voice in &mut self.voices[..active_voice_count] {
                voice
                    .midi_handler
                    .advance_frame_samples(seqs, self.sample_rate, segment_len);
            }
        }
    }

    fn select_voice(&mut self) -> usize {
        let voice_count = self.active_voice_count();
        if voice_count == 1 {
            return 0;
        }

        let index = self.voices[..voice_count]
            .iter()
            .position(|voice| !voice.midi_handler.gate())
            .unwrap_or_else(|| {
                self.voices[..voice_count]
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, voice)| voice.alloc_id)
                    .map(|(index, _)| index)
                    .unwrap_or(0)
            });

        self.voices[index].reset_for_allocation();
        // `reset_for_allocation` clears the handler's `last_host_controls`, and with
        // it every host-automated control back to its default (full hardware volume,
        // no vibrato/tremolo, 60 Hz step time, no portamento). `process` only applies
        // host automation once per block, before the event loop, so without this the
        // freshly allocated voice would sound the rest of the block on defaults
        // instead of the automated values — an audible level jump on the attack of
        // every polyphonic note.
        let controls = self.host_automation_controls();
        self.voices[index]
            .midi_handler
            .apply_host_automation(controls);
        self.alloc_counter = self.alloc_counter.wrapping_add(1);
        self.voices[index].alloc_id = self.alloc_counter;
        index
    }

    fn handle_event(&mut self, event: &NoteEvent<()>, seqs: &ActiveSequences) -> Option<usize> {
        match event {
            NoteEvent::NoteOn { .. } => {
                let index = self.select_voice();
                let voice = &mut self.voices[index];
                voice.begin_triangle_attack_ramp();
                voice.midi_handler.handle_event_all(
                    event,
                    &mut voice.pulse,
                    &mut voice.triangle,
                    Some(&mut voice.noise),
                    Some(&mut voice.vrc6_pulse),
                    Some(&mut voice.vrc6_saw),
                    seqs,
                )
            }
            NoteEvent::NoteOff { note, .. } => {
                // In polyphonic mode, route a NoteOff to one matching voice.
                // Broadcasting by pitch would release every overlapping note
                // with the same MIDI key. The oldest matching allocation is
                // released first, which gives repeated notes FIFO pairing.
                let voice_count = self.active_voice_count();
                let index = self.voices[..voice_count]
                    .iter()
                    .enumerate()
                    .filter(|(_, voice)| {
                        voice
                            .midi_handler
                            .note_stack
                            .iter()
                            .any(|(held_note, _)| *held_note == *note)
                    })
                    .min_by_key(|(_, voice)| voice.alloc_id)
                    .map(|(index, _)| index);

                if let Some(index) = index {
                    let voice = &mut self.voices[index];
                    voice.midi_handler.handle_event_all(
                        event,
                        &mut voice.pulse,
                        &mut voice.triangle,
                        Some(&mut voice.noise),
                        Some(&mut voice.vrc6_pulse),
                        Some(&mut voice.vrc6_saw),
                        seqs,
                    )
                } else {
                    None
                }
            }
            // Pitch bend and CC are channel-wide, so every voice gets them —
            // otherwise a chord would bend one note at a time.
            NoteEvent::Choke { .. }
            | NoteEvent::MidiCC { .. }
            | NoteEvent::MidiPitchBend { .. } => {
                let mut program = None;
                for voice in &mut self.voices {
                    program = voice
                        .midi_handler
                        .handle_event_all(
                            event,
                            &mut voice.pulse,
                            &mut voice.triangle,
                            Some(&mut voice.noise),
                            Some(&mut voice.vrc6_pulse),
                            Some(&mut voice.vrc6_saw),
                            seqs,
                        )
                        .or(program);
                }
                program
            }
            NoteEvent::MidiProgramChange { .. } => {
                let voice = &mut self.voices[0];
                voice.midi_handler.handle_event_all(
                    event,
                    &mut voice.pulse,
                    &mut voice.triangle,
                    Some(&mut voice.noise),
                    Some(&mut voice.vrc6_pulse),
                    Some(&mut voice.vrc6_saw),
                    seqs,
                )
            }

            _ => None,
        }
    }

    fn host_automation_controls(&self) -> HostAutomationControls {
        HostAutomationControls {
            vibrato_depth: self.params.vibrato_depth.value() as u8,
            vibrato_speed: self.params.vibrato_speed.value() as u8,
            tremolo_depth: self.params.tremolo_depth.value() as u8,
            tremolo_speed: self.params.tremolo_speed.value() as u8,
            hardware_volume: self.params.hardware_volume.value() as u8,
            fine_pitch: self.params.fine_pitch.value() as i8,
            hi_pitch: self.params.hi_pitch.value() as i8,
            pitch_slide: self.params.pitch_slide.value() as i16,
            pitch_slide_range: self.params.pitch_slide_range.value() as u8,
            step_time_hz: self.params.step_time.value() as u16,
            portamento_enabled: self.params.portamento_enabled.value(),
            portamento_speed: self.params.portamento_speed.value() as u8,
        }
    }

    /// Refreshes `self.active_seqs` for `sequence_index` if needed.
    ///
    /// Locks `shared_sequences` only long enough to read its revision counter;
    /// if `(sequence_index, revision)` matches the last refresh, returns
    /// immediately without cloning. On an actual change, `Sequence::clone_from`
    /// reuses each `Vec`'s existing allocation instead of allocating fresh —
    /// this runs on the audio thread, where allocation and long lock hold times
    /// are both real-time-safety hazards.
    ///
    /// A refresh caused by a *slot switch* also re-points every voice's sequence
    /// players at the new envelopes (dn `LoadInstrument`); see
    /// `MidiHandler::reload_sequences`. A refresh caused only by a revision bump
    /// is an in-place edit of the slot already playing, which dn does not treat as
    /// a reload, so the players are left alone.
    fn refresh_active_sequences(&mut self, sequence_index: usize) {
        let mut data = self.params.shared_sequences.lock();
        let revision = data.revision();
        if self.cached_seq_key == Some((sequence_index, revision)) {
            return;
        }
        data.set_all_selected_sequence_indices(sequence_index);

        let slot_changed = self.cached_seq_key.map(|(index, _)| index) != Some(sequence_index);
        // Compared before the clones below overwrite the outgoing slot's data.
        // Comparing `Vec<i16>` contents allocates nothing, so this stays RT-safe.
        let reload = if slot_changed {
            SequenceReload {
                vol: self.active_seqs.vol_seq != *data.selected_sequence(0),
                arp: self.active_seqs.arp_seq != *data.selected_sequence(1),
                pitch: self.active_seqs.pitch_seq != *data.selected_sequence(2),
                hipitch: self.active_seqs.hipitch_seq != *data.selected_sequence(3),
                duty: self.active_seqs.duty_seq != *data.selected_sequence(4),
            }
        } else {
            SequenceReload::default()
        };

        self.active_seqs.vol_seq.clone_from(data.selected_sequence(0));
        self.active_seqs.vol_enabled = data.sequence_enabled(0);
        self.active_seqs.arp_seq.clone_from(data.selected_sequence(1));
        self.active_seqs.arp_enabled = data.sequence_enabled(1);
        self.active_seqs.pitch_seq.clone_from(data.selected_sequence(2));
        self.active_seqs.pitch_enabled = data.sequence_enabled(2);
        self.active_seqs
            .hipitch_seq
            .clone_from(data.selected_sequence(3));
        self.active_seqs.hipitch_enabled = data.sequence_enabled(3);
        self.active_seqs.duty_seq.clone_from(data.selected_sequence(4));
        self.active_seqs.duty_enabled = data.sequence_enabled(4);
        drop(data);
        self.cached_seq_key = Some((sequence_index, revision));

        // Called even when `reload` is all-false: an envelope whose content is
        // identical across the two slots can still have flipped its enable flag,
        // and dn's `ClearSequence` branch is unconditional on the enable state.
        if slot_changed {
            for voice in &mut self.voices {
                voice.midi_handler.reload_sequences(&self.active_seqs, reload);
            }
        }
    }

    fn publish_sequence_playheads(&self, seqs: &ActiveSequences) {
        let steps = [
            sequence_play_step(
                &self.voices[0].midi_handler.vol_seq_player,
                &seqs.vol_seq,
                seqs.vol_enabled,
            ),
            sequence_play_step(
                &self.voices[0].midi_handler.arp_seq_player,
                &seqs.arp_seq,
                seqs.arp_enabled,
            ),
            sequence_play_step(
                &self.voices[0].midi_handler.pitch_seq_player,
                &seqs.pitch_seq,
                seqs.pitch_enabled,
            ),
            sequence_play_step(
                &self.voices[0].midi_handler.hipitch_seq_player,
                &seqs.hipitch_seq,
                seqs.hipitch_enabled,
            ),
            sequence_play_step(
                &self.voices[0].midi_handler.duty_seq_player,
                &seqs.duty_seq,
                seqs.duty_enabled,
            ),
        ];

        for (playhead, step) in self.sequence_playheads.iter().zip(steps) {
            playhead.store(step.unwrap_or(NO_PLAYHEAD_STEP), Ordering::Relaxed);
        }
    }

    fn clear_sequence_playheads(&self) {
        for playhead in self.sequence_playheads.iter() {
            playhead.store(NO_PLAYHEAD_STEP, Ordering::Relaxed);
        }
    }
}

fn sequence_play_step(
    player: &SequencePlayer,
    sequence: &Sequence,
    enabled: bool,
) -> Option<usize> {
    if enabled
        && !sequence.is_empty()
        && player.state == SeqState::Running
        && player.pos < sequence.len()
    {
        Some(player.pos)
    } else {
        None
    }
}

fn load_sequence_playheads(playheads: &[AtomicUsize; SEQUENCE_TYPE_COUNT]) -> SequencePlayheads {
    SequencePlayheads::from_steps(std::array::from_fn(|index| {
        let step = playheads[index].load(Ordering::Relaxed);
        (step != NO_PLAYHEAD_STEP).then_some(step)
    }))
}

impl Plugin for Rp2a03Plugin {
    const NAME: &'static str = "RP2A03_Synth";
    const VENDOR: &'static str = "PurpBatBoi";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    // `Basic` delivers note events only. Pitch bend (which drives Pitch Slide),
    // control changes (RPN 0, which drives Pitch Slide Range), and program change
    // all require this level. For VST3 that means the wrapper registers 130*16
    // hidden CC-binding parameters; that is the standard cost of MIDI CC input.
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let shared = self.params.shared_sequences.clone();
        let params = self.params.clone();
        let playheads = self.sequence_playheads.clone();

        create_egui_editor(
            self.params.egui_state.clone(),
            (),
            EguiSettings::default(),
            move |ctx, _queue, _state| {
                egui_extras::install_image_loaders(ctx);
                ctx.set_style_of(egui::Theme::Dark, style());
            },
            move |ui, setter, _queue, _state| {
                let mut data = shared.lock();
                let sequence_index = data.selected_sequence_index(0);
                let playheads = load_sequence_playheads(&playheads);
                let step_time_hz = params.step_time.value() as u32;

                ui.ctx().request_repaint_after(Duration::from_millis(30));

                egui::Frame::NONE
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        let result = render_editor_ui(
                            ui,
                            &mut data,
                            sequence_index,
                            &playheads,
                            step_time_hz,
                        );

                        if let Some(new_index) = result.new_sequence_index {
                            data.set_all_selected_sequence_indices(new_index);

                            let new_index = new_index as i32;
                            setter.begin_set_parameter(&params.sequence_number);
                            setter.set_parameter(&params.sequence_number, new_index);
                            setter.end_set_parameter(&params.sequence_number);
                        }

                        if let Some(new_hz) = result.new_step_time_hz {
                            setter.begin_set_parameter(&params.step_time);
                            setter.set_parameter(&params.step_time, new_hz);
                            setter.end_set_parameter(&params.step_time);
                        }

                        if let Some(new_mode) = result.new_channel_mode {
                            let mode_i32 = new_mode as i32;
                            setter.begin_set_parameter(&params.waveform);
                            setter.set_parameter(&params.waveform, mode_i32);
                            setter.end_set_parameter(&params.waveform);
                        }

                        if let Some(new_polyphony) = result.new_polyphony {
                            setter.begin_set_parameter(&params.polyphony);
                            setter.set_parameter(&params.polyphony, new_polyphony);
                            setter.end_set_parameter(&params.polyphony);
                        }

                        if let Some(new_max_voices) = result.new_max_voices {
                            setter.begin_set_parameter(&params.max_voices);
                            setter.set_parameter(&params.max_voices, new_max_voices);
                            setter.end_set_parameter(&params.max_voices);
                        }
                        if let Some(enabled) = result.new_portamento_enabled {
                            setter.begin_set_parameter(&params.portamento_enabled);
                            setter.set_parameter(&params.portamento_enabled, enabled);
                            setter.end_set_parameter(&params.portamento_enabled);
                        }
                        if let Some(speed) = result.new_portamento_speed {
                            setter.begin_set_parameter(&params.portamento_speed);
                            setter.set_parameter(&params.portamento_speed, speed);
                            setter.end_set_parameter(&params.portamento_speed);
                        }
                    });
            },
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        for voice in &mut self.voices {
            voice
                .blip
                .set_rates(NTSC_CPU_CLOCK, buffer_config.sample_rate as f64);
            voice.reset();
            voice.midi_handler.channel_mode = ChannelMode::from_i32(self.params.waveform.value());
        }
        self.alloc_counter = 0;
        self.last_active_voice_count = 1;
        self.midi_program_index = None;
        self.last_sequence_parameter = self.params.sequence_number.value();
        self.clear_sequence_playheads();
        true
    }

    fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.reset();
            voice.midi_handler.channel_mode = ChannelMode::from_i32(self.params.waveform.value());
        }
        self.alloc_counter = 0;
        self.last_active_voice_count = 1;
        self.midi_program_index = None;
        self.last_sequence_parameter = self.params.sequence_number.value();
        self.clear_sequence_playheads();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let host_controls = self.host_automation_controls();
        for voice in &mut self.voices {
            voice.midi_handler.apply_host_automation(host_controls);
        }
        // Sync channel mode from host parameter (handles DAW automation / state recall).
        let channel_mode = ChannelMode::from_i32(self.params.waveform.value());
        for voice in &mut self.voices {
            voice.midi_handler.channel_mode = channel_mode;
        }
        // Sync channel mode into shared GUI state so the combobox reflects the current value.
        {
            let mut data = self.params.shared_sequences.lock();
            data.channel_mode = channel_mode;
            data.polyphony = self.params.polyphony.value();
            data.max_voices = self.params.max_voices.value().clamp(1, 8);
            data.portamento_enabled = self.params.portamento_enabled.value();
            data.portamento_speed = self.params.portamento_speed.value().clamp(0, 127);
        }
        let num_samples = buffer.samples();
        let active_voice_count = self.active_voice_count();
        if active_voice_count < self.last_active_voice_count {
            for voice in &mut self.voices[active_voice_count..self.last_active_voice_count] {
                voice.reset();
            }
        }
        self.last_active_voice_count = active_voice_count;
        let mut next_event = context.next_event();
        let mut sample_pos: usize = 0;
        let mut mono_buf = vec![0.0f32; num_samples];

        let sequence_parameter = self.params.sequence_number.value();
        if sequence_parameter != self.last_sequence_parameter {
            self.last_sequence_parameter = sequence_parameter;
            self.midi_program_index = None;
        }
        let mut sequence_index = self
            .midi_program_index
            .unwrap_or(sequence_parameter as usize)
            .min(MAX_SEQUENCES - 1);
        self.refresh_active_sequences(sequence_index);

        loop {
            let chunk_end = if let Some(ref event) = next_event {
                (event.timing() as usize).min(num_samples)
            } else {
                num_samples
            };

            if chunk_end > sample_pos {
                let seqs = std::mem::take(&mut self.active_seqs);
                self.generate_samples(&mut mono_buf[sample_pos..chunk_end], &seqs);
                self.active_seqs = seqs;
                sample_pos = chunk_end;
            }

            if sample_pos >= num_samples {
                break;
            }

            while let Some(event) = next_event {
                if event.timing() as usize > sample_pos {
                    next_event = Some(event);
                    break;
                }
                let seqs = std::mem::take(&mut self.active_seqs);
                let program_index = self.handle_event(&event, &seqs);
                self.active_seqs = seqs;
                if let Some(program_index) = program_index {
                    sequence_index = program_index.min(MAX_SEQUENCES - 1);
                    self.midi_program_index = Some(sequence_index);
                    self.refresh_active_sequences(sequence_index);
                }
                next_event = context.next_event();
            }

            if next_event.is_none() && sample_pos < num_samples {
                let seqs = std::mem::take(&mut self.active_seqs);
                self.generate_samples(&mut mono_buf[sample_pos..num_samples], &seqs);
                self.active_seqs = seqs;
                break;
            }
        }

        self.publish_sequence_playheads(&self.active_seqs);

        for (sample_id, channel_samples) in buffer.iter_samples().enumerate() {
            for out_sample in channel_samples {
                *out_sample = mono_buf[sample_id];
            }
        }

        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for Rp2a03Plugin {
    const CLAP_ID: &'static str = "com.rp2a03.synth";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("NES APU+MAPPERS Synthesizer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for Rp2a03Plugin {
    const VST3_CLASS_ID: [u8; 16] = *b"Rp2a03SynthPlugX";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nice_export_clap!(Rp2a03Plugin);
nice_export_vst3!(Rp2a03Plugin);
