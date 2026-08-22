//! `rp2a03_common\src\gui\editor\tabs.rs`
//! The top-level tab bar (Envelope Editors / Wavetable Editor / Wave
//! Synthesizer / Chip Settings) and which tabs a given chip can show.

use crate::ChannelMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum TopLevelTab {
    #[default]
    Envelopes,
    Wavetable,
    Synthesizer,
    ChipSettings,
}

pub(super) const WAVETABLE_ONLY_TABS: [TopLevelTab; 3] = [
    TopLevelTab::Wavetable,
    TopLevelTab::Synthesizer,
    TopLevelTab::ChipSettings,
];

fn is_wavetable_chip(mode: ChannelMode) -> bool {
    match mode {
        ChannelMode::Fds => true,
        ChannelMode::Pulse
        | ChannelMode::Triangle
        | ChannelMode::Noise
        | ChannelMode::Vrc6Pulse
        | ChannelMode::Vrc6Saw
        | ChannelMode::S5B => false,
    }
}

pub(super) fn fall_back_from_unavailable_tab(top_tab: &mut TopLevelTab, channel_mode: ChannelMode) {
    if !is_wavetable_chip(channel_mode) && WAVETABLE_ONLY_TABS.contains(top_tab) {
        *top_tab = TopLevelTab::Envelopes;
    }
}

pub(super) fn draw_chip_tabs(
    ui: &mut egui::Ui,
    top_tab: &mut TopLevelTab,
    channel_mode: ChannelMode,
) {
    const TABS: &[(TopLevelTab, &str)] = &[
        (TopLevelTab::Envelopes, "Envelope Editors"),
        (TopLevelTab::Wavetable, "Wavetable Editor"),
        (TopLevelTab::Synthesizer, "Wave Synthesizer"),
        (TopLevelTab::ChipSettings, "Chip Settings"),
    ];

    let wavetable_chip = is_wavetable_chip(channel_mode);

    ui.horizontal(|ui| {
        for &(tab, label) in TABS {
            let enabled = wavetable_chip || !WAVETABLE_ONLY_TABS.contains(&tab);

            ui.add_enabled_ui(enabled, |ui| {
                let response = ui.selectable_label(*top_tab == tab, label);
                if response.clicked() {
                    *top_tab = tab;
                }
                if !enabled {
                    response.on_hover_text("Available for wavetable chips (FDS)");
                }
            });
        }
    });
    ui.add_space(5.0);
    ui.separator();
}
