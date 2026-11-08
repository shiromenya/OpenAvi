//! 共通 UI ウィジェット。

use eframe::egui::{self, vec2, Color32, Stroke};

/// Word / モダン OS 風の美しいカプセル型トグルスイッチ UI。
pub fn toggle_switch_ui(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let desired_size = vec2(32.0, 18.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
        let bg_fill = if *on {
            Color32::from_rgb(37, 99, 235) // Office / Word 風の鮮やかなブルー
        } else {
            Color32::from_rgb(60, 64, 75)
        };
        let stroke_color = if response.hovered() {
            Color32::from_rgb(147, 197, 253)
        } else {
            Color32::from_gray(80)
        };
        ui.painter().rect(
            rect,
            rect.height() * 0.5,
            bg_fill,
            Stroke::new(1.0_f32, stroke_color),
        );
        let radius = 0.5 * rect.height() - 2.5;
        let x = egui::lerp(
            (rect.left() + radius + 3.0)..=(rect.right() - radius - 3.0),
            how_on,
        );
        let center = egui::pos2(x, rect.center().y);
        ui.painter().circle_filled(center, radius, Color32::WHITE);
    }
    response
}
