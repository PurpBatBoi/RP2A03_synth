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
/// Maps MIDI notes to the 16 available APU noise periods.
/// Higher notes = higher pitch (lower period).
pub fn noise_period_for_midi_note(note: u8) -> u8 {
    // In FamiTracker/FamiStudio, the 16 periods are mapped to consecutive notes.
    // We map note % 16 inverted, so higher note = higher pitch (lower period index).
    (15 - (note % 16)) as u8
}
