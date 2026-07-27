//! rp2a03_common\src\gui\widgets.rs
//!
//! Custom painter elements for sequence visualization and interactive envelope editing.

use egui::{
    Color32,
    Pos2,
    Rect,
    Sense,
    Stroke,
    Vec2,
};
use rp2a03_core::sequencer::Sequence;

/// Renders a FamiTracker-style envelope bar graph and handles interactive mouse editing.
/// Returns `true` when step values or markers change so text representation can be synchronized.
pub fn draw_envelope_bar_graph(
    ui: &mut egui::Ui,
    seq: &mut Sequence,
    min_val: i16,
    max_val: i16,
) -> bool {
    let desired_size = Vec2::new(ui.available_width(), 220.0);
    let (rect, response) = ui.allocate_at_least(desired_size, Sense::click_and_drag());

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

    let header_height = 20.0;
    let graph_rect =
        Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.max.y - header_height));
    let header_rect =
        Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - header_height), rect.max);

    let mut text_needs_sync = false;

    // Handle mouse interactions when sequence is non-empty
    if num_steps > 0 {
        let step_width = graph_rect.width() / num_steps as f32;

        if let Some(pointer_pos) = response.interact_pointer_pos() {
            let is_header_click = pointer_pos.y >= graph_rect.max.y;

            if is_header_click {
                // Header interaction: Loop / Release point toggling
                if response.clicked() {
                    let clicked_step = (((pointer_pos.x - header_rect.min.x) / step_width).floor()
                        as usize)
                        .clamp(0, num_steps - 1);

                    if seq.loop_point == Some(clicked_step) {
                        seq.loop_point = None;
                    } else {
                        seq.loop_point = Some(clicked_step);
                    }
                    text_needs_sync = true;
                } else if response.secondary_clicked() {
                    let clicked_step = (((pointer_pos.x - header_rect.min.x) / step_width).floor()
                        as usize)
                        .clamp(0, num_steps - 1);

                    if seq.release_point == Some(clicked_step) {
                        seq.release_point = None;
                    } else {
                        seq.release_point = Some(clicked_step);
                    }
                    text_needs_sync = true;
                }
            } else {
                // Bar graph area interaction: Real-time envelope drawing
                if response.dragged_by(egui::PointerButton::Primary)
                    || response.clicked_by(egui::PointerButton::Primary)
                {
                    let step_idx = (((pointer_pos.x - graph_rect.min.x) / step_width).floor()
                        as i32)
                        .clamp(0, num_steps as i32 - 1) as usize;

                    let rel_y = (pointer_pos.y - graph_rect.min.y).clamp(0.0, graph_rect.height());
                    let norm_y = rel_y / graph_rect.height();
                    let raw_val = max_val as f32 - norm_y * (max_val as f32 - min_val as f32);
                    let mut new_val = raw_val.round() as i16;

                    // Zero-axis snap for bipolar sequences
                    if min_val < 0 {
                        let snap_threshold = (max_val as f32 - min_val as f32) * 0.03;
                        if raw_val.abs() <= snap_threshold {
                            new_val = 0;
                        }
                    }

                    let clamped_val = new_val.clamp(min_val, max_val);
                    if seq.values[step_idx] != clamped_val {
                        seq.values[step_idx] = clamped_val;
                        text_needs_sync = true;
                    }
                }
            }
        }

        if response.drag_stopped_by(egui::PointerButton::Primary)
            || response.clicked_by(egui::PointerButton::Primary)
        {
            text_needs_sync = true;
        }
    }

    if num_steps == 0 {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Empty Sequence (0 steps)",
            egui::FontId::proportional(13.0),
            Color32::from_rgb(90, 90, 90),
        );
        return text_needs_sync;
    }

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

    // Draw step column backgrounds
    for i in 0..num_steps {
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
    }

    // Draw zero-axis reference line for bipolar graphs
    if is_bipolar {
        painter.line_segment(
            [
                Pos2::new(graph_rect.min.x, zero_y),
                Pos2::new(graph_rect.max.x, zero_y),
            ],
            Stroke::new(1.0f32, Color32::from_rgb(90, 90, 90)),
        );
    }

    // Draw step bars
    for i in 0..num_steps {
        let val = seq.values[i].clamp(min_val, max_val);

        let bar_x_min = graph_rect.min.x + i as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0;

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

    text_needs_sync
}


pub fn group_box<R>(
    ui: &mut egui::Ui,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    const TITLE_HEIGHT: i8 = 10;
    const PADDING: i8 = 10;
    const ROUNDING: u8 = 8;

    let frame = egui::Frame::new()
        .inner_margin(egui::Margin {
            left: PADDING,
            right: PADDING,
            top: PADDING + TITLE_HEIGHT,
            bottom: PADDING,
        });

    let inner = frame.show(ui, add_contents);

    let rect = inner.response.rect;

    let painter = ui.painter();

    let stroke = egui::Stroke::new(
        1.0_f32,
        ui.visuals().widgets.noninteractive.bg_stroke.color,
    );

    let galley = painter.layout_no_wrap(
        title.to_owned(),
        egui::FontId::proportional(16.0),
        ui.visuals().text_color(),
    );

    let title_pos = egui::pos2(rect.left() + 10.0, rect.top() - galley.size().y * 0.5);

    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(ROUNDING),
        stroke,
        egui::StrokeKind::Outside,
    );

    // Erase only behind the title
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(title_pos.x - 4.0, rect.top() - 2.0),
            egui::pos2(title_pos.x + galley.size().x + 4.0, rect.top() + 2.0),
        ),
        egui::CornerRadius::ZERO,
        ui.visuals().panel_fill,
    );

    painter.galley(title_pos, galley, ui.visuals().text_color());

    inner.inner
}