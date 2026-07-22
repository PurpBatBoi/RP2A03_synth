I've identified several UX bugs in my RP2A03 synth editor that need fixing:

Bug 1: Text input validation and persistence issue
When typing invalid characters into the sequence text fields, the keyboard input sometimes locks up. The current behavior allows users to insert letters into the number fields, which causes problems. Fix this by:

    When the user presses Enter, automatically strip/remove any invalid non-numeric characters from the text field (except | and / since they are used for loops)

    When the user switches to a different editor tab, discard any invalid input and revert to the last valid sequence state

    Ensure the text field always shows the cleaned-up values after these actions

Bug 2: Missing zero-axis reference line in Arpeggio editor
In the arpeggio sequence editor, when a value is 0, there's no visual indicator line at the center/zero position to represent the "root" note. The bar graph should draw a horizontal line at the zero axis position (similar to how it's done for other bipolar sequences) so users can clearly see the root note reference.

Bug 3: Default initial values should show empty fields
When the editor/plugin first starts up, the sequence text fields should appear blank/empty instead of pre-filled with default values. The sequences should still function correctly with default playback behavior (e.g., volume at 15, duty at 0), but the text fields in the UI should be empty on initial load.

Bug 4: Keyboard input lockup
Investigate and fix the root cause of keyboard input locking up when typing invalid characters into the sequence fields. This might be related to how TextEdit::changed() and has_focus() interact with the re-parsing logic.

Bug 5: Cross-tab validation
Ensure that when switching between sequence tabs (Volume, Arpeggio, Pitch, Hi-pitch, Duty/Nosie), any pending invalid input in the previous tab's text field is properly cleaned up rather than persisting.