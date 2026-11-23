//! AviUtl2 (.aup2 / .object エイリアス) シリアライザおよびコード変換ユーティリティ。

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use au2_core::{
    Easing, Effect, EffectKind, Frame, MoveKind, ParamValue, Scene, TimelineObject, Track,
};

use crate::AUP2_DEFAULT_VERSION;

/// シーンを本家 AviUtl2 の `.aup2` (UTF-8 INI 形式) テキストにシリアライズします。
pub fn write_aup2_string(scene: &Scene, file_path: Option<&Path>, cursor_frame: Frame) -> String {
    let mut scenes = BTreeMap::new();
    scenes.insert(0, scene.clone());
    write_aup2_multi_scene_string(&scenes, 0, file_path, cursor_frame)
}

/// マルチシーン (`scenes`) を本家 AviUtl2 の `.aup2` (UTF-8 INI 形式) テキストにシリアライズします。
pub fn write_aup2_multi_scene_string(
    scenes: &BTreeMap<u32, Scene>,
    display_scene: u32,
    file_path: Option<&Path>,
    cursor_frame: Frame,
) -> String {
    let mut out = String::new();
    let file_str = file_path
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let _ = write!(
        out,
        "[project]\r\nversion={AUP2_DEFAULT_VERSION}\r\nfile={file_str}\r\ndisplay.scene={display_scene}\r\npreview.scene={display_scene}\r\n"
    );

    let mut layer_sec_idx = 0usize;
    for (&scene_id, scene) in scenes {
        let s = &scene.settings;
        let cur = if scene_id == display_scene { cursor_frame } else { 0 };
        let _ = write!(
            out,
            "[scene.{scene_id}]\r\nscene={scene_id}\r\nname={}\r\nvideo.width={}\r\nvideo.height={}\r\nvideo.rate={}\r\nvideo.scale={}\r\naudio.rate={}\r\ncursor.frame={cur}\r\ncursor.layer=0\r\npreview.frame=0\r\ndisplay.frame=0\r\ndisplay.layer=0\r\ndisplay.zoom=10000\r\ndisplay.order=0\r\ndisplay.camera=\r\ndisplay.grid.x=16,-16\r\ndisplay.grid.y=16,-16\r\ndisplay.grid.width=200\r\ndisplay.grid.height=200\r\ndisplay.grid.step=200.000000\r\ndisplay.grid.range=10000.000000\r\ndisplay.tempo.bpm=120.000000\r\ndisplay.tempo.beat=4\r\ndisplay.tempo.offset=0.000000\r\n",
            scene.name,
            s.width,
            s.height,
            s.frame_rate.num,
            s.frame_rate.den,
            s.audio_rate,
        );

        for (li, layer) in scene.layers.iter().enumerate() {
            let has_name = layer.name.as_ref().is_some_and(|n| !n.is_empty());
            if has_name
                || !layer.visible
                || layer.locked
                || layer.coordinate_link
                || layer.clip_above
            {
                let _ = write!(out, "[layer.{layer_sec_idx}]\r\nlayer={li}\r\n");
                if scene_id > 0 || scenes.len() > 1 {
                    let _ = write!(out, "scene={scene_id}\r\n");
                }
                if let Some(name) = &layer.name {
                    if !name.is_empty() {
                        let _ = write!(out, "name={name}\r\n");
                    }
                }
                if !layer.visible {
                    out.push_str("hide=1\r\n");
                }
                if layer.locked {
                    out.push_str("lock=1\r\n");
                }
                if layer.coordinate_link {
                    out.push_str("link=1\r\n");
                }
                if layer.clip_above {
                    out.push_str("clip=1\r\n");
                }
                layer_sec_idx += 1;
            }
        }
    }

    let mut obj_global_idx = 0usize;
    for (&scene_id, scene) in scenes {
        let fps = scene.settings.frame_rate.as_f64().max(1.0);
        let mut objs: Vec<(usize, &TimelineObject)> = scene
            .layers
            .iter()
            .enumerate()
            .flat_map(|(li, l)| l.objects.iter().map(move |o| (li, o)))
            .collect();
        objs.sort_by_key(|(li, o)| (*li, o.frame_start));

        for (layer, obj) in objs {
            write_aup2_object_section(
                &mut out,
                obj_global_idx,
                if scene_id > 0 || scenes.len() > 1 {
                    Some(scene_id)
                } else {
                    None
                },
                layer,
                obj,
                fps,
            );
            obj_global_idx += 1;
        }
    }

    out
}

fn write_aup2_object_section(
    out: &mut String,
    index: usize,
    scene_id: Option<u32>,
    layer: usize,
    obj: &TimelineObject,
    fps: f64,
) {
    let keys = obj.key_frames();
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

    let _ = write!(out, "[{index}]\r\n");
    if let Some(sid) = scene_id {
        if sid > 0 {
            let _ = write!(out, "scene={sid}\r\n");
        }
    }
    let _ = write!(out, "layer={layer}\r\nframe={frame_list}\r\n");
    if let Some(g) = obj.group {
        if g > 0 {
            let _ = write!(out, "group={g}\r\n");
        }
    }
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
            out,
            index,
            ei,
            eff,
            &source_kind,
            &keys,
            obj_dur_sec,
        );
    }
}

/// シーンを `.aup2` (UTF-8 バイト列) に変換します。
pub fn write_aup2_bytes(scene: &Scene, file_path: Option<&Path>, cursor_frame: Frame) -> Vec<u8> {
    write_aup2_string(scene, file_path, cursor_frame).into_bytes()
}

/// オブジェクト群を本家 AviUtl2 のエイリアスファイル（`.object`）文字列にシリアライズします。
pub fn write_object_alias_string(objects: &[TimelineObject], fps: f64) -> String {
    let mut out = String::new();
    let fps = fps.max(1.0);
    for (idx, obj) in objects.iter().enumerate() {
        write_aup2_object_section(&mut out, idx, None, 0, obj, fps);
    }
    out
}

/// オブジェクト群をエイリアスファイル（`.object` UTF-8 バイト列）に変換します。
pub fn write_object_alias_bytes(objects: &[TimelineObject], fps: f64) -> Vec<u8> {
    write_object_alias_string(objects, fps).into_bytes()
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
    // アニメーション効果 / カスタムオブジェクト / カメラ効果 や動画/音声の再生エフェクト名を本家 AviUtl2 標準名に変換
    if matches!(
        eff.name.as_str(),
        "アニメーション効果" | "カスタムオブジェクト" | "カメラ効果"
    ) {
        let default_script = match eff.name.as_str() {
            "カスタムオブジェクト" => "集中線",
            "カメラ効果" => "手ぶれ",
            _ => "震える",
        };
        let raw_name = eff
            .text("name")
            .filter(|s| !s.is_empty())
            .unwrap_or(default_script);
        let base_name = raw_name.split('@').next().unwrap_or(raw_name);
        let aup2_eff_name = match base_name {
            "揺れ" | "震える" => "震える",
            "点滅" => "点滅",
            "回転" if eff.name == "アニメーション効果" => "座標の回転(個別オブジェクト)",
            other => other,
        };
        let _ = write!(out, "[{obj_idx}.{eff_idx}]\r\neffect.name={aup2_eff_name}\r\n");
        if !eff.enabled {
            out.push_str("effect.disable=1\r\n");
        }
        for p in &eff.params {
            if p.key == "name" {
                continue;
            }
            if matches!(p.key.as_str(), "type" | "filter" | "param") {
                if let ParamValue::Text(s) = &p.value {
                    if !s.is_empty() && s != "0" {
                        let _ = write!(out, "{}={}\r\n", p.key, escape_aup2_text(s));
                    }
                }
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
            let val = format_param_value(&p.value, keys);
            let _ = write!(out, "{out_key}={val}\r\n");
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

pub(crate) fn shape_aup2_name_to_code(name: &str) -> &str {
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

pub(crate) fn text_deco_aup2_to_code(name: &str) -> &str {
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

pub(crate) fn text_align_aup2_to_code(name: &str) -> &str {
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

pub(crate) fn blend_aup2_to_code(name: &str) -> &str {
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

pub(crate) fn unescape_aup2_text(s: &str) -> String {
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