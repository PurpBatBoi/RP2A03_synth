//! `rp2a03_common\src\gui\wavesynth_state.rs`

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum WaveSynthEffect {
    #[default]
    None,
    Invert,
    Add,
    Subtract,
    Average,
    Phase,
    Chorus,

    Wipe,
    Fade,
    PingPong,
    Overlay,
    NegativeOverlay,
    Slide,
    Mix,
    PhaseModulation,
}

impl WaveSynthEffect {
    pub const SINGLE: [(Self, &'static str); 7] = [
        (Self::None, "None"),
        (Self::Invert, "Invert"),
        (Self::Add, "Add"),
        (Self::Subtract, "Subtract"),
        (Self::Average, "Average"),
        (Self::Phase, "Phase"),
        (Self::Chorus, "Chorus"),
    ];

    pub const DUAL: [(Self, &'static str); 8] = [
        (Self::Wipe, "Wipe"),
        (Self::Fade, "Fade"),
        (Self::PingPong, "Ping-Pong"),
        (Self::Overlay, "Overlay"),
        (Self::NegativeOverlay, "Negative Overlay"),
        (Self::Slide, "Slide"),
        (Self::Mix, "Mix"),
        (Self::PhaseModulation, "Phase Modulation"),
    ];

    #[must_use]
    pub fn is_dual(self) -> bool {
        Self::DUAL.iter().any(|&(e, _)| e == self)
    }

    #[must_use]
    pub fn accumulates(self) -> bool {
        matches!(
            self,
            Self::None
                | Self::Invert
                | Self::Add
                | Self::Subtract
                | Self::Average
                | Self::Overlay
                | Self::NegativeOverlay
        )
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        Self::SINGLE
            .iter()
            .chain(Self::DUAL.iter())
            .find(|&&(e, _)| e == self)
            .map_or("None", |&(_, name)| name)
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct TickState {
    pub pos: usize,

    pub stage: i32,

    pub stage_dir: bool,

    pub div_counter: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WaveSynthParams {
    pub wave1: usize,

    pub wave2: usize,

    pub rate_divider: u8,
    pub effect: WaveSynthEffect,
    pub enabled: bool,

    pub speed: u8,

    pub reset_on_note: bool,

    pub param1: u8,

    pub param2: u8,
}

impl Default for WaveSynthParams {
    fn default() -> Self {
        Self {
            wave1: 0,
            wave2: 0,

            rate_divider: 1,
            effect: WaveSynthEffect::default(),
            enabled: false,
            speed: 0,

            reset_on_note: false,
            param1: 0,
            param2: 0,
        }
    }
}

#[derive(Default)]
pub struct WaveSynthPreview {
    pub result: Vec<u16>,

    pub paused: bool,
    pub tick_state: TickState,

    pub(crate) seeded_from: Vec<u16>,
    pub(crate) seeded_dims: (usize, u16),
    pub(crate) seeded_effect: Option<WaveSynthEffect>,
}

impl WaveSynthPreview {
    pub fn restart(&mut self) {
        self.tick_state = TickState::default();

        self.seeded_from.clear();
        self.seeded_dims = (0, 0);
        self.seeded_effect = None;
    }
}
