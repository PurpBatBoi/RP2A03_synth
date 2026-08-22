//! `rp2a03_common\src\gui\editor\mod.rs`
//! The egui editor: `render_editor_ui` is the entry point, orchestrating
//! the header, tab bar, the tab-specific panel, and the footer — each in
//! its own sibling module.

mod footer;
mod header;
mod instrument_panel;
mod sequence_panel;
mod tabs;

use crate::gui::chip_settings::draw_chip_settings_panel;
use crate::gui::chip_settings_state::ChipSettingsState;
use crate::gui::state::{SequencePlayheads, SharedSequences};
use crate::gui::wavesynth::draw_wavesynth_panel;
use crate::gui::wavesynth_state::WaveSynthPreview;
use crate::gui::wavetable_editor::draw_wavetable_editor_panel;
use crate::gui::wavetable_state::WavetableEditorState;
use crate::{ChannelMode, Lane};
use footer::draw_footer;
use header::draw_header;
use instrument_panel::draw_instrument_settings_panel;
use sequence_panel::{
    draw_sequence_editor_panel, sync_volume_step_mode_5b_to_channel,
    sync_volume_step_mode_to_channel,
};
use tabs::{TopLevelTab, draw_chip_tabs, fall_back_from_unavailable_tab};

pub use sequence_panel::{
    cleanup_tab_sequence, sanitize_sequence_text, sequence_to_text, sequence_to_text_for_tab,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct EditorResult {
    pub new_sequence_index: Option<usize>,

    pub new_step_time_hz: Option<i32>,

    pub new_channel_mode: Option<ChannelMode>,

    pub new_polyphony: Option<bool>,

    pub new_max_voices: Option<i32>,
    pub new_portamento_enabled: Option<bool>,
    pub new_portamento_speed: Option<i32>,
}

/// A per-repaint snapshot of the host parameters the editor displays and
/// edits. Built fresh from `Rp2a03Params` every frame and handed in by
/// value — the audio thread never touches `SharedSequences` for these, and
/// the editor never needs to read them back out; every change it makes is
/// already mirrored into the returned `EditorResult` as a host gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostParamsView {
    pub channel_mode: ChannelMode,
    pub polyphony: bool,
    pub max_voices: i32,
    pub portamento_enabled: bool,
    pub portamento_speed: i32,
}

const STATUS_DISPLAY_DURATION: std::time::Duration = std::time::Duration::from_secs(4);

#[derive(Default)]
pub struct EditorUiState {
    status: Option<(String, std::time::Instant)>,

    last_sequence_index: Option<usize>,

    pending_save: Option<std::sync::mpsc::Receiver<Option<String>>>,
    pending_load: Option<std::sync::mpsc::Receiver<Option<std::path::PathBuf>>>,

    /// Polyphony's value just before Portamento forced it off, so it can be
    /// restored when Portamento turns back off instead of the setting being
    /// silently lost. `None` means either Portamento is currently off, or
    /// Polyphony was already off when Portamento was turned on.
    polyphony_before_portamento: Option<bool>,

    top_tab: TopLevelTab,

    wavetable: WavetableEditorState,

    chip_settings: ChipSettingsState,

    wavesynth_preview: WaveSynthPreview,
}

impl EditorUiState {
    pub fn set(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), std::time::Instant::now()));
    }

    #[must_use]
    pub fn status_text(&self) -> Option<&str> {
        self.status
            .as_ref()
            .filter(|(_, set_at)| set_at.elapsed() < STATUS_DISPLAY_DURATION)
            .map(|(msg, _)| msg.as_str())
    }

    #[must_use]
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

fn apply_channel_mode_change(
    data: &mut SharedSequences,
    host: &mut HostParamsView,
    new_mode: ChannelMode,
    result: &mut EditorResult,
) {
    // Only Triangle and Noise ever force the selected tab off: Triangle drops
    // Duty, Noise drops Pitch/HiPitch. FDS also makes HiPitch unavailable but
    // has never force-reset the tab for it — preserved here rather than
    // folded in, since that's a pre-existing UI gap, not a rule this refactor
    // should silently start enforcing.
    if matches!(new_mode, ChannelMode::Triangle | ChannelMode::Noise)
        && !data.selected_tab.available_for(new_mode)
    {
        cleanup_tab_sequence(data, host.channel_mode, data.selected_tab);
        data.selected_tab = Lane::Vol;
    }

    host.channel_mode = new_mode;
    result.new_channel_mode = Some(new_mode);
}

/// Draws into a scratch clone of `data`'s current value and reports the new
/// value only if `draw` actually changed it. Every `SharedSequences` mutator
/// bumps the single revision counter on every call (`EditGuard`, M4 step
/// 23), so a panel that draws unconditionally every repaint — the wavetable
/// editor, the wave synthesizer, chip settings, the sequence editor — must
/// not commit back through that mutator unless something really changed;
/// this is the one place that "changed vs. not" check lives, instead of a
/// hand-rolled clone/compare/write-back at every call site.
fn commit_if_changed<T: Clone + PartialEq>(original: &T, draw: impl FnOnce(&mut T)) -> Option<T> {
    let mut value = original.clone();
    draw(&mut value);
    (value != *original).then_some(value)
}

fn recall_slot_waveform(
    data: &mut SharedSequences,
    host: &mut HostParamsView,
    shared_sequence_index: usize,
    last_index: &mut Option<usize>,
    result: &mut EditorResult,
) {
    // A slot that was never explicitly assigned a waveform (`None`) keeps
    // whatever chip is already playing, rather than forcing it back to
    // Pulse — the fallback `slot_waveform` used to return. Landing on such
    // a slot is common: a Program Change from a host/controller can select
    // any index, including ones only ever used for e.g. their volume
    // sequence, never their own dropdown pick.
    if last_index.is_some_and(|prev| prev != shared_sequence_index)
        && let Some(target_mode) = data.slot_waveform(shared_sequence_index)
        && target_mode != host.channel_mode
    {
        apply_channel_mode_change(data, host, target_mode, result);
    }
    *last_index = Some(shared_sequence_index);
}

fn draw_main_content(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    host: &mut HostParamsView,
    shared_sequence_index: usize,
    playheads: &SequencePlayheads,
    step_time_hz: u32,
    result: &mut EditorResult,
) {
    const LEFT_W: f32 = 180.0;
    const GAP: f32 = 8.0;
    const TOP_GAP: f32 = 10.0;

    let available = ui.available_rect_before_wrap();

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
        draw_instrument_settings_panel(ui, data, host, shared_sequence_index, step_time_hz, result);
    });

    ui.scope_builder(egui::UiBuilder::new().max_rect(right_rect), |ui| {
        draw_sequence_editor_panel(ui, data, host.channel_mode, playheads, step_time_hz);
    });
}

pub fn render_editor_ui(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    mut host: HostParamsView,
    shared_sequence_index: usize,
    playheads: &SequencePlayheads,
    step_time_hz: u32,
    ui_state: &mut EditorUiState,
) -> EditorResult {
    ui.painter().rect_filled(
        ui.max_rect(),
        egui::CornerRadius::ZERO,
        ui.visuals().panel_fill,
    );

    ui.set_min_height(ui.available_height());

    let mut result = EditorResult::default();

    data.set_all_selected_sequence_indices(shared_sequence_index);

    recall_slot_waveform(
        data,
        &mut host,
        shared_sequence_index,
        &mut ui_state.last_sequence_index,
        &mut result,
    );

    if host.channel_mode == ChannelMode::Noise
        && !data.selected_tab.available_for(host.channel_mode)
    {
        data.selected_tab = Lane::Vol;
    }

    sync_volume_step_mode_to_channel(data, host.channel_mode);

    sync_volume_step_mode_5b_to_channel(data, host.channel_mode);

    draw_header(
        ui,
        data,
        &mut host,
        &mut result,
        ui_state,
        step_time_hz,
        shared_sequence_index,
    );

    fall_back_from_unavailable_tab(&mut ui_state.top_tab, host.channel_mode);
    draw_chip_tabs(ui, &mut ui_state.top_tab, host.channel_mode);

    const FOOTER_HEIGHT: f32 = 26.0;
    const FOOTER_GAP: f32 = 10.0;
    let available = ui.available_rect_before_wrap();

    ui.allocate_rect(available, egui::Sense::hover());

    let footer_rect = egui::Rect::from_min_max(
        egui::pos2(available.min.x, available.max.y - FOOTER_HEIGHT),
        available.max,
    );
    let main_rect = egui::Rect::from_min_max(
        available.min,
        egui::pos2(available.max.x, footer_rect.min.y - FOOTER_GAP),
    );

    ui.scope_builder(egui::UiBuilder::new().max_rect(main_rect), |ui| {
        draw_top_level_tab_content(
            ui,
            data,
            &mut host,
            shared_sequence_index,
            playheads,
            step_time_hz,
            ui_state,
            &mut result,
        );
    });

    ui.scope_builder(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
        draw_footer(ui, ui_state);
    });

    result
}

/// The main-area content for whichever top-level tab is selected.
// Same 8 pieces of context `render_editor_ui` itself already threads through
// one level up; splitting further would need an artificial bundling struct
// for no real gain.
#[allow(clippy::too_many_arguments)]
fn draw_top_level_tab_content(
    ui: &mut egui::Ui,
    data: &mut SharedSequences,
    host: &mut HostParamsView,
    shared_sequence_index: usize,
    playheads: &SequencePlayheads,
    step_time_hz: u32,
    ui_state: &mut EditorUiState,
    result: &mut EditorResult,
) {
    match ui_state.top_tab {
        TopLevelTab::Envelopes => draw_main_content(
            ui,
            data,
            host,
            shared_sequence_index,
            playheads,
            step_time_hz,
            result,
        ),

        TopLevelTab::Wavetable => {
            if let Some(slots) = commit_if_changed(data.wave_slots(), |slots| {
                draw_wavetable_editor_panel(ui, &mut ui_state.wavetable, slots);
            }) {
                *data.wave_slots_mut() = slots;
            }
        }
        TopLevelTab::Synthesizer => {
            let lane_active = host.channel_mode == ChannelMode::Fds
                && data.sequence_enabled(Lane::Duty)
                && !data.selected_sequence(Lane::Duty).values.is_empty();

            let original = (
                data.wavesynth(shared_sequence_index),
                data.wave_slots().clone(),
            );
            if let Some((params, slots)) = commit_if_changed(&original, |(params, slots)| {
                draw_wavesynth_panel(
                    ui,
                    params,
                    &mut ui_state.wavesynth_preview,
                    slots,
                    lane_active,
                );
            }) {
                data.set_wavesynth(shared_sequence_index, params);
                *data.wave_slots_mut() = slots;
            }
        }

        TopLevelTab::ChipSettings => {
            let channel_mode = host.channel_mode;
            if let Some(fds_settings) =
                commit_if_changed(data.fds_settings(shared_sequence_index), |fds_settings| {
                    draw_chip_settings_panel(
                        ui,
                        &mut ui_state.chip_settings,
                        fds_settings,
                        channel_mode,
                    );
                })
            {
                *data.fds_settings_mut(shared_sequence_index) = fds_settings;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_params_view(channel_mode: ChannelMode) -> HostParamsView {
        HostParamsView {
            channel_mode,
            polyphony: false,
            max_voices: 1,
            portamento_enabled: false,
            portamento_speed: 0,
        }
    }

    #[test]
    fn landing_on_a_slot_that_was_never_assigned_a_waveform_keeps_the_current_chip() {
        let mut data = SharedSequences::default();
        let mut host = host_params_view(ChannelMode::Vrc6Saw);
        let mut result = EditorResult::default();
        let mut last_index = Some(0);

        recall_slot_waveform(&mut data, &mut host, 1, &mut last_index, &mut result);

        assert_eq!(host.channel_mode, ChannelMode::Vrc6Saw);
        assert_eq!(result.new_channel_mode, None);
    }

    #[test]
    fn landing_on_a_slot_with_an_explicitly_assigned_waveform_still_switches() {
        let mut data = SharedSequences::default();
        data.set_slot_waveform(1, ChannelMode::S5B);
        let mut host = host_params_view(ChannelMode::Vrc6Saw);
        let mut result = EditorResult::default();
        let mut last_index = Some(0);

        recall_slot_waveform(&mut data, &mut host, 1, &mut last_index, &mut result);

        assert_eq!(host.channel_mode, ChannelMode::S5B);
        assert_eq!(result.new_channel_mode, Some(ChannelMode::S5B));
    }
}
