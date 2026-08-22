//! `rp2a03_common\src\gui\widgets\s5b_graph.rs`
//! The Sunsoft 5B "duty" lane's compound widget: duty width, noise period,
//! and the tone/noise mixer flags, since that chip packs all three into one
//! lane's step value.

use super::common::{
    LineDrawState, draw_line_draw_preview, draw_playhead_rect, for_each_step_between,
    handle_marker_drag, paint_marker_header,
};
use crate::gui::theme;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use rp2a03_core::sequencer::{
    S5B_MODE_NOISE, S5B_MODE_SQUARE, S5B_PERIOD_MASK, Sequence, s5b_duty_index, s5b_set_duty_index,
};

#[derive(Clone, Copy, Default)]
struct S5BFlagDragState {
    last_step: Option<usize>,
}

/// The rects `draw_s5b_duty_noise_graph` lays its three lanes (duty, period,
/// flags) and marker header out against.
struct S5BRegions {
    rect: Rect,
    header_rect: Rect,
    duty_rect: Rect,
    period_rect: Rect,
    flag_rect: Rect,
}

fn layout_s5b_regions(ui: &mut egui::Ui, graph_height: f32) -> (S5BRegions, egui::Painter) {
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

    const FLAG_ROW_HEIGHT: f32 = 30.0f32;
    const BOTTOM_MARGIN: f32 = 4.0f32;
    const HEADER_HEIGHT: f32 = 20.0f32;
    const SUB_BAR_GAP: f32 = 6.0f32;

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

    (
        S5BRegions {
            rect,
            header_rect,
            duty_rect,
            period_rect,
            flag_rect,
        },
        painter,
    )
}

pub fn draw_s5b_duty_noise_graph(
    ui: &mut egui::Ui,
    seq: &mut Sequence,
    playhead_step: Option<usize>,
    graph_height: f32,
) -> bool {
    let num_steps = seq.len();
    if num_steps == 0 {
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
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Empty Sequence (0 steps)",
            egui::FontId::proportional(13.0f32),
            theme::TEXT_DIM,
        );
        return false;
    }

    let (regions, painter) = layout_s5b_regions(ui, graph_height);
    let S5BRegions {
        rect,
        header_rect,
        duty_rect,
        period_rect,
        flag_rect,
    } = regions;

    let header_response = ui.interact(
        header_rect,
        ui.make_persistent_id("s5b_header_area"),
        Sense::click_and_drag(),
    );

    let step_width = rect.width() / num_steps as f32;
    let content_rect = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, header_rect.min.y));

    paint_content_backgrounds(&painter, content_rect, step_width, num_steps);
    // The gap between the duty and period sub-bars needs the panel
    // background painted over the striped columns underneath it.
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(content_rect.min.x, duty_rect.max.y),
            Pos2::new(content_rect.max.x, period_rect.min.y),
        ),
        0.0f32,
        theme::PANEL,
    );

    let mut changed = handle_marker_drag(
        ui,
        seq,
        &header_response,
        header_rect,
        num_steps,
        ui.make_persistent_id("s5b_loop_drag_state"),
        ui.make_persistent_id("s5b_release_drag_state"),
    );

    changed |= draw_duty_lane(ui, &painter, seq, duty_rect, step_width, num_steps);
    changed |= draw_period_lane(ui, &painter, seq, period_rect, step_width, num_steps);
    changed |= handle_flag_lane(ui, &painter, seq, flag_rect, step_width, num_steps);

    let loop_idx = seq.loop_point.unwrap_or(usize::MAX);
    let rel_idx = seq.release_point.unwrap_or(usize::MAX);
    paint_marker_header(
        &painter,
        header_rect,
        step_width,
        num_steps,
        loop_idx,
        rel_idx,
    );

    if let Some(step) = playhead_step.filter(|step| *step < num_steps) {
        let bar_x_min = content_rect.min.x + step as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0f32;
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

    // Painted last (on top of the playhead) to match the pre-decomposition
    // ordering: duty's own min/max labels are painted with its bars, but
    // period's were always the very last thing this function drew.
    paint_lane_min_max_labels(&painter, period_rect, "31", "0");

    changed
}

/// The alternating-shade background columns behind all three lanes.
fn paint_content_backgrounds(
    painter: &egui::Painter,
    content_rect: Rect,
    step_width: f32,
    num_steps: usize,
) {
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
}

/// Free-hand drag (primary button) and secondary-drag line-draw editing for
/// one bar lane, writing through `value_at` — the pointer-Y-to-step-value
/// conversion, which differs per lane (duty index vs raw period bits).
/// Shared by the duty and period lanes, whose interaction shape is otherwise
/// identical.
#[allow(clippy::too_many_arguments)]
fn handle_lane_drag_and_line_draw(
    ui: &egui::Ui,
    seq: &mut Sequence,
    response: &egui::Response,
    rect: Rect,
    step_width: f32,
    num_steps: usize,
    draw_drag_id: egui::Id,
    line_draw_id: egui::Id,
    value_at: impl Fn(i16, f32) -> i16,
) -> bool {
    let mut changed = false;

    if (response.dragged_by(egui::PointerButton::Primary)
        || response.clicked_by(egui::PointerButton::Primary))
        && let Some(pointer_pos) = response.interact_pointer_pos()
    {
        let last_pos: Option<Pos2> = ui.ctx().data_mut(|d| d.get_temp(draw_drag_id));
        let p0 = last_pos.unwrap_or(pointer_pos);

        for_each_step_between(rect, step_width, num_steps, p0, pointer_pos, 0.0, |s, y| {
            let new_value = value_at(seq.values[s], y);
            if new_value != seq.values[s] {
                seq.values[s] = new_value;
                changed = true;
            }
        });

        ui.ctx()
            .data_mut(|d| d.insert_temp(draw_drag_id, pointer_pos));
    }

    if response.drag_started_by(egui::PointerButton::Secondary)
        && let Some(pointer_pos) = response.interact_pointer_pos()
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

    if response.dragged_by(egui::PointerButton::Secondary)
        && let Some(pointer_pos) = response.interact_pointer_pos()
        && let Some(mut state) = ui
            .ctx()
            .data_mut(|d| d.get_temp::<LineDrawState>(line_draw_id))
    {
        state.last = pointer_pos;

        let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll_delta.abs() > 0.0f32 {
            state.tension = (state.tension + scroll_delta * 0.003f32).clamp(-1.0, 1.0);
        }

        ui.ctx().data_mut(|d| d.insert_temp(line_draw_id, state));

        for_each_step_between(
            rect,
            step_width,
            num_steps,
            state.start,
            state.last,
            state.tension,
            |s, y| {
                let new_value = value_at(seq.values[s], y);
                if new_value != seq.values[s] {
                    seq.values[s] = new_value;
                    changed = true;
                }
            },
        );
    }

    if response.drag_stopped_by(egui::PointerButton::Secondary)
        || response.lost_focus()
        || !response.hovered()
        || !ui.input(|i| i.pointer.secondary_down())
    {
        ui.ctx()
            .data_mut(|d| d.remove_temp::<LineDrawState>(line_draw_id));
    }

    if response.drag_stopped_by(egui::PointerButton::Primary)
        || !ui.input(|i| i.pointer.primary_down())
    {
        ui.ctx().data_mut(|d| d.remove_temp::<Pos2>(draw_drag_id));
    }

    changed
}

/// The duty-width sub-bar: drag/line-draw editing plus its 0..=8 bars and
/// min/max labels.
fn draw_duty_lane(
    ui: &egui::Ui,
    painter: &egui::Painter,
    seq: &mut Sequence,
    duty_rect: Rect,
    step_width: f32,
    num_steps: usize,
) -> bool {
    let duty_response = ui.interact(
        duty_rect,
        ui.make_persistent_id("s5b_duty_area"),
        Sense::click_and_drag(),
    );

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

    let line_draw_id = ui.make_persistent_id("s5b_duty_line_draw_state");
    let changed = handle_lane_drag_and_line_draw(
        ui,
        seq,
        &duty_response,
        duty_rect,
        step_width,
        num_steps,
        ui.make_persistent_id("s5b_duty_draw_last_pos"),
        line_draw_id,
        duty_y_to_value,
    );

    for i in 0..num_steps {
        let duty = s5b_duty_index(seq.values[i]).clamp(0, 8);
        let bar_x_min = duty_rect.min.x + i as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0f32;
        let norm_val = f32::from(duty) / 8.0f32;
        let bar_y_min = duty_rect.max.y - (norm_val * duty_rect.height());
        let bar_rect = Rect::from_min_max(
            Pos2::new(bar_x_min, bar_y_min),
            Pos2::new(bar_x_max, duty_rect.max.y),
        );
        painter.rect_filled(bar_rect, 1.0f32, theme::S5B_TONE_FLAG);
    }

    draw_line_draw_preview(painter, ui.ctx().data(|d| d.get_temp(line_draw_id)));

    paint_lane_min_max_labels(painter, duty_rect, "8", "0");

    changed
}

/// The noise-period sub-bar: drag/line-draw editing plus its 0..=31 bars and
/// min/max labels.
fn draw_period_lane(
    ui: &egui::Ui,
    painter: &egui::Painter,
    seq: &mut Sequence,
    period_rect: Rect,
    step_width: f32,
    num_steps: usize,
) -> bool {
    let period_response = ui.interact(
        period_rect,
        ui.make_persistent_id("s5b_period_area"),
        Sense::click_and_drag(),
    );

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

    let line_draw_id = ui.make_persistent_id("s5b_period_line_draw_state");
    let changed = handle_lane_drag_and_line_draw(
        ui,
        seq,
        &period_response,
        period_rect,
        step_width,
        num_steps,
        ui.make_persistent_id("s5b_period_draw_last_pos"),
        line_draw_id,
        period_y_to_value,
    );

    for i in 0..num_steps {
        let period = (seq.values[i] & S5B_PERIOD_MASK).clamp(0, 31);
        let bar_x_min = period_rect.min.x + i as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0f32;
        let norm_val = f32::from(period) / 31.0f32;
        let bar_y_min = period_rect.max.y - (norm_val * period_rect.height());
        let bar_rect = Rect::from_min_max(
            Pos2::new(bar_x_min, bar_y_min),
            Pos2::new(bar_x_max, period_rect.max.y),
        );
        painter.rect_filled(bar_rect, 1.0f32, Color32::from_rgb(220, 220, 220));
    }

    draw_line_draw_preview(painter, ui.ctx().data(|d| d.get_temp(line_draw_id)));

    changed
}

/// The small top/bottom value labels a bar lane paints in its top-left corner.
fn paint_lane_min_max_labels(painter: &egui::Painter, lane_rect: Rect, top: &str, bottom: &str) {
    painter.text(
        Pos2::new(lane_rect.min.x + 6.0f32, lane_rect.min.y + 2.0f32),
        egui::Align2::LEFT_TOP,
        top,
        egui::FontId::proportional(11.0f32),
        Color32::from_rgb(160, 160, 160),
    );
    painter.text(
        Pos2::new(lane_rect.min.x + 6.0f32, lane_rect.max.y - 14.0f32),
        egui::Align2::LEFT_TOP,
        bottom,
        egui::FontId::proportional(11.0f32),
        Color32::from_rgb(160, 160, 160),
    );
}

const FLAGS: [(i16, Color32); 2] = [
    (S5B_MODE_SQUARE, theme::S5B_TONE_FLAG),
    (S5B_MODE_NOISE, theme::S5B_NOISE_FLAG),
];
const FLAG_LABELS: [&str; 2] = ["T", "N"];

/// The tone/noise mixer flag row: two toggle buttons per step, drag-to-paint
/// across steps (each step visited during a drag toggles once).
fn handle_flag_lane(
    ui: &egui::Ui,
    painter: &egui::Painter,
    seq: &mut Sequence,
    flag_rect: Rect,
    step_width: f32,
    num_steps: usize,
) -> bool {
    let flag_response = ui.interact(
        flag_rect,
        ui.make_persistent_id("s5b_flag_area"),
        Sense::click_and_drag(),
    );

    let button_h = flag_rect.height() / FLAGS.len() as f32;
    let drag_id = ui.make_persistent_id("s5b_flag_drag_state");
    let mut changed = false;

    let toggling = flag_response.dragged_by(egui::PointerButton::Primary)
        || flag_response.clicked_by(egui::PointerButton::Primary);

    if toggling && let Some(pointer_pos) = flag_response.interact_pointer_pos() {
        let rel_x =
            (pointer_pos.x - flag_rect.min.x).clamp(0.0, (flag_rect.width() - 0.001).max(0.0));
        let step = (((rel_x / step_width).floor() as i32).clamp(0, num_steps as i32 - 1)) as usize;

        let state: S5BFlagDragState = ui
            .ctx()
            .data_mut(|d| d.get_temp(drag_id))
            .unwrap_or_default();

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

    paint_flag_lane(painter, seq, flag_rect, step_width, num_steps, button_h);

    changed
}

/// The flag row's per-step, per-flag colored button and label.
fn paint_flag_lane(
    painter: &egui::Painter,
    seq: &Sequence,
    flag_rect: Rect,
    step_width: f32,
    num_steps: usize,
    button_h: f32,
) {
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
            let color = if is_set {
                *on_color
            } else {
                theme::S5B_FLAG_OFF
            };
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
}
