//! rp2a03_core\src\vrc6_common.rs
//!
//! Shared building blocks for the VRC6 sound expansion channels
//! (pulse 1, pulse 2, saw). See vrc6_pulse.rs and vrc6_saw.rs.

// ─────────────────────────────────────────────
// Frequency Divider
// ─────────────────────────────────────────────

/// VRC6's free-running divider, clocked once per CPU cycle (there's no
/// "every other cycle" halving like the 2A03 timer). The reload value is
/// `(frequency >> frequency_shift) + 1`, recomputed by the caller on every
/// tick rather than cached, matching the original hardware/Mesen behavior.
#[derive(Debug, Clone)]
pub struct Divider {
    counter: i32,
}

impl Divider {
    pub fn new() -> Self {
        Self { counter: 1 }
    }

    /// Ticks the divider down by one. Returns true (and reloads) when it
    /// reaches zero.
    pub fn tick(&mut self, reload: i32) -> bool {
        self.counter -= 1;
        if self.counter == 0 {
            self.counter = reload;
            true
        } else {
            false
        }
    }
}