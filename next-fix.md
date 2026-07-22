I've identified a new bug and want to implement a major feature for the RP2A03 envelope editor:

**Bug: Sequence minimum length should be 0**
Currently in `editor.rs`, the "Size" controls (-/+) enforce a minimum sequence length of 1 step. In FamiTracker, sequences can be reduced to 0 steps (empty). Fix the behavior:
- Allow the "-" button to reduce the sequence length down to 0, not stopping at 1
- When a sequence reaches length 0, clear the text field entirely (show blank)
- Ensure the bar graph canvas handles empty sequences gracefully (it already has `if num_steps == 0 { return; }` in `widgets.rs` but verify no other edge cases)
- Sequences should never go to negative lengths — 0 is the absolute minimum
- The "+" button should work from 0 to add a default value step (e.g., "0" or "15" depending on the sequence type)

**Major Feature: Click-and-drag envelope drawing and loop/release point manipulation**

Implement interactive envelope editing directly on the bar graph canvas, matching FamiTracker's behavior:

---

### Part 1: Drawing envelope values with mouse drag

**Interaction model:**
- User clicks and holds the left mouse button on the bar graph area
- Dragging horizontally moves across sequence steps; dragging vertically sets the value for each step under the cursor
- The sequence updates in real-time as the user drags (not just on release)
- When the mouse is released, sync the updated `Sequence` back to the corresponding `*_text` field

**Dragging beyond the last step:**
- If the user drags past the right edge of the last step, the cursor remains clamped to the last valid step — it does NOT auto-extend the sequence
- The user must use the "+" button or manually type to add more steps first
- This matches FamiTracker's behavior of only editing existing steps via drag

**Technical requirements:**
- Change the widget's `Sense` from `Sense::hover()` to `Sense::click_and_drag()` to capture drag events
- Convert mouse position to step index and value:
  - Step index = `(mouse_x - graph_rect.min.x) / step_width`, clamped to `[0, num_steps - 1]`
  - Value = `max_val - (mouse_y - graph_rect.min.y) / graph_rect.height() * (max_val - min_val)`, clamped to `[min_val, max_val]`
- Modify `draw_envelope_bar_graph` to accept `seq` as `&mut Sequence` and handle the interaction, or add a separate interaction function
- Track the last modified step to avoid redundant updates when dragging slowly over the same step
- For bipolar sequences (Arpeggio, Pitch, Hi-pitch), consider adding a subtle snap behavior at the zero axis to make it easy to hit the root note exactly

**Text field synchronization:**
- After the user finishes dragging (on `DragReleased`), regenerate the text representation from the updated sequence and write it back to `data.*_text`
- Do NOT update the text field continuously during the drag to avoid visual noise and potential focus issues

---

### Part 2: Setting loop and release points with mouse clicks

**Interaction model (matching FamiTracker):**
- **Left-click in the header area** (the bottom strip where "Loop" and "Release" labels currently appear): sets the **loop point** at the clicked step position
- **Right-click in the header area**: sets the **release point** at the clicked step position
- If the user drags the loop and release points close together (adjacent or overlapping), they merge into a combined **"Loop, Release" mode** where both markers share the same step — this already has visual support in `widgets.rs` with the gold-colored header and "Loop, Release" label
- Dragging them apart again should split them back into separate loop and release regions

**Technical requirements:**
- Detect which step column the user clicked in the header area by dividing the mouse X position by `step_width`
- Left-click updates `seq.loop_point = Some(clicked_step_index)` 
- Right-click updates `seq.release_point = Some(clicked_step_index)`
- When both points are set to the same index, the existing combined "Loop, Release" rendering logic in `widgets.rs` handles the visual automatically
- Consider adding a way to clear a point (e.g., clicking the same position again, or a separate "clear" interaction) — FamiTracker allows right-clicking an existing release point to remove it
- The header area should have its own `Sense::click()` to distinguish from the bar graph drawing area

**Synchronization:**
- When loop/release points are modified, regenerate the text representation just like with value changes, so the text field reflects any loop/release syntax changes

---

### Additional considerations:

- The bar graph area and header area need separate interaction handling (drag-draw on bars, click-set-markers on header)
- The combined "Loop, Release" mode already has rendering support in the existing `is_loop_release_mode` logic — ensure setting both points to the same index triggers this code path correctly
- Both features (value drawing and marker setting) should work together seamlessly — drawing values should not accidentally modify loop/release points, and vice versa
- Test edge cases: clicking in the header area directly above a step column edge, setting loop/release on very short sequences (1-2 steps), and clearing markers when a sequence has only 1 step