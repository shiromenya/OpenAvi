//! # au2-aup
//!
//! AviUtl2 新プロジェクトファイル（`.aup2`）および初代 AviUtl プロジェクトファイル（`.aup`）の
//! 解析・シリアライズクレート。
//!
//! ## 1. `.aup2` (AviUtl ExEdit2 プロジェクト形式) の仕様と対応
//! `.aup2` は UTF-8 ベースの INI 風構造化テキストフォーマットです：
//! - `[project]`: `version=2001802`, `file=...`, `display.scene=0`
//! - `[scene.0]`: `scene=0`, `name=Root`, `video.width=1920`, `video.height=1080`, `video.rate=30`, `video.scale=1`, `audio.rate=48000`, `cursor.frame=0`, `display.zoom=10000` 等
//! - `[0]` / `[0.0]` / `[0.1]`:
//!   - オブジェクト範囲（`start=`, `end=` または `frame=start,end`）、`layer=`
//!   - エフェクト定義は旧 `.exo` の `_name=` に代わり **`effect.name=`** を使用
//!   - テキスト本文も UTF-16LE 16進ダンプではなく UTF-8 文字列（`テキスト=`）として直接記述
//!
//! ## 2. `.aup` (初代 AviUtl 0.18 バイナリプロジェクト形式) の仕様と対応
//! - 先頭 32 バイトの `"AviUtl ProjectFile version 0.18\0"` バイナリヘッダ解析・出力
//! - 拡張編集チャンクおよびレガシー `.aup` 内のメディアファイル参照抽出

use std::collections::BTreeMap;

use au2_core::{Frame, Rational, Scene, SceneSettings, TimelineObject};
use au2_exo::parse_exo_bytes;
use thiserror::Error;

/// 初代 AviUtl プロジェクトファイルの32バイト標準ヘッダ
pub const AUP_HEADER_018: &[u8; 32] = b"AviUtl ProjectFile version 0.18\0";
pub const AUP_HEADER_PREFIX: &[u8] = b"AviUtl ProjectFile version ";

pub mod legacy_aup;

/// AviUtl2 (ExEdit2) `.aup2` の既定バージョン番号
pub const AUP2_DEFAULT_VERSION: u32 = 2011400;

/// OpenAvi が `.aup` バイナリ内に格納する拡張編集チャンクマジック
const OAVI_CHUNK_MAGIC: &[u8; 16] = b"OAVI_EXEDIT_V1\0\0";

#[derive(Debug, Error)]
pub enum AupError {
    #[error("有効な AviUtl プロジェクトファイル (.aup / .aup2) ヘッダが見つかりません")]
    InvalidHeader,
    #[error(".aup 内の拡張編集データの解析に失敗しました: {0}")]
    ExoParse(#[from] au2_exo::ExoError),
    #[error("I/O エラー: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct AupDocument {
    pub version: String,
    pub scene: Scene,
    /// マルチシーン（Root = 0, Scene 1..N）のマップ
    pub scenes: BTreeMap<u32, Scene>,
    pub cursor_frame: Frame,
    pub warnings: Vec<String>,
}

mod aup2_reader;
mod aup2_writer;

pub use aup2_reader::{
    expand_aviutl2_path_vars, parse_aup2_bytes, parse_aup2_str, parse_object_alias_bytes,
    parse_object_alias_str,
};
pub use aup2_writer::{
    write_aup2_bytes, write_aup2_multi_scene_string, write_aup2_string, write_object_alias_bytes,
    write_object_alias_string,
};
pub(crate) use aup2_writer::blend_aup2_to_code;

// ============================================================================
// 初代 AviUtl (.aup) バイナリパーサー & ライター
// ============================================================================

/// シーンを本家 AviUtl 1.10 + 拡張編集 0.92 (`80EE`) 完全互換の `.aup` バイナリ形式でシリアライズします。
pub fn write_aup_bytes(scene: &Scene) -> Vec<u8> {
    legacy_aup::write_legacy_aup(scene)
}

/// マルチシーンプロジェクトを本家 AviUtl 1.10 + 拡張編集 0.92 (`80EE`) 完全互換の `.aup` バイナリ形式でシリアライズします。
pub fn write_aup_multi_scene_bytes(
    scenes: &BTreeMap<u32, Scene>,
    editing_scene: u32,
    cursor_frame: Frame,
) -> Vec<u8> {
    legacy_aup::write_legacy_aup_multi_scene(scenes, editing_scene, cursor_frame)
}

/// 初代 `.aup` バイナリまたは `.aup2` テキストを自動判別して解析します。
pub fn parse_aup_bytes(bytes: &[u8]) -> Result<AupDocument, AupError> {
    // もしテキスト形式 (.aup2) が渡された場合は自動的に .aup2 パーサーへ委譲
    if !bytes.starts_with(AUP_HEADER_PREFIX) {
        if let Ok(doc) = parse_aup2_bytes(bytes) {
            return Ok(doc);
        }
        return Err(AupError::InvalidHeader);
    }

    if bytes.len() < 32 {
        return Err(AupError::InvalidHeader);
    }

    let header_str = String::from_utf8_lossy(&bytes[..32]);
    let version = header_str
        .trim_end_matches('\0')
        .strip_prefix("AviUtl ProjectFile version ")
        .unwrap_or("unknown")
        .to_string();

    if let Some(pos) = find_subslice(bytes, OAVI_CHUNK_MAGIC) {
        let len_offset = pos + OAVI_CHUNK_MAGIC.len();
        if len_offset + 4 <= bytes.len() {
            let len = u32::from_le_bytes([
                bytes[len_offset],
                bytes[len_offset + 1],
                bytes[len_offset + 2],
                bytes[len_offset + 3],
            ]) as usize;
            let data_start = len_offset + 4;
            if data_start + len <= bytes.len() {
                let doc = parse_exo_bytes(&bytes[data_start..data_start + len])?;
                let mut scenes = BTreeMap::new();
                scenes.insert(0, doc.scene.clone());
                return Ok(AupDocument {
                    version,
                    scene: doc.scene,
                    scenes,
                    cursor_frame: 0,
                    warnings: doc.warnings,
                });
            }
        }
    }

    // 初代 AviUtl (.aup) の拡張編集バイナリおよび本体データを純粋 Rust で高精度パース
    if let Ok(doc) = legacy_aup::parse_legacy_aup(bytes) {
        if doc.scene.object_count() > 0
            || doc.scenes.values().any(|s| s.object_count() > 0)
            || doc.warnings.iter().any(|w| w.contains("拡張編集データ"))
        {
            return Ok(doc);
        }
    }

    let mut warnings = Vec::new();
    let settings = extract_legacy_settings(bytes, &mut warnings);
    let mut scene = Scene::new("Root", settings);

    let extracted_paths = extract_media_paths_from_binary(bytes);
    if extracted_paths.is_empty() {
        warnings.push(
            "レガシー .aup バイナリヘッダを読み込みました（詳細な拡張編集オブジェクトを移行する場合は .exo または .aup2 も併用できます）".into(),
        );
    } else {
        for (idx, path_str) in extracted_paths.into_iter().enumerate() {
            let layer = idx.min(99);
            let lower = path_str.to_ascii_lowercase();
            let obj = if lower.ends_with(".mp4")
                || lower.ends_with(".avi")
                || lower.ends_with(".mov")
                || lower.ends_with(".mkv")
                || lower.ends_with(".wmv")
            {
                TimelineObject::new_video(0, 149, path_str)
            } else if lower.ends_with(".png")
                || lower.ends_with(".jpg")
                || lower.ends_with(".jpeg")
                || lower.ends_with(".bmp")
                || lower.ends_with(".webp")
                || lower.ends_with(".gif")
            {
                TimelineObject::new_image(0, 149, path_str)
            } else {
                TimelineObject::new_audio(0, 149, path_str)
            };
            scene.insert_object(layer, obj);
        }
    }

    let mut scenes = BTreeMap::new();
    scenes.insert(0, scene.clone());

    Ok(AupDocument {
        version,
        scene,
        scenes,
        cursor_frame: 0,
        warnings,
    })
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn extract_legacy_settings(bytes: &[u8], _warnings: &mut Vec<String>) -> SceneSettings {
    let mut s = SceneSettings::default();
    if bytes.len() >= 64 {
        let w = u32::from_le_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]);
        let h = u32::from_le_bytes([bytes[36], bytes[37], bytes[38], bytes[39]]);
        let rate = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
        let scale = u32::from_le_bytes([bytes[44], bytes[45], bytes[46], bytes[47]]);
        let len = u32::from_le_bytes([bytes[48], bytes[49], bytes[50], bytes[51]]);
        if (64..=8192).contains(&w) && (64..=8192).contains(&h) {
            s.width = w;
            s.height = h;
        }
        if (1..=240_000).contains(&rate) && (1..=1001).contains(&scale) {
            s.frame_rate = Rational::new(rate, scale);
        }
        if (1..=10_000_000).contains(&len) {
            s.length = len;
        }
    }
    s
}

pub fn extract_media_paths_from_binary(bytes: &[u8]) -> Vec<String> {
    let mut results = Vec::new();
    let exts = [
        ".mp4", ".avi", ".mov", ".mkv", ".wmv", ".png", ".jpg", ".jpeg", ".bmp", ".webp", ".gif",
        ".wav", ".mp3", ".ogg", ".flac", ".m4a", ".aac", ".wma",
    ];
    let mut i = 0;
    while i + 6 < bytes.len() {
        let is_abs = bytes[i].is_ascii_alphabetic() && bytes[i + 1] == b':' && (bytes[i + 2] == b'\\' || bytes[i + 2] == b'/');
        let is_special = (bytes[i] == 0 || bytes[i] == b'@') && bytes[i + 1] == b':' && (bytes[i + 2] == b'\\' || bytes[i + 2] == b'/');
        if is_abs || is_special {
            let start = i + 1;
            let end = bytes[start..]
                .iter()
                .position(|&b| b == 0 || b < 0x20)
                .map(|p| start + p)
                .unwrap_or(bytes.len())
                .min(start + 260);
            let slice = &bytes[start..end];
            let (decoded, _, _) = encoding_rs::SHIFT_JIS.decode(slice);
            let mut candidate = decoded.trim().to_string();
            if is_abs {
                candidate = format!("{}{}", bytes[i] as char, candidate);
            } else {
                candidate = format!("C{}", candidate);
            }
            let lower = candidate.to_ascii_lowercase();
            // 拡張子が含まれている位置でパスを切り詰める（ゴミバイナリや後続ヘッダを排除）
            let mut matched_ext = None;
            for ext in &exts {
                if let Some(pos) = lower.find(ext) {
                    matched_ext = Some(pos + ext.len());
                    break;
                }
            }
            if let Some(end_len) = matched_ext {
                candidate.truncate(end_len);
                if !results.contains(&candidate) {
                    results.push(candidate);
                }
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }
    results
}

#[cfg(test)]
mod tests;