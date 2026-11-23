//! 初代 AviUtl (.aup) および拡張編集 (exedit.auf) バイナリのパーサー。

use au2_core::{Rational, Scene, SceneSettings};
use au2_exo::parse_exo_str;

use crate::{AupDocument, AupError, AUP_HEADER_PREFIX};

use super::rle::{clean_file_path, decode_sjis_cstring, decompress_aup, get_i16, get_i32};
use super::tables::{
    get_checkbox_name, get_trackbar_name, get_trackbar_scale, BUILTIN_ANM_NAMES, BUILTIN_CAM_NAMES,
    BUILTIN_OBJ_NAMES, BUILTIN_SCN_NAMES, BUILTIN_SCN_TYPES,
};

struct RawEffectType {
    name: String,
    trackbar_num: usize,
    checkbox_num: usize,
    ext_size: usize,
}

struct RawTrackbar {
    current: i32,
    next: i32,
    transition_type: u32,
    flag: u32,
    param: i32,
}

fn trackbar_to_exo_string(tb: &RawTrackbar, scale: i32, script_name: Option<&str>) -> String {
    let cur_str = match scale {
        100 => format!("{:.2}", tb.current as f64 / 100.0),
        10 => format!("{:.1}", tb.current as f64 / 10.0),
        _ => format!("{}", tb.current),
    };
    if tb.transition_type == 0 {
        return cur_str;
    }
    let next_str = match scale {
        100 => format!("{:.2}", tb.next as f64 / 100.0),
        10 => format!("{:.1}", tb.next as f64 / 10.0),
        _ => format!("{}", tb.next),
    };

    let trans_num = (tb.flag | tb.transition_type) as i32;
    let mut transition = format!(",{}", trans_num);
    if tb.transition_type == 0xF {
        if let Some(s) = script_name {
            transition.push('@');
            transition.push_str(s);
        }
    }
    let param_str = if tb.param != 0 {
        format!(",{}", tb.param)
    } else {
        String::new()
    };
    format!("{},{}{}{}", cur_str, next_str, transition, param_str)
}

struct ObjMeta {
    scene: u32,
    layer: u32,
    start_f: u32,
    end_f: u32,
    group: u32,
    flag: u32,
    is_chain: bool,
    cursor: usize,
}


pub fn parse_legacy_aup(bytes: &[u8]) -> Result<AupDocument, AupError> {
    if bytes.len() < 32 || !bytes.starts_with(AUP_HEADER_PREFIX) {
        return Err(AupError::InvalidHeader);
    }

    let header_str = String::from_utf8_lossy(&bytes[..32.min(bytes.len())]);
    let version = header_str
        .trim_end_matches('\0')
        .strip_prefix("AviUtl ProjectFile version ")
        .unwrap_or("0.18")
        .to_string();

    let mut warnings = Vec::new();

    // 1. AviUtl 本体の EditHandle から解像度・フレームレート等のメタデータを抽出
    let mut aup_w = 1280u32;
    let mut aup_h = 720u32;
    let mut aup_rate = 30u32;
    let mut aup_scale = 1u32;
    let mut aup_audio_rate = 44100u32;
    let mut aup_audio_ch = 2u32;
    let mut aup_len = 1800u32;
    let mut aup_cursor = 0u32;

    let header_size = 32;
    if let Some(edit_handle_size) = get_i32(bytes, header_size) {
        if edit_handle_size == 0x4c09e8 && bytes.len() >= header_size + 8 + 520 {
            let uncompressed_size = 0x20c;
            let comp_data_start = header_size + 8 + 520;
            let decomp_size = 0x4c09e8 - uncompressed_size;
            let decomp_handle = decompress_aup(&bytes[comp_data_start..], decomp_size);
            if decomp_handle.len() >= 0x470 - uncompressed_size {
                let base = uncompressed_size;
                let w = get_i32(&decomp_handle, 0x310 - base).unwrap_or(0);
                let h = get_i32(&decomp_handle, 0x314 - base).unwrap_or(0);
                let fn_num = get_i32(&decomp_handle, 0x318 - base).unwrap_or(0);
                let resized_w = get_i32(&decomp_handle, 0x328 - base).unwrap_or(0);
                let resized_h = get_i32(&decomp_handle, 0x32c - base).unwrap_or(0);
                let cur_f = get_i32(&decomp_handle, 0x330 - base).unwrap_or(0);
                let ch = get_i16(&decomp_handle, 0x3fa - base).unwrap_or(2);
                let ar = get_i32(&decomp_handle, 0x3fc - base).unwrap_or(44100);
                let vs = get_i32(&decomp_handle, 0x468 - base).unwrap_or(1);
                let vr = get_i32(&decomp_handle, 0x46c - base).unwrap_or(30);

                let final_w = if resized_w > 0 { resized_w as u32 } else if w > 0 { w as u32 } else { 1280 };
                let final_h = if resized_h > 0 { resized_h as u32 } else if h > 0 { h as u32 } else { 720 };
                if (16..=16384).contains(&final_w) && (16..=16384).contains(&final_h) {
                    aup_w = final_w;
                    aup_h = final_h;
                }
                if fn_num > 0 && fn_num < 10_000_000 {
                    aup_len = fn_num as u32;
                }
                if cur_f >= 0 {
                    aup_cursor = cur_f as u32;
                }
                if vr > 0 && vs > 0 {
                    aup_rate = vr as u32;
                    aup_scale = vs as u32;
                }
                if ar > 0 {
                    aup_audio_rate = ar as u32;
                }
                if ch > 0 {
                    aup_audio_ch = ch as u32;
                }
            }
        }
    }

    // AUP 生データからメディアファイルパスを事前抽出（本体で開かれた動画・音声のフォールバック用）
    let legacy_media_paths = crate::extract_media_paths_from_binary(bytes);
    let legacy_audio_paths: Vec<String> = legacy_media_paths
        .iter()
        .filter(|p| {
            let l = p.to_ascii_lowercase();
            l.ends_with(".wav")
                || l.ends_with(".mp3")
                || l.ends_with(".m4a")
                || l.ends_with(".aac")
                || l.ends_with(".ogg")
                || l.ends_with(".flac")
                || l.ends_with(".wma")
        })
        .cloned()
        .collect();
    let legacy_video_paths: Vec<String> = legacy_media_paths
        .iter()
        .filter(|p| {
            let l = p.to_ascii_lowercase();
            l.ends_with(".mp4")
                || l.ends_with(".avi")
                || l.ends_with(".mov")
                || l.ends_with(".mkv")
                || l.ends_with(".wmv")
                || l.ends_with(".webm")
        })
        .cloned()
        .collect();

    // 2. 拡張編集 (FilterProject) チャンクの検出と抽出
    let fp_magic = b"FilterProject 0.1\0";
    let mut exedit_chunk = None;

    let mut search_pos = 0;
    while let Some(idx) = bytes[search_pos..].windows(fp_magic.len()).position(|w| w == fp_magic) {
        let abs_pos = search_pos + idx;
        let nl_pos = abs_pos + fp_magic.len();
        if nl_pos + 4 <= bytes.len() {
            let nl = u32::from_le_bytes(bytes[nl_pos..nl_pos + 4].try_into().unwrap()) as usize;
            let n_pos = nl_pos + 4;
            if n_pos + nl <= bytes.len() {
                let (f_name, _, _) = encoding_rs::SHIFT_JIS.decode(&bytes[n_pos..n_pos + nl]);
                let f_name_clean = f_name.trim_matches('\0');
                if f_name_clean == "拡張編集" {
                    let d_len_pos = n_pos + nl;
                    if d_len_pos + 4 <= bytes.len() {
                        let decomp_sz = u32::from_le_bytes(bytes[d_len_pos..d_len_pos + 4].try_into().unwrap()) as usize;
                        let comp_start = d_len_pos + 4;
                        if comp_start < bytes.len() {
                            exedit_chunk = Some((comp_start, decomp_sz));
                            break;
                        }
                    }
                }
            }
        }
        search_pos = abs_pos + fp_magic.len();
    }

    if let Some((comp_start, decomp_sz)) = exedit_chunk {
        let decomp = decompress_aup(&bytes[comp_start..], decomp_sz);
        if decomp.len() >= 0x100 && &decomp[0..4] == b"80EE" {
            let effect_type_num = u32::from_le_bytes(decomp[4..8].try_into().unwrap()) as usize;
            let object_num = u32::from_le_bytes(decomp[8..12].try_into().unwrap()) as usize;
            let exedit_version = u32::from_le_bytes(decomp[0x2C..0x30].try_into().unwrap());
            let editing_scene = u32::from_le_bytes(decomp[0x64..0x68].try_into().unwrap());
            let scene_num = u32::from_le_bytes(decomp[0x68..0x6c].try_into().unwrap()) as usize;
            let layer_num = u32::from_le_bytes(decomp[0x6c..0x70].try_into().unwrap()) as usize;
            let scene_size = u32::from_le_bytes(decomp[0x70..0x74].try_into().unwrap()) as usize;
            let layer_size = u32::from_le_bytes(decomp[0x74..0x78].try_into().unwrap()) as usize;
            let trackbar_script_num = u32::from_le_bytes(decomp[0x7c..0x80].try_into().unwrap()) as usize;

            let hdr_base_size = u32::from_le_bytes(decomp[0x1C..0x20].try_into().unwrap()) as usize;
            let is_pre_092 = (exedit_version < 0x2300 && layer_num == 0 && scene_num == 0)
                || hdr_base_size == 0x500
                || (layer_size == 0 && scene_size == 0 && layer_num == 0 && scene_num == 0 && trackbar_script_num == 0);
            let base_size = if (0x400..=0x1000).contains(&hdr_base_size) {
                hdr_base_size
            } else if is_pre_092 {
                0x500
            } else {
                0x5c8
            };
            // ExEdit 0.90〜0.91系 (base_size == 0x500) と 0.92系 (base_size == 0x5c8) のフィールドオフセット
            // 0.90〜0.91系 (0x500):
            //   0x004..0x008: layer_index           (4 bytes)
            //   0x0F8..0x1F8: track_value_left[64]  (256 bytes)
            //   0x1F8..0x2F8: track_value_right[64] (256 bytes)
            //   0x2F8..0x3F8: track_mode[64]        (256 bytes)
            //   0x3F8..0x4B8: check_value[48]       (192 bytes)
            //   0x4B8..0x4BC: exdata_offset         (4 bytes)
            //   0x4BC..0x4C0: group_belong          (4 bytes)
            //   0x4C0..0x500: track_param[16]       (64 bytes)
            // 0.92系 (0x5C8):
            //   0x0F8..0x1F8: track_value_left[64]  (256 bytes)
            //   0x1F8..0x2F8: track_value_right[64] (256 bytes)
            //   0x2F8..0x3F8: track_mode[64]        (256 bytes)
            //   0x3F8..0x4B8: check_value[48]       (192 bytes)
            //   0x4B8..0x4BC: exdata_offset         (4 bytes)
            //   0x4BC..0x4C0: group_belong          (4 bytes)
            //   0x4C0..0x5C0: track_param[64]       (256 bytes)
            //   0x5C0..0x5C4: layer_set             (4 bytes)
            //   0x5C4..0x5C8: scene_set             (4 bytes)
            let tb_left_off: usize = 0x0F8;
            let tb_right_off: usize = 0x1F8;
            let tb_mode_off: usize = 0x2F8;
            let cb_off: usize = 0x3F8;
            let tb_param_off: usize = 0x4C0;
            let max_tracks: usize = 64;
            let (layer_off, scene_off) = (base_size - 8, base_size - 4);
            let param_end = if is_pre_092 { base_size } else { layer_off };
            let param_count: usize = param_end.saturating_sub(tb_param_off) / 4;

            let eff_layer_size = if layer_size > 0 { layer_size } else { 76 };
            let eff_scene_size = if scene_size > 0 { scene_size } else { 220 };

            // 0.90〜0.91系は固定ヘッダー長 0x474 の直後から EffectTypes が開始。
            // 0.92系は 0x100 の後に Layer[], Scene[], TrackbarScript[] が続いて EffectTypes が開始。
            let mut layers_info: std::collections::HashMap<(u32, usize), (Option<String>, bool, bool, bool, bool)> =
                std::collections::HashMap::new();
            let mut cursor = if is_pre_092 {
                0x474
            } else {
                let mut c = 0x100;
                for i in 0..layer_num {
                    if c + eff_layer_size <= decomp.len() && eff_layer_size >= 16 {
                        let l_data = &decomp[c..c + eff_layer_size];
                        let raw_sc = u32::from_le_bytes(l_data[0..4].try_into().unwrap());
                        let raw_li = u32::from_le_bytes(l_data[4..8].try_into().unwrap()) as usize;
                        let flag = u32::from_le_bytes(l_data[8..12].try_into().unwrap());
                        let name_end = eff_layer_size.min(76);
                        let raw_name = decode_sjis_cstring(&l_data[12..name_end]);
                        let name = if raw_name.is_empty() { None } else { Some(raw_name) };
                        let visible = (flag & 1) == 0;
                        let locked = (flag & 2) != 0;
                        let coordinate_link = (flag & 0x10) != 0;
                        let clip_above = (flag & 0x20) != 0;
                        if name.is_some() || !visible || locked || coordinate_link || clip_above {
                            let (sc_idx, l_idx) = if raw_sc < 100 && raw_li < 100 {
                                (raw_sc, raw_li)
                            } else {
                                ((i / 100) as u32, i % 100)
                            };
                            layers_info.insert((sc_idx, l_idx), (name, visible, locked, coordinate_link, clip_above));
                        }
                    }
                    c += eff_layer_size;
                }
                c
            };

            // Scenes
            let mut scenes_info = std::collections::HashMap::new();
            if !is_pre_092 {
                for i in 0..scene_num {
                    if cursor + eff_scene_size <= decomp.len() {
                        let s_data = &decomp[cursor..cursor + eff_scene_size];
                        let s_id0 = u32::from_le_bytes(s_data[0..4].try_into().unwrap());
                        let s_name = decode_sjis_cstring(&s_data[8..8 + 64]);
                        let w = u32::from_le_bytes(s_data[0x48..0x4c].try_into().unwrap());
                        let h = u32::from_le_bytes(s_data[0x4c..0x50].try_into().unwrap());
                        let max_f = u32::from_le_bytes(s_data[0x50..0x54].try_into().unwrap());
                        let actual_sc_id = if s_id0 < 100 && (i == 0 || s_id0 > 0) { s_id0 } else { i as u32 };
                        scenes_info.insert(actual_sc_id, (s_name, w, h, max_f));
                        cursor += eff_scene_size;
                    }
                }
            }

            // TrackbarScripts
            let mut scripts = Vec::new();
            if !is_pre_092 {
                for _ in 0..trackbar_script_num {
                    if cursor + 128 <= decomp.len() {
                        let sc_data = &decomp[cursor..cursor + 128];
                        scripts.push(decode_sjis_cstring(sc_data));
                        cursor += 128;
                    }
                }
            }

            // フォールバック: もし計算された cursor の位置が不正で 0x474 に有効な EffectType がある場合は 0x474 に補正
            if cursor + 112 <= decomp.len() {
                let test_name = decode_sjis_cstring(&decomp[cursor + 16..cursor + 16 + 32]);
                if test_name.is_empty() && decomp.len() > 0x474 + 112 {
                    let fallback_name = decode_sjis_cstring(&decomp[0x474 + 16..0x474 + 16 + 32]);
                    if !fallback_name.is_empty() {
                        cursor = 0x474;
                    }
                }
            }

            // EffectTypes
            let mut effect_types = Vec::new();
            for _ in 0..effect_type_num {
                if cursor + 112 <= decomp.len() {
                    let e_data = &decomp[cursor..cursor + 112];
                    let tb_num = u32::from_le_bytes(e_data[4..8].try_into().unwrap()) as usize;
                    let cb_num = u32::from_le_bytes(e_data[8..12].try_into().unwrap()) as usize;
                    let ext_sz = u32::from_le_bytes(e_data[12..16].try_into().unwrap()) as usize;
                    let e_name = decode_sjis_cstring(&e_data[16..16 + 96]);
                    effect_types.push(RawEffectType {
                        name: e_name,
                        trackbar_num: tb_num,
                        checkbox_num: cb_num,
                        ext_size: ext_sz,
                    });
                    cursor += 112;
                }
            }

            let proj_w = scenes_info.get(&0).and_then(|s| (s.1 > 0).then_some(s.1)).unwrap_or(aup_w);
            let proj_h = scenes_info.get(&0).and_then(|s| (s.2 > 0).then_some(s.2)).unwrap_or(aup_h);
            let proj_len = scenes_info
                .get(&0)
                .and_then(|s| (s.3 > 0 && s.3 < 10_000_000).then_some(s.3))
                .unwrap_or(if aup_len > 0 && aup_len < 10_000_000 { aup_len } else { 1800 });

            let mut exo = String::with_capacity(decomp.len() / 4);
            exo.push_str("[exedit]\r\n");
            exo.push_str(&format!(
                "width={}\r\nheight={}\r\nrate={}\r\nscale={}\r\nlength={}\r\naudio_rate={}\r\naudio_ch={}\r\n",
                proj_w, proj_h, aup_rate, aup_scale, proj_len, aup_audio_rate, aup_audio_ch
            ));

            let mut last_chain_group = 0xFFFF_FFFFu32;
            let mut all_objs = Vec::new();
            for _ in 0..object_num {
                if cursor + base_size > decomp.len() {
                    break;
                }
                let o_data = &decomp[cursor..];
                let flag = u32::from_le_bytes(o_data[0..4].try_into().unwrap());
                let start_f = u32::from_le_bytes(o_data[8..12].try_into().unwrap());
                let end_f = u32::from_le_bytes(o_data[12..16].try_into().unwrap());
                let chain_group = u32::from_le_bytes(o_data[0x50..0x54].try_into().unwrap());
                let ext_size_field = u32::from_le_bytes(o_data[0xF4..0xF8].try_into().unwrap()) as usize;
                let group = u32::from_le_bytes(o_data[0x4BC..0x4C0].try_into().unwrap());
                let layer = if is_pre_092 {
                    u32::from_le_bytes(o_data[0x04..0x08].try_into().unwrap())
                } else {
                    u32::from_le_bytes(o_data[layer_off..layer_off + 4].try_into().unwrap())
                };
                let scene = if is_pre_092 {
                    0
                } else {
                    u32::from_le_bytes(o_data[scene_off..scene_off + 4].try_into().unwrap())
                };

                let is_chain = chain_group != 0xFFFF_FFFF && chain_group == last_chain_group && ext_size_field == 0;
                last_chain_group = chain_group;

                let mut ext_sum = 0;
                let is_valid = (flag & 1) != 0
                    && flag != 0xFFFF_FFFF
                    && end_f != 0xFFFF_FFFF
                    && start_f <= end_f
                    && layer < 100;
                if is_valid && !is_chain {
                    for e in 0..12 {
                        let type_idx = i32::from_le_bytes(o_data[0x54 + e * 12..0x54 + e * 12 + 4].try_into().unwrap());
                        if type_idx < 0 || (type_idx as usize) >= effect_types.len() {
                            break;
                        }
                        ext_sum += effect_types[type_idx as usize].ext_size;
                    }
                }

                if is_valid {
                    all_objs.push(ObjMeta {
                        scene,
                        layer,
                        start_f,
                        end_f,
                        group,
                        flag,
                        is_chain,
                        cursor,
                    });
                }

                cursor += base_size + ext_sum;
            }

            // オブジェクトまたはレイヤー設定が存在する全シーン（および編集対象シーン）をそれぞれ .exo 経由で Scene に変換
            let mut target_scene_ids = std::collections::BTreeSet::new();
            target_scene_ids.insert(editing_scene);
            target_scene_ids.insert(0);
            for o in &all_objs {
                target_scene_ids.insert(o.scene);
            }
            for &(sc_id, _) in layers_info.keys() {
                target_scene_ids.insert(sc_id);
            }

            let mut parsed_scenes = std::collections::BTreeMap::new();
            let mut total_obj_count = 0usize;

            for sc_id in target_scene_ids {
                let scene_objs: Vec<_> = all_objs.iter().filter(|o| o.scene == sc_id).collect();
                let has_layer_info = layers_info.keys().any(|&(sid, _)| sid == sc_id);
                if scene_objs.is_empty() && !has_layer_info && sc_id != editing_scene && sc_id != 0 {
                    continue;
                }

                let (sc_name, sc_w, sc_h, sc_len) = scenes_info
                    .get(&sc_id)
                    .map(|(n, w, h, l)| {
                        let name = if n.is_empty() {
                            if sc_id == 0 { "Root".to_string() } else { format!("Scene {}", sc_id) }
                        } else {
                            n.clone()
                        };
                        let w = if *w > 0 { *w } else { proj_w };
                        let h = if *h > 0 { *h } else { proj_h };
                        let len = if *l > 0 && *l < 10_000_000 { *l } else { proj_len };
                        (name, w, h, len)
                    })
                    .unwrap_or_else(|| {
                        let name = if sc_id == 0 { "Root".to_string() } else { format!("Scene {}", sc_id) };
                        (name, proj_w, proj_h, proj_len)
                    });

                let mut exo = String::with_capacity(scene_objs.len() * 256 + 128);
                exo.push_str("[exedit]\r\n");
                exo.push_str(&format!(
                    "width={}\r\nheight={}\r\nrate={}\r\nscale={}\r\nlength={}\r\naudio_rate={}\r\naudio_ch={}\r\n",
                    sc_w, sc_h, aup_rate, aup_scale, sc_len, aup_audio_rate, aup_audio_ch
                ));

                for (out_obj_idx, obj) in scene_objs.iter().enumerate() {
                    let o_data = &decomp[obj.cursor..];
                    exo.push_str(&format!("[{}]\r\n", out_obj_idx));
                    exo.push_str(&format!(
                        "start={}\r\nend={}\r\nlayer={}\r\n",
                        obj.start_f.saturating_add(1),
                        obj.end_f.saturating_add(1),
                        obj.layer.saturating_add(1)
                    ));
                    if obj.group != 0 {
                        exo.push_str(&format!("group={}\r\n", obj.group));
                    }

                    let media = (obj.flag & 0x0001_0000) != 0;
                    let audio = (obj.flag & 0x0002_0000) != 0;
                    let media_filter = (obj.flag & 0x0004_0000) != 0;
                    let control = (obj.flag & 0x0008_0000) != 0;
                    if media || media_filter {
                        let clipping = (obj.flag & 0x0000_0100) != 0;
                        if clipping {
                            exo.push_str("clipping=1\r\n");
                        }
                        exo.push_str(&format!("overlay={}\r\n", if clipping { 0 } else { 1 }));
                    }
                    if (media && !audio) || (media_filter && control) {
                        let camera = (obj.flag & 0x0000_0200) != 0;
                        exo.push_str(&format!("camera={}\r\n", if camera { 1 } else { 0 }));
                    }
                    if audio {
                        exo.push_str("audio=1\r\n");
                    }
                    if obj.is_chain {
                        exo.push_str("chain=1\r\n");
                    }

                    // 各エフェクトの出力
                    let mut trackbar_count = 0;
                    let mut checkbox_count = 0;
                    for eff_idx in 0..12 {
                        let type_idx = i32::from_le_bytes(o_data[0x54 + eff_idx * 12..0x54 + eff_idx * 12 + 4].try_into().unwrap());
                        if type_idx < 0 || type_idx as usize >= effect_types.len() {
                            break;
                        }
                        let eff_type = &effect_types[type_idx as usize];
                        let raw_ext_offset = i32::from_le_bytes(o_data[0x54 + eff_idx * 12 + 8..0x54 + eff_idx * 12 + 12].try_into().unwrap());
                        let ext_offset = if raw_ext_offset >= 0 { raw_ext_offset as usize } else { 0 };
                        let ext_slice = if !obj.is_chain && eff_type.ext_size > 0 && raw_ext_offset >= 0 {
                            let avail = decomp.len().saturating_sub(obj.cursor);
                            let start = base_size.saturating_add(ext_offset);
                            let end = start.saturating_add(eff_type.ext_size);
                            if end <= avail && end <= o_data.len() {
                                &o_data[start..end]
                            } else {
                                &[]
                            }
                        } else {
                            &[]
                        };

                        exo.push_str(&format!("[{}.{}]\r\n", out_obj_idx, eff_idx));
                        exo.push_str(&format!("_name={}\r\n", eff_type.name));

                        // トラックバー
                        for tb_i in 0..eff_type.trackbar_num {
                            let idx = trackbar_count + tb_i;
                            if idx >= max_tracks {
                                break;
                            }
                            let cur = if tb_left_off + idx * 4 + 4 <= o_data.len() {
                                i32::from_le_bytes(o_data[tb_left_off + idx * 4..tb_left_off + idx * 4 + 4].try_into().unwrap())
                            } else {
                                0
                            };
                            let next = if tb_right_off + idx * 4 + 4 <= o_data.len() {
                                i32::from_le_bytes(o_data[tb_right_off + idx * 4..tb_right_off + idx * 4 + 4].try_into().unwrap())
                            } else {
                                cur
                            };
                            let trans = if idx < max_tracks && tb_mode_off + idx * 4 + 4 <= o_data.len() {
                                u32::from_le_bytes(o_data[tb_mode_off + idx * 4..tb_mode_off + idx * 4 + 4].try_into().unwrap())
                            } else {
                                0
                            };
                            let param = if idx < param_count && tb_param_off + idx * 4 + 4 <= o_data.len() {
                                i32::from_le_bytes(o_data[tb_param_off + idx * 4..tb_param_off + idx * 4 + 4].try_into().unwrap())
                            } else {
                                0
                            };

                            let sc_idx = (trans >> 16) as usize;
                            let sc_name = scripts.get(sc_idx).map(|s| s.as_str());
                            let tb = RawTrackbar {
                                current: cur,
                                next,
                                transition_type: trans & 0xF,
                                flag: trans & 0xF0,
                                param,
                            };
                            let scale = get_trackbar_scale(&eff_type.name, tb_i);
                            let tb_name = get_trackbar_name(&eff_type.name, tb_i);
                            let val_str = trackbar_to_exo_string(&tb, scale, sc_name);
                            exo.push_str(&format!("{}={}\r\n", tb_name, val_str));
                        }
                        trackbar_count += eff_type.trackbar_num;

                        // チェックボックス (48 個)
                        for cb_i in 0..eff_type.checkbox_num {
                            let idx = checkbox_count + cb_i;
                            if idx >= 48 || cb_off + idx * 4 + 4 > o_data.len() {
                                break;
                            }
                            let raw_val = i32::from_le_bytes(o_data[cb_off + idx * 4..cb_off + idx * 4 + 4].try_into().unwrap());
                            let val = if raw_val != 0 { 1 } else { 0 };
                            let cb_name = get_checkbox_name(&eff_type.name, cb_i);
                            if let Some(name) = cb_name {
                                exo.push_str(&format!("{}={}\r\n", name, val));
                            }
                        }
                        checkbox_count += eff_type.checkbox_num;

                        // 拡張データ
                        let mut has_file = false;
                        if !obj.is_chain && !ext_slice.is_empty() {
                            let prev_len = exo.len();
                            export_ext_data(&mut exo, &eff_type.name, ext_slice);
                            if exo[prev_len..].contains("file=") {
                                has_file = true;
                            }
                        }
                        // 拡張データ内に file= が存在しなかった場合（AviUtl本体側で読み込まれた音声・動画）
                        if !has_file && !obj.is_chain {
                            if eff_type.name == "音声ファイル" {
                                if let Some(audio_p) = legacy_audio_paths.first() {
                                    exo.push_str(&format!("file={}\r\n", audio_p));
                                }
                            } else if eff_type.name == "動画ファイル" {
                                if let Some(vid_p) = legacy_video_paths.first() {
                                    exo.push_str(&format!("file={}\r\n", vid_p));
                                }
                            }
                        }
                    }
                }

                if let Ok(doc) = parse_exo_str(&exo) {
                    let mut sc = doc.scene;
                    sc.name = sc_name;
                    for (&(sid, l_idx), (name, visible, locked, coord_link, clip_above)) in &layers_info {
                        if sid == sc_id && l_idx < 100 {
                            if l_idx >= sc.layers.len() {
                                sc.layers.resize_with(l_idx + 1, au2_core::Layer::default);
                            }
                            let layer = &mut sc.layers[l_idx];
                            layer.name = name.clone();
                            layer.visible = *visible;
                            layer.locked = *locked;
                            layer.coordinate_link = *coord_link;
                            layer.clip_above = *clip_above;
                        }
                    }
                    let mut max_end_frame = 0;
                    let mut obj_count = 0;
                    for l in &sc.layers {
                        for o in &l.objects {
                            obj_count += 1;
                            if o.frame_end > max_end_frame && o.frame_end < 10_000_000 {
                                max_end_frame = o.frame_end;
                            }
                        }
                    }
                    if sc.settings.length >= 10_000_000 || max_end_frame > sc.settings.length {
                        sc.settings.length = max_end_frame.max(1800) + 30;
                    }
                    total_obj_count += obj_count;
                    parsed_scenes.insert(sc_id, sc);
                }
            }

            if let Some(main_scene) = parsed_scenes
                .get(&editing_scene)
                .or_else(|| parsed_scenes.get(&0))
                .cloned()
            {
                warnings.push(format!(
                    "初代 AviUtl プロジェクトファイル（.aup）内の拡張編集データ（全 {} シーン / オブジェクト {} 個）を正常に復元しました",
                    parsed_scenes.len(),
                    total_obj_count
                ));
                return Ok(AupDocument {
                    version,
                    scene: main_scene,
                    scenes: parsed_scenes,
                    cursor_frame: aup_cursor,
                    warnings,
                });
            }
        }
    }

    // 拡張編集チャンクが存在しない、またはパースに失敗した場合のフォールバック
    let settings = SceneSettings {
        width: aup_w,
        height: aup_h,
        frame_rate: Rational::new(aup_rate, aup_scale),
        audio_rate: aup_audio_rate,
        audio_channels: aup_audio_ch,
        length: aup_len,
    };
    let scene = Scene::new("Root", settings);
    let mut scenes = std::collections::BTreeMap::new();
    scenes.insert(0, scene.clone());
    warnings.push("初代 AviUtl プロジェクトファイル (.aup) の基本設定を読み込みました".into());

    Ok(AupDocument {
        version,
        scene,
        scenes,
        cursor_frame: aup_cursor,
        warnings,
    })
}

fn export_ext_data(exo: &mut String, eff: &str, ext: &[u8]) {
    match eff {
        "動画ファイル" => {
            if let Some(path) = clean_file_path(&ext[..ext.len().min(260)]) {
                exo.push_str(&format!("file={}\r\n", path));
            }
        }
        "画像ファイル" => {
            let path = if ext.len() > 4 {
                clean_file_path(&ext[4..ext.len().min(264)])
                    .or_else(|| clean_file_path(&ext[..ext.len().min(260)]))
            } else {
                clean_file_path(&ext[..ext.len().min(260)])
            };
            if let Some(path) = path {
                exo.push_str(&format!("file={}\r\n", path));
            }
        }
        "音声ファイル" => {
            if let Some(path) = clean_file_path(&ext[..ext.len().min(260)]) {
                exo.push_str(&format!("file={}\r\n", path));
            }
        }
        "図形" => {
            if ext.len() >= 8 {
                let fig_type = i32::from_le_bytes(ext[0..4].try_into().unwrap());
                let r = ext[4];
                let g = ext[5];
                let b = ext[6];
                exo.push_str(&format!("type={}\r\ncolor={:02x}{:02x}{:02x}\r\n", fig_type, r, g, b));
                if ext.len() >= 8 + 64 {
                    let fig_name = decode_sjis_cstring(&ext[8..8 + 64]);
                    if !fig_name.is_empty() {
                        exo.push_str(&format!("name={}\r\n", fig_name));
                    }
                }
            }
        }
        "テキスト" => {
            if ext.len() >= 0x30 + 2048 {
                let text_type = ext[0];
                let autoadjust = ext[1];
                let soft = ext[2];
                let monospace = ext[3];
                let align = ext[4];
                let spacing_x = ext[5] as i8;
                let spacing_y = ext[6] as i8;
                let precision = ext[7];
                let r1 = ext[8];
                let g1 = ext[9];
                let b1 = ext[10];
                let r2 = ext[12];
                let g2 = ext[13];
                let b2 = ext[14];
                exo.push_str(&format!(
                    "type={}\r\nautoadjust={}\r\nsoft={}\r\nmonospace={}\r\nalign={}\r\nspacing_x={}\r\nspacing_y={}\r\nprecision={}\r\ncolor={:02x}{:02x}{:02x}\r\ncolor2={:02x}{:02x}{:02x}\r\n",
                    text_type, autoadjust, soft, monospace, align, spacing_x, spacing_y, precision,
                    r1, g1, b1, r2, g2, b2
                ));
                let font_name = decode_sjis_cstring(&ext[0x10..0x30]);
                if !font_name.is_empty() {
                    exo.push_str(&format!("font={}\r\n", font_name));
                }
                let text_slice = &ext[0x30..0x30 + 2048];
                let hex_str: String = text_slice.iter().map(|b| format!("{:02x}", b)).collect();
                exo.push_str(&format!("text={}\r\n", hex_str));
            }
        }
        "標準描画" | "拡張描画" => {
            if ext.len() >= 4 {
                let blend = i32::from_le_bytes(ext[0..4].try_into().unwrap());
                exo.push_str(&format!("blend={}\r\n", blend));
            }
        }
        "シーン" | "シーン(音声)" => {
            if ext.len() >= 4 {
                let scene = i32::from_le_bytes(ext[0..4].try_into().unwrap());
                exo.push_str(&format!("scene={}\r\n", scene));
            }
        }
        "カメラ制御" | "グループ制御" | "時間制御" => {
            if ext.len() >= 4 {
                let range = i32::from_le_bytes(ext[0..4].try_into().unwrap());
                exo.push_str(&format!("range={}\r\n", range));
            }
        }
        "グロー" | "発光" => {
            if ext.len() >= 4 {
                let no_color = ext[3];
                if no_color == 0 {
                    exo.push_str(&format!("color={:02x}{:02x}{:02x}\r\n", ext[0], ext[1], ext[2]));
                } else {
                    exo.push_str("color=\r\n");
                }
                exo.push_str(&format!("no_color={}\r\n", no_color));
            }
            if ext.len() >= 8 {
                let t = i32::from_le_bytes(ext[4..8].try_into().unwrap());
                exo.push_str(&format!("type={}\r\n", t));
            }
            if ext.len() >= 12 {
                let blend = i32::from_le_bytes(ext[8..12].try_into().unwrap());
                exo.push_str(&format!("blend={}\r\n", blend));
            }
        }
        "閃光" => {
            if ext.len() >= 4 {
                exo.push_str(&format!("color={:02x}{:02x}{:02x}\r\n", ext[0], ext[1], ext[2]));
            }
            if ext.len() >= 8 {
                let blend = i32::from_le_bytes(ext[4..8].try_into().unwrap());
                exo.push_str(&format!("blend={}\r\n", blend));
            }
        }
        "縁取り" | "シャドー" | "単色化" => {
            if ext.len() >= 4 {
                exo.push_str(&format!("color={:02x}{:02x}{:02x}\r\n", ext[0], ext[1], ext[2]));
            }
            if ext.len() > 4 {
                if let Some(path) = clean_file_path(&ext[4..ext.len().min(264)]) {
                    exo.push_str(&format!("file={}\r\n", path));
                }
            }
        }
        "グラデーション" => {
            if ext.len() >= 16 {
                let blend = i32::from_le_bytes(ext[0..4].try_into().unwrap());
                let no_color = ext[7];
                let no_color2 = ext[11];
                let grad_type = i32::from_le_bytes(ext[12..16].try_into().unwrap());
                exo.push_str(&format!("blend={}\r\n", blend));
                if no_color == 0 {
                    exo.push_str(&format!("color={:02x}{:02x}{:02x}\r\n", ext[4], ext[5], ext[6]));
                }
                exo.push_str(&format!("no_color={}\r\n", no_color));
                if no_color2 == 0 {
                    exo.push_str(&format!("color2={:02x}{:02x}{:02x}\r\n", ext[8], ext[9], ext[10]));
                }
                exo.push_str(&format!("no_color2={}\r\n", no_color2));
                exo.push_str(&format!("type={}\r\n", grad_type));
            }
        }
        "ワイプ" | "マスク" => {
            if ext.len() >= 4 {
                let t = i16::from_le_bytes(ext[0..2].try_into().unwrap());
                exo.push_str(&format!("type={}\r\n", t));
                if ext.len() > 4 {
                    let name = decode_sjis_cstring(&ext[4..ext.len().min(260)]);
                    if !name.is_empty() {
                        exo.push_str(&format!("name={}\r\n", name));
                    }
                }
            }
        }
        "ミラー" | "色ずれ" | "ノイズ" | "ルミナンスキー" | "インターレース解除" => {
            if ext.len() >= 4 {
                let t = i32::from_le_bytes(ext[0..4].try_into().unwrap());
                exo.push_str(&format!("type={}\r\n", t));
            }
        }
        "動画ファイル合成" | "画像ファイル合成" => {
            if ext.len() >= 4 {
                let mode = i32::from_le_bytes(ext[0..4].try_into().unwrap());
                exo.push_str(&format!("mode={}\r\n", mode));
            }
            if ext.len() > 4 {
                if let Some(path) = clean_file_path(&ext[4..ext.len().min(264)]) {
                    exo.push_str(&format!("file={}\r\n", path));
                }
            }
        }
        "音声波形表示" => {
            if ext.len() >= 4 {
                let t = i16::from_le_bytes(ext[0..2].try_into().unwrap());
                exo.push_str(&format!("type={}\r\n", t));
            }
            if ext.len() >= 8 {
                exo.push_str(&format!("color={:02x}{:02x}{:02x}\r\n", ext[4], ext[5], ext[6]));
            }
        }
        _ => {
            if ext.len() >= 0x104 + 256 {
                let script_id = i16::from_le_bytes(ext[0..2].try_into().unwrap());
                let dir = i16::from_le_bytes(ext[2..4].try_into().unwrap());
                let s_name = decode_sjis_cstring(&ext[4..4 + 256]);
                let param_str = decode_sjis_cstring(&ext[0x104..0x104 + 256]);
                // s_name が空の場合、組み込みスクリプト名テーブルから script_id に応じた名称を解決
                let resolved_name = if s_name.is_empty() && dir == 0 {
                    match eff {
                        "アニメーション効果" => BUILTIN_ANM_NAMES.get(script_id as usize).copied().unwrap_or(""),
                        "カスタムオブジェクト" => BUILTIN_OBJ_NAMES.get(script_id as usize).copied().unwrap_or(""),
                        "カメラ効果" | "スクリプト(カメラ制御)" => BUILTIN_CAM_NAMES.get(script_id as usize).copied().unwrap_or(""),
                        "シーンチェンジ" => BUILTIN_SCN_TYPES
                            .get(script_id as usize)
                            .or_else(|| BUILTIN_SCN_NAMES.get(script_id as usize))
                            .copied()
                            .unwrap_or("クロスフェード"),
                        _ => "",
                    }
                } else {
                    &s_name
                };
                exo.push_str(&format!("type={}\r\nfilter={}\r\nname={}\r\nparam={}\r\n", script_id, dir, resolved_name, param_str));
            }
        }
    }
}