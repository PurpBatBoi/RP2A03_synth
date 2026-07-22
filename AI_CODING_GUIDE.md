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

---

# Rust Best Practices

Comprehensive Rust coding guidelines covering ownership, error handling, async patterns, traits, testing, performance, clippy, and documentation.

## Core Rust Directives
1. **Memory Safety & Ownership:** Adhere strictly to Rust's ownership, borrowing, and lifetime rules. Do not use `.clone()` indiscriminately; favor references (`&T`) or passing ownership when appropriate.
2. **Error Handling:** Avoid `.unwrap()` and `.expect()` in production code. Use proper `Result` and `Option` types, and implement custom error types using `thiserror` or `anyhow`.
3. **Idiomatic Rust (Clean Code):** Write idiomatic code. Use iterators and functional patterns instead of imperative loops where applicable. Keep functions small and modular. 
4. **Concurrency:** Utilize `std::sync`, `std::thread`, or the `tokio` asynchronous runtime correctly, avoiding deadlocks and data races.
5. **Testing:** Provide robust unit and integration tests. Mark test modules with `#[cfg(test)]`.
6. **Documentation:** Document all public APIs using Rustdoc, including examples where helpful.

---

## When to Apply

- Writing new Rust code or designing APIs
- Reviewing or refactoring existing Rust code
- Implementing async systems with Tokio
- Designing error hierarchies with `thiserror`/`anyhow`
- Choosing between borrowing, cloning, or ownership transfer
- Setting up tests, benchmarks, or snapshot testing
- Configuring clippy lints and workspace settings
- Optimizing Rust code for performance

---

## Quick Reference: Coding Style

- Prefer `&T` over `.clone()` unless ownership transfer is required
- Use `&str` over `String`, `&[T]` over `Vec<T>` in function parameters
- No `get_` prefix on getters: `fn name()` not `fn get_name()`
- Conversion naming: `as_` (cheap borrow), `to_` (expensive/cloning), `into_` (ownership transfer)
- Iterator methods: `iter()` / `iter_mut()` / `into_iter()`
- Import ordering: `std` -> external crates -> workspace crates -> `super::` -> `crate::`
- Comments explain *why* (safety, workarounds), not *what*
- Use `format!` over string concatenation with `+`
- Prefer `s.bytes()` over `s.chars()` for ASCII-only operations
- Avoid macros unless necessary; prefer functions or generics

---

## Quick Reference: Error Handling

- Return `Result<T, E>` for fallible operations; reserve `panic!` for unrecoverable bugs
- **No `unwrap()` in production.** Use `expect()` with descriptive message only when the value is logically guaranteed. Prefer `?`, `if let`, `let...else` for all other cases
- Use `thiserror` for library/crate errors, `anyhow` for binaries only
- Prefer `?` operator over `match` chains for error propagation
- Use `_else` variants (`ok_or_else`, `unwrap_or_else`) to prevent eager allocation
- Use `inspect_err` and `map_err` for logging and transforming errors
- `assert!` at function entry for invariant checking (debug builds)

---

## Quick Reference: Ownership & Pointers

- Small `Copy` types (<=24 bytes, all fields `Copy`, no heap) pass by value
- Use `Cow<'_, T>` when data may or may not need ownership
- Meaningful lifetime names: `'src`, `'ctx`, `'conn` — not just `'a`
- Use `try_borrow()` on `RefCell` to avoid panics; prefer over direct `.borrow_mut()`
- Shadowing for transformations: `let x = x.parse()?`

| Pointer | When to Use |
|---------|-------------|
| `Box<T>` | Single ownership, heap allocation, recursive types |
| `Rc<T>` | Shared ownership, single-threaded |
| `Arc<T>` | Shared ownership, multi-threaded |
| `Cell<T>` / `RefCell<T>` | Interior mutability, single-threaded |
| `Mutex<T>` / `RwLock<T>` | Interior mutability, multi-threaded |

---

## Quick Reference: Traits & Generics

- Prefer generics (static dispatch) by default for zero-cost abstractions
- Use `dyn Trait` only when heterogeneous collections or plugin architectures are needed
- Box at API boundaries, not internally
- Object safety: no generic methods, no `Self: Sized`, methods use `&self`/`&mut self`/`self`
- Use sealed traits to prevent external implementors
- Type state pattern encodes valid states in the type system:

```rust
struct Connection<S> { _state: PhantomData<S> }
struct Disconnected;
struct Connected;
impl Connection<Connected> {
    fn send(&self, data: &[u8]) { /* ... */ }
}
```

---

## Quick Reference: Async & Concurrency

- Async for I/O-bound work, sync for CPU-bound work
- Never hold locks across `.await` points — use scoped guards
- Never use `std::thread::sleep` in async — use `tokio::time::sleep`
- Never spawn unboundedly — use semaphores for limits
- Ensure `Send` bounds on spawned futures
- Use `JoinSet` for managing multiple concurrent tasks
- Use `CancellationToken` (from `tokio_util`) for graceful shutdown
- Instrument with `tracing` + `#[instrument]` for async debugging

| Channel | Use Case |
|---------|----------|
| `mpsc` | Multi-producer, single-consumer message passing |
| `broadcast` | Multi-producer, multi-consumer event fan-out |
| `oneshot` | Single value, single use (request-response) |
| `watch` | Latest-value-only, change notification |

- Sync channels: `crossbeam::channel` over `std::sync::mpsc`
- Async channels: `tokio::sync::{mpsc, broadcast, oneshot, watch}`
- Atomics (`AtomicBool`, `AtomicUsize`) over `Mutex` for primitive types
- Choose memory ordering carefully: `Relaxed` / `Acquire` / `Release` / `SeqCst`

---

## Quick Reference: Testing

- Name tests descriptively: `process_should_return_error_when_input_empty()`
- One assertion per test when possible; include formatted failure messages
- Group tests in `mod` blocks by unit of work
- Use doc tests (`///`) for public API examples; run separately with `cargo test --doc`
- Snapshot testing: `cargo insta test` then `cargo insta review`; redact unstable fields
- `rstest` for parameterized tests with `#[case::name]` labels
- `proptest` for property-based testing with custom strategies
- `mockall` with `#[automock]` for mocking traits
- `criterion` for benchmarks with `iter_batched` and `BenchmarkId`
- `cargo-fuzz` with `libfuzzer_sys` for fuzz testing
- `cargo-tarpaulin` or `cargo-llvm-cov` for code coverage
- Use `#[should_panic]` and `#[ignore]` attributes where appropriate

---

## Quick Reference: Performance

- Golden rule: don't guess, measure. Always benchmark with `--release`
- Run `cargo clippy -- -D clippy::perf` for performance-related hints
- Use `cargo flamegraph` or `samply` (macOS) for profiling
- Avoid cloning in loops; clone at the last moment only
- Pre-allocate: `Vec::with_capacity()`, `String::with_capacity()`
- Prefer iterators over manual `for` loops; avoid intermediate `.collect()`
- Stack for small types, heap for large/recursive; use `smallvec` for large const arrays
- Use `Cow<'_, T>` to avoid unnecessary allocation
- Prefer `s.bytes()` over `s.chars()` for ASCII-only string operations

---

## Quick Reference: Clippy & Linting

Run regularly:

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
```

| Lint | Catches |
|------|---------|
| `redundant_clone` | Unnecessary `.clone()` calls |
| `needless_borrow` | Unnecessary `&` borrows |
| `large_enum_variant` | Oversized variants (consider `Box`) |
| `needless_collect` | Premature `.collect()` before iteration |
| `map_unwrap_or` | `.map().unwrap_or()` chains |
| `unnecessary_wraps` | Functions always returning `Ok`/`Some` |
| `clone_on_copy` | `.clone()` on `Copy` types |

- Use `#[expect(clippy::lint)]` over `#[allow(...)]` — `expect` warns when lint no longer applies
- Add justification comment on every suppression
- Set `#![warn(clippy::all)]` as workspace minimum

---

## Quick Reference: Documentation

- `//` comments explain *why*: safety invariants, workarounds, design rationale
- `///` doc comments explain *what* and *how* for all public items
- `//!` for module-level and crate-level documentation at top of `lib.rs`/`mod.rs`
- Every `TODO` needs a linked issue: `// TODO(#42): description`
- Enable `#![deny(missing_docs)]` for libraries
- Include `# Examples`, `# Errors`, `# Panics`, `# Safety` sections in doc comments

---

## Quick Reference: Data Types & Patterns

- Use newtypes for domain semantics: `struct Email(String)`
- Prefer slice patterns: `if let [first, .., last] = slice`
- Use arrays for fixed sizes; avoid `Vec` when length is known at compile time
- Shadowing for transformation: `let x = x.parse()?`
- `Cow<str>` when data might need modification of borrowed data

---

## Deprecated to Modern Migration

| Deprecated | Better | Since |
|------------|--------|-------|
| `lazy_static!` | `std::sync::OnceLock` | Rust 1.70 |
| `once_cell::Lazy` | `std::sync::LazyLock` | Rust 1.80 |
| `std::sync::mpsc` | `crossbeam::channel` (sync) | — |
| `std::sync::Mutex` | `parking_lot::Mutex` (recommended) | — |
| `failure` / `error-chain` | `thiserror` / `anyhow` | — |
| `try!()` | `?` operator | Rust 2018 |

---

## Constraints

### MUST DO

1. Use ownership and borrowing for memory safety
2. Handle all errors explicitly via `Result`/`Option` — no silent failures
3. Use `thiserror` for library errors, `anyhow` for binaries
4. Minimize `unsafe` code; document all `unsafe` blocks with safety invariants
5. Use the type system for compile-time guarantees
6. Run `cargo clippy` and fix all warnings
7. Use `cargo fmt` for consistent formatting
8. Write tests including doc tests for public APIs
9. Add `///` documentation with examples for all public items

### MUST NOT DO

1. Use `unwrap()` in production code
2. Create memory leaks or dangling pointers
3. Use `unsafe` without documented safety invariants
4. Ignore clippy warnings without `#[expect(...)]` and justification
5. Hold locks across `.await` points
6. Use `std::thread::sleep` in async context
7. Skip error handling or use `panic!` for recoverable errors
8. Use `String` where `&str` suffices; clone unnecessarily
