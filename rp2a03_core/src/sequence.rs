//! rp2a03_core\src\sequence.rs
//! FamiTracker-style sequence engine for volume, arpeggio, pitch, hi-pitch, and duty cycle envelopes.
//!
//! Each `Sequence` holds a vector of signed 16-bit step values advanced at 60 Hz.
//! Supports Loop markers (`|`) and Release markers (`/`).
//! `SequencePlayer` tracks playback position, looping, and release tail state.

/// A FamiTracker-style sequence of step values.
///
/// Steps are advanced once per 60 Hz frame tick.
/// - Loop marker `|`: defines step index where sequence loops back while key is held.
/// - Release marker `/`: defines step index where playback jumps when key is released (`NoteOff`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sequence {
    /// Step values (signed to support bipolar pitch/arpeggio/hi-pitch offsets).
    pub values: Vec<i16>,
    /// Optional loop point index (`|`).
    pub loop_point: Option<usize>,
    /// Optional release point index (`/`).
    pub release_point: Option<usize>,
}

impl Default for Sequence {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            loop_point: None,
            release_point: None,
        }
    }
}

impl Sequence {
    /// Create a new sequence with a single step value.
    pub fn single(value: i16) -> Self {
        Self {
            values: vec![value],
            loop_point: None,
            release_point: None,
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

/// Playback state for a sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqState {
    /// Sequence is not active / disabled.
    Disabled,
    /// Sequence is running and advancing.
    Running,
    /// Sequence has reached its end and holds the last value.
    Held,
}

/// Plays back a `Sequence`, advancing one step per 60 Hz tick.
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

    /// Trigger the sequence (called on NoteOn). Resets position to 0
    /// and reads the first value immediately.
    pub fn trigger(&mut self, seq: &Sequence) {
        self.pos = 0;
        self.state = SeqState::Running;
        self.is_releasing = false;
        self.current_value = seq.get(0);
    }

    /// Release the sequence (called on NoteOff).
    /// If the sequence has a release point (`/`), jumps `pos` to that release step.
    pub fn release(&mut self, seq: &Sequence) {
        self.is_releasing = true;
        if let Some(rel) = seq.release_point {
            if rel < seq.len() {
                self.pos = rel;
                self.state = SeqState::Running;
                self.current_value = seq.get(rel);
            }
        }
    }

    /// Advance the sequence by one 60 Hz tick. Returns the current step value.
    pub fn clock_tick(&mut self, seq: &Sequence) -> i16 {
        if self.state == SeqState::Disabled || seq.is_empty() {
            return 0;
        }

        let seq_len = seq.len();

        match self.state {
            SeqState::Running => {
                // Read current step value
                self.current_value = seq.get(self.pos);

                // Advance pointer
                self.pos += 1;

                let loop_pt = seq.loop_point;
                let rel_pt = seq.release_point;

                let hit_release_boundary = rel_pt.map_or(false, |r| self.pos >= r);
                let hit_seq_end = self.pos >= seq_len;

                if hit_release_boundary || hit_seq_end {
                    if let (Some(l), Some(r)) = (loop_pt, rel_pt) {
                        if l < r {
                            // Standard loop before release (Loop < Release)
                            if !self.is_releasing {
                                // While key is held, loop back to loop_pt at release boundary
                                self.pos = l;
                            } else if hit_seq_end {
                                // After release tail reaches end of sequence, hold on last step
                                self.pos = seq_len.saturating_sub(1);
                                self.state = SeqState::Held;
                            }
                        } else {
                            // Loop point is in/after release tail (Loop >= Release)
                            if hit_seq_end {
                                self.pos = l.min(seq_len.saturating_sub(1));
                            } else if !self.is_releasing {
                                // Waiting before release
                                self.pos = self.pos.saturating_sub(1);
                                self.state = SeqState::Held;
                            }
                        }
                    } else if hit_seq_end {
                        // No release point, or loop only
                        if let Some(l) = loop_pt {
                            self.pos = l.min(seq_len.saturating_sub(1));
                        } else {
                            self.pos = seq_len.saturating_sub(1);
                            self.state = SeqState::Held;
                        }
                    } else if !self.is_releasing && rel_pt.is_some() {
                        // Reached release boundary without loop: hold at release - 1
                        self.pos = self.pos.saturating_sub(1);
                        self.state = SeqState::Held;
                    }
                }

                self.current_value
            }
            SeqState::Held => {
                self.current_value
            }
            SeqState::Disabled => 0,
        }
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
        let (seq, _) = Sequence::parse_clamped("0 4 7 12 -12 | 0 -4 -7 / -12", -96, 96);
        assert_eq!(seq.values, vec![0, 4, 7, 12, -12, 0, -4, -7, -12]);
        assert_eq!(seq.loop_point, Some(5));
        assert_eq!(seq.release_point, Some(8));
    }
}
