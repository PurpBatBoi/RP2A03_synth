//! `rp2a03_common\src\gui\widgets\container.rs`
//! A bordered, titled group frame — the closest egui gets to a native
//! `GroupBox` out of the box.

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
