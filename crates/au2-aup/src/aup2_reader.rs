//! AviUtl2 (.aup2 / .object エイリアス) パーサー。

use std::collections::BTreeMap;
use std::path::Path;

use au2_core::{
    Easing, Effect, EffectKind, Frame, Interpolation, Keyframe, MoveKind, Param, ParamValue,
    Rational, Scene, SceneSettings, TimelineObject, Track,
};

use crate::aup2_writer::{
    blend_aup2_to_code, shape_aup2_name_to_code, text_align_aup2_to_code, text_deco_aup2_to_code,
    unescape_aup2_text,
};
use crate::{AupDocument, AupError, AUP2_DEFAULT_VERSION};

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

/// AviUtl2 のエイリアスファイル（`.object`）バイト列を解析して [`TimelineObject`] 一覧を返します。
pub fn parse_object_alias_bytes(bytes: &[u8]) -> Result<Vec<TimelineObject>, AupError> {
    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Ok(s) = std::str::from_utf8(bytes) {
        s.to_owned()
    } else {
        encoding_rs::SHIFT_JIS.decode(bytes).0.into_owned()
    };
    parse_object_alias_str(&text)
}

/// AviUtl2 のエイリアスファイル（`.object`）テキストを解析して [`TimelineObject`] 一覧を返します。
pub fn parse_object_alias_str(text: &str) -> Result<Vec<TimelineObject>, AupError> {
    let doc = parse_aup2_or_alias_str(text, true)?;
    let mut objs: Vec<TimelineObject> = doc
        .scene
        .layers
        .into_iter()
        .flat_map(|l| l.objects)
        .collect();
    if objs.is_empty() {
        return Err(AupError::InvalidHeader);
    }
    let min_start = objs.iter().map(|o| o.frame_start).min().unwrap_or(0);
    for o in &mut objs {
        o.frame_start -= min_start;
        o.frame_end -= min_start;
    }
    Ok(objs)
}

/// AviUtl2 のファイルパス変数（`%AppInstall%`, `%AppData%`, `%Plugin%`, `%Script%`, `%Figure%`, `%Transition%` 等）を展開します。
pub fn expand_aviutl2_path_vars(raw_path: &str) -> String {
    if !raw_path.contains('%') {
        return raw_path.to_string();
    }
    let app_install = [
        r"C:\Users\user\Desktop\aviutl2_v2.1.16",
        r"C:\Program Files\AviUtl2",
    ]
    .into_iter()
    .find(|p| Path::new(p).exists())
    .map(str::to_string)
    .or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.to_string_lossy().into_owned()))
    })
    .unwrap_or_else(|| ".".to_string());

    let app_data = [
        r"C:\ProgramData\aviutl2",
        &format!("{}\\data", app_install),
    ]
    .into_iter()
    .find(|p| Path::new(p).exists())
    .map(str::to_string)
    .unwrap_or_else(|| app_install.clone());

    let vars = [
        ("%AppInstall%", app_install.clone()),
        ("%AppData%", app_data.clone()),
        ("%Plugin%", format!("{}\\Plugin", app_data)),
        ("%Script%", format!("{}\\Script", app_data)),
        ("%Default%", format!("{}\\Default", app_data)),
        ("%Alias%", format!("{}\\Alias", app_data)),
        ("%Preset%", format!("{}\\Preset", app_data)),
        ("%Figure%", format!("{}\\Figure", app_data)),
        ("%Transition%", format!("{}\\Transition", app_data)),
        ("%Font%", format!("{}\\Font", app_data)),
        ("%Language%", format!("{}\\Language", app_data)),
        ("%Backup%", format!("{}\\Backup", app_data)),
        ("%Log%", format!("{}\\Log", app_data)),
        ("%Package%", format!("{}\\Package", app_data)),
        ("%Batch%", format!("{}\\Batch", app_data)),
    ];

    let mut out = raw_path.to_string();
    for (var, replacement) in vars {
        if out.to_ascii_lowercase().contains(&var.to_ascii_lowercase()) {
            // 大文字小文字を区別せず置換
            let lower_out = out.to_ascii_lowercase();
            let lower_var = var.to_ascii_lowercase();
            if let Some(pos) = lower_out.find(&lower_var) {
                let mut next = String::with_capacity(out.len() + replacement.len());
                next.push_str(&out[..pos]);
                next.push_str(&replacement);
                next.push_str(&out[pos + var.len()..]);
                out = next;
            }
        }
    }
    out
}

/// `.aup2` テキストを解析して [`AupDocument`] を返します。
pub fn parse_aup2_str(text: &str) -> Result<AupDocument, AupError> {
    parse_aup2_or_alias_str(text, false)
}

fn parse_aup2_or_alias_str(text: &str, allow_alias: bool) -> Result<AupDocument, AupError> {
    let mut warnings = Vec::new();
    let sections = parse_ini_sections(text, &mut warnings);

    let has_project = sections.iter().any(|s| s.name.eq_ignore_ascii_case("project"));
    let has_scene = sections
        .iter()
        .any(|s| s.name.starts_with("scene.") || s.name.eq_ignore_ascii_case("scene"));
    let has_object_sec = sections.iter().any(|s| {
        s.name.eq_ignore_ascii_case("Object")
            || s.name.starts_with("Object.")
            || s.name.parse::<u32>().is_ok()
            || s.name
                .split_once('.')
                .is_some_and(|(a, b)| a.parse::<u32>().is_ok() && b.parse::<u32>().is_ok())
    });

    if !has_project && !has_scene && !(allow_alias && has_object_sec) {
        return Err(AupError::InvalidHeader);
    }

    let mut version = AUP2_DEFAULT_VERSION.to_string();
    let mut display_scene: u32 = 0;
    if let Some(proj) = sections.iter().find(|s| s.name.eq_ignore_ascii_case("project")) {
        if let Some(v) = proj.get("version") {
            version = v.trim().to_string();
        }
        if let Some(ds) = proj.get("display.scene").and_then(|v| v.trim().parse::<u32>().ok()) {
            display_scene = ds;
        }
    }

    let mut scenes: BTreeMap<u32, Scene> = BTreeMap::new();
    let mut scene_cursors: BTreeMap<u32, Frame> = BTreeMap::new();

    for sc in sections
        .iter()
        .filter(|s| s.name.starts_with("scene.") || s.name.eq_ignore_ascii_case("scene"))
    {
        let scene_id = sc
            .get("scene")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .or_else(|| sc.name.strip_prefix("scene.").and_then(|v| v.trim().parse::<u32>().ok()))
            .unwrap_or(0);
        let mut settings = SceneSettings::default();
        let mut scene_name = if scene_id == 0 {
            "Root".to_string()
        } else {
            format!("Scene {scene_id}")
        };
        let mut cur_frame: Frame = 0;
        if let Some(n) = sc.get("name") {
            let trimmed = n.trim();
            if !trimmed.is_empty() {
                scene_name = trimmed.to_string();
            }
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
                        cur_frame = x;
                    }
                }
                _ => {}
            }
        }
        settings.frame_rate = Rational::new(rate.max(1), scale.max(1));
        scenes.insert(scene_id, Scene::new(scene_name, settings));
        scene_cursors.insert(scene_id, cur_frame);
    }

    if scenes.is_empty() {
        scenes.insert(0, Scene::new("Root", SceneSettings::default()));
    }

    // [layer.N] セクションの解析（レイヤー名・表示状態・ロック状態）
    for ls in sections.iter().filter(|s| s.name.starts_with("layer.")) {
        let scene_id = ls
            .get("scene")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let Some(layer_idx) = ls.get("layer").and_then(|v| v.trim().parse::<usize>().ok()) else {
            continue;
        };
        let target_scene = scenes
            .entry(scene_id)
            .or_insert_with(|| Scene::new(format!("Scene {scene_id}"), SceneSettings::default()));
        if layer_idx >= target_scene.layers.len() {
            target_scene
                .layers
                .resize_with(layer_idx + 1, au2_core::Layer::default);
        }
        let layer = &mut target_scene.layers[layer_idx];
        if let Some(name) = ls.get("name") {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                layer.name = Some(trimmed.to_string());
            }
        }
        if ls.get("hide").is_some_and(|v| v.trim() == "1")
            || ls.get("state").is_some_and(|v| v.trim().parse::<u32>().unwrap_or(0) & 1 != 0)
        {
            layer.visible = false;
        }
        if ls.get("lock").is_some_and(|v| v.trim() == "1")
            || ls.get("state").is_some_and(|v| v.trim().parse::<u32>().unwrap_or(0) & 2 != 0)
        {
            layer.locked = true;
        }
        if ls.get("link").is_some_and(|v| v.trim() == "1")
            || ls.get("state").is_some_and(|v| v.trim().parse::<u32>().unwrap_or(0) & 0x10 != 0)
        {
            layer.coordinate_link = true;
        }
        if ls.get("clip").is_some_and(|v| v.trim() == "1")
            || ls.get("clipping").is_some_and(|v| v.trim() == "1")
            || ls.get("state").is_some_and(|v| v.trim().parse::<u32>().unwrap_or(0) & 0x20 != 0)
        {
            layer.clip_above = true;
        }
    }

    // オブジェクトセクション [N] / [Object] / [Object.N] とエフェクトセクション [N.M] / [Object.M] / [Object.N.M] を収集
    let has_two_level_object_prefix = sections.iter().any(|s| {
        s.name
            .strip_prefix("Object.")
            .or_else(|| s.name.strip_prefix("object."))
            .and_then(|rest| rest.split_once('.'))
            .is_some_and(|(a, b)| a.parse::<u32>().is_ok() && b.parse::<u32>().is_ok())
    });
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
        let parsed = if s.name.eq_ignore_ascii_case("Object") {
            Some((0, None))
        } else if let Some(rest) = s
            .name
            .strip_prefix("Object.")
            .or_else(|| s.name.strip_prefix("object."))
        {
            if let Some((a, b)) = rest.split_once('.') {
                a.parse::<u32>()
                    .ok()
                    .zip(b.parse::<u32>().ok())
                    .map(|(n, m)| (n, Some(m)))
            } else if has_two_level_object_prefix {
                rest.parse::<u32>().ok().map(|n| (n, None))
            } else {
                rest.parse::<u32>().ok().map(|m| (0, Some(m)))
            }
        } else {
            match s.name.split_once('.') {
                None => s.name.parse::<u32>().ok().map(|n| (n, None)),
                Some((a, b)) => a
                    .parse::<u32>()
                    .ok()
                    .zip(b.parse::<u32>().ok())
                    .map(|(n, m)| (n, Some(m))),
            }
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

    // エフェクトセクション [0.0], [0.1] のみで [0] ヘッダが省略されている .object ファイルにも対応
    if headers.is_empty() && allow_alias && !effects.is_empty() {
        for (&n, _) in &effects {
            if let Some(first_sec) = sections.first() {
                headers.insert(n, first_sec);
            }
        }
    }

    // chain=1 による中間点連結および本家 .aup2 の frame=start,mid,...,end の両方に対応
    let mut pending: Option<Vec<Aup2Segment>> = None;
    for (n, h) in headers {
        let seg = parse_aup2_segment(h, effects.remove(&n).unwrap_or_default());
        if seg.chain {
            if let Some(p) = pending.as_mut() {
                if let Some(last) = p.last() {
                    if last.scene_id == seg.scene_id && last.layer == seg.layer && last.end + 1 == seg.start {
                        p.push(seg);
                        continue;
                    }
                }
            }
        }
        if let Some(p) = pending.take() {
            let sid = p[0].scene_id;
            let target_scene = scenes
                .entry(sid)
                .or_insert_with(|| Scene::new(format!("Scene {sid}"), SceneSettings::default()));
            finalize_aup2_segments(p, target_scene);
        }
        pending = Some(vec![seg]);
    }
    if let Some(p) = pending.take() {
        let sid = p[0].scene_id;
        let target_scene = scenes
            .entry(sid)
            .or_insert_with(|| Scene::new(format!("Scene {sid}"), SceneSettings::default()));
        finalize_aup2_segments(p, target_scene);
    }

    // 各シーン長がオブジェクト終端より短い場合は自動拡張
    for sc in scenes.values_mut() {
        let content_end = sc.content_end();
        if content_end > sc.settings.length {
            sc.settings.length = content_end;
        }
    }

    let active_scene_id = if scenes.contains_key(&display_scene) {
        display_scene
    } else {
        *scenes.keys().next().unwrap_or(&0)
    };
    let scene = scenes
        .get(&active_scene_id)
        .cloned()
        .unwrap_or_default();
    let cursor_frame = scene_cursors.get(&active_scene_id).copied().unwrap_or(0);

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
    scene_id: u32,
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
    let scene_id = h
        .get("scene")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(0);
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

    if !h.name.contains('.') {
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
        scene_id,
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

/// AviUtl2 `effect.conf` および `Default.aul2` に基づくエフェクト名の正規化。
/// AviUtl2 では各スクリプト効果が直接 `effect.name=震える` や `effect.name=円形配置` として記述されるため、
/// 既存のアニメーション効果・カスタムオブジェクト・カメラ効果・シーンチェンジとも相互運用できるようマッピングします。
fn normalize_aup2_effect_name(name: &str) -> (String, Option<String>) {
    match name {
        "映像再生" => ("標準描画".into(), None),
        "音声再生" | "基本出力" => ("標準再生".into(), None),
        "時間制御(オブジェクト)" => ("時間制御".into(), None),
        "グループ制御(音声)" => ("グループ制御".into(), None),
        "パーティクル出力(カメラ制御)" => ("パーティクル出力".into(), None),
        "ドロップシャドウ" => ("シャドー".into(), None),
        // AviUtl2 標準アニメーション効果 (script.anm2 全26種)
        "震える"
        | "振り子"
        | "弾む"
        | "座標の拡大縮小(個別オブジェクト)"
        | "画面外から登場"
        | "ランダム方向から登場"
        | "拡大縮小して登場"
        | "ランダム間隔で落ちながら登場"
        | "弾んで登場"
        | "広がって登場"
        | "起き上がって登場"
        | "何処からともなく登場"
        | "反復移動"
        | "座標の回転(個別オブジェクト)"
        | "立方体(カメラ制御)"
        | "球体(カメラ制御)"
        | "砕け散る"
        | "点滅"
        | "点滅して登場"
        | "簡易変形"
        | "簡易変形(カメラ制御)"
        | "リール回転"
        | "万華鏡"
        | "円形配置"
        | "ランダム配置"
        | "起き上がり" => ("アニメーション効果".into(), Some(name.to_string())),
        // AviUtl2 標準カメラ効果 (script.cam2)
        "手ぶれ" | "目標中心回転" | "目標サイズ固定視野角" => {
            ("カメラ効果".into(), Some(name.to_string()))
        }
        // AviUtl2 標準カスタムオブジェクト (script.obj2)
        "集中線"
        | "走査線"
        | "カウンター"
        | "レンズフレア"
        | "雲"
        | "星"
        | "雪"
        | "雨"
        | "ランダム小物配置(カメラ制御)"
        | "ライン(移動軌跡)"
        | "扇型"
        | "多角形"
        | "周辺ボケ光量"
        | "フレア"
        | "水面" => ("カスタムオブジェクト".into(), Some(name.to_string())),
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
        EffectKind::GroupControl | EffectKind::Camera | EffectKind::TimeControl => {
            if key == "対象レイヤー数" {
                return "range".into();
            }
        }
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
            (_, "色" | "光色" | "影色" | "縁色" | "ライン色" | "基準色") => {
                return "color".into();
            }
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
        (_, "file" | "ファイル") => expand_aviutl2_path_vars(&unescape_aup2_text(raw_val)),
        _ => unescape_aup2_text(raw_val),
    }
}

fn is_text_setting_key(key: &str) -> bool {
    if key.starts_with("psd_") {
        return true;
    }
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
            | "光色"
            | "影色"
            | "縁色"
            | "文字色"
            | "影・縁色"
            | "文字装飾"
            | "文字揃え"
            | "図形の種類"
            | "合成モード"
            | "name"
            | "param"
            | "filter"
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

    // パターン A: 本家 .aup2 形式 `v0,v1,...,vK,移動モード名[,フラグ整数]` (parts.len() >= 3)
    if parts.len() >= 3 {
        let last_token = parts[parts.len() - 1].trim();
        let last_u32 = last_token.parse::<u32>().ok();
        let (val_slice, mode_token, flag_num) = if parts.len() >= 4
            && last_u32.is_some()
            && parts[parts.len() - 2].trim().parse::<f64>().is_err()
        {
            (&parts[..parts.len() - 2], parts[parts.len() - 2].trim(), last_u32)
        } else if last_u32.is_none() && !last_token.contains('@') {
            (&parts[..parts.len() - 1], last_token, Some(0u32))
        } else {
            (&[][..], "", None)
        };
        let values: Option<Vec<f64>> = (!val_slice.is_empty())
            .then(|| {
                val_slice
                    .iter()
                    .map(|s| s.trim().parse::<f64>().ok().filter(|v| v.is_finite()))
                    .collect()
            })
            .flatten();
        if let (Some(vals), Some(flag_num)) = (values, flag_num) {
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
    if segments.is_empty() {
        return;
    }
    let first = &segments[0];
    let Some(last) = segments.last() else {
        return;
    };
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