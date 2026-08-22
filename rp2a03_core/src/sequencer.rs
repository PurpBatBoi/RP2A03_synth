//! `rp2a03_core\src\sequencer.rs`
//!
//! FamiTracker-style sequence engine for volume, arpeggio, pitch, hi-pitch, and duty cycle envelopes.
//!
//! Each `Sequence` holds a vector of signed 16-bit step values advanced at 60 Hz.
//! Supports Loop markers `|`) and Release markers `/`).
//! `SequencePlayer` tracks playback position, looping, and release tail state.
//!
//!
//! Steps are advanced once per 60 Hz frame tick.
//! - Loop marker `|`: defines step index where sequence loops back while key is held.
//! - Release marker `/`: defines step index where playback jumps when key is released `NoteOff`).

/// Whether a pitch/hi-pitch sequence's step values accumulate or set the offset outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum PitchMode {
    /// Each step adds to the running pitch offset.
    #[default]
    Relative = 0,
    /// Each step replaces the pitch offset outright.
    Absolute = 1,
}

/// Arpeggio sequence mode — mirrors `seq_setting_t` in Dn-FamiTracker `Sequence.h`.
/// (Scheme mode is intentionally omitted.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum ArpMode {
    /// `SETTING_ARP_ABSOLUTE` (0): each step is a signed semitone offset from the
    /// channel base note.  `SetPeriod(TriggerNote(BaseNote + Value))`.
    #[default]
    Absolute = 0,
    /// `SETTING_ARP_RELATIVE` (2): each step permanently shifts the channel's base
    /// note accumulating.  `SetNote(BaseNote + Value); SetPeriod(TriggerNote(BaseNote))`.
    Relative = 2,
}

/// Volume sequence resolution — mirrors `SETTING_VOL_16_STEPS` / `SETTING_VOL_64_STEPS`
/// in Dn-FamiTracker `Sequence.h`.  Only meaningful for the VRC6 sawtooth, whose
/// `$B000` accumulator rate is 6-bit; every other channel uses the 4-bit APU range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum VolMode {
    /// `SETTING_VOL_16_STEPS` (0): steps are 0..=15 and the saw register byte is
    /// `(vol << 1) | ((duty & 1) << 5)` — duty acts as the rate MSB.
    #[default]
    Steps16 = 0,
    /// `SETTING_VOL_64_STEPS` (1): steps are 0..=63 and are written straight to
    /// `$B000`; the duty sequence is ignored (dn `CSeqInstHandlerSawtooth::IsDutyIgnored`).
    Steps64 = 1,
}

/// S5B duty/mode sequence volume resolution — parallel to [`VolMode`] but scoped
/// to `ChannelMode::S5B`. Not a reuse of `VolMode`: the rescale math there is
/// hardcoded to the 16<->64 relationship for VRC6's 6-bit accumulator, whereas
/// the S5B's hardware volume register is 5-bit (0..=31), a 16<->32 relationship.
/// A deliberate addition beyond stock `DnFamiTracker`, which has no S5B
/// equivalent of `SETTING_VOL_64_STEPS`. The two modes differ in curve as well
/// as in resolution — see `MidiHandler::apply_s5b_modulation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum VolMode5B {
    /// Steps are 0..=15 and map *linearly* onto the chip's logarithmic volume
    /// ladder, so the lane tracks loudness the way the 2A03 pulse's linear
    /// 4-bit DAC does. Diverges from the chip, which packs a 4-bit volume as
    /// `(val << 1) | 1` and inherits the ladder's ~3 dB-per-step spacing.
    #[default]
    Steps16 = 0,
    /// Steps are 0..=31 and index the volume ladder directly: the chip's true
    /// resolution, ~1.5 dB per step, including the 16 levels the 4-bit
    /// register cannot address.
    Steps32 = 1,
}

/// Bit flags packed into an S5B duty/mode sequence step value, mirroring dn's
/// `S5B_MODE_ENVELOPE`/`S5B_MODE_SQUARE`/`S5B_MODE_NOISE` (`Sequence.h`).
/// Bits 0-4 (`0x1F`) hold the noise period (0..=31); these three flag bits sit
/// above it in the same `i16` step value.
pub const S5B_MODE_ENVELOPE: i16 = 0x20;
/// Set when the S5B step should enable the tone (square) generator.
pub const S5B_MODE_SQUARE: i16 = 0x40;
/// Set when the S5B step should enable the noise generator.
pub const S5B_MODE_NOISE: i16 = 0x80;
/// Mask for the noise-period bits of an S5B duty/mode step value.
pub const S5B_PERIOD_MASK: i16 = 0x1F;

/// Shift/mask for the tone duty-width bits of an S5B duty/mode step value
/// (bits 8-11), holding a signed offset from AY8930 duty preset index 4
/// (50%, stock behavior) so bit pattern 0 reads as "unset = 50%" for
/// sequences authored before this field existed. Valid decoded index range
/// is 0..=8 (3.125%..96.875%), i.e. offset -4..=4.
pub const S5B_DUTY_SHIFT: i16 = 8;
/// Mask for the duty-width bits of an S5B duty/mode step value.
pub const S5B_DUTY_MASK: i16 = 0x0F << S5B_DUTY_SHIFT;
/// Duty preset index that offset 0 (bit pattern 0) decodes to.
pub const S5B_DUTY_DEFAULT_INDEX: i16 = 4;

/// Decodes the duty preset index (0..=8) packed into an S5B step value.
#[must_use]
pub fn s5b_duty_index(value: i16) -> i16 {
    let raw = (value & S5B_DUTY_MASK) >> S5B_DUTY_SHIFT;
    // Sign-extend the 4-bit field (two's complement) before adding the
    // default offset, so e.g. 0xF (-1) still decodes to index 3, not 19.
    let offset = (raw << 12) >> 12;
    S5B_DUTY_DEFAULT_INDEX + offset
}

/// Packs a duty preset index (0..=8) into an S5B step value, preserving all
/// other bits.
#[must_use]
pub fn s5b_set_duty_index(value: i16, index: i16) -> i16 {
    let offset = index.clamp(0, 8) - S5B_DUTY_DEFAULT_INDEX;
    (value & !S5B_DUTY_MASK) | ((offset << S5B_DUTY_SHIFT) & S5B_DUTY_MASK)
}

/// One FamiTracker-style envelope: a list of step values plus optional
/// loop/release markers, authored per lane and per numbered sequence slot.
#[derive(Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sequence {
    /// Step values (signed to support bipolar pitch/arpeggio/hi-pitch offsets).
    ///
    /// Dn-FamiTracker stores sequence items as `signed char`, so every envelope type
    /// (including hi-pitch) can hold -128..=127.
    pub values: Vec<i16>,
    /// Optional loop point index `|`).
    pub loop_point: Option<usize>,
    /// Optional release point index `/`).
    pub release_point: Option<usize>,
    /// Pitch mode for pitch sequences (Relative vs Absolute).
    pub pitch_mode: PitchMode,
    /// Arpeggio mode — only meaningful when this sequence is used as an arpeggio
    /// envelope.  Ignored for all other sequence types.
    pub arp_mode: ArpMode,
    /// Volume resolution — only meaningful when this sequence is used as a volume
    /// envelope on the VRC6 sawtooth.  Ignored for all other sequence types.
    ///
    /// `#[serde(default)]` keeps saved states written before this field existed
    /// loadable; they deserialize as [`VolMode::Steps16`].
    #[serde(default)]
    pub vol_mode: VolMode,
    /// Volume resolution for the S5B duty/mode sequence — only meaningful when
    /// this sequence is used as a volume envelope on `ChannelMode::S5B`.
    /// Ignored for all other sequence types.
    ///
    /// `#[serde(default)]` keeps saved states written before this field existed
    /// loadable; they deserialize as [`VolMode5B::Steps16`].
    #[serde(default)]
    pub vol_mode_5b: VolMode5B,
}

impl Clone for Sequence {
    fn clone(&self) -> Self {
        Self {
            values: self.values.clone(),
            loop_point: self.loop_point,
            release_point: self.release_point,
            pitch_mode: self.pitch_mode,
            arp_mode: self.arp_mode,
            vol_mode: self.vol_mode,
            vol_mode_5b: self.vol_mode_5b,
        }
    }

    // Reuses `values`'s existing allocation instead of allocating a fresh Vec —
    // called on the audio thread each time the active sequence data is refreshed,
    // where a fresh allocation would be a real-time-safety violation.
    fn clone_from(&mut self, source: &Self) {
        self.values.clone_from(&source.values);
        self.loop_point = source.loop_point;
        self.release_point = source.release_point;
        self.pitch_mode = source.pitch_mode;
        self.arp_mode = source.arp_mode;
        self.vol_mode = source.vol_mode;
        self.vol_mode_5b = source.vol_mode_5b;
    }
}

impl Sequence {
    /// Create a new sequence with a single step value.
    #[must_use]
    pub fn single(value: i16) -> Self {
        Self {
            values: vec![value],
            ..Default::default()
        }
    }

    /// Parse a FamiTracker-style sequence string into a Sequence.
    ///
    /// Example: `"6 8 9 10 | 11 12 12 12 11 9 8 8 9 / 9 10 12 11 11 10"`
    /// - Numbers: step values (signed integers).
    /// - `|` or `L`: marks loop start step index.
    /// - `/` or `R`: marks release start step index.
    #[must_use]
    pub fn parse(input: &str) -> Self {
        let (seq, _) = Self::parse_clamped(input, i16::MIN, i16::MAX);
        seq
    }

    /// Parse a sequence string and clamp all numeric tokens to `[min_val, max_val]`.
    /// Returns the parsed `Sequence` and a normalized/clamped text string.
    #[must_use]
    pub fn parse_clamped(input: &str, min_val: i16, max_val: i16) -> (Self, String) {
        let mut values = Vec::new();
        let mut loop_point = None;
        let mut release_point = None;
        let mut text_tokens = Vec::new();

        for token in input.split_whitespace() {
            match token {
                "|" | "L" | "l" => {
                    loop_point = Some(values.len());
                    text_tokens.push("|".to_string());
                }
                "/" | "R" | "r" => {
                    release_point = Some(values.len());
                    text_tokens.push("/".to_string());
                }
                _ => {
                    if let Ok(v) = token.parse::<i16>() {
                        let clamped = v.clamp(min_val, max_val);
                        values.push(clamped);
                        text_tokens.push(clamped.to_string());
                    }
                }
            }
        }

        if values.is_empty() {
            values.push(0.clamp(min_val, max_val));
            text_tokens.push("0".to_string());
        }

        let normalized_text = text_tokens.join(" ");
        let sequence = Self {
            values,
            loop_point,
            release_point,
            ..Default::default()
        };

        (sequence, normalized_text)
    }

    /// Get total number of steps in sequence.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Check if sequence is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Get value at a given step index, clamped to valid bounds.
    #[must_use]
    pub fn get(&self, index: usize) -> i16 {
        if self.values.is_empty() {
            return 0;
        }
        self.values[index.min(self.values.len() - 1)]
    }
}

/// Playback state for a sequence, mirroring dnFamiTracker's `CSeqInstHandler` states
/// (`SEQ_STATE_RUNNING` / `SEQ_STATE_END` / `SEQ_STATE_HALT` / `SEQ_STATE_DISABLED`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqState {
    /// Sequence is not active / disabled (dn: `SEQ_STATE_DISABLED`).
    Disabled,
    /// Sequence is running and advancing (dn: `SEQ_STATE_RUNNING`). This also covers
    /// dn's "waiting for release" condition, which stays in the RUNNING state with the
    /// pointer frozen on the release-point step and re-processes that step every tick.
    Running,
    /// Sequence has fully played out (dn: `SEQ_STATE_END` collapsing into `SEQ_STATE_HALT`
    /// on the next tick). The last processed value holds, but no further step values are
    /// processed — important for accumulating (relative) pitch/hi-pitch sequences.
    End,
}

/// Plays back a `Sequence`, advancing one step per 60 Hz tick.
///
/// The state machine and pointer arithmetic are kept 1:1 with dnFamiTracker's
/// `CSeqInstHandler::UpdateInstrument()` so envelope timing matches exactly:
///
/// - After processing step `pos` the pointer advances; the boundary check fires when
///   the pointer reaches `release_point + 1` **or** the end of the sequence. This means
///   the step *at* the release point is processed once while the key is still held
///   (dn: `m_iSeqPointer[i] == (Release + 1)`), before looping or waiting.
/// - A loop is only honored while not releasing (or when no release point exists) and
///   only if `loop_point < release_point`.
/// - At end-of-sequence, a loop placed inside/after the release tail keeps looping;
///   otherwise the player moves to `End` and processes nothing further.
/// - "Waiting for release" (release boundary reached, key held, no valid loop) freezes
///   the pointer at `release_point` and keeps re-reading that step every tick while
///   remaining in `Running`. In dn this causes relative pitch/hi-pitch steps to keep
///   accumulating during the wait, which this model reproduces.
#[derive(Debug, Clone)]
pub struct SequencePlayer {
    /// Current step position.
    pub pos: usize,
    /// Playback state.
    pub state: SeqState,
    /// Active gate / note-held state. When `false`, key is held; when `true`, key is released.
    pub is_releasing: bool,
    /// The current output value from the sequence.
    pub current_value: i16,
}

impl Default for SequencePlayer {
    fn default() -> Self {
        Self {
            pos: 0,
            state: SeqState::Disabled,
            is_releasing: false,
            current_value: 0,
        }
    }
}

impl SequencePlayer {
    /// Create a new sequence player.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Trigger the sequence (called on `NoteOn`). Reads step 0 immediately
    /// into `current_value` and advances position to step 1, matching dnFamiTracker
    /// triggering the instrument and processing sequence step 0 within the same
    /// engine frame (`TriggerInstrument` -> `UpdateInstrument`).
    pub fn trigger(&mut self, seq: &Sequence) {
        if seq.is_empty() {
            self.state = SeqState::Disabled;
            self.pos = 0;
            self.current_value = 0;
            return;
        }
        self.pos = 0;
        self.state = SeqState::Running;
        self.is_releasing = false;
        self.clock_tick(seq);
    }

    /// Release the sequence (called on `NoteOff`).
    ///
    /// 1:1 with dn's `CSeqInstHandler::ReleaseInstrument()`: the release flag is always
    /// set, but the pointer only jumps when the sequence is still `Running` or already
    /// `End` (never when `Disabled`), and only with a valid release point. The release
    /// step value is *not* read here — dn applies it on the next engine tick via
    /// `UpdateInstrument`, so this does not call `clock_tick`.
    pub fn release(&mut self, seq: &Sequence) {
        self.is_releasing = true;
        if matches!(self.state, SeqState::Running | SeqState::End)
            && let Some(rel) = seq.release_point
            && rel < seq.len()
        {
            self.pos = rel;
            self.state = SeqState::Running;
        }
    }

    /// Advance the sequence by one 60 Hz tick. Returns the current step value.
    ///
    /// Port of dnFamiTracker `CSeqInstHandler::UpdateInstrument()` step-advance logic.
    /// Steps are only processed while `Running`; `End` and `Disabled` hold their value
    /// and process nothing (returns the held value / 0, matching dn's END/HALT handling
    /// where no sequence item is applied anymore).
    pub fn clock_tick(&mut self, seq: &Sequence) -> i16 {
        if self.state == SeqState::Disabled || seq.is_empty() {
            return 0;
        }
        if self.state == SeqState::End {
            return self.current_value;
        }

        let seq_len = seq.len();

        self.current_value = seq.get(self.pos);
        self.pos += 1;

        // dn uses -1 sentinels for "no loop/release point"; translate with signed math
        // so the comparisons below read exactly like CSeqInstHandler::UpdateInstrument.
        let release = seq.release_point.map_or(-1, |r| r as isize);
        let loop_pt = seq.loop_point.map_or(-1, |l| l as isize);
        let items = seq_len as isize;
        let pos = self.pos as isize;

        if pos == release + 1 || pos >= items {
            // End point reached
            if loop_pt != -1 && !(self.is_releasing && release != -1) && loop_pt < release {
                // Standard loop before release (Loop < Release), key still held
                // (or ignored once releasing with a release point set)
                self.pos = loop_pt as usize;
            } else if pos >= items {
                // End of sequence
                if loop_pt >= release && loop_pt != -1 {
                    // Loop point is in/after the release tail: keep looping forever
                    self.pos = (loop_pt as usize).min(seq_len.saturating_sub(1));
                } else {
                    self.state = SeqState::End;
                }
            } else if !self.is_releasing {
                // Waiting for release (dn: `--m_iSeqPointer[i]`): stay Running with the
                // pointer frozen on the release-point step, re-reading it every tick.
                self.pos -= 1;
            }
        }

        self.current_value
    }

    /// Point the player at step 0 of a freshly loaded sequence and mark it running.
    ///
    /// 1:1 with dn's `CSeqInstHandler::SetupSequence()`, which is what runs when
    /// `LoadInstrument` hands the handler an envelope it was not already playing.
    /// Unlike [`trigger`](Self::trigger) this does *not* read step 0 — dn applies
    /// the new step on the following `UpdateInstrument` tick — and it leaves
    /// `is_releasing` alone, so a sequence swapped in during a release tail stays
    /// in its release state.
    pub fn setup(&mut self) {
        self.pos = 0;
        self.state = SeqState::Running;
    }

    /// Get current output value without advancing.
    #[must_use]
    pub fn value(&self) -> i16 {
        self.current_value
    }

    /// Reset player to disabled state.
    pub fn reset(&mut self) {
        self.pos = 0;
        self.state = SeqState::Disabled;
        self.is_releasing = false;
        self.current_value = 0;
    }
}
