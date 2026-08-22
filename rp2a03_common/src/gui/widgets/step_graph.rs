//! `rp2a03_common\src\gui\widgets\step_graph.rs`
//! The general-purpose envelope/mod-table step graph: bars, blocks, or a
//! scrollable arpeggio piano roll, with loop/release marker dragging and a
//! secondary-drag line-draw tool.

use super::common::{
    LineDrawState, draw_line_draw_preview, draw_playhead_rect, for_each_step_between,
    handle_marker_drag, paint_marker_header,
};
use crate::gui::theme;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use rp2a03_core::sequencer::Sequence;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GraphStyle {
    Bars,

    Blocks,

    Arpeggio,
}

#[derive(Clone, Copy)]
struct StepGraphOpts {
    style: GraphStyle,
    playhead_step: Option<usize>,

    markers: bool,
}

pub fn draw_envelope_bar_graph(
    ui: &mut egui::Ui,
    seq: &mut Sequence,
    min_val: i16,
    max_val: i16,
    style: GraphStyle,
    playhead_step: Option<usize>,
    graph_height: f32,
) -> bool {
    draw_step_graph(
        ui,
        seq,
        min_val,
        max_val,
        graph_height,
        StepGraphOpts {
            style,
            playhead_step,
            markers: true,
        },
    )
}

pub fn draw_mod_table_graph(
    ui: &mut egui::Ui,
    seq: &mut Sequence,
    min_val: i16,
    max_val: i16,
    graph_height: f32,
) -> bool {
    draw_step_graph(
        ui,
        seq,
        min_val,
        max_val,
        graph_height,
        StepGraphOpts {
            style: GraphStyle::Blocks,
            playhead_step: None,
            markers: false,
        },
    )
}

// Orchestration, not real complexity: every step here calls one of the
// already-decomposed helpers below and threads their results to the next —
// same exception pattern already used for `PatchError`'s `Display` impl.
#[allow(clippy::too_many_lines)]
fn draw_step_graph(
    ui: &mut egui::Ui,
    seq: &mut Sequence,
    min_val: i16,
    max_val: i16,
    graph_height: f32,
    opts: StepGraphOpts,
) -> bool {
    let StepGraphOpts {
        style,
        playhead_step,
        markers,
    } = opts;
    let is_arpeggio = style == GraphStyle::Arpeggio;
    let blocks = matches!(style, GraphStyle::Blocks | GraphStyle::Arpeggio);

    let visible_span = 10i16;
    let min_center = (min_val + visible_span).min(0);
    let max_center = (max_val - visible_span).max(0);

    let regions = layout_step_graph_regions(ui, graph_height, markers, is_arpeggio);
    let StepGraphRegions {
        rect,
        graph_rect,
        scrollbar_rect,
        header_rect,
        painter,
        graph_response,
        header_response,
        scrollbar_response,
    } = regions;

    let frame = prepare_step_graph_frame(
        ui,
        seq,
        &graph_response,
        &scrollbar_response,
        scrollbar_rect,
        is_arpeggio,
        min_val,
        max_val,
        min_center,
        max_center,
        visible_span,
    );
    let StepGraphFrame {
        num_steps,
        scroll_center,
        vis_min,
        vis_max,
        loop_drag_id,
        rel_drag_id,
    } = frame;

    let text_needs_sync = dispatch_step_graph_interaction(
        ui,
        seq,
        &header_response,
        header_rect,
        &graph_response,
        graph_rect,
        num_steps,
        markers,
        is_arpeggio,
        vis_min,
        vis_max,
        min_val,
        max_val,
        loop_drag_id,
        rel_drag_id,
    );

    if is_arpeggio {
        paint_arpeggio_scrollbar(
            &painter,
            scrollbar_rect,
            min_val,
            max_val,
            min_center,
            max_center,
            scroll_center,
            visible_span,
        );
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

    let (step_width, loop_idx, rel_idx) = paint_step_graph_body(
        &painter,
        graph_rect,
        seq,
        num_steps,
        min_val,
        max_val,
        vis_min,
        vis_max,
        blocks,
        is_arpeggio,
    );

    paint_step_graph_overlays(
        ui,
        &painter,
        graph_rect,
        header_rect,
        step_width,
        num_steps,
        playhead_step,
        markers,
        loop_idx,
        rel_idx,
        vis_min,
        vis_max,
    );

    text_needs_sync
}

/// Everything painted on top of the bars: the playhead marker, the in-progress
/// line-draw preview, the loop/release header (if any), and the min/max labels.
#[allow(clippy::too_many_arguments)]
fn paint_step_graph_overlays(
    ui: &egui::Ui,
    painter: &egui::Painter,
    graph_rect: Rect,
    header_rect: Rect,
    step_width: f32,
    num_steps: usize,
    playhead_step: Option<usize>,
    markers: bool,
    loop_idx: usize,
    rel_idx: usize,
    vis_min: i16,
    vis_max: i16,
) {
    if let Some(step) = playhead_step.filter(|step| *step < num_steps) {
        let bar_x_min = graph_rect.min.x + step as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0f32;
        let col_rect = Rect::from_min_max(
            Pos2::new(bar_x_min, graph_rect.min.y),
            Pos2::new(bar_x_max, graph_rect.max.y),
        );
        draw_playhead_rect(painter, col_rect);
    }

    draw_line_draw_preview(
        painter,
        ui.ctx()
            .data(|d| d.get_temp(ui.make_persistent_id("envelope_line_draw_state"))),
    );

    if markers {
        paint_marker_header(
            painter,
            header_rect,
            step_width,
            num_steps,
            loop_idx,
            rel_idx,
        );
    }

    painter.text(
        Pos2::new(graph_rect.min.x + 6.0f32, graph_rect.min.y + 2.0f32),
        egui::Align2::LEFT_TOP,
        format!("{vis_max}"),
        egui::FontId::proportional(11.0f32),
        Color32::from_rgb(160, 160, 160),
    );
    painter.text(
        Pos2::new(graph_rect.min.x + 6.0f32, graph_rect.max.y - 14.0f32),
        egui::Align2::LEFT_TOP,
        format!("{vis_min}"),
        egui::FontId::proportional(11.0f32),
        Color32::from_rgb(160, 160, 160),
    );
}

/// The rects and interaction responses `draw_step_graph` lays everything out
/// against: the whole widget, the main graph area, the arpeggio scrollbar (if
/// any), and the loop/release marker header (if any).
struct StepGraphRegions {
    rect: Rect,
    graph_rect: Rect,
    scrollbar_rect: Rect,
    header_rect: Rect,
    painter: egui::Painter,
    graph_response: egui::Response,
    header_response: egui::Response,
    scrollbar_response: egui::Response,
}

/// The first primary/secondary click or drag on an empty sequence seeds it
/// with a starting length, so there's something to draw on. Returns the
/// (possibly updated) step count.
const MIN_DRAW_STEPS: usize = 5;
const MIN_LINE_DRAW_STEPS: usize = 10;
/// Per-frame state `draw_step_graph` derives before handling input or
/// painting: the (possibly just-seeded) step count, the arpeggio scroll
/// center and its resulting visible value range, and the marker drag-state IDs.
struct StepGraphFrame {
    num_steps: usize,
    scroll_center: i16,
    vis_min: i16,
    vis_max: i16,
    loop_drag_id: egui::Id,
    rel_drag_id: egui::Id,
}

#[allow(clippy::too_many_arguments)]
fn prepare_step_graph_frame(
    ui: &egui::Ui,
    seq: &mut Sequence,
    graph_response: &egui::Response,
    scrollbar_response: &egui::Response,
    scrollbar_rect: Rect,
    is_arpeggio: bool,
    min_val: i16,
    max_val: i16,
    min_center: i16,
    max_center: i16,
    visible_span: i16,
) -> StepGraphFrame {
    let mut num_steps = seq.len();

    let scroll_id = ui.make_persistent_id("arpeggio_scroll_center");
    let mut scroll_center: i16 = ui.ctx().data_mut(|d| d.get_temp(scroll_id)).unwrap_or(0i16);
    if is_arpeggio {
        scroll_center = scroll_center.clamp(min_center, max_center);
    }

    num_steps = resize_empty_sequence_on_first_interaction(seq, graph_response, num_steps);

    scroll_center = update_arpeggio_scroll(
        ui,
        graph_response,
        scrollbar_response,
        scrollbar_rect,
        is_arpeggio,
        scroll_center,
        min_center,
        max_center,
        visible_span,
        min_val,
        max_val,
    );

    ui.ctx()
        .data_mut(|d| d.insert_temp(scroll_id, scroll_center));

    let (vis_min, vis_max) = if is_arpeggio {
        (
            (scroll_center - visible_span).max(min_val),
            (scroll_center + visible_span).min(max_val),
        )
    } else {
        (min_val, max_val)
    };

    StepGraphFrame {
        num_steps,
        scroll_center,
        vis_min,
        vis_max,
        loop_drag_id: ui.make_persistent_id("loop_drag_state"),
        rel_drag_id: ui.make_persistent_id("release_drag_state"),
    }
}

/// Marker drag (header) and step-draw/line-draw (graph) interaction, in one
/// call so `draw_step_graph` doesn't have to sequence and OR the two itself.
/// Returns whether either changed the sequence's text representation.
#[allow(clippy::too_many_arguments)]
fn dispatch_step_graph_interaction(
    ui: &egui::Ui,
    seq: &mut Sequence,
    header_response: &egui::Response,
    header_rect: Rect,
    graph_response: &egui::Response,
    graph_rect: Rect,
    num_steps: usize,
    markers: bool,
    is_arpeggio: bool,
    vis_min: i16,
    vis_max: i16,
    min_val: i16,
    max_val: i16,
    loop_drag_id: egui::Id,
    rel_drag_id: egui::Id,
) -> bool {
    if num_steps == 0 {
        return false;
    }

    let mut text_needs_sync = false;
    if markers {
        text_needs_sync |= handle_marker_drag(
            ui,
            seq,
            header_response,
            header_rect,
            num_steps,
            loop_drag_id,
            rel_drag_id,
        );
    }
    text_needs_sync |= handle_step_drag_and_line_draw(
        ui,
        seq,
        graph_response,
        graph_rect,
        num_steps,
        is_arpeggio,
        vis_min,
        vis_max,
        min_val,
        max_val,
    );
    text_needs_sync
}

fn resize_empty_sequence_on_first_interaction(
    seq: &mut Sequence,
    graph_response: &egui::Response,
    num_steps: usize,
) -> usize {
    if num_steps != 0 {
        return num_steps;
    }
    if graph_response.clicked_by(egui::PointerButton::Primary)
        || graph_response.drag_started_by(egui::PointerButton::Primary)
        || graph_response.dragged_by(egui::PointerButton::Primary)
    {
        seq.values.resize(MIN_DRAW_STEPS, 0);
        MIN_DRAW_STEPS
    } else if graph_response.drag_started_by(egui::PointerButton::Secondary)
        || graph_response.dragged_by(egui::PointerButton::Secondary)
    {
        seq.values.resize(MIN_LINE_DRAW_STEPS, 0);
        MIN_LINE_DRAW_STEPS
    } else {
        num_steps
    }
}

/// Scroll-wheel and scrollbar (buttons + thumb drag) input for the arpeggio
/// lane's vertical scroll position. Returns the (possibly updated) center note.
#[allow(clippy::too_many_arguments)]
fn update_arpeggio_scroll(
    ui: &egui::Ui,
    graph_response: &egui::Response,
    scrollbar_response: &egui::Response,
    scrollbar_rect: Rect,
    is_arpeggio: bool,
    mut scroll_center: i16,
    min_center: i16,
    max_center: i16,
    visible_span: i16,
    min_val: i16,
    max_val: i16,
) -> i16 {
    if !is_arpeggio {
        return scroll_center;
    }

    let line_drawing = graph_response.dragged_by(egui::PointerButton::Secondary)
        || graph_response.drag_started_by(egui::PointerButton::Secondary);

    if !line_drawing && (graph_response.hovered() || scrollbar_response.hovered()) {
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
        let visible_rows = f32::from(2 * visible_span + 1);
        let total_rows = f32::from(max_val - min_val + 1);
        let thumb_h =
            (track_rect.height() * (visible_rows / total_rows)).clamp(16.0f32, track_rect.height());
        let travel_h = track_rect.height() - thumb_h;
        if travel_h > 0.0f32 {
            let rel_y = pos.y - track_rect.min.y - thumb_h / 2.0f32;
            let norm = (rel_y / travel_h).clamp(0.0f32, 1.0f32);
            let center_range = f32::from(max_center - min_center);
            scroll_center = (f32::from(max_center) - norm * center_range).round() as i16;
            scroll_center = scroll_center.clamp(min_center, max_center);
        }
    }

    scroll_center
}

fn layout_step_graph_regions(
    ui: &mut egui::Ui,
    graph_height: f32,
    markers: bool,
    is_arpeggio: bool,
) -> StepGraphRegions {
    let desired_size = Vec2::new(ui.available_width(), graph_height);
    let (rect, _response) = ui.allocate_at_least(desired_size, Sense::hover());

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0f32, theme::GRAPH_BG);
    painter.rect_stroke(
        rect,
        2.0f32,
        Stroke::new(1.0f32, Color32::from_rgb(35, 35, 35)),
        egui::StrokeKind::Outside,
    );

    let scrollbar_width = if is_arpeggio { 14.0f32 } else { 0.0f32 };
    let header_height = if markers { 20.0f32 } else { 0.0f32 };

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

    let graph_response = ui.interact(
        graph_rect,
        ui.make_persistent_id("arpeggio_graph_area"),
        Sense::click_and_drag(),
    );

    let header_response = if markers {
        ui.interact(
            header_rect,
            ui.make_persistent_id("arpeggio_header_area"),
            Sense::click_and_drag(),
        )
    } else {
        ui.interact(
            Rect::NOTHING,
            ui.make_persistent_id("arpeggio_header_dummy"),
            Sense::hover(),
        )
    };
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

    StepGraphRegions {
        rect,
        graph_rect,
        scrollbar_rect,
        header_rect,
        painter,
        graph_response,
        header_response,
        scrollbar_response,
    }
}

/// Free-hand step drawing (primary button) and the secondary-drag line-draw
/// tool, in the main graph area. Returns whether any step value changed.
#[allow(clippy::too_many_arguments)]
fn handle_step_drag_and_line_draw(
    ui: &egui::Ui,
    seq: &mut Sequence,
    graph_response: &egui::Response,
    graph_rect: Rect,
    num_steps: usize,
    is_arpeggio: bool,
    vis_min: i16,
    vis_max: i16,
    min_val: i16,
    max_val: i16,
) -> bool {
    let step_width = graph_rect.width() / num_steps as f32;

    let mut text_needs_sync = handle_primary_step_drag(
        ui,
        seq,
        graph_response,
        graph_rect,
        step_width,
        num_steps,
        is_arpeggio,
        vis_min,
        vis_max,
        min_val,
        max_val,
    );

    text_needs_sync |= handle_secondary_line_draw(
        ui,
        seq,
        graph_response,
        graph_rect,
        step_width,
        num_steps,
        is_arpeggio,
        vis_min,
        vis_max,
        min_val,
        max_val,
    );

    if graph_response.drag_stopped_by(egui::PointerButton::Primary)
        || graph_response.clicked_by(egui::PointerButton::Primary)
    {
        text_needs_sync = true;
    }

    text_needs_sync
}

/// Free-hand step drawing with the primary button: writes every step the
/// pointer crosses between the last frame's position and this one's.
#[allow(clippy::too_many_arguments)]
fn handle_primary_step_drag(
    ui: &egui::Ui,
    seq: &mut Sequence,
    graph_response: &egui::Response,
    graph_rect: Rect,
    step_width: f32,
    num_steps: usize,
    is_arpeggio: bool,
    vis_min: i16,
    vis_max: i16,
    min_val: i16,
    max_val: i16,
) -> bool {
    let mut text_needs_sync = false;
    let draw_drag_id = ui.make_persistent_id("envelope_draw_last_pos");

    if (graph_response.dragged_by(egui::PointerButton::Primary)
        || graph_response.clicked_by(egui::PointerButton::Primary))
        && let Some(pointer_pos) = graph_response.interact_pointer_pos()
    {
        let last_pos: Option<Pos2> = ui.ctx().data_mut(|d| d.get_temp(draw_drag_id));

        let p0 = last_pos.unwrap_or(pointer_pos);
        let p1 = pointer_pos;

        for_each_step_between(graph_rect, step_width, num_steps, p0, p1, 0.0, |s, y| {
            let clamped_val = pos_y_to_val(
                y,
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
        });

        ui.ctx()
            .data_mut(|d| d.insert_temp(draw_drag_id, pointer_pos));
    }

    if graph_response.drag_stopped_by(egui::PointerButton::Primary)
        || !ui.input(|i| i.pointer.primary_down())
    {
        ui.ctx().data_mut(|d| d.remove_temp::<Pos2>(draw_drag_id));
    }

    text_needs_sync
}

/// The secondary-drag line-draw tool: draws a (optionally curved, via
/// scroll-wheel tension) line between drag start and the current pointer,
/// writing every step it crosses.
#[allow(clippy::too_many_arguments)]
fn handle_secondary_line_draw(
    ui: &egui::Ui,
    seq: &mut Sequence,
    graph_response: &egui::Response,
    graph_rect: Rect,
    step_width: f32,
    num_steps: usize,
    is_arpeggio: bool,
    vis_min: i16,
    vis_max: i16,
    min_val: i16,
    max_val: i16,
) -> bool {
    let mut text_needs_sync = false;
    let line_draw_id = ui.make_persistent_id("envelope_line_draw_state");

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
                    y,
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

    text_needs_sync
}

/// The arpeggio lane's scroll thumb, track, and up/down buttons.
#[allow(clippy::too_many_arguments)]
fn paint_arpeggio_scrollbar(
    painter: &egui::Painter,
    scrollbar_rect: Rect,
    min_val: i16,
    max_val: i16,
    min_center: i16,
    max_center: i16,
    scroll_center: i16,
    visible_span: i16,
) {
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

    let center_range = f32::from((max_center - min_center).max(1));
    let norm_center = f32::from(max_center - scroll_center) / center_range;
    let visible_rows = f32::from(2 * visible_span + 1);
    let total_rows = f32::from(max_val - min_val + 1);
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

/// The alternating-shade background columns, one per step.
/// Paints the graph body — background columns, grid/zero lines, and every
/// step's bar or block — and returns the `(step_width, loop_idx, rel_idx)`
/// the overlay painting that follows still needs.
#[allow(clippy::too_many_arguments)]
fn paint_step_graph_body(
    painter: &egui::Painter,
    graph_rect: Rect,
    seq: &Sequence,
    num_steps: usize,
    min_val: i16,
    max_val: i16,
    vis_min: i16,
    vis_max: i16,
    blocks: bool,
    is_arpeggio: bool,
) -> (f32, usize, usize) {
    let step_width = graph_rect.width() / num_steps as f32;
    let loop_idx = seq.loop_point.unwrap_or(usize::MAX);
    let rel_idx = seq.release_point.unwrap_or(usize::MAX);
    let is_bipolar = !is_arpeggio && min_val < 0;

    paint_step_backgrounds(painter, graph_rect, step_width, num_steps);
    paint_step_grid_lines(
        painter,
        graph_rect,
        is_arpeggio,
        is_bipolar,
        vis_min,
        vis_max,
        min_val,
        max_val,
    );
    paint_step_bars(
        painter, graph_rect, seq, step_width, num_steps, min_val, max_val, vis_min, vis_max,
        blocks, is_bipolar, loop_idx, rel_idx,
    );

    (step_width, loop_idx, rel_idx)
}

fn paint_step_backgrounds(
    painter: &egui::Painter,
    graph_rect: Rect,
    step_width: f32,
    num_steps: usize,
) {
    for i in 0..num_steps {
        let bar_x_min = graph_rect.min.x + i as f32 * step_width;
        let bar_x_max = bar_x_min + step_width - 1.0f32;

        let col_rect = Rect::from_min_max(
            Pos2::new(bar_x_min, graph_rect.min.y),
            Pos2::new(bar_x_max, graph_rect.max.y),
        );
        let bg_color = if i % 2 == 0 {
            theme::GRAPH_BG
        } else {
            theme::GRAPH_ALT
        };
        painter.rect_filled(col_rect, 0.0f32, bg_color);
    }
}

/// The arpeggio lane's horizontal semitone gridlines, or the bipolar zero
/// line for a bars/blocks graph that spans negative values.
#[allow(clippy::too_many_arguments)]
fn paint_step_grid_lines(
    painter: &egui::Painter,
    graph_rect: Rect,
    is_arpeggio: bool,
    is_bipolar: bool,
    vis_min: i16,
    vis_max: i16,
    min_val: i16,
    max_val: i16,
) {
    if is_arpeggio {
        let num_slots = f32::from(vis_max - vis_min + 1);
        let slot_h = graph_rect.height() / num_slots;
        for slot in 0..=(vis_max - vis_min) {
            let val = vis_max - slot;
            let y = graph_rect.min.y + f32::from(slot) * slot_h;
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
        let range = (f32::from(max_val) - f32::from(min_val)).max(1.0f32);
        let norm_zero = (0.0f32 - f32::from(min_val)) / range;
        let zero_y = graph_rect.max.y - (norm_zero * graph_rect.height());

        painter.line_segment(
            [
                Pos2::new(graph_rect.min.x, zero_y),
                Pos2::new(graph_rect.max.x, zero_y),
            ],
            Stroke::new(1.0f32, Color32::from_rgb(90, 90, 90)),
        );
    }
}

/// Every step's bar or block, colored by whether it falls in the loop
/// and/or release region.
#[allow(clippy::too_many_arguments)]
fn paint_step_bars(
    painter: &egui::Painter,
    graph_rect: Rect,
    seq: &Sequence,
    step_width: f32,
    num_steps: usize,
    min_val: i16,
    max_val: i16,
    vis_min: i16,
    vis_max: i16,
    blocks: bool,
    is_bipolar: bool,
    loop_idx: usize,
    rel_idx: usize,
) {
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

        if blocks {
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
                let num_slots = f32::from(vis_max - vis_min + 1);
                let slot_h = graph_rect.height() / num_slots;
                let slot_idx = f32::from(vis_max - val);
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
                let range = (f32::from(max_val) - f32::from(min_val)).max(1.0f32);
                let norm_zero = (0.0f32 - f32::from(min_val)) / range;
                let zero_y = graph_rect.max.y - (norm_zero * graph_rect.height());
                let norm_val = (f32::from(val) - f32::from(min_val)) / range;
                let bar_y = graph_rect.max.y - (norm_val * graph_rect.height());
                if val >= 0 {
                    Rect::from_min_max(Pos2::new(bar_x_min, bar_y), Pos2::new(bar_x_max, zero_y))
                } else {
                    Rect::from_min_max(Pos2::new(bar_x_min, zero_y), Pos2::new(bar_x_max, bar_y))
                }
            } else {
                let range = f32::from(max_val.max(1));
                let norm_val = f32::from(val) / range;
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

    let num_slots = f32::from(range_max - range_min + 1);
    let slot_idx = (norm_y * num_slots).floor() as i16;
    (range_max - slot_idx).clamp(range_min, range_max)
}
