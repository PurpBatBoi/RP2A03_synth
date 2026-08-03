//! rp2a03_core\src\sequencer.rs
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum PitchMode {
    #[default]
    Relative = 0,
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

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
}

impl Default for Sequence {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            loop_point: None,
            release_point: None,
            pitch_mode: PitchMode::default(),
            arp_mode: ArpMode::default(),
            vol_mode: VolMode::default(),
        }
    }
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
    }
}

impl Sequence {
    /// Create a new sequence with a single step value.
    pub fn single(value: i16) -> Self {
        Self {
            values: vec![value],
            loop_point: None,
            release_point: None,
            pitch_mode: PitchMode::default(),
            arp_mode: ArpMode::default(),
            vol_mode: VolMode::default(),
        }
    }

    /// Parse a FamiTracker-style sequence string into a Sequence.
    ///
    /// Example: `"6 8 9 10 | 11 12 12 12 11 9 8 8 9 / 9 10 12 11 11 10"`
    /// - Numbers: step values (signed integers).
    /// - `|` or `L`: marks loop start step index.
    /// - `/` or `R`: marks release start step index.
    pub fn parse(input: &str) -> Self {
        let (seq, _) = Self::parse_clamped(input, i16::MIN, i16::MAX);
        seq
    }

    /// Parse a sequence string and clamp all numeric tokens to `[min_val, max_val]`.
    /// Returns the parsed `Sequence` and a normalized/clamped text string.
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
            pitch_mode: PitchMode::default(),
            arp_mode: ArpMode::default(),
            vol_mode: VolMode::default(),
        };

        (sequence, normalized_text)
    }

    /// Get total number of steps in sequence.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Check if sequence is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Get value at a given step index, clamped to valid bounds.
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
    pub fn new() -> Self {
        Self::default()
    }

    /// Trigger the sequence (called on NoteOn). Reads step 0 immediately
    /// into current_value and advances position to step 1, matching dnFamiTracker
    /// triggering the instrument and processing sequence step 0 within the same
    /// engine frame (TriggerInstrument -> UpdateInstrument).
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

    /// Release the sequence (called on NoteOff).
    ///
    /// 1:1 with dn's `CSeqInstHandler::ReleaseInstrument()`: the release flag is always
    /// set, but the pointer only jumps when the sequence is still `Running` or already
    /// `End` (never when `Disabled`), and only with a valid release point. The release
    /// step value is *not* read here — dn applies it on the next engine tick via
    /// `UpdateInstrument`, so this does not call `clock_tick`.
    pub fn release(&mut self, seq: &Sequence) {
        self.is_releasing = true;
        if matches!(self.state, SeqState::Running | SeqState::End) {
            if let Some(rel) = seq.release_point {
                if rel < seq.len() {
                    self.pos = rel;
                    self.state = SeqState::Running;
                }
            }
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
        let release = seq.release_point.map(|r| r as isize).unwrap_or(-1);
        let loop_pt = seq.loop_point.map(|l| l as isize).unwrap_or(-1);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_famitracker_string() {
        let seq = Sequence::parse("6 8 9 10 | 11 12 12 12 11 9 8 8 9 / 9 10 12 11 11 10");
        assert_eq!(seq.len(), 19);
        assert_eq!(seq.loop_point, Some(4));
        assert_eq!(seq.release_point, Some(13));
        assert_eq!(seq.values[0], 6);
        assert_eq!(seq.values[4], 11);
        assert_eq!(seq.values[13], 9);
    }

    #[test]
    fn test_parse_clamped_values() {
        let (seq, text) = Sequence::parse_clamped("0 5 32 -50 10", 0, 15);
        assert_eq!(seq.values, vec![0, 5, 15, 0, 10]);
        assert_eq!(text, "0 5 15 0 10");
    }

    #[test]
    fn test_signed_bipolar_values() {
        let (seq, _) = Sequence::parse_clamped("0 4 7 12 -12 | 0 -4 -7 / -12", -128, 127);
        assert_eq!(seq.values, vec![0, 4, 7, 12, -12, 0, -4, -7, -12]);
        assert_eq!(seq.loop_point, Some(5));
        assert_eq!(seq.release_point, Some(8));
    }

    #[test]
    fn test_pitch_mode_defaults() {
        let seq = Sequence::default();
        assert_eq!(seq.pitch_mode, PitchMode::Relative);
    }

    // ── SequencePlayer dn-parity tests ──
    // Expected step values below are traced from dnFamiTracker
    // CSeqInstHandler::UpdateInstrument() / ReleaseInstrument().

    #[test]
    fn sequence_without_vol_mode_deserializes_as_16_steps() {
        // Saved states written before `vol_mode` existed must still load; dn's own
        // default for a volume sequence is SETTING_VOL_16_STEPS.
        let json = r#"{
            "values": [15, 7, 0],
            "loop_point": null,
            "release_point": null,
            "pitch_mode": "Relative",
            "arp_mode": "Absolute"
        }"#;

        let seq: Sequence = serde_json::from_str(json).expect("legacy state must deserialize");
        assert_eq!(seq.vol_mode, VolMode::Steps16);
        assert_eq!(seq.values, vec![15, 7, 0]);
    }

    #[test]
    fn sequence_vol_mode_round_trips() {
        let mut seq = Sequence::parse("63 32");
        seq.vol_mode = VolMode::Steps64;

        let json = serde_json::to_string(&seq).unwrap();
        let restored: Sequence = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, seq);
    }

    #[test]
    fn player_no_markers_ends_and_holds_last_value() {
        let seq = Sequence::parse("5 7");
        let mut player = SequencePlayer::new();

        player.trigger(&seq); // reads step 0 immediately
        assert_eq!(player.value(), 5);
        assert_eq!(player.state, SeqState::Running);

        assert_eq!(player.clock_tick(&seq), 7); // step 1; pointer passes end -> End
        assert_eq!(player.state, SeqState::End);

        // dn END/HALT: value holds but no further step processing happens
        assert_eq!(player.clock_tick(&seq), 7);
        assert_eq!(player.clock_tick(&seq), 7);
        assert_eq!(player.state, SeqState::End);
    }

    #[test]
    fn player_loop_only_repeats_from_loop_point() {
        let seq = Sequence::parse("1 2 | 3");
        let mut player = SequencePlayer::new();

        player.trigger(&seq);
        for expected in [2, 3, 3, 3, 3] {
            assert_eq!(player.clock_tick(&seq), expected);
            assert_eq!(player.state, SeqState::Running);
        }
    }

    #[test]
    fn player_loop_includes_release_point_step_before_release() {
        // dn: loopback check is pointer == release + 1, so the release-point step (value 4)
        // is processed once per loop pass while the key is held.
        let seq = Sequence::parse("1 | 2 3 / 4 5"); // loop = 1, release = 3
        let mut player = SequencePlayer::new();

        player.trigger(&seq);
        let expected = [2, 3, 4, 2, 3, 4, 2, 3];
        for e in expected {
            assert_eq!(player.clock_tick(&seq), e);
        }

        // 1:1 with dn ReleaseInstrument: pointer jumps now, value is processed next tick
        player.release(&seq);
        assert_eq!(
            player.value(),
            3,
            "release() must not read the release step early"
        );
        assert_eq!(player.state, SeqState::Running);

        // Release tail: reads step 3 again, then step 4, then ends
        assert_eq!(player.clock_tick(&seq), 4);
        assert_eq!(player.state, SeqState::Running);
        assert_eq!(player.clock_tick(&seq), 5);
        assert_eq!(player.state, SeqState::End);
        assert_eq!(player.clock_tick(&seq), 5);
    }

    #[test]
    fn player_waiting_for_release_rereads_release_step_every_tick() {
        // release = 2 with one more step after it, so the wait branch (pos == release + 1
        // while pos < items) actually engages instead of falling into end-of-sequence.
        let seq = Sequence::parse("1 2 / 3 4");
        let mut player = SequencePlayer::new();

        player.trigger(&seq); // value 1
        assert_eq!(player.clock_tick(&seq), 2); // step 1
        // Boundary: pointer reached release + 1 while key held, no loop -> freeze at step 2
        assert_eq!(player.clock_tick(&seq), 3);
        assert_eq!(player.state, SeqState::Running);
        // dn re-processes the release-point step every tick during the wait (matters for
        // accumulating relative pitch/hi-pitch sequences)
        assert_eq!(player.clock_tick(&seq), 3);
        assert_eq!(player.clock_tick(&seq), 3);
        assert_eq!(player.state, SeqState::Running);

        player.release(&seq);
        // Release tail: release step processed again on the next tick, then the final step
        assert_eq!(player.clock_tick(&seq), 3);
        assert_eq!(player.state, SeqState::Running);
        assert_eq!(player.clock_tick(&seq), 4);
        assert_eq!(player.state, SeqState::End);
        assert_eq!(player.clock_tick(&seq), 4);
    }

    #[test]
    fn player_release_without_release_point_keeps_looping() {
        let seq = Sequence::parse("1 | 2 3");
        let mut player = SequencePlayer::new();

        player.trigger(&seq);
        player.release(&seq); // no release point: only the flag changes
        assert!(player.is_releasing);
        assert_eq!(player.state, SeqState::Running);

        // dn: with no release point the loop condition ignores the release flag
        for expected in [2, 3, 2, 3] {
            assert_eq!(player.clock_tick(&seq), expected);
            assert_eq!(player.state, SeqState::Running);
        }
    }

    #[test]
    fn player_release_does_nothing_when_disabled() {
        let seq = Sequence::parse("1 / 2 3");
        let mut player = SequencePlayer::new();

        // Never triggered: dn ReleaseInstrument skips sequences that are not RUNNING/END
        player.release(&seq);
        assert_eq!(player.state, SeqState::Disabled);
        assert_eq!(player.clock_tick(&seq), 0);
    }

    #[test]
    fn player_loop_at_or_after_release_point_loops_through_end() {
        // Loop inside the release tail (loop = 3 >= release = 1): dn's pre-release loop
        // branch requires loop < release, so this freezes at the release step while held;
        // after release the tail plays out and then keeps looping from the loop point.
        let seq = Sequence::parse("8 / 7 6 | 5 4"); // values [8,7,6,5,4], release = 1, loop = 3
        let mut player = SequencePlayer::new();

        player.trigger(&seq); // value 8, pos = 1 (release + 1 = 2 not reached yet)
        assert_eq!(player.clock_tick(&seq), 7); // step 1; pos = 2 == release + 1
        // loop branch fails (3 < 1 is false), pos < items (2 < 5), key held -> wait on step 1
        assert_eq!(player.state, SeqState::Running);
        assert_eq!(player.clock_tick(&seq), 7); // frozen wait re-reads the release step

        player.release(&seq);
        assert_eq!(player.clock_tick(&seq), 7); // release step processed on next tick
        assert_eq!(player.clock_tick(&seq), 6); // step 2
        assert_eq!(player.clock_tick(&seq), 5); // step 3
        // step 4 is the last item; pos = 5 >= items and loop 3 >= release 1 -> jump to 3
        assert_eq!(player.clock_tick(&seq), 4);
        assert_eq!(player.state, SeqState::Running);
        assert_eq!(player.clock_tick(&seq), 5); // loops forever from the loop point
        assert_eq!(player.clock_tick(&seq), 4);
        assert_eq!(player.state, SeqState::Running);
    }

    #[test]
    fn player_end_can_be_released_into_release_tail() {
        // dn ReleaseInstrument accepts sequences in SEQ_STATE_END and restarts them at the
        // release point (only reachable when reaching the end, e.g. release point on the
        // last step with no loop: pos == release + 1 == items -> End).
        let seq = Sequence::parse("1 2 3 / 5"); // values [1,2,3,5], release = 3
        let mut player = SequencePlayer::new();

        player.trigger(&seq);
        assert_eq!(player.clock_tick(&seq), 2);
        assert_eq!(player.clock_tick(&seq), 3);
        // After step 3 (value 5): pos = 4 == release + 1 == items -> no loop -> End
        assert_eq!(player.clock_tick(&seq), 5);
        assert_eq!(player.state, SeqState::End);

        // From End, a release re-enters at the release point and plays it once more
        player.release(&seq);
        assert_eq!(player.state, SeqState::Running);
        assert_eq!(
            player.value(),
            5,
            "release() must not read the release step early"
        );
        assert_eq!(player.clock_tick(&seq), 5);
        assert_eq!(player.state, SeqState::End);
    }
}
