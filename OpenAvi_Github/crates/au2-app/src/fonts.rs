//! 日本語フォントの読み込み。
//!
//! egui の既定フォントには CJK グリフが含まれないため、OS のフォントをフォールバックとして追加します。

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

const CANDIDATES: &[&str] = &[
    // Windows
    r"C:\Windows\Fonts\YuGothM.ttc",
    r"C:\Windows\Fonts\meiryo.ttc",
    r"C:\Windows\Fonts\msgothic.ttc",
    // macOS
    "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    // Linux
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
];

/// 見つかった日本語フォントを登録し、そのパスを返します。
pub fn install_japanese_font(ctx: &egui::Context) -> Option<&'static str> {
    for &path in CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let mut fonts = FontDefinitions::default();
        fonts.font_data.insert("jp".to_owned(), FontData::from_owned(bytes));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push("jp".to_owned());
        }
        ctx.set_fonts(fonts);
        log::info!("日本語フォント: {path}");
        return Some(path);
    }
    log::warn!("日本語フォントが見つかりませんでした");
    None
}
