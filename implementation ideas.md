FIXED:
```
In famitracker when you first create your instrument all of the envelope editors have their step count at 0, which means they are empty
if the player plays any notes while they are in that state it is handled like this:

- Volume sequence empty/disabled: falls back to volume 15.
- Duty sequence empty/disabled: falls back to duty 0 (12.5% square wave).
- Arpeggio / Pitch / Hi-pitch empty/disabled: fall back to 0

But while those behavior works currently with the code, when i first start the plugin in reaper all of the envelope editors always start with the step count at 1
```


# Plugin States and behavior

Plugin instance first starts
│   ├── All Envelope editors step count starts at 0 with no envelope data
│   ├── Volume sequence first-start/empty/disabled: falls back to volume 15.
│   ├── Duty sequence first-start/empty/disabled: falls back to duty 0 (12.5% square wave)
│   ├── Arpeggio / Pitch / Hi-pitch first-start/empty/disabled: fall back to 0


User interacting with the envelope editors
│   ├── The famitracker source code should be used as a guide/reference on how this system is implemented (IMPORTANT: This should be the first thing to be read/analyzed)
│   ├── User creates custom envelope squences
│   ├── Custom envelopes are saved in memory
│   ├── Each envelope editor can have up to 127 "user-made-sequences"
│   └── User should be able to Switch "user-made-sequences" with a MIDI instrument list or by creating a automation with the Gui's spin-box
│   └── Ways to store 127 "user-made-sequences" in memory safely
│   └── The "user-made-sequences" are exclusively unique to the a instance of the plugin, so multiple plugin instances do not share "user-made-sequences"
│   └── The "Sequence Editor/Instrument Settings" should be coded as a re-usable piece of code because the triangle, noise and expansion chips also makes use of it, though for now we are just focusing on the pulse-channels