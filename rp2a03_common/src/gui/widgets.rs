//! rp2a03_common\src\gui\widgets.rs
//!
//! Custom painter elements for sequence visualization and interactive envelope editing.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use rp2a03_core::sequencer::{
    S5B_MODE_NOISE, S5B_MODE_SQUARE, S5B_PERIOD_MASK, Sequence, s5b_duty_index, s5b_set_duty_index,
};

use super::theme;

#[derive(Clone, Copy, Default)]
struct RepeatingButtonState {
    start_time: f64,
    last_trigger_time: f64,
}

#[derive(Clone, Copy, Default)]
struct MarkerDragState {
    start_step: usize,
    was_existing: bool,
}

#[derive(Clone, Copy, Default)]
struct LineDrawState {
    start: Pos2,
    last: Pos2,
    /// Curvature applied to the drawn ramp, in `-1.0..=1.0`. `0.0` is a
    /// straight line; positive bows the curve toward an ease-out (convex)
    /// shape, negative toward an ease-in (concave) shape. Adjusted by
    /// scrolling the mouse wheel while the line is being drawn.
    tension: f32,
}

/// Warps a normalized `0.0..=1.0` position along a drawn line by `tension`
/// (`-1.0..=1.0`) to produce a curved ramp instead of a straight one. `t` and
/// the endpoints are unaffected; only the interior of the curve bows toward
/// or away from the line's start.
fn apply_tension(t: f32, tension: f32) -> f32 {
    (t + tension * t * (1.0 - t)).clamp(0.0, 1.0)
}

/// Walks every step column between `p0.x` and `p1.x` (inclusive), calling
/// `set_value` with each step index and the y position interpolated between
/// `p0`/`p1` at that column's x-center (optionally warped by `tension` via
/// [`apply_tension`] for a curved ramp instead of a straight one).
///
/// Shared by every click-and-drag graph editor in this module so a fast drag
/// that jumps several step columns between two frames doesn't skip the ones
/// the pointer passed over — filling only the single column under the current
/// pointer position (no interpolation) is the bug this fixes when `p0 != p1`.
/// `p0` is normally the previous frame's pointer position and `p1` the
/// current one; passing `p0 == p1` degenerates to touching just one step,
/// which is also the correct behavior for a plain (non-dragged) click.
fn for_each_step_between(
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

/// Draws the in-progress right-click line-draw preview (a curved or straight
/// stroke from `state.start` to `state.last`, plus a tension readout once
/// bowed away from straight). Shared by every graph editor's line-draw
/// feature; no-op when `state` is `None` (no line draw in progress).
fn draw_line_draw_preview(painter: &egui::Painter, state: Option<LineDrawState>) {
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

/// Renders a FamiTracker-style envelope bar graph and handles interactive mouse editing.
/// Returns `true` when step values or markers change so text representation can be synchronized.
pub fn draw_envelope_bar_graph(
    ui: &mut egui::Ui,
    seq: &mut Sequence,
    min_val: i16,
    max_val: i16,
    is_arpeggio: bool,
    playhead_step: Option<usize>,
    graph_height: f32,
) -> bool {
    let desired_size = Vec2::new(ui.available_width(), graph_height);
    let (rect, _response) = ui.allocate_at_least(desired_size, Sense::hover());

    let painter = ui.painter_at(rect);

    // Background panel
    painter.rect_filled(rect, 2.0f32, Color32::from_rgb(8, 8, 8));
    painter.rect_stroke(
        rect,
        2.0f32,
        Stroke::new(1.0f32, Color32::from_rgb(35, 35, 35)),
        egui::StrokeKind::Outside,
    );

    let mut num_steps = seq.len();

    // Arpeggio scroll state handling: +/- 10 semitones visible (-10 to +10, 21 rows)
    let visible_span = 10i16;
    let min_center = (min_val + visible_span).min(0);
    let max_center = (max_val - visible_span).max(0);

    let scroll_id = ui.make_persistent_id("arpeggio_scroll_center");
    let mut scroll_center: i16 = ui.ctx().data_mut(|d| d.get_temp(scroll_id)).unwrap_or(0i16);
    if is_arpeggio {
        scroll_center = scroll_center.clamp(min_center, max_center);
    }

    let scrollbar_width = if is_arpeggio { 14.0f32 } else { 0.0f32 };
    let header_height = 20.0f32;

    let graph_rect = Rect::from_min_max(
        rect.min,
        Pos2::new(rect.max.x - scrollbar_width, rect.max.y - header_height),
    );

    let scrollbar_rect = Rect::from_min_max(
        Pos2::new(rect.max.x - scrollbar_width, rect.min.y),
        Pos2::new(rect.max.x, rect.max.y - header_height),
    );
    let header_rect = Rect::from_min_max(
        Pos2::new(rect.min.x, rect.max.y - header_height),
        Pos2::new(rect.max.x - scrollbar_width, rect.max.y),
    );

    // Sub-region interactive responses
    let graph_response = ui.interact(
        graph_rect,
        ui.make_persistent_id("arpeggio_graph_area"),
        Sense::click_and_drag(),
    );
    let header_response = ui.interact(
        header_rect,
        ui.make_persistent_id("arpeggio_header_area"),
        Sense::click_and_drag(),
    );
    let scrollbar_response = if is_arpeggio {
        ui.interact(
            scrollbar_rect,
            ui.make_persistent_id("arpeggio_scrollbar_area"),
            Sense::click_and_drag(),
        )
    } else {
        ui.interact(
            Rect::NOTHING,
            ui.make_persistent_id("arpeggio_scrollbar_dummy"),
            Sense::hover(),
        )
    };

    // An empty envelope has no step columns to draw into. Start it with a
    // small, useful canvas when the user begins drawing directly on the graph.
    const MIN_DRAW_STEPS: usize = 5;
    // Right-click line drawing needs at least two distinct steps to form a
    // visible envelope shape, so give it a larger starting canvas.
    const MIN_LINE_DRAW_STEPS: usize = 10;
    if num_steps == 0 {
        if graph_response.clicked_by(egui::PointerButton::Primary)
            || graph_response.drag_started_by(egui::PointerButton::Primary)
            || graph_response.dragged_by(egui::PointerButton::Primary)
        {
            seq.values.resize(MIN_DRAW_STEPS, 0);
            num_steps = MIN_DRAW_STEPS;
        } else if graph_response.drag_started_by(egui::PointerButton::Secondary)
            || graph_response.dragged_by(egui::PointerButton::Secondary)
        {
            seq.values.resize(MIN_LINE_DRAW_STEPS, 0);
            num_steps = MIN_LINE_DRAW_STEPS;
        }
    }

    // While a right-click line draw is in progress, the scroll wheel controls
    // its curvature instead of panning the arpeggio view.
    let line_drawing = graph_response.dragged_by(egui::PointerButton::Secondary)
        || graph_response.drag_started_by(egui::PointerButton::Secondary);

    // Handle mouse wheel scrolling for Arpeggio
    if is_arpeggio && !line_drawing && (graph_response.hovered() || scrollbar_response.hovered()) {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll_delta.abs() > 0.0f32 {
            let change = (scroll_delta / 8.0f32).round() as i16;
            let step_change = if change == 0 {
                if scroll_delta > 0.0f32 { 1 } else { -1 }
            } else {
                change
            };
            scroll_center = (scroll_center + step_change).clamp(min_center, max_center);
        }
    }

    // Scrollbar interactions
    if is_arpeggio {
        let button_h = 14.0f32;
        let top_btn_rect = Rect::from_min_max(
            scrollbar_rect.min,
            Pos2::new(scrollbar_rect.max.x, scrollbar_rect.min.y + button_h),
        );
        let bot_btn_rect = Rect::from_min_max(
            Pos2::new(scrollbar_rect.min.x, scrollbar_rect.max.y - button_h),
            scrollbar_rect.max,
        );
        let track_rect = Rect::from_min_max(
            Pos2::new(scrollbar_rect.min.x, scrollbar_rect.min.y + button_h),
            Pos2::new(scrollbar_rect.max.x, scrollbar_rect.max.y - button_h),
        );

        if scrollbar_response.clicked()
            && let Some(pos) = scrollbar_response.interact_pointer_pos()
        {
            if top_btn_rect.contains(pos) {
                scroll_center = (scroll_center + 1).clamp(min_center, max_center);
            } else if bot_btn_rect.contains(pos) {
                scroll_center = (scroll_center - 1).clamp(min_center, max_center);
            }
        }

        if (scrollbar_response.dragged() || scrollbar_response.clicked())
            && let Some(pos) = scrollbar_response.interact_pointer_pos()
            && !top_btn_rect.contains(pos)
            && !bot_btn_rect.contains(pos)
        {
            let visible_rows = (2 * visible_span + 1) as f32;
            let total_rows = (max_val - min_val + 1) as f32;
            let thumb_h = (track_rect.height() * (visible_rows / total_rows))
                .clamp(16.0f32, track_rect.height());
            let travel_h = track_rect.height() - thumb_h;
            if travel_h > 0.0f32 {
                let rel_y = pos.y - track_rect.min.y - thumb_h / 2.0f32;
                let norm = (rel_y / travel_h).clamp(0.0f32, 1.0f32);
                let center_range = (max_center - min_center) as f32;
                scroll_center = (max_center as f32 - norm * center_range).round() as i16;
                scroll_center = scroll_center.clamp(min_center, max_center);
            }
        }
    }

    ui.ctx()
        .data_mut(|d| d.insert_temp(scroll_id, scroll_center));

    let mut text_needs_sync = false;

    // Determine visible Y-axis range for rendering and mouse interaction
    let (vis_min, vis_max) = if is_arpeggio {
        (
            (scroll_center - visible_span).max(min_val),
            (scroll_center + visible_span).min(max_val),
        )
    } else {
        (min_val, max_val)
    };

    // Header interaction (Loop / Release points)
    let loop_drag_id = ui.make_persistent_id("loop_drag_state");
    let rel_drag_id = ui.make_persistent_id("release_drag_state");

    if num_steps > 0 {
        let step_width = header_rect.width() / num_steps as f32;

        let pointer_pos = header_response
            .interact_pointer_pos()
            .or_else(|| ui.input(|i| i.pointer.hover_pos()));

        if let Some(pos) = pointer_pos {
            let x = pos.x.clamp(header_rect.min.x, header_rect.max.x - 1.0f32);
            let current_step =
                (((x - header_rect.min.x) / step_width).floor() as usize).clamp(0, num_steps - 1);

            // Left Mouse Button (Primary) -> Loop Point
            if header_response.drag_started_by(egui::PointerButton::Primary) {
                let was_existing = seq.loop_point == Some(current_step);
                ui.ctx().data_mut(|d| {
                    d.insert_temp(
                        loop_drag_id,
                        MarkerDragState {
                            start_step: current_step,
                            was_existing,
                        },
                    );
                });
                if !was_existing {
                    seq.loop_point = Some(current_step);
                    text_needs_sync = true;
                }
            } else if header_response.dragged_by(egui::PointerButton::Primary) {
                let state: Option<MarkerDragState> =
                    ui.ctx().data_mut(|d| d.get_temp(loop_drag_id));
                if let Some(st) = state
                    && (current_step != st.start_step || !st.was_existing)
                    && seq.loop_point != Some(current_step)
                {
                    seq.loop_point = Some(current_step);
                    text_needs_sync = true;
                }
            }

            if header_response.clicked_by(egui::PointerButton::Primary) {
                let state: Option<MarkerDragState> =
                    ui.ctx().data_mut(|d| d.get_temp(loop_drag_id));
                if let Some(st) = state {
                    if current_step == st.start_step && st.was_existing {
                        seq.loop_point = None;
                        text_needs_sync = true;
                    } else if !st.was_existing {
                        seq.loop_point = Some(current_step);
                        text_needs_sync = true;
                    }
                } else {
                    if seq.loop_point == Some(current_step) {
                        seq.loop_point = None;
                    } else {
                        seq.loop_point = Some(current_step);
                    }
                    text_needs_sync = true;
                }
                ui.ctx()
                    .data_mut(|d| d.remove_temp::<MarkerDragState>(loop_drag_id));
            }

            // Right Mouse Button (Secondary) -> Release Point
            if header_response.drag_started_by(egui::PointerButton::Secondary) {
                let was_existing = seq.release_point == Some(current_step);
                ui.ctx().data_mut(|d| {
                    d.insert_temp(
                        rel_drag_id,
                        MarkerDragState {
                            start_step: current_step,
                            was_existing,
                        },
                    );
                });
                if !was_existing {
                    seq.release_point = Some(current_step);
                    text_needs_sync = true;
                }
            } else if header_response.dragged_by(egui::PointerButton::Secondary) {
                let state: Option<MarkerDragState> = ui.ctx().data_mut(|d| d.get_temp(rel_drag_id));
                if let Some(st) = state
                    && (current_step != st.start_step || !st.was_existing)
                    && seq.release_point != Some(current_step)
                {
                    seq.release_point = Some(current_step);
                    text_needs_sync = true;
                }
            }

            if header_response.clicked_by(egui::PointerButton::Secondary)
                || header_response.secondary_clicked()
            {
                let state: Option<MarkerDragState> = ui.ctx().data_mut(|d| d.get_temp(rel_drag_id));
                if let Some(st) = state {
                    if current_step == st.start_step && st.was_existing {
                        seq.release_point = None;
                        text_needs_sync = true;
                    } else if !st.was_existing {
                        seq.release_point = Some(current_step);
                        text_needs_sync = true;
                    }
                } else {
                    if seq.release_point == Some(current_step) {
                        seq.release_point = None;
                    } else {
                        seq.release_point = Some(current_step);
                    }
                    text_needs_sync = true;
                }
                ui.ctx()
                    .data_mut(|d| d.remove_temp::<MarkerDragState>(rel_drag_id));
            }
        }
    }

    // Graph canvas interaction (Sequence drawing)
    if num_steps > 0 {
        let step_width = graph_rect.width() / num_steps as f32;
        let draw_drag_id = ui.make_persistent_id("envelope_draw_last_pos");
        let line_draw_id = ui.make_persistent_id("envelope_line_draw_state");

        if (graph_response.dragged_by(egui::PointerButton::Primary)
            || graph_response.clicked_by(egui::PointerButton::Primary))
            && let Some(pointer_pos) = graph_response.interact_pointer_pos()
        {
            let last_pos: Option<Pos2> = ui.ctx().data_mut(|d| d.get_temp(draw_drag_id));

            let p0 = last_pos.unwrap_or(pointer_pos);
            let p1 = pointer_pos;

            for_each_step_between(graph_rect, step_width, num_steps, p0, p1, 0.0, |s, y| {
                let clamped_val =
                    pos_y_to_val(y, graph_rect, is_arpeggio, vis_min, vis_max, min_val, max_val);

                if seq.values[s] != clamped_val {
                    seq.values[s] = clamped_val;
                    text_needs_sync = true;
                }
            });

            ui.ctx()
                .data_mut(|d| d.insert_temp(draw_drag_id, pointer_pos));
        }

        if graph_response.drag_started_by(egui::PointerButton::Secondary)
            && let Some(pointer_pos) = graph_response.interact_pointer_pos()
        {
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    line_draw_id,
                    LineDrawState {
                        start: pointer_pos,
                        last: pointer_pos,
                        tension: 0.0,
                    },
                );
            });
        }

        if graph_response.dragged_by(egui::PointerButton::Secondary)
            && let Some(pointer_pos) = graph_response.interact_pointer_pos()
            && let Some(mut state) = ui
                .ctx()
                .data_mut(|d| d.get_temp::<LineDrawState>(line_draw_id))
        {
            state.last = pointer_pos;

            // Scroll wheel bends the ramp's curvature instead of its endpoints.
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_delta.abs() > 0.0f32 {
                state.tension = (state.tension + scroll_delta * 0.003f32).clamp(-1.0, 1.0);
            }

            ui.ctx().data_mut(|d| d.insert_temp(line_draw_id, state));

            let p0 = state.start;
            let p1 = state.last;

            for_each_step_between(
                graph_rect,
                step_width,
                num_steps,
                p0,
                p1,
                state.tension,
                |s, y| {
                    let clamped_val = pos_y_to_val(
                        y, graph_rect, is_arpeggio, vis_min, vis_max, min_val, max_val,
                    );

                    if seq.values[s] != clamped_val {
                        seq.values[s] = clamped_val;
                        text_needs_sync = true;
                    }
                },
            );
        }

        if graph_response.drag_stopped_by(egui::PointerButton::Secondary)
            || graph_response.lost_focus()
            || !graph_response.hovered()
            || !ui.input(|i| i.pointer.secondary_down())
        {
            ui.ctx()
                .data_mut(|d| d.remove_temp::<LineDrawState>(line_draw_id));
        }

        if graph_response.drag_stopped_by(egui::PointerButton::Primary)
            || !ui.input(|i| i.pointer.primary_down())
        {
            ui.ctx().data_mut(|d| d.remove_temp::<Pos2>(draw_drag_id));
        }

        if graph_response.drag_stopped_by(egui::PointerButton::Primary)
            || graph_response.clicked_by(egui::PointerButton::Primary)
        {
            text_needs_sync = true;
        }
    }

    // Render vertical scrollbar visuals
    if is_arpeggio {
        painter.rect_filled(scrollbar_rect, 0.0f32, Color32::from_rgb(18, 18, 18));
        painter.rect_stroke(
            scrollbar_rect,
            0.0f32,
            Stroke::new(1.0f32, Color32::from_rgb(35, 35, 35)),
            egui::StrokeKind::Outside,
        );

        let button_h = 14.0f32;
        let top_btn_rect = Rect::from_min_max(
            scrollbar_rect.min,
            Pos2::new(scrollbar_rect.max.x, scrollbar_rect.min.y + button_h),
        );
        let bot_btn_rect = Rect::from_min_max(
            Pos2::new(scrollbar_rect.min.x, scrollbar_rect.max.y - button_h),
            scrollbar_rect.max,
        );
        let track_rect = Rect::from_min_max(
            Pos2::new(scrollbar_rect.min.x, scrollbar_rect.min.y + button_h),
            Pos2::new(scrollbar_rect.max.x, scrollbar_rect.max.y - button_h),
        );

        painter.rect_filled(top_btn_rect, 0.0f32, Color32::from_rgb(30, 30, 30));
        painter.text(
            top_btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            "▲",
            egui::FontId::proportional(8.0f32),
            Color32::from_rgb(180, 180, 180),
        );

        painter.rect_filled(bot_btn_rect, 0.0f32, Color32::from_rgb(30, 30, 30));
        painter.text(
            bot_btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            "▼",
            egui::FontId::proportional(8.0f32),
            Color32::from_rgb(180, 180, 180),
        );

        let center_range = (max_center - min_center).max(1) as f32;
        let norm_center = (max_center - scroll_center) as f32 / center_range;
        let visible_rows = (2 * visible_span + 1) as f32;
        let total_rows = (max_val - min_val + 1) as f32;
        let thumb_h =
            (track_rect.height() * (visible_rows / total_rows)).clamp(16.0f32, track_rect.height());
        let thumb_travel = track_rect.height() - thumb_h;
        let thumb_top = track_rect.min.y + norm_center * thumb_travel;

        let thumb_rect = Rect::from_min_max(
            Pos2::new(scrollbar_rect.min.x + 1.5f32, thumb_top),
            Pos2::new(scrollbar_rect.max.x - 1.5f32, thumb_top + thumb_h),
        );

        painter.rect_filled(thumb_rect, 2.0f32, Color32::from_rgb(120, 120, 120));
    }

    if num_steps == 0 {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Empty Sequence (0 steps)",
            egui::FontId::proportional(13.0f32),
            Color32::from_rgb(90, 90, 90),
        );
        return text_needs_sync;
    }

    let step_width = graph_rect.width() / num_steps as f32;
    let loop_idx = seq.loop_point.unwrap_or(usize::MAX);
    let rel_idx = seq.release_point.unwrap_or(usize::MAX);

    let is_bipolar = !is_arpeggio && min_val < 0;

    // Draw step column backgrounds
    for i in 0..num_steps {
        let bar_x_min = graph_rect.min.x + i as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0f32;

        let col_rect = Rect::from_min_max(
            Pos2::new(bar_x_min, graph_rect.min.y),
            Pos2::new(bar_x_max, graph_rect.max.y),
        );
        let bg_color = if i % 2 == 0 {
            Color32::from_rgb(14, 14, 14)
        } else {
            Color32::from_rgb(20, 20, 20)
        };
        painter.rect_filled(col_rect, 0.0f32, bg_color);
    }

    // Grid lines
    if is_arpeggio {
        let num_slots = (vis_max - vis_min + 1) as f32;
        let slot_h = graph_rect.height() / num_slots;
        for slot in 0..=(vis_max - vis_min) {
            let val = vis_max - slot;
            let y = graph_rect.min.y + slot as f32 * slot_h;
            let line_color = if val == 0 {
                Color32::from_rgb(55, 55, 55)
            } else {
                Color32::from_rgb(24, 24, 24)
            };
            painter.line_segment(
                [
                    Pos2::new(graph_rect.min.x, y),
                    Pos2::new(graph_rect.max.x, y),
                ],
                Stroke::new(1.0f32, line_color),
            );
        }
    } else if is_bipolar {
        let range = (max_val as f32 - min_val as f32).max(1.0f32);
        let norm_zero = (0.0f32 - min_val as f32) / range;
        let zero_y = graph_rect.max.y - (norm_zero * graph_rect.height());

        painter.line_segment(
            [
                Pos2::new(graph_rect.min.x, zero_y),
                Pos2::new(graph_rect.max.x, zero_y),
            ],
            Stroke::new(1.0f32, Color32::from_rgb(90, 90, 90)),
        );
    }

    // Draw step bars / blocks
    for i in 0..num_steps {
        let val = seq.values[i].clamp(min_val, max_val);

        let bar_x_min = graph_rect.min.x + i as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0f32;

        let is_loop_release_mode = loop_idx < num_steps && rel_idx == loop_idx;

        let bar_color = if is_loop_release_mode && i >= loop_idx {
            Color32::from_rgb(230, 190, 40)
        } else if loop_idx < rel_idx {
            if i >= rel_idx {
                Color32::from_rgb(200, 120, 220)
            } else if i >= loop_idx {
                Color32::from_rgb(100, 200, 220)
            } else {
                Color32::from_rgb(220, 220, 220)
            }
        } else {
            if i >= loop_idx && loop_idx < num_steps {
                Color32::from_rgb(100, 200, 220)
            } else if i >= rel_idx && rel_idx < num_steps {
                Color32::from_rgb(200, 120, 220)
            } else {
                Color32::from_rgb(220, 220, 220)
            }
        };

        if is_arpeggio {
            if val > vis_max {
                let ind_rect = Rect::from_min_max(
                    Pos2::new(bar_x_min + 2.0f32, graph_rect.min.y + 1.0f32),
                    Pos2::new(bar_x_max - 2.0f32, graph_rect.min.y + 4.0f32),
                );
                painter.rect_filled(ind_rect, 1.0f32, Color32::from_rgb(255, 200, 80));
            } else if val < vis_min {
                let ind_rect = Rect::from_min_max(
                    Pos2::new(bar_x_min + 2.0f32, graph_rect.max.y - 4.0f32),
                    Pos2::new(bar_x_max - 2.0f32, graph_rect.max.y - 1.0f32),
                );
                painter.rect_filled(ind_rect, 1.0f32, Color32::from_rgb(255, 200, 80));
            } else {
                let num_slots = (vis_max - vis_min + 1) as f32;
                let slot_h = graph_rect.height() / num_slots;
                let slot_idx = (vis_max - val) as f32;
                let bar_y_min = graph_rect.min.y + slot_idx * slot_h;
                let bar_y_max = bar_y_min + slot_h;
                let bar_rect = Rect::from_min_max(
                    Pos2::new(bar_x_min + 0.5f32, bar_y_min + 0.5f32),
                    Pos2::new(bar_x_max - 0.5f32, bar_y_max - 0.5f32),
                );
                painter.rect_filled(bar_rect, 1.0f32, bar_color);
            }
        } else {
            let bar_rect = if is_bipolar {
                let range = (max_val as f32 - min_val as f32).max(1.0f32);
                let norm_zero = (0.0f32 - min_val as f32) / range;
                let zero_y = graph_rect.max.y - (norm_zero * graph_rect.height());
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

            if val != 0 || !is_bipolar {
                painter.rect_filled(bar_rect, 1.0f32, bar_color);
            }
        }
    }

    if let Some(step) = playhead_step.filter(|step| *step < num_steps) {
        let bar_x_min = graph_rect.min.x + step as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0f32;
        let col_rect = Rect::from_min_max(
            Pos2::new(bar_x_min, graph_rect.min.y),
            Pos2::new(bar_x_max, graph_rect.max.y),
        );
        draw_playhead_rect(&painter, col_rect);
    }

    draw_line_draw_preview(
        &painter,
        ui.ctx()
            .data(|d| d.get_temp(ui.make_persistent_id("envelope_line_draw_state"))),
    );

    // Render loop/release region headers
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

    // Min / Max labels
    painter.text(
        Pos2::new(graph_rect.min.x + 6.0f32, graph_rect.min.y + 2.0f32),
        egui::Align2::LEFT_TOP,
        format!("{}", vis_max),
        egui::FontId::proportional(11.0f32),
        Color32::from_rgb(160, 160, 160),
    );
    painter.text(
        Pos2::new(graph_rect.min.x + 6.0f32, graph_rect.max.y - 14.0f32),
        egui::Align2::LEFT_TOP,
        format!("{}", vis_min),
        egui::FontId::proportional(11.0f32),
        Color32::from_rgb(160, 160, 160),
    );

    text_needs_sync
}

/// S5B-specific state for the duty/mode graph's flag row: which step column
/// the last toggle landed on, so a drag across the row doesn't rapid-fire
/// toggle the same column repeatedly (mirrors dn's `m_iLastIndex` guard,
/// `GraphEditor.cpp`).
#[derive(Clone, Copy, Default)]
struct S5BFlagDragState {
    last_step: Option<usize>,
}

/// Renders the S5B duty/mode graph: a period bar (upper region, 0..=31) plus
/// three Envelope/Tone/Noise toggle buttons per step (lower region), packed
/// into one `i16` step value the way dn's `CNoiseEditor` does. Returns `true`
/// when any step value changes so the caller can regenerate `text` via
/// `sequence_to_text`, same contract as [`draw_envelope_bar_graph`].
pub fn draw_s5b_duty_noise_graph(
    ui: &mut egui::Ui,
    seq: &mut Sequence,
    playhead_step: Option<usize>,
    graph_height: f32,
) -> bool {
    let desired_size = Vec2::new(ui.available_width(), graph_height);
    let (rect, _response) = ui.allocate_at_least(desired_size, Sense::hover());

    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 2.0f32, theme::PANEL);
    painter.rect_stroke(
        rect,
        2.0f32,
        Stroke::new(1.0f32, theme::BORDER),
        egui::StrokeKind::Outside,
    );

    let num_steps = seq.len();

    if num_steps == 0 {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Empty Sequence (0 steps)",
            egui::FontId::proportional(13.0f32),
            theme::TEXT_DIM,
        );
        return false;
    }

    // Two lanes at the same ~15px per button the three-lane layout used, so
    // dropping the Envelope lane gives the period bar the row back instead of
    // stretching the two that remain.
    const FLAG_ROW_HEIGHT: f32 = 30.0f32;
    // Leaves the panel's own bottom border visible below the flag row instead
    // of the last row of buttons sitting flush against it.
    const BOTTOM_MARGIN: f32 = 4.0f32;
    // Reserves a Loop/Release marker strip below the flag row, same idea as
    // `draw_envelope_bar_graph`'s `header_rect`.
    const HEADER_HEIGHT: f32 = 20.0f32;

    let content_rect = Rect::from_min_max(
        rect.min,
        Pos2::new(rect.max.x, rect.max.y - HEADER_HEIGHT - BOTTOM_MARGIN),
    );
    let header_rect = Rect::from_min_max(
        Pos2::new(rect.min.x, rect.max.y - HEADER_HEIGHT - BOTTOM_MARGIN),
        Pos2::new(rect.max.x, rect.max.y - BOTTOM_MARGIN),
    );
    let period_area_rect = Rect::from_min_max(
        content_rect.min,
        Pos2::new(content_rect.max.x, content_rect.max.y - FLAG_ROW_HEIGHT),
    );
    let flag_rect = Rect::from_min_max(
        Pos2::new(content_rect.min.x, content_rect.max.y - FLAG_ROW_HEIGHT),
        content_rect.max,
    );
    // Period area splits into a duty-width sub-bar (top) and noise-period
    // sub-bar (bottom), same columns, half the remaining height each with a
    // gap between them so the two lanes read as separate graphs rather than
    // one bar chart with a seam.
    const SUB_BAR_GAP: f32 = 6.0f32;
    let sub_bar_height = ((period_area_rect.height() - SUB_BAR_GAP) * 0.5f32).max(0.0f32);
    let duty_rect = Rect::from_min_max(
        period_area_rect.min,
        Pos2::new(
            period_area_rect.max.x,
            period_area_rect.min.y + sub_bar_height,
        ),
    );
    let period_rect = Rect::from_min_max(
        Pos2::new(period_area_rect.min.x, duty_rect.max.y + SUB_BAR_GAP),
        period_area_rect.max,
    );

    let header_response = ui.interact(
        header_rect,
        ui.make_persistent_id("s5b_header_area"),
        Sense::click_and_drag(),
    );

    let step_width = rect.width() / num_steps as f32;
    let mut changed = false;

    // Step column backgrounds, spanning the period bar and flag row (not the
    // header strip, which paints its own background).
    for i in 0..num_steps {
        let bar_x_min = content_rect.min.x + i as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0f32;
        let col_rect = Rect::from_min_max(
            Pos2::new(bar_x_min, content_rect.min.y),
            Pos2::new(bar_x_max, content_rect.max.y),
        );
        let bg_color = if i % 2 == 0 {
            theme::BG
        } else {
            theme::PANEL_ALT
        };
        painter.rect_filled(col_rect, 0.0f32, bg_color);
    }

    // The column stripes above run the full height of the content area, so the
    // gap between the two sub-bars has to be painted back over in the panel
    // color — otherwise the "gap" is just more striped background and the two
    // lanes still read as one continuous graph.
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(content_rect.min.x, duty_rect.max.y),
            Pos2::new(content_rect.max.x, period_rect.min.y),
        ),
        0.0f32,
        theme::PANEL,
    );

    // Header interaction (Loop / Release points) — ported from
    // `draw_envelope_bar_graph`'s header block.
    let loop_drag_id = ui.make_persistent_id("s5b_loop_drag_state");
    let rel_drag_id = ui.make_persistent_id("s5b_release_drag_state");

    {
        let pointer_pos = header_response
            .interact_pointer_pos()
            .or_else(|| ui.input(|i| i.pointer.hover_pos()));

        if let Some(pos) = pointer_pos {
            let x = pos.x.clamp(header_rect.min.x, header_rect.max.x - 1.0f32);
            let current_step =
                (((x - header_rect.min.x) / step_width).floor() as usize).clamp(0, num_steps - 1);

            if header_response.drag_started_by(egui::PointerButton::Primary) {
                let was_existing = seq.loop_point == Some(current_step);
                ui.ctx().data_mut(|d| {
                    d.insert_temp(
                        loop_drag_id,
                        MarkerDragState {
                            start_step: current_step,
                            was_existing,
                        },
                    );
                });
                if !was_existing {
                    seq.loop_point = Some(current_step);
                    changed = true;
                }
            } else if header_response.dragged_by(egui::PointerButton::Primary) {
                let state: Option<MarkerDragState> =
                    ui.ctx().data_mut(|d| d.get_temp(loop_drag_id));
                if let Some(st) = state
                    && (current_step != st.start_step || !st.was_existing)
                    && seq.loop_point != Some(current_step)
                {
                    seq.loop_point = Some(current_step);
                    changed = true;
                }
            }

            if header_response.clicked_by(egui::PointerButton::Primary) {
                let state: Option<MarkerDragState> =
                    ui.ctx().data_mut(|d| d.get_temp(loop_drag_id));
                if let Some(st) = state {
                    if current_step == st.start_step && st.was_existing {
                        seq.loop_point = None;
                        changed = true;
                    } else if !st.was_existing {
                        seq.loop_point = Some(current_step);
                        changed = true;
                    }
                } else if seq.loop_point == Some(current_step) {
                    seq.loop_point = None;
                    changed = true;
                } else {
                    seq.loop_point = Some(current_step);
                    changed = true;
                }
                ui.ctx()
                    .data_mut(|d| d.remove_temp::<MarkerDragState>(loop_drag_id));
            }

            if header_response.drag_started_by(egui::PointerButton::Secondary) {
                let was_existing = seq.release_point == Some(current_step);
                ui.ctx().data_mut(|d| {
                    d.insert_temp(
                        rel_drag_id,
                        MarkerDragState {
                            start_step: current_step,
                            was_existing,
                        },
                    );
                });
                if !was_existing {
                    seq.release_point = Some(current_step);
                    changed = true;
                }
            } else if header_response.dragged_by(egui::PointerButton::Secondary) {
                let state: Option<MarkerDragState> = ui.ctx().data_mut(|d| d.get_temp(rel_drag_id));
                if let Some(st) = state
                    && (current_step != st.start_step || !st.was_existing)
                    && seq.release_point != Some(current_step)
                {
                    seq.release_point = Some(current_step);
                    changed = true;
                }
            }

            if header_response.clicked_by(egui::PointerButton::Secondary)
                || header_response.secondary_clicked()
            {
                let state: Option<MarkerDragState> = ui.ctx().data_mut(|d| d.get_temp(rel_drag_id));
                if let Some(st) = state {
                    if current_step == st.start_step && st.was_existing {
                        seq.release_point = None;
                        changed = true;
                    } else if !st.was_existing {
                        seq.release_point = Some(current_step);
                        changed = true;
                    }
                } else if seq.release_point == Some(current_step) {
                    seq.release_point = None;
                    changed = true;
                } else {
                    seq.release_point = Some(current_step);
                    changed = true;
                }
                ui.ctx()
                    .data_mut(|d| d.remove_temp::<MarkerDragState>(rel_drag_id));
            }
        }
    }

    //------------------------------------------------------
    // Duty-width bar graph (0..=8), stacked above the noise-period bar.
    //------------------------------------------------------
    let duty_response = ui.interact(
        duty_rect,
        ui.make_persistent_id("s5b_duty_area"),
        Sense::click_and_drag(),
    );

    let duty_draw_drag_id = ui.make_persistent_id("s5b_duty_draw_last_pos");
    let duty_line_draw_id = ui.make_persistent_id("s5b_duty_line_draw_state");

    let duty_y_to_value = |value: i16, y: f32| -> i16 {
        let rel_y = (y - duty_rect.min.y).clamp(0.0, duty_rect.height());
        let norm_y = if duty_rect.height() > 0.0 {
            1.0 - rel_y / duty_rect.height()
        } else {
            0.0
        };
        let new_index = ((norm_y * 9.0).floor() as i16).clamp(0, 8);
        s5b_set_duty_index(value, new_index)
    };

    if (duty_response.dragged_by(egui::PointerButton::Primary)
        || duty_response.clicked_by(egui::PointerButton::Primary))
        && let Some(pointer_pos) = duty_response.interact_pointer_pos()
    {
        let last_pos: Option<Pos2> = ui.ctx().data_mut(|d| d.get_temp(duty_draw_drag_id));
        let p0 = last_pos.unwrap_or(pointer_pos);
        let p1 = pointer_pos;

        for_each_step_between(duty_rect, step_width, num_steps, p0, p1, 0.0, |s, y| {
            let new_value = duty_y_to_value(seq.values[s], y);
            if new_value != seq.values[s] {
                seq.values[s] = new_value;
                changed = true;
            }
        });

        ui.ctx()
            .data_mut(|d| d.insert_temp(duty_draw_drag_id, pointer_pos));
    }

    if duty_response.drag_started_by(egui::PointerButton::Secondary)
        && let Some(pointer_pos) = duty_response.interact_pointer_pos()
    {
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                duty_line_draw_id,
                LineDrawState {
                    start: pointer_pos,
                    last: pointer_pos,
                    tension: 0.0,
                },
            );
        });
    }

    if duty_response.dragged_by(egui::PointerButton::Secondary)
        && let Some(pointer_pos) = duty_response.interact_pointer_pos()
        && let Some(mut state) = ui
            .ctx()
            .data_mut(|d| d.get_temp::<LineDrawState>(duty_line_draw_id))
    {
        state.last = pointer_pos;

        let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll_delta.abs() > 0.0f32 {
            state.tension = (state.tension + scroll_delta * 0.003f32).clamp(-1.0, 1.0);
        }

        ui.ctx()
            .data_mut(|d| d.insert_temp(duty_line_draw_id, state));

        for_each_step_between(
            duty_rect,
            step_width,
            num_steps,
            state.start,
            state.last,
            state.tension,
            |s, y| {
                let new_value = duty_y_to_value(seq.values[s], y);
                if new_value != seq.values[s] {
                    seq.values[s] = new_value;
                    changed = true;
                }
            },
        );
    }

    if duty_response.drag_stopped_by(egui::PointerButton::Secondary)
        || duty_response.lost_focus()
        || !duty_response.hovered()
        || !ui.input(|i| i.pointer.secondary_down())
    {
        ui.ctx()
            .data_mut(|d| d.remove_temp::<LineDrawState>(duty_line_draw_id));
    }

    if duty_response.drag_stopped_by(egui::PointerButton::Primary)
        || !ui.input(|i| i.pointer.primary_down())
    {
        ui.ctx().data_mut(|d| {
            d.remove_temp::<Pos2>(duty_draw_drag_id);
        });
    }

    for i in 0..num_steps {
        let duty = s5b_duty_index(seq.values[i]).clamp(0, 8);

        let bar_x_min = duty_rect.min.x + i as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0f32;
        let norm_val = duty as f32 / 8.0f32;
        let bar_y_min = duty_rect.max.y - (norm_val * duty_rect.height());
        let bar_rect = Rect::from_min_max(
            Pos2::new(bar_x_min, bar_y_min),
            Pos2::new(bar_x_max, duty_rect.max.y),
        );
        painter.rect_filled(bar_rect, 1.0f32, theme::S5B_TONE_FLAG);
    }

    draw_line_draw_preview(&painter, ui.ctx().data(|d| d.get_temp(duty_line_draw_id)));

    // Min / Max labels for the duty bar.
    painter.text(
        Pos2::new(duty_rect.min.x + 6.0f32, duty_rect.min.y + 2.0f32),
        egui::Align2::LEFT_TOP,
        "8",
        egui::FontId::proportional(11.0f32),
        Color32::from_rgb(160, 160, 160),
    );
    painter.text(
        Pos2::new(duty_rect.min.x + 6.0f32, duty_rect.max.y - 14.0f32),
        egui::Align2::LEFT_TOP,
        "0",
        egui::FontId::proportional(11.0f32),
        Color32::from_rgb(160, 160, 160),
    );

    //------------------------------------------------------
    // Upper region: noise-period bar graph (0..=31).
    //------------------------------------------------------
    let period_response = ui.interact(
        period_rect,
        ui.make_persistent_id("s5b_period_area"),
        Sense::click_and_drag(),
    );

    // Interpolates between last frame's pointer position and this frame's,
    // filling every step column in between — same fix `draw_envelope_bar_graph`
    // uses (`draw_drag_id`) so a fast drag across several step columns between
    // two frames doesn't skip the ones the pointer jumped over. Shared via
    // `for_each_step_between`, same helper the plain graph's left-drag and
    // right-click line both use.
    let period_draw_drag_id = ui.make_persistent_id("s5b_period_draw_last_pos");
    let period_line_draw_id = ui.make_persistent_id("s5b_period_line_draw_state");

    let period_y_to_value = |value: i16, y: f32| -> i16 {
        let rel_y = (y - period_rect.min.y).clamp(0.0, period_rect.height());
        let norm_y = if period_rect.height() > 0.0 {
            1.0 - rel_y / period_rect.height()
        } else {
            0.0
        };
        let new_period = ((norm_y * 32.0).floor() as i16).clamp(0, 31);
        (value & !S5B_PERIOD_MASK) | new_period
    };

    if (period_response.dragged_by(egui::PointerButton::Primary)
        || period_response.clicked_by(egui::PointerButton::Primary))
        && let Some(pointer_pos) = period_response.interact_pointer_pos()
    {
        let last_pos: Option<Pos2> = ui.ctx().data_mut(|d| d.get_temp(period_draw_drag_id));
        let p0 = last_pos.unwrap_or(pointer_pos);
        let p1 = pointer_pos;

        for_each_step_between(period_rect, step_width, num_steps, p0, p1, 0.0, |s, y| {
            let new_value = period_y_to_value(seq.values[s], y);
            if new_value != seq.values[s] {
                seq.values[s] = new_value;
                changed = true;
            }
        });

        ui.ctx()
            .data_mut(|d| d.insert_temp(period_draw_drag_id, pointer_pos));
    }

    // Right-click line draw — same feature and interaction as the plain
    // graph's (drag a straight or curved ramp across several steps at once),
    // scoped to the period bar.
    if period_response.drag_started_by(egui::PointerButton::Secondary)
        && let Some(pointer_pos) = period_response.interact_pointer_pos()
    {
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                period_line_draw_id,
                LineDrawState {
                    start: pointer_pos,
                    last: pointer_pos,
                    tension: 0.0,
                },
            );
        });
    }

    if period_response.dragged_by(egui::PointerButton::Secondary)
        && let Some(pointer_pos) = period_response.interact_pointer_pos()
        && let Some(mut state) = ui
            .ctx()
            .data_mut(|d| d.get_temp::<LineDrawState>(period_line_draw_id))
    {
        state.last = pointer_pos;

        let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll_delta.abs() > 0.0f32 {
            state.tension = (state.tension + scroll_delta * 0.003f32).clamp(-1.0, 1.0);
        }

        ui.ctx()
            .data_mut(|d| d.insert_temp(period_line_draw_id, state));

        for_each_step_between(
            period_rect,
            step_width,
            num_steps,
            state.start,
            state.last,
            state.tension,
            |s, y| {
                let new_value = period_y_to_value(seq.values[s], y);
                if new_value != seq.values[s] {
                    seq.values[s] = new_value;
                    changed = true;
                }
            },
        );
    }

    if period_response.drag_stopped_by(egui::PointerButton::Secondary)
        || period_response.lost_focus()
        || !period_response.hovered()
        || !ui.input(|i| i.pointer.secondary_down())
    {
        ui.ctx()
            .data_mut(|d| d.remove_temp::<LineDrawState>(period_line_draw_id));
    }

    if period_response.drag_stopped_by(egui::PointerButton::Primary)
        || !ui.input(|i| i.pointer.primary_down())
    {
        ui.ctx().data_mut(|d| {
            d.remove_temp::<Pos2>(period_draw_drag_id);
        });
    }

    for i in 0..num_steps {
        let period = (seq.values[i] & S5B_PERIOD_MASK).clamp(0, 31);

        let bar_x_min = period_rect.min.x + i as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0f32;
        let norm_val = period as f32 / 31.0f32;
        let bar_y_min = period_rect.max.y - (norm_val * period_rect.height());
        let bar_rect = Rect::from_min_max(
            Pos2::new(bar_x_min, bar_y_min),
            Pos2::new(bar_x_max, period_rect.max.y),
        );
        // Matches `draw_envelope_bar_graph`'s unmarked-step bar color.
        painter.rect_filled(bar_rect, 1.0f32, Color32::from_rgb(220, 220, 220));
    }

    //------------------------------------------------------
    // Lower region: Envelope / Tone / Noise toggle buttons.
    //------------------------------------------------------
    let flag_response = ui.interact(
        flag_rect,
        ui.make_persistent_id("s5b_flag_area"),
        Sense::click_and_drag(),
    );

    // dn has a third Envelope lane here; this synth omits it — the chip's
    // hardware envelope has no period/shape control in a DAW host, so the
    // flag could only ever select an untunable free-running ramp. See
    // `apply_s5b_modulation`, which never sets the envelope-select bit.
    const FLAGS: [(i16, Color32); 2] = [
        (S5B_MODE_SQUARE, theme::S5B_TONE_FLAG),
        (S5B_MODE_NOISE, theme::S5B_NOISE_FLAG),
    ];
    const FLAG_LABELS: [&str; 2] = ["T", "N"];
    let button_h = flag_rect.height() / FLAGS.len() as f32;

    let drag_id = ui.make_persistent_id("s5b_flag_drag_state");

    let toggling = flag_response.dragged_by(egui::PointerButton::Primary)
        || flag_response.clicked_by(egui::PointerButton::Primary);

    if toggling
        && let Some(pointer_pos) = flag_response.interact_pointer_pos()
    {
        let rel_x =
            (pointer_pos.x - flag_rect.min.x).clamp(0.0, (flag_rect.width() - 0.001).max(0.0));
        let step = (((rel_x / step_width).floor() as i32).clamp(0, num_steps as i32 - 1)) as usize;

        let state: S5BFlagDragState = ui.ctx().data_mut(|d| d.get_temp(drag_id)).unwrap_or_default();

        if state.last_step != Some(step) {
            let rel_y = (pointer_pos.y - flag_rect.min.y).clamp(0.0, flag_rect.height() - 0.001);
            let row = ((rel_y / button_h).floor() as usize).min(FLAGS.len() - 1);
            let (bit, _) = FLAGS[row];

            seq.values[step] ^= bit;
            changed = true;

            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    drag_id,
                    S5BFlagDragState {
                        last_step: Some(step),
                    },
                );
            });
        }
    }

    if flag_response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
        ui.ctx().data_mut(|d| {
            d.remove_temp::<S5BFlagDragState>(drag_id);
        });
    }

    for i in 0..num_steps {
        let value = seq.values[i];
        let bar_x_min = flag_rect.min.x + i as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0f32;

        for (row, (bit, on_color)) in FLAGS.iter().enumerate() {
            let y_min = flag_rect.min.y + row as f32 * button_h;
            let y_max = y_min + button_h - 1.0f32;
            let btn_rect = Rect::from_min_max(
                Pos2::new(bar_x_min + 1.0f32, y_min + 1.0f32),
                Pos2::new(bar_x_max - 1.0f32, y_max - 1.0f32),
            );

            let is_set = value & bit != 0;
            let color = if is_set { *on_color } else { theme::S5B_FLAG_OFF };
            painter.rect_filled(btn_rect, 1.0f32, color);

            if is_set {
                painter.text(
                    btn_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    FLAG_LABELS[row],
                    egui::FontId::proportional(10.0f32),
                    theme::TEXT,
                );
            }
        }
    }

    draw_line_draw_preview(
        &painter,
        ui.ctx().data(|d| d.get_temp(period_line_draw_id)),
    );

    // Render loop/release region headers — ported from `draw_envelope_bar_graph`.
    painter.rect_filled(header_rect, 0.0f32, Color32::from_rgb(25, 25, 25));

    let loop_idx = seq.loop_point.unwrap_or(usize::MAX);
    let rel_idx = seq.release_point.unwrap_or(usize::MAX);
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

    if let Some(step) = playhead_step.filter(|step| *step < num_steps) {
        let bar_x_min = content_rect.min.x + step as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0f32;
        // Drawn per lane rather than as one full-height column, so the
        // playhead doesn't paint a bridge back across the sub-bar gap.
        for lane in [duty_rect, period_rect, flag_rect] {
            draw_playhead_rect(
                &painter,
                Rect::from_min_max(
                    Pos2::new(bar_x_min, lane.min.y),
                    Pos2::new(bar_x_max, lane.max.y),
                ),
            );
        }
    }

    // Min / Max labels for the period bar, same style as `draw_envelope_bar_graph`.
    painter.text(
        Pos2::new(period_rect.min.x + 6.0f32, period_rect.min.y + 2.0f32),
        egui::Align2::LEFT_TOP,
        "31",
        egui::FontId::proportional(11.0f32),
        Color32::from_rgb(160, 160, 160),
    );
    painter.text(
        Pos2::new(period_rect.min.x + 6.0f32, period_rect.max.y - 14.0f32),
        egui::Align2::LEFT_TOP,
        "0",
        egui::FontId::proportional(11.0f32),
        Color32::from_rgb(160, 160, 160),
    );

    changed
}

pub fn group_box<R>(
    ui: &mut egui::Ui,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    const TITLE_HEIGHT: i8 = 10;
    const PADDING: i8 = 10;
    const ROUNDING: u8 = 8;

    let frame = egui::Frame::new().inner_margin(egui::Margin {
        left: PADDING,
        right: PADDING,
        top: PADDING + TITLE_HEIGHT,
        bottom: PADDING,
    });

    let inner = frame.show(ui, add_contents);

    let rect = inner.response.rect;

    let painter = ui.painter();

    let stroke = egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color);

    let galley = painter.layout_no_wrap(
        title.to_owned(),
        egui::FontId::proportional(16.0f32),
        ui.visuals().text_color(),
    );

    let title_pos = egui::pos2(rect.left() + 10.0f32, rect.top() - galley.size().y * 0.5f32);

    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(ROUNDING),
        stroke,
        egui::StrokeKind::Outside,
    );

    // Erase only behind the title
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(title_pos.x - 4.0f32, rect.top() - 2.0f32),
            egui::pos2(title_pos.x + galley.size().x + 4.0f32, rect.top() + 2.0f32),
        ),
        egui::CornerRadius::ZERO,
        ui.visuals().panel_fill,
    );

    painter.galley(title_pos, galley, ui.visuals().text_color());

    inner.inner
}

/// Renders a button that triggers immediately on click and auto-repeats continuously when held down.
pub fn repeating_button(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) -> bool {
    let button = egui::Button::new(text).sense(egui::Sense::click_and_drag());
    let response = ui.add(button);
    let id = response.id;

    let mut triggered = false;

    let (primary_down, pointer_pos) =
        ui.input(|i| (i.pointer.primary_down(), i.pointer.hover_pos()));

    let pointer_over_button = if let Some(pos) = pointer_pos {
        response.rect.expand(2.0f32).contains(pos)
    } else {
        false
    };

    let state = ui
        .ctx()
        .data_mut(|d| d.get_temp::<RepeatingButtonState>(id));

    if primary_down {
        match state {
            None => {
                if response.is_pointer_button_down_on() {
                    ui.ctx().request_repaint();
                    let now = ui.input(|i| i.time);
                    triggered = true;
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(
                            id,
                            RepeatingButtonState {
                                start_time: now,
                                last_trigger_time: now,
                            },
                        );
                    });
                }
            }
            Some(mut st) => {
                if pointer_over_button {
                    ui.ctx().request_repaint();
                    let now = ui.input(|i| i.time);

                    const INITIAL_DELAY: f64 = 0.35;
                    const REPEAT_INTERVAL: f64 = 0.05;

                    if now - st.start_time >= INITIAL_DELAY
                        && now - st.last_trigger_time >= REPEAT_INTERVAL
                    {
                        triggered = true;
                        st.last_trigger_time = now;
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(id, st);
                        });
                    }
                } else {
                    ui.ctx().data_mut(|d| {
                        d.remove_temp::<RepeatingButtonState>(id);
                    });
                }
            }
        }
    } else if state.is_some() {
        ui.ctx().data_mut(|d| {
            d.remove_temp::<RepeatingButtonState>(id);
        });
    }

    triggered
}

fn draw_playhead_rect(painter: &egui::Painter, rect: Rect) {
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

fn pos_y_to_val(
    y: f32,
    graph_rect: Rect,
    is_arpeggio: bool,
    vis_min: i16,
    vis_max: i16,
    min_val: i16,
    max_val: i16,
) -> i16 {
    let rel_y = (y - graph_rect.min.y).clamp(0.0f32, graph_rect.height());
    let norm_y = if graph_rect.height() > 0.0 {
        rel_y / graph_rect.height()
    } else {
        0.0
    };

    let (range_min, range_max) = if is_arpeggio {
        (vis_min, vis_max)
    } else {
        (min_val, max_val)
    };

    let num_slots = (range_max - range_min + 1) as f32;
    let slot_idx = (norm_y * num_slots).floor() as i16;
    (range_max - slot_idx).clamp(range_min, range_max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect};

    #[test]
    fn test_pos_y_to_val_volume_unipolar() {
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0));
        assert_eq!(pos_y_to_val(0.0, rect, false, 0, 15, 0, 15), 15);
        assert_eq!(pos_y_to_val(100.0, rect, false, 0, 15, 0, 15), 0);
        assert_eq!(pos_y_to_val(50.0, rect, false, 0, 15, 0, 15), 7);
    }

    #[test]
    fn test_pos_y_to_val_bipolar_reaches_bounds_without_zero_snap() {
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0));
        assert_eq!(pos_y_to_val(0.0, rect, false, -128, 127, -128, 127), 127);
        assert_eq!(pos_y_to_val(100.0, rect, false, -128, 127, -128, 127), -128);
        assert_eq!(pos_y_to_val(0.1, rect, false, -128, 127, -128, 127), 127);
        assert_eq!(pos_y_to_val(99.9, rect, false, -128, 127, -128, 127), -128);
        // Near center line: norm_y = 49.8 / 100 = 0.498 -> slot_idx = (0.498 * 256).floor() = 127 -> 127 - 127 = 0
        assert_eq!(pos_y_to_val(49.8, rect, false, -128, 127, -128, 127), 0);
        // Slightly above center: norm_y = 49.5 / 100 = 0.495 -> slot_idx = (0.495 * 256).floor() = 126 -> 127 - 126 = 1 (no zero snapping!)
        assert_eq!(pos_y_to_val(49.5, rect, false, -128, 127, -128, 127), 1);
    }

    #[test]
    fn test_apply_tension_endpoints_are_fixed() {
        for tension in [-1.0f32, -0.5, 0.0, 0.5, 1.0] {
            assert_eq!(apply_tension(0.0, tension), 0.0);
            assert_eq!(apply_tension(1.0, tension), 1.0);
        }
    }

    #[test]
    fn test_apply_tension_zero_is_linear() {
        assert_eq!(apply_tension(0.25, 0.0), 0.25);
        assert_eq!(apply_tension(0.5, 0.0), 0.5);
        assert_eq!(apply_tension(0.75, 0.0), 0.75);
    }

    #[test]
    fn test_apply_tension_bows_curve_and_stays_in_bounds() {
        // Positive tension bows the midpoint above the straight line (ease-out).
        assert!(apply_tension(0.5, 1.0) > 0.5);
        // Negative tension bows the midpoint below the straight line (ease-in).
        assert!(apply_tension(0.5, -1.0) < 0.5);
        // Curve never overshoots the 0..=1 range even at extreme tension.
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            assert!((0.0..=1.0).contains(&apply_tension(t, 1.0)));
            assert!((0.0..=1.0).contains(&apply_tension(t, -1.0)));
        }
    }
}
