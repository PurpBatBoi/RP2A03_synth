//! `rp2a03_common\src\gui\widgets\common.rs`
//! Drag-state types and pointer-to-step helpers shared by more than one
//! step-graph widget (`step_graph`, `s5b_graph`, `wavetable_graph`).

use crate::gui::theme;
use egui::{Color32, Pos2, Rect, Stroke, Vec2};
use rp2a03_core::sequencer::Sequence;

#[derive(Clone, Copy, Default)]
pub(super) struct MarkerDragState {
    pub(super) start_step: usize,
    pub(super) was_existing: bool,
}

#[derive(Clone, Copy, Default)]
pub(super) struct LineDrawState {
    pub(super) start: Pos2,
    pub(super) last: Pos2,

    pub(super) tension: f32,
}

fn apply_tension(t: f32, tension: f32) -> f32 {
    (t + tension * t * (1.0 - t)).clamp(0.0, 1.0)
}

pub(super) fn for_each_step_between(
    region: Rect,
    step_width: f32,
    num_steps: usize,
    p0: Pos2,
    p1: Pos2,
    tension: f32,
    mut set_value: impl FnMut(usize, f32),
) {
    let step_of = |x: f32| -> usize {
        let rel_x = (x - region.min.x).clamp(0.0, (region.width() - 0.001).max(0.0));
        (((rel_x / step_width).floor() as i32).clamp(0, num_steps as i32 - 1)) as usize
    };

    let step0 = step_of(p0.x);
    let step1 = step_of(p1.x);
    let min_step = step0.min(step1);
    let max_step = step0.max(step1);
    let dx = p1.x - p0.x;

    for s in min_step..=max_step {
        let target_y = if dx.abs() < 1e-4 {
            p1.y
        } else {
            let s_x = region.min.x + (s as f32 + 0.5) * step_width;
            let t = ((s_x - p0.x) / dx).clamp(0.0, 1.0);
            let curved_t = apply_tension(t, tension);
            p0.y + curved_t * (p1.y - p0.y)
        };
        set_value(s, target_y);
    }
}

pub(super) fn draw_line_draw_preview(painter: &egui::Painter, state: Option<LineDrawState>) {
    let Some(state) = state else { return };

    let preview_stroke = Stroke::new(4.0f32, Color32::from_rgba_unmultiplied(255, 255, 255, 200));
    const PREVIEW_SEGMENTS: u32 = 24;
    let mut prev = state.start;
    for i in 1..=PREVIEW_SEGMENTS {
        let t = i as f32 / PREVIEW_SEGMENTS as f32;
        let curved_t = apply_tension(t, state.tension);
        let next = Pos2::new(
            state.start.x + (state.last.x - state.start.x) * t,
            state.start.y + (state.last.y - state.start.y) * curved_t,
        );
        painter.line_segment([prev, next], preview_stroke);
        prev = next;
    }

    if state.tension.abs() > 0.01f32 {
        painter.text(
            state.last + Vec2::new(8.0f32, -8.0f32),
            egui::Align2::LEFT_BOTTOM,
            format!("tension {:+.2}", state.tension),
            egui::FontId::proportional(11.0f32),
            Color32::from_rgb(255, 220, 120),
        );
    }
}

/// Loop/release marker drag-and-click handling in a step graph's header
/// strip: primary button drags/places the loop marker, secondary the release
/// marker: a single click toggles a marker, a drag places it wherever the
/// drag ends. Shared by `step_graph` and `s5b_graph`, whose header behavior
/// is identical. Returns whether either marker moved.
pub(super) fn handle_marker_drag(
    ui: &egui::Ui,
    seq: &mut Sequence,
    header_response: &egui::Response,
    header_rect: Rect,
    num_steps: usize,
    loop_drag_id: egui::Id,
    rel_drag_id: egui::Id,
) -> bool {
    let step_width = header_rect.width() / num_steps as f32;

    let pointer_pos = header_response
        .interact_pointer_pos()
        .or_else(|| ui.input(|i| i.pointer.hover_pos()));

    let Some(pos) = pointer_pos else {
        return false;
    };

    let x = pos.x.clamp(header_rect.min.x, header_rect.max.x - 1.0f32);
    let current_step =
        (((x - header_rect.min.x) / step_width).floor() as usize).clamp(0, num_steps - 1);

    let loop_clicked = header_response.clicked_by(egui::PointerButton::Primary);
    let mut text_needs_sync = handle_one_marker(
        ui,
        egui::PointerButton::Primary,
        loop_clicked,
        header_response,
        loop_drag_id,
        &mut seq.loop_point,
        current_step,
    );

    let release_clicked = header_response.clicked_by(egui::PointerButton::Secondary)
        || header_response.secondary_clicked();
    text_needs_sync |= handle_one_marker(
        ui,
        egui::PointerButton::Secondary,
        release_clicked,
        header_response,
        rel_drag_id,
        &mut seq.release_point,
        current_step,
    );

    text_needs_sync
}

/// One marker's (loop or release) drag-to-place and click-to-toggle
/// behavior, shared by `handle_marker_drag`'s two symmetric call sites.
fn handle_one_marker(
    ui: &egui::Ui,
    button: egui::PointerButton,
    clicked: bool,
    header_response: &egui::Response,
    drag_id: egui::Id,
    marker: &mut Option<usize>,
    current_step: usize,
) -> bool {
    let mut text_needs_sync = false;

    if header_response.drag_started_by(button) {
        let was_existing = *marker == Some(current_step);
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                drag_id,
                MarkerDragState {
                    start_step: current_step,
                    was_existing,
                },
            );
        });
        if !was_existing {
            *marker = Some(current_step);
            text_needs_sync = true;
        }
    } else if header_response.dragged_by(button) {
        let state: Option<MarkerDragState> = ui.ctx().data_mut(|d| d.get_temp(drag_id));
        if let Some(st) = state
            && (current_step != st.start_step || !st.was_existing)
            && *marker != Some(current_step)
        {
            *marker = Some(current_step);
            text_needs_sync = true;
        }
    }

    if clicked {
        let state: Option<MarkerDragState> = ui.ctx().data_mut(|d| d.get_temp(drag_id));
        if let Some(st) = state {
            if current_step == st.start_step && st.was_existing {
                *marker = None;
                text_needs_sync = true;
            } else if !st.was_existing {
                *marker = Some(current_step);
                text_needs_sync = true;
            }
        } else {
            *marker = if *marker == Some(current_step) {
                None
            } else {
                Some(current_step)
            };
            text_needs_sync = true;
        }
        ui.ctx()
            .data_mut(|d| d.remove_temp::<MarkerDragState>(drag_id));
    }

    text_needs_sync
}

/// The loop/release marker labels and colored bands in a step graph's header
/// strip. Shared by `step_graph` and `s5b_graph`.
pub(super) fn paint_marker_header(
    painter: &egui::Painter,
    header_rect: Rect,
    step_width: f32,
    num_steps: usize,
    loop_idx: usize,
    rel_idx: usize,
) {
    painter.rect_filled(header_rect, 0.0f32, Color32::from_rgb(25, 25, 25));

    let has_loop = loop_idx < num_steps;
    let has_release = rel_idx < num_steps;

    if has_loop && has_release && loop_idx == rel_idx {
        let x_min = header_rect.min.x + loop_idx as f32 * step_width;
        let x_max = header_rect.max.x;
        let lr_rect = Rect::from_min_max(
            Pos2::new(x_min, header_rect.min.y),
            Pos2::new(x_max, header_rect.max.y),
        );
        painter.rect_filled(lr_rect, 0.0f32, Color32::from_rgb(180, 140, 20));
        painter.text(
            Pos2::new(x_min + 4.0f32, header_rect.min.y + 2.0f32),
            egui::Align2::LEFT_TOP,
            "Loop, Release",
            egui::FontId::proportional(12.0f32),
            Color32::WHITE,
        );
    } else {
        if has_loop {
            let loop_start = loop_idx;
            let loop_end = if has_release && loop_idx < rel_idx {
                rel_idx
            } else {
                num_steps
            };
            let x_min = header_rect.min.x + loop_start as f32 * step_width;
            let x_max = header_rect.min.x + loop_end as f32 * step_width;
            let l_rect = Rect::from_min_max(
                Pos2::new(x_min, header_rect.min.y),
                Pos2::new(x_max, header_rect.max.y),
            );
            painter.rect_filled(l_rect, 0.0f32, Color32::from_rgb(0, 120, 130));
            painter.text(
                Pos2::new(x_min + 4.0f32, header_rect.min.y + 2.0f32),
                egui::Align2::LEFT_TOP,
                "Loop",
                egui::FontId::proportional(12.0f32),
                Color32::WHITE,
            );
        }

        if has_release {
            let rel_start = rel_idx;
            let rel_end = if has_loop && rel_idx < loop_idx {
                loop_idx
            } else {
                num_steps
            };
            let x_min = header_rect.min.x + rel_start as f32 * step_width;
            let x_max = header_rect.min.x + rel_end as f32 * step_width;
            let r_rect = Rect::from_min_max(
                Pos2::new(x_min, header_rect.min.y),
                Pos2::new(x_max, header_rect.max.y),
            );
            painter.rect_filled(r_rect, 0.0f32, Color32::from_rgb(120, 0, 130));
            painter.text(
                Pos2::new(x_min + 4.0f32, header_rect.min.y + 2.0f32),
                egui::Align2::LEFT_TOP,
                "Release",
                egui::FontId::proportional(12.0f32),
                Color32::WHITE,
            );
        }
    }
}

pub(super) fn draw_playhead_rect(painter: &egui::Painter, rect: Rect) {
    let rect = rect.shrink2(Vec2::new(1.0f32, 1.0f32));
    if rect.is_negative() {
        return;
    }

    let top_rect = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.center().y));
    let bottom_rect = Rect::from_min_max(Pos2::new(rect.min.x, rect.center().y), rect.max);

    painter.rect_filled(top_rect, 1.0f32, theme::PLAYHEAD_TOP);
    painter.rect_filled(bottom_rect, 1.0f32, theme::PLAYHEAD_BOTTOM);
    painter.line_segment(
        [rect.left_top(), rect.right_top()],
        Stroke::new(1.0f32, theme::PLAYHEAD_EDGE_TOP),
    );
    painter.line_segment(
        [rect.left_top(), rect.left_bottom()],
        Stroke::new(1.0f32, theme::PLAYHEAD_EDGE_TOP),
    );
    painter.line_segment(
        [rect.right_top(), rect.right_bottom()],
        Stroke::new(1.0f32, theme::PLAYHEAD_EDGE_BOTTOM),
    );
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0f32, theme::PLAYHEAD_EDGE_BOTTOM),
    );
}
