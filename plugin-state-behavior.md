## 1. Startup State

When a new plugin instance is created, all envelope editors must initialize in the “empty” state:

- All envelope editors should start with `step_count = 0`
- No envelope data should be present initially
- This empty state must be distinguishable from a one-step envelope

### Default fallback behavior when an envelope is empty / disabled

If the player plays notes while an envelope editor is empty, the plugin must fall back to these defaults:

- Volume envelope empty/disabled → use volume `15`
- Duty envelope empty/disabled → use duty `0` (12.5% square wave)
- Arpeggio / Pitch / Hi-pitch empty/disabled → use value `0`

> Note: This has seemengly been achieved with the current state of the codebase, but double-checking is imporant

---

## 2. Runtime Behavior

### User interaction with envelope editors

The plugin should support the following flow:

- The user opens an envelope editor
- The user creates or edits custom envelope sequences
- Custom sequences are stored in plugin memory
- The editor must support up to `127` user-created sequences

### Sequence selection and automation

- The user must be able to switch between stored sequences
- Sequence selection should be possible via:
  - a MIDI instrument list
  - GUI controls such as a spin-box
  - automation parameters

---

## 3. Data and Memory Model

### User-made sequences

- The plugin must allow up to `127` unique sequences per envelope editor
- These sequences are private to the plugin instance
- Multiple plugin instances must not share sequence data
- Each instance must preserve its own sequences independently

### Persistence and storage safety

- Use a safe storage layout that can handle 127 sequences without data corruption
- Consider how to store:
  - sequence length
  - step values
  - loop settings
  - enabled/disabled state

---

## 4. Reusable Code Design

### Shared editor logic

- The sequence editor / instrument settings system should be implemented as reusable code
- This is important because the same sequence editor concept will also apply to:
  - triangle channel
  - noise channel
  - expansion chips

### Current scope

- For now, implement the reusable editor logic with pulse channels first
- Keep the design generic so it can later be reused for other waveform types

---

## 5. Important Implementation Notes

- The FamiTracker source code is the primary reference and should be reviewed first (Ideas-ref-folder\dn-famitracker-source)
- The current plugin behavior must be corrected so startup state matches FamiTracker:
  - step count starts at `0`
  - only when the user creates or loads data does the step count become greater than `0`
- The empty state must behave correctly for playback fallback
