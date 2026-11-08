//! データモデル → `.exo` のシリアライズ。

use std::fmt::Write as _;

use au2_core::{Easing, EffectKind, Frame, Interpolation, MoveKind, ParamValue, Scene, TimelineObject, Track};

use crate::encoding::{self, encode_text_hex, TextEncoding};
use crate::reader::{FLAG_ACCEL, FLAG_DECEL};

/// シーンを `.exo` テキストに変換します。
pub fn write_exo_string(scene: &Scene) -> String {
    let mut out = String::new();
    let s = &scene.settings;
    // 改行は AviUtl に合わせて CRLF
    let _ = write!(
        out,
        "[exedit]\r\nwidth={}\r\nheight={}\r\nrate={}\r\nscale={}\r\nlength={}\r\naudio_rate={}\r\naudio_ch={}\r\n",
        s.width, s.height, s.frame_rate.num, s.frame_rate.den, s.length, s.audio_rate, s.audio_channels
    );

    let mut objs: Vec<(usize, &TimelineObject)> =
        scene.layers.iter().enumerate().flat_map(|(li, l)| l.objects.iter().map(move |o| (li, o))).collect();
    objs.sort_by_key(|(li, o)| (o.frame_start, *li));

    let mut index = 0usize;
    for (layer, obj) in objs {
        let keys = obj.key_frames();
        let seg_count = obj.midpoints.len() + 1;
        for seg in 0..seg_count {
            let rel_start = keys[seg];
            let rel_end = if seg + 1 < seg_count { keys[seg + 1] - 1 } else { obj.length() - 1 };
            write_header(&mut out, index, obj, layer, rel_start, rel_end, seg > 0);
            for (ei, eff) in obj.effects.iter().enumerate() {
                let _ = write!(out, "[{index}.{ei}]\r\n_name={}\r\n", eff.name);
                if !eff.enabled {
                    out.push_str("_disable=1\r\n");
                }
                for p in &eff.params {
                    let value = match &p.value {
                        ParamValue::Text(t) if eff.kind == EffectKind::Text && p.key == "text" => encode_text_hex(t),
                        ParamValue::Text(t) => t.clone(),
                        ParamValue::Track(t) => format_track_segment(t, &keys, seg),
                    };
                    let _ = write!(out, "{}={}\r\n", p.key, value);
                }
            }
            index += 1;
        }
    }
    out
}

/// 指定した文字コードでバイト列に変換します。
pub fn write_exo_bytes(scene: &Scene, enc: TextEncoding) -> Vec<u8> {
    encoding::encode(&write_exo_string(scene), enc)
}

fn write_header(out: &mut String, index: usize, obj: &TimelineObject, layer: usize, rel_start: Frame, rel_end: Frame, chain: bool) {
    let start = obj.frame_start + rel_start + 1;
    let end = obj.frame_start + rel_end + 1;
    let _ = write!(out, "[{index}]\r\nstart={start}\r\nend={end}\r\nlayer={}\r\n", layer + 1);
    if let Some(g) = obj.group {
        let _ = write!(out, "group={g}\r\n");
    }
    for (k, v) in &obj.extra {
        let _ = write!(out, "{k}={v}\r\n");
    }
    if obj.clipping {
        out.push_str("clipping=1\r\n");
    }
    if obj.audio {
        out.push_str("audio=1\r\n");
    } else {
        let _ = write!(out, "camera={}\r\n", obj.camera_target as u8);
    }
    if chain {
        out.push_str("chain=1\r\n");
    }
}

fn value_at(track: &Track, frame: Frame) -> f64 {
    track.keyframes.iter().find(|k| k.frame == frame).map(|k| k.value).unwrap_or_else(|| track.evaluate(frame))
}

fn format_track_segment(track: &Track, keys: &[Frame], seg: usize) -> String {
    let p = track.precision as usize;
    match &track.interpolation {
        Some(interp) if track.keyframes.len() >= 2 => {
            let a = value_at(track, keys[seg]);
            let b = value_at(track, *keys.get(seg + 1).unwrap_or(keys.last().unwrap()));
            let mut s = format!("{a:.p$},{b:.p$},{}", mode_string(interp));
            if let Some(param) = &interp.param {
                s.push(',');
                s.push_str(param);
            }
            s
        }
        _ => format!("{:.p$}", track.start_value()),
    }
}

fn mode_string(interp: &Interpolation) -> String {
    let base = match &interp.kind {
        MoveKind::Linear => 1,
        MoveKind::Curve => 2,
        MoveKind::Instant => 3,
        MoveKind::IgnoreMidpoint => 4,
        MoveKind::MoveAmount => 5,
        MoveKind::Random => 6,
        MoveKind::Other(c) => *c,
        // TODO: スクリプト移動の番号体系は実ファイルで要検証。読込時の値があればそれを使う。
        MoveKind::Script(_) => interp.legacy_code.unwrap_or(15),
    };
    let flags = match interp.easing {
        Easing::None => 0,
        Easing::EaseIn => FLAG_ACCEL,
        Easing::EaseOut => FLAG_DECEL,
        Easing::EaseInOut => FLAG_ACCEL | FLAG_DECEL,
    };
    match &interp.kind {
        MoveKind::Script(name) => format!("{}@{name}", base | flags),
        _ => (base | flags).to_string(),
    }
}
