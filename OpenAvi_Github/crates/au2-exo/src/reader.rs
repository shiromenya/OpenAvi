//! `.exo` → データモデル のデシリアライズ。

use std::collections::BTreeMap;

use au2_core::{
    Easing, Effect, EffectKind, Interpolation, Keyframe, MoveKind, Param, ParamValue, Rational, Scene,
    SceneSettings, TimelineObject, Track, MAX_LAYERS,
};

use crate::encoding::{self, decode_text_hex, TextEncoding};
use crate::error::ExoError;
use crate::ini::{self, RawSection};

/// 加速フラグ（移動方法番号のビット）
pub(crate) const FLAG_ACCEL: u32 = 0x40;
/// 減速フラグ（移動方法番号のビット）
pub(crate) const FLAG_DECEL: u32 = 0x20;

/// 読み込み結果。
#[derive(Debug, Clone)]
pub struct ExoDocument {
    pub scene: Scene,
    /// 元ファイルの文字コード（書き出し時に同じ文字コードを使うため）
    pub encoding: TextEncoding,
    /// 致命的ではない問題（未知の行・重なり・連結できない chain など）
    pub warnings: Vec<String>,
}

/// バイト列（Shift_JIS / UTF-8 自動判別）から読み込みます。
pub fn parse_exo_bytes(bytes: &[u8]) -> Result<ExoDocument, ExoError> {
    let (text, enc) = encoding::decode(bytes);
    let mut doc = parse_exo_str(&text)?;
    doc.encoding = enc;
    Ok(doc)
}

/// デコード済み文字列から読み込みます。
pub fn parse_exo_str(text: &str) -> Result<ExoDocument, ExoError> {
    let mut warnings = Vec::new();
    let sections = ini::parse(text, &mut warnings);

    let header = sections.iter().find(|s| s.name.eq_ignore_ascii_case("exedit")).ok_or(ExoError::MissingHeader)?;
    let mut scene = Scene::new("Root", parse_settings(header, &mut warnings));

    // セクションを [N] と [N.M] に振り分ける
    let mut headers: BTreeMap<u32, &RawSection> = BTreeMap::new();
    let mut effects: BTreeMap<u32, Vec<(u32, &RawSection)>> = BTreeMap::new();
    for s in &sections {
        if std::ptr::eq(s, header) {
            continue;
        }
        let parsed = match s.name.split_once('.') {
            None => s.name.parse::<u32>().ok().map(|n| (n, None)),
            Some((a, b)) => a.parse::<u32>().ok().zip(b.parse::<u32>().ok()).map(|(n, m)| (n, Some(m))),
        };
        match parsed {
            Some((n, None)) => {
                if headers.insert(n, s).is_some() {
                    warnings.push(format!("{} 行目: オブジェクト [{n}] が重複しています（後勝ち）", s.line));
                }
            }
            Some((n, Some(m))) => effects.entry(n).or_default().push((m, s)),
            None => warnings.push(format!("{} 行目: 未知のセクション [{}] を無視しました", s.line, s.name)),
        }
    }
    for n in effects.keys().filter(|n| !headers.contains_key(n)) {
        warnings.push(format!("オブジェクト [{n}] の見出しが無いためエフェクトを無視しました"));
    }

    // 区間（セグメント）を chain で連結しながらオブジェクト化する
    let mut pending: Option<Vec<Segment>> = None;
    for (n, h) in headers {
        let seg = parse_segment(n, h, effects.remove(&n).unwrap_or_default(), &mut warnings)?;
        if seg.chain {
            if let Some(p) = pending.as_mut() {
                let last = p.last().unwrap();
                if last.layer == seg.layer && last.end + 1 == seg.start {
                    p.push(seg);
                    continue;
                }
            }
            warnings.push(format!("{} 行目: [{n}] の chain=1 を直前の区間に連結できないため独立オブジェクトとして扱います", seg.line));
        }
        if let Some(p) = pending.take() {
            finalize(p, &mut scene, &mut warnings);
        }
        pending = Some(vec![seg]);
    }
    if let Some(p) = pending.take() {
        finalize(p, &mut scene, &mut warnings);
    }

    Ok(ExoDocument { scene, encoding: TextEncoding::default(), warnings })
}

fn parse_settings(h: &RawSection, warnings: &mut Vec<String>) -> SceneSettings {
    let mut s = SceneSettings::default();
    let (mut rate, mut scale) = (s.frame_rate.num, s.frame_rate.den);
    for (k, v) in &h.entries {
        let target = match k.as_str() {
            "width" => &mut s.width,
            "height" => &mut s.height,
            "rate" => &mut rate,
            "scale" => &mut scale,
            "length" => &mut s.length,
            "audio_rate" => &mut s.audio_rate,
            "audio_ch" => &mut s.audio_channels,
            _ => continue,
        };
        match v.trim().parse::<u32>() {
            Ok(x) => *target = x,
            Err(_) => warnings.push(format!("[exedit] {k}={v} を解釈できません（既定値を使用）")),
        }
    }
    s.frame_rate = Rational::new(rate.max(1), scale.max(1));
    s
}

/// `.exo` 上の 1 区間（`[N]` + `[N.M]...`）。
struct Segment {
    line: usize,
    /// 1 始まり
    start: u32,
    end: u32,
    layer: u32,
    chain: bool,
    group: Option<u32>,
    clipping: bool,
    camera: bool,
    audio: bool,
    extra: Vec<(String, String)>,
    effects: Vec<RawEffect>,
}

struct RawEffect {
    name: String,
    enabled: bool,
    entries: Vec<(String, String)>,
}

impl RawEffect {
    fn get(&self, key: &str) -> Option<&str> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

fn required_u32(h: &RawSection, key: &'static str) -> Result<u32, ExoError> {
    let v = h.get(key).ok_or_else(|| ExoError::MissingKey { line: h.line, section: h.name.clone(), key })?;
    v.trim().parse().map_err(|_| ExoError::InvalidValue { line: h.line, key: key.into(), value: v.into() })
}

fn parse_segment(
    n: u32,
    h: &RawSection,
    mut effs: Vec<(u32, &RawSection)>,
    warnings: &mut Vec<String>,
) -> Result<Segment, ExoError> {
    let start = required_u32(h, "start")?.max(1);
    let end = required_u32(h, "end")?.max(start);
    let layer = required_u32(h, "layer")?.max(1);

    let flag = |k: &str| h.get(k).map(|v| v.trim() == "1").unwrap_or(false);
    let extra = h
        .entries
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "start" | "end" | "layer" | "chain" | "group" | "clipping" | "camera" | "audio"))
        .cloned()
        .collect();

    effs.sort_by_key(|(m, _)| *m);
    let effects = effs
        .into_iter()
        .map(|(m, s)| {
            let name = s.get("_name").map(str::to_string).unwrap_or_else(|| {
                warnings.push(format!("{} 行目: [{n}.{m}] に _name がありません", s.line));
                "(不明なエフェクト)".to_string()
            });
            RawEffect {
                name,
                enabled: s.get("_disable").map(|v| v.trim() != "1").unwrap_or(true),
                entries: s.entries.iter().filter(|(k, _)| k != "_name" && k != "_disable").cloned().collect(),
            }
        })
        .collect();

    Ok(Segment {
        line: h.line,
        start,
        end,
        layer,
        chain: flag("chain"),
        group: h.get("group").and_then(|v| v.trim().parse().ok()),
        clipping: flag("clipping"),
        camera: flag("camera"),
        audio: flag("audio"),
        extra,
        effects,
    })
}

/// トラック値 1 区間分の解析結果。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TrackSeg {
    Static { value: f64, precision: u8 },
    Animated { start: f64, end: f64, precision: u8, code: u32, script: Option<String>, param: Option<String> },
}

impl TrackSeg {
    fn start(&self) -> f64 {
        match self {
            TrackSeg::Static { value, .. } => *value,
            TrackSeg::Animated { start, .. } => *start,
        }
    }
    fn end(&self) -> f64 {
        match self {
            TrackSeg::Static { value, .. } => *value,
            TrackSeg::Animated { end, .. } => *end,
        }
    }
}

fn precision_of(s: &str) -> u8 {
    s.trim().split_once('.').map(|(_, f)| f.len().min(6) as u8).unwrap_or(0)
}

/// `値` または `開始値,終了値,移動方法[@スクリプト名][,付加パラメータ...]` を解析します。
pub(crate) fn parse_track_value(raw: &str) -> Option<TrackSeg> {
    let parts: Vec<&str> = raw.split(',').collect();
    let start: f64 = parts[0].trim().parse().ok().filter(|v: &f64| v.is_finite())?;
    let precision = precision_of(parts[0]);
    match parts.len() {
        1 => Some(TrackSeg::Static { value: start, precision }),
        2 => None,
        _ => {
            let end: f64 = parts[1].trim().parse().ok().filter(|v: &f64| v.is_finite())?;
            let (code_str, script) = match parts[2].split_once('@') {
                Some((c, s)) => (c, Some(s.to_string())),
                None => (parts[2], None),
            };
            let code = code_str.trim().parse().ok()?;
            let param = (parts.len() > 3).then(|| parts[3..].join(","));
            Some(TrackSeg::Animated { start, end, precision: precision.max(precision_of(parts[1])), code, script, param })
        }
    }
}

/// 移動方法番号を補間設定に変換します。
pub(crate) fn interpolation_from_code(code: u32, script: Option<String>, param: Option<String>) -> Interpolation {
    let base = code & !(FLAG_ACCEL | FLAG_DECEL);
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
    Interpolation {
        kind,
        easing: Easing::from_flags(code & FLAG_ACCEL != 0, code & FLAG_DECEL != 0),
        param,
        legacy_code: Some(base),
    }
}

/// 数値トラックとして扱わない設定キー（`file`, `color`, `font`, `blend` など英小文字・`_` 始まり）。
pub(crate) fn is_setting_key(key: &str) -> bool {
    key.chars().next().map_or(true, |c| c.is_ascii_lowercase() || c == '_')
}

fn finalize(segments: Vec<Segment>, scene: &mut Scene, warnings: &mut Vec<String>) {
    let first = &segments[0];
    let last = segments.last().unwrap();
    let mut obj = TimelineObject::new(first.start - 1, last.end - 1);
    let base = obj.frame_start;
    let last_rel = obj.frame_end - base;
    obj.midpoints = segments[1..].iter().map(|s| s.start - 1 - base).collect();
    obj.group = first.group;
    obj.clipping = first.clipping;
    obj.camera_target = first.camera;
    obj.audio = first.audio;
    obj.extra = first.extra.clone();

    for (ei, raw) in first.effects.iter().enumerate() {
        let mut eff = Effect::new(raw.name.clone());
        eff.enabled = raw.enabled;
        for (key, value) in &raw.entries {
            let pv = if eff.kind == EffectKind::Text && key == "text" {
                ParamValue::Text(decode_text_hex(value).unwrap_or_else(|| value.clone()))
            } else if is_setting_key(key) {
                ParamValue::Text(value.clone())
            } else {
                match parse_track_value(value) {
                    None => ParamValue::Text(value.clone()),
                    Some(TrackSeg::Static { value, precision }) => ParamValue::Track(Track::constant(value, precision)),
                    Some(TrackSeg::Animated { precision, code, script, param, .. }) => {
                        let mut keyframes = Vec::with_capacity(segments.len() + 1);
                        let mut last_end = 0.0;
                        for seg in &segments {
                            let seg_val = seg
                                .effects
                                .get(ei)
                                .and_then(|e| e.get(key))
                                .and_then(parse_track_value);
                            let Some(sv) = seg_val else {
                                warnings.push(format!("{} 行目: 中間点区間に `{key}` がありません", seg.line));
                                continue;
                            };
                            keyframes.push(Keyframe { frame: seg.start - 1 - base, value: sv.start() });
                            last_end = sv.end();
                        }
                        match keyframes.last_mut() {
                            Some(k) if k.frame >= last_rel => k.value = last_end,
                            _ => keyframes.push(Keyframe { frame: last_rel, value: last_end }),
                        }
                        ParamValue::Track(Track {
                            keyframes,
                            interpolation: Some(interpolation_from_code(code, script, param)),
                            precision,
                        })
                    }
                }
            };
            eff.params.push(Param { key: key.clone(), value: pv });
        }
        obj.effects.push(eff);
    }

    let layer = (first.layer - 1) as usize;
    if layer >= MAX_LAYERS {
        warnings.push(format!("{} 行目: レイヤー {} は上限 ({MAX_LAYERS}) を超えています", first.line, first.layer));
    }
    if !scene.insert_object(layer, obj) {
        warnings.push(format!("{} 行目: レイヤー {} でオブジェクトが重なっています", first.line, first.layer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_value_forms() {
        assert_eq!(parse_track_value("12.50"), Some(TrackSeg::Static { value: 12.5, precision: 2 }));
        assert_eq!(
            parse_track_value("0.0,100.0,1"),
            Some(TrackSeg::Animated { start: 0.0, end: 100.0, precision: 1, code: 1, script: None, param: None })
        );
        assert_eq!(
            parse_track_value("0.0,100.0,15@揺れ,30"),
            Some(TrackSeg::Animated {
                start: 0.0,
                end: 100.0,
                precision: 1,
                code: 15,
                script: Some("揺れ".into()),
                param: Some("30".into())
            })
        );
        assert_eq!(parse_track_value("ffffff"), None);
        assert_eq!(parse_track_value("1,2"), None);
    }

    #[test]
    fn easing_flags() {
        let i = interpolation_from_code(1 | FLAG_ACCEL | FLAG_DECEL, None, None);
        assert_eq!(i.kind, MoveKind::Linear);
        assert_eq!(i.easing, Easing::EaseInOut);
    }
}
