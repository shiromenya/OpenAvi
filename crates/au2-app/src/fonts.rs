//! # fonts
//!
//! システムフォントの設定。
//! 美しく視認性に優れた「LINE Seed JP」を最優先の標準システムフォントとして登録し、
//! OS のフォントを動的に検出してオブジェクトごとのフォント切り替え（メイリオ、游ゴシック、明朝等）に対応します。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
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

/// 代表的なフォント表示名と対応ファイル名の候補マッピングテーブル
const FONT_NAME_TO_FILES: &[(&str, &[&str])] = &[
    ("LINE Seed JP", &[]),
    ("メイリオ", &[r"C:\Windows\Fonts\meiryo.ttc", r"C:\Windows\Fonts\meiryob.ttc"]),
    ("Meiryo UI", &[r"C:\Windows\Fonts\meiryo.ttc", r"C:\Windows\Fonts\meiryob.ttc"]),
    ("游ゴシック", &[r"C:\Windows\Fonts\YuGothM.ttc", r"C:\Windows\Fonts\YuGothR.ttc", r"C:\Windows\Fonts\YuGothB.ttc"]),
    ("Yu Gothic UI", &[r"C:\Windows\Fonts\YuGothM.ttc", r"C:\Windows\Fonts\YuGothR.ttc"]),
    ("MS ゴシック", &[r"C:\Windows\Fonts\msgothic.ttc"]),
    ("MS Pゴシック", &[r"C:\Windows\Fonts\msgothic.ttc"]),
    ("MS UI Gothic", &[r"C:\Windows\Fonts\msgothic.ttc"]),
    ("MS 明朝", &[r"C:\Windows\Fonts\msmincho.ttc"]),
    ("MS P明朝", &[r"C:\Windows\Fonts\msmincho.ttc"]),
    ("BIZ UDPGothic", &[r"C:\Windows\Fonts\BIZ-UDGothicR.ttc", r"C:\Windows\Fonts\BIZ-UDPGothicR.ttc"]),
    ("BIZ UDPMincho", &[r"C:\Windows\Fonts\BIZ-UDMinchoM.ttc", r"C:\Windows\Fonts\BIZ-UDPMinchoM.ttc"]),
    ("UD デジタル 教科書体", &[r"C:\Windows\Fonts\UDDigiKyokashoN-R.ttc", r"C:\Windows\Fonts\UDDigiKyokashoNP-R.ttc"]),
    ("Arial", &[r"C:\Windows\Fonts\arial.ttf", "/System/Library/Fonts/Supplemental/Arial.ttf"]),
    ("Times New Roman", &[r"C:\Windows\Fonts\times.ttf", "/System/Library/Fonts/Supplemental/Times New Roman.ttf"]),
    ("Impact", &[r"C:\Windows\Fonts\impact.ttf", "/System/Library/Fonts/Supplemental/Impact.ttf"]),
    ("Comic Sans MS", &[r"C:\Windows\Fonts\comic.ttf", "/System/Library/Fonts/Supplemental/Comic Sans MS.ttf"]),
    ("Consolas", &[r"C:\Windows\Fonts\consola.ttf"]),
    ("Segoe UI", &[r"C:\Windows\Fonts\segoeui.ttf"]),
    // macOS
    ("Hiragino Sans", &["/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", "/System/Library/Fonts/Hiragino Sans GB.ttc"]),
    ("ヒラギノ明朝", &["/System/Library/Fonts/ヒラギノ明朝 ProN.ttc"]),
    // Linux
    ("Noto Sans CJK JP", &["/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc"]),
    ("Noto Serif CJK JP", &["/usr/share/fonts/opentype/noto/NotoSerifCJK-Regular.ttc", "/usr/share/fonts/noto-cjk/NotoSerifCJK-Regular.ttc"]),
];

static LOADED_FONTS: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static REGISTERED_FAMILIES: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// 利用可能なフォント候補一覧を取得します。
pub fn list_available_fonts() -> Vec<String> {
    let mut list = Vec::new();
    for &(name, files) in FONT_NAME_TO_FILES {
        if files.is_empty() {
            list.push(name.to_string());
            continue;
        }
        let exists = files.iter().any(|f| Path::new(f).exists());
        if exists {
            list.push(name.to_string());
        }
    }
    list
}

/// 指定したフォントファミリが `egui::Context` に登録済みかどうかを確認します。
pub fn is_font_family_registered(font_name: &str) -> bool {
    let lock = REGISTERED_FAMILIES.lock().unwrap();
    lock.as_ref().map(|set| set.contains(font_name)).unwrap_or(false)
}

/// 指定したフォント名に対応するフォントファイルを動的に `egui::Context` の `FontDefinitions` にロード・登録します。
pub fn ensure_font_family(ctx: &egui::Context, font_name: &str) -> bool {
    if font_name.is_empty() || font_name.eq_ignore_ascii_case("LINE Seed JP") {
        return false;
    }

    let mut lock = LOADED_FONTS.lock().unwrap();
    let loaded = lock.get_or_insert_with(HashSet::new);
    if loaded.contains(font_name) {
        return is_font_family_registered(font_name);
    }
    loaded.insert(font_name.to_string());

    // 対応するフォントファイルを探す
    let mut font_bytes: Option<Vec<u8>> = None;
    for &(name, files) in FONT_NAME_TO_FILES {
        if name.eq_ignore_ascii_case(font_name) {
            for &f in files {
                if let Ok(b) = std::fs::read(f) {
                    font_bytes = Some(b);
                    break;
                }
            }
            break;
        }
    }

    // テーブルにない場合は C:\Windows\Fonts や OS ディレクトリから同名ファイルを探索
    if font_bytes.is_none() {
        let clean = font_name.replace(' ', "").to_lowercase();
        let dirs = &[
            PathBuf::from(r"C:\Windows\Fonts"),
            PathBuf::from("/System/Library/Fonts"),
            PathBuf::from("/Library/Fonts"),
            PathBuf::from("/usr/share/fonts"),
        ];
        for d in dirs {
            if let Ok(entries) = std::fs::read_dir(d) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                    if stem == clean {
                        if let Ok(b) = std::fs::read(&p) {
                            font_bytes = Some(b);
                            break;
                        }
                    }
                }
            }
            if font_bytes.is_some() {
                break;
            }
        }
    }

    if let Some(bytes) = font_bytes {
        let mut fonts = FontDefinitions::default();
        // 既存の標準フォントを復元
        fonts.font_data.insert("line_seed_jp".to_owned(), FontData::from_static(LINE_SEED_JP_REGULAR));
        for &path in FALLBACK_CANDIDATES {
            if let Ok(b) = std::fs::read(path) {
                fonts.font_data.insert("os_fallback".to_owned(), FontData::from_owned(b));
                break;
            }
        }
        let prop = fonts.families.entry(FontFamily::Proportional).or_default();
        prop.insert(0, "line_seed_jp".to_owned());
        if fonts.font_data.contains_key("os_fallback") {
            prop.push("os_fallback".to_owned());
        }

        // 新しいフォントを登録
        let font_key = format!("user_font_{}", font_name);
        fonts.font_data.insert(font_key.clone(), FontData::from_owned(bytes));
        fonts.families.insert(
            FontFamily::Name(font_name.to_string().into()),
            vec![font_key, "line_seed_jp".to_owned()],
        );

        ctx.set_fonts(fonts);
        let mut reg_lock = REGISTERED_FAMILIES.lock().unwrap();
        let registered = reg_lock.get_or_insert_with(HashSet::new);
        registered.insert(font_name.to_string());
        log::info!("動的フォントをロードしました: {font_name}");
        true
    } else {
        false
    }
}

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
    //    UI全体で統一された美しい LINE Seed JP のタイポグラフィを適用します
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

    #[test]
    fn test_list_available_fonts() {
        let list = list_available_fonts();
        assert!(!list.is_empty(), "フォント一覧が取得できること");
        assert!(list.iter().any(|f| f == "LINE Seed JP"));
    }
}
