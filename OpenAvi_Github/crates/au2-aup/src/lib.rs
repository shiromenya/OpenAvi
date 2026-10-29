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
use std::fmt::Write as _;
use std::path::Path;

use au2_core::{
    Easing, Effect, EffectKind, Frame, Interpolation, Keyframe, MoveKind, Param, ParamValue,
    Rational, Scene, SceneSettings, TimelineObject, Track,
};
use au2_exo::{parse_exo_bytes, write_exo_bytes, TextEncoding};
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

// ============================================================================
// AviUtl2 (.aup2) パーサー & ライター
// ============================================================================

/// シーンを本家 AviUtl2 の `.aup2` (UTF-8 INI 形式) テキストにシリアライズします。
pub fn write_aup2_string(scene: &Scene, file_path: Option<&Path>, cursor_frame: Frame) -> String {
    let mut out = String::new();
    let file_str = file_path
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let _ = write!(
        out,
        "[project]\r\nversion={AUP2_DEFAULT_VERSION}\r\nfile={file_str}\r\ndisplay.scene=0\r\npreview.scene=0\r\n"
    );

    let s = &scene.settings;
    let fps = s.frame_rate.as_f64().max(1.0);
    let _ = write!(
        out,
        "[scene.0]\r\nscene=0\r\nname={}\r\nvideo.width={}\r\nvideo.height={}\r\nvideo.rate={}\r\nvideo.scale={}\r\naudio.rate={}\r\ncursor.frame={cursor_frame}\r\ncursor.layer=0\r\npreview.frame=0\r\ndisplay.frame=0\r\ndisplay.layer=0\r\ndisplay.zoom=10000\r\ndisplay.order=0\r\ndisplay.camera=\r\ndisplay.grid.x=16,-16\r\ndisplay.grid.y=16,-16\r\ndisplay.grid.width=200\r\ndisplay.grid.height=200\r\ndisplay.grid.step=200.000000\r\ndisplay.grid.range=10000.000000\r\ndisplay.tempo.bpm=120.000000\r\ndisplay.tempo.beat=4\r\ndisplay.tempo.offset=0.000000\r\n",
        scene.name,
        s.width,
        s.height,
        s.frame_rate.num,
        s.frame_rate.den,
        s.audio_rate,
    );

    let mut objs: Vec<(usize, &TimelineObject)> = scene
        .layers
        .iter()
        .enumerate()
        .flat_map(|(li, l)| l.objects.iter().map(move |o| (li, o)))
        .collect();
    objs.sort_by_key(|(li, o)| (*li, o.frame_start));

    for (index, (layer, obj)) in objs.into_iter().enumerate() {
        let keys = obj.key_frames();
        // 本家 AviUtl2 (.aup2) では:
        // - layer= は 0 始まり (Layer 1 -> 0)
        // - frame= は start,mid1,...,end のカンマ区切り 0 始まり絶対フレーム列
        let frame_list: String = if obj.midpoints.is_empty() {
            format!("{},{}", obj.frame_start, obj.frame_end)
        } else {
            let mut frames = Vec::with_capacity(obj.midpoints.len() + 2);
            frames.push(obj.frame_start.to_string());
            for &m in &obj.midpoints {
                frames.push((obj.frame_start + m).to_string());
            }
            frames.push(obj.frame_end.to_string());
            frames.join(",")
        };

        let _ = write!(out, "[{index}]\r\nlayer={layer}\r\nframe={frame_list}\r\n");
        if obj.clipping {
            out.push_str("clipping=1\r\n");
        }
        if obj.camera_target {
            out.push_str("camera=1\r\n");
        }

        let source_kind = obj.source().map(|e| e.kind.clone());
        let obj_dur_sec = (obj.length() as f64) / fps;

        for (ei, eff) in obj.effects.iter().enumerate() {
            write_aup2_effect(
                &mut out,
                index,
                ei,
                eff,
                &source_kind,
                &keys,
                obj_dur_sec,
            );
        }
    }

    out
}

/// シーンを `.aup2` (UTF-8 バイト列) に変換します。
pub fn write_aup2_bytes(scene: &Scene, file_path: Option<&Path>, cursor_frame: Frame) -> Vec<u8> {
    write_aup2_string(scene, file_path, cursor_frame).into_bytes()
}

fn write_aup2_effect(
    out: &mut String,
    obj_idx: usize,
    eff_idx: usize,
    eff: &Effect,
    source_kind: &Option<EffectKind>,
    keys: &[Frame],
    obj_dur_sec: f64,
) {
    // アニメーション効果 (@揺れ 等) や動画/音声の再生エフェクト名を本家 AviUtl2 標準名に変換
    if eff.name == "アニメーション効果" {
        let raw_name = eff.text("name").unwrap_or("震える");
        let base_name = raw_name.split('@').next().unwrap_or(raw_name);
        let aup2_eff_name = match base_name {
            "揺れ" | "震える" => "震える",
            "点滅" => "点滅",
            "回転" => "座標の回転(個別オブジェクト)",
            other => other,
        };
        let _ = write!(out, "[{obj_idx}.{eff_idx}]\r\neffect.name={aup2_eff_name}\r\n");
        if !eff.enabled {
            out.push_str("effect.disable=1\r\n");
        }
        for p in &eff.params {
            if matches!(p.key.as_str(), "name" | "filter" | "param") {
                continue;
            }
            let out_key = match (aup2_eff_name, p.key.as_str()) {
                ("震える", "track0") => "振幅",
                ("震える", "track1") => "角度",
                ("震える", "track2") => "間隔",
                ("点滅", "track0") => "速さ",
                ("点滅", "track1") => "点滅割合",
                ("点滅", "track2") => "点灯速度",
                ("点滅", "track3") => "消灯速度",
                ("座標の回転(個別オブジェクト)", "track0") => "角度",
                ("座標の回転(個別オブジェクト)", "track1") => "中心X",
                ("座標の回転(個別オブジェクト)", "track2") => "中心Y",
                (_, k) => k,
            };
            if let ParamValue::Track(t) = &p.value {
                let _ = write!(out, "{out_key}={}\r\n", format_aup2_track(t, keys));
            }
        }
        return;
    }

    let eff_name = match (&eff.kind, source_kind) {
        (EffectKind::StandardDraw | EffectKind::ExtendedDraw, Some(EffectKind::VideoFile)) => {
            "映像再生"
        }
        (EffectKind::ExtendedDraw, _) => "標準描画",
        (EffectKind::StandardPlayback, _) => "音声再生",
        _ if eff.name == "シャドー" => "ドロップシャドウ",
        _ => eff.name.as_str(),
    };

    let _ = write!(out, "[{obj_idx}.{eff_idx}]\r\neffect.name={eff_name}\r\n");
    if !eff.enabled {
        out.push_str("effect.disable=1\r\n");
    }

    match eff.kind {
        EffectKind::Shape => {
            let mut has_round = false;
            for p in &eff.params {
                match p.key.as_str() {
                    "name" => continue,
                    "type" | "図形の種類" => {
                        let raw = match &p.value {
                            ParamValue::Text(s) => s.as_str(),
                            _ => "1",
                        };
                        let shape_name = shape_code_to_aup2_name(raw);
                        let _ = write!(out, "図形の種類={shape_name}\r\n");
                    }
                    "color" | "色" => {
                        if let ParamValue::Text(c) = &p.value {
                            let _ = write!(out, "色={c}\r\n");
                        }
                    }
                    "角を丸くする" => {
                        has_round = true;
                        let val = format_param_value(&p.value, keys);
                        let _ = write!(out, "角を丸くする={val}\r\n");
                    }
                    other => {
                        let val = format_param_value(&p.value, keys);
                        let _ = write!(out, "{other}={val}\r\n");
                    }
                }
            }
            if !has_round {
                out.push_str("角を丸くする=0\r\n");
            }
        }
        EffectKind::Text => {
            for p in &eff.params {
                match p.key.as_str() {
                    // 旧 .exo 専用の互換用フラグは .aup2 では存在しないため除外
                    "soft" | "monospace" | "precision" => continue,
                    "サイズ" | "テキストサイズ" => {
                        let val = format_param_value(&p.value, keys);
                        let _ = write!(out, "サイズ={val}\r\n");
                    }
                    "text" | "テキスト" => {
                        if let ParamValue::Text(t) = &p.value {
                            let _ = write!(out, "テキスト={}\r\n", escape_aup2_text(t));
                        }
                    }
                    "font" | "フォント" => {
                        if let ParamValue::Text(f) = &p.value {
                            let _ = write!(out, "フォント={f}\r\n");
                        }
                    }
                    "color" | "文字色" => {
                        if let ParamValue::Text(c) = &p.value {
                            let _ = write!(out, "文字色={c}\r\n");
                        }
                    }
                    "color2" | "影・縁色" => {
                        if let ParamValue::Text(c) = &p.value {
                            let _ = write!(out, "影・縁色={c}\r\n");
                        }
                    }
                    "type" | "文字装飾" => {
                        let raw = match &p.value {
                            ParamValue::Text(s) => s.as_str(),
                            _ => "0",
                        };
                        let deco = text_deco_code_to_aup2(raw);
                        let _ = write!(out, "文字装飾={deco}\r\n");
                    }
                    "align" | "文字揃え" => {
                        let raw = match &p.value {
                            ParamValue::Text(s) => s.as_str(),
                            _ => "0",
                        };
                        let align = text_align_code_to_aup2(raw);
                        let _ = write!(out, "文字揃え={align}\r\n");
                    }
                    "spacing_x" | "字間" => {
                        let val = format_param_value(&p.value, keys);
                        let _ = write!(out, "字間={val}\r\n");
                    }
                    "spacing_y" | "行間" => {
                        let val = format_param_value(&p.value, keys);
                        let _ = write!(out, "行間={val}\r\n");
                    }
                    "autoadjust" | "オブジェクトの長さを自動調節" => {
                        let val = format_param_value(&p.value, keys);
                        let _ = write!(out, "オブジェクトの長さを自動調節={val}\r\n");
                    }
                    "移動座標上に表示する" | "移動座標上に表示" => {
                        let val = format_param_value(&p.value, keys);
                        let _ = write!(out, "移動座標上に表示={val}\r\n");
                    }
                    other => {
                        let val = format_param_value(&p.value, keys);
                        let _ = write!(out, "{other}={val}\r\n");
                    }
                }
            }
        }
        EffectKind::VideoFile | EffectKind::AudioFile => {
            let mut has_pos = false;
            for p in &eff.params {
                match p.key.as_str() {
                    "アルファチャンネルを読み込む" | "動画ファイルと連携" => continue,
                    "file" | "ファイル" => {
                        if let ParamValue::Text(f) = &p.value {
                            let _ = write!(out, "ファイル={f}\r\n");
                        }
                    }
                    "再生位置" => {
                        has_pos = true;
                        let start_sec = match &p.value {
                            ParamValue::Track(t) => {
                                let v = t.start_value();
                                if eff.kind == EffectKind::VideoFile && v >= 1.0 {
                                    (v - 1.0) / 30.0
                                } else {
                                    v.max(0.0)
                                }
                            }
                            _ => 0.0,
                        };
                        let end_sec = start_sec + obj_dur_sec;
                        let _ = write!(
                            out,
                            "再生位置={start_sec:.3},{end_sec:.3},再生範囲,0\r\n"
                        );
                    }
                    other => {
                        let val = format_param_value(&p.value, keys);
                        let _ = write!(out, "{other}={val}\r\n");
                    }
                }
            }
            if !has_pos {
                let _ = write!(
                    out,
                    "再生位置=0.000,{obj_dur_sec:.3},再生範囲,0\r\n"
                );
            }
        }
        EffectKind::ImageFile => {
            for p in &eff.params {
                match p.key.as_str() {
                    "file" | "ファイル" => {
                        if let ParamValue::Text(f) = &p.value {
                            let _ = write!(out, "ファイル={f}\r\n");
                        }
                    }
                    other => {
                        let val = format_param_value(&p.value, keys);
                        let _ = write!(out, "{other}={val}\r\n");
                    }
                }
            }
        }
        EffectKind::StandardDraw | EffectKind::ExtendedDraw => {
            for p in &eff.params {
                match p.key.as_str() {
                    "回転" | "Z軸回転" => {
                        let val = format_param_value(&p.value, keys);
                        let _ = write!(out, "Z軸回転={val}\r\n");
                    }
                    "blend" | "合成モード" => {
                        let raw = match &p.value {
                            ParamValue::Text(s) => s.as_str(),
                            _ => "0",
                        };
                        let blend = blend_code_to_aup2(raw);
                        let _ = write!(out, "合成モード={blend}\r\n");
                    }
                    other => {
                        let val = format_param_value(&p.value, keys);
                        let _ = write!(out, "{other}={val}\r\n");
                    }
                }
            }
            if eff_name == "映像再生" {
                out.push_str("音量=100.00\r\n左右=0.00\r\n");
            }
        }
        _ => {
            for p in &eff.params {
                let out_key = match p.key.as_str() {
                    "ｺﾝﾄﾗｽﾄ" => "コントラスト",
                    "range" if matches!(eff.kind, EffectKind::GroupControl | EffectKind::Camera) => {
                        "対象レイヤー数"
                    }
                    other => other,
                };
                let val = format_param_value(&p.value, keys);
                let _ = write!(out, "{out_key}={val}\r\n");
            }
        }
    }
}

fn format_param_value(pv: &ParamValue, keys: &[Frame]) -> String {
    match pv {
        ParamValue::Text(t) => escape_aup2_text(t),
        ParamValue::Track(t) => format_aup2_track(t, keys),
    }
}

fn shape_code_to_aup2_name(code: &str) -> &str {
    match code.trim() {
        "0" | "背景" => "背景",
        "1" | "円" => "円",
        "2" | "四角形" => "四角形",
        "3" | "三角形" => "三角形",
        "4" | "五角形" => "五角形",
        "5" | "六角形" => "六角形",
        "6" | "星型" => "星型",
        other => other,
    }
}

fn shape_aup2_name_to_code(name: &str) -> &str {
    match name.trim() {
        "背景" | "0" => "0",
        "円" | "1" => "1",
        "四角形" | "2" => "2",
        "三角形" | "3" => "3",
        "五角形" | "4" => "4",
        "六角形" | "5" => "5",
        "星型" | "6" => "6",
        other => other,
    }
}

fn text_deco_code_to_aup2(code: &str) -> &str {
    match code.trim() {
        "0" | "標準文字" => "標準文字",
        "1" | "影付き文字" => "影付き文字",
        "2" | "影付き文字(薄)" => "影付き文字(薄)",
        "3" | "縁取り文字" => "縁取り文字",
        "4" | "縁取り文字(細)" => "縁取り文字(細)",
        "5" | "縁取り文字(太)" => "縁取り文字(太)",
        "6" | "縁取り文字(角)" => "縁取り文字(角)",
        other => other,
    }
}

fn text_deco_aup2_to_code(name: &str) -> &str {
    match name.trim() {
        "標準文字" | "0" => "0",
        "影付き文字" | "1" => "1",
        "影付き文字(薄)" | "2" => "2",
        "縁取り文字" | "3" => "3",
        "縁取り文字(細)" | "4" => "4",
        "縁取り文字(太)" | "5" => "5",
        "縁取り文字(角)" | "6" => "6",
        other => other,
    }
}

fn text_align_code_to_aup2(code: &str) -> &str {
    match code.trim() {
        "0" | "左寄せ[上]" => "左寄せ[上]",
        "1" | "中央揃え[上]" => "中央揃え[上]",
        "2" | "右寄せ[上]" => "右寄せ[上]",
        "3" | "左寄せ[中]" => "左寄せ[中]",
        "4" | "中央揃え[中]" => "中央揃え[中]",
        "5" | "右寄せ[中]" => "右寄せ[中]",
        "6" | "左寄せ[下]" => "左寄せ[下]",
        "7" | "中央揃え[下]" => "中央揃え[下]",
        "8" | "右寄せ[下]" => "右寄せ[下]",
        other if other.contains('[') => other,
        _ => "左寄せ[上]",
    }
}

fn text_align_aup2_to_code(name: &str) -> &str {
    match name.trim() {
        "左寄せ[上]" | "0" => "0",
        "中央揃え[上]" | "1" => "1",
        "右寄せ[上]" | "2" => "2",
        "左寄せ[中]" | "3" => "3",
        "中央揃え[中]" | "4" => "4",
        "右寄せ[中]" | "5" => "5",
        "左寄せ[下]" | "6" => "6",
        "中央揃え[下]" | "7" => "7",
        "右寄せ[下]" | "8" => "8",
        other => other,
    }
}

fn blend_code_to_aup2(code: &str) -> &str {
    match code.trim() {
        "0" | "通常" => "通常",
        "1" | "加算" => "加算",
        "2" | "減算" => "減算",
        "3" | "乗算" => "乗算",
        "4" | "スクリーン" => "スクリーン",
        "5" | "オーバーレイ" => "オーバーレイ",
        "6" | "比較(明)" => "比較(明)",
        "7" | "比較(暗)" => "比較(暗)",
        "8" | "輝度" => "輝度",
        "9" | "色差" => "色差",
        "10" | "陰影" => "陰影",
        "11" | "明暗" => "明暗",
        "12" | "差分" => "差分",
        other => other,
    }
}

fn blend_aup2_to_code(name: &str) -> &str {
    match name.trim() {
        "通常" | "0" => "0",
        "加算" | "1" => "1",
        "減算" | "2" => "2",
        "乗算" | "3" => "3",
        "スクリーン" | "4" => "4",
        "オーバーレイ" | "5" => "5",
        "比較(明)" | "6" => "6",
        "比較(暗)" | "7" => "7",
        "輝度" | "8" => "8",
        "色差" | "9" => "9",
        "陰影" | "10" => "10",
        "明暗" | "11" => "11",
        "差分" | "12" => "12",
        other => other,
    }
}

fn escape_aup2_text(s: &str) -> String {
    s.replace("\r\n", "\\n").replace('\n', "\\n")
}

fn unescape_aup2_text(s: &str) -> String {
    s.replace("\\n", "\r\n")
}

/// 本家 AviUtl2 `Effect::EffectItemNumber::fromString` の仕様に完全準拠したトラック文字列化:
/// - 定数の場合: `<val>`
/// - アニメーションの場合: `<v0>,<v1>,...,<vK>,<移動モード名>,<フラグ整数>[|<時間制御パラメータ>]`
fn format_aup2_track(track: &Track, keys: &[Frame]) -> String {
    let p = track.precision as usize;
    let val_at = |f: Frame| {
        track
            .keyframes
            .iter()
            .find(|k| k.frame == f)
            .map(|k| k.value)
            .unwrap_or_else(|| track.evaluate(f))
    };
    match &track.interpolation {
        Some(interp) if track.keyframes.len() >= 2 && keys.len() >= 2 => {
            let mut parts: Vec<String> = keys
                .iter()
                .map(|&f| format!("{:.p$}", val_at(f)))
                .collect();

            let has_easing = interp.easing != Easing::None;
            let (mode_name, flag_val): (&str, u32) = match &interp.kind {
                MoveKind::Linear => (
                    if has_easing {
                        "直線移動(時間制御)"
                    } else {
                        "直線移動"
                    },
                    0,
                ),
                MoveKind::Curve => (
                    if has_easing {
                        "補間移動(時間制御)"
                    } else {
                        "補間移動"
                    },
                    0,
                ),
                MoveKind::Instant => ("瞬間移動", 0),
                MoveKind::IgnoreMidpoint => ("直線移動", 4),
                MoveKind::MoveAmount => ("移動量指定", 0),
                MoveKind::Random => ("ランダム移動", 0),
                MoveKind::Script(name) => (name.as_str(), 0),
                MoveKind::Other(_) => ("直線移動", 0),
            };

            parts.push(mode_name.to_string());
            parts.push(flag_val.to_string());
            let mut s = parts.join(",");

            if has_easing {
                let tc = match interp.easing {
                    Easing::EaseIn => "0.5,0,0,0",
                    Easing::EaseOut => "0,0,0.5,0",
                    Easing::EaseInOut => "0.5,0,0.5,0",
                    Easing::None => "",
                };
                if !tc.is_empty() {
                    s.push('|');
                    s.push_str(tc);
                }
            } else if let Some(param) = &interp.param {
                s.push('|');
                s.push_str(param);
            }
            s
        }
        _ => format!("{:.p$}", track.start_value()),
    }
}

/// `.aup2` バイト列（UTF-8 / Shift_JIS 自動判別）を解析します。
pub fn parse_aup2_bytes(bytes: &[u8]) -> Result<AupDocument, AupError> {
    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Ok(s) = std::str::from_utf8(bytes) {
        s.to_owned()
    } else {
        encoding_rs::SHIFT_JIS.decode(bytes).0.into_owned()
    };
    parse_aup2_str(&text)
}

/// `.aup2` テキストを解析して [`AupDocument`] を返します。
pub fn parse_aup2_str(text: &str) -> Result<AupDocument, AupError> {
    let mut warnings = Vec::new();
    let sections = parse_ini_sections(text, &mut warnings);

    let has_project = sections.iter().any(|s| s.name.eq_ignore_ascii_case("project"));
    let has_scene = sections
        .iter()
        .any(|s| s.name.starts_with("scene.") || s.name.eq_ignore_ascii_case("scene"));
    if !has_project && !has_scene {
        return Err(AupError::InvalidHeader);
    }

    let mut version = AUP2_DEFAULT_VERSION.to_string();
    if let Some(proj) = sections.iter().find(|s| s.name.eq_ignore_ascii_case("project")) {
        if let Some(v) = proj.get("version") {
            version = v.trim().to_string();
        }
    }

    let mut settings = SceneSettings::default();
    let mut scene_name = "Root".to_string();
    let mut cursor_frame: Frame = 0;

    if let Some(sc) = sections
        .iter()
        .find(|s| s.name.starts_with("scene.") || s.name.eq_ignore_ascii_case("scene"))
    {
        if let Some(n) = sc.get("name") {
            scene_name = n.trim().to_string();
        }
        let mut rate = settings.frame_rate.num;
        let mut scale = settings.frame_rate.den;
        for (k, v) in &sc.entries {
            let val = v.trim();
            match k.as_str() {
                "video.width" | "width" => {
                    if let Ok(x) = val.parse() {
                        settings.width = x;
                    }
                }
                "video.height" | "height" => {
                    if let Ok(x) = val.parse() {
                        settings.height = x;
                    }
                }
                "video.rate" | "rate" => {
                    if let Ok(x) = val.parse() {
                        rate = x;
                    }
                }
                "video.scale" | "scale" => {
                    if let Ok(x) = val.parse() {
                        scale = x;
                    }
                }
                "audio.rate" | "audio_rate" => {
                    if let Ok(x) = val.parse() {
                        settings.audio_rate = x;
                    }
                }
                "length" => {
                    if let Ok(x) = val.parse() {
                        settings.length = x;
                    }
                }
                "cursor.frame" => {
                    if let Ok(x) = val.parse() {
                        cursor_frame = x;
                    }
                }
                _ => {}
            }
        }
        settings.frame_rate = Rational::new(rate.max(1), scale.max(1));
    }

    let mut scene = Scene::new(scene_name, settings);

    // オブジェクトセクション [N] とエフェクトセクション [N.M] を収集
    let mut headers: BTreeMap<u32, &IniSection> = BTreeMap::new();
    let mut effects: BTreeMap<u32, Vec<(u32, &IniSection)>> = BTreeMap::new();

    for s in &sections {
        if s.name.eq_ignore_ascii_case("project")
            || s.name.starts_with("scene.")
            || s.name.eq_ignore_ascii_case("scene")
            || s.name.starts_with("layer.")
        {
            continue;
        }
        let parsed = match s.name.split_once('.') {
            None => s.name.parse::<u32>().ok().map(|n| (n, None)),
            Some((a, b)) => a
                .parse::<u32>()
                .ok()
                .zip(b.parse::<u32>().ok())
                .map(|(n, m)| (n, Some(m))),
        };
        match parsed {
            Some((n, None)) => {
                headers.insert(n, s);
            }
            Some((n, Some(m))) => {
                effects.entry(n).or_default().push((m, s));
            }
            None => {}
        }
    }

    // chain=1 による中間点連結および本家 .aup2 の frame=start,mid,...,end の両方に対応
    let mut pending: Option<Vec<Aup2Segment>> = None;
    for (n, h) in headers {
        let seg = parse_aup2_segment(h, effects.remove(&n).unwrap_or_default());
        if seg.chain {
            if let Some(p) = pending.as_mut() {
                let last = p.last().unwrap();
                if last.layer == seg.layer && last.end + 1 == seg.start {
                    p.push(seg);
                    continue;
                }
            }
        }
        if let Some(p) = pending.take() {
            finalize_aup2_segments(p, &mut scene);
        }
        pending = Some(vec![seg]);
    }
    if let Some(p) = pending.take() {
        finalize_aup2_segments(p, &mut scene);
    }

    // シーン長がオブジェクト終端より短い場合は自動拡張
    let content_end = scene.content_end();
    if content_end > scene.settings.length {
        scene.settings.length = content_end;
    }

    let mut scenes = BTreeMap::new();
    scenes.insert(0, scene.clone());

    Ok(AupDocument {
        version,
        scene,
        scenes,
        cursor_frame,
        warnings,
    })
}

struct IniSection {
    name: String,
    entries: Vec<(String, String)>,
}

impl IniSection {
    fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

fn parse_ini_sections(text: &str, warnings: &mut Vec<String>) -> Vec<IniSection> {
    let mut out: Vec<IniSection> = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches(['\r', '\0']);
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        if let Some(sec) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            out.push(IniSection {
                name: sec.trim().to_string(),
                entries: Vec::new(),
            });
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if let Some(cur) = out.last_mut() {
                cur.entries.push((k.trim().to_string(), v.to_string()));
            }
        } else {
            warnings.push(format!("{} 行目: 不正な行をスキップしました", idx + 1));
        }
    }
    out
}

struct Aup2Segment {
    start: Frame,
    end: Frame,
    /// `frame=start,mid1,...,end` 形式で単一セクションに中間点が含まれている場合の絶対フレーム列
    inline_midpoints: Vec<Frame>,
    layer: usize,
    chain: bool,
    group: Option<u32>,
    clipping: bool,
    camera: bool,
    audio: bool,
    effects: Vec<(String, bool, Vec<(String, String)>)>,
}

fn parse_aup2_segment(
    h: &IniSection,
    mut sub_sections: Vec<(u32, &IniSection)>,
) -> Aup2Segment {
    // frame=start,mid,...,end 形式または start= / end= 形式の両対応
    let mut start = 0u32;
    let mut end = 60u32;
    let mut inline_midpoints = Vec::new();
    let uses_frame_key = h.get("frame").is_some();

    if let Some(frame_list) = h.get("frame") {
        let nums: Vec<u32> = frame_list
            .split(',')
            .filter_map(|s| s.trim().parse::<u32>().ok())
            .collect();
        if nums.len() >= 2 {
            start = nums[0];
            end = *nums.last().unwrap();
            if nums.len() > 2 {
                inline_midpoints.extend_from_slice(&nums[1..nums.len() - 1]);
            }
        }
    } else {
        if let Some(s) = h.get("start").and_then(|v| v.trim().parse::<u32>().ok()) {
            start = s;
        }
        if let Some(e) = h.get("end").and_then(|v| v.trim().parse::<u32>().ok()) {
            end = e.max(start);
        }
    }

    let layer_raw = h
        .get("layer")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    // 本家 .aup2 (frame= 形式) は layer=0 始まり、旧 start=/end= 形式は 1 始まり
    let layer = if uses_frame_key {
        layer_raw
    } else {
        layer_raw.saturating_sub(1)
    };

    let flag = |k: &str| h.get(k).map(|v| v.trim() == "1").unwrap_or(false);

    let mut effects = Vec::new();

    if let Some(inline_name) = h.get("effect.name").or_else(|| h.get("_name")) {
        let enabled = h
            .get("effect.disable")
            .or_else(|| h.get("_disable"))
            .map(|v| v.trim() != "1")
            .unwrap_or(true);
        let params: Vec<(String, String)> = h
            .entries
            .iter()
            .filter(|(k, _)| {
                !matches!(
                    k.as_str(),
                    "start"
                        | "end"
                        | "frame"
                        | "focus"
                        | "scene"
                        | "layer"
                        | "alpha"
                        | "blend"
                        | "chain"
                        | "group"
                        | "group.control"
                        | "clipping"
                        | "clipping.upper"
                        | "camera"
                        | "audio"
                        | "overlay"
                        | "effect.name"
                        | "_name"
                        | "effect.disable"
                        | "_disable"
                )
            })
            .cloned()
            .collect();
        effects.push((inline_name.to_string(), enabled, params));
    }

    sub_sections.sort_by_key(|(m, _)| *m);
    for (_, s) in sub_sections {
        let name = s
            .get("effect.name")
            .or_else(|| s.get("_name"))
            .unwrap_or("フィルタ")
            .to_string();
        let enabled = s
            .get("effect.disable")
            .or_else(|| s.get("_disable"))
            .map(|v| v.trim() != "1")
            .unwrap_or(true);
        let params = s
            .entries
            .iter()
            .filter(|(k, _)| {
                !matches!(
                    k.as_str(),
                    "effect.name" | "_name" | "effect.disable" | "_disable" | "display.hide"
                )
            })
            .cloned()
            .collect();
        effects.push((name, enabled, params));
    }

    Aup2Segment {
        start,
        end,
        inline_midpoints,
        layer,
        chain: flag("chain"),
        group: h.get("group").and_then(|v| v.trim().parse().ok()),
        clipping: flag("clipping") || flag("clipping.upper"),
        camera: flag("camera"),
        audio: flag("audio"),
        effects,
    }
}

fn normalize_aup2_effect_name(name: &str) -> (String, Option<&'static str>) {
    match name {
        "映像再生" => ("標準描画".into(), None),
        "音声再生" => ("標準再生".into(), None),
        "震える" => ("アニメーション効果".into(), Some("揺れ@OpenAvi")),
        "点滅" => ("アニメーション効果".into(), Some("点滅@OpenAvi")),
        "座標の回転(個別オブジェクト)" => {
            ("アニメーション効果".into(), Some("回転@OpenAvi"))
        }
        other => (other.to_string(), None),
    }
}

fn normalize_aup2_key(eff_kind: &EffectKind, orig_eff_name: &str, key: &str) -> String {
    match eff_kind {
        EffectKind::Shape => match key {
            "図形の種類" => return "type".into(),
            "色" => return "color".into(),
            _ => {}
        },
        EffectKind::Text => match key {
            "テキスト" => return "text".into(),
            "テキストサイズ" => return "サイズ".into(),
            "フォント" => return "font".into(),
            "文字色" => return "color".into(),
            "影・縁色" => return "color2".into(),
            "文字装飾" => return "type".into(),
            "文字揃え" => return "align".into(),
            "字間" => return "spacing_x".into(),
            "行間" => return "spacing_y".into(),
            "オブジェクトの長さを自動調節" => return "autoadjust".into(),
            "移動座標上に表示" => return "移動座標上に表示する".into(),
            _ => {}
        },
        EffectKind::VideoFile | EffectKind::ImageFile | EffectKind::AudioFile => {
            if key == "ファイル" {
                return "file".into();
            }
        }
        EffectKind::StandardDraw | EffectKind::ExtendedDraw => match key {
            "Z軸回転" => return "回転".into(),
            "合成モード" => return "blend".into(),
            _ => {}
        },
        _ => match (orig_eff_name, key) {
            ("震える", "振幅") => return "track0".into(),
            ("震える", "角度") => return "track1".into(),
            ("震える", "間隔") => return "track2".into(),
            ("点滅", "速さ") => return "track0".into(),
            ("点滅", "点滅割合") => return "track1".into(),
            ("点滅", "点灯速度") => return "track2".into(),
            ("点滅", "消灯速度") => return "track3".into(),
            ("座標の回転(個別オブジェクト)", "角度") => return "track0".into(),
            ("座標の回転(個別オブジェクト)", "中心X") => return "track1".into(),
            ("座標の回転(個別オブジェクト)", "中心Y") => return "track2".into(),
            (_, "コントラスト") => return "ｺﾝﾄﾗｽﾄ".into(),
            _ => {}
        },
    }
    key.to_string()
}

fn normalize_aup2_text_value(eff_kind: &EffectKind, key: &str, raw_val: &str) -> String {
    match (eff_kind, key) {
        (EffectKind::Shape, "type") => shape_aup2_name_to_code(raw_val).to_string(),
        (EffectKind::Text, "type") => text_deco_aup2_to_code(raw_val).to_string(),
        (EffectKind::Text, "align") => text_align_aup2_to_code(raw_val).to_string(),
        (EffectKind::StandardDraw | EffectKind::ExtendedDraw, "blend") => {
            blend_aup2_to_code(raw_val).to_string()
        }
        _ => unescape_aup2_text(raw_val),
    }
}

fn is_text_setting_key(key: &str) -> bool {
    matches!(
        key,
        "text"
            | "テキスト"
            | "file"
            | "ファイル"
            | "font"
            | "フォント"
            | "color"
            | "color2"
            | "色"
            | "文字色"
            | "影・縁色"
            | "文字装飾"
            | "文字揃え"
            | "図形の種類"
            | "合成モード"
            | "name"
            | "blend"
            | "type"
            | "align"
            | "YUV"
    )
}

/// 本家 `.aup2` の `<v0>,<v1>,...,<vK>,<移動モード名>,<フラグ>[|<param>]` 形式と
/// 旧 `<v0>,<v1>,<数値コード>` 形式の両方を解析します。
fn parse_aup2_track_values(raw: &str) -> Option<(Vec<f64>, u8, Option<Interpolation>)> {
    let (main_part, pipe_param) = match raw.split_once('|') {
        Some((m, p)) => (m, Some(p.to_string())),
        None => (raw, None),
    };
    let parts: Vec<&str> = main_part.split(',').collect();
    let first: f64 = parts[0].trim().parse().ok().filter(|v: &f64| v.is_finite())?;
    let prec = parts[0]
        .trim()
        .split_once('.')
        .map(|(_, f)| f.len().min(6) as u8)
        .unwrap_or(0);
    if parts.len() == 1 {
        return Some((vec![first], prec, None));
    }

    // パターン A: 本家 .aup2 形式 `v0,v1,...,vK,移動モード名,フラグ整数` (parts.len() >= 4)
    if parts.len() >= 4 {
        let mode_token = parts[parts.len() - 2].trim();
        let flag_token = parts[parts.len() - 1].trim();
        let values: Option<Vec<f64>> = parts[..parts.len() - 2]
            .iter()
            .map(|s| s.trim().parse::<f64>().ok().filter(|v| v.is_finite()))
            .collect();
        if let (Some(vals), Ok(flag_num)) = (values, flag_token.parse::<u32>()) {
            let (kind, mut easing) = match mode_token {
                "直線移動" => (
                    if flag_num & 4 != 0 {
                        MoveKind::IgnoreMidpoint
                    } else {
                        MoveKind::Linear
                    },
                    Easing::None,
                ),
                "直線移動(時間制御)" => (MoveKind::Linear, Easing::EaseInOut),
                "補間移動" => (MoveKind::Curve, Easing::None),
                "補間移動(時間制御)" => (MoveKind::Curve, Easing::EaseInOut),
                "瞬間移動" => (MoveKind::Instant, Easing::None),
                "移動量指定" => (MoveKind::MoveAmount, Easing::None),
                "ランダム移動" => (MoveKind::Random, Easing::None),
                other => (MoveKind::Script(other.to_string()), Easing::None),
            };
            if let Some(ref tc) = pipe_param {
                match tc.trim() {
                    "0.5,0,0,0" => easing = Easing::EaseIn,
                    "0,0,0.5,0" => easing = Easing::EaseOut,
                    "0.5,0,0.5,0" => easing = Easing::EaseInOut,
                    _ => {}
                }
            }
            return Some((
                vals,
                prec,
                Some(Interpolation {
                    kind,
                    easing,
                    param: pipe_param,
                    legacy_code: None,
                }),
            ));
        }
    }

    // パターン B: 旧 .exo 互換 `v0,v1,code[@script][,param]` (parts.len() >= 3)
    if parts.len() >= 3 {
        let end: f64 = parts[1].trim().parse().ok().filter(|v: &f64| v.is_finite())?;
        let (code_str, script) = match parts[2].split_once('@') {
            Some((c, s)) => (c, Some(s.to_string())),
            None => (parts[2], None),
        };
        if let Ok(code) = code_str.trim().parse::<u32>() {
            let base = code & !0x60;
            let kind = match (script, base) {
                (Some(name), _) => MoveKind::Script(name),
                (None, 1) => MoveKind::Linear,
                (None, 2) => MoveKind::Curve,
                (None, 3) => MoveKind::Instant,
                (None, 4) => MoveKind::IgnoreMidpoint,
                (None, 5) => MoveKind::MoveAmount,
                (None, 6) => MoveKind::Random,
                (None, other) => MoveKind::Other(other),
            };
            let easing = Easing::from_flags(code & 0x40 != 0, code & 0x20 != 0);
            let param = (parts.len() > 3)
                .then(|| parts[3..].join(","))
                .or(pipe_param);
            return Some((
                vec![first, end],
                prec,
                Some(Interpolation {
                    kind,
                    easing,
                    param,
                    legacy_code: Some(base),
                }),
            ));
        }
    }
    None
}

fn finalize_aup2_segments(segments: Vec<Aup2Segment>, scene: &mut Scene) {
    let first = &segments[0];
    let last = segments.last().unwrap();
    let mut obj = TimelineObject::new(first.start, last.end);
    let base = obj.frame_start;
    let last_rel = obj.frame_end - base;
    if segments.len() > 1 {
        obj.midpoints = segments[1..].iter().map(|s| s.start - base).collect();
    } else if !first.inline_midpoints.is_empty() {
        obj.midpoints = first
            .inline_midpoints
            .iter()
            .filter_map(|&abs_f| abs_f.checked_sub(base))
            .filter(|&rel| rel > 0 && rel < last_rel)
            .collect();
    }
    obj.group = first.group;
    obj.clipping = first.clipping;
    obj.camera_target = first.camera;
    obj.audio = first.audio;

    let keys = obj.key_frames();

    for (ei, (orig_name, enabled, entries)) in first.effects.iter().enumerate() {
        let (norm_name, anim_script_name) = normalize_aup2_effect_name(orig_name);
        let mut eff = Effect::new(norm_name);
        eff.enabled = *enabled;
        if let Some(script_name) = anim_script_name {
            eff.push_text("name", script_name);
        }

        for (raw_key, raw_val) in entries {
            let key = normalize_aup2_key(&eff.kind, orig_name, raw_key);
            let pv = if eff.kind == EffectKind::Text && key == "text" {
                let decoded = au2_exo::decode_text_hex(raw_val)
                    .filter(|_| {
                        raw_val.len() >= 16 && raw_val.chars().all(|c| c.is_ascii_hexdigit())
                    })
                    .unwrap_or_else(|| unescape_aup2_text(raw_val));
                ParamValue::Text(decoded)
            } else if is_text_setting_key(&key) {
                ParamValue::Text(normalize_aup2_text_value(&eff.kind, &key, raw_val))
            } else {
                match parse_aup2_track_values(raw_val) {
                    None => ParamValue::Text(raw_val.clone()),
                    Some((vals, prec, None)) => ParamValue::Track(Track::constant(vals[0], prec)),
                    Some((vals, prec, Some(interp))) => {
                        let keyframes = if segments.len() == 1 && vals.len() >= 2 {
                            // 単一セクション [N] 内に全キーフレーム値が並んでいる場合 (本家 .aup2 標準)
                            vals.iter()
                                .enumerate()
                                .map(|(idx, &v)| {
                                    let f = keys
                                        .get(idx)
                                        .copied()
                                        .unwrap_or(last_rel);
                                    Keyframe { frame: f, value: v }
                                })
                                .collect()
                        } else {
                            // chain=1 複数セクション形式の場合
                            let mut kf = Vec::with_capacity(segments.len() + 1);
                            let mut last_end = *vals.last().unwrap_or(&0.0);
                            for seg in &segments {
                                if let Some((_, _, seg_entries)) = seg.effects.get(ei) {
                                    if let Some((_, sv)) = seg_entries.iter().find(|(k, _)| {
                                        normalize_aup2_key(&eff.kind, orig_name, k) == key
                                    }) {
                                        if let Some((svals, _, _)) = parse_aup2_track_values(sv) {
                                            kf.push(Keyframe {
                                                frame: seg.start - base,
                                                value: svals[0],
                                            });
                                            last_end = *svals.last().unwrap_or(&svals[0]);
                                        }
                                    }
                                }
                            }
                            match kf.last_mut() {
                                Some(k) if k.frame >= last_rel => k.value = last_end,
                                _ => kf.push(Keyframe {
                                    frame: last_rel,
                                    value: last_end,
                                }),
                            }
                            kf
                        };
                        ParamValue::Track(Track {
                            keyframes,
                            interpolation: Some(interp),
                            precision: prec,
                        })
                    }
                }
            };
            eff.params.push(Param { key, value: pv });
        }
        obj.effects.push(eff);
    }

    if matches!(
        obj.source().map(|e| &e.kind),
        Some(EffectKind::AudioFile | EffectKind::SceneAudio)
    ) {
        obj.audio = true;
    }

    scene.insert_object(first.layer, obj);
}

// ============================================================================
// 初代 AviUtl (.aup) バイナリパーサー & ライター
// ============================================================================

/// シーンを初代 `.aup` バイナリ形式でシリアライズします。
pub fn write_aup_bytes(scene: &Scene) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(AUP_HEADER_018);

    let s = &scene.settings;
    out.extend_from_slice(&s.width.to_le_bytes());
    out.extend_from_slice(&s.height.to_le_bytes());
    out.extend_from_slice(&s.frame_rate.num.to_le_bytes());
    out.extend_from_slice(&s.frame_rate.den.to_le_bytes());
    out.extend_from_slice(&s.length.to_le_bytes());
    out.extend_from_slice(&s.audio_rate.to_le_bytes());
    out.extend_from_slice(&s.audio_channels.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());

    out.extend_from_slice(OAVI_CHUNK_MAGIC);
    let exo_bytes = write_exo_bytes(scene, TextEncoding::ShiftJis);
    out.extend_from_slice(&(exo_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&exo_bytes);

    out
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
        if doc.scene.object_count() > 0 {
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
mod tests {
    use super::*;

    #[test]
    fn aup2_roundtrip_and_spec_compliance() {
        let mut scene = Scene::default();
        scene.settings.width = 1920;
        scene.settings.height = 1080;
        let text_obj = TimelineObject::new_text(0, 120, "OpenAvi\r\nテスト文字列");
        scene.insert_object(0, text_obj);

        let mut shape_obj = TimelineObject::new_shape(0, 120, 1, "e94560");
        shape_obj.add_midpoint(60);
        if let Some(draw) = shape_obj
            .effects
            .iter_mut()
            .find(|e| e.kind == EffectKind::StandardDraw)
        {
            if let Some(p) = draw.params.iter_mut().find(|p| p.key == "X") {
                p.value = ParamValue::Track(Track {
                    keyframes: vec![
                        Keyframe {
                            frame: 0,
                            value: -400.0,
                        },
                        Keyframe {
                            frame: 60,
                            value: 0.0,
                        },
                        Keyframe {
                            frame: 120,
                            value: 400.0,
                        },
                    ],
                    interpolation: Some(Interpolation {
                        kind: MoveKind::Linear,
                        easing: Easing::None,
                        param: None,
                        legacy_code: Some(1),
                    }),
                    precision: 1,
                });
            }
        }
        scene.insert_object(1, shape_obj);

        let aup2_text = write_aup2_string(&scene, Some(Path::new("C:\\video\\test.aup2")), 42);
        assert!(aup2_text.contains("[project]"));
        assert!(aup2_text.contains("version=2011400"));
        assert!(aup2_text.contains("[scene.0]"));
        assert!(aup2_text.contains("video.width=1920"));
        // 本家 AviUtl2 必須: layer=0 始まり & frame=start,end 形式
        assert!(aup2_text.contains("[0]\r\nlayer=0\r\nframe=0,120\r\n"));
        assert!(aup2_text.contains("[1]\r\nlayer=1\r\nframe=0,60,120\r\n"));
        assert!(aup2_text.contains("effect.name=テキスト"));
        assert!(aup2_text.contains("文字装飾=標準文字"));
        assert!(aup2_text.contains("文字揃え=中央揃え[中]"));
        assert!(aup2_text.contains("テキスト=OpenAvi\\nテスト文字列"));
        assert!(aup2_text.contains("effect.name=図形"));
        assert!(aup2_text.contains("図形の種類=円"));
        assert!(aup2_text.contains("色=e94560"));
        assert!(aup2_text.contains("角を丸くする=0"));
        assert!(aup2_text.contains("effect.name=標準描画"));
        assert!(aup2_text.contains("X=-400.0,0.0,400.0,直線移動,0"));
        assert!(aup2_text.contains("合成モード=通常"));

        let parsed = parse_aup2_str(&aup2_text).expect("parse aup2");
        assert_eq!(parsed.version, "2011400");
        assert_eq!(parsed.cursor_frame, 42);
        assert_eq!(parsed.scene.object_count(), 2);
        let obj = &parsed.scene.layers[0].objects[0];
        assert_eq!(obj.display_name(), "OpenAvi");
        assert_eq!(
            obj.source().unwrap().text("text"),
            Some("OpenAvi\r\nテスト文字列")
        );
        let shape = &parsed.scene.layers[1].objects[0];
        assert_eq!(shape.midpoints, vec![60]);
        let draw = shape.effect(&EffectKind::StandardDraw).unwrap();
        assert_eq!(draw.eval("X", 0, 0.0), -400.0);
        assert_eq!(draw.eval("X", 60, 0.0), 0.0);
        assert_eq!(draw.eval("X", 120, 0.0), 400.0);
    }

    #[test]
    fn parse_inline_effect_aup2_format() {
        let sample_aup2 = "\
[project]
version=2001802
file=F:\\test.aup2
display.scene=0

[scene.0]
scene=0
name=Root
video.width=1920
video.height=1080
video.rate=60
video.scale=1
audio.rate=44100
cursor.frame=80

[0]
start=0
end=120
layer=1
effect.name=テキスト
テキストサイズ=40.00
フォント=Yu Gothic UI
文字色=ffffff
テキスト=サンプルテキスト

[0.1]
effect.name=標準描画
X=10.00
Y=20.00
拡大率=100.00
透明度=0.00
回転=0.00
";
        let doc = parse_aup2_str(sample_aup2).expect("parse inline aup2");
        assert_eq!(doc.scene.settings.frame_rate.num, 60);
        assert_eq!(doc.cursor_frame, 80);
        let obj = &doc.scene.layers[0].objects[0];
        assert_eq!(obj.display_name(), "サンプルテキスト");
        let draw = obj.effect(&EffectKind::StandardDraw).unwrap();
        assert_eq!(draw.eval("X", 0, 0.0), 10.0);
        assert_eq!(draw.eval("Y", 0, 0.0), 20.0);
    }

    #[test]
    fn aup_roundtrip() {
        let mut scene = Scene::default();
        let mut obj = TimelineObject::new(0, 59);
        obj.effects.push(Effect::new("テキスト"));
        scene.insert_object(0, obj);

        let bytes = write_aup_bytes(&scene);
        assert!(bytes.starts_with(AUP_HEADER_018));

        let doc = parse_aup_bytes(&bytes).expect("parse aup");
        assert_eq!(doc.version, "0.18");
        assert_eq!(doc.scene.object_count(), 1);
    }

    #[test]
    fn legacy_aup_extracts_media_paths() {
        let mut raw = Vec::new();
        raw.extend_from_slice(AUP_HEADER_018);
        raw.resize(128, 0);
        raw.extend_from_slice(b"C:\\videos\\test_clip.mp4\0");
        raw.extend_from_slice(b"D:\\images\\logo.png\0");

        let doc = parse_aup_bytes(&raw).expect("parse legacy aup");
        assert_eq!(doc.scene.object_count(), 2);
        assert_eq!(doc.scene.layers[0].objects[0].display_name(), "test_clip.mp4");
        assert_eq!(doc.scene.layers[1].objects[0].display_name(), "logo.png");
    }

    #[test]
    fn parse_real_goot_time_pf_aup_if_present() {
        let candidates = [
            r"C:\Users\user\Desktop\goot time pf.aup",
            r"c:\Users\user\AppData\Local\Temp\goot time pf.aup",
        ];
        for candidate in candidates {
            let path = std::path::Path::new(candidate);
            if path.exists() {
                let bytes = std::fs::read(path).expect("read real aup");
                let doc = parse_aup_bytes(&bytes).expect("parse real aup");
                assert_eq!(doc.scene.settings.width, 1280);
                assert_eq!(doc.scene.settings.height, 720);
                assert_eq!(doc.scene.settings.frame_rate.num, 30);
                assert_eq!(doc.scene.settings.frame_rate.den, 1);
                assert!(doc.scene.object_count() >= 3000);
                assert!(doc.scene.layers.len() >= 90);
                break;
            }
        }
    }

    #[test]
    fn test_parse_all_aupdotnet_samples() {
        let files = [
            "Chain.aup",
            "DisabledEffect.aup",
            "EffectSet01.aup",
            "ExeditBpm.aup",
            "ExeditCamera.aup",
            "ExeditXY.aup",
            "Group.aup",
            "LayerScene.aup",
            "Trackbar.aup",
        ];

        for fname in &files {
            let path = format!("C:\\Users\\user\\.gemini\\antigravity\\scratch\\aupdotnet\\AupDotNet-main\\testdata\\Exedit\\{}", fname);
            let bytes = std::fs::read(&path).unwrap();
            let fp_magic = b"FilterProject 0.1\0";
            let mut exedit_chunk = None;
            let mut search_pos = 0;
            while let Some(idx) = bytes[search_pos..].windows(fp_magic.len()).position(|w| w == fp_magic) {
                let abs_pos = search_pos + idx;
                let nl_pos = abs_pos + fp_magic.len();
                let nl = u32::from_le_bytes(bytes[nl_pos..nl_pos + 4].try_into().unwrap()) as usize;
                let n_pos = nl_pos + 4;
                let (f_name, _, _) = encoding_rs::SHIFT_JIS.decode(&bytes[n_pos..n_pos + nl]);
                if f_name.trim_matches('\0') == "拡張編集" {
                    let d_len_pos = n_pos + nl;
                    let decomp_sz = u32::from_le_bytes(bytes[d_len_pos..d_len_pos + 4].try_into().unwrap()) as usize;
                    let comp_start = d_len_pos + 4;
                    exedit_chunk = Some((comp_start, decomp_sz));
                    break;
                }
                search_pos = abs_pos + fp_magic.len();
            }

            let (comp_start, decomp_sz) = exedit_chunk.unwrap();
            let decomp = legacy_aup::decompress_aup(&bytes[comp_start..], decomp_sz);

            let effect_type_num = u32::from_le_bytes(decomp[4..8].try_into().unwrap()) as usize;
            let object_num = u32::from_le_bytes(decomp[8..12].try_into().unwrap()) as usize;
            let scene_num = u32::from_le_bytes(decomp[0x68..0x6c].try_into().unwrap()) as usize;
            let layer_num = u32::from_le_bytes(decomp[0x6c..0x70].try_into().unwrap()) as usize;
            let trackbar_script_num = u32::from_le_bytes(decomp[0x7c..0x80].try_into().unwrap()) as usize;

            let mut eff_types = Vec::new();
            let mut eff_cur = 0x100 + layer_num * 76 + scene_num * 220 + trackbar_script_num * 128;
            for _ in 0..effect_type_num {
                let e_data = &decomp[eff_cur..eff_cur + 112];
                let tb_num = u32::from_le_bytes(e_data[4..8].try_into().unwrap()) as usize;
                let cb_num = u32::from_le_bytes(e_data[8..12].try_into().unwrap()) as usize;
                let ext_sz = u32::from_le_bytes(e_data[12..16].try_into().unwrap()) as usize;
                let (e_name, _, _) = encoding_rs::SHIFT_JIS.decode(&e_data[16..16 + 96]);
                let e_name = e_name.trim_matches('\0').to_string();
                eff_types.push((e_name, tb_num, cb_num, ext_sz));
                eff_cur += 112;
            }

            let mut cursor = eff_cur;
            let mut last_chain_group = 0xFFFF_FFFFu32;
            let disabled_count = 0;
            for i in 0..object_num {
                let o_data = &decomp[cursor..];
                let _flag = u32::from_le_bytes(o_data[0..4].try_into().unwrap());
                let chain_group = u32::from_le_bytes(o_data[0x50..0x54].try_into().unwrap());
                let ext_size_field = u32::from_le_bytes(o_data[0xF4..0xF8].try_into().unwrap()) as usize;
                let is_chain = chain_group != 0xFFFF_FFFF && chain_group == last_chain_group && ext_size_field == 0;

                let mut ext_size = 0usize;
                for e in 0..12 {
                    let type_idx = i32::from_le_bytes(o_data[0x54 + e * 12..0x54 + e * 12 + 4].try_into().unwrap());
                    if type_idx == -1 {
                        break;
                    }
                    let (_, _, _, sz) = &eff_types[type_idx as usize];
                    ext_size += *sz;
                }
                if is_chain {
                    ext_size = 0;
                }

                assert_eq!(ext_size_field, ext_size, "{}: obj {} ext_size_field ({}) != computed ext_size ({})",
                    fname, i, ext_size_field, ext_size);

                let obj_size = 0x5C8 + ext_size_field;
                last_chain_group = chain_group;
                cursor += obj_size;
            }
            let version = u32::from_le_bytes(decomp[0x2C..0x30].try_into().unwrap());
            println!("{}: version=0x{:04x}, objects={}, disabled={}, cursor={}/{}", fname, version, object_num, disabled_count, cursor, decomp.len());
            assert_eq!(cursor, decomp.len(), "{}: cursor mismatch", fname);
        }
    }

    #[test]
    fn test_find_kudaketa() {
        let path = std::path::Path::new(r"C:\Users\user\Desktop\配布　1280x720_24fps_44100Hz.aup");
        if !path.exists() { return; }
        let bytes = std::fs::read(path).unwrap();
        let fp_magic = b"FilterProject 0.1\0";
        let mut search_pos = 0;
        let mut exedit_chunk = None;
        while let Some(idx) = bytes[search_pos..].windows(fp_magic.len()).position(|w| w == fp_magic) {
            let abs_pos = search_pos + idx;
            let nl_pos = abs_pos + fp_magic.len();
            let nl = u32::from_le_bytes(bytes[nl_pos..nl_pos + 4].try_into().unwrap()) as usize;
            let n_pos = nl_pos + 4;
            let (f_name, _, _) = encoding_rs::SHIFT_JIS.decode(&bytes[n_pos..n_pos + nl]);
            if f_name.trim_matches('\0') == "拡張編集" {
                let d_len_pos = n_pos + nl;
                let decomp_sz = u32::from_le_bytes(bytes[d_len_pos..d_len_pos + 4].try_into().unwrap()) as usize;
                let comp_start = d_len_pos + 4;
                exedit_chunk = Some((comp_start, decomp_sz));
                break;
            }
            search_pos = abs_pos + fp_magic.len();
        }
        let (comp_start, decomp_sz) = exedit_chunk.unwrap();
        let decomp = legacy_aup::decompress_aup(&bytes[comp_start..], decomp_sz);

        let s_kuda = b"\x8d\xd3\x82\xaf\x82\xbd"; // 砕けた
        for i in 0..decomp.len().saturating_sub(s_kuda.len()) {
            if &decomp[i..i + s_kuda.len()] == s_kuda {
                println!("FOUND 砕けた at 0x{:X}", i);
                let start = i.saturating_sub(64);
                let end = (i + 32).min(decomp.len());
                println!("CONTEXT [-64..+32]: {:02X?}", &decomp[start..end]);
                let (txt, _, _) = encoding_rs::SHIFT_JIS.decode(&decomp[start..end]);
                println!("  as text: {:?}", txt);
            }
        }
    }

    #[test]
    fn test_inspect_all_effects_in_target() {
        let path = std::path::Path::new(r"C:\Users\user\Desktop\配布　1280x720_24fps_44100Hz.aup");
        if !path.exists() {
            return;
        }
        let bytes = std::fs::read(path).expect("read target aup");
        let doc = legacy_aup::parse_legacy_aup(&bytes).expect("parse_legacy_aup should succeed");

        println!("=== ALL DISTINCT EFFECT NAMES IN TARGET AUP ===");
        let mut effect_counts = std::collections::BTreeMap::new();
        let mut blend_modes = std::collections::BTreeMap::new();
        let mut shapes_with_line_width = Vec::new();
        let mut camera_objects = Vec::new();
        let mut glow_objects = Vec::new();

        for (li, layer) in doc.scene.layers.iter().enumerate() {
            for obj in &layer.objects {
                for eff in &obj.effects {
                    *effect_counts.entry(eff.name.clone()).or_insert(0) += 1;
                    if eff.name == "図形" {
                        let line_width = eff.eval("ライン幅", 0, 0.0);
                        if line_width > 0.0 {
                            shapes_with_line_width.push((li, obj.frame_start..obj.frame_end, eff.text("name").map(|s| s.to_string()), line_width));
                        }
                    }
                    if eff.name.contains("グロー") || eff.name.contains("発光") || eff.name.contains("閃光") || eff.name.contains("拡散光") {
                        glow_objects.push((li, obj.frame_start..obj.frame_end, eff.name.clone(), eff.params.clone()));
                    }
                    if eff.name == "標準描画" || eff.name == "拡張描画" {
                        if let Some(b) = eff.text("blend") {
                            *blend_modes.entry(b.to_string()).or_insert(0) += 1;
                        }
                    }
                }
                if obj.effects.iter().any(|e| e.name == "カメラ制御" || matches!(e.kind, EffectKind::Camera)) {
                    camera_objects.push((li, obj.frame_start..obj.frame_end, obj.effects.clone()));
                }
            }
        }

        println!("Effect counts:");
        for (name, count) in &effect_counts {
            println!("  {:30}: {}", name, count);
        }

        println!("\nBlend modes:");
        for (bm, count) in &blend_modes {
            println!("  {:15}: {}", bm, count);
        }

        println!("\nObjects with non-zero blend mode:");
        for (li, layer) in doc.scene.layers.iter().enumerate() {
            for obj in &layer.objects {
                let draw = obj.effect(&EffectKind::StandardDraw).or_else(|| obj.effect(&EffectKind::ExtendedDraw));
                if let Some(d) = draw {
                    if let Some(b) = d.text("blend") {
                        if b != "0" {
                            let src_name = obj.source().map(|s| s.name.clone()).unwrap_or_default();
                            println!("  layer {}, range {}..{}, src='{}', blend={}, cam_target={}, effs={:?}",
                                li, obj.frame_start, obj.frame_end, src_name, b, obj.camera_target,
                                obj.effects.iter().map(|e| &e.name).collect::<Vec<_>>());
                        }
                    }
                }
            }
        }

        println!("\nShapes with line_width > 0 (total {}):", shapes_with_line_width.len());
        for (li, range, name, lw) in shapes_with_line_width.iter().take(20) {
            println!("  layer {}, range {:?}, name {:?}, line_width {}", li, range, name, lw);
        }

        println!("\nGlow-like effects (total {}):", glow_objects.len());
        for (li, range, name, params) in glow_objects.iter().take(20) {
            println!("  layer {}, range {:?}, name '{}', params={:?}", li, range, name, params);
        }

        println!("=== SCRIPT NAMES IN TARGET AUP ===");
        let mut script_names = std::collections::BTreeSet::new();
        let mut camera_effects = Vec::new();
        for layer in &doc.scene.layers {
            for obj in &layer.objects {
                for eff in &obj.effects {
                    if eff.name == "アニメーション効果" || eff.name == "カスタムオブジェクト" || eff.name == "カメラ効果" || eff.name == "スクリプト(カメラ制御)" {
                        let s_name = eff.text("name").unwrap_or_default();
                        let s_type = eff.text("type").unwrap_or_default();
                        let s_param = eff.text("param").unwrap_or_default();
                        script_names.insert(format!("{}: name='{}', type='{}', param='{}'", eff.name, s_name, s_type, s_param));
                    }
                    if eff.name == "カメラ効果" {
                        camera_effects.push(eff.clone());
                    }
                }
            }
        }
        for s in &script_names {
            println!("  {}", s);
        }
        println!("Camera effects detail count: {}", camera_effects.len());
        for ce in &camera_effects {
            println!("  ce params: {:?}", ce.params);
        }

        assert_eq!(doc.scene.layers.iter().map(|l| l.objects.len()).sum::<usize>(), 237, "全237個のオブジェクトが欠損なく読み込まれていること");
        assert!(!shapes_with_line_width.is_empty(), "枠線（ライン幅>0）図形が存在すること");
        assert!(!camera_objects.is_empty(), "カメラ制御オブジェクトが存在すること");
        assert!(effect_counts.get("グロー").copied().unwrap_or(0) >= 70, "70個以上のグローエフェクトが存在すること");

        // スクリプト名が正しく解決され、null終端ゴミが含まれていないことの検証
        let has_camera_shake = doc.scene.layers.iter().flat_map(|l| &l.objects).any(|obj| {
            obj.effects.iter().any(|eff| eff.name == "カメラ効果" && eff.text("name") == Some("手ぶれ"))
        });
        assert!(has_camera_shake, "組み込みカメラ効果 '手ぶれ' が正しく名前解決されていること");

        let has_circular_placement = doc.scene.layers.iter().flat_map(|l| &l.objects).any(|obj| {
            obj.effects.iter().any(|eff| eff.name == "アニメーション効果" && eff.text("name") == Some("円形配置"))
        });
        assert!(has_circular_placement, "組み込みアニメーション効果 '円形配置' が正しく名前解決されていること");
    }

    #[test]
    fn test_builtin_script_mapping_and_null_termination() {
        use crate::legacy_aup::*;

        // 1. 組み込みスクリプト名テーブルの検証
        assert_eq!(BUILTIN_ANM_NAMES[0], "震える");
        assert_eq!(BUILTIN_ANM_NAMES[1], "振り子");
        assert_eq!(BUILTIN_ANM_NAMES[14], "立方体(カメラ制御)");
        assert_eq!(BUILTIN_ANM_NAMES[23], "円形配置");
        assert_eq!(BUILTIN_CAM_NAMES[0], "手ぶれ");
        assert_eq!(BUILTIN_CAM_NAMES[1], "目標中心回転");
        assert_eq!(BUILTIN_CAM_NAMES[2], "目標サイズ固定視野角");
        assert_eq!(BUILTIN_OBJ_NAMES[4], "雲");
        assert_eq!(BUILTIN_OBJ_NAMES[13], "フレア");
        assert_eq!(BUILTIN_SCN_NAMES[0], "発光");

        // 2. decode_sjis_cstring の null 終端テスト（末尾ゴミの除去）
        let mut dirty_buf = [0u8; 64];
        let hello_sjis = encoding_rs::SHIFT_JIS.encode("テスト効果").0;
        dirty_buf[..hello_sjis.len()].copy_from_slice(&hello_sjis);
        dirty_buf[hello_sjis.len()] = 0; // null 終端
        // null の後にゴミデータを混入
        dirty_buf[hello_sjis.len() + 1] = b'G';
        dirty_buf[hello_sjis.len() + 2] = b'A';
        dirty_buf[hello_sjis.len() + 3] = b'R';
        dirty_buf[hello_sjis.len() + 4] = b'B';

        let decoded = decode_sjis_cstring(&dirty_buf);
        assert_eq!(decoded, "テスト効果", "null終端以降のゴミ文字列が含まれないこと");
    }

    #[test]
    fn test_parse_real_goot_time_pf_aup() {
        let path = r"C:\Users\user\Desktop\goot time pf.aup";
        if let Ok(bytes) = std::fs::read(path) {
            match legacy_aup::parse_legacy_aup(&bytes) {
                Ok(doc) => {
                    assert!(
                        doc.scene.object_count() >= 3000,
                        "3000個以上の正規オブジェクトが正常に読み込めること: got {}",
                        doc.scene.object_count()
                    );
                    assert_eq!(doc.scene.layers.len(), 100, "100レイヤーが存在すること");
                    assert!(doc.scene.settings.length < 100_000, "巨大オーバーフローせず安全なフレーム長であること");
                }
                Err(e) => panic!("goot time pf.aup の解析でエラーが発生しました: {:?}", e),
            }
        }
    }
}
