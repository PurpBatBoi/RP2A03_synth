//! `rp2a03_common\src\gui\widgets\wavetable_graph.rs`
//! The FDS wave RAM step graph — drawn as filled steps or a connected line,
//! editable by dragging.

use super::common::for_each_step_between;
use crate::gui::theme;
use egui::{Pos2, Rect, Sense, Stroke, Vec2};

fn pos_y_to_wave_value(y: f32, graph_rect: Rect, max: u16) -> u16 {
    if graph_rect.height() <= 0.0 {
        return 0;
    }

    let rel_y = (y - graph_rect.min.y).clamp(0.0, graph_rect.height());
    let norm = 1.0 - rel_y / graph_rect.height();

    (norm * f32::from(max)).round().clamp(0.0, f32::from(max)) as u16
}

pub fn draw_wavetable_graph(
    ui: &mut egui::Ui,
    data: &mut [u16],
    max_val: u16,
    style: crate::gui::wavetable_state::DrawStyle,
    graph_height: f32,
    editable: bool,
) -> bool {
    let desired_size = Vec2::new(ui.available_width(), graph_height);
    let sense = if editable {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_at_least(desired_size, sense);

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0f32, theme::GRAPH_BG);
    painter.rect_stroke(
        rect,
        2.0f32,
        Stroke::new(1.0f32, theme::BORDER),
        egui::StrokeKind::Outside,
    );

    let num_steps = data.len();
    if num_steps == 0 || rect.width() <= 0.0 {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No wavetables — press + to add one",
            egui::FontId::proportional(13.0f32),
            theme::TEXT_DIM,
        );
        return false;
    }

    let step_width = rect.width() / num_steps as f32;
    let changed = editable && handle_wavetable_drag(ui, &response, rect, step_width, data, max_val);

    paint_wavetable_columns(&painter, rect, step_width, num_steps);
    paint_wavetable_values(&painter, rect, data, max_val, step_width, style);

    changed
}

/// Free-hand drag/click editing: drags a line between the last and current
/// pointer position, writing every step it crosses. Returns whether any
/// step's value actually changed.
fn handle_wavetable_drag(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: Rect,
    step_width: f32,
    data: &mut [u16],
    max_val: u16,
) -> bool {
    let mut changed = false;
    let drag_id = ui.make_persistent_id("wavetable_draw_last_pos");

    if (response.dragged_by(egui::PointerButton::Primary)
        || response.clicked_by(egui::PointerButton::Primary))
        && let Some(pointer_pos) = response.interact_pointer_pos()
    {
        let last_pos: Option<Pos2> = ui.ctx().data_mut(|d| d.get_temp(drag_id));
        let p0 = last_pos.unwrap_or(pointer_pos);

        for_each_step_between(
            rect,
            step_width,
            data.len(),
            p0,
            pointer_pos,
            0.0,
            |s, y| {
                let value = pos_y_to_wave_value(y, rect, max_val);
                if data[s] != value {
                    data[s] = value;
                    changed = true;
                }
            },
        );

        ui.ctx().data_mut(|d| d.insert_temp(drag_id, pointer_pos));
    }

    if response.drag_stopped_by(egui::PointerButton::Primary)
        || !ui.input(|i| i.pointer.primary_down())
    {
        ui.ctx().data_mut(|d| d.remove_temp::<Pos2>(drag_id));
    }

    changed
}

/// The alternating-shade background columns, one per step.
fn paint_wavetable_columns(painter: &egui::Painter, rect: Rect, step_width: f32, num_steps: usize) {
    for i in 0..num_steps {
        let x_min = rect.min.x + i as f32 * step_width;
        let x_max = x_min + step_width - 1.0f32;
        let col = Rect::from_min_max(
            Pos2::new(x_min, rect.min.y),
            Pos2::new(x_max.max(x_min), rect.max.y),
        );
        let bg = if i % 2 == 0 {
            theme::GRAPH_BG
        } else {
            theme::GRAPH_ALT
        };
        painter.rect_filled(col, 0.0f32, bg);
    }
}

/// The wave data itself, as filled step blocks or a connected line.
fn paint_wavetable_values(
    painter: &egui::Painter,
    rect: Rect,
    data: &[u16],
    max_val: u16,
    step_width: f32,
    style: crate::gui::wavetable_state::DrawStyle,
) {
    use crate::gui::wavetable_state::DrawStyle;

    let value_y = |val: u16| -> f32 {
        let norm = f32::from(val) / f32::from(max_val.max(1));
        rect.max.y - norm * rect.height()
    };

    match style {
        DrawStyle::Steps => {
            let num_slots = f32::from(max_val) + 1.0;
            let slot_h = rect.height() / num_slots;

            for (i, &val) in data.iter().enumerate() {
                let val = val.min(max_val);
                let x_min = rect.min.x + i as f32 * step_width;

                let x_max = x_min + step_width - 1.0f32;

                let y_min = rect.min.y + f32::from(max_val - val) * slot_h;
                let block = Rect::from_min_max(
                    Pos2::new(x_min + 0.5f32, y_min + 0.5f32),
                    Pos2::new(
                        (x_max - 0.5f32).max(x_min),
                        (y_min + slot_h - 0.5f32).max(y_min),
                    ),
                );
                painter.rect_filled(block, 1.0f32, theme::TEXT);
            }
        }
        DrawStyle::Lines => {
            let mut points = Vec::with_capacity(data.len() + 2);
            points.push(Pos2::new(rect.min.x, value_y(data[0].min(max_val))));
            points.extend(data.iter().enumerate().map(|(i, &val)| {
                Pos2::new(
                    rect.min.x + (i as f32 + 0.5) * step_width,
                    value_y(val.min(max_val)),
                )
            }));
            points.push(Pos2::new(
                rect.max.x,
                value_y(data[data.len() - 1].min(max_val)),
            ));

            painter.add(egui::Shape::line(points, Stroke::new(2.0f32, theme::TEXT)));
        }
    }
}
