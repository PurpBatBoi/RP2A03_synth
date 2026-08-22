//! `rp2a03_common\src\gui\wavetable_state.rs`

pub const FDS_WAVE_LEN: usize = 64;
pub const FDS_WAVE_MAX: u16 = 63;

pub const MAX_SLOTS: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum DrawStyle {
    #[default]
    Steps,

    Lines,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WaveSlot {
    data: Vec<u16>,
}

impl<'de> serde::Deserialize<'de> for WaveSlot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Raw {
            data: Vec<u16>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let mut slot = Self::new();
        slot.set_data(&raw.data);
        Ok(slot)
    }
}

impl Default for WaveSlot {
    fn default() -> Self {
        Self::new()
    }
}

impl WaveSlot {
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: vec![0; FDS_WAVE_LEN],
        }
    }

    #[must_use]
    pub fn data(&self) -> &[u16] {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut [u16] {
        &mut self.data
    }

    pub fn set_data(&mut self, values: &[u16]) {
        let mut data = vec![0u16; FDS_WAVE_LEN];
        for (dst, &src) in data.iter_mut().zip(values) {
            *dst = src.min(FDS_WAVE_MAX);
        }
        self.data = data;
    }
}

pub const PARTIAL_COUNT: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum BaseShape {
    #[default]
    Sine,
    Triangle,
    Saw,
    Pulse,
}

impl BaseShape {
    pub const ALL: [(Self, &'static str); 4] = [
        (Self::Sine, "Sine"),
        (Self::Triangle, "Triangle"),
        (Self::Saw, "Saw"),
        (Self::Pulse, "Pulse"),
    ];
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum DisplayMode {
    #[default]
    Dec,
    Hex,
}

#[derive(Clone, PartialEq)]
pub struct ToolsGenState {
    pub offset_x: i32,

    pub offset_y: i32,

    pub smooth: usize,

    pub amplify: f32,
}

impl Default for ToolsGenState {
    fn default() -> Self {
        Self {
            offset_x: 0,
            offset_y: 0,
            smooth: 1,
            amplify: 1.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum GeneratorTab {
    #[default]
    Shapes,
    WaveTools,
}

#[derive(Clone, PartialEq)]
pub struct ShapesGenState {
    pub shape: BaseShape,

    pub duty: f32,

    pub exponent: i32,

    pub invert_point: f32,

    pub amp: [f32; PARTIAL_COUNT],

    pub phase: [f32; PARTIAL_COUNT],
}

impl Default for ShapesGenState {
    fn default() -> Self {
        let mut amp = [0.0; PARTIAL_COUNT];
        amp[0] = 1.0;
        Self {
            shape: BaseShape::default(),
            duty: 0.5,
            exponent: 1,
            invert_point: 1.0,
            amp,
            phase: [0.0; PARTIAL_COUNT],
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WaveSlots {
    slots: Vec<WaveSlot>,
    current_slot: usize,
}

pub struct WavetableEditorState {
    pub draw_style: DrawStyle,
    pub generator_visible: bool,
    pub generator_tab: GeneratorTab,
    pub shapes: ShapesGenState,
    pub tools: ToolsGenState,

    pub display: DisplayMode,

    pub signed: bool,

    pub readout_text: String,
}

impl Default for WavetableEditorState {
    fn default() -> Self {
        Self {
            draw_style: DrawStyle::default(),
            generator_visible: true,
            generator_tab: GeneratorTab::default(),
            shapes: ShapesGenState::default(),
            tools: ToolsGenState::default(),
            display: DisplayMode::default(),
            signed: false,
            readout_text: String::new(),
        }
    }
}

impl WaveSlots {
    #[must_use]
    pub fn slots(&self) -> &[WaveSlot] {
        &self.slots
    }

    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    #[must_use]
    pub fn current_slot(&self) -> usize {
        self.current_slot
    }

    pub fn set_current_slot(&mut self, index: usize) {
        self.current_slot = index.min(self.slots.len().saturating_sub(1));
    }

    #[must_use]
    pub fn current(&self) -> Option<&WaveSlot> {
        self.slots.get(self.current_slot)
    }

    pub fn current_mut(&mut self) -> Option<&mut WaveSlot> {
        self.slots.get_mut(self.current_slot)
    }

    pub fn add_slot(&mut self) {
        if self.is_full() {
            return;
        }
        let new = self.current().cloned().unwrap_or_default();
        self.slots.push(new);
        self.current_slot = self.slots.len() - 1;
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        self.slots.len() >= MAX_SLOTS
    }

    pub fn add_slot_from(&mut self, values: &[u16]) {
        if self.is_full() {
            return;
        }
        let mut slot = WaveSlot::new();
        slot.set_data(values);
        self.slots.push(slot);
        self.current_slot = self.slots.len() - 1;
    }

    pub fn remove_slot(&mut self) {
        if self.slots.is_empty() {
            return;
        }
        self.slots.remove(self.current_slot);
        self.current_slot = self.current_slot.min(self.slots.len().saturating_sub(1));
    }

    pub fn set_slots(&mut self, slots: Vec<WaveSlot>, current_slot: usize) {
        self.current_slot = current_slot.min(slots.len().saturating_sub(1));
        self.slots = slots;
    }
}
