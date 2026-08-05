//! rp2a03_common\src\format\patch.rs
//! `.rp2a03patch` native preset file format: encode/decode/validate for one
//! instrument's envelope/sequence data, active-slot selection, waveform, and
//! engine speed, plus conversion to/from the plugin's live `SharedSequences`.
//!
//! Format spec: Ideas-ref-folder\MARKDOWNs\Save&Load\Rp2a03_patch_format.md
//! (outside version control — see that file's own header for why).

use crate::{ChannelMode, MAX_SEQUENCES, SequenceBank, SequenceSlot, SharedSequences, sequence_to_text};
use rp2a03_core::sequencer::{ArpMode, PitchMode, Sequence, VolMode};
use std::fmt;

/// Fixed file-identity prefix, never bumped — see "Wire format" in the spec.
pub const PATCH_MAGIC: [u8; 4] = *b"RP2P";
pub const CURRENT_FORMAT_VERSION: u32 = 1;
const MIN_STEP_TIME_HZ: u16 = 1;
const MAX_STEP_TIME_HZ: u16 = 600;

/// Top-level `.rp2a03patch` contents. Field order is load-bearing — see the
/// "Schema evolution rule" section of the format spec. New fields are always
/// appended after `sequences`; existing fields are never reordered.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Patch {
    pub format_version: u32,
    pub waveform: ChannelMode,
    pub step_time_hz: u16,
    pub active_indices: ActiveIndices,
    pub sequences: PatchSequences,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActiveIndices {
    pub vol: usize,
    pub arp: usize,
    pub pitch: usize,
    pub hipitch: usize,
    pub duty: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PatchSequences {
    pub vol: Vec<PatchSequenceEntry>,
    pub arp: Vec<PatchSequenceEntry>,
    pub pitch: Vec<PatchSequenceEntry>,
    pub hipitch: Vec<PatchSequenceEntry>,
    pub duty: Vec<PatchSequenceEntry>,
}

/// One populated numbered slot within an envelope type's bank. Field order is
/// load-bearing (see `Patch`'s doc comment); `vol_mode` must stay last.
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
        }
    }
}

fn collect_used_entries(bank: &SequenceBank) -> Vec<PatchSequenceEntry> {
    bank.slots()
        .iter()
        .enumerate()
        .filter_map(|(index, slot)| PatchSequenceEntry::from_slot(index, slot))
        .collect()
}

/// Failure decoding or validating a `.rp2a03patch` byte payload.
#[derive(Debug)]
pub enum PatchError {
    /// The file doesn't start with the `RP2P` magic bytes.
    BadMagic,
    /// The MessagePack payload after the magic bytes is malformed.
    Decode(rmp_serde::decode::Error),
    /// `format_version` is newer than this build understands.
    UnsupportedVersion { found: u32, max_supported: u32 },
    /// `step_time_hz` is outside `1..=600`.
    StepTimeOutOfRange(u16),
    /// An `active_indices` entry is `>= MAX_SEQUENCES`.
    ActiveIndexOutOfRange { envelope: &'static str, index: usize },
    /// A sequence entry's `index` is `>= MAX_SEQUENCES`.
    InvalidSequenceIndex { envelope: &'static str, index: usize },
    /// The same `index` appears more than once in one envelope type's array.
    DuplicateSequenceIndex { envelope: &'static str, index: usize },
}

impl fmt::Display for PatchError {
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
    /// Envelope name, tab index, and entries for all 5 envelope types, in the
    /// codebase's standard tab order (see `ActiveSequences`/`SequenceReload`
    /// in `rp2a03_common::midi::types`).
    fn entries(&self) -> [(&'static str, usize, &Vec<PatchSequenceEntry>); 5] {
        [
            ("vol", 0, &self.vol),
            ("arp", 1, &self.arp),
            ("pitch", 2, &self.pitch),
            ("hipitch", 3, &self.hipitch),
            ("duty", 4, &self.duty),
        ]
    }
}

impl Patch {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = PATCH_MAGIC.to_vec();
        // Vec<u8> writes cannot fail, and `Patch` holds only plain data (no
        // maps, no NaN-sensitive floats) — encoding cannot fail either.
        rmp_serde::encode::write(&mut out, self).expect("Patch encoding is infallible");
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PatchError> {
        let payload = bytes.strip_prefix(&PATCH_MAGIC).ok_or(PatchError::BadMagic)?;
        let patch: Self = rmp_serde::from_slice(payload).map_err(PatchError::Decode)?;
        patch.validate()?;
        Ok(patch)
    }

    fn validate(&self) -> Result<(), PatchError> {
        if self.format_version > CURRENT_FORMAT_VERSION {
            return Err(PatchError::UnsupportedVersion {
                found: self.format_version,
                max_supported: CURRENT_FORMAT_VERSION,
            });
        }
        if !(MIN_STEP_TIME_HZ..=MAX_STEP_TIME_HZ).contains(&self.step_time_hz) {
            return Err(PatchError::StepTimeOutOfRange(self.step_time_hz));
        }
        for (envelope, index) in [
            ("vol", self.active_indices.vol),
            ("arp", self.active_indices.arp),
            ("pitch", self.active_indices.pitch),
            ("hipitch", self.active_indices.hipitch),
            ("duty", self.active_indices.duty),
        ] {
            if index >= MAX_SEQUENCES {
                return Err(PatchError::ActiveIndexOutOfRange { envelope, index });
            }
        }
        for (envelope, _tab, entries) in self.sequences.entries() {
            let mut seen = [false; MAX_SEQUENCES];
            for entry in entries {
                if entry.index >= MAX_SEQUENCES {
                    return Err(PatchError::InvalidSequenceIndex {
                        envelope,
                        index: entry.index,
                    });
                }
                if seen[entry.index] {
                    return Err(PatchError::DuplicateSequenceIndex {
                        envelope,
                        index: entry.index,
                    });
                }
                seen[entry.index] = true;
            }
        }
        Ok(())
    }

    pub fn from_shared_sequences(
        shared: &SharedSequences,
        waveform: ChannelMode,
        step_time_hz: u16,
    ) -> Self {
        Self {
            format_version: CURRENT_FORMAT_VERSION,
            waveform,
            step_time_hz,
            active_indices: ActiveIndices {
                vol: shared.selected_sequence_index(0),
                arp: shared.selected_sequence_index(1),
                pitch: shared.selected_sequence_index(2),
                hipitch: shared.selected_sequence_index(3),
                duty: shared.selected_sequence_index(4),
            },
            sequences: PatchSequences {
                vol: collect_used_entries(shared.sequence_bank(0)),
                arp: collect_used_entries(shared.sequence_bank(1)),
                pitch: collect_used_entries(shared.sequence_bank(2)),
                hipitch: collect_used_entries(shared.sequence_bank(3)),
                duty: collect_used_entries(shared.sequence_bank(4)),
            },
        }
    }

    /// Replaces `shared`'s sequence-bank content and active-slot selections
    /// with this patch's. Does **not** touch `shared.channel_mode` or any
    /// host parameter — `waveform`/`step_time_hz` are plain fields on `Patch`
    /// for the caller to push through the host parameter system instead (see
    /// this plan's "What this plan deliberately does NOT wire up").
    pub fn apply_to_shared_sequences(&self, shared: &mut SharedSequences) {
        shared.clear_all_sequences();
        for (_envelope, tab, entries) in self.sequences.entries() {
            let bank = shared.sequence_bank_mut(tab);
            for entry in entries {
                let sequence = entry.to_sequence();
                let slot = bank.slot_mut(entry.index);
                slot.text = sequence_to_text(&sequence);
                slot.enabled = !sequence.is_empty();
                slot.sequence = sequence;
            }
        }
        shared.set_selected_sequence_index(0, self.active_indices.vol);
        shared.set_selected_sequence_index(1, self.active_indices.arp);
        shared.set_selected_sequence_index(2, self.active_indices.pitch);
        shared.set_selected_sequence_index(3, self.active_indices.hipitch);
        shared.set_selected_sequence_index(4, self.active_indices.duty);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(index: usize) -> PatchSequenceEntry {
        PatchSequenceEntry {
            index,
            values: vec![15, 12, 8],
            loop_point: Some(1),
            release_point: None,
            pitch_mode: PitchMode::Relative,
            arp_mode: ArpMode::Absolute,
            vol_mode: VolMode::Steps16,
        }
    }

    fn sample_patch() -> Patch {
        Patch {
            format_version: CURRENT_FORMAT_VERSION,
            waveform: ChannelMode::Pulse,
            step_time_hz: 60,
            active_indices: ActiveIndices {
                vol: 3,
                arp: 0,
                pitch: 0,
                hipitch: 0,
                duty: 3,
            },
            sequences: PatchSequences {
                vol: vec![sample_entry(3)],
                arp: vec![],
                pitch: vec![],
                hipitch: vec![],
                duty: vec![sample_entry(3)],
            },
        }
    }

    #[test]
    fn round_trips_through_bytes() {
        let patch = sample_patch();
        let bytes = patch.to_bytes();
        let restored = Patch::from_bytes(&bytes).expect("valid patch must decode");
        assert_eq!(restored, patch);
    }

    #[test]
    fn file_starts_with_magic_bytes() {
        let bytes = sample_patch().to_bytes();
        assert_eq!(&bytes[..4], b"RP2P");
    }

    #[test]
    fn legacy_entry_without_vol_mode_defaults_to_steps16() {
        // Proves the array-mode back-compat mechanism the whole wire format
        // design depends on: an older-shaped struct missing the trailing
        // `vol_mode` field must still decode, defaulting that field.
        #[derive(serde::Serialize)]
        struct LegacyEntry {
            index: usize,
            values: Vec<i16>,
            loop_point: Option<usize>,
            release_point: Option<usize>,
            pitch_mode: PitchMode,
            arp_mode: ArpMode,
        }

        let legacy = LegacyEntry {
            index: 5,
            values: vec![10, 20],
            loop_point: None,
            release_point: None,
            pitch_mode: PitchMode::Relative,
            arp_mode: ArpMode::Absolute,
        };

        let bytes = rmp_serde::to_vec(&legacy).expect("legacy struct must encode");
        let restored: PatchSequenceEntry =
            rmp_serde::from_slice(&bytes).expect("legacy bytes must still decode");
        assert_eq!(restored.vol_mode, VolMode::Steps16);
        assert_eq!(restored.values, vec![10, 20]);
    }

    #[test]
    fn rejects_missing_magic_bytes() {
        let err = Patch::from_bytes(&[1, 2, 3]).unwrap_err();
        assert!(matches!(err, PatchError::BadMagic));
    }

    #[test]
    fn rejects_empty_file() {
        let err = Patch::from_bytes(&[]).unwrap_err();
        assert!(matches!(err, PatchError::BadMagic));
    }

    #[test]
    fn rejects_unsupported_format_version() {
        let mut patch = sample_patch();
        patch.format_version = CURRENT_FORMAT_VERSION + 1;
        let bytes = patch.to_bytes();
        let err = Patch::from_bytes(&bytes).unwrap_err();
        assert!(matches!(
            err,
            PatchError::UnsupportedVersion { found, max_supported }
                if found == CURRENT_FORMAT_VERSION + 1 && max_supported == CURRENT_FORMAT_VERSION
        ));
    }

    #[test]
    fn rejects_step_time_out_of_range() {
        let mut patch = sample_patch();
        patch.step_time_hz = 0;
        let bytes = patch.to_bytes();
        let err = Patch::from_bytes(&bytes).unwrap_err();
        assert!(matches!(err, PatchError::StepTimeOutOfRange(0)));

        let mut patch = sample_patch();
        patch.step_time_hz = 601;
        let bytes = patch.to_bytes();
        let err = Patch::from_bytes(&bytes).unwrap_err();
        assert!(matches!(err, PatchError::StepTimeOutOfRange(601)));
    }

    #[test]
    fn accepts_step_time_hz_boundaries() {
        for hz in [1u16, 600u16] {
            let mut patch = sample_patch();
            patch.step_time_hz = hz;
            let bytes = patch.to_bytes();
            assert!(
                Patch::from_bytes(&bytes).is_ok(),
                "step_time_hz = {hz} is within 1..=600 and must be accepted"
            );
        }
    }

    #[test]
    fn rejects_active_index_out_of_range() {
        let mut patch = sample_patch();
        patch.active_indices.vol = MAX_SEQUENCES;
        let bytes = patch.to_bytes();
        let err = Patch::from_bytes(&bytes).unwrap_err();
        assert!(matches!(
            err,
            PatchError::ActiveIndexOutOfRange { envelope: "vol", index } if index == MAX_SEQUENCES
        ));
    }

    #[test]
    fn rejects_sequence_index_out_of_range() {
        let mut patch = sample_patch();
        patch.sequences.vol[0].index = MAX_SEQUENCES;
        let bytes = patch.to_bytes();
        let err = Patch::from_bytes(&bytes).unwrap_err();
        assert!(matches!(
            err,
            PatchError::InvalidSequenceIndex { envelope: "vol", index } if index == MAX_SEQUENCES
        ));
    }

    #[test]
    fn rejects_duplicate_sequence_index() {
        let mut patch = sample_patch();
        patch.sequences.vol.push(sample_entry(3));
        let bytes = patch.to_bytes();
        let err = Patch::from_bytes(&bytes).unwrap_err();
        assert!(matches!(
            err,
            PatchError::DuplicateSequenceIndex { envelope: "vol", index: 3 }
        ));
    }

    #[test]
    fn rejects_duplicate_sequence_index_for_duty() {
        // Proves the reported envelope name tracks the envelope actually at
        // fault (not just "vol") — catches an envelope-name/array mismatch in
        // `PatchSequences::entries` that a vol-only test would miss.
        let mut patch = sample_patch();
        patch.sequences.duty.push(sample_entry(3));
        let bytes = patch.to_bytes();
        let err = Patch::from_bytes(&bytes).unwrap_err();
        assert!(matches!(
            err,
            PatchError::DuplicateSequenceIndex { envelope: "duty", index: 3 }
        ));
    }

    #[test]
    fn from_shared_sequences_collects_only_used_slots() {
        let mut shared = SharedSequences::default();
        shared.set_selected_sequence_index(0, 5); // vol tab, slot 5
        shared.selected_sequence_mut(0).1.values.extend([1, 2, 3]);

        let patch = Patch::from_shared_sequences(&shared, ChannelMode::Pulse, 60);

        assert_eq!(patch.sequences.vol.len(), 1);
        assert_eq!(patch.sequences.vol[0].index, 5);
        assert_eq!(patch.sequences.vol[0].values, vec![1, 2, 3]);
        assert!(patch.sequences.arp.is_empty());
        assert_eq!(patch.active_indices.vol, 5);
        assert_eq!(patch.waveform, ChannelMode::Pulse);
        assert_eq!(patch.step_time_hz, 60);
    }

    #[test]
    fn apply_to_shared_sequences_round_trips_and_derives_enabled() {
        let mut original = SharedSequences::default();
        original.set_selected_sequence_index(0, 3); // vol
        original.selected_sequence_mut(0).1.values = vec![15, 14, 12];
        original.set_selected_sequence_index(4, 3); // duty
        original.selected_sequence_mut(4).1.values = vec![0, 2];

        let patch = Patch::from_shared_sequences(&original, ChannelMode::Pulse, 60);

        let mut restored = SharedSequences::default();
        patch.apply_to_shared_sequences(&mut restored);

        assert_eq!(restored.selected_sequence_index(0), 3);
        assert_eq!(restored.selected_sequence(0).values, vec![15, 14, 12]);
        assert!(restored.sequence_enabled(0));
        assert_eq!(restored.selected_sequence_index(4), 3);
        assert_eq!(restored.selected_sequence(4).values, vec![0, 2]);
    }

    #[test]
    fn apply_to_shared_sequences_clears_slots_not_present_in_the_patch() {
        let mut shared = SharedSequences::default();
        shared.selected_sequence_mut(1).1.values.push(9); // arp slot 0
        *shared.sequence_enabled_mut(1) = true;

        let empty_patch =
            Patch::from_shared_sequences(&SharedSequences::default(), ChannelMode::Pulse, 60);
        empty_patch.apply_to_shared_sequences(&mut shared);

        assert!(shared.selected_sequence(1).is_empty());
        assert!(!shared.sequence_enabled(1));
    }
}
