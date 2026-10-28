//! 初代 AviUtl (.aup) プロジェクトファイルおよび拡張編集 (exedit.auf)
//! バイナリデータのポータブル・パーサー。
//!
//! Windows のレガシー API や外部ランタイムに依存せず、AviUtl の公式バイナリ仕様および
//! 拡張編集の内部データ構造（RLE 圧縮、EditHandle、FilterProject、ExEditProject）を
//! 純粋な Rust のみで高速かつ安全に解析します。

use au2_core::{Rational, Scene, SceneSettings};
use au2_exo::parse_exo_str;

use crate::{AupDocument, AupError, AUP_HEADER_PREFIX};

/// 初代 AviUtl 拡張編集の組み込みアニメーション効果一覧 (exedit.anm)
pub const BUILTIN_ANM_NAMES: &[&str] = &[
    "震える",                              // 0
    "振り子",                              // 1
    "弾む",                                // 2
    "座標の拡大縮小(個別オブジェクト)",    // 3
    "画面外から登場",                      // 4
    "ランダム方向から登場",                // 5
    "拡大縮小して登場",                    // 6
    "ランダム間隔で落ちながら登場",        // 7
    "弾んで登場",                          // 8
    "広がって登場",                        // 9
    "起き上がって登場",                    // 10
    "何処からともなく登場",                // 11
    "反復移動",                            // 12
    "座標の回転(個別オブジェクト)",        // 13
    "立方体(カメラ制御)",                  // 14
    "球体(カメラ制御)",                    // 15
    "砕け散る",                            // 16
    "点滅",                                // 17
    "点滅して登場",                        // 18
    "簡易変形",                            // 19
    "簡易変形(カメラ制御)",                // 20
    "リール回転",                          // 21
    "万華鏡",                              // 22
    "円形配置",                            // 23
    "ランダム配置",                        // 24
];

/// 初代 AviUtl 拡張編集の組み込みカスタムオブジェクト一覧 (exedit.obj)
pub const BUILTIN_OBJ_NAMES: &[&str] = &[
    "集中線",                              // 0
    "走査線",                              // 1
    "カウンター",                          // 2
    "レンズフレア",                        // 3
    "雲",                                  // 4
    "星",                                  // 5
    "雪",                                  // 6
    "雨",                                  // 7
    "ランダム小物配置(カメラ制御)",        // 8
    "ライン(移動軌跡)",                    // 9
    "扇型",                                // 10
    "多角形",                              // 11
    "周辺ボケ光量",                        // 12
    "フレア",                              // 13
    "水面",                                // 14
];

/// 初代 AviUtl 拡張編集の組み込みカメラ効果一覧 (exedit.cam)
pub const BUILTIN_CAM_NAMES: &[&str] = &[
    "手ぶれ",                              // 0
    "目標中心回転",                        // 1
    "目標サイズ固定視野角",                // 2
];

/// 初代 AviUtl 拡張編集の組み込みシーンチェンジ一覧 (exedit.scn)
pub const BUILTIN_SCN_NAMES: &[&str] = &[
    "発光",                                // 0
    "レンズブラー",                        // 1
    "ドア",                                // 2
    "起き上がる",                          // 3
    "リール回転",                          // 4
    "図形ワイプ",                          // 5
    "図形で隠す",                          // 6
    "図形で隠す(放射)",                    // 7
    "砕け散る",                            // 8
    "ページめくり",                        // 9
];

/// 固定長バイナリバッファ内の先頭 null 終端（0x00）までのバイト列を Shift-JIS デコードします。
pub fn decode_sjis_cstring(slice: &[u8]) -> String {
    let null_idx = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(&slice[..null_idx]);
    cow.into_owned()
}

/// AviUtl 独自の RLE 伸張（AupUtil.Decomp 互換）
pub fn decompress_aup(input: &[u8], decomp_size: usize) -> Vec<u8> {
    let mut out = vec![0u8; decomp_size];
    let mut in_idx = 0;
    let mut out_idx = 0;

    while out_idx < decomp_size && in_idx < input.len() {
        let size1 = input[in_idx];
        in_idx += 1;
        if (size1 & 0x80) != 0 {
            let rep = if (size1 & 0x7F) != 0 {
                (size1 & 0x7F) as usize
            } else {
                if in_idx + 3 > input.len() {
                    break;
                }
                let s = input[in_idx] as usize
                    | ((input[in_idx + 1] as usize) << 8)
                    | ((input[in_idx + 2] as usize) << 16);
                in_idx += 3;
                s
            };
            if in_idx >= input.len() {
                break;
            }
            let b = input[in_idx];
            in_idx += 1;
            let end = (out_idx + rep).min(decomp_size);
            for i in out_idx..end {
                out[i] = b;
            }
            out_idx += rep;
        } else {
            let count = if size1 != 0 {
                size1 as usize
            } else {
                if in_idx + 3 > input.len() {
                    break;
                }
                let s = input[in_idx] as usize
                    | ((input[in_idx + 1] as usize) << 8)
                    | ((input[in_idx + 2] as usize) << 16);
                in_idx += 3;
                s
            };
            let end = (out_idx + count).min(decomp_size);
            for i in out_idx..end {
                if in_idx < input.len() {
                    out[i] = input[in_idx];
                    in_idx += 1;
                }
            }
            out_idx += count;
        }
    }
    out
}

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

/// 初代 `.aup` バイナリから拡張編集プロジェクトまたは本体データを完全解析します。
pub fn parse_legacy_aup(bytes: &[u8]) -> Result<AupDocument, AupError> {
    if !bytes.starts_with(AUP_HEADER_PREFIX) {
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
    let aup_len = 1800u32;

    let header_size = 32;
    if bytes.len() >= header_size + 4 {
        let edit_handle_size = i32::from_le_bytes(bytes[header_size..header_size + 4].try_into().unwrap());
        if edit_handle_size == 0x4c09e8 && bytes.len() >= header_size + 8 + 520 {
            let uncompressed_size = 0x20c;
            let comp_data_start = header_size + 8 + 520;
            let decomp_size = 0x4c09e8 - uncompressed_size;
            let decomp_handle = decompress_aup(&bytes[comp_data_start..], decomp_size);
            if decomp_handle.len() >= 0x470 - uncompressed_size {
                let w = i32::from_le_bytes(decomp_handle[0x310 - uncompressed_size..0x310 - uncompressed_size + 4].try_into().unwrap());
                let h = i32::from_le_bytes(decomp_handle[0x314 - uncompressed_size..0x314 - uncompressed_size + 4].try_into().unwrap());
                let resized_w = i32::from_le_bytes(decomp_handle[0x328 - uncompressed_size..0x328 - uncompressed_size + 4].try_into().unwrap());
                let resized_h = i32::from_le_bytes(decomp_handle[0x32c - uncompressed_size..0x32c - uncompressed_size + 4].try_into().unwrap());
                let ch = i16::from_le_bytes(decomp_handle[0x3fa - uncompressed_size..0x3fa - uncompressed_size + 2].try_into().unwrap());
                let ar = i32::from_le_bytes(decomp_handle[0x3fc - uncompressed_size..0x3fc - uncompressed_size + 4].try_into().unwrap());
                let vs = i32::from_le_bytes(decomp_handle[0x468 - uncompressed_size..0x468 - uncompressed_size + 4].try_into().unwrap());
                let vr = i32::from_le_bytes(decomp_handle[0x46c - uncompressed_size..0x46c - uncompressed_size + 4].try_into().unwrap());

                let final_w = if resized_w > 0 { resized_w as u32 } else if w > 0 { w as u32 } else { 1280 };
                let final_h = if resized_h > 0 { resized_h as u32 } else if h > 0 { h as u32 } else { 720 };
                if (16..=16384).contains(&final_w) && (16..=16384).contains(&final_h) {
                    aup_w = final_w;
                    aup_h = final_h;
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
            let is_pre_092 = exedit_version < 0x23f0;
            let base_size = if hdr_base_size >= 0x500 && hdr_base_size <= 0x1000 {
                hdr_base_size
            } else if is_pre_092 {
                0x508
            } else {
                0x5c8
            };
            let param_count = if base_size >= 0x5c0 { 64 } else { 16 };
            let (layer_off, scene_off) = (base_size - 8, base_size - 4);

            println!(
                "EXEDIT HEADER: ver=0x{:x}, eff_types={}, objs={}, edit_scene={}, scenes={}, layers={}, tb_scripts={}, base_size=0x{:x}",
                exedit_version, effect_type_num, object_num, editing_scene, scene_num, layer_num, trackbar_script_num, base_size
            );

            let eff_layer_size = if layer_size > 0 { layer_size } else { 76 };
            let eff_scene_size = if scene_size > 0 { scene_size } else { 220 };

            let mut cursor = 0x100;
            cursor += layer_num * eff_layer_size;

            // Scenes
            let mut scenes_info = Vec::new();
            for _ in 0..scene_num {
                if cursor + eff_scene_size <= decomp.len() {
                    let s_data = &decomp[cursor..cursor + eff_scene_size];
                    let s_name = decode_sjis_cstring(&s_data[8..8 + 64]);
                    let w = u32::from_le_bytes(s_data[0x48..0x4c].try_into().unwrap());
                    let h = u32::from_le_bytes(s_data[0x4c..0x50].try_into().unwrap());
                    let max_f = u32::from_le_bytes(s_data[0x50..0x54].try_into().unwrap());
                    scenes_info.push((s_name, w, h, max_f));
                    cursor += eff_scene_size;
                }
            }

            // TrackbarScripts
            let mut scripts = Vec::new();
            for _ in 0..trackbar_script_num {
                if cursor + 128 <= decomp.len() {
                    let sc_data = &decomp[cursor..cursor + 128];
                    scripts.push(decode_sjis_cstring(sc_data));
                    cursor += 128;
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

            let proj_w = if !scenes_info.is_empty() && scenes_info[0].1 > 0 {
                scenes_info[0].1
            } else {
                aup_w
            };
            let proj_h = if !scenes_info.is_empty() && scenes_info[0].2 > 0 {
                scenes_info[0].2
            } else {
                aup_h
            };
            let proj_len = if !scenes_info.is_empty() && scenes_info[0].3 > 0 && scenes_info[0].3 < 10_000_000 {
                scenes_info[0].3
            } else if aup_len > 0 && aup_len < 10_000_000 {
                aup_len
            } else {
                1800
            };

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
                let layer = u32::from_le_bytes(o_data[layer_off..layer_off + 4].try_into().unwrap());
                let scene = u32::from_le_bytes(o_data[scene_off..scene_off + 4].try_into().unwrap());

                let is_chain = chain_group != 0xFFFF_FFFF && chain_group == last_chain_group && ext_size_field == 0;
                last_chain_group = chain_group;

                let mut ext_sum = 0;
                let is_valid = (flag & 1) != 0
                    && flag != 0xFFFF_FFFF
                    && end_f != 0xFFFF_FFFF
                    && start_f <= end_f
                    && layer < layer_num as u32;
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

            // 編集対象のシーン（通常 Scene 0）のオブジェクトを .exo へ変換
            let scene_objs: Vec<_> = all_objs.into_iter().filter(|o| o.scene == editing_scene).collect();
            let mut out_obj_idx = 0;

            for obj in &scene_objs {
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
                        let cur = i32::from_le_bytes(o_data[0xF8 + idx * 4..0xF8 + idx * 4 + 4].try_into().unwrap());
                        let next = i32::from_le_bytes(o_data[0x1F8 + idx * 4..0x1F8 + idx * 4 + 4].try_into().unwrap());
                        let trans = u32::from_le_bytes(o_data[0x2F8 + idx * 4..0x2F8 + idx * 4 + 4].try_into().unwrap());
                        let param = if idx < param_count {
                            i32::from_le_bytes(o_data[0x4C0 + idx * 4..0x4C0 + idx * 4 + 4].try_into().unwrap())
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

                    // チェックボックス
                    for cb_i in 0..eff_type.checkbox_num {
                        let idx = checkbox_count + cb_i;
                        let val = i32::from_le_bytes(o_data[0x3F8 + idx * 4..0x3F8 + idx * 4 + 4].try_into().unwrap());
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

                out_obj_idx += 1;
            }

            // au2_exo の高度なパース処理（イージング・中間点連結・トラック値解析）へ委譲
            match parse_exo_str(&exo) {
                Ok(doc) => {
                    let mut scene = doc.scene;
                    let mut max_end_frame = 0;
                    let mut obj_count = 0;
                    for l in &scene.layers {
                        for o in &l.objects {
                            obj_count += 1;
                            if o.frame_end > max_end_frame && o.frame_end < 10_000_000 {
                                max_end_frame = o.frame_end;
                            }
                        }
                    }
                    if scene.settings.length >= 10_000_000 || max_end_frame > scene.settings.length {
                        scene.settings.length = max_end_frame.max(1800) + 30;
                    }
                    warnings.push(format!(
                        "初代 AviUtl プロジェクトファイル（.aup）内の拡張編集データ（オブジェクト {} 個）を正常に復元しました",
                        obj_count
                    ));
                    return Ok(AupDocument {
                        version,
                        scene,
                        cursor_frame: 0,
                        warnings,
                    });
                }
                Err(e) => {
                    warnings.push(format!("拡張編集データの詳細パースに一部失敗したため基本設定のみ読み込みました: {e}"));
                }
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
    warnings.push("初代 AviUtl プロジェクトファイル (.aup) の基本設定を読み込みました".into());

    Ok(AupDocument {
        version,
        scene,
        cursor_frame: 0,
        warnings,
    })
}

fn get_trackbar_scale(eff: &str, i: usize) -> i32 {
    match eff {
        "標準描画" | "拡張描画" => match i {
            0..=2 => 10,
            3 => 100,
            4 => 10,
            5 => 100,
            6..=8 => 100,
            9..=11 => 10,
            _ => 1,
        },
        "動画ファイル" => match i {
            0 => 1,
            1 => 10,
            _ => 1,
        },
        "標準再生" => match i {
            0..=1 => 10,
            _ => 1,
        },
        "図形" => match i {
            0 => 1,
            1 => 10,
            2 => 1,
            _ => 1,
        },
        "テキスト" => match i {
            0 => 1,
            1 => 10,
            _ => 1,
        },
        "フェード" => 100,
        "振動" => match i {
            0..=2 => 10,
            3 => 100,
            _ => 1,
        },
        "シャドー" => match i {
            0..=1 => 10,
            _ => 1,
        },
        "色調補正" => 10,
        "閃光" => match i {
            4..=5 => 10,
            _ => 1,
        },
        "マスク" => match i {
            0..=1 => 10,
            2 => 100,
            4 => 10,
            _ => 1,
        },
        "カメラ制御" => match i {
            0..=5 => 10,
            6 => 1,
            7 => 100,
            8 => 10,
            9 => 100,
            _ => 1,
        },
        "カメラ効果" => 100,
        "シャドー(カメラ制御)" => match i {
            0..=2 => 10,
            3..=4 => 1,
            _ => 1,
        },
        _ => 1,
    }
}

fn get_trackbar_name(eff: &str, i: usize) -> String {
    match eff {
        "標準描画" => match i {
            0 => "X".into(),
            1 => "Y".into(),
            2 => "Z".into(),
            3 => "拡大率".into(),
            4 => "透明度".into(),
            5 => "回転".into(),
            _ => format!("track{}", i),
        },
        "拡張描画" => match i {
            0 => "X".into(),
            1 => "Y".into(),
            2 => "Z".into(),
            3 => "拡大率".into(),
            4 => "透明度".into(),
            5 => "縦横比".into(),
            6 => "X軸回転".into(),
            7 => "Y軸回転".into(),
            8 => "Z軸回転".into(),
            9 => "中心X".into(),
            10 => "中心Y".into(),
            11 => "中心Z".into(),
            _ => format!("track{}", i),
        },
        "動画ファイル" | "音声ファイル" => match i {
            0 => "再生位置".into(),
            1 => "再生速度".into(),
            _ => format!("track{}", i),
        },
        "標準再生" => match i {
            0 => "音量".into(),
            1 => "左右".into(),
            _ => format!("track{}", i),
        },
        "図形" => match i {
            0 => "サイズ".into(),
            1 => "縦横比".into(),
            2 => "ライン幅".into(),
            _ => format!("track{}", i),
        },
        "テキスト" => match i {
            0 => "サイズ".into(),
            1 => "表示速度".into(),
            _ => format!("track{}", i),
        },
        "グロー" => match i {
            0 => "強さ".into(),
            1 => "拡散".into(),
            2 => "しきい値".into(),
            3 => "ぼかし".into(),
            _ => format!("track{}", i),
        },
        "ぼかし" => match i {
            0 => "範囲".into(),
            _ => format!("track{}", i),
        },
        "フェード" => match i {
            0 => "イン".into(),
            1 => "アウト".into(),
            _ => format!("track{}", i),
        },
        "振動" => match i {
            0 => "X".into(),
            1 => "Y".into(),
            2 => "Z".into(),
            3 => "周期".into(),
            _ => format!("track{}", i),
        },
        "クリッピング" => match i {
            0 => "上".into(),
            1 => "下".into(),
            2 => "左".into(),
            3 => "右".into(),
            _ => format!("track{}", i),
        },
        "縁取り" => match i {
            0 => "サイズ".into(),
            1 => "ぼかし".into(),
            _ => format!("track{}", i),
        },
        "シャドー" => match i {
            0 => "X".into(),
            1 => "Y".into(),
            2 => "濃さ".into(),
            3 => "拡散".into(),
            _ => format!("track{}", i),
        },
        "色調補正" => match i {
            0 => "明るさ".into(),
            1 => "コントラスト".into(),
            2 => "色相".into(),
            3 => "輝度".into(),
            4 => "彩度".into(),
            _ => format!("track{}", i),
        },
        "拡散光" => match i {
            0 => "強さ".into(),
            1 => "拡散".into(),
            2 => "しきい値".into(),
            _ => format!("track{}", i),
        },
        "閃光" => match i {
            0 => "強さ".into(),
            1 => "拡散".into(),
            2 => "しきい値".into(),
            3 => "前景比率".into(),
            4 => "X".into(),
            5 => "Y".into(),
            _ => format!("track{}", i),
        },
        "マスク" => match i {
            0 => "X".into(),
            1 => "Y".into(),
            2 => "回転".into(),
            3 => "サイズ".into(),
            4 => "縦横比".into(),
            5 => "ぼかし".into(),
            _ => format!("track{}", i),
        },
        "シーン" => match i {
            0 => "再生位置".into(),
            1 => "再生速度".into(),
            _ => format!("track{}", i),
        },
        "カメラ制御" => match i {
            0 => "X".into(),
            1 => "Y".into(),
            2 => "Z".into(),
            3 => "目標X".into(),
            4 => "目標Y".into(),
            5 => "目標Z".into(),
            6 => "目標レイヤー".into(),
            7 => "傾き".into(),
            8 => "深度ぼけ".into(),
            9 => "視野角".into(),
            _ => format!("track{}", i),
        },
        "シャドー(カメラ制御)" => match i {
            0 => "X".into(),
            1 => "Y".into(),
            2 => "Z".into(),
            3 => "濃さ".into(),
            4 => "精度".into(),
            _ => format!("track{}", i),
        },
        _ => format!("track{}", i),
    }
}

fn get_checkbox_name(eff: &str, i: usize) -> Option<String> {
    match eff {
        "動画ファイル" | "音声ファイル" => match i {
            0 => Some("ループ再生".into()),
            1 => Some("アルファチャンネルを読み込む".into()),
            _ => Some(format!("check{}", i)),
        },
        "画像ファイル" => Some(format!("check{}", i)),
        "テキスト" => match i {
            0 => Some("B".into()),
            1 => Some("I".into()),
            2 => Some("自動スクロール".into()),
            _ => Some(format!("check{}", i)),
        },
        "カメラ制御" => match i {
            0 => Some("Zバッファ/シャドウマップを有効にする".into()),
            _ => Some(format!("check{}", i)),
        },
        "グロー" => match i {
            0 => Some("光色の設定".into()),
            1 => Some("光成分のみ".into()),
            _ => Some(format!("check{}", i)),
        },
        "ぼかし" => match i {
            0 => Some("光の強さを維持する".into()),
            _ => Some(format!("check{}", i)),
        },
        "反転" => match i {
            0 => Some("上下反転".into()),
            1 => Some("左右反転".into()),
            2 => Some("輝度反転".into()),
            3 => Some("色相反転".into()),
            4 => Some("透明度反転".into()),
            _ => Some(format!("check{}", i)),
        },
        _ => Some(format!("check{}", i)),
    }
}

fn clean_file_path(raw: &[u8]) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let mut slice = raw;
    let mut is_null_drive = false;
    // 先頭バイトが 0x00 で直後に ":\" または ":/" が続く場合（AviUtl拡張編集特有の先頭nullドライブ文字）
    if slice.len() >= 3 && slice[0] == 0 && slice[1] == b':' && (slice[2] == b'\\' || slice[2] == b'/') {
        is_null_drive = true;
        slice = &slice[1..];
    } else if slice.len() >= 2 && slice[0] == 0 && (slice[1] == b'\\' || slice[1] == b'/') {
        slice = &slice[1..];
    }
    let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    let valid_slice = &slice[..end];
    if valid_slice.is_empty() {
        return None;
    }
    let (decoded, _, _) = encoding_rs::SHIFT_JIS.decode(valid_slice);
    let mut path = decoded.trim().to_string();
    if is_null_drive || path.starts_with(":\\") || path.starts_with(":/") {
        if path.starts_with(":\\") || path.starts_with(":/") {
            path = format!("C{}", path);
        } else {
            path = format!("C:{}", path);
        }
    }
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
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
        "シーン" => {
            if ext.len() >= 4 {
                let scene = i32::from_le_bytes(ext[0..4].try_into().unwrap());
                exo.push_str(&format!("scene={}\r\n", scene));
            }
        }
        "カメラ制御" => {
            if ext.len() >= 4 {
                let range = i32::from_le_bytes(ext[0..4].try_into().unwrap());
                exo.push_str(&format!("range={}\r\n", range));
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
                        "シーンチェンジ" => BUILTIN_SCN_NAMES.get(script_id as usize).copied().unwrap_or(""),
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
