//! `rp2a03_common\src\gui\state.rs`

use super::chip_settings_state::FdsSettings;
use super::wavesynth_state::WaveSynthParams;
use super::wavetable_state::WaveSlots;
use crate::{ChannelMode, Lane};
use rp2a03_core::sequencer::Sequence;

pub const MAX_SEQUENCES: usize = 128;
pub const NO_PLAYHEAD_STEP: usize = usize::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequencePlayheads {
    steps: [Option<usize>; Lane::COUNT],
}

impl Default for SequencePlayheads {
    fn default() -> Self {
        Self {
            steps: [None; Lane::COUNT],
        }
    }
}

impl SequencePlayheads {
    #[must_use]
    pub fn from_steps(steps: [Option<usize>; Lane::COUNT]) -> Self {
        Self { steps }
    }

    #[must_use]
    pub fn step(&self, lane: Lane) -> Option<usize> {
        self.steps[lane]
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SequenceSlot {
    pub text: String,
    pub sequence: Sequence,

    pub(crate) enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SequenceBank {
    slots: Vec<SequenceSlot>,
}

impl<'de> serde::Deserialize<'de> for SequenceBank {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Raw {
            slots: Vec<SequenceSlot>,
        }
        let mut raw = Raw::deserialize(deserializer)?;
        raw.slots.resize(MAX_SEQUENCES, SequenceSlot::default());
        Ok(Self { slots: raw.slots })
    }
}

impl Default for SequenceBank {
    fn default() -> Self {
        Self {
            slots: vec![SequenceSlot::default(); MAX_SEQUENCES],
        }
    }
}

impl SequenceBank {
    #[must_use]
    pub fn slot(&self, index: usize) -> &SequenceSlot {
        &self.slots[index.min(MAX_SEQUENCES - 1)]
    }

    pub fn slot_mut(&mut self, index: usize) -> &mut SequenceSlot {
        &mut self.slots[index.min(MAX_SEQUENCES - 1)]
    }

    pub(crate) fn slots(&self) -> &[SequenceSlot] {
        &self.slots
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SharedSequences {
    pub selected_tab: Lane,

    sequence_indices: [usize; Lane::COUNT],
    sequence_banks: [SequenceBank; Lane::COUNT],

    #[serde(default = "default_slot_waveforms")]
    slot_waveforms: Vec<Option<ChannelMode>>,

    #[serde(skip)]
    revision: u64,

    #[serde(default)]
    wave_slots: WaveSlots,

    #[serde(default)]
    wavesynth: WaveSynthParams,

    #[serde(default = "default_fds_settings")]
    fds_settings: Vec<FdsSettings>,

    #[serde(default)]
    wavesynth_slots: Vec<WaveSynthParams>,
}

fn default_slot_waveforms() -> Vec<Option<ChannelMode>> {
    vec![None; MAX_SEQUENCES]
}

fn per_slot_index(len: usize, index: usize) -> Option<usize> {
    let last = len.checked_sub(1)?;
    Some(index.min(MAX_SEQUENCES - 1).min(last))
}

fn default_fds_settings() -> Vec<FdsSettings> {
    vec![FdsSettings::default(); MAX_SEQUENCES]
}

fn default_wavesynth_slots() -> Vec<WaveSynthParams> {
    vec![WaveSynthParams::default(); MAX_SEQUENCES]
}

/// Bumps `revision` on `Drop`, so a mutator that returns early — or grows a
/// new branch later — cannot forget to mark its edit. Constructed directly
/// against `&mut self.revision`, never through a method, so the borrow
/// checker still sees it as touching only that one field and leaves the
/// rest of `self` free to borrow alongside it.
struct EditGuard<'a> {
    revision: &'a mut u64,
}

impl Drop for EditGuard<'_> {
    fn drop(&mut self) {
        *self.revision += 1;
    }
}

impl Default for SharedSequences {
    fn default() -> Self {
        Self {
            selected_tab: Lane::Vol,
            sequence_indices: [0; Lane::COUNT],
            sequence_banks: std::array::from_fn(|_| SequenceBank::default()),
            slot_waveforms: default_slot_waveforms(),
            revision: 0,
            wave_slots: WaveSlots::default(),
            wavesynth: WaveSynthParams::default(),
            fds_settings: default_fds_settings(),
            wavesynth_slots: default_wavesynth_slots(),
        }
    }
}

impl SharedSequences {
    #[must_use]
    pub fn wave_slots(&self) -> &WaveSlots {
        &self.wave_slots
    }

    pub fn wave_slots_mut(&mut self) -> &mut WaveSlots {
        let _guard = EditGuard {
            revision: &mut self.revision,
        };
        &mut self.wave_slots
    }

    #[must_use]
    pub fn wavesynth(&self, index: usize) -> WaveSynthParams {
        per_slot_index(self.wavesynth_slots.len(), index)
            .map_or(self.wavesynth, |i| self.wavesynth_slots[i])
    }

    pub fn set_wavesynth(&mut self, index: usize, params: WaveSynthParams) {
        let _guard = EditGuard {
            revision: &mut self.revision,
        };
        if self.wavesynth_slots.is_empty() {
            self.wavesynth_slots = vec![self.wavesynth; MAX_SEQUENCES];
        }
        if let Some(i) = per_slot_index(self.wavesynth_slots.len(), index) {
            self.wavesynth_slots[i] = params;
        }
    }

    #[must_use]
    pub fn fds_settings(&self, index: usize) -> &FdsSettings {
        static FALLBACK: std::sync::OnceLock<FdsSettings> = std::sync::OnceLock::new();
        per_slot_index(self.fds_settings.len(), index).map_or_else(
            || FALLBACK.get_or_init(FdsSettings::default),
            |i| &self.fds_settings[i],
        )
    }

    pub fn fds_settings_mut(&mut self, index: usize) -> &mut FdsSettings {
        let _guard = EditGuard {
            revision: &mut self.revision,
        };
        if self.fds_settings.is_empty() {
            self.fds_settings = default_fds_settings();
        }
        let i = per_slot_index(self.fds_settings.len(), index).unwrap_or(0);
        &mut self.fds_settings[i]
    }

    #[must_use]
    pub fn selected_sequence_index(&self, lane: Lane) -> usize {
        self.sequence_indices[lane]
    }

    pub fn set_selected_sequence_index(&mut self, lane: Lane, index: usize) {
        self.sequence_indices[lane] = index.min(MAX_SEQUENCES - 1);
    }

    pub fn set_all_selected_sequence_indices(&mut self, index: usize) {
        self.sequence_indices.fill(index.min(MAX_SEQUENCES - 1));
    }

    fn slot_waveform_index(&self, index: usize) -> Option<usize> {
        per_slot_index(self.slot_waveforms.len(), index)
    }

    /// The waveform explicitly assigned to this slot, or `None` if it was
    /// never set — distinct from an explicit choice of `ChannelMode::Pulse`,
    /// so callers can tell "never touched" apart from "chosen as Pulse".
    #[must_use]
    pub fn slot_waveform(&self, index: usize) -> Option<ChannelMode> {
        self.slot_waveform_index(index)
            .and_then(|i| self.slot_waveforms[i])
    }

    pub fn set_slot_waveform(&mut self, index: usize, mode: ChannelMode) {
        let Some(i) = self.slot_waveform_index(index) else {
            return;
        };
        let _guard = EditGuard {
            revision: &mut self.revision,
        };
        self.slot_waveforms[i] = Some(mode);
    }

    #[must_use]
    pub fn selected_sequence(&self, lane: Lane) -> &Sequence {
        &self.sequence_banks[lane]
            .slot(self.sequence_indices[lane])
            .sequence
    }

    #[must_use]
    pub fn selected_sequence_text(&self, lane: Lane) -> &str {
        &self.sequence_banks[lane]
            .slot(self.sequence_indices[lane])
            .text
    }

    /// Reads a lane's sequence at an explicit slot index, independent of
    /// which slot the editor currently has selected. The audio thread uses
    /// this — never `selected_sequence` — so pulling playback data never
    /// requires writing the GUI's own selection state back into
    /// `SharedSequences` (see M4 step 26).
    #[must_use]
    pub fn sequence_at(&self, lane: Lane, index: usize) -> &Sequence {
        &self.sequence_banks[lane].slot(index).sequence
    }

    #[must_use]
    pub fn sequence_enabled_at(&self, lane: Lane, index: usize) -> bool {
        self.sequence_banks[lane].slot(index).enabled
    }

    pub fn selected_sequence_mut(&mut self, lane: Lane) -> (&mut String, &mut Sequence) {
        let index = self.sequence_indices[lane];
        let _guard = EditGuard {
            revision: &mut self.revision,
        };
        let slot = self.sequence_banks[lane].slot_mut(index);
        (&mut slot.text, &mut slot.sequence)
    }

    #[must_use]
    pub fn sequence_enabled(&self, lane: Lane) -> bool {
        self.sequence_banks[lane]
            .slot(self.sequence_indices[lane])
            .enabled
    }

    pub fn sequence_enabled_mut(&mut self, lane: Lane) -> &mut bool {
        let index = self.sequence_indices[lane];
        let _guard = EditGuard {
            revision: &mut self.revision,
        };
        &mut self.sequence_banks[lane].slot_mut(index).enabled
    }

    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn sequence_bank(&self, lane: Lane) -> &SequenceBank {
        &self.sequence_banks[lane]
    }

    pub(crate) fn sequence_bank_mut(&mut self, lane: Lane) -> &mut SequenceBank {
        let _guard = EditGuard {
            revision: &mut self.revision,
        };
        &mut self.sequence_banks[lane]
    }

    pub(crate) fn clear_all_sequences(&mut self) {
        let _guard = EditGuard {
            revision: &mut self.revision,
        };
        self.sequence_banks = std::array::from_fn(|_| SequenceBank::default());
        self.sequence_indices = [0; Lane::COUNT];
        self.slot_waveforms = default_slot_waveforms();
        self.wave_slots = WaveSlots::default();
        self.fds_settings = default_fds_settings();
        self.wavesynth_slots = default_wavesynth_slots();
    }
}

#[cfg(test)]
mod tests {
    //! M4 step 21: revision-counter tests, written before steps 22-24 close
    //! the gaps they pin. `set_wavesynth`/`fds_settings_mut`/`set_slot_waveform`
    //! are the plan's three "currently-forgetting mutators" (step 23); the
    //! `wave_slots_mut`-uses-`revision` case pins the counter collapse
    //! (step 24) — both red against the code as of M3.

    use super::*;
    use crate::ChannelMode;

    #[test]
    fn default_revision_is_zero() {
        assert_eq!(SharedSequences::default().revision(), 0);
    }

    #[test]
    fn selected_sequence_mut_bumps_revision() {
        let mut data = SharedSequences::default();
        let before = data.revision();
        let _ = data.selected_sequence_mut(Lane::Vol);
        assert_eq!(data.revision(), before + 1);
    }

    #[test]
    fn sequence_enabled_mut_bumps_revision() {
        let mut data = SharedSequences::default();
        let before = data.revision();
        let _ = data.sequence_enabled_mut(Lane::Arp);
        assert_eq!(data.revision(), before + 1);
    }

    #[test]
    fn sequence_bank_mut_bumps_revision() {
        let mut data = SharedSequences::default();
        let before = data.revision();
        let _ = data.sequence_bank_mut(Lane::Duty);
        assert_eq!(data.revision(), before + 1);
    }

    #[test]
    fn set_wavesynth_bumps_revision() {
        let mut data = SharedSequences::default();
        let before = data.revision();
        data.set_wavesynth(0, data.wavesynth(0));
        assert_eq!(data.revision(), before + 1);
    }

    #[test]
    fn fds_settings_mut_bumps_revision() {
        let mut data = SharedSequences::default();
        let before = data.revision();
        let _ = data.fds_settings_mut(0);
        assert_eq!(data.revision(), before + 1);
    }

    #[test]
    fn set_slot_waveform_bumps_revision() {
        let mut data = SharedSequences::default();
        let before = data.revision();
        data.set_slot_waveform(0, ChannelMode::Fds);
        assert_eq!(data.revision(), before + 1);
    }

    #[test]
    fn wave_slots_mut_bumps_the_same_revision_counter() {
        let mut data = SharedSequences::default();
        let before = data.revision();
        let _ = data.wave_slots_mut();
        assert_eq!(
            data.revision(),
            before + 1,
            "post-collapse there is one counter; wave_slots_mut must move it"
        );
    }

    #[test]
    fn pure_reads_never_move_the_revision() {
        let data = SharedSequences::default();
        let before = data.revision();

        let _ = data.selected_sequence(Lane::Vol);
        let _ = data.sequence_enabled(Lane::Vol);
        let _ = data.wave_slots();
        let _ = data.wavesynth(0);
        let _ = data.fds_settings(0);
        let _ = data.slot_waveform(0);
        let _ = data.selected_sequence_index(Lane::Vol);

        assert_eq!(data.revision(), before);
    }

    #[test]
    fn clear_all_sequences_bumps_revision_exactly_once() {
        let mut data = SharedSequences::default();
        let before = data.revision();
        data.clear_all_sequences();
        assert_eq!(data.revision(), before + 1);
    }
}
