# `.rp2a03patch` File Format

A standalone, shareable binary file capturing one instrument's envelope/sequence
data — the numbered volume, arpeggio, pitch, hi-pitch, and duty sequences a
user has authored, plus which slot is currently active per envelope type,
which APU channel (waveform) the instrument is currently set to, and the
current engine speed.

This is separate from the plugin's in-DAW session state (`SharedSequences`,
persisted automatically via `#[persist = "envelope_data"]`). A `.rp2a03patch`
file is something the user explicitly saves/loads to move a sound between
projects or share it, independent of any DAW.

## Scope

A patch stores **envelope data only**. It does *not* include:

- Performance/host params: `vibrato_depth/speed`, `tremolo_depth/speed`,
  `hardware_volume`, `fine_pitch`, `hi_pitch`, `pitch_slide(_range)`,
  `polyphony`, `max_voices`, `portamento_enabled/speed`.
- Editor/UI state: which tab is shown, window size, etc.
- Metadata: no name, author, or description fields. The file's identity is
  its filename on disk.

Two host-automatable params **are** in scope, saved anyway because in
practice neither is something a host automates:

- The instrument's currently-selected APU channel (`waveform` — Pulse,
  Triangle, Noise, VRC6 Pulse, or VRC6 Saw); see `waveform` below.
- The engine speed (`step_time_hz`, labeled "Engine Speed"/"Step Time" in
  the UI); see `step_time_hz` below.

Loading a patch replaces the plugin's sequence banks, active-slot selection,
per-slot remembered waveforms, waveform selection, and engine speed; it leaves
every other parameter untouched.

## Wire format

The file is **binary**, not text:

```
[4-byte magic: "RP2P"] [MessagePack-encoded payload]
```

- **Magic bytes:** the ASCII literal `RP2P` (`0x52 0x50 0x32 0x50`), always
  exactly these 4 bytes, never bumped. Its only job is fast file-type
  sniffing — a file that doesn't start with `RP2P` is rejected immediately
  with a clear "not a `.rp2a03patch` file" error, instead of an obscure
  decode failure partway through parsing. This is independent of
  `format_version` (below), which governs schema-level compatibility, not
  file identity.
- **Payload encoding:** [MessagePack](https://msgpack.org/), via the
  `rmp-serde` crate, in **array mode** (the crate's default — not
  `with_struct_map()`). Every struct serializes as a positional array of its
  fields' values, with no field-name strings on the wire. This is
  substantially smaller than the same data as JSON text or as map-mode
  MessagePack, at the cost of a schema-evolution constraint — see below.
- **Trailing bytes are ignored.** `rmp_serde::from_slice` decodes the first
  complete MessagePack value from the payload and does not check whether the
  input was fully consumed. This format has no length field or checksum, so a
  file consisting of `RP2P` + a valid payload + arbitrary garbage appended
  after it loads "successfully," silently discarding the trailing garbage.
  This is tolerated deliberately (not a bug to fix): the loader has no way to
  distinguish meaningful trailing data from accidental padding without adding
  a length field or checksum to every future file, which hasn't been judged
  worth the complexity for this format's use case.

### Schema evolution rule: append-only struct fields

Because array-mode structs are positional, a deserializer reading an older
file relies on two things to stay compatible: MessagePack arrays carry an
explicit element count, so `serde`'s existing `#[serde(default)]` back-compat
convention (already used elsewhere in this codebase, e.g. `Sequence`'s
`vol_mode`) still works for fields a newer version added past the end of an
older file's array — the reader sees the array end early and falls back to
`default()`.

What this **breaks**, compared to JSON's arbitrary key-based flexibility: a
field can never be inserted in the middle of a struct, and existing fields
can never be reordered. Either change silently misaligns every field that
follows it, with no error — old files would deserialize into the wrong
fields rather than fail loudly. The rule going forward, for every struct in
this format:

> **New fields are always appended at the end. Existing fields are never
> reordered, renamed in position, or removed** (removing/repurposing a field
> is a breaking change — bump `format_version` and handle it explicitly
> instead).

This format also embeds several unit-only enums (`waveform`'s `ChannelMode`,
and `pitch_mode`/`arp_mode`/`vol_mode`'s `PitchMode`/`ArpMode`/`VolMode`).
`rmp-serde` writes unit enum variants as their **name string**, not their
numeric discriminant — the exact **inverse** of the struct-field rule above:

> Enum variant **names** are load-bearing (renaming `Vrc6Saw`, `Steps16`,
> `Relative`, etc. breaks every existing file that used that variant).
> Variant **order**, and the `#[repr(u8)]` discriminant values in the Rust
> definitions, are *not* load-bearing — reordering variants or renumbering
> their discriminants is harmless, since the wire only ever carries the name.

A contributor who has internalized "field order is load-bearing, field names
aren't" will get this backwards for enums — keep both rules in mind
separately.

## `format_version`

Integer, starts at `1`. Bumped whenever a future change to this format is not
representable by an appended, `#[serde(default)]`-backed field alone (e.g. a
field is removed, repurposed, or a struct's field order needs to change).
Loaders must reject files whose `format_version` is newer than they
understand, and may migrate older ones.

## Logical shape

The examples below are shown as JSON for readability only — the actual file
is the binary encoding described in "Wire format" above, not literal JSON
text. The *logical* structure is:

```json
{
  "format_version": 1,
  "waveform": "Pulse",
  "step_time_hz": 60,
  "active_indices": {
    "vol": 0,
    "arp": 0,
    "pitch": 0,
    "hipitch": 0,
    "duty": 0
  },
  "sequences": {
    "vol": [],
    "arp": [],
    "pitch": [],
    "hipitch": [],
    "duty": []
  },
  "slot_waveforms": []
}
```

Field order (top-level): `format_version`, `waveform`, `step_time_hz`,
`active_indices`, `sequences`, `slot_waveforms` — the two instrument-wide
settings sit up front, ahead of the envelope data itself. This order is
load-bearing (see "Schema evolution rule" above): it's fixed once shipped, and
any future top-level field is appended after `slot_waveforms`, the current last
field.

`slot_waveforms` was itself added this way — appended after `sequences`, backed
by `#[serde(default)]`, with no `format_version` bump, since a file written
before it existed simply ends its top-level array early and decodes the field as
an empty list. It is the worked example of the rule above.

### `waveform`

The APU channel/instrument type currently selected in the plugin (mirrors
the `waveform` param and its `ChannelMode` enum — see
`rp2a03_common::midi::types::ChannelMode`): `"Pulse"` | `"Triangle"` |
`"Noise"` | `"Vrc6Pulse"` | `"Vrc6Saw"`. Loading a patch sets this as the
plugin's active channel selection, same as `active_indices`.

### `step_time_hz`

The engine speed — how fast the sequencer engine advances envelope steps,
in Hz. Mirrors the `step_time` param (labeled "Engine Speed"/"Step Time" in
the UI depending on panel) and `HostAutomationControls::step_time_hz`.
Integer, `1..=600`, matching the param's own range.

This param is technically host-automatable, but in practice a host
automating engine speed mid-performance is not a realistic use case for this
synth, so storing its current value in the patch (same treatment as
`waveform`) is a reasonable simplification rather than a fidelity
compromise. Loading a patch sets this as the plugin's current engine speed.

### `active_indices`

The numbered slot (`0..=127`, see `MAX_SEQUENCES`) currently selected for
each of the 5 envelope types. Loading a patch sets these as the plugin's
active selection, so the patch is immediately playable rather than just a
library of authored-but-unselected slots.

All 5 keys are required.

### `sequences`

One array per envelope type (`vol`, `arp`, `pitch`, `hipitch`, `duty` —
matching `ActiveSequences`' field names and the tab order used throughout the
codebase). Each array holds only the numbered slots that are actually
*used*; slots that were never touched are omitted entirely rather than
serialized as 128 empty entries per type.

A slot counts as used if `values` is non-empty; an empty slot is simply
omitted from the file entirely. (Enabled-but-empty editor state — flipped on
but no steps entered yet — still doesn't survive a save/load round-trip,
which is fine since it isn't meaningful outside a live editing session.)

For a *non-empty* slot, `SequenceSlot::enabled` **is** stored on the wire
(see the `enabled` field below). A populated slot the user explicitly
switched off is authored data, not disposable editor state — the same
asymmetry `SequenceSlot`'s own `Serialize`/`Deserialize` derive already
respects (DAW project save/load preserves `enabled`), so `.rp2a03patch` must
not be the one path that silently flips it back to "on". Absent/older files
(no `enabled` key) default to `true` on load, matching the fact that every
entry ever written by an earlier build was, in practice, always enabled.

Each array entry (shown here in field order, since that order is now
load-bearing — see "Schema evolution rule" above):

```json
{
  "index": 3,
  "values": [15, 12, 8],
  "loop_point": 1,
  "release_point": null,
  "pitch_mode": "Relative",
  "arp_mode": "Absolute",
  "vol_mode": "Steps16",
  "enabled": true
}
```

| Field            | Type              | Notes |
|------------------|-------------------|-------|
| `index`          | integer, `0..=127`| The numbered slot this entry occupies. Unique within its array. |
| `values`         | array of integers | Signed step values, mirrors `Sequence::values` (`Vec<i16>`). |
| `loop_point`     | integer or `null` | Step index the sequence loops back to while a key is held. `null` = no loop marker. |
| `release_point`  | integer or `null` | Step index the sequence jumps to on note-off. `null` = no release marker. |
| `pitch_mode`     | `"Relative"` \| `"Absolute"` | Only meaningful when this entry is a pitch sequence; carried uniformly on every entry for 1:1 correspondence with `Sequence`, ignored otherwise. |
| `arp_mode`       | `"Absolute"` \| `"Relative"` | Only meaningful when this entry is an arpeggio sequence; ignored otherwise. No third value is defined — see note below. |
| `vol_mode`       | `"Steps16"` \| `"Steps64"` | Only meaningful when this entry is a volume sequence on the VRC6 sawtooth; ignored otherwise. Absent/omitted on older files defaults to `"Steps16"` (`#[serde(default)]`), matching `Sequence`'s own back-compat rule. |
| `enabled`        | boolean           | Whether `SequenceSlot::enabled` was set for this numbered slot when saved — see above. Absent/omitted on older files defaults to `true` (`#[serde(default = "default_enabled")]`; deliberately *not* a bare `#[serde(default)]`, which would default to `false` and disable every pre-existing envelope on load). This field must stay last in the struct — any future field is appended after it. |

Every field except `index` and `enabled` is a direct mirror of
`rp2a03_core::sequencer::Sequence` — no renaming, no derived/cached fields.
`SequenceSlot::enabled` **is** stored (see above), since it is authored data
that `SequenceSlot` itself already round-trips via serde.
`SequenceSlot::text`, the FamiTracker-string editing cache, is still *not*
stored; it is regenerated from `values`/`loop_point`/`release_point` on load
via the existing `sequence_to_text` formatter.

### `arp_mode` and FamiTracker import

`arp_mode` intentionally has no representation for FamiTracker's
`SETTING_ARP_FIXED` or `SETTING_ARP_SCHEME` (see
`Famitracker_Fti_format.md` §2.4 for the source enum). This is a deliberate
scope decision, not an oversight: implementing Fixed-mode arpeggios is
separate future work on the playback engine itself, not just a schema slot,
so there's nothing to gain from reserving a value for it ahead of time.

This is a real fidelity gap for a future `.fti` importer, not a cosmetic one
— and **not one that can be papered over by mapping Fixed to `"Absolute"`**,
despite the similar-sounding names. In this project's terms, `"Absolute"`
means *offset from the played note* (`SETTING_ARP_ABSOLUTE`'s own semantics,
carried over under the same name). FamiTracker's Fixed mode instead treats
each step as a **literal note value**, played regardless of what note was
pressed — an unrelated concept. Silently coercing Fixed into `"Absolute"`
would reinterpret literal notes as pitch offsets and produce audibly wrong
output, not a reasonable approximation. An importer encountering Fixed or
Scheme must pick its own explicit policy (reject the sequence, drop the
setting with a warning, refuse the whole file, etc.) — this spec deliberately
leaves that choice unresolved rather than baking in a wrong default.

### `slot_waveforms`

The waveform each numbered slot (`0..=127`) remembers, so selecting a sequence
index behaves like recalling an instrument: switching to slot 5 also switches
the plugin to whatever waveform was in use when slot 5 was last edited.

One waveform per *index*, not per envelope type — the plugin's shared Sequence
Index parameter already drives all 5 envelope banks in lockstep, so there is
only ever one slot number in play at a time.

Sparsely encoded, the same way `sequences` omits untouched slots: an entry is
written only for a slot whose waveform differs from the default (`"Pulse"`).
Slots nobody set are omitted entirely and decode back to `"Pulse"`.

```json
"slot_waveforms": [
  { "index": 0, "waveform": "Triangle" },
  { "index": 5, "waveform": "Noise" }
]
```

| Field      | Type               | Notes |
|------------|--------------------|-------|
| `index`    | integer, `0..=127` | The numbered slot this entry describes. Unique within the array. |
| `waveform` | `ChannelMode` name | Same value set as the top-level `waveform` field: `"Pulse"` \| `"Triangle"` \| `"Noise"` \| `"Vrc6Pulse"` \| `"Vrc6Saw"`. |

This is distinct from the top-level `waveform` field, which is the instrument's
*live* channel selection at save time. On load the two are reconciled: the
loaded `waveform` is also written onto the slot named by `active_indices.vol`,
so the waveform recall that fires on the next index change agrees with the patch
instead of overriding it. This matters most for files written before
`slot_waveforms` existed, whose slots all decode to the default.

Because loading is a full replacement (not a merge), a load clears every slot's
remembered waveform first — otherwise slots the incoming patch doesn't mention
would keep the *previous* instrument's waveforms.

## Full example

An instrument using only a volume envelope (slot 3) and a duty envelope
(also slot 3, since `sequence_number` normally keeps them aligned), with
nothing authored in the other three envelope types. Shown as logical JSON
for readability — see "Wire format" for the actual binary encoding:

```json
{
  "format_version": 1,
  "waveform": "Pulse",
  "step_time_hz": 60,
  "active_indices": {
    "vol": 3,
    "arp": 0,
    "pitch": 0,
    "hipitch": 0,
    "duty": 3
  },
  "sequences": {
    "vol": [
      {
        "index": 3,
        "values": [15, 14, 12, 10, 8],
        "loop_point": 3,
        "release_point": null,
        "pitch_mode": "Relative",
        "arp_mode": "Absolute",
        "vol_mode": "Steps16",
        "enabled": true
      }
    ],
    "arp": [],
    "pitch": [],
    "hipitch": [],
    "duty": [
      {
        "index": 3,
        "values": [0, 2],
        "loop_point": 0,
        "release_point": null,
        "pitch_mode": "Relative",
        "arp_mode": "Absolute",
        "vol_mode": "Steps16",
        "enabled": true
      }
    ]
  },
  "slot_waveforms": []
}
```

`slot_waveforms` is empty here because slot 3 is on `"Pulse"`, the default —
had this instrument been authored on the triangle channel, it would instead
read `[{ "index": 3, "waveform": "Triangle" }]`.

## Extension

`.rp2a03patch` (lowercase). Binary content — despite the extension having no
`.json`/`.bin`-style hint, the leading `RP2P` magic bytes (see "Wire format")
make the file self-identifying regardless of name.

## Invariants a loader should validate

- The first 4 bytes of the file are the ASCII magic `RP2P`; otherwise refuse
  to load with a "not a `.rp2a03patch` file" error rather than attempting to
  decode MessagePack.
- `index` values within a given envelope type's array are unique and in
  `0..=127`.
- `active_indices` values are in `0..=127`.
- `slot_waveforms` `index` values are in `0..=127` and unique across the array.
  A repeated `index` is rejected rather than resolved last-write-wins, matching
  the duplicate rule for sequence entries. Uniqueness also caps an *accepted*
  array at 128 entries, so nothing downstream has to cope with a vec padded out
  with repeats. Note this is not a bound on decode-time memory: a loader that
  validates after deserializing (as the reference implementation does) has
  already materialized the whole array by then, so what actually limits that
  allocation is the file's own size. `MAX_SEQUENCE_LEN` carries the same caveat.
- `step_time_hz` is in `1..=600`.
- `format_version` is a version the loader understands; otherwise refuse to
  load rather than guess.
- Every element of a sequence entry's `values` is in `-128..=127`, matching
  `Sequence::values`' documented range (Dn-FamiTracker stores sequence items
  as `signed char`). A value outside this range is rejected, not clamped —
  consistent with this format's "refuse to load rather than guess" stance.
- `values.len()` does not exceed `MAX_SEQUENCE_LEN` (256, matching the GUI's
  own sequence-length cap). This bounds how large a single sequence a crafted
  file can force the loader to allocate/play.
- `loop_point`/`release_point`, when present, are `<= values.len()` (strictly
  greater than `values.len()` is rejected; a marker positioned exactly at
  `values.len()` is legal — this is the same position the editor itself can
  produce and render).
