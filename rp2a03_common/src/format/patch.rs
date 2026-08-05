//! rp2a03_common\src\format\patch.rs
//! `.rp2a03patch` native preset file format: encode/decode/validate for one
//! instrument's envelope/sequence data, active-slot selection, waveform, and
//! engine speed, plus conversion to/from the plugin's live `SharedSequences`.
//!
//! Format spec: Ideas-ref-folder\MARKDOWNs\Save&Load\Rp2a03_patch_format.md
//! (outside version control — see that file's own header for why).

use crate::ChannelMode;
use rp2a03_core::sequencer::{ArpMode, PitchMode, VolMode};
use std::fmt;

/// Fixed file-identity prefix, never bumped — see "Wire format" in the spec.
pub const PATCH_MAGIC: [u8; 4] = *b"RP2P";
pub const CURRENT_FORMAT_VERSION: u32 = 1;

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

/// Failure decoding or validating a `.rp2a03patch` byte payload.
#[derive(Debug)]
pub enum PatchError {
    /// The file doesn't start with the `RP2P` magic bytes.
    BadMagic,
    /// The MessagePack payload after the magic bytes is malformed.
    Decode(rmp_serde::decode::Error),
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => write!(f, "not a .rp2a03patch file (missing RP2P magic bytes)"),
            Self::Decode(e) => write!(f, "malformed .rp2a03patch payload: {e}"),
        }
    }
}

impl std::error::Error for PatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            Self::BadMagic => None,
        }
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
        Ok(patch)
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
}
