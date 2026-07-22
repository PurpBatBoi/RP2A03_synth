# RP2A03-SYNTH Codebase Guide for AI Models

This document defines how AI models should approach writing code, fixing bugs, and resolving errors in the RP2A03-SYNTH project. These are hard rules, not suggestions.

## Project Architecture

```
RP2A03-SYNTH/
├── rp2a03_core/         # Pure NES APU emulation library (no plugin deps)
│   └── src/
│       ├── apu_pulse.rs     # Pulse channel hardware emulation
│       ├── apu_timer.rs     # 11-bit hardware timer (CPU clock rate)
│       ├── apu_envelope.rs  # 4-bit volume envelope
│       ├── lfo.rs           # Software LFO engine (FamiTracker vibrato/tremolo)
│       ├── sequence.rs      # FamiTracker sequence engine (volume/duty envelopes)
│       └── blip_buf.rs      # Band-limited audio resampling
├── rp2a03_niceplug/     # VST/CLAP plugin wrapper using nice-plug framework
│   └── src/
│       ├── lib.rs           # Plugin struct, params, egui editor, audio loop
│       └── midi.rs          # MIDI event handling, CC mapping, modulation
└── Ideas-ref-folder/    # Reference code (read-only, never modify)
    ├── dn-famitracker-source/   # DN-FamiTracker C++ source
    ├── famistudio-code/         # FamiStudio C# source
    ├── sflt-source/             # SFLT plugin source
    └── rp2a_old/gui-code/       # Old egui GUI reference code
```

## Core Rules

### 1. Never "Cheat" to Fix Errors

**BAD**: Removing functionality, commenting out broken code, stubbing methods with empty bodies, or weakening types to make the compiler happy.

**GOOD**: Understanding WHY the error occurs and fixing the root cause.

Examples of cheating vs proper fixes:

```rust
// BAD: Removing a function argument to fix "expected 4 args, found 3"
fn note_off(&mut self, note: u8, pulse: &mut Pulse) { ... }

// GOOD: Adding the missing arguments that the updated signature requires
fn note_off(&mut self, note: u8, pulse: &mut Pulse, vol_seq: &Sequence, duty_seq: &Sequence) { ... }
```

```rust
// BAD: Ignoring a trait method's required signature
fn editor(&self, ...) -> ... { }  // Compiler says &mut self

// GOOD: Matching the trait's exact signature
fn editor(&mut self, ...) -> ... { }
```

### 2. Check Dependency Versions Before Using APIs

This project uses `nice-plug` and `nice-plug-egui` from a git source. The egui version used internally by `nice-plug-egui` may differ from crates.io latest.

**Before writing egui code:**
1. Check `nice-plug-egui/Cargo.toml` to find the exact `egui` version it depends on.
2. Use that EXACT version in `rp2a03_niceplug/Cargo.toml`.
3. Never add a different version—it creates two incompatible copies of `egui` in the dependency tree, causing "expected X, found X" type mismatch errors.

**When an API method signature doesn't match:**
1. Read the actual source code of the dependency (usually in `~/.cargo/registry/src/` or `~/.cargo/git/checkouts/`).
2. Check the method's current signature in the version being used.
3. Do NOT guess at parameter types or counts.

### 3. Understand Hardware vs Software Boundaries

| Layer | What It Does | Clock Rate | Where |
|-------|-------------|-----------|-------|
| APU Timer | Drives square wave frequency | ~1.79 MHz (CPU) | `apu_timer.rs` |
| APU Envelope | 4-bit volume decay | Frame counter | `apu_envelope.rs` |
| Software LFO | Vibrato/Tremolo modulation | 60 Hz (software) | `lfo.rs` |
| Sequences | Volume/Duty envelopes | 60 Hz (software) | `sequence.rs` |
| MIDI Handler | CC mapping, note priority | Per audio block | `midi.rs` |

**Critical hardware behavior:**
- Writing to `$4003` (`write_timer_hi`) resets the 8-step duty cycle phase. This MUST be cached—only write when the high 3 bits actually change or on NoteOn attack. Continuous writes cause buzzing artifacts.
- The APU volume is 4-bit (0..15). All volume calculations must clamp to this range before writing to the control register.

### 4. How to Fix Compilation Errors

Follow this decision tree:

1. **"unresolved import"**: Check if the crate is in `Cargo.toml`. Check the exact module path in the dependency source code.
2. **"mismatched types" with "multiple versions of crate X"**: You have two versions of the same dependency. Find which version the framework uses and match it exactly.
3. **"expected N arguments, found M"**: Read the actual function signature in the source. Don't guess—navigate to the definition. Add/update all call sites.
4. **"method not found"**: The method may have been renamed between versions. Search the dependency source for the equivalent method name.
5. **"trait bound not satisfied"**: Often caused by version mismatches. Ensure all crates agree on the same version of shared dependencies.

### 5. How to Update Call Sites After Changing a Function Signature

When you change a function's parameters (e.g., adding `vol_seq: &Sequence` to `note_off`):

1. Update the function definition.
2. Search ALL call sites: `grep -rn "note_off"` across the entire project.
3. Update every call site, including unit tests.
4. Do NOT skip tests—they are call sites too.

### 6. Reference Code Usage

The `Ideas-ref-folder/` contains reference implementations. Use them to understand algorithms and data structures, but:

- **Never modify** files in `Ideas-ref-folder/`.
- **Do adapt** algorithms to Rust idioms (e.g., DN-FamiTracker's C++ sequence parsing → Rust `Sequence::parse()`).
- **Do verify** lookup tables and constants against the original source (e.g., `FT_VIBRATO_TABLE` from `vibrato.s`).

### 7. NES APU Register Write Rules

```rust
// $4000 (write_ctrl): Duty + Volume. Safe to write every frame.
pulse.write_ctrl((duty << 6) | 0x30 | volume);

// $4001 (write_sweep): Set once to 0x08 (disable sweep). Rarely changes.
pulse.write_sweep(0x08);

// $4002 (write_timer_lo): Lower 8 bits of period. Safe to write freely.
pulse.write_timer_lo(period_lo);

// $4003 (write_timer_hi): Upper 3 bits of period. RESETS PHASE.
// ONLY write when high bits change or on NoteOn attack.
if new_hi_bits != cached_hi_bits {
    pulse.write_timer_hi(0xF8 | new_hi_bits);
    cached_hi_bits = new_hi_bits;
}
```

### 8. MIDI CC Mapping Reference

| CC | Name | Maps To | Range |
|----|------|---------|-------|
| 01 | Mod Wheel | Vibrato Depth | 0..15 (value >> 3) |
| 02 | Breath | Vibrato Speed | 0..63 (value >> 1) |
| 03 | Undefined | Tremolo Depth | 0..15 (value >> 3) |
| 04 | Foot | Tremolo Speed | 0..63 (value >> 1) |
| 07 | Volume | APU 15-step volume | Scales discrete 0..15 |
| 11 | Expression | Plugin gain multiplier | 0.0..1.0 continuous |
| 14 | Undefined | Fine Pitch | -64..+63 offset |

### 9. Sequence Format

FamiTracker-style text sequences:
```
6 8 9 10 | 11 12 12 12 11 9 8 8 9 / 9 10 12 11 11 10
```

- Numbers are step values (volume: 0..15, duty: 0..3).
- `|` marks the **Loop** point (step index where looping begins while key is held).
- `/` marks the **Release** point (step index where playback jumps on NoteOff).
- Length is determined by the number of numeric tokens.
- Loop and Release markers go BETWEEN numbers, not replacing them.

### 10. Testing Requirements

- Always run `cargo test -p rp2a03_core` after modifying core emulation code.
- Always run `cargo check -p rp2a03_niceplug` after modifying plugin code.
- When adding new features, add unit tests in the same file under `#[cfg(test)] mod tests`.
- Test edge cases: zero values, maximum values, empty sequences, single-step sequences.
