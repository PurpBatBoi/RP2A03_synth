    //! rp2a03_preview/src/main.rs
    //! Standalone egui preview window for the RP2A03 Synth UI.
    //! Run with: `cargo run -p rp2a03_preview`

    use eframe::egui;
    use rp2a03_common::{render_editor_ui, style, SharedSequences};
    use egui_extras;

    fn main() -> eframe::Result {
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("RP2A03 Synth — UI Preview")
                .with_inner_size([758.0, 506.0])
                .with_resizable(true),
            ..Default::default()
        };

        eframe::run_native(
            "RP2A03 Synth Preview",
            options,
            Box::new(|cc| {
                // Register PNG/JPEG/etc. loaders
                egui_extras::install_image_loaders(&cc.egui_ctx);

                // Apply the same dark theme as the plugin.
                cc.egui_ctx.set_style_of(egui::Theme::Dark, style());

                Ok(Box::new(PreviewApp::default()))
            }),
        )
    }

    struct PreviewApp {
        shared_sequences: SharedSequences,
        sequence_index: usize,
    }

    impl Default for PreviewApp {
        fn default() -> Self {
            Self {
                shared_sequences: SharedSequences::default(),
                sequence_index: 0,
            }
        }
    }

    impl eframe::App for PreviewApp {
        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            egui::CentralPanel::default().show(ui, |ui| {
                if let Some(new_index) =
                    render_editor_ui(ui, &mut self.shared_sequences, self.sequence_index)
                {
                    self.sequence_index = new_index;
                    self.shared_sequences
                        .set_all_selected_sequence_indices(new_index);
                }
            });
        }
    }
