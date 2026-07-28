use std::sync::Arc;

use egui::{Color32, Style};

pub const BG: Color32 = Color32::from_rgb(14, 14, 14);
pub const PANEL: Color32 = Color32::from_rgb(8, 8, 8);
pub const PANEL_ALT: Color32 = Color32::from_rgb(20, 20, 20);

pub const BORDER: Color32 = Color32::from_rgb(35, 35, 35);

pub const TEXT: Color32 = Color32::WHITE;
pub const TEXT_DIM: Color32 = Color32::from_rgb(90, 90, 90);

pub const ACCENT: Color32 = Color32::from_rgb(53, 94, 183);

pub const LOOP: Color32 = Color32::from_rgb(100, 200, 220);
pub const RELEASE: Color32 = Color32::from_rgb(200, 120, 220);
pub const LOOP_RELEASE: Color32 = Color32::from_rgb(230, 190, 40);
pub const PLAYHEAD_TOP: Color32 = Color32::from_rgb(160, 240, 160);
pub const PLAYHEAD_BOTTOM: Color32 = Color32::from_rgb(134, 220, 134);
pub const PLAYHEAD_EDGE_TOP: Color32 = Color32::from_rgb(198, 242, 198);
pub const PLAYHEAD_EDGE_BOTTOM: Color32 = Color32::from_rgb(106, 223, 106);

pub fn style() -> Arc<Style> {
    let mut style = Style::default();

    style.visuals.extreme_bg_color = BG;
    style.visuals.panel_fill = BG;
    style.visuals.override_text_color = Some(TEXT);

    style.visuals.popup_shadow.offset = [0, 0];
    style.visuals.popup_shadow.spread = 2;

    // Thicker widget strokes
    let stroke = 1.5;
    style.visuals.widgets.noninteractive.bg_stroke.width = stroke;
    style.visuals.widgets.inactive.bg_stroke.width = stroke;
    style.visuals.widgets.hovered.bg_stroke.width = stroke;
    style.visuals.widgets.active.bg_stroke.width = stroke;

    // Thicker window border
    style.visuals.window_stroke = egui::Stroke::new(1.5_f32, BORDER);

    // Font sizes
    use egui::{FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(20.0)),
        (TextStyle::Body, FontId::proportional(15.0)),
        (TextStyle::Button, FontId::proportional(15.0)),
        (TextStyle::Small, FontId::proportional(10.0)),
        (TextStyle::Monospace, FontId::monospace(12.0)),
    ]
    .into();

    style.visuals.widgets.noninteractive.corner_radius = 2.0.into();
    style.visuals.widgets.inactive.corner_radius = 2.0.into();
    style.visuals.widgets.hovered.corner_radius = 2.0.into();
    style.visuals.widgets.active.corner_radius = 2.0.into();

    style.visuals.window_corner_radius = 10.0.into();

    Arc::new(style)
}
