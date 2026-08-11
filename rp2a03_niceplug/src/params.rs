//! rp2a03_niceplug\src\params.rs
//! Host-facing parameter set, and the projections from it that the audio thread
//! actually consumes.
//!
//! Adding a parameter is a three-step edit: declare the field with its `#[id]`,
//! give it a default/range below, and — if the synth needs to react to it — fold
//! it into one of the projections at the bottom of this file.

use nice_plug::prelude::*;
use nice_plug_egui::EguiState;
use parking_lot::Mutex;
use rp2a03_common::{ChannelMode, HostAutomationSnapshot, MAX_SEQUENCES, SharedSequences};
use std::sync::Arc;

use crate::voice_bank::MAX_VOICES;

const EDITOR_WIDTH: u32 = 758;
const EDITOR_HEIGHT: u32 = 590;

#[derive(Params)]
pub(crate) struct Rp2a03Params {
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

/// Shorthand for the linear integer ranges every parameter here uses.
fn linear(min: i32, max: i32) -> IntRange {
    IntRange::Linear { min, max }
}

impl Default for Rp2a03Params {
    fn default() -> Self {
        Self {
            egui_state: EguiState::from_size(EDITOR_WIDTH, EDITOR_HEIGHT),
            shared_sequences: Arc::new(Mutex::new(SharedSequences::default())),
            sequence_number: IntParam::new(
                "Sequence Index",
                0,
                linear(0, (MAX_SEQUENCES - 1) as i32),
            ),
            vibrato_depth: IntParam::new("Vibrato Depth", 0, linear(0, 15)),
            vibrato_speed: IntParam::new("Vibrato Speed", 4, linear(0, 63)),
            tremolo_depth: IntParam::new("Tremolo Depth", 0, linear(0, 15)),
            tremolo_speed: IntParam::new("Tremolo Speed", 4, linear(0, 63)),
            hardware_volume: IntParam::new("HW Volume", 15, linear(0, 15)),
            fine_pitch: IntParam::new("Pitch", 0, linear(-64, 63)),
            hi_pitch: IntParam::new("Hi-Pitch", 0, linear(-64, 63)),
            pitch_slide: IntParam::new("Pitch Slide", 0, linear(-8192, 8191)),
            pitch_slide_range: IntParam::new("Pitch Slide Range", 2, linear(0, 24)),
            step_time: IntParam::new("Step Time", 60, linear(1, 600)),
            // Hidden, not merely non-automatable. Waveform is slot-owned
            // instrument data: the editor is its only writer, and every editor
            // path — combo box, patch load, per-slot recall — writes through to
            // `slot_waveforms`. Any writer the host offers — automation lane or
            // generic UI — would not, so its value would be silently discarded
            // the next time Sequence Index moved off the slot and back.
            // `.non_automatable()` closes only the lane and leaves the generic
            // UI, which is the same footgun somewhere harder to find. Automate
            // Sequence Index instead. Note this is advertisement, not
            // enforcement: the wrappers consult these flags only when telling
            // the host what exists, and apply incoming events by hash without
            // checking them. Hiding does not remove the parameter from saved
            // host state, and does not stop the editor writing it.
            waveform: IntParam::new("Waveform", 0, linear(0, 5)).hide(),
            polyphony: BoolParam::new("Polyphony", false),
            max_voices: IntParam::new("Max Voices", 8, linear(1, MAX_VOICES as i32)),
            portamento_enabled: BoolParam::new("Portamento", false),
            portamento_speed: IntParam::new("Portamento Speed", 0, linear(0, PORTAMENTO_SPEED_MAX)),
        }
    }
}

const PORTAMENTO_SPEED_MAX: i32 = 127;

impl Rp2a03Params {
    /// Takes one immutable render-block snapshot of host automation.
    ///
    /// The snapshot is the boundary between host-facing parameter storage and
    /// per-voice runtime state, similar to SFLT's render-time override view.
    pub(crate) fn host_automation_snapshot(&self) -> HostAutomationSnapshot {
        HostAutomationSnapshot {
            vibrato_depth: self.vibrato_depth.value() as u8,
            vibrato_speed: self.vibrato_speed.value() as u8,
            tremolo_depth: self.tremolo_depth.value() as u8,
            tremolo_speed: self.tremolo_speed.value() as u8,
            hardware_volume: self.hardware_volume.value() as u8,
            fine_pitch: self.fine_pitch.value() as i8,
            hi_pitch: self.hi_pitch.value() as i8,
            pitch_slide: self.pitch_slide.value() as i16,
            pitch_slide_range: self.pitch_slide_range.value() as u8,
            step_time_hz: self.step_time.value() as u16,
            portamento_enabled: self.portamento_enabled.value(),
            portamento_speed: self.portamento_speed.value() as u8,
        }
    }

    pub(crate) fn channel_mode(&self) -> ChannelMode {
        ChannelMode::from_i32(self.waveform.value())
    }

    /// How many voices the render loop should run this block.
    pub(crate) fn active_voice_count(&self) -> usize {
        if self.polyphony.value() {
            (self.max_voices.value() as usize).clamp(1, MAX_VOICES)
        } else {
            1
        }
    }

    /// Mirrors the parameters the editor renders from into shared GUI state, so
    /// host automation and state recall are reflected in the UI widgets.
    pub(crate) fn publish_to_gui(&self, shared: &mut SharedSequences, channel_mode: ChannelMode) {
        shared.channel_mode = channel_mode;
        shared.polyphony = self.polyphony.value();
        shared.max_voices = self.max_voices.value().clamp(1, MAX_VOICES as i32);
        shared.portamento_enabled = self.portamento_enabled.value();
        shared.portamento_speed = self.portamento_speed.value().clamp(0, PORTAMENTO_SPEED_MAX);
    }
}
