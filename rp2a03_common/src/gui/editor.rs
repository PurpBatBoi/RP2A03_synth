//! rp2a03_common\src\gui\editor.rs
//! Layout rendering logic for the reusable sequence editor window.

use super::state::{MAX_SEQUENCES, SequencePlayheads, SharedSequences};
use super::widgets::{
    draw_envelope_bar_graph, draw_s5b_duty_noise_graph, group_box, repeating_button,
};
use crate::ChannelMode;
use rp2a03_core::sequencer::{ArpMode, PitchMode, Sequence, VolMode, VolMode5B};

/// Result returned by [`render_editor_ui`] to communicate parameter changes back
/// to the host plugin wrapper.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditorResult {
    /// If the user changed the shared sequence index via the GUI.
    pub new_sequence_index: Option<usize>,
    /// If the user changed the step time Hz value via the GUI.
    pub new_step_time_hz: Option<i32>,
    /// If the user changed the channel mode via the waveform combobox.
    pub new_channel_mode: Option<ChannelMode>,
    /// If the user changed the polyphony toggle.
    pub new_polyphony: Option<bool>,
    /// If the user changed the maximum voice count.
    pub new_max_voices: Option<i32>,
    pub new_portamento_enabled: Option<bool>,
    pub new_portamento_speed: Option<i32>,
}

/// How long a footer status message stays visible after being set.
const STATUS_DISPLAY_DURATION: std::time::Duration = std::time::Duration::from_secs(4);

/// Transient, UI-only editor state: display state such as the footer status
/// message after a Save/Load, plus frame-to-frame bookkeeping that has no other
/// home. Owned by the editor alone — nothing here is read by the audio thread,
/// and it is neither [`SharedSequences`] (audio-relevant state) nor
/// [`EditorResult`] (host-parameter signals).
#[derive(Default)]
pub struct EditorUiState {
    status: Option<(String, std::time::Instant)>,
    /// The Sequence Index seen on the previous `render_editor_ui` call, so a
    /// real cross-frame transition can be told apart from `shared_sequence_index`
    /// merely agreeing with `data` — which it always does within one call, since
    /// `shared_sequence_index` *is* `data.selected_sequence_index(0)` read one
    /// call earlier by the plugin's editor. `None` only before the first frame,
    /// so that frame cannot misfire a recall.
    last_sequence_index: Option<usize>,
    /// Set while a Save dialog is showing on its own thread; see
    /// `handle_save_click`'s doc comment for why this can't run on the GUI
    /// thread directly. Polled once per frame in `poll_pending_dialogs`.
    pending_save: Option<std::sync::mpsc::Receiver<Option<String>>>,
    pending_load: Option<std::sync::mpsc::Receiver<Option<std::path::PathBuf>>>,
}

impl EditorUiState {
    /// Sets the footer status message, restarting its display timer.
    pub fn set(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), std::time::Instant::now()));
    }

    /// Returns the status message if one is set and still within its display
    /// window; `None` otherwise (including once it has expired).
    pub fn status_text(&self) -> Option<&str> {
        self.status
            .as_ref()
            .filter(|(_, set_at)| set_at.elapsed() < STATUS_DISPLAY_DURATION)
            .map(|(msg, _)| msg.as_str())
    }

    /// Returns the status message's opacity, starting at 50% and fading
    /// linearly to 0 over its display window.
    pub fn status_opacity(&self) -> Option<f32> {
        self.status.as_ref().and_then(|(_, set_at)| {
            let elapsed = set_at.elapsed();
            (elapsed < STATUS_DISPLAY_DURATION).then(|| {
                let remaining = 1.0 - elapsed.as_secs_f32() / STATUS_DISPLAY_DURATION.as_secs_f32();
                0.5 * remaining
            })
        })
    }
}

/// Converts a Sequence engine instance back to FamiTracker formatted text.
pub fn sequence_to_text(seq: &Sequence) -> String {
    if seq.values.is_empty() {
        return String::new();
    }

    let mut tokens = Vec::with_capacity(seq.values.len() * 2);
    for (i, value) in seq.values.iter().enumerate() {
        if seq.loop_point == Some(i) {
            tokens.push("|".to_string());
        }
        if seq.release_point == Some(i) {
            tokens.push("/".to_string());
        }
        tokens.push(value.to_string());
    }
    if seq.loop_point == Some(seq.values.len()) {
        tokens.push("|".to_string());
    }
    if seq.release_point == Some(seq.values.len()) {
        tokens.push("/".to_string());
    }
    tokens.join(" ")
}

/// Same as [`sequence_to_text`], but uses the S5B-specific MML-style duty/mode
/// text form (e.g. `"5tne"`: period 5, tone+noise+envelope) when `tab == 4`
/// and `channel_mode == ChannelMode::S5B`. Every other tab/channel combination
/// falls through to the plain numeric form.
pub fn sequence_to_text_for_tab(tab: usize, channel_mode: ChannelMode, seq: &Sequence) -> String {
    if tab != 4 || channel_mode != ChannelMode::S5B {
        return sequence_to_text(seq);
    }

    if seq.values.is_empty() {
        return String::new();
    }

    let mut tokens = Vec::with_capacity(seq.values.len() * 2);
    for (i, value) in seq.values.iter().enumerate() {
        if seq.loop_point == Some(i) {
            tokens.push("|".to_string());
        }
        if seq.release_point == Some(i) {
            tokens.push("/".to_string());
        }

        let period = value & rp2a03_core::sequencer::S5B_PERIOD_MASK;
        let mut token = period.to_string();
        if value & rp2a03_core::sequencer::S5B_MODE_SQUARE != 0 {
            token.push('t');
        }
        if value & rp2a03_core::sequencer::S5B_MODE_NOISE != 0 {
            token.push('n');
        }
        // Duty width is only spelled out when it differs from the 50%
        // default, so sequences authored before the field existed (and any
        // that simply never touch it) keep their original text form.
        let duty = rp2a03_core::sequencer::s5b_duty_index(*value);
        if duty != rp2a03_core::sequencer::S5B_DUTY_DEFAULT_INDEX {
            token.push('w');
            token.push_str(&duty.to_string());
        }
        tokens.push(token);
    }
    if seq.loop_point == Some(seq.values.len()) {
        tokens.push("|".to_string());
    }
    if seq.release_point == Some(seq.values.len()) {
        tokens.push("/".to_string());
    }
    tokens.join(" ")
}

/// Strips non-sequence characters from raw text input.
///
/// `t`/`n`/`w` (case-insensitive) are S5B duty/mode letters (see
/// [`sequence_to_text_for_tab`]); harmless to allow through unconditionally
/// since plain-numeric parsing (`Sequence::parse_clamped`) already ignores any
/// token that doesn't parse as an integer. dn's third letter `e` (hardware
/// envelope) is deliberately not accepted — this synth never selects the
/// envelope, so letting it through would set a bit nothing reads.
pub fn sanitize_sequence_text(text: &str) -> String {
    text.chars()
        .filter(|c| {
            c.is_ascii_digit()
                || matches!(*c, '|' | '/' | '-')
                || matches!(c.to_ascii_lowercase(), 't' | 'n' | 'w')
                || c.is_ascii_whitespace()
        })
        .collect()
}

/// Parses an S5B duty/mode sequence text string (e.g. `"5tn"`) into a
/// `Sequence`, mirroring dn's `CSeqConversion5B::ToValue`
/// (`SequenceParser.cpp`). Each token is a period (0..=31) followed by
/// optional flag letters `t`/`n` and an optional `w<digit>` duty width
/// (case-insensitive, any order); malformed
/// or unrecognized flag letters are ignored rather than rejected, matching
/// dn's permissive `[TtNnEe]*` regex — text-field edits happen
/// keystroke-by-keystroke and can't hard-error mid-edit. dn's `e` (hardware
/// envelope) falls into that ignored set here: this synth has no envelope
/// period/shape control, so the flag is not offered on either surface.
fn parse_s5b_duty_text(input: &str) -> (Sequence, String) {
    let mut values = Vec::new();
    let mut loop_point = None;
    let mut release_point = None;
    let mut text_tokens = Vec::new();

    for token in input.split_whitespace() {
        match token {
            "|" | "L" | "l" => {
                loop_point = Some(values.len());
                text_tokens.push("|".to_string());
            }
            "/" | "R" | "r" => {
                release_point = Some(values.len());
                text_tokens.push("/".to_string());
            }
            _ => {
                let digits_end = token
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(token.len());
                let (digits, flags) = token.split_at(digits_end);
                if digits.is_empty() {
                    continue;
                }
                let Ok(period) = digits.parse::<i16>() else {
                    continue;
                };
                let period = period.clamp(0, 31);

                let mut value = period;
                let mut out_token = period.to_string();
                if flags.contains(['t', 'T']) {
                    value |= rp2a03_core::sequencer::S5B_MODE_SQUARE;
                    out_token.push('t');
                }
                if flags.contains(['n', 'N']) {
                    value |= rp2a03_core::sequencer::S5B_MODE_NOISE;
                    out_token.push('n');
                }
                // `w<digit>` is the duty width (0..=8). A bare `w`, or one
                // followed by something that isn't a digit, is dropped like
                // any other unrecognized letter rather than rejecting the
                // token mid-keystroke.
                if let Some(rest) = flags
                    .split(['w', 'W'])
                    .nth(1)
                    .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
                {
                    let digits_end = rest
                        .find(|c: char| !c.is_ascii_digit())
                        .unwrap_or(rest.len());
                    if let Ok(duty) = rest[..digits_end].parse::<i16>() {
                        let duty = duty.clamp(0, 8);
                        value = rp2a03_core::sequencer::s5b_set_duty_index(value, duty);
                        if duty != rp2a03_core::sequencer::S5B_DUTY_DEFAULT_INDEX {
                            out_token.push('w');
                            out_token.push_str(&duty.to_string());
                        }
                    }
                }

                values.push(value);
                text_tokens.push(out_token);
            }
        }
    }

    if values.is_empty() {
        values.push(0);
        text_tokens.push("0".to_string());
    }

    let normalized_text = text_tokens.join(" ");
    let sequence = Sequence {
        values,
        loop_point,
        release_point,
        ..Sequence::default()
    };
    (sequence, normalized_text)
}

/// dnFamiTracker keeps all sequence items as `signed char`; the editor clamps each
/// envelope type to its documented range. Pitch and hi-pitch share one graph editor
/// (`CPitchGraphEditor`) clamped to [-128, 127] (GraphEditor.cpp: DrawRange(127, -128)).
///
/// `vol_mode` only matters for tab 0 on the VRC6 sawtooth: dn stores the
/// 16-step/64-step setting per volume sequence (`CSequence::GetSetting`), and the
/// bar graph max follows it (SequenceEditor.cpp: `0x0F` vs `0x3F`).
///
/// `vol_mode_5b` is the same idea scoped to `ChannelMode::S5B`'s duty/mode
/// sequence, which is a 16-step/32-step split instead of 16/64 — the S5B's
/// hardware volume register is 5-bit, not 6-bit. See UI.md §1 for why this is
/// a separate enum from `VolMode` rather than a reused one.
fn sequence_range(
    tab: usize,
    channel_mode: ChannelMode,
    vol_mode: VolMode,
    vol_mode_5b: VolMode5B,
) -> (i16, i16) {
    match tab {
        0 => {
            if channel_mode == ChannelMode::Vrc6Saw && vol_mode == VolMode::Steps64 {
                (0, 63)
            } else if channel_mode == ChannelMode::S5B && vol_mode_5b == VolMode5B::Steps32 {
                (0, 31)
            } else {
                (0, 15)
            }
        }
        1 => (-96, 96),
        2 | 3 => (-128, 127),
        _ => match channel_mode {
            ChannelMode::Vrc6Pulse => (0, 7),
            // dn `CVRC6Sawtooth::MAX_DUTY = 0x01` — the saw's duty is a single bit
            // that becomes bit 5 of the $B000 accumulator rate.
            ChannelMode::Vrc6Saw => (0, 1),
            // Noise period is the graph's Y-axis; mode flags (envelope/tone/
            // noise) are drawn separately by `draw_s5b_duty_noise_graph`.
            ChannelMode::S5B => (0, 31),
            _ => (0, 3),
        },
    }
}

/// Switches a volume sequence between the 4-bit and 6-bit step ranges.
///
/// dn remaps the existing steps when the setting flips (`SequenceSetting.cpp`:
/// `x * 4` / `x / 4`) so the envelope keeps its shape instead of flattening
/// against the new ceiling. Halving is lossy — the low two bits are gone for good.
fn set_volume_step_mode(text: &mut String, sequence: &mut Sequence, next: VolMode) {
    if sequence.vol_mode == next {
        return;
    }

    let scale_up = next == VolMode::Steps64;
    let (min_val, max_val) = sequence_range(0, ChannelMode::Vrc6Saw, next, sequence.vol_mode_5b);

    for value in &mut sequence.values {
        *value = if scale_up { *value * 4 } else { *value / 4 };
        *value = (*value).clamp(min_val, max_val);
    }

    sequence.vol_mode = next;
    *text = sequence_to_text(sequence);
}

/// Switches an S5B duty/mode-as-volume sequence between the 4-bit and 5-bit
/// step ranges. Same shape as [`set_volume_step_mode`], but the S5B's
/// hardware volume register is 5-bit, so the scale factor is `*2`/`/2`
/// (16<->32) instead of `*4`/`/4` (16<->64).
fn set_volume_step_mode_5b(text: &mut String, sequence: &mut Sequence, next: VolMode5B) {
    if sequence.vol_mode_5b == next {
        return;
    }

    let scale_up = next == VolMode5B::Steps32;
    let (min_val, max_val) = sequence_range(0, ChannelMode::S5B, sequence.vol_mode, next);

    for value in &mut sequence.values {
        *value = if scale_up { *value * 2 } else { *value / 2 };
        *value = (*value).clamp(min_val, max_val);
    }

    sequence.vol_mode_5b = next;
    *text = sequence_to_text(sequence);
}

/// Brings the selected volume sequence back to the 4-bit range once the editor is
/// no longer on the VRC6 sawtooth.
///
/// 64-step is a saw-only setting, so leaving that waveform has to behave like
/// unticking the toggle — otherwise the stored steps keep their 0..63 values while
/// every other channel reads them through a 0..15 clamp, and the text box disagrees
/// with the graph. Idempotent, so it is safe to run every frame and it catches host
/// automation and state recall as well as the waveform combobox.
fn sync_volume_step_mode_to_channel(data: &mut SharedSequences) {
    if data.channel_mode == ChannelMode::Vrc6Saw {
        return;
    }

    let (text, sequence) = data.selected_sequence_mut(0);
    set_volume_step_mode(text, sequence, VolMode::Steps16);
}

/// Same idea as [`sync_volume_step_mode_to_channel`], but for the S5B's
/// 16/32-step volume setting: brings the selected volume sequence back to the
/// 4-bit range once the editor is no longer on `ChannelMode::S5B`.
fn sync_volume_step_mode_5b_to_channel(data: &mut SharedSequences) {
    if data.channel_mode == ChannelMode::S5B {
        return;
    }

    let (text, sequence) = data.selected_sequence_mut(0);
    set_volume_step_mode_5b(text, sequence, VolMode5B::Steps16);
}

/// Sanitizes the selected numbered sequence for an envelope type.
pub fn cleanup_tab_sequence(data: &mut SharedSequences, tab: usize) {
    let (sanitized, prev_pitch_mode, prev_arp_mode, prev_vol_mode, prev_vol_mode_5b) = {
        let (text, sequence) = data.selected_sequence_mut(tab);
        (
            sanitize_sequence_text(text),
            sequence.pitch_mode,
            sequence.arp_mode,
            sequence.vol_mode,
            sequence.vol_mode_5b,
        )
    };
    let channel_mode = data.channel_mode;
    let (min_val, max_val) = sequence_range(tab, channel_mode, prev_vol_mode, prev_vol_mode_5b);
    let (text, sequence) = data.selected_sequence_mut(tab);

    if sanitized.trim().is_empty() {
        *sequence = Sequence::default();
        sequence.pitch_mode = prev_pitch_mode;
        sequence.arp_mode = prev_arp_mode;
        sequence.vol_mode = prev_vol_mode;
        sequence.vol_mode_5b = prev_vol_mode_5b;
        text.clear();
    } else {
        let (mut parsed, normalized) = if tab == 4 && channel_mode == ChannelMode::S5B {
            parse_s5b_duty_text(&sanitized)
        } else {
            Sequence::parse_clamped(&sanitized, min_val, max_val)
        };
        parsed.pitch_mode = prev_pitch_mode;
        parsed.arp_mode = prev_arp_mode;
        parsed.vol_mode = prev_vol_mode;
        parsed.vol_mode_5b = prev_vol_mode_5b;
        let len = parsed.len();
        if parsed.loop_point.is_some_and(|point| point >= len) {
            parsed.loop_point = None;
        }
        if parsed.release_point.is_some_and(|point| point >= len) {
            parsed.release_point = None;
        }
        *sequence = parsed;
        *text = normalized;
    }
}

/// Whether an envelope tab means anything for `channel_mode`.
///
/// The instrument settings panel greys out unavailable tabs; `.rp2a03patch`
/// load reads the same rule to move off a tab the loaded waveform doesn't
/// support, so the two can't drift apart. Triangle and noise have no duty
/// envelope, and noise has no pitch/hi-pitch.
fn tab_is_available(tab: usize, channel_mode: ChannelMode) -> bool {
    let is_no_duty = tab == 4 && matches!(channel_mode, ChannelMode::Triangle | ChannelMode::Noise);
    let is_noise_pitch = matches!(tab, 2 | 3) && channel_mode == ChannelMode::Noise;
    !is_no_duty && !is_noise_pitch
}

/// Opens the Save dialog and writes the file on a dedicated OS thread.
///
/// `rfd`'s blocking dialog runs its own nested Win32 message loop. If that
/// loop runs on the same thread as the editor's baseview window, Windows
/// dispatches cross-window messages back into baseview's WndProc while it is
/// already mid-callback, and baseview's window state (a `RefCell`) is not
/// reentrant — the nested call panics with "already borrowed", which nothing
/// upstream catches, so the panic reaches the top of the thread and aborts
/// the whole host process. Spawning a plain thread with no baseview window on
/// it sidesteps this entirely: that thread's message queue only ever serves
/// the dialog's own windows. This is a bare `std::thread`, not an async
/// runtime — the "no async runtime" constraint from this feature's spec is
/// about not pulling in an executor, not about avoiding threads.
fn handle_save_click(data: &SharedSequences, step_time_hz: u32, ui_state: &mut EditorUiState) {
    if ui_state.pending_save.is_some() {
        return;
    }

    let patch = crate::Patch::from_shared_sequences(data, step_time_hz as u16);
    let (tx, rx) = std::sync::mpsc::channel();
    ui_state.pending_save = Some(rx);

    std::thread::spawn(move || {
        let status = save_dialog_and_write(&patch);
        let _ = tx.send(status);
    });
}

/// Returns `None` when the user cancels the dialog (no status to report).
fn save_dialog_and_write(patch: &crate::Patch) -> Option<String> {
    let mut path = rfd::FileDialog::new()
        .add_filter("RP2A03 Patch", &["rp2a03patch"])
        .set_file_name("User_Instrument.rp2a03patch")
        .save_file()?;

    if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rp2a03patch") {
        let mut file_name = path.into_os_string();
        file_name.push(".rp2a03patch");
        path = file_name.into();
    }

    Some(match crate::save_patch_to_path(&path, patch) {
        Ok(()) => format!(
            "{} saved",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        ),
        Err(e) => e.to_string(),
    })
}

/// Opens the Load dialog on a dedicated OS thread — see `handle_save_click`'s
/// doc comment for why the dialog itself can never run on the GUI thread.
/// The picked path (if any) is applied later, from `poll_pending_dialogs`,
/// since that needs `&mut SharedSequences`/`&mut EditorResult` from whatever
/// frame the pick actually lands on.
fn handle_load_click(ui_state: &mut EditorUiState) {
    if ui_state.pending_load.is_some() {
        return;
    }

    let (tx, rx) = std::sync::mpsc::channel();
    ui_state.pending_load = Some(rx);

    std::thread::spawn(move || {
        let path = rfd::FileDialog::new()
            .add_filter("RP2A03 Patch", &["rp2a03patch"])
            .pick_file();
        let _ = tx.send(path);
    });
}

/// Applies whichever background dialog (Save, Load, or neither) has finished
/// since the last frame. Must run every frame regardless of whether a dialog
/// is pending — that's what actually advances a finished one.
fn poll_pending_dialogs(
    data: &mut SharedSequences,
    result: &mut EditorResult,
    ui_state: &mut EditorUiState,
) {
    use std::sync::mpsc::TryRecvError;

    if let Some(rx) = &ui_state.pending_save {
        match rx.try_recv() {
            Ok(status) => {
                ui_state.pending_save = None;
                if let Some(status) = status {
                    ui_state.set(status);
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => ui_state.pending_save = None,
        }
    }

    if let Some(rx) = &ui_state.pending_load {
        match rx.try_recv() {
            Ok(Some(path)) => {
                ui_state.pending_load = None;
                // Nothing has been written yet on `Err`, so a rejected file
                // leaves the current instrument exactly as it was — never a
                // partial application.
                match crate::load_patch_from_path(&path) {
                    Ok(patch) => {
                        apply_loaded_patch(data, result, &patch);
                        let file_name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string());
                        ui_state.set(format!("{file_name} loaded"));
                    }
                    Err(e) => ui_state.set(e.to_string()),
                }
            }
            Ok(None) => ui_state.pending_load = None,
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => ui_state.pending_load = None,
        }
    }
}

/// Installs a decoded patch into the live editor state.
///
/// Split out from the dialog half of Load so the ordering below is reachable
/// from tests — the step order here is load-bearing, not stylistic.
fn apply_loaded_patch(data: &mut SharedSequences, result: &mut EditorResult, patch: &crate::Patch) {
    // The instrument's live channel isn't a separate field on `Patch` — it's
    // whichever waveform `active_indices.vol`'s slot remembers (see
    // `Patch::active_waveform`), same as switching to that slot by hand would
    // recall. Reading it here, before `apply_to_shared_sequences` below,
    // means it comes straight from the patch's own data rather than from
    // `data`, which the next line is about to overwrite anyway.
    let waveform = patch.active_waveform();

    // `channel_mode` must be updated BEFORE the sequences land.
    // `sync_volume_step_mode_to_channel` runs unconditionally at the top of
    // every frame, and rescales tab 0's volume sequence back to the 4-bit
    // range whenever the channel is not the VRC6 saw. Writing the sequences
    // first would leave a freshly loaded saw patch's 64-step volume data
    // sitting under the previous channel mode, and the next frame's sync
    // would silently clamp it away. Setting the mode here means that sync
    // sees the loaded channel and takes its early return instead.
    data.channel_mode = waveform;
    result.new_channel_mode = Some(waveform);

    // This also settles `active_indices.vol`'s slot to `waveform` (or leaves
    // it at the default, which is where `waveform` just came from), so
    // `data.channel_mode` and `data.slot_waveform(patch.active_indices.vol)`
    // already agree — no separate reconciliation step needed.
    patch.apply_to_shared_sequences(data);

    // The loaded waveform may not have the tab the editor is sitting on.
    if !tab_is_available(data.selected_tab, data.channel_mode) {
        data.selected_tab = 0;
    }

    result.new_step_time_hz = Some(patch.step_time_hz as i32);
    // The slot selection has to travel out as a parameter gesture too, not
    // just into `data`: the audio thread re-asserts `sequence_number` over
    // every envelope's selected index on each `process` block
    // (`SequenceCache::refresh`), so leaving the parameter stale would slam
    // the freshly loaded selection back to the old slot within one block.
    // A `Patch` carries five independent indices but this build only has the
    // one shared parameter, so tab 0's is the one that can be honored — every
    // file this build writes has all five equal anyway, since the plugin
    // keeps them in lockstep.
    result.new_sequence_index = Some(patch.active_indices.vol);
}

/// Switches the live channel mode, cleaning up any tab the new waveform does
/// not support, and records the change as a host-parameter gesture.
///
/// Shared by the Waveform combo box and the per-slot waveform recall that runs
/// when the Sequence Index changes, so both paths do the same tab cleanup and
/// both push the same `new_channel_mode` gesture — `SequenceCache::refresh`
/// re-asserts `params.waveform` every audio block, so a mode change that skipped
/// the gesture would be stomped back within one block.
fn apply_channel_mode_change(
    data: &mut SharedSequences,
    new_mode: ChannelMode,
    result: &mut EditorResult,
) {
    match new_mode {
        // If Duty tab was active, revert to Volume
        ChannelMode::Triangle => {
            if data.selected_tab == 4 {
                cleanup_tab_sequence(data, 4);
                data.selected_tab = 0;
            }
        }
        // Fine pitch is not an NES noise-channel control. Keep the editor on a
        // supported tab when switching into Noise.
        ChannelMode::Noise => {
            if matches!(data.selected_tab, 2..=4) {
                cleanup_tab_sequence(data, data.selected_tab);
                data.selected_tab = 0;
            }
        }
        // The saw keeps its Duty tab: in 16-step mode duty bit 0 is the $B000
        // rate MSB, so tab 4 stays selectable. Pulse, VRC6 pulse, and S5B
        // support every tab, so none of them need cleanup either.
        ChannelMode::Pulse | ChannelMode::Vrc6Pulse | ChannelMode::Vrc6Saw | ChannelMode::S5B => {}
    }

    data.channel_mode = new_mode;
    result.new_channel_mode = Some(new_mode);
}

/// Switches the live waveform to the one `shared_sequence_index`'s slot
/// remembers, but only when the index actually changed since the previous
/// frame. `last_index` is that previous frame's value, updated here.
///
/// Called from `render_editor_ui` immediately after
/// `set_all_selected_sequence_indices`, the one point every path that can change
/// the index converges on: the GUI drag box, host automation, and MIDI Program
/// Change. The latter two are already written into `SharedSequences` by
/// `SequenceCache::refresh` on the audio thread before this frame runs, which is
/// also why the transition can't be detected by comparing against `data` —
/// `shared_sequence_index` *is* `data.selected_sequence_index(0)`, so within one
/// call the two always agree and the previous frame's value has to be kept here.
///
/// Gating on a real transition is load-bearing, not defensive style:
/// `apply_loaded_patch` runs *mid-frame*, from `poll_pending_dialogs` inside
/// `draw_header`, and sets `channel_mode` to the loaded patch's waveform while
/// this frame's `shared_sequence_index` is still the **old** slot. An
/// unconditional compare-and-apply would read `slot_waveform(old_index)` and
/// clobber the waveform that was just loaded. The gate is what lets a load
/// survive to the next frame, where the index has caught up.
///
/// (Until the Waveform parameter was hidden, this gate also kept the recall
/// from fighting direct host automation of it. That writer no longer exists —
/// see the comment on `waveform` in `rp2a03_niceplug/src/params.rs` — but the
/// load path above keeps the gate mandatory.)
///
/// A `None` `last_index` (before the first frame) deliberately recalls nothing,
/// so the first frame can't misfire over a restored project's waveform.
fn recall_slot_waveform(
    data: &mut SharedSequences,
    shared_sequence_index: usize,
    last_index: &mut Option<usize>,
    result: &mut EditorResult,
) {
    if last_index.is_some_and(|prev| prev != shared_sequence_index) {
        let target_mode = data.slot_waveform(shared_sequence_index);
        if target_mode != data.channel_mode {
            apply_channel_mode_change(data, target_mode, result);
        }
    }
    *last_index = Some(shared_sequence_index);
}

fn draw_header(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    result: &mut EditorResult,
    ui_state: &mut EditorUiState,
    step_time_hz: u32,
    shared_sequence_index: usize,
) {
    poll_pending_dialogs(data, result, ui_state);

    let mut portamento = data.portamento_enabled;

    const HEADER_H: f32 = 92.0;
    const LOGO_W: f32 = 312.0;
    const LOGO_H: f32 = 92.0;
    const CENTER_Y: f32 = LOGO_H / 2.0; // 46.0 — everything centers on this

    ui.allocate_ui(egui::vec2(ui.available_width(), HEADER_H), |ui| {
        let origin = ui.min_rect().min;

        //------------------------------------------------------
        // Logo
        //------------------------------------------------------

        ui.scope_builder(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                origin,
                egui::vec2(LOGO_W, LOGO_H),
            )),
            |ui| {
                ui.add(
                    egui::Image::new(egui::include_image!("logo.png"))
                        .fit_to_exact_size(egui::vec2(LOGO_W, LOGO_H))
                        .tint(egui::Color32::from_white_alpha(128)),
                );
            },
        );

        //------------------------------------------------------
        // Layout constants
        //------------------------------------------------------

        let controls_x = LOGO_W + 28.0;

        // Dropdown + checkbox row treated as ONE block, then that
        // block is centered on CENTER_Y.
        const COMBO_H: f32 = 24.0;
        const CHECK_H: f32 = 22.0;
        const ROW_GAP: f32 = 6.0;
        const BLOCK_H: f32 = COMBO_H + ROW_GAP + CHECK_H;

        let block_top = CENTER_Y - BLOCK_H / 2.0;
        let combo_y = block_top;
        let check_y = block_top + COMBO_H + ROW_GAP;

        //------------------------------------------------------
        // Dropdown
        //------------------------------------------------------

        ui.scope_builder(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::pos2(origin.x + controls_x, origin.y + combo_y),
                egui::vec2(190.0, COMBO_H),
            )),
            |ui| {
                let mut waveform_id = data.channel_mode as i32;
                egui::ComboBox::from_id_salt("waveform")
                    .width(180.0)
                    .selected_text(match data.channel_mode {
                        ChannelMode::Pulse => "2A03 | Pulse",
                        ChannelMode::Triangle => "2A03 | Triangle",
                        ChannelMode::Noise => "2A03 | Noise",
                        ChannelMode::Vrc6Pulse => "VRC6 | Pulse",
                        ChannelMode::Vrc6Saw => "VRC6 | Saw",
                        ChannelMode::S5B => "S5B | PSG",
                    })
                    .show_ui(ui, |ui| {
                        const WAVEFORMS: [(i32, ChannelMode, &str); 6] = [
                            (0, ChannelMode::Pulse, "2A03 | Pulse"),
                            (1, ChannelMode::Triangle, "2A03 | Triangle"),
                            (2, ChannelMode::Noise, "2A03 | Noise"),
                            (3, ChannelMode::Vrc6Pulse, "VRC6 | Pulse"),
                            (4, ChannelMode::Vrc6Saw, "VRC6 | Saw"),
                            (5, ChannelMode::S5B, "S5B | PSG"),
                        ];
                        for (id, new_mode, label) in WAVEFORMS {
                            if ui.selectable_value(&mut waveform_id, id, label).clicked() {
                                apply_channel_mode_change(data, new_mode, result);
                                // Picking a waveform by hand writes through to the
                                // selected slot, so returning to this slot later
                                // recalls it.
                                data.set_slot_waveform(shared_sequence_index, new_mode);
                            }
                        }
                    });
            },
        );

        //------------------------------------------------------
        // Checkboxes
        //------------------------------------------------------

        // Capture the row's response so we can measure its actual width,
        // rather than guessing a constant.
        let check_row = ui.scope_builder(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::pos2(origin.x + controls_x, origin.y + check_y),
                egui::vec2(340.0, CHECK_H),
            )),
            |ui| {
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!data.portamento_enabled, |ui| {
                        if ui.checkbox(&mut data.polyphony, "Polyphony").changed() {
                            result.new_polyphony = Some(data.polyphony);
                        }
                    });
                    if ui.checkbox(&mut portamento, "Portamento").changed() {
                        data.portamento_enabled = portamento;
                        result.new_portamento_enabled = Some(portamento);
                    }
                })
                .response
            },
        );

        let _check_row_w = check_row.inner.rect.width();

        //------------------------------------------------------
        // Save / Load
        //------------------------------------------------------

        const BUTTON_W: f32 = 64.0;
        const BUTTON_H: f32 = 24.0;
        const BUTTON_GAP: f32 = 8.0;
        let block_w = BUTTON_W * 2.0 + BUTTON_GAP;
        let block_x = origin.x + ui.available_width() - block_w;

        ui.scope_builder(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::pos2(block_x, origin.y + CENTER_Y - BUTTON_H / 2.0),
                egui::vec2(block_w, BUTTON_H),
            )),
            |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled_ui(ui_state.pending_save.is_none(), |ui| {
                            ui.add_sized([BUTTON_W, BUTTON_H], egui::Button::new("Save"))
                        })
                        .inner
                        .clicked()
                    {
                        handle_save_click(data, step_time_hz, ui_state);
                    }
                    if ui
                        .add_enabled_ui(ui_state.pending_load.is_none(), |ui| {
                            ui.add_sized([BUTTON_W, BUTTON_H], egui::Button::new("Load"))
                        })
                        .inner
                        .clicked()
                    {
                        handle_load_click(ui_state);
                    }
                });
            },
        );
    });

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(5.0);
}

fn draw_chip_tabs(ui: &mut egui::Ui, _data: &mut SharedSequences) {
    ui.horizontal(|ui| {
        let _ = ui.selectable_label(true, "Envelope Editors");
    });
    ui.add_space(5.0);
    ui.separator();
}

fn draw_instrument_settings_panel(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    shared_sequence_index: usize,
    step_time_hz: u32,
    result: &mut EditorResult,
) {
    // "Duty / Noise" tab is disabled for Triangle mode.
    let seq_types: &[(&str, usize)] = &[
        ("Volume", 0),
        ("Arpeggio", 1),
        ("Pitch", 2),
        ("Hi-Pitch", 3),
        ("Duty / Noise", 4),
    ];

    const PANEL_WIDTH: f32 = 180.0;
    const GROUP_GAP: f32 = 12.0;

    ui.set_width(PANEL_WIDTH);

    // ---------------------------------------------------------
    // Instrument settings
    // ---------------------------------------------------------

    group_box(ui, "Instrument settings", |ui| {
        // Make the group box fill the width of the left panel.
        ui.set_min_width(ui.available_width());

        // Grid with checkboxes and effect names
        egui::Grid::new("seq_type_grid")
            .num_columns(2)
            .spacing([6.0, 6.0])
            .show(ui, |ui| {
                ui.label("");
                ui.label(
                    egui::RichText::new("Effect name")
                        .color(egui::Color32::from_rgb(130, 130, 130)),
                );
                ui.end_row();

                // Separator line under header row
                ui.separator();
                ui.separator();
                ui.end_row();

                for &(name, tab) in seq_types {
                    let enabled = tab_is_available(tab, data.channel_mode);
                    let name = if tab == 4 && data.channel_mode == ChannelMode::S5B {
                        "Noise / Mode"
                    } else {
                        name
                    };

                    ui.add_enabled(
                        enabled,
                        egui::Checkbox::new(data.sequence_enabled_mut(tab), ""),
                    );

                    if ui
                        .add_enabled(
                            enabled,
                            egui::Button::new(name).selected(data.selected_tab == tab),
                        )
                        .clicked()
                        && data.selected_tab != tab
                    {
                        cleanup_tab_sequence(data, data.selected_tab);
                        data.selected_tab = tab;
                        cleanup_tab_sequence(data, tab);
                    }

                    ui.end_row();
                }
            });

        // Sequence Index control inside the frame, below the grid
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label("Sequence Index:");

            let mut index = shared_sequence_index;

            if ui
                .add(egui::DragValue::new(&mut index).range(0..=MAX_SEQUENCES - 1))
                .changed()
            {
                result.new_sequence_index = Some(index);
            }
        });
    });

    // ---------------------------------------------------------
    // Time settings
    // ---------------------------------------------------------

    // The current cursor is now immediately below Instrument settings.
    // Move down slightly to create a gap between the group boxes.
    ui.add_space(GROUP_GAP);

    // Everything remaining in the left panel belongs to Time settings.
    let remaining = ui.available_rect_before_wrap();

    ui.scope_builder(egui::UiBuilder::new().max_rect(remaining), |ui| {
        group_box(ui, "Settings", |ui| {
            // Fill the same width as Instrument settings.
            ui.set_min_width(ui.available_width());

            // group_box() adds:
            //
            // top    = PADDING + TITLE_HEIGHT = 20px
            // bottom = PADDING                = 10px
            //
            // Therefore its contents need to be 30px shorter
            // than the outer available rectangle.
            ui.set_min_height(ui.available_height());

            // -------------------------------------------------
            // Time controls
            // -------------------------------------------------
            ui.horizontal(|ui| {
                ui.label("Engine Speed:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut hz = step_time_hz as i32;
                    if ui
                        .add(egui::DragValue::new(&mut hz).range(1..=600).suffix(" Hz"))
                        .changed()
                    {
                        result.new_step_time_hz = Some(hz);
                    }
                });
            });

            ui.horizontal(|ui| {
                ui.label("Polyphony:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(!data.portamento_enabled, |ui| {
                        if ui
                            .add(
                                egui::DragValue::new(&mut data.max_voices)
                                    .range(1..=8)
                                    .suffix(" voices"),
                            )
                            .changed()
                        {
                            result.new_max_voices = Some(data.max_voices);
                        }
                    });
                });
            });

            ui.horizontal(|ui| {
                ui.label("Porta. Speed:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::DragValue::new(&mut data.portamento_speed).range(0..=127))
                        .changed()
                    {
                        result.new_portamento_speed = Some(data.portamento_speed);
                    }
                });
            });
        });
    });
}

fn draw_sequence_editor_panel(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    playheads: &SequencePlayheads,
    step_time_hz: u32,
) {
    let tab = data.selected_tab;

    let title = match tab {
        0 => "Volume",
        1 => "Arpeggio",
        2 => "Pitch",
        3 => "Hi-pitch",
        _ if data.channel_mode == ChannelMode::S5B => "Noise / Mode",
        _ => "Duty / Noise",
    };

    group_box(ui, &format!("Sequence editor - {}", title), |ui| {
        // Controls below graph: add_space(6) + Size row (~22) + add_space(6) + TextEdit (~20) = ~54px.
        const CONTROLS_HEIGHT: f32 = 54.0;
        let graph_height = (ui.available_height() - CONTROLS_HEIGHT).max(150.0);
        let mut auto_enable = false;

        // Both reads must happen before `selected_sequence_mut` takes the mutable
        // borrow of `data` below.
        let channel_mode = data.channel_mode;
        let vol_mode = data.selected_sequence(tab).vol_mode;
        let vol_mode_5b = data.selected_sequence(tab).vol_mode_5b;
        let (min_val, max_val) = sequence_range(tab, channel_mode, vol_mode, vol_mode_5b);

        {
            let (text, sequence) = data.selected_sequence_mut(tab);

            let is_arpeggio = tab == 1;
            let graph_changed = if tab == 4 && channel_mode == ChannelMode::S5B {
                draw_s5b_duty_noise_graph(ui, sequence, playheads.step(tab), graph_height)
            } else {
                draw_envelope_bar_graph(
                    ui,
                    sequence,
                    min_val,
                    max_val,
                    is_arpeggio,
                    playheads.step(tab),
                    graph_height,
                )
            };
            if graph_changed {
                *text = sequence_to_text_for_tab(tab, channel_mode, sequence);
                auto_enable = true;
            }

            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.label("Size:");

                let mut desired_len = sequence.len();

                if repeating_button(ui, "-") {
                    desired_len = desired_len.saturating_sub(1);
                }

                // FamiTracker-style size control: drag the number vertically to
                // grow or shrink the sequence, while retaining the +/- buttons.
                ui.add_sized(
                    [28.0, 18.0],
                    egui::DragValue::new(&mut desired_len)
                        .speed(1.0)
                        .range(0..=256),
                );

                if repeating_button(ui, "+") {
                    desired_len = desired_len.saturating_add(1).min(256);
                }

                if desired_len != sequence.len() {
                    auto_enable = true;
                    sequence.values.resize(desired_len, 0);

                    if sequence.loop_point.is_some_and(|p| p >= desired_len) {
                        sequence.loop_point = None;
                    }

                    if sequence.release_point.is_some_and(|p| p >= desired_len) {
                        sequence.release_point = None;
                    }

                    *text = sequence_to_text_for_tab(tab, channel_mode, sequence);
                }

                ui.add_space(15.0);

                ui.label(format!(
                    "{} ms",
                    (sequence.len() as u64 * 1000) / (step_time_hz as u64).max(1)
                ));

                if tab == 1 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.radio_value(&mut sequence.arp_mode, ArpMode::Absolute, "Absolute");

                        ui.radio_value(&mut sequence.arp_mode, ArpMode::Relative, "Relative");

                        ui.label(egui::RichText::new("Mode:").weak());
                    });
                }

                if tab == 2 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.radio_value(&mut sequence.pitch_mode, PitchMode::Absolute, "Absolute");

                        ui.radio_value(&mut sequence.pitch_mode, PitchMode::Relative, "Relative");

                        ui.label(egui::RichText::new("Mode:").weak());
                    });
                }

                // dn exposes 16-step/64-step as a volume-sequence setting and only
                // enables it for VRC6 instruments (SequenceSetting.cpp), so the
                // toggle occupies the same slot as the arpeggio/pitch mode radios.
                if tab == 0 && channel_mode == ChannelMode::Vrc6Saw {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut is_64 = sequence.vol_mode == VolMode::Steps64;

                        if ui.checkbox(&mut is_64, "64-Step").changed() {
                            let next = if is_64 {
                                VolMode::Steps64
                            } else {
                                VolMode::Steps16
                            };
                            set_volume_step_mode(text, sequence, next);
                            auto_enable = true;
                        }
                    });
                }

                // Same idea, scoped to S5B's 5-bit hardware volume register
                // (16-step vs 32-step) instead of VRC6's 6-bit one. The two
                // checkboxes are mutually exclusive by construction since
                // they're gated on different channel modes.
                if tab == 0 && channel_mode == ChannelMode::S5B {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut is_32 = sequence.vol_mode_5b == VolMode5B::Steps32;

                        if ui.checkbox(&mut is_32, "32-Step").changed() {
                            let next = if is_32 {
                                VolMode5B::Steps32
                            } else {
                                VolMode5B::Steps16
                            };
                            set_volume_step_mode_5b(text, sequence, next);
                            auto_enable = true;
                        }
                    });
                }
            });

            ui.add_space(6.0);

            let edit = ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(ui.available_width())
                    .font(egui::TextStyle::Monospace),
            );

            let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));

            if edit.changed() {
                auto_enable = true;
                let sanitized = sanitize_sequence_text(text);
                // `parse_clamped` resets every mode field to its default, so all
                // three have to survive the round trip.
                let prev_pitch_mode = sequence.pitch_mode;
                let prev_arp_mode = sequence.arp_mode;
                let prev_vol_mode = sequence.vol_mode;
                let prev_vol_mode_5b = sequence.vol_mode_5b;

                *sequence = if sanitized.trim().is_empty() {
                    Sequence::default()
                } else if tab == 4 && channel_mode == ChannelMode::S5B {
                    parse_s5b_duty_text(&sanitized).0
                } else {
                    Sequence::parse_clamped(&sanitized, min_val, max_val).0
                };

                sequence.pitch_mode = prev_pitch_mode;
                sequence.arp_mode = prev_arp_mode;
                sequence.vol_mode = prev_vol_mode;
                sequence.vol_mode_5b = prev_vol_mode_5b;
            }

            if enter_pressed || edit.lost_focus() {
                *text = sequence_to_text_for_tab(tab, channel_mode, sequence);
            }
        }

        if auto_enable {
            *data.sequence_enabled_mut(tab) = true;
        }
    });
}

fn draw_main_content(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    shared_sequence_index: usize,
    playheads: &SequencePlayheads,
    step_time_hz: u32,
    result: &mut EditorResult,
) {
    const LEFT_W: f32 = 180.0;
    const GAP: f32 = 8.0;
    const TOP_GAP: f32 = 10.0;

    let available = ui.available_rect_before_wrap();

    // Move the start of BOTH columns down.
    let content_top = available.min.y + TOP_GAP;

    let left_rect = egui::Rect::from_min_max(
        egui::pos2(available.min.x, content_top),
        egui::pos2(available.min.x + LEFT_W, available.max.y),
    );

    let right_rect = egui::Rect::from_min_max(
        egui::pos2(left_rect.max.x + GAP, content_top),
        available.max,
    );

    ui.scope_builder(egui::UiBuilder::new().max_rect(left_rect), |ui| {
        draw_instrument_settings_panel(ui, data, shared_sequence_index, step_time_hz, result);
    });

    ui.scope_builder(egui::UiBuilder::new().max_rect(right_rect), |ui| {
        draw_sequence_editor_panel(ui, data, playheads, step_time_hz);
    });
}

fn draw_footer(ui: &mut egui::Ui, ui_state: &EditorUiState) {
    ui.separator();
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(env!("CARGO_PKG_VERSION")).weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let (Some(status), Some(opacity)) =
                (ui_state.status_text(), ui_state.status_opacity())
            {
                ui.scope(|ui| {
                    ui.set_opacity(opacity);
                    ui.label(status);
                });
                ui.ctx().request_repaint();
            }
        });
    });
}

pub fn render_editor_ui(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    shared_sequence_index: usize,
    playheads: &SequencePlayheads,
    step_time_hz: u32,
    ui_state: &mut EditorUiState,
) -> EditorResult {
    // Apply non-selectable labels to the global Context style so all child scopes,
    // grids, and group boxes inherit it
    ui.ctx().global_style_mut(|style| {
        style.interaction.selectable_labels = false;
    });

    ui.set_min_height(ui.available_height());

    // Each panel records what the user touched straight into this, so the change
    // set travels as one value instead of a fan of `&mut Option<_>` out-params.
    let mut result = EditorResult::default();

    data.set_all_selected_sequence_indices(shared_sequence_index);

    // Called right after the line above because that is the one place every path
    // that can change the index converges — see `recall_slot_waveform`.
    recall_slot_waveform(
        data,
        shared_sequence_index,
        &mut ui_state.last_sequence_index,
        &mut result,
    );

    // Noise has no fine-pitch control. This also handles channel changes made
    // through host automation rather than through the editor combobox.
    if data.channel_mode == ChannelMode::Noise && matches!(data.selected_tab, 2 | 3) {
        data.selected_tab = 0;
    }

    // 64-step volume is saw-only; drop back to the 4-bit range on every other
    // channel. Runs after the recall above so it sees the recalled channel mode,
    // matching the ordering `apply_loaded_patch` documents.
    sync_volume_step_mode_to_channel(data);
    // Same idea for S5B's 32-step volume option.
    sync_volume_step_mode_5b_to_channel(data);

    draw_header(
        ui,
        data,
        &mut result,
        ui_state,
        step_time_hz,
        shared_sequence_index,
    );
    draw_chip_tabs(ui, data);

    // Reserve footer space at the bottom of the window with spacing gap
    const FOOTER_HEIGHT: f32 = 26.0;
    const FOOTER_GAP: f32 = 10.0;
    let available = ui.available_rect_before_wrap();

    // Allocate full available space in parent UI so outer frames don't shrink
    ui.allocate_rect(available, egui::Sense::hover());

    let footer_rect = egui::Rect::from_min_max(
        egui::pos2(available.min.x, available.max.y - FOOTER_HEIGHT),
        available.max,
    );
    let main_rect = egui::Rect::from_min_max(
        available.min,
        egui::pos2(available.max.x, footer_rect.min.y - FOOTER_GAP),
    );

    // Main content fills everything above the footer
    ui.scope_builder(egui::UiBuilder::new().max_rect(main_rect), |ui| {
        draw_main_content(
            ui,
            data,
            shared_sequence_index,
            playheads,
            step_time_hz,
            &mut result,
        );
    });

    // Footer is always at the very bottom
    ui.scope_builder(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
        draw_footer(ui, ui_state);
    });

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A saw instrument whose volume envelope uses the 64-step range — the
    /// only data shape the load-ordering hazard can destroy.
    fn saw_patch_with_64_step_volume() -> crate::Patch {
        let mut source = SharedSequences::default();
        source.channel_mode = ChannelMode::Vrc6Saw;
        source.set_selected_sequence_index(0, 7);
        source.set_slot_waveform(7, ChannelMode::Vrc6Saw);
        {
            let (text, sequence) = source.selected_sequence_mut(0);
            sequence.values = vec![63, 40, 20];
            sequence.vol_mode = VolMode::Steps64;
            *text = sequence_to_text(sequence);
        }
        crate::Patch::from_shared_sequences(&source, 60)
    }

    #[test]
    fn loading_a_saw_patch_survives_the_next_frames_volume_sync() {
        let patch = saw_patch_with_64_step_volume();

        // The editor is on a non-saw channel, which is what arms the hazard.
        let mut data = SharedSequences::default();
        data.channel_mode = ChannelMode::Pulse;
        let mut result = EditorResult::default();

        apply_loaded_patch(&mut data, &mut result, &patch);

        // `render_editor_ui` runs this unconditionally at the top of every
        // frame, so the very next repaint after the click does exactly this.
        sync_volume_step_mode_to_channel(&mut data);

        assert_eq!(
            data.selected_sequence(0).vol_mode,
            VolMode::Steps64,
            "the loaded saw patch must still be in 64-step mode"
        );
        assert_eq!(
            data.selected_sequence(0).values,
            vec![63, 40, 20],
            "64-step volume data must survive the next frame's sync"
        );
    }

    #[test]
    fn writing_sequences_before_the_channel_mode_would_clamp_a_saw_patch() {
        // The inverse of the test above: proves that test is actually load
        // bearing rather than passing for unrelated reasons. This reproduces
        // the wrong ordering by hand — sequences first, channel mode second —
        // and shows the next frame's sync halves every step. If a refactor
        // ever makes this ordering safe, delete both tests together.
        let patch = saw_patch_with_64_step_volume();

        let mut data = SharedSequences::default();
        data.channel_mode = ChannelMode::Pulse;

        patch.apply_to_shared_sequences(&mut data);
        sync_volume_step_mode_to_channel(&mut data);
        data.channel_mode = patch.active_waveform();

        assert_eq!(
            data.selected_sequence(0).values,
            vec![15, 10, 5],
            "the wrong ordering is expected to scale the steps down by 4"
        );
    }

    #[test]
    fn channel_mode_change_cleans_up_tabs_the_new_waveform_lacks() {
        // Triangle has no duty envelope; noise has neither duty nor fine pitch.
        for (mode, tab, expected_tab) in [
            (ChannelMode::Triangle, 4, 0),
            (ChannelMode::Triangle, 0, 0),
            (ChannelMode::Noise, 2, 0),
            (ChannelMode::Noise, 3, 0),
            (ChannelMode::Noise, 4, 0),
            (ChannelMode::Noise, 1, 1),
            // Every tab is valid on these, including the saw's duty tab.
            (ChannelMode::Pulse, 4, 4),
            (ChannelMode::Vrc6Pulse, 4, 4),
            (ChannelMode::Vrc6Saw, 4, 4),
        ] {
            let mut data = SharedSequences::default();
            data.selected_tab = tab;
            let mut result = EditorResult::default();

            apply_channel_mode_change(&mut data, mode, &mut result);

            assert_eq!(data.channel_mode, mode);
            assert_eq!(result.new_channel_mode, Some(mode));
            assert_eq!(
                data.selected_tab, expected_tab,
                "{mode:?} from tab {tab} should land on tab {expected_tab}"
            );
        }
    }

    #[test]
    fn switching_slots_recalls_that_slots_remembered_waveform() {
        let mut data = SharedSequences::default();
        data.set_slot_waveform(0, ChannelMode::Triangle);
        data.set_slot_waveform(5, ChannelMode::Noise);
        data.channel_mode = ChannelMode::Triangle;

        let mut last = Some(0);
        let mut result = EditorResult::default();

        recall_slot_waveform(&mut data, 5, &mut last, &mut result);
        assert_eq!(data.channel_mode, ChannelMode::Noise);
        assert_eq!(
            result.new_channel_mode,
            Some(ChannelMode::Noise),
            "the recall must travel out as a host-parameter gesture, or the \
             audio thread re-asserts the old waveform on its next block"
        );

        // Back to 0, and on to a slot nobody ever touched.
        let mut result = EditorResult::default();
        recall_slot_waveform(&mut data, 0, &mut last, &mut result);
        assert_eq!(data.channel_mode, ChannelMode::Triangle);

        let mut result = EditorResult::default();
        recall_slot_waveform(&mut data, 10, &mut last, &mut result);
        assert_eq!(
            data.channel_mode,
            ChannelMode::Pulse,
            "an untouched slot remembers the default waveform"
        );
    }

    #[test]
    fn recall_only_fires_on_a_real_index_transition() {
        // `apply_loaded_patch` sets `channel_mode` mid-frame, while this
        // frame's index is still the old slot. If the recall compared and
        // applied on every repaint instead of only on a transition, it would
        // read the old slot's remembered waveform and clobber the loaded one.
        // This pins that guard: repaints at an unchanged index change nothing.
        let mut data = SharedSequences::default();
        data.set_slot_waveform(3, ChannelMode::Triangle);
        let mut last = None;
        let mut result = EditorResult::default();

        // First frame on slot 3: seeds `last`, recalls nothing.
        recall_slot_waveform(&mut data, 3, &mut last, &mut result);
        assert_eq!(data.channel_mode, ChannelMode::Pulse);
        assert_eq!(result.new_channel_mode, None);

        // Stand in for a load having just written `channel_mode` directly,
        // then repaint several times with the index unchanged.
        data.channel_mode = ChannelMode::Noise;
        for _ in 0..3 {
            let mut result = EditorResult::default();
            recall_slot_waveform(&mut data, 3, &mut last, &mut result);
            assert_eq!(
                data.channel_mode,
                ChannelMode::Noise,
                "a repaint with no index change must not overwrite a waveform \
                 that was set directly this frame"
            );
            assert_eq!(result.new_channel_mode, None);
        }
    }

    #[test]
    fn recall_after_a_load_agrees_with_the_loaded_patch() {
        // Load pushes a new Sequence Index as a parameter gesture, so the next
        // frame sees a transition and recalls. Since the loaded channel comes
        // from the landing slot's own remembered waveform, that recall reads
        // back the same value it just loaded rather than reverting it.
        let mut source = SharedSequences::default();
        source.set_selected_sequence_index(0, 9);
        source.set_slot_waveform(9, ChannelMode::Vrc6Pulse);
        source.selected_sequence_mut(0).1.values = vec![15, 7];
        let patch = crate::Patch::from_shared_sequences(&source, 120);

        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();
        apply_loaded_patch(&mut data, &mut result, &patch);

        // The frame after the host applies `result.new_sequence_index`.
        let mut last = Some(0);
        let mut next_result = EditorResult::default();
        recall_slot_waveform(&mut data, 9, &mut last, &mut next_result);

        assert_eq!(
            data.channel_mode,
            ChannelMode::Vrc6Pulse,
            "the recall must not revert the just-loaded patch's waveform"
        );
    }

    #[test]
    fn s5b_duty_slots_reload_as_token_text_not_raw_integers() {
        // A packed S5B duty step is a bitfield, so it must come back out of a
        // save/load as `"5tnw2"`. Rendering it with the plain numeric
        // formatter yields `"3781"`, which then reparses as period 3781
        // (clamped to 31) with every flag and the duty width lost — silent
        // data loss the moment the text box is touched after a load. The
        // second case is the one that was already broken before duty width
        // existed: the noise flag alone puts the value past 127.
        for (duty_index, expected_text) in [(Some(2), "5tnw2"), (None, "5tn")] {
            let mut source = SharedSequences::default();
            source.channel_mode = ChannelMode::S5B;
            source.set_slot_waveform(0, ChannelMode::S5B);

            let base = 5
                | rp2a03_core::sequencer::S5B_MODE_SQUARE
                | rp2a03_core::sequencer::S5B_MODE_NOISE;
            let value = match duty_index {
                Some(index) => rp2a03_core::sequencer::s5b_set_duty_index(base, index),
                None => base,
            };
            source.selected_sequence_mut(4).1.values = vec![value];

            let patch = crate::Patch::from_shared_sequences(&source, 60);
            let bytes = patch.to_bytes();
            let restored_patch =
                crate::Patch::from_bytes(&bytes).expect("packed S5B duty patch must decode");

            let mut data = SharedSequences::default();
            let mut result = EditorResult::default();
            apply_loaded_patch(&mut data, &mut result, &restored_patch);

            assert_eq!(
                data.selected_sequence(4).values,
                vec![value],
                "packed S5B duty value must survive the load"
            );
            assert_eq!(
                data.sequence_bank(4).slot(0).text,
                expected_text,
                "reloaded text must be the S5B token form, not raw integers"
            );

            // The text is what the next edit reparses, so round-trip it the
            // way the text box would and confirm nothing is lost.
            let (reparsed, _) = parse_s5b_duty_text(&data.sequence_bank(4).slot(0).text);
            assert_eq!(
                reparsed.values,
                vec![value],
                "reloaded text must reparse back to the same packed value"
            );
        }
    }

    #[test]
    fn non_s5b_duty_slots_still_reload_as_plain_numbers() {
        // The tab-aware formatter must not change how every other channel's
        // duty lane round-trips.
        let mut source = SharedSequences::default();
        source.channel_mode = ChannelMode::Vrc6Pulse;
        source.set_slot_waveform(0, ChannelMode::Vrc6Pulse);
        source.selected_sequence_mut(4).1.values = vec![3, 5, 7];

        let patch = crate::Patch::from_shared_sequences(&source, 60);
        let restored_patch =
            crate::Patch::from_bytes(&patch.to_bytes()).expect("patch must decode");

        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();
        apply_loaded_patch(&mut data, &mut result, &restored_patch);

        assert_eq!(data.sequence_bank(4).slot(0).text, "3 5 7");
    }

    #[test]
    fn loading_moves_off_a_tab_the_loaded_waveform_does_not_have() {
        let mut source = SharedSequences::default();
        source.set_selected_sequence_index(4, 1);
        source.set_slot_waveform(0, ChannelMode::Triangle); // slot 0: vol's active index
        source.selected_sequence_mut(4).1.values = vec![1, 2];
        let patch = crate::Patch::from_shared_sequences(&source, 60);

        let mut data = SharedSequences::default();
        data.selected_tab = 4; // Duty / Noise — triangle has no duty envelope
        let mut result = EditorResult::default();

        apply_loaded_patch(&mut data, &mut result, &patch);

        assert_eq!(
            data.selected_tab, 0,
            "loading a triangle patch must move off the duty tab"
        );
    }

    #[test]
    fn loading_reports_every_host_parameter_the_patch_carries() {
        // Without these the shared state and the host parameters disagree, and
        // the audio thread re-asserts the stale parameter over the loaded slot
        // selection on its next block (see `apply_loaded_patch`).
        let mut source = SharedSequences::default();
        source.set_selected_sequence_index(0, 9);
        source.set_slot_waveform(9, ChannelMode::Vrc6Pulse);
        source.selected_sequence_mut(0).1.values = vec![15, 7];
        let patch = crate::Patch::from_shared_sequences(&source, 120);

        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();

        apply_loaded_patch(&mut data, &mut result, &patch);

        assert_eq!(result.new_channel_mode, Some(ChannelMode::Vrc6Pulse));
        assert_eq!(result.new_step_time_hz, Some(120));
        assert_eq!(result.new_sequence_index, Some(9));
        assert_eq!(
            data.channel_mode,
            ChannelMode::Vrc6Pulse,
            "the shared state must be updated alongside the parameter signal"
        );
    }

    #[test]
    fn unavailable_tabs_match_the_channels_missing_envelopes() {
        for tab in 0..crate::SEQUENCE_TYPE_COUNT {
            assert!(tab_is_available(tab, ChannelMode::Pulse));
            assert!(tab_is_available(tab, ChannelMode::Vrc6Pulse));
            assert!(tab_is_available(tab, ChannelMode::Vrc6Saw));
        }

        assert!(!tab_is_available(4, ChannelMode::Triangle));
        assert!(tab_is_available(0, ChannelMode::Triangle));

        for tab in [2, 3, 4] {
            assert!(!tab_is_available(tab, ChannelMode::Noise));
        }
        assert!(tab_is_available(0, ChannelMode::Noise));
        assert!(tab_is_available(1, ChannelMode::Noise));

        // S5B is a tone channel with pitch, hi-pitch, and a duty/mode tab,
        // same shape as Pulse/Vrc6Pulse — no exclusions.
        for tab in 0..crate::SEQUENCE_TYPE_COUNT {
            assert!(tab_is_available(tab, ChannelMode::S5B));
        }
    }

    #[test]
    fn switching_slots_recalls_an_s5b_waveform() {
        // Parallel to `switching_slots_recalls_that_slots_remembered_waveform`.
        let mut data = SharedSequences::default();
        data.set_slot_waveform(5, ChannelMode::S5B);
        data.channel_mode = ChannelMode::Pulse;

        let mut last = Some(0);
        let mut result = EditorResult::default();
        recall_slot_waveform(&mut data, 5, &mut last, &mut result);

        assert_eq!(data.channel_mode, ChannelMode::S5B);
        assert_eq!(result.new_channel_mode, Some(ChannelMode::S5B));
    }

    #[test]
    fn status_text_expires_after_its_display_window() {
        let mut ui_state = EditorUiState::default();
        assert_eq!(ui_state.status_text(), None);

        ui_state.set("Saved");
        assert_eq!(ui_state.status_text(), Some("Saved"));

        ui_state.status = Some((
            "Saved".to_string(),
            std::time::Instant::now() - STATUS_DISPLAY_DURATION,
        ));
        assert_eq!(
            ui_state.status_text(),
            None,
            "a message older than the display window must stop rendering"
        );
    }

    #[test]
    fn polling_an_empty_pending_save_channel_does_not_block_or_clear_it() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut ui_state = EditorUiState {
            pending_save: Some(rx),
            ..Default::default()
        };
        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();

        poll_pending_dialogs(&mut data, &mut result, &mut ui_state);

        assert!(
            ui_state.pending_save.is_some(),
            "an unfinished dialog must stay pending, not silently drop"
        );
    }

    #[test]
    fn polling_a_finished_save_channel_reports_the_status_and_clears_pending() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Some("Saved test.rp2a03patch".to_string())).unwrap();
        let mut ui_state = EditorUiState {
            pending_save: Some(rx),
            ..Default::default()
        };
        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();

        poll_pending_dialogs(&mut data, &mut result, &mut ui_state);

        assert!(ui_state.pending_save.is_none());
        assert_eq!(ui_state.status_text(), Some("Saved test.rp2a03patch"));
    }

    #[test]
    fn polling_a_cancelled_save_dialog_clears_pending_without_a_status() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(None).unwrap();
        let mut ui_state = EditorUiState {
            pending_save: Some(rx),
            ..Default::default()
        };
        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();

        poll_pending_dialogs(&mut data, &mut result, &mut ui_state);

        assert!(
            ui_state.pending_save.is_none(),
            "the Save button must re-enable after a cancelled dialog"
        );
        assert_eq!(ui_state.status_text(), None);
    }

    #[test]
    fn polling_a_cancelled_load_dialog_clears_pending_without_touching_state() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(None).unwrap();
        let mut ui_state = EditorUiState {
            pending_load: Some(rx),
            ..Default::default()
        };
        let mut data = SharedSequences::default();
        data.channel_mode = ChannelMode::Vrc6Saw;
        let mut result = EditorResult::default();

        poll_pending_dialogs(&mut data, &mut result, &mut ui_state);

        assert!(
            ui_state.pending_load.is_none(),
            "the Load button must re-enable after a cancelled dialog"
        );
        assert_eq!(data.channel_mode, ChannelMode::Vrc6Saw);
        assert_eq!(result.new_channel_mode, None);
    }

    #[test]
    fn polling_a_finished_load_dialog_loads_the_file_from_the_picked_path() {
        let mut source = SharedSequences::default();
        source.set_selected_sequence_index(0, 3);
        source.selected_sequence_mut(0).1.values = vec![10, 5];
        let patch = crate::Patch::from_shared_sequences(&source, 60);

        let path = std::env::temp_dir().join(format!(
            "rp2a03_editor_test_{}_{}.rp2a03patch",
            std::process::id(),
            line!()
        ));
        crate::save_patch_to_path(&path, &patch).expect("setup save must succeed");

        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Some(path.clone())).unwrap();
        let mut ui_state = EditorUiState {
            pending_load: Some(rx),
            ..Default::default()
        };
        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();

        poll_pending_dialogs(&mut data, &mut result, &mut ui_state);
        std::fs::remove_file(&path).expect("cleanup must succeed");

        assert!(ui_state.pending_load.is_none());
        assert_eq!(data.selected_sequence(0).values, vec![10, 5]);
        assert_eq!(result.new_channel_mode, Some(ChannelMode::Pulse));
    }

    #[test]
    fn polling_a_disconnected_channel_clears_pending_instead_of_wedging_the_button() {
        // Simulates the dialog thread panicking before it can send anything —
        // dropping the sender is exactly what that looks like from here.
        let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
        drop(tx);
        let mut ui_state = EditorUiState {
            pending_save: Some(rx),
            ..Default::default()
        };
        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();

        poll_pending_dialogs(&mut data, &mut result, &mut ui_state);

        assert!(
            ui_state.pending_save.is_none(),
            "a dead sender must not leave the Save button permanently disabled"
        );
    }

    #[test]
    fn sequence_text_round_trips_markers() {
        let sequence = Sequence {
            values: vec![15, 12, 10],
            loop_point: Some(1),
            release_point: Some(1),
            ..Sequence::default()
        };

        let text = sequence_to_text(&sequence);
        assert_eq!(text, "15 | / 12 10");
        let (reparsed, _) = Sequence::parse_clamped(&text, 0, 15);
        assert_eq!(reparsed, sequence);
    }

    #[test]
    fn empty_sequence_has_empty_text() {
        assert!(sequence_to_text(&Sequence::default()).is_empty());
    }

    #[test]
    fn selected_sequence_cleanup_does_not_modify_another_slot() {
        let mut data = SharedSequences::default();
        data.selected_sequence_mut(0).0.push_str("15 12");
        cleanup_tab_sequence(&mut data, 0);
        data.set_selected_sequence_index(0, 1);
        cleanup_tab_sequence(&mut data, 0);
        assert_eq!(data.selected_sequence(0).len(), 0);
        data.set_selected_sequence_index(0, 0);
        assert_eq!(data.selected_sequence(0).values, vec![15, 12]);
    }

    #[test]
    fn hi_pitch_and_pitch_accept_the_full_dn_signed_char_range() {
        // dnFamiTracker's CPitchGraphEditor serves both tabs with a 127..-128 axis, and
        // both the graph and the text box must agree on it.
        assert_eq!(
            sequence_range(2, ChannelMode::Pulse, VolMode::Steps16, VolMode5B::Steps16),
            (-128, 127)
        );
        assert_eq!(
            sequence_range(3, ChannelMode::Pulse, VolMode::Steps16, VolMode5B::Steps16),
            (-128, 127)
        );

        let mut data = SharedSequences::default();
        data.selected_sequence_mut(3)
            .0
            .push_str("127 -128 64 -64 200 -200");
        cleanup_tab_sequence(&mut data, 3);
        assert_eq!(
            data.selected_sequence(3).values,
            vec![127, -128, 64, -64, 127, -128]
        );
    }

    #[test]
    fn vrc6_saw_volume_range_follows_the_step_mode() {
        // dn defaults the VRC6 saw volume sequence to SETTING_VOL_16_STEPS; the 6-bit
        // range is only reachable through SETTING_VOL_64_STEPS.
        assert_eq!(
            sequence_range(
                0,
                ChannelMode::Vrc6Saw,
                VolMode::Steps16,
                VolMode5B::Steps16
            ),
            (0, 15)
        );
        assert_eq!(
            sequence_range(
                0,
                ChannelMode::Vrc6Saw,
                VolMode::Steps64,
                VolMode5B::Steps16
            ),
            (0, 63)
        );

        // The step mode is saw-only — every other channel stays 4-bit either way.
        assert_eq!(
            sequence_range(0, ChannelMode::Pulse, VolMode::Steps64, VolMode5B::Steps16),
            (0, 15)
        );
        assert_eq!(
            sequence_range(
                0,
                ChannelMode::Vrc6Pulse,
                VolMode::Steps64,
                VolMode5B::Steps16
            ),
            (0, 15)
        );
    }

    #[test]
    fn s5b_volume_range_follows_the_step_mode() {
        // Parallel to `vrc6_saw_volume_range_follows_the_step_mode`, but the
        // S5B's hardware volume register is 5-bit (16<->32) instead of the
        // saw's 6-bit (16<->64).
        assert_eq!(
            sequence_range(0, ChannelMode::S5B, VolMode::Steps16, VolMode5B::Steps16),
            (0, 15)
        );
        assert_eq!(
            sequence_range(0, ChannelMode::S5B, VolMode::Steps16, VolMode5B::Steps32),
            (0, 31)
        );

        // The step mode is S5B-only — every other channel stays 4-bit either way.
        assert_eq!(
            sequence_range(0, ChannelMode::Pulse, VolMode::Steps16, VolMode5B::Steps32),
            (0, 15)
        );
        assert_eq!(
            sequence_range(
                0,
                ChannelMode::Vrc6Pulse,
                VolMode::Steps16,
                VolMode5B::Steps32
            ),
            (0, 15)
        );
    }

    #[test]
    fn vrc6_saw_duty_range_is_one_bit() {
        // dn `CVRC6Sawtooth::MAX_DUTY = 0x01` — the saw's duty is the $B000 rate MSB.
        assert_eq!(
            sequence_range(
                4,
                ChannelMode::Vrc6Saw,
                VolMode::Steps16,
                VolMode5B::Steps16
            ),
            (0, 1)
        );
        assert_eq!(
            sequence_range(
                4,
                ChannelMode::Vrc6Pulse,
                VolMode::Steps16,
                VolMode5B::Steps16
            ),
            (0, 7)
        );
        assert_eq!(
            sequence_range(4, ChannelMode::Pulse, VolMode::Steps16, VolMode5B::Steps16),
            (0, 3)
        );
    }

    #[test]
    fn s5b_duty_noise_range_is_five_bit_period() {
        // Parallel to `vrc6_saw_duty_range_is_one_bit`. Tab 4's Y-axis is the
        // noise period (0..=31); mode flags aren't part of the graph's range.
        assert_eq!(
            sequence_range(4, ChannelMode::S5B, VolMode::Steps16, VolMode5B::Steps16),
            (0, 31)
        );
    }

    #[test]
    fn leaving_the_saw_waveform_rescales_a_64_step_volume_sequence() {
        for channel_mode in [
            ChannelMode::Pulse,
            ChannelMode::Triangle,
            ChannelMode::Noise,
            ChannelMode::Vrc6Pulse,
        ] {
            let mut data = SharedSequences::default();
            data.channel_mode = ChannelMode::Vrc6Saw;
            data.selected_sequence_mut(0).0.push_str("63 32 4 0");
            cleanup_tab_sequence(&mut data, 0);

            {
                let (text, sequence) = data.selected_sequence_mut(0);
                set_volume_step_mode(text, sequence, VolMode::Steps64);
            }
            // "63 32 4 0" was authored under 16-step, so cleanup clamped it to
            // [15, 15, 4, 0] before the x4 promotion.
            assert_eq!(data.selected_sequence(0).values, vec![60, 60, 16, 0]);

            data.channel_mode = channel_mode;
            sync_volume_step_mode_to_channel(&mut data);

            assert_eq!(data.selected_sequence(0).vol_mode, VolMode::Steps16);
            assert_eq!(
                data.selected_sequence(0).values,
                vec![15, 15, 4, 0],
                "{channel_mode:?} must see 4-bit steps"
            );
            // The text box has to agree with the graph, not keep showing 0..63.
            assert_eq!(data.selected_sequence_mut(0).0, "15 15 4 0");

            // Idempotent — a second frame on the same channel changes nothing.
            sync_volume_step_mode_to_channel(&mut data);
            assert_eq!(data.selected_sequence(0).values, vec![15, 15, 4, 0]);
        }
    }

    #[test]
    fn staying_on_the_saw_waveform_keeps_64_step_values() {
        let mut data = SharedSequences::default();
        data.channel_mode = ChannelMode::Vrc6Saw;
        data.selected_sequence_mut(0).0.push_str("15 8 0");
        cleanup_tab_sequence(&mut data, 0);

        {
            let (text, sequence) = data.selected_sequence_mut(0);
            set_volume_step_mode(text, sequence, VolMode::Steps64);
        }

        sync_volume_step_mode_to_channel(&mut data);
        assert_eq!(data.selected_sequence(0).vol_mode, VolMode::Steps64);
        assert_eq!(data.selected_sequence(0).values, vec![60, 32, 0]);
    }

    #[test]
    fn cleanup_preserves_the_saw_step_mode_and_clamps_to_it() {
        let mut data = SharedSequences::default();
        data.channel_mode = ChannelMode::Vrc6Saw;
        data.selected_sequence_mut(0).1.vol_mode = VolMode::Steps64;
        data.selected_sequence_mut(0).0.push_str("63 32 0");
        cleanup_tab_sequence(&mut data, 0);
        assert_eq!(data.selected_sequence(0).vol_mode, VolMode::Steps64);
        assert_eq!(data.selected_sequence(0).values, vec![63, 32, 0]);

        // Back to 16-step: the same text now clamps to the 4-bit range.
        data.selected_sequence_mut(0).1.vol_mode = VolMode::Steps16;
        cleanup_tab_sequence(&mut data, 0);
        assert_eq!(data.selected_sequence(0).vol_mode, VolMode::Steps16);
        assert_eq!(data.selected_sequence(0).values, vec![15, 15, 0]);
    }

    #[test]
    fn leaving_s5b_rescales_a_32_step_volume_sequence() {
        // Parallel to `leaving_the_saw_waveform_rescales_a_64_step_volume_sequence`.
        for channel_mode in [
            ChannelMode::Pulse,
            ChannelMode::Triangle,
            ChannelMode::Noise,
            ChannelMode::Vrc6Pulse,
            ChannelMode::Vrc6Saw,
        ] {
            let mut data = SharedSequences::default();
            data.channel_mode = ChannelMode::S5B;
            data.selected_sequence_mut(0).0.push_str("31 16 4 0");
            cleanup_tab_sequence(&mut data, 0);

            {
                let (text, sequence) = data.selected_sequence_mut(0);
                set_volume_step_mode_5b(text, sequence, VolMode5B::Steps32);
            }
            // "31 16 4 0" was authored under 16-step, so cleanup clamped it to
            // [15, 15, 4, 0] before the x2 promotion.
            assert_eq!(data.selected_sequence(0).values, vec![30, 30, 8, 0]);

            data.channel_mode = channel_mode;
            sync_volume_step_mode_5b_to_channel(&mut data);

            assert_eq!(data.selected_sequence(0).vol_mode_5b, VolMode5B::Steps16);
            assert_eq!(
                data.selected_sequence(0).values,
                vec![15, 15, 4, 0],
                "{channel_mode:?} must see 4-bit steps"
            );
            assert_eq!(data.selected_sequence_mut(0).0, "15 15 4 0");

            // Idempotent — a second frame on the same channel changes nothing.
            sync_volume_step_mode_5b_to_channel(&mut data);
            assert_eq!(data.selected_sequence(0).values, vec![15, 15, 4, 0]);
        }
    }

    #[test]
    fn staying_on_s5b_keeps_32_step_values() {
        // Parallel to `staying_on_the_saw_waveform_keeps_64_step_values`.
        let mut data = SharedSequences::default();
        data.channel_mode = ChannelMode::S5B;
        data.selected_sequence_mut(0).0.push_str("15 8 0");
        cleanup_tab_sequence(&mut data, 0);

        {
            let (text, sequence) = data.selected_sequence_mut(0);
            set_volume_step_mode_5b(text, sequence, VolMode5B::Steps32);
        }

        sync_volume_step_mode_5b_to_channel(&mut data);
        assert_eq!(data.selected_sequence(0).vol_mode_5b, VolMode5B::Steps32);
        assert_eq!(data.selected_sequence(0).values, vec![30, 16, 0]);
    }

    #[test]
    fn cleanup_preserves_the_s5b_step_mode_and_clamps_to_it() {
        // Parallel to `cleanup_preserves_the_saw_step_mode_and_clamps_to_it`.
        let mut data = SharedSequences::default();
        data.channel_mode = ChannelMode::S5B;
        data.selected_sequence_mut(0).1.vol_mode_5b = VolMode5B::Steps32;
        data.selected_sequence_mut(0).0.push_str("31 16 0");
        cleanup_tab_sequence(&mut data, 0);
        assert_eq!(data.selected_sequence(0).vol_mode_5b, VolMode5B::Steps32);
        assert_eq!(data.selected_sequence(0).values, vec![31, 16, 0]);

        // Back to 16-step: the same text now clamps to the 4-bit range.
        data.selected_sequence_mut(0).1.vol_mode_5b = VolMode5B::Steps16;
        cleanup_tab_sequence(&mut data, 0);
        assert_eq!(data.selected_sequence(0).vol_mode_5b, VolMode5B::Steps16);
        assert_eq!(data.selected_sequence(0).values, vec![15, 15, 0]);
    }

    /// An S5B instrument whose volume envelope uses the 32-step range — the
    /// only data shape the load-ordering hazard can destroy. Parallel to
    /// `saw_patch_with_64_step_volume`.
    fn s5b_patch_with_32_step_volume() -> crate::Patch {
        let mut source = SharedSequences::default();
        source.channel_mode = ChannelMode::S5B;
        source.set_selected_sequence_index(0, 7);
        source.set_slot_waveform(7, ChannelMode::S5B);
        {
            let (text, sequence) = source.selected_sequence_mut(0);
            sequence.values = vec![31, 20, 10];
            sequence.vol_mode_5b = VolMode5B::Steps32;
            *text = sequence_to_text(sequence);
        }
        crate::Patch::from_shared_sequences(&source, 60)
    }

    #[test]
    fn loading_an_s5b_patch_survives_the_next_frames_volume_sync() {
        // Parallel to `loading_a_saw_patch_survives_the_next_frames_volume_sync`.
        let patch = s5b_patch_with_32_step_volume();
        let mut data = SharedSequences::default();
        let mut result = EditorResult::default();

        apply_loaded_patch(&mut data, &mut result, &patch);

        assert_eq!(data.selected_sequence(0).vol_mode_5b, VolMode5B::Steps32);
        assert_eq!(data.selected_sequence(0).values, vec![31, 20, 10]);
    }

    #[test]
    fn s5b_duty_period_set_preserves_flags() {
        // Widget-level: setting the period must not disturb the flag bits
        // already set on that step.
        let mut seq = Sequence {
            values: vec![
                5 | rp2a03_core::sequencer::S5B_MODE_SQUARE
                    | rp2a03_core::sequencer::S5B_MODE_NOISE,
            ],
            ..Sequence::default()
        };
        let value = seq.values[0];
        let new_period: i16 = 12;
        seq.values[0] = (value & !rp2a03_core::sequencer::S5B_PERIOD_MASK) | new_period;

        assert_eq!(seq.values[0] & rp2a03_core::sequencer::S5B_PERIOD_MASK, 12);
        assert_ne!(seq.values[0] & rp2a03_core::sequencer::S5B_MODE_SQUARE, 0);
        assert_ne!(seq.values[0] & rp2a03_core::sequencer::S5B_MODE_NOISE, 0);
    }

    #[test]
    fn s5b_duty_flag_toggle_preserves_period() {
        // Widget-level: toggling a flag bit must not disturb the period bits.
        let mut value: i16 = 9;
        value ^= rp2a03_core::sequencer::S5B_MODE_NOISE;

        assert_eq!(value & rp2a03_core::sequencer::S5B_PERIOD_MASK, 9);
        assert_ne!(value & rp2a03_core::sequencer::S5B_MODE_NOISE, 0);

        value ^= rp2a03_core::sequencer::S5B_MODE_NOISE;
        assert_eq!(value, 9);
    }

    #[test]
    fn s5b_duty_text_round_trips_period_and_flags() {
        let value: i16 =
            5 | rp2a03_core::sequencer::S5B_MODE_SQUARE | rp2a03_core::sequencer::S5B_MODE_NOISE;
        let seq = Sequence {
            values: vec![value],
            ..Sequence::default()
        };

        let text = sequence_to_text_for_tab(4, ChannelMode::S5B, &seq);
        assert_eq!(text, "5tn");

        let (reparsed, normalized) = parse_s5b_duty_text(&text);
        assert_eq!(normalized, "5tn");
        assert_eq!(reparsed.values, vec![value]);
    }

    #[test]
    fn s5b_duty_text_round_trips_width() {
        let value = rp2a03_core::sequencer::s5b_set_duty_index(
            5 | rp2a03_core::sequencer::S5B_MODE_SQUARE | rp2a03_core::sequencer::S5B_MODE_NOISE,
            2,
        );
        let seq = Sequence {
            values: vec![value],
            ..Sequence::default()
        };

        let text = sequence_to_text_for_tab(4, ChannelMode::S5B, &seq);
        assert_eq!(text, "5tnw2");

        let (reparsed, normalized) = parse_s5b_duty_text(&text);
        assert_eq!(normalized, "5tnw2");
        assert_eq!(reparsed.values, vec![value]);
    }

    #[test]
    fn s5b_duty_text_omits_default_width() {
        // Width 4 (50%) is the default, so it stays out of the text form and
        // sequences that never touch the field keep their old spelling.
        let value = rp2a03_core::sequencer::s5b_set_duty_index(
            5 | rp2a03_core::sequencer::S5B_MODE_SQUARE,
            4,
        );
        let seq = Sequence {
            values: vec![value],
            ..Sequence::default()
        };

        assert_eq!(sequence_to_text_for_tab(4, ChannelMode::S5B, &seq), "5t");

        let (reparsed, normalized) = parse_s5b_duty_text("5tw4");
        assert_eq!(normalized, "5t");
        assert_eq!(
            rp2a03_core::sequencer::s5b_duty_index(reparsed.values[0]),
            4
        );
    }

    #[test]
    fn s5b_duty_text_ignores_bare_width_letter() {
        // Permissive like the flag letters: a `w` with no digit after it is
        // dropped, leaving the default width, not rejected mid-keystroke.
        let (reparsed, normalized) = parse_s5b_duty_text("5tw");
        assert_eq!(normalized, "5t");
        assert_eq!(
            rp2a03_core::sequencer::s5b_duty_index(reparsed.values[0]),
            rp2a03_core::sequencer::S5B_DUTY_DEFAULT_INDEX
        );
    }

    #[test]
    fn s5b_duty_text_drops_dns_envelope_letter() {
        // The envelope flag is not offered on either surface, so `e` is just
        // another ignored letter rather than a way to set a bit nothing reads.
        assert_eq!(sanitize_sequence_text("5tne"), "5tn");
        let (reparsed, normalized) = parse_s5b_duty_text("5tne");
        assert_eq!(normalized, "5tn");
        assert_eq!(
            reparsed.values[0] & rp2a03_core::sequencer::S5B_MODE_ENVELOPE,
            0
        );
    }

    #[test]
    fn s5b_duty_text_ignores_malformed_flag_letters() {
        // dn's parser is permissive — unrecognized letters are dropped, not
        // rejected, since text-field edits happen keystroke-by-keystroke.
        let (reparsed, _) = parse_s5b_duty_text("5tzq");
        assert_eq!(
            reparsed.values,
            vec![5 | rp2a03_core::sequencer::S5B_MODE_SQUARE]
        );
    }
}
