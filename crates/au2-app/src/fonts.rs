//! # fonts
//!
//! システムフォントの設定と動的ロード管理。
//! 美しく視認性に優れた「LINE Seed JP」を最優先の標準システムフォントとして登録し、
//! OS のフォント（TTF / OTF / TTC）を初回キャッシュスキャンして
//! オブジェクトごとのフォント切り替え（メイリオ、游ゴシック、ＭＳ ゴシック等）に対応します。

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

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

#[derive(Clone, Debug)]
struct FontFaceRef {
    path: PathBuf,
    index: u32,
}

struct FontCatalog {
    available_names: Vec<String>,
    entries_by_key: HashMap<String, FontFaceRef>,
}

fn font_state_id() -> egui::Id {
    egui::Id::new("openavi_context_font_state")
}

/// `egui::Context` ごとに保持する累積フォント状態（複数フォントの併用・別コンテキスト間の干渉防止）。
#[derive(Clone)]
struct ContextFontState {
    definitions: FontDefinitions,
    registered: HashSet<String>,
    attempted: HashSet<String>,
}

static FONT_CATALOG: OnceLock<FontCatalog> = OnceLock::new();
static FONT_BYTES_CACHE: OnceLock<Mutex<HashMap<PathBuf, &'static [u8]>>> = OnceLock::new();
static OS_FALLBACK_BYTES: OnceLock<Option<&'static [u8]>> = OnceLock::new();

/// フォント名を検索用に正規化します（全角英数を半角化、小文字化、空白と区切りを除去）。
fn normalize_font_key(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        let mapped = match ch {
            'Ａ'..='Ｚ' => char::from_u32(ch as u32 - 'Ａ' as u32 + 'A' as u32).unwrap_or(ch),
            'ａ'..='ｚ' => char::from_u32(ch as u32 - 'ａ' as u32 + 'a' as u32).unwrap_or(ch),
            '０'..='９' => char::from_u32(ch as u32 - '０' as u32 + '0' as u32).unwrap_or(ch),
            '　' | ' ' | '\t' | '\r' | '\n' => continue,
            _ => ch,
        };
        for lc in mapped.to_lowercase() {
            out.push(lc);
        }
    }
    out
}

fn read_u16_be_slice(b: &[u8], off: usize) -> Option<u16> {
    let s = b.get(off..off + 2)?;
    Some(u16::from_be_bytes([s[0], s[1]]))
}

fn read_u32_be_slice(b: &[u8], off: usize) -> Option<u32> {
    let s = b.get(off..off + 4)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_exact_at(file: &mut File, offset: u64, len: usize) -> Option<Vec<u8>> {
    if len == 0 || len > 16 * 1024 * 1024 {
        return None;
    }
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn is_valid_sfnt_version(tag: &[u8]) -> bool {
    matches!(tag, b"\x00\x01\x00\x00" | b"OTTO" | b"true" | b"typ1")
}

/// SFNT テーブルから `unitsPerEm` 妥当性とアウトラインテーブル存在を検証し、`name` テーブルから名前一覧を抽出します。
fn inspect_sfnt_face<F>(read_at: &mut F, face_offset: u64) -> Option<Vec<String>>
where
    F: FnMut(u64, usize) -> Option<Vec<u8>>,
{
    let hdr = read_at(face_offset, 12)?;
    if !is_valid_sfnt_version(&hdr[0..4]) {
        return None;
    }
    let num_tables = read_u16_be_slice(&hdr, 4)? as usize;
    if num_tables == 0 || num_tables > 128 {
        return None;
    }

    let dir = read_at(face_offset + 12, num_tables * 16)?;
    let mut head_info: Option<(u64, u32)> = None;
    let mut name_info: Option<(u64, u32)> = None;
    let mut has_outline = false;
    let mut has_cmap = false;
    let mut has_hhea = false;
    let mut has_maxp = false;

    for i in 0..num_tables {
        let base = i * 16;
        let tag = &dir[base..base + 4];
        let t_off = read_u32_be_slice(&dir, base + 8)? as u64;
        let t_len = read_u32_be_slice(&dir, base + 12)?;
        match tag {
            b"head" => head_info = Some((t_off, t_len)),
            b"name" => name_info = Some((t_off, t_len)),
            b"glyf" | b"CFF " | b"CFF2" => has_outline = true,
            b"cmap" => has_cmap = true,
            b"hhea" => has_hhea = true,
            b"maxp" => has_maxp = true,
            _ => {}
        }
    }

    if !has_outline || !has_cmap || !has_hhea || !has_maxp {
        return None;
    }

    let (head_off, head_len) = head_info?;
    if head_len < 54 {
        return None;
    }
    let head_buf = read_at(head_off, 54)?;
    let units_per_em = read_u16_be_slice(&head_buf, 18)?;
    if !(16..=16384).contains(&units_per_em) {
        return None;
    }

    let mut names = Vec::new();
    if let Some((name_off, name_len)) = name_info {
        if let Some(name_buf) = read_at(name_off, (name_len as usize).min(2 * 1024 * 1024)) {
            if name_buf.len() >= 6 {
                let count = read_u16_be_slice(&name_buf, 2).unwrap_or(0) as usize;
                let str_storage_off = read_u16_be_slice(&name_buf, 4).unwrap_or(0) as usize;
                for i in 0..count.min(1024) {
                    let rec_off = 6 + i * 12;
                    if rec_off + 12 > name_buf.len() {
                        break;
                    }
                    let platform_id = read_u16_be_slice(&name_buf, rec_off).unwrap_or(0);
                    let encoding_id = read_u16_be_slice(&name_buf, rec_off + 2).unwrap_or(0);
                    let name_id = read_u16_be_slice(&name_buf, rec_off + 6).unwrap_or(0);
                    if !matches!(name_id, 1 | 4 | 16) {
                        continue;
                    }
                    let len = read_u16_be_slice(&name_buf, rec_off + 8).unwrap_or(0) as usize;
                    let off = read_u16_be_slice(&name_buf, rec_off + 10).unwrap_or(0) as usize;
                    let abs_start = str_storage_off + off;
                    let Some(raw) = name_buf.get(abs_start..abs_start + len) else {
                        continue;
                    };
                    let decoded = if platform_id == 0 || (platform_id == 3 && (encoding_id == 1 || encoding_id == 10)) {
                        let units: Vec<u16> = raw
                            .chunks_exact(2)
                            .map(|c| u16::from_be_bytes([c[0], c[1]]))
                            .collect();
                        String::from_utf16_lossy(&units)
                    } else if platform_id == 1 && encoding_id == 0 {
                        raw.iter()
                            .filter(|&&b| (0x20..=0x7E).contains(&b))
                            .map(|&b| b as char)
                            .collect()
                    } else {
                        continue;
                    };
                    let trimmed = decoded.trim_matches('\0').trim().to_string();
                    if !trimmed.is_empty() && !names.contains(&trimmed) {
                        names.push(trimmed);
                    }
                }
            }
        }
    }

    Some(names)
}

/// 指定パスのフォントファイルから利用可能な全フェイス `(index, 名前一覧)` をヘッダのみシーク読みして抽出します。
fn inspect_font_file_faces(path: &Path) -> Vec<(u32, Vec<String>)> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let mut reader = |off: u64, len: usize| read_exact_at(&mut file, off, len);
    let Some(head) = reader(0, 12) else {
        return Vec::new();
    };

    if &head[0..4] == b"ttcf" {
        let num_fonts = read_u32_be_slice(&head, 8).unwrap_or(0).min(64);
        let Some(off_buf) = reader(12, num_fonts as usize * 4) else {
            return Vec::new();
        };
        let mut faces = Vec::new();
        for idx in 0..num_fonts {
            if let Some(face_off) = read_u32_be_slice(&off_buf, idx as usize * 4) {
                if let Some(names) = inspect_sfnt_face(&mut reader, face_off as u64) {
                    faces.push((idx, names));
                }
            }
        }
        faces
    } else if let Some(names) = inspect_sfnt_face(&mut reader, 0) {
        vec![(0, names)]
    } else {
        Vec::new()
    }
}

/// メモリ上のフォントデータと指定インデックスが `epaint` / `ab_glyph` で安全に読み込めるかを事前検証します。
pub fn validate_font_face_bytes(bytes: &[u8], face_index: u32) -> bool {
    let mut reader = |off: u64, len: usize| -> Option<Vec<u8>> {
        let start = usize::try_from(off).ok()?;
        bytes.get(start..start + len).map(|s| s.to_vec())
    };
    let Some(head) = reader(0, 12) else {
        return false;
    };
    if &head[0..4] == b"ttcf" {
        let num_fonts = read_u32_be_slice(&head, 8).unwrap_or(0);
        if face_index >= num_fonts {
            return false;
        }
        let Some(off_buf) = reader(12 + face_index as u64 * 4, 4) else {
            return false;
        };
        let Some(face_off) = read_u32_be_slice(&off_buf, 0) else {
            return false;
        };
        inspect_sfnt_face(&mut reader, face_off as u64).is_some()
    } else {
        face_index == 0 && inspect_sfnt_face(&mut reader, 0).is_some()
    }
}

fn scan_font_dir_recursive(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    if depth == 0 || !dir.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            scan_font_dir_recursive(&p, depth - 1, out);
        } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            let ext_lc = ext.to_ascii_lowercase();
            if matches!(ext_lc.as_str(), "ttf" | "otf" | "ttc") {
                out.push(p);
            }
        }
    }
}

/// ファイル内のフェイス候補から、`target_name` に最も一致するフェイスインデックスを選択します。
fn pick_best_face_index(faces: &[(u32, Vec<String>)], target_name: &str) -> u32 {
    if faces.is_empty() {
        return 0;
    }
    let target_key = normalize_font_key(target_name);
    // 完全正規化一致を最優先
    for (idx, names) in faces {
        if names.iter().any(|n| normalize_font_key(n) == target_key) {
            return *idx;
        }
    }
    // サフィックス/サブストリング一致（例: "Yu Gothic UI" / "MS Pゴシック"）
    for (idx, names) in faces {
        if names.iter().any(|n| normalize_font_key(n).contains(&target_key)) {
            return *idx;
        }
    }
    faces[0].0
}

fn build_font_catalog() -> FontCatalog {
    let mut entries_by_key: HashMap<String, FontFaceRef> = HashMap::new();
    let mut available_names: Vec<String> = Vec::new();

    // 1. OS 標準フォントディレクトリを起動時に1回だけ高速ヘッダスキャン
    let mut search_dirs = vec![
        PathBuf::from(r"C:\Windows\Fonts"),
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/Library/Fonts"),
        PathBuf::from("/usr/share/fonts"),
    ];
    if let Some(home) = au2_platform::home_dir() {
        search_dirs.push(home.join("Library/Fonts"));
        search_dirs.push(home.join(".local/share/fonts"));
        search_dirs.push(home.join(".fonts"));
    }
    if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
        search_dirs.push(PathBuf::from(local_app).join(r"Microsoft\Windows\Fonts"));
    }

    let mut font_files = Vec::new();
    for dir in &search_dirs {
        scan_font_dir_recursive(dir, 3, &mut font_files);
    }

    for path in font_files {
        let faces = inspect_font_file_faces(&path);
        if faces.is_empty() {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            let key = normalize_font_key(stem);
            if !key.is_empty() {
                entries_by_key.entry(key).or_insert_with(|| FontFaceRef {
                    path: path.clone(),
                    index: faces[0].0,
                });
            }
        }
        for (idx, names) in faces {
            for n in names {
                let key = normalize_font_key(&n);
                if !key.is_empty() {
                    entries_by_key.entry(key).or_insert_with(|| FontFaceRef {
                        path: path.clone(),
                        index: idx,
                    });
                }
            }
        }
    }

    // 2. 代表フォント候補テーブルを検証・優先登録
    for &(name, files) in FONT_NAME_TO_FILES {
        if files.is_empty() {
            available_names.push(name.to_string());
            continue;
        }
        let mut matched_ref: Option<FontFaceRef> = None;
        for &f in files {
            let p = Path::new(f);
            if p.exists() {
                let faces = inspect_font_file_faces(p);
                if !faces.is_empty() {
                    let best_idx = pick_best_face_index(&faces, name);
                    matched_ref = Some(FontFaceRef {
                        path: p.to_path_buf(),
                        index: best_idx,
                    });
                    break;
                }
            }
        }
        if let Some(fref) = matched_ref {
            entries_by_key.insert(normalize_font_key(name), fref);
            available_names.push(name.to_string());
        }
    }

    // 英語・日本語の代表的な別名エイリアスを補完
    let aliases = [
        ("ＭＳ ゴシック", "MS ゴシック"),
        ("ＭＳ Ｐゴシック", "MS Pゴシック"),
        ("ＭＳ 明朝", "MS 明朝"),
        ("ＭＳ Ｐ明朝", "MS P明朝"),
        ("Meiryo", "メイリオ"),
        ("Yu Gothic", "游ゴシック"),
        ("Yu Gothic Medium", "游ゴシック"),
    ];
    for (alias, canonical) in aliases {
        let a_key = normalize_font_key(alias);
        let c_key = normalize_font_key(canonical);
        if !entries_by_key.contains_key(&a_key) {
            if let Some(found) = entries_by_key.get(&c_key).cloned() {
                entries_by_key.insert(a_key, found);
            }
        }
    }

    FontCatalog {
        available_names,
        entries_by_key,
    }
}

fn catalog() -> &'static FontCatalog {
    FONT_CATALOG.get_or_init(build_font_catalog)
}

fn load_cached_font_bytes(path: &Path) -> Option<&'static [u8]> {
    let cache = FONT_BYTES_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let guard = cache.lock().ok()?;
        if let Some(&slice) = guard.get(path) {
            return Some(slice);
        }
    }
    let bytes = std::fs::read(path).ok()?;
    let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    let mut guard = cache.lock().ok()?;
    let entry = guard.entry(path.to_path_buf()).or_insert(leaked);
    Some(*entry)
}

fn os_fallback_font_bytes() -> Option<&'static [u8]> {
    *OS_FALLBACK_BYTES.get_or_init(|| {
        for &path_str in FALLBACK_CANDIDATES {
            let p = Path::new(path_str);
            if p.exists() {
                if let Some(bytes) = load_cached_font_bytes(p) {
                    if validate_font_face_bytes(bytes, 0) {
                        return Some(bytes);
                    }
                }
            }
        }
        None
    })
}

fn base_font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "line_seed_jp".to_owned(),
        FontData::from_static(LINE_SEED_JP_REGULAR),
    );
    if let Some(fb_bytes) = os_fallback_font_bytes() {
        fonts.font_data.insert("os_fallback".to_owned(), FontData::from_static(fb_bytes));
    }

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
    fonts
}

/// 利用可能なフォント候補一覧を取得します（初回のみ探索し、以降はキャッシュを返します）。
pub fn list_available_fonts() -> &'static [String] {
    &catalog().available_names
}

/// 指定したフォントファミリが現在のフレームの `egui::Context` に既にバインド完了しているか確認します。
pub fn is_font_family_bound_now(ctx: &egui::Context, font_name: &str) -> bool {
    let fam = FontFamily::Name(font_name.to_string().into());
    ctx.fonts(|f| f.lock().fonts.definitions().families.contains_key(&fam))
}

/// 指定したフォント名に対応するフォントを、該当 `egui::Context` の累積フォント定義へ安全に追加・登録します。
/// 新しくフォント登録が発生して `ctx.set_fonts(...)` が呼ばれた場合は `true` を返します。
pub fn ensure_font_family(ctx: &egui::Context, font_name: &str) -> bool {
    let trimmed = font_name.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("LINE Seed JP") {
        return false;
    }

    let key = normalize_font_key(trimmed);
    let already_attempted = ctx.data(|d| {
        d.get_temp::<ContextFontState>(font_state_id())
            .is_some_and(|st| st.attempted.contains(&key) || st.registered.contains(trimmed))
    });
    if already_attempted {
        return false;
    }

    let found_ref = catalog().entries_by_key.get(&key).cloned();
    let loaded_face = found_ref.and_then(|fref| {
        let bytes = load_cached_font_bytes(&fref.path)?;
        if !validate_font_face_bytes(bytes, fref.index) {
            log::warn!("フォントファイルの事前検証に失敗しました: {} (index={})", fref.path.display(), fref.index);
            return None;
        }
        Some((fref, bytes))
    });

    if let Some((fref, bytes)) = loaded_face {
        let new_defs = ctx.data_mut(|d| {
            let state = d
                .get_temp_mut_or_insert_with(font_state_id(), || ContextFontState {
                    definitions: base_font_definitions(),
                    registered: HashSet::new(),
                    attempted: HashSet::new(),
                });
            state.attempted.insert(key.clone());

            let path_stem = fref
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(normalize_font_key)
                .unwrap_or_else(|| "font".to_string());
            let data_key = format!("user_font_{}_{}", path_stem, fref.index);

            let mut fdata = FontData::from_static(bytes);
            fdata.index = fref.index;
            state.definitions.font_data.entry(data_key.clone()).or_insert(fdata);

            let mut chain = vec![data_key, "line_seed_jp".to_owned()];
            if state.definitions.font_data.contains_key("os_fallback") {
                chain.push("os_fallback".to_owned());
            }
            state
                .definitions
                .families
                .insert(FontFamily::Name(trimmed.to_string().into()), chain);
            state.registered.insert(trimmed.to_string());
            state.definitions.clone()
        });

        ctx.set_fonts(new_defs);
        log::info!("動的フォントをロードしました: {trimmed}");
        true
    } else {
        ctx.data_mut(|d| {
            let state = d
                .get_temp_mut_or_insert_with(font_state_id(), || ContextFontState {
                    definitions: base_font_definitions(),
                    registered: HashSet::new(),
                    attempted: HashSet::new(),
                });
            state.attempted.insert(key);
        });
        false
    }
}

/// 指定したフォント名が即座に使用可能な場合はその `FontFamily::Name` を返し、
/// 未ロード・未バインド・未インストールの場合は安全に `FontFamily::Proportional` へフォールバックします。
/// 新規ロードが発生した場合は再描画を要求します。
pub fn resolve_bound_font_family(ctx: &egui::Context, font_name: Option<&str>) -> FontFamily {
    let Some(name) = font_name.map(|s| s.trim()).filter(|s| !s.is_empty()) else {
        return FontFamily::Proportional;
    };
    if name.eq_ignore_ascii_case("LINE Seed JP") {
        return FontFamily::Proportional;
    }

    if ensure_font_family(ctx, name) {
        ctx.request_repaint();
    }

    if is_font_family_bound_now(ctx, name) {
        FontFamily::Name(name.to_string().into())
    } else {
        FontFamily::Proportional
    }
}

/// 書き出し時などに、シーンと全サブシーン内のテキストオブジェクトで使用される全フォントを事前に登録します。
/// 1つ以上のフォントが新規登録された場合は `true` を返します（呼び出し元で空フレームを実行しアトラス確定するため）。
pub fn prepare_fonts_for_scenes(
    ctx: &egui::Context,
    main_scene: &au2_core::Scene,
    sub_scenes: &std::collections::BTreeMap<u32, au2_core::Scene>,
) -> bool {
    let mut any_added = false;
    let mut process_scene = |scene: &au2_core::Scene| {
        for layer in &scene.layers {
            for obj in &layer.objects {
                for eff in &obj.effects {
                    if eff.kind == au2_core::EffectKind::Text {
                        if let Some(name) = eff.text("font").map(|f| f.trim()).filter(|f| !f.is_empty()) {
                            if ensure_font_family(ctx, name) {
                                any_added = true;
                            }
                        }
                    }
                }
            }
        }
    };

    process_scene(main_scene);
    for sub in sub_scenes.values() {
        process_scene(sub);
    }
    any_added
}

/// LINE Seed JP を最優先システムフォントとして登録します。
pub fn install_japanese_font(ctx: &egui::Context) -> Option<&'static str> {
    let fonts = base_font_definitions();
    ctx.data_mut(|d| {
        d.insert_temp(
            font_state_id(),
            ContextFontState {
                definitions: fonts.clone(),
                registered: HashSet::new(),
                attempted: HashSet::new(),
            },
        );
    });
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

    #[test]
    fn test_multiple_fonts_and_context_isolation() {
        let ctx1 = egui::Context::default();
        let ctx2 = egui::Context::default();
        install_japanese_font(&ctx1);
        install_japanese_font(&ctx2);

        let available = list_available_fonts();
        let candidates: Vec<&str> = available
            .iter()
            .map(|s| s.as_str())
            .filter(|&n| n != "LINE Seed JP")
            .take(2)
            .collect();

        if candidates.len() >= 2 {
            // ctx1 に 2つの異なるフォントを登録
            assert!(ensure_font_family(&ctx1, candidates[0]));
            assert!(ensure_font_family(&ctx1, candidates[1]));

            // 1つ目も2つ目も Monospace も全て ctx1 に保持されていること（上書き消失しないこと）
            let defs1 = ctx1.data(|d| {
                d.get_temp::<ContextFontState>(font_state_id())
                    .map(|s| s.definitions)
                    .unwrap()
            });
            assert!(defs1.families.contains_key(&FontFamily::Name(candidates[0].into())));
            assert!(defs1.families.contains_key(&FontFamily::Name(candidates[1].into())));
            assert!(defs1.families.contains_key(&FontFamily::Monospace));
            assert!(defs1.families.contains_key(&FontFamily::Proportional));

            // ctx2 は別コンテキストなので、ctx1 で登録済みであっても独立して新規登録できること
            assert!(ensure_font_family(&ctx2, candidates[0]));
        }
    }

    #[test]
    fn test_corrupted_font_validation_and_alias_normalization() {
        // 壊れたバイト列が安全に拒否されること
        assert!(!validate_font_face_bytes(b"", 0));
        assert!(!validate_font_face_bytes(b"not a valid font file", 0));

        // 全角・空白の違いが正規化で一致すること
        assert_eq!(normalize_font_key("ＭＳ Ｐゴシック"), normalize_font_key("MS Pゴシック"));
        assert_eq!(normalize_font_key("Yu Gothic"), normalize_font_key("yugothic"));

        // Windows で msgothic.ttc がある場合、サブフォントインデックスが正しく分かれていること
        let cat = catalog();
        if let (Some(gothic), Some(pgothic), Some(ui)) = (
            cat.entries_by_key.get(&normalize_font_key("MS ゴシック")),
            cat.entries_by_key.get(&normalize_font_key("MS Pゴシック")),
            cat.entries_by_key.get(&normalize_font_key("MS UI Gothic")),
        ) {
            assert_ne!(gothic.index, pgothic.index, "MS ゴシックと MS Pゴシックで index が異なること");
            assert_ne!(pgothic.index, ui.index, "MS Pゴシックと MS UI Gothic で index が異なること");
        }
    }
}
