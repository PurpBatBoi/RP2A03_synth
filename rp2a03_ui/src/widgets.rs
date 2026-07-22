//! rp2a03_ui\src\widgets.rs
//! 
//! Custom painter elements for sequence visualization.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use rp2a03_core::sequence::Sequence;

/// Renders a FamiTracker-style envelope bar graph with unipolar and bipolar range support.
pub fn draw_envelope_bar_graph(ui: &mut egui::Ui, seq: &Sequence, min_val: i16, max_val: i16) {
    let desired_size = Vec2::new(450.0, 220.0);
    let (rect, _response) = ui.allocate_at_least(desired_size, Sense::hover());

    let painter = ui.painter_at(rect);

    // Background panel
    painter.rect_filled(rect, 2.0, Color32::from_rgb(8, 8, 8));
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0f32, Color32::from_rgb(35, 35, 35)),
        egui::StrokeKind::Outside,
    );

    let num_steps = seq.len();
    if num_steps == 0 {
        return;
    }

    let header_height = 20.0;
    let graph_rect = Rect::from_min_max(
        rect.min,
        Pos2::new(rect.max.x, rect.max.y - header_height),
    );
    let header_rect = Rect::from_min_max(
        Pos2::new(rect.min.x, rect.max.y - header_height),
        rect.max,
    );

    let step_width = graph_rect.width() / num_steps as f32;
    let loop_idx = seq.loop_point.unwrap_or(usize::MAX);
    let rel_idx = seq.release_point.unwrap_or(usize::MAX);

    let is_bipolar = min_val < 0;

    // Zero-axis Y offset calculation
    let zero_y = if is_bipolar {
        let range = (max_val as f32 - min_val as f32).max(1.0);
        let norm_zero = (0.0 - min_val as f32) / range;
        graph_rect.max.y - (norm_zero * graph_rect.height())
    } else {
        graph_rect.max.y
    };

    if is_bipolar {
        painter.line_segment(
            [
                Pos2::new(graph_rect.min.x, zero_y),
                Pos2::new(graph_rect.max.x, zero_y),
            ],
            Stroke::new(1.0f32, Color32::from_rgb(60, 60, 60)),
        );
    }

    // Draw individual steps
    for i in 0..num_steps {
        let val = seq.values[i].clamp(min_val, max_val);

        let bar_x_min = graph_rect.min.x + i as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0;

        let col_rect = Rect::from_min_max(
            Pos2::new(bar_x_min, graph_rect.min.y),
            Pos2::new(bar_x_max, graph_rect.max.y),
        );
        let bg_color = if i % 2 == 0 {
            Color32::from_rgb(14, 14, 14)
        } else {
            Color32::from_rgb(20, 20, 20)
        };
        painter.rect_filled(col_rect, 0.0, bg_color);

        let bar_rect = if is_bipolar {
            let range = (max_val as f32 - min_val as f32).max(1.0);
            let norm_val = (val as f32 - min_val as f32) / range;
            let bar_y = graph_rect.max.y - (norm_val * graph_rect.height());
            if val >= 0 {
                Rect::from_min_max(Pos2::new(bar_x_min, bar_y), Pos2::new(bar_x_max, zero_y))
            } else {
                Rect::from_min_max(Pos2::new(bar_x_min, zero_y), Pos2::new(bar_x_max, bar_y))
            }
        } else {
            let range = max_val.max(1) as f32;
            let norm_val = val as f32 / range;
            let bar_y_min = graph_rect.max.y - (norm_val * graph_rect.height());
            Rect::from_min_max(
                Pos2::new(bar_x_min, bar_y_min),
                Pos2::new(bar_x_max, graph_rect.max.y),
            )
        };

        let is_loop_release_mode = loop_idx < num_steps && rel_idx == loop_idx;

        let bar_color = if is_loop_release_mode && i >= loop_idx {
            Color32::from_rgb(230, 190, 40)
        } else if i >= rel_idx {
            Color32::from_rgb(200, 120, 220)
        } else if i >= loop_idx {
            Color32::from_rgb(100, 200, 220)
        } else {
            Color32::from_rgb(220, 220, 220)
        };

        if val != 0 || !is_bipolar {
            painter.rect_filled(bar_rect, 1.0, bar_color);
        }
    }

    // Render loop/release region headers
    painter.rect_filled(header_rect, 0.0, Color32::from_rgb(25, 25, 25));

    let is_loop_release_mode = loop_idx < num_steps && rel_idx == loop_idx;

    if is_loop_release_mode {
        let x_min = header_rect.min.x + loop_idx as f32 * step_width;
        let x_max = header_rect.max.x;
        let lr_rect = Rect::from_min_max(
            Pos2::new(x_min, header_rect.min.y),
            Pos2::new(x_max, header_rect.max.y),
        );
        painter.rect_filled(lr_rect, 0.0, Color32::from_rgb(180, 140, 20));
        painter.text(
            Pos2::new(x_min + 4.0, header_rect.min.y + 2.0),
            egui::Align2::LEFT_TOP,
            "Loop, Release",
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
    } else {
        let loop_end = if rel_idx < usize::MAX {
            rel_idx
        } else {
            num_steps
        };

        if loop_idx < num_steps {
            let x_min = header_rect.min.x + loop_idx as f32 * step_width;
            let x_max = header_rect.min.x + loop_end as f32 * step_width;
            let l_rect = Rect::from_min_max(
                Pos2::new(x_min, header_rect.min.y),
                Pos2::new(x_max, header_rect.max.y),
            );
            painter.rect_filled(l_rect, 0.0, Color32::from_rgb(0, 120, 130));
            painter.text(
                Pos2::new(x_min + 4.0, header_rect.min.y + 2.0),
                egui::Align2::LEFT_TOP,
                "Loop",
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
        }

        if rel_idx < num_steps {
            let x_min = header_rect.min.x + rel_idx as f32 * step_width;
            let x_max = header_rect.max.x;
            let r_rect = Rect::from_min_max(
                Pos2::new(x_min, header_rect.min.y),
                Pos2::new(x_max, header_rect.max.y),
            );
            painter.rect_filled(r_rect, 0.0, Color32::from_rgb(120, 0, 130));
            painter.text(
                Pos2::new(x_min + 4.0, header_rect.min.y + 2.0),
                egui::Align2::LEFT_TOP,
                "Release",
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
        }
    }

    // Min / Max labels
    painter.text(
        Pos2::new(graph_rect.min.x + 6.0, graph_rect.min.y + 2.0),
        egui::Align2::LEFT_TOP,
        format!("{}", max_val),
        egui::FontId::proportional(11.0),
        Color32::from_rgb(160, 160, 160),
    );
    painter.text(
        Pos2::new(graph_rect.min.x + 6.0, graph_rect.max.y - 14.0),
        egui::Align2::LEFT_TOP,
        format!("{}", min_val),
        egui::FontId::proportional(11.0),
        Color32::from_rgb(160, 160, 160),
    );
}