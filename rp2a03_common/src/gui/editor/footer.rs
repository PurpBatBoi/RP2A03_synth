//! `rp2a03_common\src\gui\editor\footer.rs`
//! The version number and the transient status line (save/load results).

use super::EditorUiState;

pub(super) fn draw_footer(ui: &mut egui::Ui, ui_state: &EditorUiState) {
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
