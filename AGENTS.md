# AGENTS.md

Guidance for coding agents working in this repository.

## What this is

RP2A03 Synth — a WiP NES synthesizer plugin (CLAP + VST3) for DAWs. It's a
register-level emulation of the NES RP2A03 APU (pulse ×2, triangle, noise)
plus Konami VRC6 expansion audio, driven by MIDI/host automation rather than
sample playback. A FamiTracker-style sequencer (volume/arpeggio/pitch/hi-pitch/duty,
128 slots each, loop/release points) sits on top. See `README.md` for the full
feature list and the deliberate hardware-accuracy liberties taken (voice count,
triangle volume, linear mixing, software LFOs, adjustable sequencer rate,
anti-click voice-steal ramping) — worth reading before changing core emulation
or sequencer behavior, since those liberties are intentional design decisions,
not bugs.

## Commands

```bash
cargo build --release                          # whole workspace
cargo build --release -p rp2a03_niceplug        # plugin crate only
cargo test --workspace                          # all tests
cargo test -p rp2a03_common                      # one crate
cargo test <test_name>                           # single test, any crate
cargo xtask bundle rp2a03_niceplug --release     # produce .clap/.vst3 in target/bundled/
```

Windows (this environment's default) uses the `opengl` egui backend and needs
no extra setup beyond the MSVC rustup toolchain. `wgpu` does not currently
build on Windows (upstream `wgpu-hal`/`gpu-allocator` version mismatch) — don't
try to switch the default backend here. Full cross-platform build/packaging
instructions (macOS AUv2/VST3 via clap-wrapper, Linux Vulkan deps, release
zip layout) are in `README.md` and rarely relevant to day-to-day code changes.

Lints are centralized in the workspace `Cargo.toml` (`[workspace.lints]`) and
inherited via `[lints] workspace = true` in each crate — don't set per-crate
lint levels.

## Workspace architecture

Four crates, layered so the emulation core has zero UI/plugin dependencies:

- **`rp2a03_core`** — the emulation core. Only depends on `serde`. Pulse/Triangle/Noise
  APU channels (`apu_pulse.rs`, `apu_triangle.rs`, `apu_noise.rs`), VRC6 (`vrc6_pulse.rs`,
  `vrc6_saw.rs`), the FamiTracker-style `sequencer.rs` (`Sequence`/`SequencePlayer`,
  `PitchMode`/`ArpMode`/`VolMode`), `software_lfo.rs` (vibrato/tremolo — not real hardware),
  and `blip_buf.rs` (band-limited resampling from APU clock to host sample rate).
- **`rp2a03_common`** — MIDI + GUI logic shared by any plugin wrapper (currently one, but
  kept separate so a second host adapter wouldn't need to duplicate this layer).
  - `gui/state.rs` — `SharedSequences` (per-instrument state: selected tab, channel mode,
    polyphony settings, per-slot remembered waveforms), `SequenceBank` (128 `SequenceSlot`s
    per envelope type), `SequenceSlot` (text + parsed `Sequence` + per-slot `enabled` flag).
  - `gui/editor.rs`, `widgets.rs`, `theme.rs` — the egui sequence-editor UI.
  - `midi/handler.rs` — `MidiHandler`; `midi/types.rs` — `ChannelMode`, note→period conversion.
  - `format/patch.rs` — the native `.rp2a03patch` save/load format: encodes one instrument's
    sequences/active-slot-selection/waveform/step-rate to/from `SharedSequences`, with its own
    magic bytes (`RP2P`), a `format_version`, and validation on load (`PatchError`). **Field
    order in `Patch`/`PatchSequenceEntry`/`PatchSlotWaveform` is load-bearing for schema
    evolution** — new fields are always appended at the end, never inserted or reordered; see
    the doc comments in `patch.rs` before touching these structs. The full format spec is
    `docs/format.md` — read it before changing wire format, not just the code.
- **`rp2a03_niceplug`** — the actual plugin (CLAP+VST3 via the `nice-plug` framework).
  - `voice_bank.rs` — polyphony: voice allocation/stealing, per-voice event routing, and the
    per-clock render loop mixing up to 8 voices to one mono bus. Runs entirely on the audio
    thread; scratch buffers are pre-owned and reused so the render path never allocates.
  - `voice.rs` — one voice's channel units + resampler.
  - `plugin.rs`, `params.rs`, `editor.rs` — nice-plug plugin trait impl, automatable
    parameters, editor window glue.
  - `sequences.rs` — plugin-side sequence integration (`ActiveSequences`, `SequenceReload`).
- **`xtask`** — `cargo xtask bundle`/`bundle-universal` packaging tooling (produces the
  `.clap`/`.vst3` bundles in `target/bundled/`; separate from the CMake steps used only for
  macOS AUv2/VST3 wrapping).

The `parking_lot`-guarded `SharedSequences` is the boundary between the UI
thread (editor reads/writes it) and the audio thread (voices read from it) —
when touching sequence data, be conscious of which side of that boundary you're on.

`SharedSequences` is also persisted wholesale by the host as DAW project state
(nice-plug's `#[persist = "envelope_data"]` in `params.rs`). Any new field on it
therefore needs a `#[serde(default)]`, or projects saved by an older build fail
to load. Note also that serde only implements `Deserialize` for arrays up to
length 32 — collections sized to `MAX_SEQUENCES` (128) are `Vec`s for that
reason, not by preference.