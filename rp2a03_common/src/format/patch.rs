//! `rp2a03_common\src\format\patch.rs`
//! `.rp2a03patch` native preset file format: encode/decode/validate for one
//! instrument's envelope/sequence data, active-slot selection, per-slot
//! waveforms, and engine speed, plus conversion to/from the plugin's live
//! `SharedSequences`.
//!
//! Format spec: docs\format.md — read it before changing the wire format.

use crate::{
    ChannelMode, FDS_MOD_TABLE_LEN, FDS_MOD_TABLE_MAX, FDS_MOD_TABLE_MIN, FDS_WAVE_LEN,
    FDS_WAVE_MAX, FdsSettings, Lane, MAX_SEQUENCES, MAX_SLOTS, SequenceBank, SequenceSlot,
    SharedSequences, WaveSlot, WaveSynthParams, sequence_to_text_for_tab,
};
use rp2a03_core::sequencer::{ArpMode, PitchMode, Sequence, VolMode, VolMode5B};
use std::fmt;

pub const PATCH_MAGIC: [u8; 4] = *b"RP2P";
pub const CURRENT_FORMAT_VERSION: u32 = 1;
const MIN_STEP_TIME_HZ: u16 = 1;
const MAX_STEP_TIME_HZ: u16 = 600;
pub const MAX_SEQUENCE_LEN: usize = 256;
const MAX_DUTY_SEQUENCE_VALUE: i16 = 0x0FFF;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Patch {
    pub format_version: u32,
    pub step_time_hz: u16,
    pub active_indices: ActiveIndices,
    pub sequences: PatchSequences,

    #[serde(default)]
    pub slot_waveforms: Vec<PatchSlotWaveform>,
    #[serde(default)]
    pub wave_slots: Vec<Vec<u16>>,
    #[serde(default)]
    pub wave_current_slot: usize,
    #[serde(default)]
    pub fds_settings: Vec<PatchFdsSettings>,
    #[serde(default)]
    pub wavesynth: Vec<PatchWaveSynth>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActiveIndices {
    pub vol: usize,
    pub arp: usize,
    pub pitch: usize,
    pub hipitch: usize,
    pub duty: usize,
}

impl ActiveIndices {
    fn entries(&self) -> [(Lane, usize); Lane::COUNT] {
        [
            (Lane::Vol, self.vol),
            (Lane::Arp, self.arp),
            (Lane::Pitch, self.pitch),
            (Lane::HiPitch, self.hipitch),
            (Lane::Duty, self.duty),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PatchSequences {
    pub vol: Vec<PatchSequenceEntry>,
    pub arp: Vec<PatchSequenceEntry>,
    pub pitch: Vec<PatchSequenceEntry>,
    pub hipitch: Vec<PatchSequenceEntry>,
    pub duty: Vec<PatchSequenceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PatchSequenceEntry {
    pub index: usize,
    pub values: Vec<i16>,
    pub loop_point: Option<usize>,
    pub release_point: Option<usize>,
    pub pitch_mode: PitchMode,
    pub arp_mode: ArpMode,
    #[serde(default)]
    pub vol_mode: VolMode,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub vol_mode_5b: VolMode5B,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PatchSlotWaveform {
    pub index: usize,
    pub waveform: ChannelMode,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PatchFdsSettings {
    pub index: usize,
    pub mod_depth: i32,
    pub mod_speed: i32,
    pub mod_table: Vec<i16>,
    pub mod_delay: i32,
}

impl PatchFdsSettings {
    fn from_settings(index: usize, settings: &FdsSettings) -> Self {
        Self {
            index,
            mod_depth: settings.mod_depth,
            mod_speed: settings.mod_speed,
            mod_table: settings.mod_table.values.clone(),
            mod_delay: settings.mod_delay,
        }
    }

    fn to_settings(&self) -> FdsSettings {
        FdsSettings {
            mod_depth: self.mod_depth,
            mod_speed: self.mod_speed,
            mod_table: Sequence {
                values: self.mod_table.clone(),
                ..Default::default()
            },
            mod_delay: self.mod_delay,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PatchWaveSynth {
    pub index: usize,
    pub params: WaveSynthParams,
}

fn default_enabled() -> bool {
    true
}

impl PatchSequenceEntry {
    fn from_slot(index: usize, slot: &SequenceSlot) -> Option<Self> {
        let seq = &slot.sequence;
        if seq.is_empty() {
            return None;
        }
        Some(Self {
            index,
            values: seq.values.clone(),
            loop_point: seq.loop_point,
            release_point: seq.release_point,
            pitch_mode: seq.pitch_mode,
            arp_mode: seq.arp_mode,
            vol_mode: seq.vol_mode,
            enabled: slot.enabled,
            vol_mode_5b: seq.vol_mode_5b,
        })
    }

    fn to_sequence(&self) -> Sequence {
        Sequence {
            values: self.values.clone(),
            loop_point: self.loop_point,
            release_point: self.release_point,
            pitch_mode: self.pitch_mode,
            arp_mode: self.arp_mode,
            vol_mode: self.vol_mode,
            vol_mode_5b: self.vol_mode_5b,
        }
    }
}

/// Range-checks `index` against `MAX_SEQUENCES` and marks it seen in `seen`,
/// rejecting a repeat. Shared by every wire-format array whose entries carry
/// their own `index: usize` (sequences, `slot_waveforms`, `fds_settings`,
/// `wavesynth`) — each has its own pair of `PatchError` variants for the two
/// failure cases, since the error needs to name which array it came from.
fn check_index(
    seen: &mut [bool; MAX_SEQUENCES],
    index: usize,
    out_of_range: PatchError,
    duplicate: PatchError,
) -> Result<(), PatchError> {
    if index >= MAX_SEQUENCES {
        return Err(out_of_range);
    }
    if std::mem::replace(&mut seen[index], true) {
        return Err(duplicate);
    }
    Ok(())
}

fn collect_used_entries(bank: &SequenceBank) -> Vec<PatchSequenceEntry> {
    bank.slots()
        .iter()
        .enumerate()
        .filter_map(|(index, slot)| PatchSequenceEntry::from_slot(index, slot))
        .collect()
}

#[derive(Debug)]
pub enum PatchError {
    BadMagic,
    Decode(rmp_serde::decode::Error),
    UnsupportedVersion {
        found: u32,
        max_supported: u32,
    },
    StepTimeOutOfRange(u16),
    ActiveIndexOutOfRange {
        envelope: &'static str,
        index: usize,
    },
    InvalidSequenceIndex {
        envelope: &'static str,
        index: usize,
    },
    DuplicateSequenceIndex {
        envelope: &'static str,
        index: usize,
    },
    SequenceValueOutOfRange {
        envelope: &'static str,
        index: usize,
        value: i16,
    },
    SequenceTooLong {
        envelope: &'static str,
        index: usize,
        len: usize,
    },
    MarkerOutOfRange {
        envelope: &'static str,
        index: usize,
        marker: &'static str,
        position: usize,
    },
    InvalidSlotWaveformIndex {
        index: usize,
    },
    DuplicateSlotWaveformIndex {
        index: usize,
    },

    TooManyWaveSlots {
        len: usize,
    },

    InvalidWaveSlotLength {
        index: usize,
        len: usize,
    },

    WaveSlotValueOutOfRange {
        index: usize,
        value: u16,
    },

    InvalidFdsSettingsIndex {
        index: usize,
    },

    DuplicateFdsSettingsIndex {
        index: usize,
    },

    InvalidModTableLength {
        index: usize,
        len: usize,
    },

    ModTableValueOutOfRange {
        index: usize,
        value: i16,
    },

    InvalidWaveSynthIndex {
        index: usize,
    },

    DuplicateWaveSynthIndex {
        index: usize,
    },
}

impl fmt::Display for PatchError {
    // One `write!` per enum variant, each independent — long because
    // `PatchError` has ~19 variants, not because any arm is complex.
    // Splitting it would fragment the variant-to-message mapping without
    // reducing real complexity anywhere.
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => write!(f, "not a .rp2a03patch file (missing RP2P magic bytes)"),
            Self::Decode(e) => write!(f, "malformed .rp2a03patch payload: {e}"),
            Self::UnsupportedVersion {
                found,
                max_supported,
            } => write!(
                f,
                "unsupported .rp2a03patch format_version {found} (this build understands up to {max_supported})"
            ),
            Self::StepTimeOutOfRange(hz) => write!(
                f,
                "step_time_hz {hz} is out of range ({MIN_STEP_TIME_HZ}..={MAX_STEP_TIME_HZ})"
            ),
            Self::ActiveIndexOutOfRange { envelope, index } => write!(
                f,
                "active_indices.{envelope} = {index} is out of range (0..{MAX_SEQUENCES})"
            ),
            Self::InvalidSequenceIndex { envelope, index } => write!(
                f,
                "{envelope} sequence entry index {index} is out of range (0..{MAX_SEQUENCES})"
            ),
            Self::DuplicateSequenceIndex { envelope, index } => write!(
                f,
                "{envelope} sequence entry index {index} appears more than once"
            ),
            Self::SequenceValueOutOfRange {
                envelope,
                index,
                value,
            } => {
                let range = if *envelope == "duty" {
                    format!("0..={MAX_DUTY_SEQUENCE_VALUE}")
                } else {
                    "-128..=127".to_string()
                };
                write!(
                    f,
                    "{envelope} sequence entry index {index} has a step value {value} out of range ({range})"
                )
            }
            Self::SequenceTooLong {
                envelope,
                index,
                len,
            } => write!(
                f,
                "{envelope} sequence entry index {index} has {len} steps, exceeding the maximum of {MAX_SEQUENCE_LEN}"
            ),
            Self::MarkerOutOfRange {
                envelope,
                index,
                marker,
                position,
            } => write!(
                f,
                "{envelope} sequence entry index {index}'s {marker} = {position} is out of range"
            ),
            Self::InvalidSlotWaveformIndex { index } => write!(
                f,
                "slot_waveforms entry index {index} is out of range (0..{MAX_SEQUENCES})"
            ),
            Self::DuplicateSlotWaveformIndex { index } => {
                write!(
                    f,
                    "slot_waveforms entry index {index} appears more than once"
                )
            }
            Self::TooManyWaveSlots { len } => write!(
                f,
                "wave_slots has {len} entries, exceeding the maximum of {MAX_SLOTS}"
            ),
            Self::InvalidWaveSlotLength { index, len } => write!(
                f,
                "wave_slots entry {index} has {len} values, expected exactly {FDS_WAVE_LEN}"
            ),
            Self::WaveSlotValueOutOfRange { index, value } => write!(
                f,
                "wave_slots entry {index} has a value {value} out of range (0..={FDS_WAVE_MAX})"
            ),
            Self::InvalidFdsSettingsIndex { index } => write!(
                f,
                "fds_settings entry index {index} is out of range (0..{MAX_SEQUENCES})"
            ),
            Self::DuplicateFdsSettingsIndex { index } => {
                write!(f, "fds_settings entry index {index} appears more than once")
            }
            Self::InvalidModTableLength { index, len } => write!(
                f,
                "fds_settings entry {index}'s mod_table has {len} values, expected exactly {FDS_MOD_TABLE_LEN}"
            ),
            Self::ModTableValueOutOfRange { index, value } => write!(
                f,
                "fds_settings entry {index}'s mod_table has a value {value} out of range ({FDS_MOD_TABLE_MIN}..={FDS_MOD_TABLE_MAX})"
            ),
            Self::InvalidWaveSynthIndex { index } => write!(
                f,
                "wavesynth entry index {index} is out of range (0..{MAX_SEQUENCES})"
            ),
            Self::DuplicateWaveSynthIndex { index } => {
                write!(f, "wavesynth entry index {index} appears more than once")
            }
        }
    }
}

impl std::error::Error for PatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            _ => None,
        }
    }
}

impl PatchSequences {
    fn entries(&self) -> [(Lane, &Vec<PatchSequenceEntry>); Lane::COUNT] {
        [
            (Lane::Vol, &self.vol),
            (Lane::Arp, &self.arp),
            (Lane::Pitch, &self.pitch),
            (Lane::HiPitch, &self.hipitch),
            (Lane::Duty, &self.duty),
        ]
    }
}

impl Patch {
    /// # Panics
    ///
    /// Never in practice: the encode target is an in-memory `Vec`, which
    /// cannot fail to accept bytes, so `MessagePack` encoding a well-formed
    /// `Patch` cannot produce an error here.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = PATCH_MAGIC.to_vec();

        rmp_serde::encode::write(&mut out, self).expect("Patch encoding is infallible");
        out
    }

    /// # Errors
    ///
    /// Returns [`PatchError`] if `bytes` doesn't start with the `RP2P` magic,
    /// fails to decode as `MessagePack`, or decodes to a patch [`Patch::validate`] rejects.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PatchError> {
        let payload = bytes
            .strip_prefix(&PATCH_MAGIC)
            .ok_or(PatchError::BadMagic)?;
        let patch: Self = rmp_serde::from_slice(payload).map_err(PatchError::Decode)?;
        patch.validate()?;
        Ok(patch)
    }

    /// # Errors
    ///
    /// Returns [`PatchError`] if the format version is newer than this build
    /// supports, or any index/range field is out of bounds.
    pub fn validate(&self) -> Result<(), PatchError> {
        self.validate_header()?;
        self.validate_sequences()?;
        self.validate_slot_waveforms()?;
        self.validate_wave_slots()?;
        self.validate_fds_settings()?;
        self.validate_wavesynth()
    }

    /// Format version and step time, plus which sequence index each lane has
    /// selected — the fields outside `sequences`/`slot_waveforms`/etc.
    fn validate_header(&self) -> Result<(), PatchError> {
        if self.format_version > CURRENT_FORMAT_VERSION {
            return Err(PatchError::UnsupportedVersion {
                found: self.format_version,
                max_supported: CURRENT_FORMAT_VERSION,
            });
        }
        if !(MIN_STEP_TIME_HZ..=MAX_STEP_TIME_HZ).contains(&self.step_time_hz) {
            return Err(PatchError::StepTimeOutOfRange(self.step_time_hz));
        }
        for (lane, index) in self.active_indices.entries() {
            if index >= MAX_SEQUENCES {
                return Err(PatchError::ActiveIndexOutOfRange {
                    envelope: lane.wire_name(),
                    index,
                });
            }
        }
        Ok(())
    }

    fn validate_sequences(&self) -> Result<(), PatchError> {
        for (lane, entries) in self.sequences.entries() {
            let envelope = lane.wire_name();
            let mut seen = [false; MAX_SEQUENCES];
            for entry in entries {
                check_index(
                    &mut seen,
                    entry.index,
                    PatchError::InvalidSequenceIndex {
                        envelope,
                        index: entry.index,
                    },
                    PatchError::DuplicateSequenceIndex {
                        envelope,
                        index: entry.index,
                    },
                )?;

                if entry.values.len() > MAX_SEQUENCE_LEN {
                    return Err(PatchError::SequenceTooLong {
                        envelope,
                        index: entry.index,
                        len: entry.values.len(),
                    });
                }

                let allowed = if envelope == "duty" {
                    0..=MAX_DUTY_SEQUENCE_VALUE
                } else {
                    i16::from(i8::MIN)..=i16::from(i8::MAX)
                };
                for &value in &entry.values {
                    if !allowed.contains(&value) {
                        return Err(PatchError::SequenceValueOutOfRange {
                            envelope,
                            index: entry.index,
                            value,
                        });
                    }
                }
                for (marker, position) in [
                    ("loop_point", entry.loop_point),
                    ("release_point", entry.release_point),
                ] {
                    if let Some(position) = position.filter(|&p| p > entry.values.len()) {
                        return Err(PatchError::MarkerOutOfRange {
                            envelope,
                            index: entry.index,
                            marker,
                            position,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_slot_waveforms(&self) -> Result<(), PatchError> {
        let mut seen_waveform_slots = [false; MAX_SEQUENCES];
        for entry in &self.slot_waveforms {
            check_index(
                &mut seen_waveform_slots,
                entry.index,
                PatchError::InvalidSlotWaveformIndex { index: entry.index },
                PatchError::DuplicateSlotWaveformIndex { index: entry.index },
            )?;
        }
        Ok(())
    }

    fn validate_wave_slots(&self) -> Result<(), PatchError> {
        if self.wave_slots.len() > MAX_SLOTS {
            return Err(PatchError::TooManyWaveSlots {
                len: self.wave_slots.len(),
            });
        }
        for (index, slot) in self.wave_slots.iter().enumerate() {
            if slot.len() != FDS_WAVE_LEN {
                return Err(PatchError::InvalidWaveSlotLength {
                    index,
                    len: slot.len(),
                });
            }
            for &value in slot {
                if value > FDS_WAVE_MAX {
                    return Err(PatchError::WaveSlotValueOutOfRange { index, value });
                }
            }
        }
        Ok(())
    }

    fn validate_fds_settings(&self) -> Result<(), PatchError> {
        let mut seen_fds_settings = [false; MAX_SEQUENCES];
        for entry in &self.fds_settings {
            check_index(
                &mut seen_fds_settings,
                entry.index,
                PatchError::InvalidFdsSettingsIndex { index: entry.index },
                PatchError::DuplicateFdsSettingsIndex { index: entry.index },
            )?;

            if entry.mod_table.len() != FDS_MOD_TABLE_LEN {
                return Err(PatchError::InvalidModTableLength {
                    index: entry.index,
                    len: entry.mod_table.len(),
                });
            }
            for &value in &entry.mod_table {
                if !(FDS_MOD_TABLE_MIN..=FDS_MOD_TABLE_MAX).contains(&value) {
                    return Err(PatchError::ModTableValueOutOfRange {
                        index: entry.index,
                        value,
                    });
                }
            }
        }
        Ok(())
    }

    fn validate_wavesynth(&self) -> Result<(), PatchError> {
        let mut seen_wavesynth = [false; MAX_SEQUENCES];
        for entry in &self.wavesynth {
            check_index(
                &mut seen_wavesynth,
                entry.index,
                PatchError::InvalidWaveSynthIndex { index: entry.index },
                PatchError::DuplicateWaveSynthIndex { index: entry.index },
            )?;
        }
        Ok(())
    }

    #[must_use]
    pub fn from_shared_sequences(shared: &SharedSequences, step_time_hz: u16) -> Self {
        Self {
            format_version: CURRENT_FORMAT_VERSION,
            step_time_hz,
            active_indices: ActiveIndices {
                vol: shared.selected_sequence_index(Lane::Vol),
                arp: shared.selected_sequence_index(Lane::Arp),
                pitch: shared.selected_sequence_index(Lane::Pitch),
                hipitch: shared.selected_sequence_index(Lane::HiPitch),
                duty: shared.selected_sequence_index(Lane::Duty),
            },
            sequences: PatchSequences {
                vol: collect_used_entries(shared.sequence_bank(Lane::Vol)),
                arp: collect_used_entries(shared.sequence_bank(Lane::Arp)),
                pitch: collect_used_entries(shared.sequence_bank(Lane::Pitch)),
                hipitch: collect_used_entries(shared.sequence_bank(Lane::HiPitch)),
                duty: collect_used_entries(shared.sequence_bank(Lane::Duty)),
            },
            slot_waveforms: (0..MAX_SEQUENCES)
                .filter_map(|index| {
                    let remembered = shared.slot_waveform(index).unwrap_or_default();
                    (remembered != ChannelMode::default()).then_some(PatchSlotWaveform {
                        index,
                        waveform: remembered,
                    })
                })
                .collect(),
            wave_slots: shared
                .wave_slots()
                .slots()
                .iter()
                .map(|slot| slot.data().to_vec())
                .collect(),
            wave_current_slot: shared.wave_slots().current_slot(),
            fds_settings: (0..MAX_SEQUENCES)
                .filter_map(|index| {
                    let settings = shared.fds_settings(index);
                    (*settings != FdsSettings::default())
                        .then(|| PatchFdsSettings::from_settings(index, settings))
                })
                .collect(),
            wavesynth: (0..MAX_SEQUENCES)
                .filter_map(|index| {
                    let params = shared.wavesynth(index);
                    (params != WaveSynthParams::default())
                        .then_some(PatchWaveSynth { index, params })
                })
                .collect(),
        }
    }

    #[must_use]
    pub fn active_waveform(&self) -> ChannelMode {
        self.slot_waveforms
            .iter()
            .find(|entry| entry.index == self.active_indices.vol)
            .map_or_else(ChannelMode::default, |entry| entry.waveform)
    }

    pub fn apply_to_shared_sequences(&self, shared: &mut SharedSequences) {
        shared.clear_all_sequences();
        for (lane, entries) in self.sequences.entries() {
            let bank = shared.sequence_bank_mut(lane);
            for entry in entries {
                let sequence = entry.to_sequence();

                let slot_waveform = self
                    .slot_waveforms
                    .iter()
                    .find(|w| w.index == entry.index)
                    .map_or_else(ChannelMode::default, |w| w.waveform);
                let text = sequence_to_text_for_tab(lane, slot_waveform, &sequence);
                let slot = bank.slot_mut(entry.index);
                slot.text = text;
                slot.enabled = entry.enabled;
                slot.sequence = sequence;
            }
        }
        for (lane, index) in self.active_indices.entries() {
            shared.set_selected_sequence_index(lane, index);
        }

        for entry in &self.slot_waveforms {
            shared.set_slot_waveform(entry.index, entry.waveform);
        }

        let slots = self
            .wave_slots
            .iter()
            .map(|values| {
                let mut slot = WaveSlot::new();
                slot.set_data(values);
                slot
            })
            .collect();
        shared
            .wave_slots_mut()
            .set_slots(slots, self.wave_current_slot);

        for entry in &self.fds_settings {
            *shared.fds_settings_mut(entry.index) = entry.to_settings();
        }
        for entry in &self.wavesynth {
            shared.set_wavesynth(entry.index, entry.params);
        }
    }
}

#[derive(Debug)]
pub enum PatchFileError {
    Io(std::io::Error),
    Format(PatchError),
}

impl fmt::Display for PatchFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Format(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PatchFileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Format(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for PatchFileError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<PatchError> for PatchFileError {
    fn from(e: PatchError) -> Self {
        Self::Format(e)
    }
}

/// # Errors
///
/// Returns [`PatchFileError`] if `patch` fails validation or the file write fails.
pub fn save_to_path(path: &std::path::Path, patch: &Patch) -> Result<(), PatchFileError> {
    patch.validate()?;
    std::fs::write(path, patch.to_bytes())?;
    Ok(())
}

/// # Errors
///
/// Returns [`PatchFileError`] if the file can't be read or doesn't decode as a valid [`Patch`].
pub fn load_from_path(path: &std::path::Path) -> Result<Patch, PatchFileError> {
    let bytes = std::fs::read(path)?;
    Ok(Patch::from_bytes(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRE_M3_FIXTURE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/pre_m3.rp2a03patch"
    ));

    #[test]
    fn pre_m3_patch_loads_byte_identically() {
        let patch = Patch::from_bytes(PRE_M3_FIXTURE).expect("pre-M3 fixture must still load");

        assert_eq!(patch.format_version, 1);
        assert_eq!(patch.step_time_hz, 45);
        assert_eq!(
            patch.active_indices,
            ActiveIndices {
                vol: 5,
                arp: 6,
                pitch: 7,
                hipitch: 8,
                duty: 9,
            }
        );
        assert_eq!(patch.sequences.vol[0].values, vec![1, 2, 3, 4, 0]);
        assert_eq!(patch.sequences.arp[0].values, vec![1, 2, 3, 4, 10]);
        assert_eq!(patch.sequences.pitch[0].values, vec![1, 2, 3, 4, 20]);
        assert_eq!(patch.sequences.hipitch[0].values, vec![1, 2, 3, 4, 30]);
        assert_eq!(patch.sequences.duty[0].values, vec![1, 2, 3, 4, 40]);
        assert_eq!(patch.slot_waveforms.len(), 2);
        assert!(patch.slot_waveforms.contains(&PatchSlotWaveform {
            index: 5,
            waveform: ChannelMode::Vrc6Saw,
        }));
        assert!(patch.slot_waveforms.contains(&PatchSlotWaveform {
            index: 9,
            waveform: ChannelMode::Fds,
        }));
        assert_eq!(patch.wave_slots.len(), 2);
        assert_eq!(patch.wave_current_slot, 1);
        assert_eq!(patch.fds_settings.len(), 1);
        assert_eq!(patch.fds_settings[0].index, 9);
        assert_eq!(patch.fds_settings[0].mod_depth, 12);
        assert_eq!(patch.wavesynth.len(), 1);
        assert_eq!(patch.wavesynth[0].index, 9);

        // Round trip must reproduce the exact wire bytes — proves the Lane
        // refactor didn't shift field order, add/remove a field, or rename an
        // enum variant on the wire.
        assert_eq!(patch.to_bytes(), PRE_M3_FIXTURE);
    }
}
