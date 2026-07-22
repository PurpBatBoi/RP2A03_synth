# RP2A03-SYNTH: Quality Guide for AI Coding Agents

This guide defines the expected way to investigate, change, and verify code in
this workspace. It applies to feature work, bug fixes, refactors, and reviews.
Follow the intent as well as the literal rules: make the smallest correct change
that preserves the project's audio, MIDI, and public behaviour.

## Non-negotiable rules

1. **Fix causes, not symptoms.** Do not delete behaviour, comment out code,
   add empty stubs, weaken types, or swallow errors merely to make a build pass.
2. **Respect scope.** Change only files needed for the task. Preserve unrelated
   user changes and never modify `Ideas-ref-folder/`; it is read-only reference
   material.
3. **Treat the audio path as real-time code.** It must not allocate, lock, block,
   perform I/O, log noisily, or use unbounded work in a per-sample/per-block path
   unless the existing architecture explicitly provides a safe mechanism.
4. **Verify every claim.** Read the local implementation, manifest, tests, and
   resolved dependency source when relevant. Do not invent API signatures,
   timing behaviour, or hardware rules from memory.
5. **Finish with evidence.** Format changed Rust code and run the narrowest
   relevant checks. State what ran and any check that could not run.

## Project map and ownership

| Area | Responsibility | Change carefully because |
|---|---|---|
| `rp2a03_core/` | NES APU emulation, sequencing, resampling, LFOs | Timing and register semantics directly affect sound. |
| `rp2a03_niceplug/` | CLAP/VST wrapper, parameters, audio processing, MIDI | This is the host-facing and real-time boundary. |
| `rp2a03_ui/` | Egui editor, widgets, UI state | UI and processor state must stay coherent. |
| `xtask/` | Workspace tooling | It may encode packaging or development workflows. |
| `Ideas-ref-folder/` | External/legacy reference implementations | Read and adapt ideas; never edit or copy blindly. |

The workspace currently contains Rust 2021 and Rust 2024 crates. Keep the
edition and style of the crate being edited; do not perform edition-wide changes
as incidental cleanup.

## Required workflow

### 1. Understand before editing

- Read the target code and its immediate callers, tests, and manifest.
- Check `git status` before making changes. Treat existing modifications as user
  work unless the task clearly owns them.
- Write down the observable behaviour, invariant, or failure being changed.
- When using reference code, identify the original algorithm or constant and
  translate it to the project's Rust design rather than porting it mechanically.

### 2. Design the smallest safe change

- Prefer a localized, explicit fix over a broad rewrite.
- Preserve public APIs, saved parameter/state behaviour, MIDI mappings, and
  audio output unless their change is part of the request.
- Make invalid states difficult to represent. Use domain types, ranges, and
  clear ownership where they genuinely clarify an invariant.
- Do not add dependencies, `unsafe`, background threads, or synchronization to
  solve a local problem without a demonstrated need and a scoped design.

### 3. Implement deliberately

- Keep functions focused and names specific to the audio/MIDI domain.
- Explain *why* for hardware workarounds, timing decisions, non-obvious maths,
  and safety invariants; do not restate obvious code.
- Avoid speculative refactors and unrelated formatting churn.
- Use `Result`/`Option` where failure is meaningful. Do not use `unwrap()` or
  `expect()` in production paths unless a documented invariant makes failure
  impossible; test code may use them for concise assertions.
- If `unsafe` is necessary, minimize its scope and document the precise safety
  invariant at the block.

### 4. Update the whole contract

After changing a signature, parameter, enum, serialization format, or behaviour:

1. Search all call sites, tests, examples, UI bindings, and state handling.
2. Update every affected consumer instead of making an adapter that hides a
   broken contract.
3. Add or adjust tests for the intended behaviour and boundary conditions.
4. Check that the host-facing parameter and UI behaviour still agree.

## Dependency and framework compatibility

`nice-plug` and `nice-plug-egui` are Git dependencies. APIs and their internal
`egui` version are determined by the revisions resolved in `Cargo.lock`, not by
online examples or the newest crates.io release.

Before changing framework/UI code:

1. Inspect the workspace manifests and lockfile.
2. Inspect the resolved dependency source when a signature, trait, or type is
   uncertain.
3. Keep shared framework types on compatible versions. Two versions of a type
   such as `egui::Context` are distinct Rust types even if their names match.
4. Prefer the dependency's local source and existing project usage over guessed
   methods or copied snippets.

For compiler errors, investigate in this order:

| Symptom | First checks |
|---|---|
| Unresolved import | Manifest dependency, feature flags, actual module path. |
| Same-looking types mismatch | Duplicate versions in the resolved dependency graph. |
| Wrong argument count or trait signature | Definition in local source and every call site/implementation. |
| Method missing | Version-specific API and equivalent local usage. |
| Trait bound failure | Type/version mismatch, feature requirement, then ownership/lifetime constraints. |

## Real-time audio and NES APU rules

### Audio-thread discipline

- Keep `process`-path work bounded and predictable.
- Reuse buffers and precompute lookup data outside the process path where
  possible. Do not introduce per-sample allocation or string formatting.
- Avoid mutexes, file/network I/O, sleeping, blocking channels, and waiting for
  UI work in audio processing.
- Consider denormals, integer range conversions, and parameter smoothing where
  they affect audible output. Do not add smoothing to discrete hardware-style
  controls without an explicit behaviour decision.

### Preserve hardware semantics

- The APU timer runs at CPU-clock timing; software LFOs and sequences have their
  own update cadence. Do not conflate those rates.
- APU volume is 4-bit (`0..=15`); clamp/quantize at the register boundary.
- `$4003` / `write_timer_hi` resets the pulse duty phase. Write it only when the
  high timer bits change or on a deliberate note-on attack. Repeated writes can
  cause audible buzzing.
- `$4000` control writes and `$4002` timer-low writes have different side effects
  from `$4003`; preserve those distinctions when reorganizing register updates.
- Validate constants, tables, and sequence semantics against an authoritative
  reference before changing them.

### MIDI, state, and UI

- Preserve MIDI ordering, note-priority, note-off, and controller behaviour
  unless the task explicitly changes them.
- Keep persisted parameter/state formats backward-compatible when practical. If
  a breaking migration is required, implement and test it deliberately.
- Make UI edits flow through the same parameter/state model as host automation;
  do not create a second, unsynchronized source of truth.
- For CC mappings, retain the expected range, quantization, and musical meaning.
  In particular, discrete APU volume is not interchangeable with continuous
  output gain.

## Testing and verification

Choose checks based on the changed surface, starting narrow and expanding when
the change crosses crate boundaries:

| Changed area | Minimum verification |
|---|---|
| `rp2a03_core` | `cargo test -p rp2a03_core` |
| `rp2a03_niceplug` | `cargo check -p rp2a03_niceplug` |
| `rp2a03_ui` | `cargo check -p rp2a03_ui` |
| Shared API, manifest, or cross-crate code | Relevant checks above, then `cargo check --workspace` when feasible |
| Rust source | `cargo fmt --check` (or run `cargo fmt` before finalizing) |

Add tests when behaviour changes or a bug can regress. Good cases include:

- minimum/maximum register and controller values;
- empty, one-step, looped, and released sequences;
- note-on/note-off ordering and repeated events;
- timer boundaries and phase-reset conditions;
- state/parameter round trips where persistence is involved.

Run Clippy when it is available and proportionate to the change; fix relevant
warnings rather than suppressing them. Any lint suppression needs a specific
reason and the narrowest scope. Never claim that audio sounds correct solely
because code compiles--use available tests, targeted reasoning, and manual host
verification when the task requires audible validation.

## Completion checklist

Before handing work back, confirm:

- [ ] The change addresses the stated root cause and preserves unrelated behaviour.
- [ ] All affected call sites, tests, UI/state paths, and docs are updated.
- [ ] No reference files, generated artifacts, secrets, or unrelated files changed.
- [ ] Audio-thread changes have no new blocking, allocation, locking, or I/O risk.
- [ ] Dependency APIs and versions were verified locally where applicable.
- [ ] Formatting and relevant build/test checks were run, with outcomes reported.
- [ ] The final summary names the behavioural change, modified files, and any
      remaining limitation or unrun check.
