//! rp2a03_common\src\format\patch.rs
//! `.rp2a03patch` native preset file format: encode/decode/validate for one
//! instrument's envelope/sequence data, active-slot selection, per-slot
//! waveforms, and engine speed, plus conversion to/from the plugin's live
//! `SharedSequences`.
//!
//! Format spec: docs\format.md — read it before changing the wire format.

use crate::{
    ChannelMode, MAX_SEQUENCES, SequenceBank, SequenceSlot, SharedSequences, sequence_to_text,
};
use rp2a03_core::sequencer::{ArpMode, PitchMode, Sequence, VolMode};
use std::fmt;

/// Fixed file-identity prefix, never bumped — see "Wire format" in the spec.
pub const PATCH_MAGIC: [u8; 4] = *b"RP2P";
/// The highest `Patch::format_version` this build can load. `validate`
/// rejects any file whose `format_version` is greater than this. Bumped only
/// when a schema change cannot be expressed as an appended,
/// `#[serde(default)]`-backed field (see the spec's "`format_version`"
/// section) — e.g. a field is removed, repurposed, or a struct's field order
/// needs to change.
pub const CURRENT_FORMAT_VERSION: u32 = 1;
const MIN_STEP_TIME_HZ: u16 = 1;
const MAX_STEP_TIME_HZ: u16 = 600;
/// Upper bound on `PatchSequenceEntry::values.len()`, matching the GUI's own
/// sequence-length cap (`rp2a03_common::gui::editor`'s size-`DragValue`
/// `range(0..=256)`). Without this cap a crafted multi-megabyte file can
/// decode into a multi-million-step sequence; `Sequence::clone_from` is
/// documented allocation-free for real-time safety
/// (`rp2a03_core/src/sequencer.rs`), so an oversized sequence would blow that
/// budget the moment it's played.
pub const MAX_SEQUENCE_LEN: usize = 256;

/// Top-level `.rp2a03patch` contents. Field order is load-bearing — see the
/// "Schema evolution rule" section of the format spec. New fields are always
/// appended after `sequences`; existing fields are never reordered.
///
/// This format also embeds several unit-only enums (`ChannelMode`,
/// `PitchMode`, `ArpMode`, `VolMode`). `rmp-serde` writes unit variants as
/// their **name string**, not their discriminant — the exact inverse of the
/// struct-field rule above: variant **names** are load-bearing (renaming
/// `Vrc6Saw`, `Steps16`, `Relative`, etc. breaks every existing file), while
/// variant **order**/discriminant values are not (reordering or renumbering
/// them is harmless).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Patch {
    pub format_version: u32,
    pub step_time_hz: u16,
    pub active_indices: ActiveIndices,
    pub sequences: PatchSequences,
    /// The waveform each numbered slot remembers, sparsely encoded: a slot
    /// whose waveform is `ChannelMode::default()` is omitted entirely and
    /// decodes back to the default via `SharedSequences::default`, the same way
    /// an empty sequence slot is omitted from `sequences`.
    ///
    /// This is also the *only* place the instrument's live channel selection
    /// lives on disk: the channel a load switches to is whatever
    /// `active_indices.vol`'s slot remembers here (see `active_waveform`),
    /// rather than a separately stored value — the live plugin keeps those two
    /// always in agreement (see `apply_channel_mode_change`'s and the waveform
    /// combo box's write-through in `rp2a03_common::gui::editor`), so storing
    /// both would just be the same fact twice.
    #[serde(default)]
    pub slot_waveforms: Vec<PatchSlotWaveform>,
}

/// Field order is load-bearing — see `Patch`'s doc comment. New fields are
/// always appended at the end; existing fields are never reordered.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActiveIndices {
    pub vol: usize,
    pub arp: usize,
    pub pitch: usize,
    pub hipitch: usize,
    pub duty: usize,
}

/// Field order is load-bearing — see `Patch`'s doc comment. New fields are
/// always appended at the end; existing fields are never reordered.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PatchSequences {
    pub vol: Vec<PatchSequenceEntry>,
    pub arp: Vec<PatchSequenceEntry>,
    pub pitch: Vec<PatchSequenceEntry>,
    pub hipitch: Vec<PatchSequenceEntry>,
    pub duty: Vec<PatchSequenceEntry>,
}

/// One populated numbered slot within an envelope type's bank. Field order is
/// load-bearing (see `Patch`'s doc comment); `enabled` must stay last — any
/// future field is appended after it.
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
    /// Whether this envelope was enabled when saved. Stored (not derived) so
    /// a populated-but-disabled slot round-trips correctly — an empty slot is
    /// simply omitted from the file entirely, but a disabled non-empty slot
    /// is authored data. Absent/older files (no `enabled` key) default to
    /// `true` via `default_enabled`, matching every pre-existing entry, which
    /// was always enabled in practice.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

/// One numbered slot's remembered waveform. Field order is load-bearing — see
/// `Patch`'s doc comment; new fields are always appended after `waveform`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PatchSlotWaveform {
    pub index: usize,
    pub waveform: ChannelMode,
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
    ActiveIndexOutOfRange {
        envelope: &'static str,
        index: usize,
    },
    /// A sequence entry's `index` is `>= MAX_SEQUENCES`.
    InvalidSequenceIndex {
        envelope: &'static str,
        index: usize,
    },
    /// The same `index` appears more than once in one envelope type's array.
    DuplicateSequenceIndex {
        envelope: &'static str,
        index: usize,
    },
    /// A sequence entry's `values` contains an element outside `-128..=127`
    /// (`Sequence::values`' documented range — Dn-FamiTracker stores sequence
    /// items as `signed char`).
    SequenceValueOutOfRange {
        envelope: &'static str,
        index: usize,
        value: i16,
    },
    /// A sequence entry's `values.len()` exceeds `MAX_SEQUENCE_LEN`.
    SequenceTooLong {
        envelope: &'static str,
        index: usize,
        len: usize,
    },
    /// A sequence entry's `loop_point`/`release_point` is `> values.len()`
    /// (a marker positioned exactly at `values.len()` is legal).
    MarkerOutOfRange {
        envelope: &'static str,
        index: usize,
        marker: &'static str,
        position: usize,
    },
    /// A `slot_waveforms` entry's `index` is `>= MAX_SEQUENCES`.
    InvalidSlotWaveformIndex { index: usize },
    /// The same `index` appears more than once in `slot_waveforms`. Rejected
    /// rather than resolved last-write-wins, matching `DuplicateSequenceIndex`.
    ///
    /// This also caps an *accepted* patch at `MAX_SEQUENCES` entries, so no
    /// downstream code has to cope with a vec padded out with repeats. It is
    /// not a bound on decode-time memory: like every check in `validate`, it
    /// runs after `rmp_serde` has already materialized the whole vec, so what
    /// actually limits that allocation is the file's own size (each entry costs
    /// bytes on the wire). `MAX_SEQUENCE_LEN` carries the same caveat.
    DuplicateSlotWaveformIndex { index: usize },
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
            Self::SequenceValueOutOfRange {
                envelope,
                index,
                value,
            } => write!(
                f,
                "{envelope} sequence entry index {index} has a step value {value} out of range (-128..=127)"
            ),
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
        let payload = bytes
            .strip_prefix(&PATCH_MAGIC)
            .ok_or(PatchError::BadMagic)?;
        let patch: Self = rmp_serde::from_slice(payload).map_err(PatchError::Decode)?;
        patch.validate()?;
        Ok(patch)
    }

    /// Checks every loader invariant documented in the format spec's
    /// "Invariants a loader should validate" section (version, ranges,
    /// sequence value bounds, marker bounds, duplicate indices). `from_bytes`
    /// always calls this; it is also `pub` so a future `.fti` importer (or
    /// any other code constructing a `Patch` from foreign data, bypassing
    /// `from_bytes` entirely) can validate before use.
    pub fn validate(&self) -> Result<(), PatchError> {
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

                if entry.values.len() > MAX_SEQUENCE_LEN {
                    return Err(PatchError::SequenceTooLong {
                        envelope,
                        index: entry.index,
                        len: entry.values.len(),
                    });
                }
                for &value in &entry.values {
                    if !(i16::from(i8::MIN)..=i16::from(i8::MAX)).contains(&value) {
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
        let mut seen_waveform_slots = [false; MAX_SEQUENCES];
        for entry in &self.slot_waveforms {
            if entry.index >= MAX_SEQUENCES {
                return Err(PatchError::InvalidSlotWaveformIndex { index: entry.index });
            }
            if seen_waveform_slots[entry.index] {
                return Err(PatchError::DuplicateSlotWaveformIndex { index: entry.index });
            }
            seen_waveform_slots[entry.index] = true;
        }
        Ok(())
    }

    pub fn from_shared_sequences(shared: &SharedSequences, step_time_hz: u16) -> Self {
        Self {
            format_version: CURRENT_FORMAT_VERSION,
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
            slot_waveforms: (0..MAX_SEQUENCES)
                .filter_map(|index| {
                    let remembered = shared.slot_waveform(index);
                    (remembered != ChannelMode::default()).then_some(PatchSlotWaveform {
                        index,
                        waveform: remembered,
                    })
                })
                .collect(),
        }
    }

    /// The waveform `active_indices.vol`'s slot remembers — the channel a load
    /// switches the instrument to. Defaults to `ChannelMode::default()`
    /// (`Pulse`) if that slot has no `slot_waveforms` entry, the same fallback
    /// `SharedSequences::slot_waveform` uses.
    pub fn active_waveform(&self) -> ChannelMode {
        self.slot_waveforms
            .iter()
            .find(|entry| entry.index == self.active_indices.vol)
            .map_or(ChannelMode::default(), |entry| entry.waveform)
    }

    /// Replaces `shared`'s sequence-bank content, active-slot selections, and
    /// per-slot remembered waveforms with this patch's. Does **not** touch the
    /// *live* `shared.channel_mode` (a different, independently-settable field
    /// that the editor keeps in sync with the active slot's remembered
    /// waveform, but which this function has no reason to touch directly) or
    /// any host parameter — `step_time_hz` is a plain field on `Patch` for the
    /// caller to push through the host parameter system instead, because
    /// `SequenceCache::refresh` re-asserts params from the audio thread every
    /// block and would stomp anything written here directly.
    /// `rp2a03_common::gui::editor::apply_loaded_patch` is that caller; it
    /// sets `shared.channel_mode` itself (from `active_waveform`, before
    /// calling this) and hands `step_time_hz` back through `EditorResult`.
    pub fn apply_to_shared_sequences(&self, shared: &mut SharedSequences) {
        shared.clear_all_sequences();
        for (_envelope, tab, entries) in self.sequences.entries() {
            let bank = shared.sequence_bank_mut(tab);
            for entry in entries {
                let sequence = entry.to_sequence();
                let slot = bank.slot_mut(entry.index);
                slot.text = sequence_to_text(&sequence);
                slot.enabled = entry.enabled;
                slot.sequence = sequence;
            }
        }
        shared.set_selected_sequence_index(0, self.active_indices.vol);
        shared.set_selected_sequence_index(1, self.active_indices.arp);
        shared.set_selected_sequence_index(2, self.active_indices.pitch);
        shared.set_selected_sequence_index(3, self.active_indices.hipitch);
        shared.set_selected_sequence_index(4, self.active_indices.duty);
        // `clear_all_sequences` above reset every slot to the default waveform,
        // so the omitted (default-valued) slots are already correct and only the
        // encoded ones need writing back.
        for entry in &self.slot_waveforms {
            shared.set_slot_waveform(entry.index, entry.waveform);
        }
    }
}

/// Failure saving or loading a `.rp2a03patch` file at a filesystem path.
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

pub fn save_to_path(path: &std::path::Path, patch: &Patch) -> Result<(), PatchFileError> {
    patch.validate()?;
    std::fs::write(path, patch.to_bytes())?;
    Ok(())
}

pub fn load_from_path(path: &std::path::Path) -> Result<Patch, PatchFileError> {
    let bytes = std::fs::read(path)?;
    Ok(Patch::from_bytes(&bytes)?)
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
            enabled: true,
        }
    }

    fn sample_patch() -> Patch {
        Patch {
            format_version: CURRENT_FORMAT_VERSION,
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
            slot_waveforms: vec![PatchSlotWaveform {
                index: 3,
                waveform: ChannelMode::Vrc6Saw,
            }],
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
    fn legacy_entry_without_vol_mode_or_enabled_defaults_correctly() {
        // Proves the array-mode back-compat mechanism the whole wire format
        // design depends on: an older-shaped struct missing the two trailing
        // fields (`vol_mode`, `enabled`) must still decode, defaulting both.
        // `enabled` must default to `true` — a bare `#[serde(default)]` would
        // give `false`, silently disabling every pre-existing envelope.
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
        assert!(
            restored.enabled,
            "legacy payloads must default enabled to true"
        );
    }

    #[test]
    fn apply_to_shared_sequences_round_trips_disabled_but_non_empty_slot() {
        // A populated slot the user explicitly turned OFF must stay off after
        // a save -> load round trip through the actual bytes, not just the
        // two in-memory conversions — going through `to_bytes`/`from_bytes`
        // is what proves `enabled` is really carried on the wire.
        let mut original = SharedSequences::default();
        original.set_selected_sequence_index(0, 6); // vol tab, slot 6
        original.selected_sequence_mut(0).1.values = vec![9, 7, 5];
        *original.sequence_enabled_mut(0) = false;

        let patch = Patch::from_shared_sequences(&original, 60);
        let bytes = patch.to_bytes();
        let restored_patch = Patch::from_bytes(&bytes).expect("round-tripped patch must decode");

        let mut restored = SharedSequences::default();
        restored_patch.apply_to_shared_sequences(&mut restored);

        assert_eq!(restored.selected_sequence_index(0), 6);
        assert_eq!(restored.selected_sequence(0).values, vec![9, 7, 5]);
        assert!(
            !restored.sequence_enabled(0),
            "a populated slot that was disabled before saving must still be disabled after loading"
        );
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
    fn rejects_active_index_out_of_range_for_every_envelope() {
        // `ActiveIndexOutOfRange`'s envelope<->field pairing
        // (`patch.rs::validate`) is a structurally separate literal list from
        // `PatchSequences::entries()` — this loops over all 5 envelopes so a
        // future typo like `("hipitch", self.active_indices.pitch)` would be
        // caught, not just the `"vol"` case the other test exercises.
        for envelope in ["vol", "arp", "pitch", "hipitch", "duty"] {
            let mut patch = sample_patch();
            match envelope {
                "vol" => patch.active_indices.vol = MAX_SEQUENCES,
                "arp" => patch.active_indices.arp = MAX_SEQUENCES,
                "pitch" => patch.active_indices.pitch = MAX_SEQUENCES,
                "hipitch" => patch.active_indices.hipitch = MAX_SEQUENCES,
                "duty" => patch.active_indices.duty = MAX_SEQUENCES,
                _ => unreachable!(),
            }
            let bytes = patch.to_bytes();
            let err = Patch::from_bytes(&bytes).unwrap_err();
            assert!(
                matches!(
                    err,
                    PatchError::ActiveIndexOutOfRange { envelope: e, index }
                        if e == envelope && index == MAX_SEQUENCES
                ),
                "expected ActiveIndexOutOfRange{{envelope: {envelope:?}, ..}}, got {err:?}"
            );
        }
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
            PatchError::DuplicateSequenceIndex {
                envelope: "vol",
                index: 3
            }
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
            PatchError::DuplicateSequenceIndex {
                envelope: "duty",
                index: 3
            }
        ));
    }

    #[test]
    fn rejects_slot_waveform_index_out_of_range() {
        let mut patch = sample_patch();
        patch.slot_waveforms.push(PatchSlotWaveform {
            index: MAX_SEQUENCES,
            waveform: ChannelMode::Noise,
        });
        let bytes = patch.to_bytes();
        let err = Patch::from_bytes(&bytes).unwrap_err();
        assert!(
            matches!(
                err,
                PatchError::InvalidSlotWaveformIndex { index } if index == MAX_SEQUENCES
            ),
            "expected InvalidSlotWaveformIndex, got {err:?}"
        );
    }

    #[test]
    fn rejects_duplicate_slot_waveform_index() {
        // Rejected rather than resolved last-write-wins. This is also what caps
        // an accepted patch at MAX_SEQUENCES entries, so no downstream code has
        // to cope with a vec padded out with repeats — see
        // `DuplicateSlotWaveformIndex`'s docs for why that is not the same
        // thing as a bound on decode-time allocation.
        let mut patch = sample_patch();
        patch.slot_waveforms.push(PatchSlotWaveform {
            index: 3,
            waveform: ChannelMode::Noise,
        });
        let bytes = patch.to_bytes();
        let err = Patch::from_bytes(&bytes).unwrap_err();
        assert!(
            matches!(err, PatchError::DuplicateSlotWaveformIndex { index: 3 }),
            "expected DuplicateSlotWaveformIndex, got {err:?}"
        );
    }

    #[test]
    fn slot_waveforms_round_trip_through_shared_sequences() {
        let mut source = SharedSequences::default();
        source.set_slot_waveform(0, ChannelMode::Triangle);
        source.set_slot_waveform(5, ChannelMode::Noise);
        source.set_slot_waveform(127, ChannelMode::Vrc6Saw);
        // Explicitly set back to the default — must encode as absent, not as an
        // entry, so the sparse encoding stays sparse.
        source.set_slot_waveform(9, ChannelMode::Pulse);

        let patch = Patch::from_shared_sequences(&source, 60);
        assert_eq!(
            patch.slot_waveforms,
            vec![
                PatchSlotWaveform {
                    index: 0,
                    waveform: ChannelMode::Triangle
                },
                PatchSlotWaveform {
                    index: 5,
                    waveform: ChannelMode::Noise
                },
                PatchSlotWaveform {
                    index: 127,
                    waveform: ChannelMode::Vrc6Saw
                },
            ],
            "only slots differing from the default belong in the file"
        );

        let restored = Patch::from_bytes(&patch.to_bytes()).expect("valid patch must decode");
        // Load into state carrying a stale waveform on a slot the patch omits:
        // `clear_all_sequences` must wipe it rather than let it survive.
        let mut target = SharedSequences::default();
        target.set_slot_waveform(20, ChannelMode::Vrc6Pulse);
        restored.apply_to_shared_sequences(&mut target);

        assert_eq!(target.slot_waveform(0), ChannelMode::Triangle);
        assert_eq!(target.slot_waveform(5), ChannelMode::Noise);
        assert_eq!(target.slot_waveform(127), ChannelMode::Vrc6Saw);
        assert_eq!(
            target.slot_waveform(9),
            ChannelMode::Pulse,
            "an omitted slot decodes back to the default"
        );
        assert_eq!(
            target.slot_waveform(20),
            ChannelMode::Pulse,
            "loading a patch must not leave the previous instrument's per-slot \
             waveform on a slot the patch does not mention"
        );
    }

    #[test]
    fn rejects_sequence_value_out_of_range() {
        let mut patch = sample_patch();
        patch.sequences.vol[0].values = vec![15, i16::from(i8::MAX) + 1, 8];
        let bytes = patch.to_bytes();
        let err = Patch::from_bytes(&bytes).unwrap_err();
        assert!(
            matches!(
                err,
                PatchError::SequenceValueOutOfRange {
                    envelope: "vol",
                    index: 3,
                    value: 128
                }
            ),
            "expected SequenceValueOutOfRange{{envelope: \"vol\", index: 3, value: 128}}, got {err:?}"
        );

        let mut patch = sample_patch();
        patch.sequences.vol[0].values = vec![i16::from(i8::MIN) - 1, 12, 8];
        let bytes = patch.to_bytes();
        let err = Patch::from_bytes(&bytes).unwrap_err();
        assert!(
            matches!(
                err,
                PatchError::SequenceValueOutOfRange {
                    envelope: "vol",
                    index: 3,
                    value: -129
                }
            ),
            "expected SequenceValueOutOfRange{{envelope: \"vol\", index: 3, value: -129}}, got {err:?}"
        );
    }

    #[test]
    fn accepts_sequence_value_boundaries() {
        let mut patch = sample_patch();
        patch.sequences.vol[0].values = vec![-128, 0, 127];
        let bytes = patch.to_bytes();
        assert!(
            Patch::from_bytes(&bytes).is_ok(),
            "values of -128 and 127 are within -128..=127 and must be accepted"
        );
    }

    #[test]
    fn rejects_sequence_too_long() {
        let mut patch = sample_patch();
        patch.sequences.vol[0].values = vec![0; MAX_SEQUENCE_LEN + 1];
        let bytes = patch.to_bytes();
        let err = Patch::from_bytes(&bytes).unwrap_err();
        assert!(
            matches!(
                err,
                PatchError::SequenceTooLong {
                    envelope: "vol",
                    index: 3,
                    len
                } if len == MAX_SEQUENCE_LEN + 1
            ),
            "expected SequenceTooLong{{envelope: \"vol\", index: 3, len: {}}}, got {err:?}",
            MAX_SEQUENCE_LEN + 1
        );
    }

    #[test]
    fn accepts_sequence_at_max_length() {
        let mut patch = sample_patch();
        patch.sequences.vol[0].values = vec![0; MAX_SEQUENCE_LEN];
        patch.sequences.vol[0].loop_point = None;
        let bytes = patch.to_bytes();
        assert!(
            Patch::from_bytes(&bytes).is_ok(),
            "a sequence with exactly MAX_SEQUENCE_LEN steps must be accepted"
        );
    }

    #[test]
    fn rejects_loop_point_out_of_range() {
        let mut patch = sample_patch();
        // sample_entry(3) has 3 values (indices 0..=2); a legal marker can be
        // at most 3 (== values.len()), so 4 is one past legal.
        patch.sequences.vol[0].loop_point = Some(4);
        let bytes = patch.to_bytes();
        let err = Patch::from_bytes(&bytes).unwrap_err();
        assert!(
            matches!(
                err,
                PatchError::MarkerOutOfRange {
                    envelope: "vol",
                    index: 3,
                    marker: "loop_point",
                    position: 4
                }
            ),
            "expected MarkerOutOfRange{{envelope: \"vol\", index: 3, marker: \"loop_point\", position: 4}}, got {err:?}"
        );
    }

    #[test]
    fn rejects_release_point_out_of_range() {
        let mut patch = sample_patch();
        patch.sequences.vol[0].release_point = Some(4);
        let bytes = patch.to_bytes();
        let err = Patch::from_bytes(&bytes).unwrap_err();
        assert!(
            matches!(
                err,
                PatchError::MarkerOutOfRange {
                    envelope: "vol",
                    index: 3,
                    marker: "release_point",
                    position: 4
                }
            ),
            "expected MarkerOutOfRange{{envelope: \"vol\", index: 3, marker: \"release_point\", position: 4}}, got {err:?}"
        );
    }

    #[test]
    fn accepts_marker_at_exactly_values_len() {
        // `== values.len()` must stay legal — the conservative rule is the
        // one that can never reject a file this build itself wrote (see
        // `sequence_to_text` rendering a marker at `len` in `gui/editor.rs`).
        let mut patch = sample_patch();
        patch.sequences.vol[0].values = vec![15, 12, 8];
        patch.sequences.vol[0].loop_point = Some(3);
        patch.sequences.vol[0].release_point = Some(3);
        let bytes = patch.to_bytes();
        assert!(
            Patch::from_bytes(&bytes).is_ok(),
            "a marker positioned exactly at values.len() must be accepted"
        );
    }

    #[test]
    fn from_shared_sequences_collects_only_used_slots() {
        // All 5 tabs populated, each with a distinct slot index and a
        // distinct values vector, so a transposition among any tab pair
        // (not just vol/duty) is detectable.
        let mut shared = SharedSequences::default();

        shared.set_selected_sequence_index(0, 5); // vol
        shared.selected_sequence_mut(0).1.values = vec![1, 2, 3];

        shared.set_selected_sequence_index(1, 9); // arp
        shared.selected_sequence_mut(1).1.values = vec![4, 5];

        shared.set_selected_sequence_index(2, 12); // pitch
        shared.selected_sequence_mut(2).1.values = vec![6, 7, 8, 9];

        shared.set_selected_sequence_index(3, 20); // hipitch
        shared.selected_sequence_mut(3).1.values = vec![10];

        shared.set_selected_sequence_index(4, 2); // duty
        shared.selected_sequence_mut(4).1.values = vec![11, 12];

        let patch = Patch::from_shared_sequences(&shared, 60);

        // Only the one populated slot per bank is collected out of the 128
        // available — proves the "used only" filtering still holds.
        assert_eq!(patch.sequences.vol.len(), 1);
        assert_eq!(patch.sequences.vol[0].index, 5);
        assert_eq!(patch.sequences.vol[0].values, vec![1, 2, 3]);

        assert_eq!(patch.sequences.arp.len(), 1);
        assert_eq!(patch.sequences.arp[0].index, 9);
        assert_eq!(patch.sequences.arp[0].values, vec![4, 5]);

        assert_eq!(patch.sequences.pitch.len(), 1);
        assert_eq!(patch.sequences.pitch[0].index, 12);
        assert_eq!(patch.sequences.pitch[0].values, vec![6, 7, 8, 9]);

        assert_eq!(patch.sequences.hipitch.len(), 1);
        assert_eq!(patch.sequences.hipitch[0].index, 20);
        assert_eq!(patch.sequences.hipitch[0].values, vec![10]);

        assert_eq!(patch.sequences.duty.len(), 1);
        assert_eq!(patch.sequences.duty[0].index, 2);
        assert_eq!(patch.sequences.duty[0].values, vec![11, 12]);

        assert_eq!(patch.active_indices.vol, 5);
        assert_eq!(patch.active_indices.arp, 9);
        assert_eq!(patch.active_indices.pitch, 12);
        assert_eq!(patch.active_indices.hipitch, 20);
        assert_eq!(patch.active_indices.duty, 2);

        assert_eq!(patch.step_time_hz, 60);
    }

    #[test]
    fn apply_to_shared_sequences_round_trips_all_tabs() {
        // All 5 tabs populated, each with a distinct slot index and distinct
        // values, so transposing any tab pair (including the two middle
        // tabs, pitch/hipitch) is detectable on both the save and restore
        // sides.
        let mut original = SharedSequences::default();

        original.set_selected_sequence_index(0, 4); // vol
        original.selected_sequence_mut(0).1.values = vec![15, 14, 12];
        *original.sequence_enabled_mut(0) = true;

        original.set_selected_sequence_index(1, 8); // arp
        original.selected_sequence_mut(1).1.values = vec![3, 6, 9];
        *original.sequence_enabled_mut(1) = true;

        original.set_selected_sequence_index(2, 15); // pitch
        {
            let (_text, seq) = original.selected_sequence_mut(2);
            seq.values = vec![1, -1, 2];
            seq.loop_point = Some(1);
            seq.release_point = Some(2);
            seq.pitch_mode = PitchMode::Absolute;
            seq.arp_mode = ArpMode::Relative;
            seq.vol_mode = VolMode::Steps64;
        }
        *original.sequence_enabled_mut(2) = false; // populated but disabled

        original.set_selected_sequence_index(3, 20); // hipitch
        original.selected_sequence_mut(3).1.values = vec![7, -3];
        *original.sequence_enabled_mut(3) = true;

        original.set_selected_sequence_index(4, 2); // duty
        original.selected_sequence_mut(4).1.values = vec![0, 1];
        *original.sequence_enabled_mut(4) = true;

        let patch = Patch::from_shared_sequences(&original, 60);

        // Full-entry equality for the pitch tab: proves every field
        // (loop_point, release_point, all three modes, and enabled) — not
        // just index/values — survives the save side.
        let expected_pitch_entry = PatchSequenceEntry {
            index: 15,
            values: vec![1, -1, 2],
            loop_point: Some(1),
            release_point: Some(2),
            pitch_mode: PitchMode::Absolute,
            arp_mode: ArpMode::Relative,
            vol_mode: VolMode::Steps64,
            enabled: false,
        };
        assert_eq!(patch.sequences.pitch, vec![expected_pitch_entry]);

        assert_eq!(patch.sequences.vol[0].index, 4);
        assert_eq!(patch.sequences.vol[0].values, vec![15, 14, 12]);
        assert_eq!(patch.sequences.arp[0].index, 8);
        assert_eq!(patch.sequences.arp[0].values, vec![3, 6, 9]);
        assert_eq!(patch.sequences.hipitch[0].index, 20);
        assert_eq!(patch.sequences.hipitch[0].values, vec![7, -3]);
        assert_eq!(patch.sequences.duty[0].index, 2);
        assert_eq!(patch.sequences.duty[0].values, vec![0, 1]);

        // Computed the same way production code computes it, so this isn't a
        // hand-typed magic string: "1 | -1 / 2" (loop marker before step 1,
        // release marker before step 2) is non-trivial proof that `slot.text`
        // is really regenerated from the loaded sequence.
        let expected_pitch_text = sequence_to_text(&Sequence {
            values: vec![1, -1, 2],
            loop_point: Some(1),
            release_point: Some(2),
            pitch_mode: PitchMode::Absolute,
            arp_mode: ArpMode::Relative,
            vol_mode: VolMode::Steps64,
        });

        let mut restored = SharedSequences::default();
        patch.apply_to_shared_sequences(&mut restored);

        assert_eq!(restored.selected_sequence_index(0), 4);
        assert_eq!(restored.selected_sequence(0).values, vec![15, 14, 12]);
        assert!(restored.sequence_enabled(0));

        assert_eq!(restored.selected_sequence_index(1), 8);
        assert_eq!(restored.selected_sequence(1).values, vec![3, 6, 9]);
        assert!(restored.sequence_enabled(1));

        assert_eq!(restored.selected_sequence_index(2), 15);
        assert_eq!(restored.selected_sequence(2).values, vec![1, -1, 2]);
        assert_eq!(restored.selected_sequence(2).loop_point, Some(1));
        assert_eq!(restored.selected_sequence(2).release_point, Some(2));
        assert_eq!(
            restored.selected_sequence(2).pitch_mode,
            PitchMode::Absolute
        );
        assert_eq!(restored.selected_sequence(2).arp_mode, ArpMode::Relative);
        assert_eq!(restored.selected_sequence(2).vol_mode, VolMode::Steps64);
        assert!(!restored.sequence_enabled(2));
        assert_eq!(restored.sequence_bank(2).slot(15).text, expected_pitch_text);

        assert_eq!(restored.selected_sequence_index(3), 20);
        assert_eq!(restored.selected_sequence(3).values, vec![7, -3]);
        assert!(restored.sequence_enabled(3));

        assert_eq!(restored.selected_sequence_index(4), 2);
        assert_eq!(restored.selected_sequence(4).values, vec![0, 1]);
        assert!(restored.sequence_enabled(4));
    }

    #[test]
    fn apply_to_shared_sequences_clears_slots_not_present_in_the_patch() {
        let mut shared = SharedSequences::default();
        shared.selected_sequence_mut(1).1.values.push(9); // arp slot 0
        *shared.sequence_enabled_mut(1) = true;

        let empty_patch = Patch::from_shared_sequences(&SharedSequences::default(), 60);
        empty_patch.apply_to_shared_sequences(&mut shared);

        assert!(shared.selected_sequence(1).is_empty());
        assert!(!shared.sequence_enabled(1));
    }

    #[test]
    fn save_and_load_round_trip_through_a_real_file() {
        let path = std::env::temp_dir().join(format!(
            "rp2a03patch_test_{}_{}.rp2a03patch",
            std::process::id(),
            line!()
        ));
        let patch = sample_patch();

        save_to_path(&path, &patch).expect("save must succeed");
        let restored = load_from_path(&path).expect("load must succeed");
        std::fs::remove_file(&path).expect("cleanup must succeed");

        assert_eq!(restored, patch);
    }

    #[test]
    fn load_from_path_reports_missing_file_as_io_error() {
        let path = std::env::temp_dir().join(format!(
            "rp2a03patch_test_missing_{}_{}.rp2a03patch",
            std::process::id(),
            line!()
        ));
        let err = load_from_path(&path).unwrap_err();
        assert!(matches!(err, PatchFileError::Io(_)));
        match err {
            PatchFileError::Io(inner) => assert_eq!(
                inner.kind(),
                std::io::ErrorKind::NotFound,
                "missing file must surface as a NotFound io::Error, not merely some Io variant"
            ),
            other => panic!("expected PatchFileError::Io, got {other:?}"),
        }
    }

    #[test]
    fn load_from_path_reports_invalid_contents_as_format_error() {
        let path = std::env::temp_dir().join(format!(
            "rp2a03patch_test_badformat_{}_{}.rp2a03patch",
            std::process::id(),
            line!()
        ));
        std::fs::write(&path, b"not a patch at all").expect("setup write must succeed");

        let err = load_from_path(&path).unwrap_err();
        std::fs::remove_file(&path).expect("cleanup must succeed");

        assert!(matches!(err, PatchFileError::Format(PatchError::BadMagic)));
    }

    #[test]
    fn save_to_path_rejects_an_invalid_patch_before_touching_the_filesystem() {
        let path = std::env::temp_dir().join(format!(
            "rp2a03patch_test_invalid_save_{}_{}.rp2a03patch",
            std::process::id(),
            line!()
        ));
        let mut patch = sample_patch();
        patch.step_time_hz = 0; // deliberately invalid

        let err = save_to_path(&path, &patch).unwrap_err();
        assert!(
            matches!(
                err,
                PatchFileError::Format(PatchError::StepTimeOutOfRange(0))
            ),
            "expected PatchFileError::Format(StepTimeOutOfRange(0)), got {err:?}"
        );
        assert!(
            !path.exists(),
            "save_to_path must validate before writing anything to disk"
        );
    }

    // Real output of `sample_patch().to_bytes()` on the final wire layout
    // (captured after every other change in this file, per the plan's
    // required ordering). If this test fails, the wire layout changed
    // incompatibly — i.e. some struct's field order (or an enum variant's
    // *name*) was reordered/renamed in a way that breaks existing
    // `.rp2a03patch` files. The fix is almost never to regenerate this
    // constant: regenerating it only hides the incompatibility from this
    // test, it does not undo the fact that files already on users' disks
    // would now decode into the wrong fields. Only regenerate it as a
    // deliberate, deserving-of-its-own-changelog-entry action alongside a
    // `format_version` bump and explicit migration handling.
    //
    // Regenerated when the top-level `waveform` field was removed (the
    // instrument's live channel selection is now derived from
    // `slot_waveforms[active_indices.vol]` — see `Patch::active_waveform` —
    // rather than stored redundantly). Unlike the `slot_waveforms` append this
    // constant was previously regenerated for, removing a field is exactly the
    // kind of change the paragraph above calls a `format_version` bump for;
    // it was skipped anyway because the project is unreleased, so there is no
    // real `.rp2a03patch` file on any disk this could orphan, and no
    // pre-removal golden bytes are kept around for the same reason.
    const GOLDEN_V1: &[u8] = &[
        82, 80, 50, 80, 149, 1, 60, 149, 3, 0, 0, 0, 3, 149, 145, 152, 3, 147, 15, 12, 8, 1, 192,
        168, 82, 101, 108, 97, 116, 105, 118, 101, 168, 65, 98, 115, 111, 108, 117, 116, 101, 167,
        83, 116, 101, 112, 115, 49, 54, 195, 144, 144, 144, 145, 152, 3, 147, 15, 12, 8, 1, 192,
        168, 82, 101, 108, 97, 116, 105, 118, 101, 168, 65, 98, 115, 111, 108, 117, 116, 101, 167,
        83, 116, 101, 112, 115, 49, 54, 195, 145, 146, 3, 167, 86, 114, 99, 54, 83, 97, 119,
    ];

    #[test]
    fn golden_v1_bytes_still_decode_to_the_same_patch() {
        assert_eq!(
            Patch::from_bytes(GOLDEN_V1).expect("golden v1 bytes must still decode"),
            sample_patch()
        );
    }

    #[test]
    fn golden_v1_bytes_are_still_exactly_what_the_encoder_writes() {
        // The decode direction above is what protects files already on disk.
        // This pins the encode direction, which nothing else does: a change
        // affecting only what *new* files look like — adding
        // `skip_serializing_if`, switching a struct to a map representation,
        // reordering how a field is written — would leave every other test in
        // this module green while silently splitting the format in two.
        assert_eq!(sample_patch().to_bytes(), GOLDEN_V1);
    }
}
