//! 各パネル（ソース・プレビュー・タイムライン・設定ダイアログ）の UI。

pub mod preview;
pub mod settings;
pub mod sources;
pub mod timeline;

use eframe::egui::{self, vec2, Color32};

/// `ffffff` 形式の色文字列を解釈します。
pub fn parse_color(hex: &str) -> Option<Color32> {
    let v = u32::from_str_radix(hex.trim(), 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// ボタンの標準高さ。
pub const BUTTON_H: f32 = 30.0;
/// 行内に置く小さめボタンの標準高さ。
pub const SMALL_BUTTON_H: f32 = 26.0;

/// 利用可能幅いっぱいに、同じ幅のボタンを整列して並べます（最終行は残りの数で等分）。
/// 各項目は `(ラベル, ツールチップ)`。クリックされた項目の添字を返します。
pub fn button_grid(ui: &mut egui::Ui, items: &[(egui::WidgetText, &str)], min_col_w: f32) -> Option<usize> {
    let spacing = ui.spacing().item_spacing;
    let avail = ui.available_width().floor().max(min_col_w);
    let cols = (((avail + spacing.x) / (min_col_w + spacing.x)).floor() as usize).clamp(1, items.len().max(1));
    let mut clicked = None;
    let mut idx = 0usize;
    for row in items.chunks(cols) {
        let n = row.len();
        let w = ((avail - spacing.x * (n as f32 - 1.0)) / n as f32).floor();
        ui.horizontal(|ui| {
            for (label, tip) in row {
                let mut resp = ui.add_sized(vec2(w, BUTTON_H), egui::Button::new(label.clone()));
                if !tip.is_empty() {
                    resp = resp.on_hover_text(*tip);
                }
                if resp.clicked() {
                    clicked = Some(idx);
                }
                idx += 1;
            }
        });
    }
    clicked
}

/// 一覧の行末などに置く、文字が切れない最小幅・十分な高さを持つ小ボタン。
pub fn small_button(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add(egui::Button::new(text).min_size(vec2(64.0, SMALL_BUTTON_H)))
}
