//! `rp2a03_common\src\gui\widgets\repeating_button.rs`
//! A button that fires repeatedly while held, like a native spinner's
//! press-and-hold.

#[derive(Clone, Copy, Default)]
struct RepeatingButtonState {
    start_time: f64,
    last_trigger_time: f64,
}

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
