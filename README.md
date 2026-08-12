![RP2A03 Logo](readme_assets/logo.png)
---

**RP2A03 Synth** is a work-in-progress NES synthesizer plugin for modern DAWs.

It emulates the NES **RP2A03 APU** at the register level rather than using samples or approximated chiptune waveforms. It also supports **Konami VRC6** and **Sunsoft 5B** expansion audio.

A FamiTracker-style sequencer, software LFOs, portamento, MIDI, and host automation turn the hardware emulation into a practical software synth.

> **Alpha software:** expect bugs and incomplete features.

## Synth Showcase

### "SUNNY" — Snail House cover by [recme](https://github.com/recm3)

https://github.com/user-attachments/assets/b758d92e-7e35-460d-8dde-89ff16a07951

### "The Brilliant Truth" — *Mina the Hollower* cover by Purpbatboi

https://github.com/user-attachments/assets/d7d7984e-a96e-4002-8f1c-d2bf56444ee7

## Features

* Register-level **2A03** emulation

  * Pulse 1/2
  * Triangle
  * Noise
  * Envelope, sweep, length/linear counters, and frame counter
* **VRC6** pulse and sawtooth
* **Sunsoft 5B** PSG
* Band-limited output via `blip_buf`
* FamiTracker-style sequences:

  * Volume
  * Arpeggio
  * Pitch
  * Hi-pitch
  * Duty
  * Loop and release points
* Up to **8-voice polyphony**
* Software vibrato and tremolo
* Portamento
* MIDI pitch wheel and pitch-bend sensitivity
* DAW host automation
* Adjustable sequence rate from **1–600 Hz**

## Supported Chips

| Chip            | Waveforms              | Examples                                     |
| --------------- | ---------------------- | -------------------------------------------- |
| **Ricoh 2A03**  | Pulse, Triangle, Noise | Most NES games                               |
| **Konami VRC6** | Pulse, Sawtooth        | *Castlevania III*, *Madara*, *Esper Dream 2* |
| **Sunsoft 5B**  | PSG tone + noise       | *Gimmick!*, *Batman: Return of the Joker*    |

The 2A03 DPCM channel is **not implemented** because it is a sample-playback channel rather than a synthesis channel.

### 5B note

The 5B implementation includes nine selectable tone-duty widths based on the **AY8930**. This feature was not present on the original Sunsoft 5B, so the S5B mode is intentionally a hybrid.

The 5B hardware envelope is not implemented.

## Accuracy

This is a **synthesizer inspired by hardware emulation**, not a cycle-perfect NES emulator.

The core models the important APU behavior at the register/channel level, but intentionally removes or adds some hardware limitations:

* Polyphony is expanded to 8 voices.
* Triangle volume is controllable.
* Mixing uses conventional linear voice summing.
* Software LFOs and portamento are added.
* Sequence rate is adjustable rather than fixed to 60 Hz.
* Voice allocation uses short ramps to prevent clicks.
* Pitch controls extend beyond raw hardware period registers.
* The 5B adds AY8930-style duty control.

For bit-accurate tracker playback, use [FamiStudio](https://famistudio.org/), [Furnace](https://tildearrow.org/furnace/), or [Dn-FamiTracker](https://github.com/Dn-Programming-Core-Management/Dn-FamiTracker).


## Building

### Requirements

* Rust stable
* Rust 2024 edition support

The core is pure Rust; no C/C++ toolchain is required for the normal builds.

### Build

```bash
cargo build --release
```

Run tests:

```bash
cargo test --workspace
```

Build the plugin bundles:

```bash
cargo xtask bundle rp2a03_niceplug --release
```

Output:

```text
target/release/
target/bundled/
```

## Platform Builds

### Windows

Uses the default **OpenGL** backend.

```powershell
cargo xtask bundle rp2a03_niceplug --release
```

Install:

| Format  | Location                     |
| ------- | ---------------------------- |
| `.vst3` | `%COMMONPROGRAMFILES%\VST3\` |
| `.clap` | `%COMMONPROGRAMFILES%\CLAP\` |

### Linux

Linux builds use **wgpu/Vulkan**.

Install the development dependencies:

```bash
sudo apt-get install -y \
  libasound2-dev libgl1-mesa-dev libx11-dev libxcursor-dev \
  libxrandr-dev libxi-dev libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev
```

Build:

```bash
cargo xtask bundle rp2a03_niceplug --release \
  --no-default-features --features wgpu
```

Install:

| Format  | Location   |
| ------- | ---------- |
| `.vst3` | `~/.vst3/` |
| `.clap` | `~/.clap/` |

### macOS

macOS uses **wgpu/Metal**.

For a universal Intel + Apple Silicon build:

```bash
rustup target add x86_64-apple-darwin aarch64-apple-darwin

cargo xtask bundle-universal rp2a03_niceplug --release \
  --no-default-features --features wgpu
```

The normal bundle command can be used for a single architecture.

#### AUv2

Build the AU wrapper after building the CLAP:

```bash
cmake -B build/auv2 -S packaging/auv2
cmake --build build/auv2 --config Release
```

Output:

```text
build/auv2/RP2A03 Synth.component
```

#### macOS VST3

The release build wraps the CLAP using `clap-wrapper`:

```bash
cmake -B build/vst3-macos -S packaging/vst3-macos
cmake --build build/vst3-macos --config Release
```

Install:

| Format       | Location                              |
| ------------ | ------------------------------------- |
| `.vst3`      | `/Library/Audio/Plug-Ins/VST3/`       |
| `.clap`      | `/Library/Audio/Plug-Ins/CLAP/`       |
| `.component` | `/Library/Audio/Plug-Ins/Components/` |

Because development builds are ad-hoc signed, downloaded macOS releases may require:

```bash
xattr -dr com.apple.quarantine "/Library/Audio/Plug-Ins/VST3/RP2A03 Synth.vst3"
```

## Project Structure

```text
RP2A03-SYNTH/
├── rp2a03_core/       # APU and expansion-chip emulation
├── rp2a03_common/     # MIDI and shared GUI logic
├── rp2a03_niceplug/   # CLAP/VST3 plugin
├── xtask/              # Build and packaging tools
├── packaging/          # AUv2/VST3 wrappers
├── readme_assets/
├── Cargo.toml
└── README.md
```

The emulation core is independent of the plugin/UI and can be tested separately.

## Release Builds

Pushing a `v*.*.*` tag builds all supported platforms and produces:

```text
RP2A03_synth/
├── Windows/   # x86_64, OpenGL
├── Linux/     # x86_64, wgpu/Vulkan
└── Mac/       # x86_64 + arm64, wgpu/Metal
```

A manual GitHub Actions run also produces the build artifact without creating a release.

## Crash Reporting

If the plugin crashes or hangs your DAW, attach the crash log to the issue.

| Platform | Crash log                                              |
| -------- | ------------------------------------------------------ |
| Windows  | `%LOCALAPPDATA%\rp2a03_synth\crash.log`                |
| macOS    | `~/Library/Application Support/rp2a03_synth/crash.log` |
| Linux    | `$XDG_DATA_HOME/rp2a03_synth/crash.log`                |

Set `NICE_LOG` to override the log location.

## Feature Status

| Feature        | Status      |
| -------------- | ----------- |
| 2A03 Pulse     | ✅ Done      |
| 2A03 Triangle  | ✅ Done      |
| 2A03 Noise     | ✅ Done      |
| VRC6 Pulse     | ✅ Done      |
| VRC6 Sawtooth  | ✅ Done      |
| Sunsoft 5B PSG | ✅ Done      |
| FDS Wavetable  | Planned     |
| Namco 163      | Planned     |
| 2A03 DPCM      | Not planned |

## Credits

This project builds on research and open-source work from:

* [TetaNES](https://github.com/lukexor/tetanes) — 2A03/APU architecture
* [MesenCE](https://github.com/nesdev-org/MesenCE) — VRC6 implementation reference
* [emu2149](https://github.com/digital-sound-antiques/emu2149) — Sunsoft 5B PSG
* [Furnace](https://github.com/tildearrow/furnace) — AY8930 duty-cycle reference
* [FamiStudio](https://famistudio.org/)
* [Dn-FamiTracker](https://github.com/Dn-Programming-Core-Management/Dn-FamiTracker)
* [puNES](https://github.com/punesemu/puNES)
* [NesDEV Wiki](https://www.nesdev.org/wiki/Nesdev_Wiki)

See the upstream projects for their respective licenses.

## License

Original code written specifically for this project is released under the **WTFPL v2**.

Adapted code remains under its original upstream license.

## AI Disclosure

Development has used **Anthropic Claude Code**.
