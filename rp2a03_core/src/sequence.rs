//! FamiTracker-style sequence engine for volume and duty cycle envelopes.
//!
//! Each `Sequence` holds an array of step values that are advanced at 60 Hz.
//! Supports Loop markers (`|`) and Release markers (`/`).
//! `SequencePlayer` tracks playback position, looping, and release tail state.

/// A FamiTracker-style sequence of step values.
///
/// Steps are advanced once per 60 Hz frame tick.
/// - Loop marker `|`: defines step index where sequence loops back while key is held.
/// - Release marker `/`: defines step index where playback jumps when key is released (`NoteOff`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sequence {
    /// Step values (e.g. 0..15 for volume, 0..3 for duty).
    pub values: Vec<u8>,
    /// Optional loop point index (`|`).
    pub loop_point: Option<usize>,
    /// Optional release point index (`/`).
    pub release_point: Option<usize>,
}

impl Default for Sequence {
    fn default() -> Self {
        Self {
            values: vec![0],
            loop_point: None,
            release_point: None,
        }
    }
}

impl Sequence {
    /// Create a new sequence with a single step value.
    pub fn single(value: u8) -> Self {
        Self {
            values: vec![value],
            loop_point: None,
            release_point: None,
        }
    }

    /// Parse a FamiTracker-style sequence string.
    ///
    /// Example: `"6 8 9 10 | 11 12 12 12 11 9 8 8 9 / 9 10 12 11 11 10"`
    /// - Numbers: step values.
    /// - `|` or `L`: marks loop start step index.
    /// - `/` or `R`: marks release start step index.
    pub fn parse(input: &str) -> Self {
        let mut values = Vec::new();
        let mut loop_point = None;
        let mut release_point = None;

        for token in input.split_whitespace() {
            match token {
                "|" | "L" | "l" => {
                    loop_point = Some(values.len());
                }
                "/" | "R" | "r" => {
                    release_point = Some(values.len());
                }
                _ => {
                    if let Ok(v) = token.parse::<u8>() {
                        values.push(v);
                    }
                }
            }
        }

        if values.is_empty() {
            values.push(0);
        }

        Self {
            values,
            loop_point,
            release_point,
        }
    }

    /// Get total number of steps in sequence.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Check if sequence is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Get value at a given step index, clamped to valid range.
    pub fn get(&self, index: usize) -> u8 {
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
    /// Active gate / note-held state. When `false`, key is released.
    pub is_releasing: bool,
    /// The current output value from the sequence.
    pub current_value: u8,
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
    pub fn clock_tick(&mut self, seq: &Sequence) -> u8 {
        if self.state == SeqState::Disabled || seq.is_empty() {
            return 0;
        }

        let seq_len = seq.len();

        match self.state {
            SeqState::Running => {
                // Read current step value
                self.current_value = seq.get(self.pos);

                // Determine next position
                let next_pos = self.pos + 1;

                if !self.is_releasing {
                    // Key is held
                    if let Some(rel) = seq.release_point {
                        if next_pos >= rel {
                            // Hit release boundary while key is held
                            if let Some(loop_pt) = seq.loop_point {
                                if loop_pt < rel {
                                    // Loop back between loop_point and release_point
                                    self.pos = loop_pt;
                                    return self.current_value;
                                }
                            }
                            // No loop before release: hold at release - 1
                            self.pos = rel.saturating_sub(1);
                            self.state = SeqState::Held;
                            return self.current_value;
                        }
                    }
                }

                if next_pos >= seq_len {
                    // Reached end of sequence
                    if let Some(loop_pt) = seq.loop_point {
                        // Loop back to loop point
                        self.pos = loop_pt.min(seq_len.saturating_sub(1));
                    } else {
                        // Hold on last step
                        self.pos = seq_len.saturating_sub(1);
                        self.state = SeqState::Held;
                    }
                } else {
                    self.pos = next_pos;
                }

                self.current_value
            }
            SeqState::Held => {
                // Return held value
                self.current_value
            }
            SeqState::Disabled => 0,
        }
    }

    /// Get current output value without advancing.
    pub fn value(&self) -> u8 {
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
    fn test_loop_and_release_playback() {
        let seq = Sequence::parse("10 12 | 15 14 / 8 4 0");
        let mut player = SequencePlayer::new();

        // Trigger note (key held)
        player.trigger(&seq);
        assert_eq!(player.current_value, 10);

        assert_eq!(player.clock_tick(&seq), 10); // pos=0 -> 1
        assert_eq!(player.clock_tick(&seq), 12); // pos=1 -> hits release boundary at pos=2, loops to 2 (|)
        assert_eq!(player.clock_tick(&seq), 15); // pos=2 -> 3
        assert_eq!(player.clock_tick(&seq), 14); // pos=3 -> hits release boundary pos=4, loops to 2
        assert_eq!(player.clock_tick(&seq), 15); // pos=2 -> 3

        // Release key -> jumps to release point (pos=4)
        player.release(&seq);
        assert_eq!(player.current_value, 8); // pos=4 value

        assert_eq!(player.clock_tick(&seq), 8); // pos=4 -> 5
        assert_eq!(player.clock_tick(&seq), 4); // pos=5 -> 6
        assert_eq!(player.clock_tick(&seq), 0); // pos=6 -> end, holds 0
        assert_eq!(player.state, SeqState::Held);
    }

    #[test]
    fn test_no_loop_holds_at_release_boundary() {
        let seq = Sequence::parse("15 12 / 6 0");
        let mut player = SequencePlayer::new();

        player.trigger(&seq);
        player.clock_tick(&seq); // 15
        player.clock_tick(&seq); // 12 -> holds at 12 (pos=1) while key held
        assert_eq!(player.current_value, 12);
        assert_eq!(player.state, SeqState::Held);

        // Key release
        player.release(&seq);
        assert_eq!(player.current_value, 6);
        player.clock_tick(&seq); // 6
        player.clock_tick(&seq); // 0
    }
}
