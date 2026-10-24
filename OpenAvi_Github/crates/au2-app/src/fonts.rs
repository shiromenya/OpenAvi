//! # fonts
//!
//! システムフォントの設定。
//! 美しく視認性に優れた「LINE Seed JP」を最優先の標準システムフォントとして登録し、
//! OS のフォントをフォールバックとして構成します。

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

/// 同梱の LINE Seed JP Regular フォントデータ
const LINE_SEED_JP_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/LINESeedJP-Regular.ttf");

const FALLBACK_CANDIDATES: &[&str] = &[
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

/// LINE Seed JP を最優先システムフォントとして登録します。
pub fn install_japanese_font(ctx: &egui::Context) -> Option<&'static str> {
    let mut fonts = FontDefinitions::default();

    // 1. LINE Seed JP を最優先フォントとして登録
    fonts.font_data.insert(
        "line_seed_jp".to_owned(),
        FontData::from_static(LINE_SEED_JP_REGULAR),
    );

    // 2. OS の日本語フォールバックフォントを探して追加
    for &path in FALLBACK_CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert("os_fallback".to_owned(), FontData::from_owned(bytes));
            break;
        }
    }

    // 3. Proportional（通常UIテキスト）と Monospace（等幅）の先頭に LINE Seed JP を配置
    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, "line_seed_jp".to_owned());
    if fonts.font_data.contains_key("os_fallback") {
        prop.push("os_fallback".to_owned());
    }

    let mono = fonts.families.entry(FontFamily::Monospace).or_default();
    mono.insert(0, "line_seed_jp".to_owned());
    if fonts.font_data.contains_key("os_fallback") {
        mono.push("os_fallback".to_owned());
    }

    ctx.set_fonts(fonts);
    log::info!("システムフォントに LINE Seed JP を適用しました");
    Some("LINE Seed JP")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_font_installation() {
        let ctx = egui::Context::default();
        let res = install_japanese_font(&ctx);
        assert_eq!(res, Some("LINE Seed JP"));
    }
}
