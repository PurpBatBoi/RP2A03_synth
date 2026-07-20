use rp2a03_core::nes_core::NTSC_CPU_CLOCK;

/// Converts a MIDI note (0-127) to a FamiStudio-compatible frequency in Hz.
/// Standard MIDI to frequency (A4 = 440 Hz).
pub fn midi_note_to_freq(note: u8) -> f64 {
    // FamiStudio's C4 sounds like a standard MIDI C5 (Note 72).
    // To match this in Reaper (where C4 = Note 60), we shift the note up by 12 semitones.
    let shifted_note = note as f64 + 12.0;
    440.0 * 2f64.powf((shifted_note - 69.0) / 12.0)
}

/// Computes the Square and Triangle period matching FamiStudio's internal table.
/// FamiStudio calculates both using: (CPU_CLOCK / (16.0 * freq)) - 0.5.
/// This causes Triangle to sound one octave lower than Square for the same note, matching FamiStudio exactly.
pub fn period_for_frequency(freq_hz: f64) -> u16 {
    let p = (NTSC_CPU_CLOCK / (16.0 * freq_hz)) - 0.5;
    p.round().clamp(0.0, 0x7FF as f64) as u16
}

/// Computes the Noise period index (0-15) for a given MIDI note, mimicking FamiStudio.
///
/// FamiStudio's noise mapping (descending, with +12 octave correction applied):
///   C0 → 0xE,  C#0 → 0xD,  D0 → 0xC,  D#0 → 0xB,
///   E0 → 0xA,  F0  → 0x9,  F#0 → 0x8,  G0  → 0x7,
///   G#0 → 0x6, A0  → 0x5,  A#0 → 0x4,  B0  → 0x3,
///   C1 → 0x2,  C#1 → 0x1,  D1  → 0x0,  D#1 → 0xF
pub fn noise_period_for_midi_note(note: u8) -> u8 {
    // FamiStudio D1 = period 0, and periods descend as notes go lower.
    // After applying the +12 octave correction (Reaper→FamiStudio),
    // FamiStudio's D1 is at internal note 38, so:
    //   period = (38 - (note + 12)) % 16 = (26 - note) % 16
    ((26 - note as i16).rem_euclid(16)) as u8
}
