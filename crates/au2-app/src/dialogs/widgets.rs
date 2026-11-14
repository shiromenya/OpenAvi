//! 共通 UI ウィジェット。

use eframe::egui::{self, vec2, Color32, Stroke};

/// カプセル型トグルスイッチ UI。
pub fn toggle_switch_ui(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let desired_size = vec2(28.0, 15.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
        let bg_fill = if *on {
            crate::theme::accent()
        } else {
            crate::theme::flip(Color32::from_rgb(54, 54, 60))
        };
        let stroke_color = if response.hovered() {
            crate::theme::accent_hi()
        } else {
            crate::theme::border()
        };
        ui.painter().rect(
            rect,
            rect.height() * 0.5,
            bg_fill,
            Stroke::new(1.0_f32, stroke_color),
        );
        let radius = 0.5 * rect.height() - 2.0;
        let x = egui::lerp(
            (rect.left() + radius + 2.0)..=(rect.right() - radius - 2.0),
            how_on,
        );
        let center = egui::pos2(x, rect.center().y);
        ui.painter().circle_filled(center, radius, Color32::WHITE);
    }
    response
}
