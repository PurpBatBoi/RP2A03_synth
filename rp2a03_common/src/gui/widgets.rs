//! rp2a03_common\src\gui\widgets.rs
//!
//! Custom painter elements for sequence visualization and interactive envelope editing.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use rp2a03_core::sequencer::Sequence;

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

    // Handle mouse wheel scrolling for Arpeggio
    if is_arpeggio && (graph_response.hovered() || scrollbar_response.hovered()) {
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

            let step_of = |x: f32| -> usize {
                let rel_x =
                    (x - graph_rect.min.x).clamp(0.0, (graph_rect.width() - 0.001).max(0.0));
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
                    let s_x = graph_rect.min.x + (s as f32 + 0.5) * step_width;
                    let t = ((s_x - p0.x) / dx).clamp(0.0, 1.0);
                    p0.y + t * (p1.y - p0.y)
                };

                let clamped_val = pos_y_to_val(
                    target_y,
                    graph_rect,
                    is_arpeggio,
                    vis_min,
                    vis_max,
                    min_val,
                    max_val,
                );

                if seq.values[s] != clamped_val {
                    seq.values[s] = clamped_val;
                    text_needs_sync = true;
                }
            }

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
            ui.ctx().data_mut(|d| d.insert_temp(line_draw_id, state));

            let p0 = state.start;
            let p1 = state.last;

            let step_of = |x: f32| -> usize {
                let rel_x =
                    (x - graph_rect.min.x).clamp(0.0, (graph_rect.width() - 0.001).max(0.0));
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
                    let s_x = graph_rect.min.x + (s as f32 + 0.5) * step_width;
                    let t = ((s_x - p0.x) / dx).clamp(0.0, 1.0);
                    p0.y + t * (p1.y - p0.y)
                };

                let clamped_val = pos_y_to_val(
                    target_y,
                    graph_rect,
                    is_arpeggio,
                    vis_min,
                    vis_max,
                    min_val,
                    max_val,
                );

                if seq.values[s] != clamped_val {
                    seq.values[s] = clamped_val;
                    text_needs_sync = true;
                }
            }
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

    if let Some(state) = ui
        .ctx()
        .data(|d| d.get_temp::<LineDrawState>(ui.make_persistent_id("envelope_line_draw_state")))
    {
        painter.line_segment(
            [state.start, state.last],
            Stroke::new(4.0f32, Color32::from_rgba_unmultiplied(255, 255, 255, 200)),
        );
    }

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
}
