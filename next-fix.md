it did compile but when i opened it all editors always started with the step size set to 1 automatically, i suspect this has something to do with how when i switch between editors it resets to a "valid' value, as we implemented ealier, but this
```
Volume sequence empty/disabled: falls back to volume 15.
Duty sequence empty/disabled: falls back to duty 0 (12.5% square wave).
Arpeggio / Pitch / Hi-pitch empty/disabled: fall back to 0
```
is to handle when the plugin open for the first time, since there is no user-defined envelopes yet

i did implement a fix but it didn't work it seems i want you to check and implement the behavior as seen in famitracker source code

when i run famitracker this seems to be the initial state of the apu according to the registers
```
2A03 registers
$4000: $30 $00 $00 $00    pitch = $000 ( 0.00Hz --- +00), vol = 00, duty = 0
$4004: $30 $00 $00 $00    pitch = $000 ( 0.00Hz --- +00), vol = 00, duty = 0
$4008: $00 $00 $00 $00    pitch = $000 ( 0.00Hz --- +00)
$400C: $30 $00 $00 $00    rate = $0 ( 0.00Hz ), vol = 00, mode = 0
$4010: $00 $00 $00 $00    rate = $0 ( 0.00Hz ), once, size = 1    byte
    position: 00, delta = $00
```

key info is the duty, which is 0, volume is zero too but thats because it only reacts when i play a note then it uses the value 15 for the volume

when i create a instrument all of the editors are toggled OFF, and the "size" is set 0 zero, thats the default values in famitracker from looking at the ui directly






In famitracker when you first create your instrument all of the envelope editors have their step count at 0, which means they are empty
if the player plays any notes while they are in that state it is handled like this:

- Volume sequence empty/disabled: falls back to volume 15.
- Duty sequence empty/disabled: falls back to duty 0 (12.5% square wave).
- Arpeggio / Pitch / Hi-pitch empty/disabled: fall back to 0

But while those behavior works currently with the code, when i first start the plugin in reaper all of the envelope editors always start with the step count at 1



Plugin first starts
│   ├── All Envelope editors step count starts at 0 with no envelope data
│   ├── Volume sequence first-start/empty/disabled: falls back to volume 15.
│   ├── Duty sequence first-start/empty/disabled: falls back to duty 0 (12.5% square wave)
│   ├── Arpeggio / Pitch / Hi-pitch first-start/empty/disabled: fall back to 0


User interacting with the envelope editors
│   ├── User creates custom envelope squences
│   ├── Custom envelopes are saved in memory
│   ├── Each envelope editor can have up to 127 "sequences"
│   └── User should be able to Switch "sequences" with a MIDI instrument list or by creating a automation with the Gui's spin-box
│   └── Ways to store 127 "sequences" in memory safely
│   └── the famitracker source code should be used as a guide/reference on how this system is implemented