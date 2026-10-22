//! 各パネル（ソース・プレビュー・タイムライン・設定ダイアログ）の UI。

pub mod preview;
pub mod settings;
pub mod sources;
pub mod timeline;

use eframe::egui::Color32;

/// `ffffff` 形式の色文字列を解釈します。
pub fn parse_color(hex: &str) -> Option<Color32> {
    let v = u32::from_str_radix(hex.trim(), 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}
