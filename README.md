![RP2A03 Logo](readme_assets/logo.png)
---

**RP2A03 Synth** is a WiP NES synthesizer plugin for modern DAWs.

The sound engine is not a sample library or an approximated "chiptune" waveform
generator — it is a emulation of the NES RP2A03 APU, driven
directly by MIDI and host automation. So the plugin reproduces the actual quirks of the hardware instead of
approximating them — though some deliberate liberties were taken for the sake of
flexibility, see [Accuracy & Creative Liberties](#accuracy--creative-liberties).

Beyond the stock 2A03 channels, the plugin also emulates the Konami **VRC6**
and the Sunsoft **5B** (More soon!) — see
[Supported Chips](#supported-chips).
On top of the emulation core sits a FamiTracker-style sequence engine
(volume / arpeggio / pitch / hi-pitch / duty sequences with loop and release
points) plus software vibrato/tremolo LFOs (Also from FamiTracker), so the instrument plays and automates
like a normal synth while the audio math underneath stays somewhat hardware-accurate.

# Synth Showcase
### Cover of Snail House's "SUNNY" by [recme](https://github.com/recm3)
https://github.com/user-attachments/assets/b758d92e-7e35-460d-8dde-89ff16a07951


### Cover of "The Brilliant Truth (Mina The Hollower)" by Me (Purpbatboi)
https://github.com/user-attachments/assets/d7d7984e-a96e-4002-8f1c-d2bf56444ee7







## Features

- **Register-level 2A03 emulation** — pulse 1/2, triangle, noise, with real
  envelope, sweep, length counter, linear counter, and frame-counter behavior.
- **VRC6 and Sunsoft 5B expansion audio** — see
  [Supported Chips](#supported-chips).
- **Band-limited output** — Blargg-style `blip_buf` synthesis resamples from the
  APU clock down to the host sample rate without aliasing.
- **FamiTracker-style sequencer** — 5 sequence types × 128 slots, with loop (`|`)
  and release (`/`) markers, `ArpMode` (absolute/fixed/relative), `PitchMode`,
  and `VolMode` step handling. Adjustable step rate (1–600 Hz, default 60).
- **Optional polyphony, up to 8 voices** (monophonic by default) with voice
  stealing and a short allocation ramp so recycling a voice mid-release doesn't
  click.
- **Software vibrato / tremolo LFOs** (the real 2A03 has none of its own) and
  portamento.
- **Host automation** for vibrato/tremolo depth+speed, hardware volume, fine and
  hi pitch, pitch slide (+ range), step time, sequence index, polyphony, max
  voices, and portamento (+ speed). Waveform is deliberately *not* automatable:
  it belongs to the sequence slot, which remembers its own waveform, so you
  change it by automating the sequence index.
- **MIDI pitch wheel** drives Pitch Slide, and RPN 0 (Pitch Bend Sensitivity)
  drives Pitch Slide Range, up to two octaves. Whichever moved last wins, so the
  wheel takes over from the parameter and the parameter takes it back the moment
  it moves; Reset All Controllers (CC 121) hands both back to the parameters.

## Supported Chips

Each voice picks one waveform from the list below; every waveform gets the same
sequencer, LFOs, portamento, and automation on top. The 2A03 is the console's
own APU; the VRC6 and 5B are cartridge expansion chips that a handful of
Japanese releases shipped with their own sound hardware.

| Chip | Waveforms in this plugin | Where you'd have heard it |
|---|---|---|
| **Ricoh 2A03** (stock NES APU) | Pulse (2A03 duties), Triangle, Noise | Essentially every NES game |
| **Konami VRC6** | VRC6 Pulse, VRC6 Sawtooth | *Akumajō Densetsu* (JP *Castlevania III*), *Madara*, *Esper Dream 2* |
| **Sunsoft 5B** | S5B (PSG tone + noise) | *Gimmick!*, *Batman: Return of the Joker* |

The 2A03's DPCM/sample channel is **not** implemented — this is a synthesizer,
and DPCM is a sample-playback channel with nothing to synthesize.

### Ricoh 2A03 — the stock APU

- **Pulse 1 & 2** — 4 duty cycles (12.5% / 25% / 50% / 75%), hardware envelope,
  length counter, and the real sweep unit including pulse 1's off-by-one negate
  quirk.
- **Triangle** — 32-step stepped waveform with the linear counter. On hardware
  it has no volume control at all; here it responds to volume and volume
  sequences (see [Accuracy & Creative Liberties](#accuracy--creative-liberties)).
- **Noise** — 15-bit LFSR with both tap modes (long "hiss" and short "tone"),
  16 hardware periods. Has no pitch/hi-pitch or duty lanes, matching the chip.

### Konami VRC6

- **VRC6 Pulse** — 8 duty cycles (6.25% .. 50% in 1/16 steps, twice the 2A03's
  four) plus the "ignore duty" mode that forces the output high for a constant
  DC level. Unlike the 2A03's fixed waveform table, the duty here is a
  programmable threshold against a free-running 16-step counter.
- **VRC6 Sawtooth** — the chip's accumulator-based saw, a waveform the stock
  2A03 simply cannot make. Its volume lane is 6-bit (64 steps) rather than 4-bit,
  because the saw's accumulator rate register is wider than a normal volume
  register.

### Sunsoft 5B — an AY-3-8910 with a bit of AY8930 grafted on

This is the most interesting one, and it is **deliberately not a pure 5B**.

The real Sunsoft 5B is a Yamaha YM2149 (an AY-3-8910 derivative) sitting on the
cartridge: three square-wave tone channels, one shared noise generator, one
hardware envelope. Its stock tone generator is a plain 50% square with no duty
control whatsoever.

The **AY8930** is a later, backward-compatible superset of that same PSG family.
In its "expanded mode" it adds, among other things, nine selectable tone duty
widths. It was never the chip on a Sunsoft cartridge — but the two are close
enough relatives that its duty feature ports cleanly onto the 5B's tone
generator.

So this plugin's S5B is a hybrid:

| Feature | Behavior here | Source |
|---|---|---|
| Tone generator | 12-bit period, three channels | AY-3-8910 / YM2149 (stock 5B) |
| **Tone duty width** | **9 presets, 3.125% .. 96.875%** | **AY8930 expanded mode** |
| Noise generator | 17-bit LFSR, 5-bit period, shared | stock 5B |
| Mixer | Per-step tone/noise enable, active-low | stock 5B |
| Volume | 5-bit (32-step) logarithmic ladder | stock 5B |
| Hardware envelope | **not implemented** — see below | — |

**The duty width** is the AY8930 part. It uses that chip's own nine 32-bit duty
patterns rather than a re-derived approximation, indexed by the tone's position
in its current wave — so index 4 (50%) reproduces exactly the plain square a
stock 5B produces, and the other eight are genuine AY8930 ratios. It
lives in the **Noise / Mode** sequence lane as a second bar graph stacked above
the noise period, so duty width is per-step and automatable like everything
else. It only affects the tone generator's mark/space ratio and is fully
orthogonal to tone/noise mixing.

**The hardware envelope is deliberately absent.** The chip has one, but its
period and shape are only reachable through tracker effect columns, which a
DAW-hosted synth has no equivalent of — it could therefore only ever run as an
untunable free-running ramp. The volume lane covers the musically useful part.

Two more 5B-specific choices worth knowing:

- **Volume resolution.** The 5B's volume register is 5-bit, and its ladder is
  logarithmic at roughly 1.5 dB per step. The volume lane offers 16-step
  (default) or 32-step mode. 16-step maps *linearly onto loudness* so the lane
  behaves like the 2A03 pulse's lane; 32-step drives the ladder index directly
  for the chip's true resolution, including the 16 levels the 4-bit register
  cannot even address.
- **Noise pitch direction.** Register 6 is a divider, so raising it *lowers* the
  pitch. The editor inverts it, so dragging the noise bar up raises the pitch,
  following dnFamiTracker's convention rather than the raw register.

## Accuracy & Creative Liberties

**This is not a 100% hardware-accurate emulation, and it isn't trying to be.**
The per-channel building blocks — timers, envelopes, sweep, length/linear
counters, the frame counter, the duty sequencers — are modeled at the register
level. But this is a *synthesizer*, not an emulator, so a number of deliberate
liberties were taken — mostly where strict accuracy would just make the
instrument annoying to use, and in one case (the 5B's duty width) where a
sibling chip had something worth stealing:

- **Voice count.** Real hardware gives you exactly two pulse channels, one
  triangle, and one noise. Here every voice owns its own full set of channel
  units and picks one via `ChannelMode`, so you can stack up to 8 voices of
  *any* waveform — eight triangles, if you want.
- **Triangle volume.** The real 2A03 triangle has no volume control at all; it's
  on or off at fixed amplitude. This one responds to the volume parameter and to
  volume sequences.
- **Mixing.** Voices are summed linearly with per-channel amplitude scaling and a
  master gain, rather than through the APU's real non-linear pulse/TND mixer
  lookup tables. Channel balance here is a mix decision, not a hardware
  measurement.
- **Software LFOs and portamento.** Neither exists on the 2A03 in any form. Both
  are layered on top of the emulation as ordinary synth features.
- **Sequencer rate.** Tracker sequences on hardware advance at the 60 Hz frame
  rate. Here the step rate is a parameter, adjustable from 1–600 Hz.
- **Anti-click ramping.** Voice reallocation applies a short amplitude ramp, and
  triangle phase is deliberately preserved across reallocation — the hardware
  would happily click.
- **Pitch.** Fine-pitch and hi-pitch offsets extend beyond what a period-register
  write alone would give you on hardware.
- **The Sunsoft 5B is a hybrid chip that never existed.** Every other liberty on
  this list *removes* a hardware limit; this one *adds* a feature from a
  different chip. The real 5B's tone generator is a fixed 50% square with no
  duty control at all — the nine duty widths here (3.125% .. 96.875%) come from
  the **AY8930**, a later superset of the same PSG family that no Sunsoft
  cartridge ever carried. The two chips are close enough relatives that the
  feature drops in cleanly, and the default (50%) is exactly the stock square,
  so a patch that never touches the width is a faithful 5B. Reach for the other
  eight and you are playing hardware that was never manufactured. See
  [Supported Chips](#supported-chips).
- **5B volume curve.** The chip's 5-bit volume ladder is logarithmic (~1.5 dB
  per step), so a half-height lane is about −21 dB, not −6 dB. The default
  16-step volume mode remaps the lane *linearly onto loudness* so it behaves
  like the 2A03 pulse's lane instead — a deliberate divergence from both the
  chip and dnFamiTracker. The 32-step mode is the raw ladder if you want it.
- **5B hardware envelope.** Not implemented, deliberately. Its period and shape
  are only reachable through tracker effect columns, so in a DAW it could only
  ever be an untunable free-running ramp.

The intent is that anything sounding like the NES sounds *right*, while none of
the hardware's arbitrary limits stop you from writing music. If you need
bit-exact hardware behavior, use [FamiStudio](https://famistudio.org/), [Furnace Tracker](https://tildearrow.org/furnace/) or [dnFamiTracker](https://github.com/Dn-Programming-Core-Management/Dn-FamiTracker)!

## Workspace Structure

```
RP2A03-SYNTH/
├── rp2a03_core/           # emulation core — no plugin/UI dependencies
│   ├── src/
│   │   ├── apu.rs             # envelope, frame_counter, length_counter, timer
│   │   ├── apu_pulse.rs       # Pulse, Sweep, DutySequencer
│   │   ├── apu_triangle.rs    # Triangle, LinearCounter
│   │   ├── apu_noise.rs       # Noise, ShiftMode
│   │   ├── vrc6_common.rs     # Divider
│   │   ├── vrc6_pulse.rs      # Vrc6Pulse
│   │   ├── vrc6_saw.rs        # Vrc6Saw
│   │   ├── s5b_audio.rs       # Sunsoft, Psg (5B tone/noise/duty)
│   │   ├── sequencer.rs       # Sequence / SequencePlayer, PitchMode/ArpMode/VolMode
│   │   ├── software_lfo.rs    # SoftwareLfo (vibrato / tremolo)
│   │   ├── blip_buf.rs        # band-limited resampling
│   │   └── lib.rs             # NTSC_CPU_CLOCK
│   └── Cargo.toml
├── rp2a03_common/         # MIDI + GUI logic shared by any plugin wrapper
│   ├── src/
│   │   ├── gui/
│   │   │   ├── mod.rs
│   │   │   ├── state.rs       # SharedSequences, SequenceBank, SequenceSlot
│   │   │   ├── editor.rs      # render_editor_ui
│   │   │   ├── theme.rs
│   │   │   └── widgets.rs
│   │   ├── midi/
│   │   │   ├── mod.rs
│   │   │   ├── handler.rs     # MidiHandler
│   │   │   ├── events.rs
│   │   │   ├── types.rs       # ChannelMode, note→period conversion
│   │   │   └── tests.rs
│   │   └── lib.rs
│   └── Cargo.toml
├── rp2a03_niceplug/       # the plugin itself — CLAP + VST3 exports
│   ├── src/
│   │   ├── editor.rs
│   │   ├── lib.rs
│   │   ├── params.rs
│   │   ├── plugin.rs
│   │   ├── sequences.rs
│   │   ├── tests.rs
│   │   ├── voice_bank.rs
│   │   └── voice.rs
│   └── Cargo.toml
├── xtask/                 # packaging / bundling tooling (nice-plug-xtask)
│   ├── src/
│   │   └── main.rs
│   └── Cargo.toml
├── packaging/             # platform-specific plugin packaging (CMake)
│   ├── auv2/              # clap-wrapper CMake project — wraps the .clap as AUv2 (macOS)
│   │   └── CMakeLists.txt
│   └── vst3-macos/        # clap-wrapper CMake project — wraps the .clap as VST3 (macOS)
│       └── CMakeLists.txt
├── readme_assets/         # images and media for README
├── Cargo.toml             # workspace root
├── Cargo.lock
├── LICENSE
└── README.md
```

`rp2a03_core` depends only on `serde`, so it can be tested and reasoned about in
isolation from the plugin framework and the UI.

## Tech Stack

- **Rust**, 2024-edition workspace.
- **[nice-plug](https://crates.io/crates/nice-plug) / nice-plug-egui** — the
  CLAP+VST3 plugin framework (an `nih-plug`-lineage API) and its egui editor
  windowing helper.
- **egui / egui_extras** — the custom sequence-editor UI.
- **parking_lot** — the shared-state boundary between the UI and audio threads.
- **serde** — parameter and sequence persistence.

## Building

**Prerequisites**: Rust toolchain (stable, 2024 edition support). There is no
C/C++ toolchain, CMake step, or vendored native dependency — the whole tree is
pure Rust and builds identically on all three platforms.

Build the whole workspace (release):

```bash
cargo build --release
```

Build only the plugin crate:

```bash
cargo build --release -p rp2a03_niceplug
```

Produce installable CLAP and VST3 bundles:

```bash
cargo xtask bundle rp2a03_niceplug --release
```

Raw build output lands in `target/release/`; the packaged `.clap` / `.vst3`
bundles land in `target/bundled/`.

Run the tests:

```bash
cargo test --workspace
```

### Rendering backend

The editor is drawn by `egui` through one of two backends, selected by a Cargo
feature on `rp2a03_niceplug`:

| Feature | Backend | Official builds |
|---|---|---|
| `opengl` (default) | OpenGL via `egui_glow` | Windows |
| `wgpu` | Metal / Vulkan / DX12 via `wgpu` | macOS, Linux |

macOS deprecated OpenGL, so `wgpu` (which runs on Metal there) is the supported
path on Apple hardware, and Linux uses it for the same Vulkan-first reasons.

Select it with:

```bash
cargo xtask bundle rp2a03_niceplug --release --no-default-features --features wgpu
```

`--no-default-features` matters. `wgpu` takes priority over `opengl` when both
are on, so plain `--features wgpu` still *works* — it just also compiles
`egui_glow` for nothing.

> **`wgpu` does not currently build on Windows.** `wgpu-hal 29.0.4` builds
> against `windows 0.62` while its own `gpu-allocator 0.28` is still on
> `windows 0.61`, so the D3D12 types come from two different crates and the
> trait bounds don't line up. This is an upstream issue; use the default
> `opengl` backend on Windows.

### Windows

No extra setup. Use the MSVC toolchain (`stable-x86_64-pc-windows-msvc`, the
rustup default). Windows builds use the default `opengl` backend.

```powershell
cargo xtask bundle rp2a03_niceplug --release
```

Install by copying from `target/bundled/`:

| Format | Destination |
|---|---|
| `.vst3` | `%COMMONPROGRAMFILES%\VST3\` |
| `.clap` | `%COMMONPROGRAMFILES%\CLAP\` |

### Linux

Windowing goes through X11, so the development headers must be present:

```bash
sudo apt-get install -y \
  libasound2-dev libgl1-mesa-dev libx11-dev libxcursor-dev \
  libxrandr-dev libxi-dev libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev
```

Official Linux builds use the `wgpu` backend, which reaches the GPU through
Vulkan — end users need working Vulkan drivers (`mesa-vulkan-drivers` or a
vendor driver):

```bash
cargo xtask bundle rp2a03_niceplug --release --no-default-features --features wgpu
```

On Windows, WSL2 is a workable way to produce Linux bundles.

Install by copying from `target/bundled/`:

| Format | Destination |
|---|---|
| `.vst3` | `~/.vst3/` |
| `.clap` | `~/.clap/` |

> Release builds are produced on Ubuntu 22.04 (glibc 2.35). Building locally on
> a newer distro is fine, but binaries built there will not load on older ones.

### macOS

Add both Darwin targets once, then use `bundle-universal` — it builds x86_64 and
aarch64 and `lipo`s them into one fat bundle that runs on both Intel and Apple
Silicon:

```bash
rustup target add x86_64-apple-darwin aarch64-apple-darwin
cargo xtask bundle-universal rp2a03_niceplug --release --no-default-features --features wgpu
```

For a single-architecture build, `cargo xtask bundle rp2a03_niceplug --release
--no-default-features --features wgpu` works too. `xtask` ad-hoc signs
(`codesign -s -`) every macOS bundle it produces.

#### Audio Unit (AUv2)

`cargo xtask` only emits CLAP and VST3. The `.component` is produced by
[`clap-wrapper`](https://github.com/free-audio/clap-wrapper), which wraps the
already-built `.clap` — so this step runs *after* the bundle step above:

```bash
cmake -B build/auv2 -S packaging/auv2
cmake --build build/auv2 --config Release
```

Result: `build/auv2/RP2A03 Synth.component`, with the `.clap` embedded
inside it, so the AU is self-contained.

Requirements: CMake ≥ 3.21 and the Xcode command line tools. clap-wrapper is
pulled in by `FetchContent` (pinned to `v0.15.1`) and downloads the CLAP headers
and AudioUnitSDK itself — no submodule to initialise.

> **Why AUv2 and not AUv3, given Apple deprecated AUv2?**
> clap-wrapper's AUv3 container app does not compile on current Xcode. In its
> `AUv3HostAppDelegate.mm` the `AVAuthorizationStatusNotDetermined` case body is
> unbraced, so the completion block it creates is still in scope at the next
> `default:` label — which Objective-C++ rejects outright:
> `error: cannot jump from switch statement to this case label`.
>
> The `.appex` itself builds and embeds the `.clap` correctly; only the
> container `.app` fails. A container is not optional: macOS registers an app
> extension solely through the app that carries it, so without it there is
> nothing for a user to install. AUv3 also requires the Xcode CMake generator
> (`-G Xcode`), since the `.appex` must be linked as an app-extension product
> and signed by Xcode.
>
> The bug is present in `v0.15.1` and still on upstream `main`, so there is no
> newer tag to move to. Worth revisiting once upstream fixes it — the change is
> two braces. AUv2 loads in every current major DAW in the meantime.

Notes on [`packaging/auv2/CMakeLists.txt`](packaging/auv2/CMakeLists.txt):

- clap-wrapper consumes the compiled `.clap`, not our source, so nothing about
  it is tied to nice-plug. The widely-linked write-up of this technique targets
  `nih-plug`, but that detail never mattered — any spec-compliant CLAP works.
- That write-up wraps a gain plugin and sets `INSTRUMENT_TYPE "aufx"`. That is
  the AU *effect* type. This is a synth, so it uses `aumu` (music device).
- `CMAKE_OSX_ARCHITECTURES` is forced to `x86_64;arm64` so the AU is universal
  like the `.clap` and `.vst3`.
- `MANUFACTURER_CODE` / `SUBTYPE_CODE` must each be exactly 4 characters and
  must not be all-lowercase (Apple reserves those).

#### VST3 (macOS, via clap-wrapper)

niceplug's own VST3 output has had unresolved issues on macOS, so the release
workflow instead wraps the `.clap` with clap-wrapper — the same mechanism used
for the AU above — and ships that in place of niceplug's VST3:

```bash
cmake -B build/vst3-macos -S packaging/vst3-macos
cmake --build build/vst3-macos --config Release
```

Result: `build/vst3-macos/RP2A03 Synth.vst3`, with the `.clap` embedded
inside it. This *replaces* the `.vst3` that `cargo xtask` produced in
`target/bundled/` — the release workflow deletes niceplug's copy before copying
this one in.

Install by copying from `target/bundled/`:

| Format | Destination |
|---|---|
| `.vst3` | `/Library/Audio/Plug-Ins/VST3/` or `~/Library/Audio/Plug-Ins/VST3/` |
| `.clap` | `/Library/Audio/Plug-Ins/CLAP/` or `~/Library/Audio/Plug-Ins/CLAP/` |
| `.component` | `/Library/Audio/Plug-Ins/Components/` or `~/Library/Audio/Plug-Ins/Components/` |

Because the plugin is ad-hoc signed rather than notarized, macOS quarantines it
when it arrives via a downloaded release archive. Strip the flag after copying:

```bash
xattr -dr com.apple.quarantine "/Library/Audio/Plug-Ins/VST3/RP2A03 Synth.vst3"
```

Logic caches AU scan results, so after installing a `.component` you may need to
force a rescan:

```bash
killall -9 AudioComponentRegistrar
auval -a | grep -i rp2a03    # should list the aumu component
```

### Release packaging

Pushing a `v*.*.*` tag runs [`.github/workflows/release.yml`](.github/workflows/release.yml),
which builds all three platforms and publishes a single `RP2A03_synth.zip`:

```
RP2A03_synth/
├── Windows/    # x86_64, OpenGL                          .clap + .vst3
├── Linux/      # x86_64, wgpu (Vulkan)                   .clap + .vst3
└── Mac/        # universal x86_64 + arm64, wgpu (Metal)  .clap + .vst3 + .component
```

The workflow can also be run manually from the Actions tab; a manual run
produces the same zip as a build artifact but does not create a release.

## Feature status

| Component | Status | Location |
|---|---|---|
| Pulse 1 & 2 | done | `rp2a03_core/src/apu_pulse.rs` |
| Triangle | done | `rp2a03_core/src/apu_triangle.rs` |
| Noise | done | `rp2a03_core/src/apu_noise.rs` |
| VRC6 pulse | done | `rp2a03_core/src/vrc6_pulse.rs` |
| VRC6 sawtooth | done | `rp2a03_core/src/vrc6_saw.rs` |
| Sunsoft 5B (PSG) | done | `rp2a03_core/src/s5b_audio.rs` |
| 5B hardware envelope | not planned — see [Supported Chips](#supported-chips) | — |
| FDS wavetable | planned | — |
| Namco 163 | planned | — |
| 2A03 DPCM | not planned (sample playback, not synthesis) | — |

---

## Credits & Attribution

This synthesizer relies on foundational open-source code and deep research into
NES APU behavior, emulation, and tracker design. We gratefully acknowledge the
authors and projects below:

### APU Architecture & DSP Reference

* **[TetaNES](https://github.com/lukexor/tetanes)** (License: **MIT / Apache 2.0**)
  * *Author*: Luke Petherbridge
  * *Contribution*: The core APU channel structure, pulse timer, envelope, and frame counter implementations in `rp2a03_core` were adapted and referenced from TetaNES's core APU module.
* **[MesenCE](https://github.com/nesdev-org/MesenCE)** (License: **GPL-3.0-or-later**)
  * *Author*: SourMesen
  * *Contribution*: The original C++ VRC6 Pulse and Saw Wave implementations
* **[emu2149](https://github.com/digital-sound-antiques/emu2149)** (License: **MIT**)
  * *Authors*: Mitsutaka Okazaki, with envelope/noise fixes by alexmush
  * *Contribution*: The Sunsoft 5B's PSG core in `rp2a03_core/src/s5b_audio.rs` — tone/noise/envelope generators and both volume tables — is adapted from emu2149. The register-select latch and channel-enable masking around it are adapted from **Nes_Sunsoft** (FamiStudio's 5B wrapper for Nes_Snd_Emu) by [@NesBleuBleu](https://github.com/BleuBleu).
* **[Furnace Tracker](https://github.com/tildearrow/furnace)** (License: **GPL-2.0-or-later**)
  * *Author*: tildearrow and contributors
  * *Contribution*: The AY8930 duty-cycle implementation in `ay8910.cpp` — the nine 32-bit duty patterns and the phase-counter approach that indexes them — is the reference for this plugin's S5B tone duty width.

---

### Reference Architecture

The following projects were extensively referenced during research/development:

* **[FamiStudio](https://famistudio.org/)** / **[FamiStudio GitHub](https://github.com/BleuBleu/FamiStudio)** (License: **MIT**)
* **[Dn-FamiTracker](https://github.com/Dn-Programming-Core-Management/Dn-FamiTracker)** (License: **GPL-3.0-or-later**)
* **[Furnace Tracker](https://github.com/tildearrow/furnace)** (License: **GPL-2.0-or-later**)
* **[Mesen / MesenCE](https://github.com/nesdev-org/MesenCE)** (License: **GPL-3.0-or-later**)
* **[puNES](https://github.com/punesemu/puNES)** (License: **GPL-2.0-or-later**)
* **[NesDEV and its community](https://www.nesdev.org/wiki/Nesdev_Wiki)**

---

## License

Original code written specifically for this project is released under the **WTFPL** (Do What The Fuck You Want To Public License). 

Code adapted from upstream open-source projects continues to retain and respect their original permissive/open licenses (e.g. TetaNES and Dn-FamiTracker).

```text
            DO WHAT THE FUCK YOU WANT TO PUBLIC LICENSE
                    Version 2, December 2004

 Copyright (C) 2026 Purpbatboi

 Everyone is permitted to copy and distribute verbatim or modified
 copies of this license document, and changing it is allowed as long
 as the name is changed.

            DO WHAT THE FUCK YOU WANT TO PUBLIC LICENSE
   TERMS AND CONDITIONS FOR COPYING, DISTRIBUTION AND MODIFICATION

  0. You just DO WHAT THE FUCK YOU WANT TO.
```

---

## AI Disclosure

OpenAI's Codex, Google Antigravity, and Anthropic's Claude Code AI agents were
used in the development of this plugin.
