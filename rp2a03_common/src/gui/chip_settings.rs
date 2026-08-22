//! `rp2a03_common\src\gui\chip_settings.rs`

use super::chip_settings_state::{
    ChipSettingsState, FDS_MOD_DELAY_MAX, FDS_MOD_DEPTH_MAX, FDS_MOD_SPEED_MAX, FDS_MOD_TABLE_LEN,
    FDS_MOD_TABLE_MAX, FDS_MOD_TABLE_MIN, FdsSettings, N163_MAX_CHANNELS, decode_mod_table_text,
    encode_mod_table_text,
};
use super::widgets::draw_mod_table_graph;
use crate::ChannelMode;

const MOD_TABLE_HEIGHT: f32 = 170.0;

pub fn draw_chip_settings_panel(
    ui: &mut egui::Ui,
    state: &mut ChipSettingsState,
    fds: &mut FdsSettings,
    channel_mode: ChannelMode,
) {
    ui.add_space(10.0);

    match channel_mode {
        ChannelMode::Fds => draw_fds_fields(ui, state, fds),

        _ => draw_n163_fields(ui, state),
    }
}

fn draw_fds_fields(ui: &mut egui::Ui, state: &mut ChipSettingsState, fds: &mut FdsSettings) {
    egui::Grid::new("fds_fields")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Modulation Depth");
            ui.add(egui::DragValue::new(&mut fds.mod_depth).range(0..=FDS_MOD_DEPTH_MAX));
            ui.end_row();

            ui.label("Modulation Speed");
            ui.add(egui::DragValue::new(&mut fds.mod_speed).range(0..=FDS_MOD_SPEED_MAX));
            ui.end_row();

            ui.label("Modulation Delay");
            ui.add(egui::DragValue::new(&mut fds.mod_delay).range(0..=FDS_MOD_DELAY_MAX))
                .on_hover_text(
                    "Engine frames of no modulation after each note-on, so the \
                     note starts clean and blooms into the wobble. Measured in \
                     frames, so its length follows Step Time.",
                );
            ui.end_row();
        });

    ui.add_space(8.0);

    ui.label("Modulation Table").on_hover_text(
        "32 steps, each written to two of the chip's 64 entries.\n\
         −4 is the RESET entry: it snaps modulation back to centre, \
         rather than being the deepest downward step.",
    );

    draw_mod_table_graph(
        ui,
        &mut fds.mod_table,
        FDS_MOD_TABLE_MIN,
        FDS_MOD_TABLE_MAX,
        MOD_TABLE_HEIGHT,
    );

    fds.mod_table.values.resize(FDS_MOD_TABLE_LEN, 0);

    ui.add_space(6.0);
    let edit = ui.add(
        egui::TextEdit::singleline(&mut state.mod_table_text)
            .desired_width(ui.available_width())
            .font(egui::TextStyle::Monospace),
    );

    if edit.changed() {
        let parsed = decode_mod_table_text(&state.mod_table_text);
        fds.mod_table.values = parsed;
        fds.mod_table.loop_point = None;
        fds.mod_table.release_point = None;
    }

    if !edit.has_focus() {
        state.mod_table_text = encode_mod_table_text(&fds.mod_table.values);
    }
}

fn draw_n163_fields(ui: &mut egui::Ui, state: &mut ChipSettingsState) {
    ui.label(
        egui::RichText::new(
            "Placeholder fields — nothing here affects audio yet. \
             They exist so the panel is ready when N163 lands.",
        )
        .weak(),
    );
    ui.add_space(8.0);

    let n163 = &mut state.n163;

    egui::Grid::new("n163_fields")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Emulated Channels");
            ui.add(egui::DragValue::new(&mut n163.channels).range(1..=N163_MAX_CHANNELS))
                .on_hover_text(
                    "Real hardware time-slices one DAC across the active channels, \
                     so more channels means a lower per-channel sample rate — \
                     grittier by design, not by emulation error.",
                );
            ui.end_row();

            ui.label("High Quality Mixer");
            ui.checkbox(&mut n163.high_quality_mixer, "").on_hover_text(
                "Off (default) is the authentic multiplexed output. \
                 On sums every channel for a cleaner, less accurate signal.",
            );
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(
            "Jitter Alias is not implemented: no vendored N163 core describes \
             the mechanism. Tracked as a blocker in PROGRESS.md §4.",
        )
        .weak(),
    );
}
