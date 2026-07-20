use egui::{Color32, Style};

pub fn rp2a03_style() -> Style {
    let mut style = egui::Style::default();

    style.visuals.extreme_bg_color = Color32::from_rgb(14, 14, 14);
    style.visuals.panel_fill = Color32::from_rgb(14, 14, 14);
    style.visuals.override_text_color = Some(Color32::WHITE);
    style.visuals.popup_shadow.offset = [0, 0];
    style.visuals.popup_shadow.spread = 2;
    style
}

pub struct ParamResponse {
    pub changed: bool,
    pub drag_started: bool,
    pub drag_stopped: bool,
}

impl ParamResponse {
    pub fn from_response(response: &egui::Response) -> Self {
        Self {
            changed: response.changed(),
            drag_started: response.drag_started(),
            drag_stopped: response.drag_stopped(),
        }
    }
}

pub struct SynthUiResponses {
    pub mode: ParamResponse,
    pub duty: ParamResponse,
    pub volume: ParamResponse,
    pub noise_mode: ParamResponse,
}

pub fn draw_synth_ui(
    ui: &mut egui::Ui,
    mode: &mut usize,
    duty: &mut f32,
    volume: &mut f32,
    noise_mode: &mut bool,
) -> SynthUiResponses {
    let mut mode_changed = false;

    ui.vertical(|ui| {
        let mode_text = match *mode {
            0 => "2A03 Square",
            1 => "2A03 Triangle",
            2 => "2A03 Noise",
            _ => "Unknown",
        };

        egui::ComboBox::from_label("Mode")
            .selected_text(mode_text)
            .show_ui(ui, |ui| {
                if ui.selectable_value(mode, 0, "2A03 Square").clicked() {
                    mode_changed = true;
                }
                if ui.selectable_value(mode, 1, "2A03 Triangle").clicked() {
                    mode_changed = true;
                }
                if ui.selectable_value(mode, 2, "2A03 Noise").clicked() {
                    mode_changed = true;
                }
            });

        ui.add_space(20.0);

        let mut duty_response = None;
        let mut volume_response = None;
        let mut noise_mode_changed = false;

        ui.horizontal(|ui| {
            if *mode == 2 {
                // Noise mode: swap the Duty knob for a metallic-mode toggle
                if ui.checkbox(noise_mode, "Metallic").changed() {
                    noise_mode_changed = true;
                }
            } else {
                duty_response = Some(ui.add(
                    crate::knob::Knob::new(duty, 0.0, 3.0, crate::knob::KnobStyle::Wiper)
                        .with_label("Duty", crate::knob::LabelPosition::Bottom)
                        .with_step(1.0)
                        .with_size(35.0),
                ));
            }

            ui.add_space(30.0);

            volume_response = Some(ui.add(
                crate::knob::Knob::new(volume, 0.0, 15.0, crate::knob::KnobStyle::Wiper)
                    .with_label("Volume", crate::knob::LabelPosition::Bottom)
                    .with_step(1.0)
                    .with_size(35.0),
            ));
        });

        SynthUiResponses {
            mode: ParamResponse {
                changed: mode_changed,
                drag_started: false,
                drag_stopped: false,
            },
            duty: duty_response
                .map(|r| ParamResponse::from_response(&r))
                .unwrap_or(ParamResponse { changed: false, drag_started: false, drag_stopped: false }),
            volume: ParamResponse::from_response(&volume_response.unwrap()),
            noise_mode: ParamResponse {
                changed: noise_mode_changed,
                drag_started: false,
                drag_stopped: false,
            },
        }
    })
    .inner
}
