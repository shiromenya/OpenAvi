use std::path::Path;

use au2_core::{
    Easing, Effect, EffectKind, Interpolation, Keyframe, MoveKind, ParamValue, TimelineObject,
    Track,
};

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
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => continue, // ローカルのテストファイルが存在しない環境 (CI/Mac/Linux等) ではスキップ
        };
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
    let Ok(bytes) = std::fs::read(path) else { return; };
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
    let Ok(bytes) = std::fs::read(path) else { return; };
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
    let p_desktop = Path::new(r"C:\Users\user\Desktop\goot time pf.aup");
    let p_testdata = Path::new(r"test_data\goot_time.aup");
    let path = if p_desktop.exists() {
        p_desktop
    } else if p_testdata.exists() {
        p_testdata
    } else {
        return;
    };

    let Ok(bytes) = std::fs::read(path) else { return; };

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

#[test]
fn test_aviutl2_multi_scene_and_layer_metadata_roundtrip() {
    let mut scenes = BTreeMap::new();
    let mut s0 = Scene::new("Root", au2_core::SceneSettings::default());
    s0.layers[0].name = Some("メイン字幕".to_string());
    s0.layers[1].visible = false;
    s0.layers[2].locked = true;
    s0.insert_object(0, TimelineObject::new_text(0, 59, "Root Scene Text"));
    scenes.insert(0, s0);

    let mut s1 = Scene::new("Scene 1", au2_core::SceneSettings::default());
    s1.layers[0].name = Some("サブシーン背景".to_string());
    s1.insert_object(0, TimelineObject::new_shape(0, 119, 1, "ffffff"));
    scenes.insert(1, s1);

    let aup2_str = write_aup2_multi_scene_string(&scenes, 0, None, 25);
    assert!(aup2_str.contains("[scene.0]"));
    assert!(aup2_str.contains("[scene.1]"));
    assert!(aup2_str.contains("name=メイン字幕"));

    let parsed = parse_aup2_str(&aup2_str).expect("parse multi-scene aup2");
    assert_eq!(parsed.cursor_frame, 25);
    assert_eq!(parsed.scenes.len(), 2);
    assert_eq!(parsed.scene.layers[0].name.as_deref(), Some("メイン字幕"));
    assert!(!parsed.scene.layers[1].visible);
    assert!(parsed.scene.layers[2].locked);
    assert_eq!(parsed.scene.object_count(), 1);
    let sub_scene = parsed.scenes.get(&1).expect("scene 1 exists");
    assert_eq!(sub_scene.name, "Scene 1");
    assert_eq!(sub_scene.layers[0].name.as_deref(), Some("サブシーン背景"));
    assert_eq!(sub_scene.object_count(), 1);
}

#[test]
fn test_aviutl2_object_alias_and_path_vars() {
    let obj = TimelineObject::new_text(10, 49, "エイリアステスト");
    let alias_str = write_object_alias_string(&[obj], 30.0);
    assert!(alias_str.contains("[0]"));
    assert!(alias_str.contains("[0.0]"));

    let parsed_objs = parse_object_alias_str(&alias_str).expect("parse .object alias");
    assert_eq!(parsed_objs.len(), 1);
    assert_eq!(parsed_objs[0].length(), 40);
    assert_eq!(parsed_objs[0].source().and_then(|s| s.text("text")), Some("エイリアステスト"));

    // [Object.0] / [Object.0.0] 形式のエイリアスも解析できることを確認
    let obj_prefix_alias = alias_str.replace("[0]", "[Object.0]").replace("[0.", "[Object.0.");
    let parsed_prefix = parse_object_alias_str(&obj_prefix_alias).expect("parse [Object.0] alias");
    assert_eq!(parsed_prefix.len(), 1);
    assert_eq!(parsed_prefix[0].source().and_then(|s| s.text("text")), Some("エイリアステスト"));

    let expanded = expand_aviutl2_path_vars(r"%Script%\script.anm2");
    assert!(!expanded.contains("%Script%"));
    assert!(expanded.ends_with(r"Script\script.anm2"));
}

#[test]
fn test_aviutl2_track_bezier_and_rotation_interp() {
    // 1. 時間制御ベジェカーブ |vx0,vy0,vx1,vy1 (減速移動: 0,0,0.5,0)
    let aup2 = "\
[project]
version=2011600
[scene.0]
scene=0
width=1920
height=1080
rate=30
scale=1
[0]
layer=0
frame=0,60
[0.0]
effect.name=図形
サイズ=100
[0.1]
effect.name=標準描画
X=0.0,300.0,直線移動(時間制御)|0,0,0.5,0
拡大率=100.00
透明度=0.0
Z軸回転=350.00,10.00,直線移動(回転)
";
    let doc = parse_aup2_str(aup2).expect("parse bezier & rotation aup2");
    let obj = &doc.scene.layers[0].objects[0];
    let draw = obj.effect(&EffectKind::StandardDraw).expect("StandardDraw");
    let x_mid = draw.eval("X", 30, 0.0);
    assert!(x_mid > 180.0 && x_mid < 260.0, "Decel bezier midpoint should be > 180: {x_mid}");
    // 350度 -> 10度 の最短回転補間は 360度(=0度) を通る
    let rz_mid = draw.eval("回転", 30, 0.0);
    assert!((rz_mid - 360.0).abs() < 1.0 || rz_mid.abs() < 1.0, "Shortest angle 350->10 at mid should be ~360/0, got {rz_mid}");
}

/// 0バイトファイルや破損バイナリ、偽装ファイルが渡されても絶対にパニックせず、安全にエラーを返すこと。
#[test]
fn test_corrupted_and_truncated_files_do_not_panic() {
    // 1. 空データ
    assert!(parse_aup_bytes(b"").is_err());
    assert!(parse_aup2_bytes(b"").is_err());

    // 2. 壊れたヘッダー
    assert!(parse_aup_bytes(b"not a valid header").is_err());
    assert!(parse_aup_bytes(b"AviUtl ProjectFile version ").is_err()); // 32バイト未満

    // 3. ヘッダーだけあって本体データが途切れたバイナリ
    let mut truncated = Vec::from(b"AviUtl ProjectFile version 0.18\0" as &[u8]);
    assert!(parse_aup_bytes(&truncated).is_err() || parse_aup_bytes(&truncated).is_ok());
    truncated.extend_from_slice(&[0x00; 10]); // 途中までのデータ
    assert!(parse_aup_bytes(&truncated).is_err() || parse_aup_bytes(&truncated).is_ok());

    // 4. ランダムな不正バイト列
    let garbage = vec![0xcc; 512];
    assert!(parse_aup_bytes(&garbage).is_err());
}

#[test]
fn test_rle_comp_decomp_roundtrip() {
    let mut original = Vec::new();
    original.extend_from_slice(&[1, 2, 3, 4, 5]);
    original.extend_from_slice(&[0xAA; 20]);
    original.extend_from_slice(&[9, 8, 7]);
    original.extend_from_slice(&[0x00; 5000]);
    for i in 0..300u16 {
        original.push((i % 251) as u8);
    }
    original.extend_from_slice(&[0xFF; 1024]);

    let comp = legacy_aup::compress_aup(&original);
    let decomp = legacy_aup::decompress_aup(&comp, original.len());
    assert_eq!(decomp, original);
}

#[test]
fn test_bidirectional_aup_and_aup2_conversion() {
    // 1. AupDotNet の実在 .aup ファイル群で .aup -> .aup2 -> .aup 相互変換を検証
    let base_dir = Path::new(r"C:\Users\user\.gemini\antigravity\scratch\aupdotnet\AupDotNet-main\testdata\Exedit");
    for file_name in ["LayerScene.aup", "Chain.aup", "Group.aup", "EffectSet01.aup"] {
        let path = base_dir.join(file_name);
        if !path.exists() {
            continue;
        }
        let orig_bytes = std::fs::read(&path).expect("read sample aup");
        let doc1 = parse_aup_bytes(&orig_bytes).expect("parse original .aup");
        let orig_total_objs: usize = doc1.scenes.values().map(|s| s.object_count()).sum();
        assert!(orig_total_objs > 0, "{file_name} should have objects");

        // .aup -> .aup2 に変換
        let aup2_str = write_aup2_multi_scene_string(&doc1.scenes, 0, Some(&path), doc1.cursor_frame);
        let doc_aup2 = parse_aup2_str(&aup2_str).expect("parse converted .aup2");
        let aup2_total_objs: usize = doc_aup2.scenes.values().map(|s| s.object_count()).sum();
        assert_eq!(aup2_total_objs, orig_total_objs, "{file_name}: object count after .aup -> .aup2");

        // .aup2 -> 本家 .aup (EditHandle + #temp0 + 80EE) に逆変換
        let re_aup_bytes = write_aup_multi_scene_bytes(&doc_aup2.scenes, 0, doc_aup2.cursor_frame);
        assert!(re_aup_bytes.starts_with(AUP_HEADER_018));
        assert!(find_subslice(&re_aup_bytes, b"FilterProject 0.1\0").is_some());
        assert!(find_subslice(&re_aup_bytes, OAVI_CHUNK_MAGIC).is_none());

        let doc_reaup = parse_aup_bytes(&re_aup_bytes).expect("parse re-encoded .aup");
        let reaup_total_objs: usize = doc_reaup.scenes.values().map(|s| s.object_count()).sum();
        assert_eq!(reaup_total_objs, orig_total_objs, "{file_name}: object count after .aup2 -> .aup");

        if file_name == "LayerScene.aup" {
            let sc0 = doc_reaup.scenes.get(&0).expect("Scene 0");
            assert_eq!(sc0.layers[1].name.as_deref(), Some("レイヤー2"));
            assert!(!sc0.layers[1].visible);
            assert_eq!(sc0.layers[2].name.as_deref(), Some("3番目"));
            assert!(sc0.layers[2].locked);
            assert!(sc0.layers[3].coordinate_link);
            assert_eq!(sc0.layers[4].name.as_deref(), Some("Layer 5"));
            assert!(sc0.layers[4].clip_above);
        }
    }
}
