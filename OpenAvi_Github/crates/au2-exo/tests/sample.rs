//! サンプル `.exo`（Shift_JIS）を用いた結合テスト。

use au2_core::{Easing, EffectKind, MoveKind, ObjectCategory, ParamValue};
use au2_exo::{parse_exo_bytes, parse_exo_str, write_exo_bytes, write_exo_string, TextEncoding};

const SAMPLE: &[u8] = include_bytes!("../../../samples/sample.exo");

#[test]
fn parse_sample_header_and_objects() {
    let doc = parse_exo_bytes(SAMPLE).expect("parse");
    assert_eq!(doc.encoding, TextEncoding::ShiftJis);
    assert!(doc.warnings.is_empty(), "warnings: {:?}", doc.warnings);

    let s = &doc.scene;
    assert_eq!((s.settings.width, s.settings.height), (1280, 720));
    assert_eq!(s.settings.frame_rate.as_f64(), 30.0);
    assert_eq!(s.settings.length, 300);
    // [1] と [2] は chain で 1 オブジェクトに統合される → 8 区間 / 7 オブジェクト
    assert_eq!(s.object_count(), 7);
    assert_eq!(s.used_layer_count(), 7);
}

#[test]
fn chain_becomes_midpoint_keyframes() {
    let doc = parse_exo_bytes(SAMPLE).unwrap();
    let obj = &doc.scene.layers[1].objects[0];
    assert_eq!((obj.frame_start, obj.frame_end), (0, 179));
    assert_eq!(obj.midpoints, vec![90]);

    let draw = obj.effect(&EffectKind::StandardDraw).unwrap();
    let x = draw.track("X").unwrap();
    let frames: Vec<_> = x.keyframes.iter().map(|k| (k.frame, k.value)).collect();
    assert_eq!(frames, vec![(0, -450.0), (90, 0.0), (179, 450.0)]);
    assert_eq!(x.interpolation.as_ref().unwrap().kind, MoveKind::Linear);
    assert_eq!(x.evaluate(45), -225.0);

    let y = draw.track("Y").unwrap();
    assert_eq!(y.interpolation.as_ref().unwrap().kind, MoveKind::Curve);
    // 固定値トラック
    assert!(!draw.track("Z").unwrap().is_animated());
}

#[test]
fn text_and_settings() {
    let doc = parse_exo_bytes(SAMPLE).unwrap();
    let text_obj = &doc.scene.layers[3].objects[0];
    let src = text_obj.source().unwrap();
    assert_eq!(src.kind, EffectKind::Text);
    assert_eq!(src.text("text"), Some("OpenAvi\r\nへようこそ！"));
    assert_eq!(src.text("font"), Some("MS UI Gothic"));
    assert_eq!(text_obj.display_name(), "OpenAvi");
    assert_eq!(text_obj.effects[1].name, "フェード");

    let y = text_obj.effects[2].track("Y").unwrap();
    assert_eq!(y.interpolation.as_ref().unwrap().easing, Easing::EaseOut);

    let audio = &doc.scene.layers[5].objects[0];
    assert!(audio.audio);
    assert_eq!(audio.category(), ObjectCategory::Audio);
    assert_eq!(audio.display_name(), "bgm.wav");

    let filter = &doc.scene.layers[6].objects[0];
    assert_eq!(filter.category(), ObjectCategory::Filter);
    assert!(matches!(filter.source().unwrap().param("ｺﾝﾄﾗｽﾄ"), Some(ParamValue::Track(_))));
}

#[test]
fn roundtrip_is_stable() {
    let doc = parse_exo_bytes(SAMPLE).unwrap();
    let first = write_exo_string(&doc.scene);
    let reparsed = parse_exo_str(&first).unwrap();
    assert!(reparsed.warnings.is_empty(), "{:?}", reparsed.warnings);
    let second = write_exo_string(&reparsed.scene);
    assert_eq!(first, second);
    assert!(first.contains("chain=1"));
    assert!(first.contains("Y=-220.0,220.0,97"));

    // Shift_JIS で書き出して読み戻せること
    let bytes = write_exo_bytes(&doc.scene, TextEncoding::ShiftJis);
    let back = parse_exo_bytes(&bytes).unwrap();
    assert_eq!(back.encoding, TextEncoding::ShiftJis);
    assert_eq!(back.scene.object_count(), 7);
}

#[test]
fn missing_header_is_error() {
    assert!(parse_exo_str("[0]\nstart=1\nend=2\nlayer=1\n").is_err());
}
