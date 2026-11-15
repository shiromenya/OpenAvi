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

/// 初代 AviUtl 拡張編集の組み込みシーンチェンジ一覧 (exedit.scn スクリプト)
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

/// 初代 AviUtl 拡張編集の組み込みシーンチェンジ種別一覧 (type=0..=22、スクリプト名が空のときに使用)
pub const BUILTIN_SCN_TYPES: &[&str] = &[
    "クロスフェード",                      // 0
    "ワイプ(円)",                          // 1
    "ワイプ(四角)",                        // 2
    "ワイプ(時計)",                        // 3
    "スライス",                            // 4
    "スワップ",                            // 5
    "スライド",                            // 6
    "縮小回転",                            // 7
    "押し出し(横)",                        // 8
    "押し出し(縦)",                        // 9
    "回転(横)",                            // 10
    "回転(縦)",                            // 11
    "キューブ回転(横)",                    // 12
    "キューブ回転(縦)",                    // 13
    "フェードアウトイン",                  // 14
    "放射ブラー",                          // 15
    "ぼかし",                              // 16
    "ワイプ(横)",                          // 17
    "ワイプ(縦)",                          // 18
    "ロール(横)",                          // 19
    "ロール(縦)",                          // 20
    "ランダムライン",                      // 21
    "ルミナンス",                          // 22
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

#[inline]
fn get_i32(slice: &[u8], offset: usize) -> Option<i32> {
    slice.get(offset..offset + 4).and_then(|s| s.try_into().ok()).map(i32::from_le_bytes)
}

#[inline]
fn get_i16(slice: &[u8], offset: usize) -> Option<i16> {
    slice.get(offset..offset + 2).and_then(|s| s.try_into().ok()).map(i16::from_le_bytes)
}

/// 初代 `.aup` バイナリから拡張編集プロジェクトまたは本体データを完全解析します。
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
        "グループ制御" => match i {
            0..=2 => 10,
            3..=6 => 100,
            _ => 1,
        },
        "時間制御" => match i {
            0 => 10,
            1 => 1,
            _ => 1,
        },
        "動画ファイル" | "シーン" => match i {
            0 => 1,
            1 => 10,
            _ => 1,
        },
        "音声ファイル" | "シーン(音声)" => match i {
            0 => 100,
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
        "グロー" => match i {
            0 => 10,
            1 => 1,
            2 => 10,
            3 => 1,
            4 => 10,
            _ => 1,
        },
        "発光" => match i {
            0 => 10,
            1 => 1,
            2 => 10,
            3 => 1,
            _ => 1,
        },
        "拡散光" => match i {
            0 => 10,
            1 => 1,
            _ => 1,
        },
        "閃光" => match i {
            0..=2 => 10,
            3 => 1,
            4..=5 => 10,
            _ => 1,
        },
        "ライト" => match i {
            0 => 10,
            1 => 1,
            2 => 10,
            _ => 1,
        },
        "ぼかし" | "境界ぼかし" => match i {
            0 => 1,
            1 => 10,
            _ => 1,
        },
        "方向ブラー" => match i {
            0 => 1,
            1 => 10,
            _ => 1,
        },
        "フェード" => 100,
        "ワイプ" => match i {
            0..=1 => 100,
            2 => 1,
            _ => 1,
        },
        "振動" => match i {
            0..=2 => 10,
            3 => 100,
            _ => 1,
        },
        "シャドー" => match i {
            0..=2 => 10,
            3 => 1,
            _ => 1,
        },
        "凸エッジ" => match i {
            0 => 1,
            1 => 100,
            2 => 1,
            _ => 1,
        },
        "エッジ抽出" => match i {
            0..=1 => 10,
            _ => 1,
        },
        "色調補正" => 10,
        "グラデーション" => match i {
            0 => 10,
            1..=2 => 1,
            3 => 10,
            4 => 1,
            _ => 1,
        },
        "単色化" => 10,
        "斜めクリッピング" => match i {
            0..=1 => 1,
            2 => 10,
            3..=4 => 1,
            _ => 1,
        },
        "マスク" => match i {
            0..=1 => 10,
            2 => 100,
            3 => 1,
            4 => 10,
            5 => 1,
            _ => 1,
        },
        "座標" => 10,
        "拡大率" | "リサイズ" => 100,
        "透明度" => 10,
        "回転" => 100,
        "ミラー" => match i {
            0..=1 => 10,
            2 => 1,
            _ => 1,
        },
        "ラスター" => match i {
            0..=1 => 1,
            2 => 100,
            _ => 1,
        },
        "波紋" => match i {
            0..=3 => 1,
            4 => 10,
            _ => 1,
        },
        "画像ループ" => match i {
            0..=1 => 1,
            2..=3 => 10,
            _ => 1,
        },
        "極座標変換" => match i {
            0 => 1,
            1..=2 => 10,
            3 => 100,
            _ => 1,
        },
        "ディスプレイスメントマップ" => match i {
            0..=3 => 10,
            4 => 100,
            5 => 1,
            6 => 10,
            7 => 1,
            _ => 1,
        },
        "ノイズ" => match i {
            0..=3 => 10,
            4..=5 => 100,
            6 => 10,
            _ => 1,
        },
        "色ずれ" => match i {
            0 => 1,
            1 => 10,
            2 => 1,
            _ => 1,
        },
        "パーティクル出力" => match i {
            0..=12 => 10,
            13 => 100,
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
        "アニメーション効果"
        | "カスタムオブジェクト"
        | "カメラ効果"
        | "スクリプト(カメラ制御)"
        | "シーンチェンジ" => 100,
        "シャドー(カメラ制御)" => match i {
            0..=2 => 10,
            3..=4 => 1,
            _ => 1,
        },
        "音声フェード" => 100,
        "音声ディレイ" => match i {
            0 => 10,
            1 => 1,
            _ => 1,
        },
        "モノラル化" | "音量フィルタ" => 10,
        "動画ファイル合成" => match i {
            0 => 1,
            1..=3 => 10,
            4 => 100,
            _ => 1,
        },
        "画像ファイル合成" => match i {
            0..=1 => 10,
            2 => 100,
            _ => 1,
        },
        "音声波形表示" => match i {
            0..=1 => 1,
            2 => 10,
            3 => 100,
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
        "グループ制御" => match i {
            0 => "X".into(),
            1 => "Y".into(),
            2 => "Z".into(),
            3 => "拡大率".into(),
            4 => "X軸回転".into(),
            5 => "Y軸回転".into(),
            6 => "Z軸回転".into(),
            _ => format!("track{}", i),
        },
        "時間制御" => match i {
            0 => "位置".into(),
            1 => "距離".into(),
            _ => format!("track{}", i),
        },
        "動画ファイル" | "音声ファイル" | "シーン" | "シーン(音声)" => match i {
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
            4 => "比率".into(),
            _ => format!("track{}", i),
        },
        "発光" => match i {
            0 => "強さ".into(),
            1 => "拡散".into(),
            2 => "しきい値".into(),
            3 => "拡散速度".into(),
            _ => format!("track{}", i),
        },
        "ぼかし" => match i {
            0 => "範囲".into(),
            1 => "縦横比".into(),
            2 => "光の強さ".into(),
            _ => format!("track{}", i),
        },
        "境界ぼかし" => match i {
            0 => "範囲".into(),
            1 => "縦横比".into(),
            _ => format!("track{}", i),
        },
        "放射ブラー" => match i {
            0 => "範囲".into(),
            1 => "X".into(),
            2 => "Y".into(),
            _ => format!("track{}", i),
        },
        "方向ブラー" => match i {
            0 => "範囲".into(),
            1 => "角度".into(),
            _ => format!("track{}", i),
        },
        "レンズブラー" => match i {
            0 => "範囲".into(),
            1 => "光の強さ".into(),
            _ => format!("track{}", i),
        },
        "モーションブラー" => match i {
            0 => "間隔".into(),
            1 => "分解能".into(),
            _ => format!("track{}", i),
        },
        "モザイク" => match i {
            0 => "サイズ".into(),
            _ => format!("track{}", i),
        },
        "フェード" => match i {
            0 => "イン".into(),
            1 => "アウト".into(),
            _ => format!("track{}", i),
        },
        "ワイプ" => match i {
            0 => "イン".into(),
            1 => "アウト".into(),
            2 => "ぼかし".into(),
            _ => format!("track{}", i),
        },
        "振動" => match i {
            0 => "X".into(),
            1 => "Y".into(),
            2 => "Z".into(),
            3 => "周期".into(),
            _ => format!("track{}", i),
        },
        "クリッピング" | "領域拡張" => match i {
            0 => "上".into(),
            1 => "下".into(),
            2 => "左".into(),
            3 => "右".into(),
            _ => format!("track{}", i),
        },
        "斜めクリッピング" => match i {
            0 => "中心X".into(),
            1 => "中心Y".into(),
            2 => "角度".into(),
            3 => "ぼかし".into(),
            4 => "幅".into(),
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
        "凸エッジ" => match i {
            0 => "幅".into(),
            1 => "高さ".into(),
            2 => "角度".into(),
            _ => format!("track{}", i),
        },
        "エッジ抽出" => match i {
            0 => "強さ".into(),
            1 => "しきい値".into(),
            _ => format!("track{}", i),
        },
        "シャープ" => match i {
            0 => "強さ".into(),
            1 => "範囲".into(),
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
        "グラデーション" => match i {
            0 => "強さ".into(),
            1 => "中心X".into(),
            2 => "中心Y".into(),
            3 => "角度".into(),
            4 => "幅".into(),
            _ => format!("track{}", i),
        },
        "単色化" => match i {
            0 => "強さ".into(),
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
            1 => "X".into(),
            2 => "Y".into(),
            3 => "サイズ".into(),
            4 => "X".into(),
            5 => "Y".into(),
            _ => format!("track{}", i),
        },
        "ライト" => match i {
            0 => "強さ".into(),
            1 => "拡散".into(),
            2 => "比率".into(),
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
        "座標" => match i {
            0 => "X".into(),
            1 => "Y".into(),
            2 => "Z".into(),
            _ => format!("track{}", i),
        },
        "拡大率" => match i {
            0 => "拡大率".into(),
            1 => "X".into(),
            2 => "Y".into(),
            3 => "Z".into(),
            _ => format!("track{}", i),
        },
        "リサイズ" => match i {
            0 => "拡大率".into(),
            1 => "X".into(),
            2 => "Y".into(),
            _ => format!("track{}", i),
        },
        "透明度" => match i {
            0 => "透明度".into(),
            _ => format!("track{}", i),
        },
        "回転" => match i {
            0 => "X".into(),
            1 => "Y".into(),
            2 => "Z".into(),
            _ => format!("track{}", i),
        },
        "ローテーション" => match i {
            0 => "90度回転".into(),
            _ => format!("track{}", i),
        },
        "ミラー" => match i {
            0 => "透明度".into(),
            1 => "減衰".into(),
            2 => "境目調整".into(),
            _ => format!("track{}", i),
        },
        "ラスター" => match i {
            0 => "横幅".into(),
            1 => "高さ".into(),
            2 => "周期".into(),
            _ => format!("track{}", i),
        },
        "波紋" => match i {
            0 => "中心X".into(),
            1 => "中心Y".into(),
            2 => "幅".into(),
            3 => "高さ".into(),
            4 => "速度".into(),
            _ => format!("track{}", i),
        },
        "画像ループ" => match i {
            0 => "横回数".into(),
            1 => "縦回数".into(),
            2 => "速度X".into(),
            3 => "速度Y".into(),
            _ => format!("track{}", i),
        },
        "極座標変換" => match i {
            0 => "中心幅".into(),
            1 => "拡大率".into(),
            2 => "回転".into(),
            3 => "渦巻".into(),
            _ => format!("track{}", i),
        },
        "ディスプレイスメントマップ" => match i {
            0 => "変形X".into(),
            1 => "変形Y".into(),
            2 => "X".into(),
            3 => "Y".into(),
            4 => "回転".into(),
            5 => "サイズ".into(),
            6 => "縦横比".into(),
            7 => "ぼかし".into(),
            _ => format!("track{}", i),
        },
        "ノイズ" => match i {
            0 => "強さ".into(),
            1 => "速度X".into(),
            2 => "速度Y".into(),
            3 => "変化速度".into(),
            4 => "周期X".into(),
            5 => "周期Y".into(),
            6 => "しきい値".into(),
            _ => format!("track{}", i),
        },
        "色ずれ" => match i {
            0 => "ずれ幅".into(),
            1 => "角度".into(),
            2 => "強さ".into(),
            _ => format!("track{}", i),
        },
        "オブジェクト分割" => match i {
            0 => "横分割数".into(),
            1 => "縦分割数".into(),
            _ => format!("track{}", i),
        },
        "パーティクル出力" => match i {
            0 => "X".into(),
            1 => "Y".into(),
            2 => "Z".into(),
            3 => "出力頻度".into(),
            4 => "出力速度".into(),
            5 => "加速度".into(),
            6 => "出力方向".into(),
            7 => "拡散角度".into(),
            8 => "透過速度".into(),
            9 => "拡大速度".into(),
            10 => "回転角".into(),
            11 => "回転速度".into(),
            12 => "重力".into(),
            13 => "生存時間".into(),
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
        "クロマキー" | "特定色域変換" => match i {
            0 => "色相範囲".into(),
            1 => "彩度範囲".into(),
            2 => "境界補正".into(),
            _ => format!("track{}", i),
        },
        "カラーキー" => match i {
            0 => "輝度範囲".into(),
            1 => "色差範囲".into(),
            2 => "境界補正".into(),
            _ => format!("track{}", i),
        },
        "ルミナンスキー" => match i {
            0 => "基準輝度".into(),
            1 => "ぼかし".into(),
            _ => format!("track{}", i),
        },
        "拡張色設定" => match i {
            0 => "R".into(),
            1 => "G".into(),
            2 => "B".into(),
            _ => format!("track{}", i),
        },
        "音声フェード" => match i {
            0 => "イン".into(),
            1 => "アウト".into(),
            _ => format!("track{}", i),
        },
        "音声ディレイ" => match i {
            0 => "強さ".into(),
            1 => "遅延".into(),
            _ => format!("track{}", i),
        },
        "モノラル化" => match i {
            0 => "比率".into(),
            _ => format!("track{}", i),
        },
        "音量フィルタ" => match i {
            0 => "レベル".into(),
            _ => format!("track{}", i),
        },
        "動画ファイル合成" => match i {
            0 => "再生位置".into(),
            1 => "再生速度".into(),
            2 => "X".into(),
            3 => "Y".into(),
            4 => "拡大率".into(),
            _ => format!("track{}", i),
        },
        "画像ファイル合成" => match i {
            0 => "X".into(),
            1 => "Y".into(),
            2 => "拡大率".into(),
            _ => format!("track{}", i),
        },
        "音声波形表示" => match i {
            0 => "横幅".into(),
            1 => "高さ".into(),
            2 => "音量".into(),
            3 => "再生位置".into(),
            4 => "編集全体の音声を元にする".into(),
            _ => format!("track{}", i),
        },
        _ => format!("track{}", i),
    }
}

fn get_checkbox_name(eff: &str, i: usize) -> Option<String> {
    match eff {
        "動画ファイル" | "音声ファイル" | "シーン" | "シーン(音声)" => match i {
            0 => Some("ループ再生".into()),
            1 => Some("アルファチャンネルを読み込む".into()),
            _ => Some(format!("check{}", i)),
        },
        "画像ファイル" => Some(format!("check{}", i)),
        "テキスト" => match i {
            0 => Some("文字毎に個別オブジェクト".into()),
            1 => Some("移動座標上に表示する".into()),
            2 => Some("自動スクロール".into()),
            3 => Some("B".into()),
            4 => Some("I".into()),
            _ => Some(format!("check{}", i)),
        },
        "拡張描画" => match i {
            0 => Some("裏面を表示しない".into()),
            _ => Some(format!("check{}", i)),
        },
        "グループ制御" => match i {
            0 => Some("上位グループ制御の影響を受ける".into()),
            1 => Some("同じグループのオブジェクトを対象にする".into()),
            _ => Some(format!("check{}", i)),
        },
        "時間制御" => match i {
            0 => Some("フレーム番号指定".into()),
            1 => Some("ループ画像".into()),
            _ => Some(format!("check{}", i)),
        },
        "カメラ制御" => match i {
            0 => Some("Zバッファ/シャドウマップを有効にする".into()),
            _ => Some(format!("check{}", i)),
        },
        "グロー" | "発光" => match i {
            0 => Some("光成分のみ".into()),
            _ => Some(format!("check{}", i)),
        },
        "ぼかし" | "レンズブラー" => match i {
            0 => Some("サイズ固定".into()),
            _ => Some(format!("check{}", i)),
        },
        "境界ぼかし" => match i {
            0 => Some("透明度の境界をぼかす".into()),
            _ => Some(format!("check{}", i)),
        },
        "クリッピング" => match i {
            0 => Some("中心の位置を変更".into()),
            _ => Some(format!("check{}", i)),
        },
        "領域拡張" => match i {
            0 => Some("塗りつぶし".into()),
            _ => Some(format!("check{}", i)),
        },
        "ワイプ" => match i {
            0 => Some("反転(イン)".into()),
            1 => Some("反転(アウト)".into()),
            _ => Some(format!("check{}", i)),
        },
        "マスク" => match i {
            0 => Some("マスクの反転".into()),
            1 => Some("元のサイズに合わせる".into()),
            _ => Some(format!("check{}", i)),
        },
        "単色化" => match i {
            0 => Some("輝度を保持する".into()),
            _ => Some(format!("check{}", i)),
        },
        "シャドー" => match i {
            0 => Some("影を別オブジェクトで描画".into()),
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
        "クロマキー" => match i {
            0 => Some("色彩補正".into()),
            1 => Some("透過補正".into()),
            _ => Some(format!("check{}", i)),
        },
        "カラーキー" => match i {
            0 => Some("透過補正".into()),
            _ => Some(format!("check{}", i)),
        },
        "特定色域変換" => match i {
            0 => Some("補正".into()),
            _ => Some(format!("check{}", i)),
        },
        "色調補正" => match i {
            0 => Some("飽和する".into()),
            _ => Some(format!("check{}", i)),
        },
        "音声ディレイ" => match i {
            0 => Some("原音の音量を維持".into()),
            _ => Some(format!("check{}", i)),
        },
        "動画ファイル合成" => match i {
            0 => Some("ループ再生".into()),
            1 => Some("動画ファイルの同期".into()),
            2 => Some("ループ画像".into()),
            _ => Some(format!("check{}", i)),
        },
        "画像ファイル合成" => match i {
            0 => Some("ループ画像".into()),
            _ => Some(format!("check{}", i)),
        },
        "カメラ制御オプション" => match i {
            0 => Some("シャドウの対象から外す".into()),
            1 => Some("カメラの方を向く".into()),
            2 => Some("カメラの方を向く(縦横方向のみ)".into()),
            3 => Some("カメラの方を向く(横方向のみ)".into()),
            _ => Some(format!("check{}", i)),
        },
        "フレームバッファ" => match i {
            0 => Some("フレームバッファをクリア".into()),
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

// ============================================================================
// 初代 AviUtl (.aup) & 拡張編集 0.92 (80EE) ネイティブバイナリエンコーダ
// ============================================================================

/// 4バイト先頭がすべて同一バイトか判定します (`AupUtil.IsSame4Bytes` 互換)。
#[inline]
fn is_same_4_bytes(data: &[u8]) -> bool {
    data.len() >= 4 && data[0] == data[1] && data[0] == data[2] && data[0] == data[3]
}

/// AviUtl プロジェクトファイル (.aup) 固有の RLE 圧縮を行います (`AupUtil.Comp` 完全互換)。
pub fn compress_aup(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 4 + 64);
    let mut pos = 0usize;
    while pos < data.len() {
        if is_same_4_bytes(&data[pos..]) {
            let b = data[pos];
            let max_len = (data.len() - pos).min(0x7F_FFFF);
            let mut rep = 4usize;
            while rep < max_len && data[pos + rep] == b {
                rep += 1;
            }
            if rep < 0x80 {
                out.push((rep as u8) | 0x80);
            } else {
                let size_data = ((rep as u32) << 8) | 0x80;
                out.extend_from_slice(&size_data.to_le_bytes());
            }
            out.push(b);
            pos += rep;
        } else {
            let max_len = (data.len() - pos).min(0x7F_FFFF);
            let mut rep = 0usize;
            while rep < max_len && !is_same_4_bytes(&data[pos + rep..]) {
                rep += 1;
            }
            if rep < 0x80 {
                out.push(rep as u8);
            } else {
                let size_data = (rep as u32) << 8;
                out.extend_from_slice(&size_data.to_le_bytes());
            }
            out.extend_from_slice(&data[pos..pos + rep]);
            pos += rep;
        }
    }
    out
}

/// 拡張編集 0.92 の標準 108 `EffectType` 定義テーブル `(id, flag, trackbar_num, checkbox_num, ext_size, name)`
pub const DEFAULT_EFFECT_TYPES: &[(u32, u32, usize, usize, usize, &str)] = &[
    (0, 0x04000448, 2, 3, 284, "動画ファイル"),
    (1, 0x04000408, 0, 1, 260, "画像ファイル"),
    (2, 0x04200408, 2, 3, 280, "音声ファイル"),
    (3, 0x04000408, 2, 5, 2096, "テキスト"),
    (4, 0x04000408, 3, 2, 264, "図形"),
    (5, 0x04000008, 0, 1, 0, "フレームバッファ"),
    (6, 0x04000408, 4, 5, 304, "音声波形表示"),
    (7, 0x04000408, 2, 2, 4, "シーン"),
    (8, 0x04200408, 2, 3, 4, "シーン(音声)"),
    (9, 0x04000008, 0, 0, 0, "直前オブジェクト"),
    (10, 0x440004D0, 6, 1, 4, "標準描画"),
    (11, 0x440004D0, 12, 2, 4, "拡張描画"),
    (12, 0x04200090, 2, 0, 0, "標準再生"),
    (13, 0x44000450, 16, 5, 4, "パーティクル出力"),
    (14, 0x04000400, 2, 3, 516, "シーンチェンジ"),
    (15, 0x04000020, 5, 1, 0, "色調補正"),
    (16, 0x04000000, 5, 1, 0, "色調補正"),
    (17, 0x04000020, 4, 1, 0, "クリッピング"),
    (18, 0x04000020, 3, 1, 0, "ぼかし"),
    (19, 0x04000020, 2, 1, 0, "境界ぼかし"),
    (20, 0x04000000, 3, 0, 0, "ぼかし"),
    (21, 0x04000020, 1, 1, 0, "モザイク"),
    (22, 0x04000000, 1, 1, 0, "モザイク"),
    (23, 0x04000420, 4, 2, 4, "発光"),
    (24, 0x04000400, 4, 1, 4, "発光"),
    (25, 0x04000420, 3, 3, 8, "閃光"),
    (26, 0x04000020, 2, 1, 0, "拡散光"),
    (27, 0x04000000, 2, 0, 0, "拡散光"),
    (28, 0x04000420, 4, 3, 8, "グロー"),
    (29, 0x04000400, 4, 2, 8, "グロー"),
    (30, 0x04000420, 3, 3, 12, "クロマキー"),
    (31, 0x04000420, 3, 1, 12, "カラーキー"),
    (32, 0x04000420, 2, 1, 4, "ルミナンスキー"),
    (33, 0x04000420, 3, 2, 4, "ライト"),
    (34, 0x44000420, 4, 3, 260, "シャドー"),
    (35, 0x44000420, 2, 2, 260, "縁取り"),
    (36, 0x04000020, 3, 0, 0, "凸エッジ"),
    (37, 0x04000420, 2, 3, 4, "エッジ抽出"),
    (38, 0x04000020, 2, 0, 0, "シャープ"),
    (39, 0x04000020, 2, 0, 0, "フェード"),
    (40, 0x04000420, 3, 3, 260, "ワイプ"),
    (41, 0x04000620, 6, 3, 264, "マスク"),
    (42, 0x04000020, 5, 0, 0, "斜めクリッピング"),
    (43, 0x04000020, 3, 1, 0, "放射ブラー"),
    (44, 0x44000000, 3, 0, 0, "放射ブラー"),
    (45, 0x04000020, 2, 1, 0, "方向ブラー"),
    (46, 0x44000000, 2, 0, 0, "方向ブラー"),
    (47, 0x04000020, 2, 1, 0, "レンズブラー"),
    (48, 0x04000000, 2, 0, 0, "レンズブラー"),
    (49, 0x04000020, 2, 3, 0, "モーションブラー"),
    (50, 0x04000000, 2, 1, 0, "モーションブラー"),
    (51, 0x04008020, 3, 0, 0, "座標"),
    (52, 0x04008020, 3, 0, 0, "拡大率"),
    (53, 0x04008020, 1, 0, 0, "透明度"),
    (54, 0x04008020, 3, 0, 0, "回転"),
    (55, 0x04008020, 4, 1, 0, "領域拡張"),
    (56, 0x04008020, 3, 2, 0, "リサイズ"),
    (57, 0x04008020, 1, 0, 0, "ローテーション"),
    (58, 0x04000020, 4, 2, 0, "振動"),
    (59, 0x04000000, 4, 2, 0, "振動"),
    (60, 0x04008020, 0, 5, 0, "反転"),
    (61, 0x04000000, 0, 4, 0, "反転"),
    (62, 0x04000420, 3, 2, 4, "ミラー"),
    (63, 0x04000020, 3, 2, 0, "ラスター"),
    (64, 0x04000000, 3, 2, 0, "ラスター"),
    (65, 0x04000420, 5, 1, 12, "波紋"),
    (66, 0x04000020, 4, 1, 0, "画像ループ"),
    (67, 0x04000000, 4, 0, 0, "画像ループ"),
    (68, 0x04000020, 4, 0, 0, "極座標変換"),
    (69, 0x04000620, 8, 3, 268, "ディスプレイスメントマップ"),
    (70, 0x04000420, 7, 3, 16, "ノイズ"),
    (71, 0x04000420, 3, 1, 4, "色ずれ"),
    (72, 0x04000400, 3, 1, 4, "色ずれ"),
    (73, 0x04000420, 1, 2, 4, "単色化"),
    (74, 0x04000400, 1, 2, 4, "単色化"),
    (75, 0x04000420, 5, 4, 16, "グラデーション"),
    (76, 0x04000220, 3, 3, 0, "拡張色設定"),
    (77, 0x04000200, 3, 3, 0, "拡張色設定"),
    (78, 0x04000420, 3, 2, 16, "特定色域変換"),
    (79, 0x04000420, 4, 2, 516, "アニメーション効果"),
    (80, 0x04000408, 4, 2, 516, "カスタムオブジェクト"),
    (81, 0x04000420, 0, 0, 2048, "スクリプト制御"),
    (82, 0x04000420, 5, 5, 284, "動画ファイル合成"),
    (83, 0x04000420, 3, 3, 260, "画像ファイル合成"),
    (84, 0x04000420, 0, 1, 4, "インターレース解除"),
    (85, 0x04000020, 0, 4, 0, "カメラ制御オプション"),
    (86, 0x04000020, 0, 0, 0, "オフスクリーン描画"),
    (87, 0x04000020, 2, 0, 0, "オブジェクト分割"),
    (88, 0x44000500, 6, 2, 260, "部分フィルタ"),
    (89, 0x04200020, 2, 0, 0, "音量フェード"),
    (90, 0x04200020, 2, 0, 0, "音声ディレイ"),
    (91, 0x04200000, 2, 0, 0, "音声ディレイ"),
    (92, 0x04200020, 1, 0, 0, "モノラル化"),
    (93, 0x05000400, 3, 1, 20, "時間制御"),
    (94, 0x45000420, 7, 2, 20, "グループ制御"),
    (95, 0x45800400, 10, 1, 20, "カメラ制御"),
    (96, 0x45800400, 8, 2, 4, "カメラ制御(拡張描画)"),
    (97, 0x05000400, 4, 2, 516, "カメラ効果"),
    (98, 0x45000000, 5, 0, 0, "シャドー(カメラ制御)"),
    (99, 0x05000400, 0, 0, 2048, "スクリプト(カメラ制御)"),
    (100, 0x02000000, 3, 0, 0, "ノイズ除去フィルタ"),
    (101, 0x02000000, 4, 0, 0, "シャープフィルタ"),
    (102, 0x02000000, 4, 0, 0, "ぼかしフィルタ"),
    (103, 0x02004400, 4, 0, 12, "クリッピング＆リサイズ"),
    (104, 0x02000000, 4, 2, 0, "縁塗りつぶし"),
    (105, 0x02001000, 6, 0, 0, "色調補正"),
    (106, 0x02001000, 15, 3, 0, "拡張色調補正"),
    (107, 0x02200000, 1, 0, 0, "音量の調整"),
];

fn encode_sjis_fixed(s: &str, max_len: usize) -> Vec<u8> {
    let mut out = vec![0u8; max_len];
    if max_len == 0 {
        return out;
    }
    let (cow, _, _) = encoding_rs::SHIFT_JIS.encode(s);
    let copy_len = cow.len().min(max_len.saturating_sub(1));
    out[..copy_len].copy_from_slice(&cow[..copy_len]);
    out
}

/// AviUtl 本体で「拡張編集」および「拡張編集(音声)」を有効化する 2005 バイトの `#temp0` `FilterConfig` を生成します。
fn build_default_filter_config(width: u32, height: u32) -> Vec<u8> {
    let mut cfg = vec![0u8; 2005];
    let patches: &[(usize, &[u8])] = &[
        (0, &[0x41, 0x76, 0x69, 0x55, 0x74, 0x6C, 0x20, 0x46, 0x69, 0x6C, 0x74, 0x65, 0x72, 0x43, 0x6F, 0x6E, 0x66, 0x69, 0x67, 0x46, 0x69, 0x6C, 0x65, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x30, 0x2E, 0x31, 0x00, 0x2F, 0x00]),
        (40, &[0x8D, 0xC4, 0x90, 0xB6, 0x83, 0x45, 0x83, 0x42, 0x83, 0x93, 0x83, 0x68, 0x83, 0x45, 0x00, 0x04, 0x00]),
        (59, &[0x04, 0x00]),
        (63, &[0x04, 0x00]),
        (75, &[0x05, 0xD0, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0x00]),
        (87, &[0x83, 0x63, 0x81, 0x5B, 0x83, 0x8B, 0x83, 0x45, 0x83, 0x42, 0x83, 0x93, 0x83, 0x68, 0x83, 0x45, 0x00, 0x04, 0x00]),
        (108, &[0x04, 0x00]),
        (112, &[0x04, 0x00]),
        (124, &[0x05, 0xD0, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x2D, 0x00]),
        (136, &[0x83, 0x71, 0x83, 0x58, 0x83, 0x67, 0x83, 0x4F, 0x83, 0x89, 0x83, 0x80, 0x00, 0x04, 0x00]),
        (153, &[0x04, 0x00]),
        (157, &[0x04, 0x00]),
        (169, &[0x05, 0xD0, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x3F, 0x00]),
        (181, &[0x83, 0x6D, 0x83, 0x43, 0x83, 0x59, 0x8F, 0x9C, 0x8B, 0x8E, 0x83, 0x74, 0x83, 0x42, 0x83, 0x8B, 0x83, 0x5E, 0x00, 0x04, 0x00]),
        (204, &[0x04, 0x00]),
        (208, &[0x04, 0x00]),
        (212, &[0x0C, 0x00]),
        (223, &[0x02, 0x01, 0x00]),
        (229, &[0x01, 0x00]),
        (232, &[0x02, 0x00]),
        (236, &[0x18, 0x00]),
        (240, &[0x47, 0x00]),
        (244, &[0x83, 0x6D, 0x83, 0x43, 0x83, 0x59, 0x8F, 0x9C, 0x8B, 0x8E, 0x28, 0x8E, 0x9E, 0x8A, 0xD4, 0x8E, 0xB2, 0x29, 0x83, 0x74, 0x83, 0x42, 0x83, 0x8B, 0x83, 0x5E, 0x00, 0x04, 0x00]),
        (275, &[0x04, 0x00]),
        (279, &[0x04, 0x00]),
        (283, &[0x0C, 0x00]),
        (292, &[0x08, 0x00, 0x0A, 0x00]),
        (300, &[0x01, 0x00]),
        (303, &[0x03, 0x00]),
        (307, &[0x24, 0x00]),
        (311, &[0x41, 0x00]),
        (315, &[0x83, 0x56, 0x83, 0x83, 0x81, 0x5B, 0x83, 0x76, 0x83, 0x74, 0x83, 0x42, 0x83, 0x8B, 0x83, 0x5E, 0x00, 0x04, 0x00]),
        (336, &[0x04, 0x00]),
        (340, &[0x04, 0x00]),
        (344, &[0x10, 0x00]),
        (355, &[0x02, 0x02, 0x00]),
        (361, &[0x01, 0x00]),
        (364, &[0x02, 0x00]),
        (368, &[0x80, 0x00]),
        (373, &[0x04, 0x00]),
        (376, &[0x3F, 0x00]),
        (380, &[0x82, 0xDA, 0x82, 0xA9, 0x82, 0xB5, 0x83, 0x74, 0x83, 0x42, 0x83, 0x8B, 0x83, 0x5E, 0x00, 0x04, 0x00]),
        (399, &[0x04, 0x00]),
        (403, &[0x04, 0x00]),
        (407, &[0x10, 0x00]),
        (418, &[0x02, 0x03, 0x00]),
        (424, &[0x01, 0x00]),
        (427, &[0x02, 0x00]),
        (436, &[0x04, 0x00]),
        (439, &[0x57, 0x00]),
        (443, &[0x83, 0x4E, 0x83, 0x8A, 0x83, 0x62, 0x83, 0x73, 0x83, 0x93, 0x83, 0x4F, 0x81, 0x95, 0x83, 0x8A, 0x83, 0x54, 0x83, 0x43, 0x83, 0x59, 0x00, 0x05, 0x00]),
        (470, &[0x04, 0x00]),
        (474, &[0x04, 0x00]),
        (478, &[0x10, 0x00]),
        (486, &[0x0C, 0x00]),
        (491, &[0x44, 0x00, 0x02, 0x04, 0x00]),
        (526, &[0x45, 0x00]),
        (530, &[0x89, 0x8F, 0x93, 0x68, 0x82, 0xE8, 0x82, 0xC2, 0x82, 0xD4, 0x82, 0xB5, 0x00, 0x04, 0x00]),
        (547, &[0x04, 0x00]),
        (551, &[0x04, 0x00]),
        (555, &[0x10, 0x00]),
        (559, &[0x08, 0x00]),
        (566, &[0x02, 0x05, 0x00]),
        (595, &[0x41, 0x00]),
        (599, &[0x90, 0x46, 0x92, 0xB2, 0x95, 0xE2, 0x90, 0xB3, 0x00, 0x04, 0x00]),
        (612, &[0x04, 0x00]),
        (616, &[0x04, 0x00]),
        (620, &[0x18, 0x00]),
        (629, &[0x10, 0x00, 0x02, 0x7F, 0x00]),
        (660, &[0x75, 0x00]),
        (664, &[0x8A, 0x67, 0x92, 0xA3, 0x90, 0x46, 0x92, 0xB2, 0x95, 0xE2, 0x90, 0xB3, 0x00, 0x04, 0x00]),
        (681, &[0x04, 0x00]),
        (685, &[0x04, 0x00]),
        (689, &[0x3C, 0x00]),
        (693, &[0x0C, 0x00]),
        (698, &[0x10, 0x00, 0x02, 0x7F, 0x00]),
        (777, &[0x33, 0x00]),
        (781, &[0x89, 0xB9, 0x90, 0xBA, 0x82, 0xCC, 0x88, 0xCA, 0x92, 0x75, 0x92, 0xB2, 0x90, 0xAE, 0x00, 0x04, 0x00]),
        (800, &[0x04, 0x00]),
        (804, &[0x04, 0x00]),
        (808, &[0x04, 0x00]),
        (817, &[0x40, 0x20, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]),
        (828, &[0x2F, 0x00]),
        (832, &[0x89, 0xB9, 0x97, 0xCA, 0x82, 0xCC, 0x92, 0xB2, 0x90, 0xAE, 0x00, 0x04, 0x00]),
        (847, &[0x04, 0x00]),
        (851, &[0x04, 0x00]),
        (855, &[0x04, 0x00]),
        (865, &[0x20, 0x02, 0x06, 0x00]),
        (875, &[0x31, 0x00]),
        (879, &[0x89, 0xB9, 0x97, 0xCA, 0x82, 0xCC, 0x8D, 0xC5, 0x91, 0xE5, 0x89, 0xBB, 0x00, 0x04, 0x00]),
        (896, &[0x04, 0x00]),
        (900, &[0x04, 0x00]),
        (904, &[0x04, 0x00]),
        (913, &[0x08, 0x20, 0x02, 0x00]),
        (921, &[0x01, 0x00]),
        (924, &[0x55, 0x00]),
        (928, &[0x83, 0x54, 0x83, 0x43, 0x83, 0x59, 0x82, 0xCC, 0x95, 0xCF, 0x8D, 0x58, 0x00, 0x05, 0x00]),
        (945, &[0x04, 0x00]),
        (949, &[0x04, 0x00]),
        (957, &[0x1C, 0x00]),
        (961, &[0x08, 0x00]),
        (965, &[0x2D, 0x06, 0x00, 0x02, 0x07, 0x00]),
        (973, &[0x01, 0x00]),
        (1009, &[0x4D, 0x00]),
        (1013, &[0x83, 0x74, 0x83, 0x8C, 0x81, 0x5B, 0x83, 0x80, 0x83, 0x8C, 0x81, 0x5B, 0x83, 0x67, 0x82, 0xCC, 0x95, 0xCF, 0x8D, 0x58, 0x00, 0x04, 0x00]),
        (1038, &[0x04, 0x00]),
        (1042, &[0x04, 0x00]),
        (1050, &[0x18, 0x00]),
        (1054, &[0xAD, 0x02, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00]),
        (1086, &[0x39, 0x00]),
        (1090, &[0x83, 0x74, 0x83, 0x8C, 0x81, 0x5B, 0x83, 0x80, 0x83, 0x8C, 0x81, 0x5B, 0x83, 0x67, 0x82, 0xCC, 0x90, 0xDD, 0x92, 0xE8, 0x00, 0x04, 0x00]),
        (1115, &[0x04, 0x00]),
        (1119, &[0x04, 0x00]),
        (1127, &[0x04, 0x00]),
        (1131, &[0x15, 0x03, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]),
        (1143, &[0x51, 0x01, 0x00]),
        (1147, &[0x83, 0x43, 0x83, 0x93, 0x83, 0x5E, 0x81, 0x5B, 0x83, 0x8C, 0x81, 0x5B, 0x83, 0x58, 0x82, 0xCC, 0x89, 0xF0, 0x8F, 0x9C, 0x00, 0x05, 0x00]),
        (1172, &[0x04, 0x00]),
        (1176, &[0x04, 0x00]),
        (1184, &[0x18, 0x00]),
        (1189, &[0x01, 0x00]),
        (1192, &[0xAD, 0x06, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00]),
        (1224, &[0x82, 0xC8, 0x82, 0xB5, 0x00]),
        (1480, &[0x3B, 0x00]),
        (1484, &[0x83, 0x74, 0x83, 0x42, 0x81, 0x5B, 0x83, 0x8B, 0x83, 0x68, 0x83, 0x49, 0x81, 0x5B, 0x83, 0x5F, 0x81, 0x5B, 0x00, 0x04, 0x00]),
        (1507, &[0x04, 0x00]),
        (1511, &[0x04, 0x00]),
        (1519, &[0x08, 0x00]),
        (1523, &[0xA5, 0x03, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]),
        (1535, &[0x01, 0x00]),
        (1539, &[0x25, 0x00]),
        (1543, &[0x82, 0xC8, 0x82, 0xB5, 0x00, 0x04, 0x00]),
        (1552, &[0x04, 0x00]),
        (1556, &[0x04, 0x00]),
        (1568, &[0x05, 0x00, 0x10, 0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0x2D, 0x00]),
        (1580, &[0x8A, 0xEF, 0x90, 0x94, 0x00, 0x04, 0x00]),
        (1589, &[0x04, 0x00]),
        (1593, &[0x04, 0x00]),
        (1601, &[0x08, 0x00]),
        (1605, &[0x05, 0x00]),
        (1608, &[0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00]),
        (1617, &[0x01, 0x00]),
        (1621, &[0x2D, 0x00]),
        (1625, &[0x8B, 0xF4, 0x90, 0x94, 0x00, 0x04, 0x00]),
        (1634, &[0x04, 0x00]),
        (1638, &[0x04, 0x00]),
        (1646, &[0x08, 0x00]),
        (1650, &[0x05, 0x00]),
        (1653, &[0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00]),
        (1662, &[0x01, 0x00]),
        (1666, &[0x27, 0x00]),
        (1670, &[0x93, 0xF1, 0x8F, 0x64, 0x89, 0xBB, 0x00, 0x04, 0x00]),
        (1681, &[0x04, 0x00]),
        (1685, &[0x04, 0x00]),
        (1697, &[0x05, 0x00, 0x10, 0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0x36, 0x00]),
        (1709, &[0x8E, 0xA9, 0x93, 0xAE, 0x32, 0x34, 0x66, 0x70, 0x73, 0x00, 0x04, 0x00]),
        (1723, &[0x04, 0x00]),
        (1727, &[0x04, 0x00]),
        (1731, &[0x08, 0x00]),
        (1735, &[0x04, 0x00]),
        (1739, &[0x05, 0x00]),
        (1742, &[0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0x40, 0x00]),
        (1751, &[0x10, 0x00]),
        (1759, &[0x25, 0x00]),
        (1763, &[0x8E, 0xA9, 0x93, 0xAE, 0x00, 0x04, 0x00]),
        (1772, &[0x04, 0x00]),
        (1776, &[0x04, 0x00]),
        (1788, &[0x05, 0x00, 0x10, 0x06, 0xFF, 0xFF, 0xFF, 0xFF, 0x3D, 0x00]),
        (1800, &[0x90, 0x46, 0x95, 0xCF, 0x8A, 0xB7, 0x82, 0xCC, 0x90, 0xDD, 0x92, 0xE8, 0x00, 0x04, 0x00]),
        (1817, &[0x04, 0x00]),
        (1821, &[0x04, 0x00]),
        (1829, &[0x10, 0x00]),
        (1833, &[0xAD, 0x02, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]),
        (1853, &[0x01, 0x00]),
        (1857, &[0x3C, 0x00]),
        (1861, &[0x5F, 0x5F, 0x43, 0x4F, 0x4C, 0x4F, 0x52, 0x5F, 0x4F, 0x55, 0x54, 0x00, 0x04, 0x00]),
        (1877, &[0x04, 0x00]),
        (1881, &[0x04, 0x00]),
        (1889, &[0x10, 0x00]),
        (1893, &[0x25, 0x03, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]),
        (1913, &[0x01, 0x00]),
        (1917, &[0x29, 0x00]),
        (1921, &[0x8A, 0x67, 0x92, 0xA3, 0x95, 0xD2, 0x8F, 0x57, 0x00, 0x04, 0x00]),
        (1934, &[0x04, 0x00]),
        (1938, &[0x04, 0x00]),
        (1950, &[0x07, 0x20, 0x06, 0x40, 0x08, 0x00]),
        (1958, &[0x2F, 0x00]),
        (1962, &[0x8A, 0x67, 0x92, 0xA3, 0x95, 0xD2, 0x8F, 0x57, 0x28, 0x89, 0xB9, 0x90, 0xBA, 0x29, 0x00, 0x04, 0x00]),
        (1981, &[0x04, 0x00]),
        (1985, &[0x04, 0x00]),
        (1997, &[0x05, 0x00, 0x35, 0x00, 0x09, 0x00]),
    ];
    for &(off, bytes) in patches {
        cfg[off..off + bytes.len()].copy_from_slice(bytes);
    }
    cfg[1001..1005].copy_from_slice(&(width as i32).to_le_bytes());
    cfg[1005..1009].copy_from_slice(&(height as i32).to_le_bytes());
    cfg
}

/// 単一シーンを本家 AviUtl 1.10 + 拡張編集 0.92 互換の `.aup` バイナリとして出力します。
pub fn write_legacy_aup(scene: &Scene) -> Vec<u8> {
    let mut scenes = std::collections::BTreeMap::new();
    scenes.insert(0, scene.clone());
    write_legacy_aup_multi_scene(&scenes, 0, 0)
}

/// マルチシーンプロジェクトを本家 AviUtl 1.10 (`EditHandle` + `#temp0`) + 拡張編集 0.92 (`80EE`) 完全互換の `.aup` バイナリとして出力します。
pub fn write_legacy_aup_multi_scene(
    scenes: &std::collections::BTreeMap<u32, Scene>,
    editing_scene: u32,
    cursor_frame: au2_core::Frame,
) -> Vec<u8> {
    let fallback_scene = Scene::new("Root", SceneSettings::default());
    let main_scene = scenes
        .get(&editing_scene)
        .or_else(|| scenes.get(&0))
        .or_else(|| scenes.values().next())
        .unwrap_or(&fallback_scene);

    let width = main_scene.settings.width.max(16);
    let height = main_scene.settings.height.max(16);
    let video_rate = main_scene.settings.frame_rate.num.max(1);
    let video_scale = main_scene.settings.frame_rate.den.max(1);
    let audio_rate = main_scene.settings.audio_rate.max(8000);
    let audio_ch = main_scene.settings.audio_channels.max(1);
    let frame_num = main_scene
        .settings
        .length
        .max(main_scene.content_end())
        .max(1);

    let edit_filename = if video_scale == 1 {
        format!("{}x{}_{}fps_{}Hz.exedit", width, height, video_rate, audio_rate)
    } else {
        format!(
            "{}x{}_{}-{}fps_{}Hz.exedit",
            width, height, video_rate, video_scale, audio_rate
        )
    };

    let mut out = Vec::with_capacity(65536);

    // 1. 先頭ヘッダ (32 bytes)
    out.extend_from_slice(crate::AUP_HEADER_018);

    // 2. EditHandle チャンク
    out.extend_from_slice(&0x004C_09E8i32.to_le_bytes()); // Size
    out.extend_from_slice(&0x0000_0001u32.to_le_bytes()); // Flag (EDIT_HANDLE_FLAG_OPEN)
    out.extend_from_slice(&encode_sjis_fixed(&edit_filename, 260)); // EditFilename
    out.extend_from_slice(&[0u8; 260]); // OutputFilename

    // EditHandle.Data (0x4C09E8 - 0x20C = 0x4C07DC bytes)
    const EH_UNCOMP_OFFSET: usize = 0x20C;
    const EH_DATA_LEN: usize = 0x4C09E8 - EH_UNCOMP_OFFSET;
    let mut eh_data = vec![0u8; EH_DATA_LEN];
    let put_u32 = |buf: &mut [u8], struct_off: usize, val: u32| {
        let rel = struct_off - EH_UNCOMP_OFFSET;
        buf[rel..rel + 4].copy_from_slice(&val.to_le_bytes());
    };
    let samples_per_frame =
        ((audio_rate as u64 * video_scale as u64) / (video_rate as u64).max(1)) as u32;

    put_u32(&mut eh_data, 0x000310, width);
    put_u32(&mut eh_data, 0x000314, height);
    put_u32(&mut eh_data, 0x000318, frame_num);
    put_u32(&mut eh_data, 0x00031C, 0);
    put_u32(&mut eh_data, 0x000320, frame_num.saturating_sub(1));
    put_u32(&mut eh_data, 0x000328, width);
    put_u32(&mut eh_data, 0x00032C, height);
    put_u32(&mut eh_data, 0x000330, cursor_frame.min(frame_num.saturating_sub(1)));
    put_u32(&mut eh_data, 0x000338, 40);
    put_u32(&mut eh_data, 0x00033C, width);
    put_u32(&mut eh_data, 0x000340, height);
    put_u32(&mut eh_data, 0x000344, 0x0018_0001);
    put_u32(&mut eh_data, 0x00034C, width.saturating_mul(height).saturating_mul(3));
    put_u32(&mut eh_data, 0x000368, samples_per_frame.saturating_mul(2));
    put_u32(&mut eh_data, 0x00036C, 0x0008_000D);
    put_u32(&mut eh_data, 0x000378, width / 2);
    put_u32(&mut eh_data, 0x00037C, 636);
    put_u32(&mut eh_data, 0x000380, 1);
    put_u32(&mut eh_data, 0x0003A8, width / 2);
    put_u32(&mut eh_data, 0x0003AC, height / 2);
    put_u32(&mut eh_data, 0x0003B0, width);
    put_u32(&mut eh_data, 0x0003B4, height);

    for base in [0x0003C0usize, 0x026F9Cusize] {
        put_u32(&mut eh_data, base + 0x0000, frame_num);
        put_u32(&mut eh_data, base + 0x0004, width);
        put_u32(&mut eh_data, base + 0x0008, height);
        put_u32(&mut eh_data, base + 0x0010, 40);
        put_u32(&mut eh_data, base + 0x0014, width);
        put_u32(&mut eh_data, base + 0x0018, height);
        put_u32(&mut eh_data, base + 0x001C, 0x0030_0001);
        put_u32(&mut eh_data, base + 0x0020, 0x3834_4359); // 'YC48'
        put_u32(&mut eh_data, base + 0x0024, width.saturating_mul(height).saturating_mul(6));
        put_u32(&mut eh_data, base + 0x0038, 0x0001 | (audio_ch << 16));
        put_u32(&mut eh_data, base + 0x003C, audio_rate);
        put_u32(&mut eh_data, base + 0x0040, audio_rate * audio_ch * 2);
        put_u32(&mut eh_data, base + 0x0044, (audio_ch * 2) | (16 << 16));
        put_u32(&mut eh_data, base + 0x0050, 3);
        put_u32(&mut eh_data, base + 0x0054, video_rate);
        put_u32(&mut eh_data, base + 0x0058, 1);
        put_u32(&mut eh_data, base + 0x005C, 1);
        put_u32(&mut eh_data, base + 0x0064, 40);
        put_u32(&mut eh_data, base + 0x0068, 0x7FFF_FFFF);
        put_u32(&mut eh_data, base + 0x0070, 18);
        put_u32(&mut eh_data, base + 0x0074, 0x3834_4359); // 'YC48'
        put_u32(&mut eh_data, base + 0x0094, 0x7364_6976); // 'vids'
        put_u32(&mut eh_data, base + 0x0098, 0x3834_4359); // 'YC48'
        put_u32(&mut eh_data, base + 0x00A8, video_scale);
        put_u32(&mut eh_data, base + 0x00AC, video_rate);
        put_u32(&mut eh_data, base + 0x00B4, 1);
        put_u32(&mut eh_data, base + 0x0120, 0x7364_7561); // 'auds'
        put_u32(&mut eh_data, base + 0x0134, audio_ch * 2);
        put_u32(&mut eh_data, base + 0x0138, audio_rate * audio_ch * 2);
        put_u32(&mut eh_data, base + 0x0140, 0x7FFF_FFFF);
        put_u32(&mut eh_data, base + 0x0150, audio_ch * 2);
        put_u32(&mut eh_data, base + 0x01AC, 40);
        put_u32(&mut eh_data, base + 0x01B0, width);
        put_u32(&mut eh_data, base + 0x01B4, height);
        put_u32(&mut eh_data, base + 0x01B8, 0x0030_0001);
        put_u32(&mut eh_data, base + 0x01BC, 0x3834_4359);
        put_u32(&mut eh_data, base + 0x01C0, width.saturating_mul(height).saturating_mul(6));
        put_u32(&mut eh_data, base + 0x21AC, 40);
        put_u32(&mut eh_data, base + 0x21B0, 0x0001 | (audio_ch << 16));
        put_u32(&mut eh_data, base + 0x21B4, audio_rate);
        put_u32(&mut eh_data, base + 0x21B8, audio_rate * audio_ch * 2);
        put_u32(&mut eh_data, base + 0x21BC, (audio_ch * 2) | (16 << 16));
        put_u32(&mut eh_data, base + 0x41B0, 18);
    }

    put_u32(&mut eh_data, 0x00C75C, samples_per_frame);
    put_u32(&mut eh_data, 0x020CF4, 1);
    eh_data[0x020D18 - EH_UNCOMP_OFFSET..0x020D18 - EH_UNCOMP_OFFSET + 7]
        .copy_from_slice(b"#temp0\0");
    let fn_bytes = encode_sjis_fixed(&edit_filename, 260);
    eh_data[0x026E98 - EH_UNCOMP_OFFSET..0x026E98 - EH_UNCOMP_OFFSET + 260]
        .copy_from_slice(&fn_bytes);
    eh_data[0x4BBD98 - EH_UNCOMP_OFFSET..0x4BC198 - EH_UNCOMP_OFFSET].fill(0xFF);
    let out_plugin_sjis: &[u8] = &[
        0x8A, 0x67, 0x92, 0xA3, 0x95, 0xD2, 0x8F, 0x57, 0x41, 0x56, 0x49, 0x2F, 0x42, 0x4D, 0x50,
        0x8F, 0x6F, 0x97, 0xCD, 0x20, 0x28, 0x52, 0x47, 0x42, 0x41, 0x29, 0x00,
    ];
    eh_data[0x4BC49C - EH_UNCOMP_OFFSET..0x4BC49C - EH_UNCOMP_OFFSET + out_plugin_sjis.len()]
        .copy_from_slice(out_plugin_sjis);

    out.extend_from_slice(&compress_aup(&eh_data));

    // FrameStatus (10 配列)
    let fn_usize = frame_num as usize;
    out.extend_from_slice(&(frame_num as i32).to_le_bytes());
    let zero_u32_bytes = vec![0u8; fn_usize * 4];
    let mut audio_u32_bytes = Vec::with_capacity(fn_usize * 4);
    for i in 0..frame_num {
        audio_u32_bytes.extend_from_slice(&i.to_le_bytes());
    }
    let zero_u8_bytes = vec![0u8; fn_usize];
    let mut edit_flag_bytes = vec![0u8; fn_usize];
    if !edit_flag_bytes.is_empty() {
        edit_flag_bytes[0] = 1; // 先頭フレームにキーフレームフラグ
    }

    out.extend_from_slice(&compress_aup(&zero_u32_bytes)); // videos
    out.extend_from_slice(&compress_aup(&audio_u32_bytes)); // audios
    out.extend_from_slice(&compress_aup(&zero_u32_bytes)); // field2
    out.extend_from_slice(&compress_aup(&zero_u32_bytes)); // field3
    out.extend_from_slice(&compress_aup(&zero_u8_bytes)); // inter
    out.extend_from_slice(&compress_aup(&zero_u8_bytes)); // index24fps
    out.extend_from_slice(&compress_aup(&edit_flag_bytes)); // editFlag
    out.extend_from_slice(&compress_aup(&zero_u8_bytes)); // config (#temp0 = 0)
    out.extend_from_slice(&compress_aup(&zero_u8_bytes)); // vcm
    out.extend_from_slice(&compress_aup(&zero_u8_bytes)); // clip

    // FilterConfigs[0] (#temp0)
    let filter_cfg = build_default_filter_config(width, height);
    out.extend_from_slice(&(filter_cfg.len() as i32).to_le_bytes());
    out.extend_from_slice(&filter_cfg);

    // EditHandle 終端フッタ (32 bytes)
    out.extend_from_slice(crate::AUP_HEADER_018);

    // 3. FilterProject 0.1 "拡張編集" (80EE) チャンク
    out.extend_from_slice(b"FilterProject 0.1\0");
    let exedit_name_sjis: &[u8] = &[0x8A, 0x67, 0x92, 0xA3, 0x95, 0xD2, 0x8F, 0x57, 0x00];
    out.extend_from_slice(&(exedit_name_sjis.len() as i32).to_le_bytes());
    out.extend_from_slice(exedit_name_sjis);

    let exedit_data = build_exedit_80ee_bytes(scenes, editing_scene, cursor_frame);
    out.extend_from_slice(&(exedit_data.len() as i32).to_le_bytes());
    out.extend_from_slice(&compress_aup(&exedit_data));

    out
}

fn build_exedit_80ee_bytes(
    scenes: &std::collections::BTreeMap<u32, Scene>,
    editing_scene: u32,
    cursor_frame: au2_core::Frame,
) -> Vec<u8> {
    // 1. Layers (76 bytes each)
    let mut layer_records: Vec<[u8; 76]> = Vec::new();
    for (&sc_id, sc) in scenes {
        if sc_id >= 50 {
            continue;
        }
        for (li, layer) in sc.layers.iter().enumerate() {
            if li >= 100 {
                break;
            }
            let has_name = layer.name.as_ref().is_some_and(|n| !n.is_empty());
            if has_name || !layer.visible || layer.locked || layer.coordinate_link || layer.clip_above {
                let mut rec = [0u8; 76];
                rec[0..4].copy_from_slice(&sc_id.to_le_bytes());
                rec[4..8].copy_from_slice(&(li as u32).to_le_bytes());
                let mut flag = 0u32;
                if !layer.visible {
                    flag |= 1;
                }
                if layer.locked {
                    flag |= 2;
                }
                if layer.coordinate_link {
                    flag |= 0x10;
                }
                if layer.clip_above {
                    flag |= 0x20;
                }
                rec[8..12].copy_from_slice(&flag.to_le_bytes());
                if let Some(name) = &layer.name {
                    let sjis = encode_sjis_fixed(name, 64);
                    rec[12..76].copy_from_slice(&sjis);
                }
                layer_records.push(rec);
            }
        }
    }

    // 2. Scenes (50 scenes * 220 bytes each)
    let mut scene_records: Vec<[u8; 220]> = Vec::with_capacity(50);
    for sc_id in 0..50u32 {
        let mut rec = [0u8; 220];
        rec[0..4].copy_from_slice(&sc_id.to_le_bytes());
        let flag = 1u32; // SceneFlag.Base
        rec[4..8].copy_from_slice(&flag.to_le_bytes());
        if let Some(sc) = scenes.get(&sc_id) {
            let default_name = format!("Scene {}", sc_id);
            if sc_id > 0 && !sc.name.is_empty() && sc.name != "Root" && sc.name != default_name {
                let sjis = encode_sjis_fixed(&sc.name, 64);
                rec[8..72].copy_from_slice(&sjis);
            }
            if sc_id > 0 {
                rec[0x48..0x4C].copy_from_slice(&sc.settings.width.to_le_bytes());
                rec[0x4C..0x50].copy_from_slice(&sc.settings.height.to_le_bytes());
                rec[0x50..0x54].copy_from_slice(&sc.settings.length.to_le_bytes());
            }
        }
        let cur = if sc_id == editing_scene { cursor_frame } else { 0 };
        rec[0x54..0x58].copy_from_slice(&cur.to_le_bytes());
        rec[0x58..0x5C].copy_from_slice(&12u32.to_le_bytes()); // Zoom
        rec[0x60..0x64].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // EditingObject
        rec[0x70..0x74].copy_from_slice(&1_200_000u32.to_le_bytes()); // BpmGridTempo
        rec[0x74..0x78].copy_from_slice(&1u32.to_le_bytes()); // BpmGridOffset
        rec[0x7C..0x80].copy_from_slice(&200u32.to_le_bytes()); // XYGridWidth
        rec[0x80..0x84].copy_from_slice(&200u32.to_le_bytes()); // XYGridHeight
        rec[0x88..0x8C].copy_from_slice(&200u32.to_le_bytes()); // CameraGridSize
        rec[0x8C..0x90].copy_from_slice(&10u32.to_le_bytes()); // CameraGridNum
        rec[0x94..0x98].copy_from_slice(&18u32.to_le_bytes()); // OutsideFrameScale
        rec[0x98..0x9C].copy_from_slice(&4u32.to_le_bytes()); // BpmGridBeat
        scene_records.push(rec);
    }

    // 3. TrackbarScripts & Objects
    let mut trackbar_scripts: Vec<String> = vec!["補間移動".to_string(), "回転".to_string()];
    let mut obj_blobs: Vec<Vec<u8>> = Vec::new();
    let mut obj_counter: u32 = 0;

    for (&sc_id, sc) in scenes {
        if sc_id >= 50 {
            continue;
        }
        let fps = sc.settings.frame_rate.as_f64().max(1.0);
        for (li, layer) in sc.layers.iter().enumerate() {
            if li >= 100 {
                break;
            }
            let mut sorted_objs: Vec<&au2_core::TimelineObject> = layer.objects.iter().collect();
            sorted_objs.sort_by_key(|o| o.frame_start);
            for obj in sorted_objs {
                encode_timeline_object_to_80ee(
                    obj,
                    sc_id,
                    li as u32,
                    editing_scene,
                    fps,
                    &mut trackbar_scripts,
                    &mut obj_blobs,
                    &mut obj_counter,
                );
            }
        }
    }

    // 4. EffectTypes (108 * 112 bytes)
    let mut effect_type_bytes = Vec::with_capacity(DEFAULT_EFFECT_TYPES.len() * 112);
    for &(_id, flag, tb_num, cb_num, ext_size, name) in DEFAULT_EFFECT_TYPES {
        let mut et = [0u8; 112];
        et[0..4].copy_from_slice(&flag.to_le_bytes());
        et[4..8].copy_from_slice(&(tb_num as u32).to_le_bytes());
        et[8..12].copy_from_slice(&(cb_num as u32).to_le_bytes());
        et[12..16].copy_from_slice(&(ext_size as u32).to_le_bytes());
        let sjis = encode_sjis_fixed(name, 96);
        et[16..112].copy_from_slice(&sjis);
        effect_type_bytes.extend_from_slice(&et);
    }

    // 5. 80EE 全体バッファの構築
    let total_size = 0x100
        + layer_records.len() * 76
        + scene_records.len() * 220
        + trackbar_scripts.len() * 128
        + effect_type_bytes.len()
        + obj_blobs.iter().map(|b| b.len()).sum::<usize>();
    let mut data = vec![0u8; 0x100];
    data.reserve(total_size.saturating_sub(0x100));

    data[0..4].copy_from_slice(b"80EE");
    data[4..8].copy_from_slice(&(DEFAULT_EFFECT_TYPES.len() as u32).to_le_bytes());
    data[8..12].copy_from_slice(&(obj_blobs.len() as u32).to_le_bytes());
    data[0x0C..0x10].copy_from_slice(&0x0001_1A8Bu32.to_le_bytes());
    data[0x10..0x14].copy_from_slice(&12u32.to_le_bytes()); // Zoom
    data[0x14..0x18].copy_from_slice(&400u32.to_le_bytes());
    data[0x18..0x1C].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // EditingObject
    data[0x1C..0x20].copy_from_slice(&0x0000_05C8u32.to_le_bytes()); // BaseSize = 0x5C8
    data[0x20..0x24].copy_from_slice(&5000u32.to_le_bytes());
    data[0x24..0x28].copy_from_slice(&1_000_000u32.to_le_bytes());
    data[0x28..0x2C].copy_from_slice(&50u32.to_le_bytes());
    data[0x2C..0x30].copy_from_slice(&0x0000_23F0u32.to_le_bytes()); // Version 0.92 (9200)
    data[0x30..0x34].copy_from_slice(&0u32.to_le_bytes()); // EnableBpmGrid
    data[0x34..0x38].copy_from_slice(&1_200_000u32.to_le_bytes()); // BpmGridTempo
    data[0x38..0x3C].copy_from_slice(&1u32.to_le_bytes()); // BpmGridOffset
    data[0x3C..0x40].copy_from_slice(&0u32.to_le_bytes()); // EnableXYGrid
    data[0x40..0x44].copy_from_slice(&200u32.to_le_bytes()); // XYGridWidth
    data[0x44..0x48].copy_from_slice(&200u32.to_le_bytes()); // XYGridHeight
    data[0x48..0x4C].copy_from_slice(&0u32.to_le_bytes()); // EnableCameraGrid
    data[0x4C..0x50].copy_from_slice(&200u32.to_le_bytes()); // CameraGridWidth
    data[0x50..0x54].copy_from_slice(&10u32.to_le_bytes()); // CameraGridHeight
    data[0x54..0x58].copy_from_slice(&0u32.to_le_bytes()); // ShowOutsideFrame
    data[0x58..0x5C].copy_from_slice(&18u32.to_le_bytes()); // OutsideFrameScale
    data[0x5C..0x60].copy_from_slice(&4u32.to_le_bytes()); // BpmGridBeat
    data[0x60..0x64].copy_from_slice(&0u32.to_le_bytes()); // Field0x60
    data[0x64..0x68].copy_from_slice(&editing_scene.to_le_bytes());
    data[0x68..0x6C].copy_from_slice(&(scene_records.len() as u32).to_le_bytes());
    data[0x6C..0x70].copy_from_slice(&(layer_records.len() as u32).to_le_bytes());
    data[0x70..0x74].copy_from_slice(&220u32.to_le_bytes()); // Scene.Size
    data[0x74..0x78].copy_from_slice(&76u32.to_le_bytes()); // Layer.Size
    data[0x78..0x7C].copy_from_slice(&2u32.to_le_bytes()); // Field0x78
    data[0x7C..0x80].copy_from_slice(&(trackbar_scripts.len() as u32).to_le_bytes());

    for rec in &layer_records {
        data.extend_from_slice(rec);
    }
    for rec in &scene_records {
        data.extend_from_slice(rec);
    }
    for ts in &trackbar_scripts {
        data.extend_from_slice(&encode_sjis_fixed(ts, 128));
    }
    data.extend_from_slice(&effect_type_bytes);
    for blob in &obj_blobs {
        data.extend_from_slice(blob);
    }

    data
}

/// OpenAvi / `.aup2` のエフェクトを拡張編集 0.92 の `EffectType` に解決します。
fn map_effect_to_legacy_type(
    eff: &au2_core::Effect,
) -> Option<(u32, u32, usize, usize, usize, &'static str, Option<String>)> {
    let raw_name = eff.name.as_str();
    let (legacy_name, implicit_script_name): (&str, Option<String>) = match raw_name {
        "映像再生" => ("標準描画", None),
        "音声再生" | "基本出力" => ("標準再生", None),
        "ドロップシャドウ" => ("シャドー", None),
        "時間制御(オブジェクト)" => ("時間制御", None),
        "グループ制御(音声)" => ("グループ制御", None),
        "パーティクル出力(カメラ制御)" => ("パーティクル出力", None),
        other => {
            if BUILTIN_ANM_NAMES.contains(&other) {
                ("アニメーション効果", Some(other.to_string()))
            } else if BUILTIN_OBJ_NAMES.contains(&other) {
                ("カスタムオブジェクト", Some(other.to_string()))
            } else if BUILTIN_CAM_NAMES.contains(&other) {
                ("カメラ効果", Some(other.to_string()))
            } else if BUILTIN_SCN_TYPES.contains(&other) || BUILTIN_SCN_NAMES.contains(&other) {
                ("シーンチェンジ", Some(other.to_string()))
            } else {
                (other, None)
            }
        }
    };

    DEFAULT_EFFECT_TYPES
        .iter()
        .find(|&&(_, _, _, _, _, n)| n == legacy_name)
        .map(|&(id, flag, tb_num, cb_num, ext_size, name)| {
            (id, flag, tb_num, cb_num, ext_size, name, implicit_script_name)
        })
}

fn encode_timeline_object_to_80ee(
    obj: &au2_core::TimelineObject,
    scene_id: u32,
    layer_id: u32,
    editing_scene: u32,
    fps: f64,
    trackbar_scripts: &mut Vec<String>,
    obj_blobs: &mut Vec<Vec<u8>>,
    obj_counter: &mut u32,
) {
    // 1. エフェクト列を ExEdit 0.92 の最大 12 エフェクトに変換
    let mut resolved_effects: Vec<(
        u32,
        u32,
        usize,
        usize,
        usize,
        &'static str,
        Option<String>,
        std::borrow::Cow<'_, au2_core::Effect>,
    )> = Vec::new();

    for eff in &obj.effects {
        if let Some((id, flag, tb, cb, ext, name, script_name)) = map_effect_to_legacy_type(eff) {
            resolved_effects.push((
                id,
                flag,
                tb,
                cb,
                ext,
                name,
                script_name,
                std::borrow::Cow::Borrowed(eff),
            ));
            if resolved_effects.len() >= 12 {
                break;
            }
        }
    }

    if resolved_effects.is_empty() {
        return;
    }

    // メディアオブジェクトまたは音声オブジェクトに描画/再生フィルタが欠けている場合は自動補完
    let first_id = resolved_effects[0].0;
    let is_visual_media = matches!(first_id, 0 | 1 | 3 | 4 | 5 | 6 | 7 | 9 | 80);
    let is_audio_media = matches!(first_id, 2 | 8);
    let has_draw = resolved_effects.iter().any(|e| matches!(e.0, 10 | 11 | 13));
    let has_playback = resolved_effects.iter().any(|e| e.0 == 12);

    if is_visual_media && !has_draw && resolved_effects.len() < 12 {
        let mut draw_eff = au2_core::Effect::new("標準描画");
        draw_eff.push_track("X", au2_core::Track::constant(0.0, 1));
        draw_eff.push_track("Y", au2_core::Track::constant(0.0, 1));
        draw_eff.push_track("Z", au2_core::Track::constant(0.0, 1));
        draw_eff.push_track("拡大率", au2_core::Track::constant(100.0, 2));
        draw_eff.push_track("透明度", au2_core::Track::constant(0.0, 1));
        draw_eff.push_track("回転", au2_core::Track::constant(0.0, 2));
        if let Some((id, flag, tb, cb, ext, name, sn)) = map_effect_to_legacy_type(&draw_eff) {
            resolved_effects.push((
                id,
                flag,
                tb,
                cb,
                ext,
                name,
                sn,
                std::borrow::Cow::Owned(draw_eff),
            ));
        }
    } else if is_audio_media && !has_playback && resolved_effects.len() < 12 {
        let mut play_eff = au2_core::Effect::new("標準再生");
        play_eff.push_track("音量", au2_core::Track::constant(100.0, 1));
        play_eff.push_track("左右", au2_core::Track::constant(0.0, 1));
        if let Some((id, flag, tb, cb, ext, name, sn)) = map_effect_to_legacy_type(&play_eff) {
            resolved_effects.push((
                id,
                flag,
                tb,
                cb,
                ext,
                name,
                sn,
                std::borrow::Cow::Owned(play_eff),
            ));
        }
    }

    // トラックバー数上限(64)およびチェックボックス数上限(48)に収まるように調整
    let mut tb_sum = 0usize;
    let mut cb_sum = 0usize;
    let mut valid_len = 0usize;
    for (_, _, tb, cb, _, _, _, _) in &resolved_effects {
        if tb_sum + tb > 64 || cb_sum + cb > 48 {
            break;
        }
        tb_sum += tb;
        cb_sum += cb;
        valid_len += 1;
    }
    resolved_effects.truncate(valid_len);
    if resolved_effects.is_empty() {
        return;
    }

    // 2. オブジェクトフラグの構築
    let mut obj_flag: u32 = 0x0000_0001; // TimelineObjectFlag.Enable
    if obj.clipping {
        obj_flag |= 0x0000_0100;
    }
    if obj.camera_target {
        obj_flag |= 0x0000_0200;
    }
    match first_id {
        0 | 1 | 3 | 4 | 5 | 6 | 7 | 9 | 80 => {
            obj_flag |= 0x0001_0000; // Media
        }
        2 | 8 => {
            obj_flag |= 0x0001_0000 | 0x0002_0000; // Media | Audio
        }
        93 | 95 | 96 | 97 | 98 | 99 => {
            obj_flag |= 0x0008_0000; // Control
            let range = resolved_effects[0]
                .7
                .text("range")
                .or_else(|| resolved_effects[0].7.text("対象レイヤー数"))
                .and_then(|s| s.trim().parse::<i32>().ok())
                .unwrap_or(0);
            if range > 0 {
                obj_flag |= 0x0010_0000; // Range
            }
        }
        94 => {
            obj_flag |= 0x0004_0000 | 0x0008_0000; // MediaFilter | Control (グループ制御)
            let range = resolved_effects[0]
                .7
                .text("range")
                .or_else(|| resolved_effects[0].7.text("対象レイヤー数"))
                .and_then(|s| s.trim().parse::<i32>().ok())
                .unwrap_or(0);
            if range > 0 {
                obj_flag |= 0x0010_0000;
            }
        }
        _ => {
            obj_flag |= 0x0004_0000; // MediaFilter (フィルタオブジェクト)
        }
    }
    if obj.audio {
        obj_flag |= 0x0002_0000;
    }

    // プレビュー文字列
    let preview_name = resolved_effects[0].5;
    let preview_sjis = encode_sjis_fixed(preview_name, 64);

    // 3. 中間点セグメントへの分割
    let keys = obj.key_frames();
    let seg_count = keys.len().saturating_sub(1).max(1);
    let chain_group = if seg_count > 1 {
        *obj_counter
    } else {
        0xFFFF_FFFF
    };

    for seg_idx in 0..seg_count {
        let is_chain = seg_idx > 0;
        let rel_start = keys.get(seg_idx).copied().unwrap_or(0);
        let rel_end = keys
            .get(seg_idx + 1)
            .copied()
            .unwrap_or_else(|| obj.frame_end.saturating_sub(obj.frame_start));
        let abs_start = obj.frame_start + rel_start;
        let abs_end = if seg_idx + 1 < seg_count {
            (obj.frame_start + rel_end).saturating_sub(1).max(abs_start)
        } else {
            obj.frame_end.max(abs_start)
        };

        let total_ext_size: usize = if is_chain {
            0
        } else {
            resolved_effects.iter().map(|e| e.4).sum()
        };

        let mut blob = vec![0u8; 0x5C8 + total_ext_size];
        blob[0..4].copy_from_slice(&obj_flag.to_le_bytes());
        let field0x4 = if scene_id == editing_scene {
            layer_id
        } else {
            0xFFFF_FFFF
        };
        blob[4..8].copy_from_slice(&field0x4.to_le_bytes());
        blob[8..12].copy_from_slice(&abs_start.to_le_bytes());
        blob[12..16].copy_from_slice(&abs_end.to_le_bytes());
        blob[0x10..0x50].copy_from_slice(&preview_sjis);
        blob[0x50..0x54].copy_from_slice(&chain_group.to_le_bytes());
        blob[0xF4..0xF8].copy_from_slice(&(total_ext_size as u32).to_le_bytes());
        blob[0x4B8..0x4BC].copy_from_slice(&0u32.to_le_bytes());
        blob[0x4BC..0x4C0].copy_from_slice(&obj.group.unwrap_or(0).to_le_bytes());
        blob[0x5C0..0x5C4].copy_from_slice(&layer_id.to_le_bytes());
        blob[0x5C4..0x5C8].copy_from_slice(&scene_id.to_le_bytes());

        let mut ext_cursor = 0usize;
        let mut trackbar_count = 0usize;
        let mut checkbox_count = 0usize;

        for (ei, (eff_id, _eff_type_flag, tb_num, cb_num, ext_size, eff_name, script_name, eff)) in
            resolved_effects.iter().enumerate()
        {
            let hdr_off = 0x54 + ei * 12;
            blob[hdr_off..hdr_off + 4].copy_from_slice(&(*eff_id as i32).to_le_bytes());
            blob[hdr_off + 4..hdr_off + 6].copy_from_slice(&(trackbar_count as u16).to_le_bytes());
            blob[hdr_off + 6..hdr_off + 8].copy_from_slice(&(checkbox_count as u16).to_le_bytes());
            blob[hdr_off + 8..hdr_off + 12].copy_from_slice(&(ext_cursor as i32).to_le_bytes());

            let mut e_flag: u8 = if eff.enabled { 1 } else { 0 };
            if matches!(*eff_id, 10 | 11 | 13 | 88 | 94 | 95 | 96) {
                e_flag |= 4; // EffectFlag.Interactive
            }
            blob[0xE4 + ei] = e_flag;

            let scale_fn = |tb_i: usize| get_trackbar_scale(eff_name, tb_i);
            for tb_i in 0..*tb_num {
                let idx = trackbar_count + tb_i;
                let scale = scale_fn(tb_i);
                let (cur, next, trans, param) = encode_trackbar_for_segment(
                    eff_name,
                    tb_i,
                    scale,
                    eff,
                    &keys,
                    seg_idx,
                    fps,
                    trackbar_scripts,
                );
                blob[0x0F8 + idx * 4..0x0F8 + idx * 4 + 4].copy_from_slice(&cur.to_le_bytes());
                blob[0x1F8 + idx * 4..0x1F8 + idx * 4 + 4].copy_from_slice(&next.to_le_bytes());
                blob[0x2F8 + idx * 4..0x2F8 + idx * 4 + 4].copy_from_slice(&trans.to_le_bytes());
                blob[0x4C0 + idx * 4..0x4C0 + idx * 4 + 4].copy_from_slice(&param.to_le_bytes());
            }

            for cb_i in 0..*cb_num {
                let idx = checkbox_count + cb_i;
                let cb_val = encode_checkbox_value(eff_name, cb_i, eff);
                blob[0x3F8 + idx * 4..0x3F8 + idx * 4 + 4].copy_from_slice(&cb_val.to_le_bytes());
            }

            if !is_chain && *ext_size > 0 {
                let ext_bytes = build_ext_data(eff_name, *ext_size, script_name.as_deref(), eff);
                blob[0x5C8 + ext_cursor..0x5C8 + ext_cursor + *ext_size]
                    .copy_from_slice(&ext_bytes[..*ext_size]);
                ext_cursor += *ext_size;
            }

            trackbar_count += *tb_num;
            checkbox_count += *cb_num;
        }

        blob[0xF0..0xF2].copy_from_slice(&(trackbar_count as u16).to_le_bytes());
        blob[0xF2..0xF4].copy_from_slice(&(checkbox_count as u16).to_le_bytes());
        for ei in resolved_effects.len()..12 {
            let hdr_off = 0x54 + ei * 12;
            blob[hdr_off..hdr_off + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        }

        obj_blobs.push(blob);
        *obj_counter += 1;
    }
}

fn get_default_trackbar_value(eff_name: &str, tb_i: usize) -> f64 {
    match (eff_name, tb_i) {
        ("標準描画" | "拡張描画", 3) => 100.0, // 拡大率
        ("グループ制御", 3) => 100.0,           // 拡大率
        ("動画ファイル" | "シーン", 0) => 1.0,  // 再生位置 (1フレーム目)
        ("動画ファイル" | "音声ファイル" | "シーン" | "シーン(音声)", 1) => 100.0, // 再生速度
        ("標準再生", 0) => 100.0,               // 音量
        ("図形", 0) => 100.0,                   // サイズ
        ("図形", 2) => 4000.0,                  // ライン幅
        ("テキスト", 0) => 34.0,                // サイズ
        ("拡大率" | "リサイズ", _) => 100.0,
        ("カメラ制御", 2) => -1024.0,           // Z
        ("カメラ制御", 9) => 30.0,              // 視野角
        ("グロー", 0) => 50.0,
        ("グロー", 1) => 60.0,
        ("グロー", 2) => 50.0,
        ("グロー", 3) => 20.0,
        ("発光", 0) => 100.0,
        ("発光", 1) => 75.0,
        ("発光", 2) => 75.0,
        ("ぼかし", 0) => 5.0,
        ("モザイク", 0) => 10.0,
        ("シャドー", 0) => 5.0,
        ("シャドー", 1) => 5.0,
        ("シャドー", 2) => 40.0,
        ("シャドー", 3) => 10.0,
        ("縁取り", 0) => 3.0,
        ("縁取り", 1) => 10.0,
        ("色調補正", 1 | 3 | 4) => 100.0,       // コントラスト・輝度・彩度
        ("グラデーション", 0) => 100.0,
        ("グラデーション", 4) => 100.0,
        ("単色化", 0) => 100.0,
        ("マスク", 3) => 100.0,
        _ => 0.0,
    }
}

fn find_track_param<'a>(
    eff_name: &str,
    tb_i: usize,
    eff: &'a au2_core::Effect,
) -> Option<&'a au2_core::ParamValue> {
    let primary_name = get_trackbar_name(eff_name, tb_i);
    let fallback_track = format!("track{}", tb_i);
    for p in &eff.params {
        if p.key == primary_name || p.key == fallback_track {
            return Some(&p.value);
        }
        if (eff_name == "標準描画" && tb_i == 5 || eff_name == "拡張描画" && tb_i == 8)
            && matches!(p.key.as_str(), "回転" | "Z軸回転")
        {
            return Some(&p.value);
        }
        if eff_name == "色調補正" && tb_i == 1 && matches!(p.key.as_str(), "ｺﾝﾄﾗｽﾄ" | "コントラスト") {
            return Some(&p.value);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn encode_trackbar_for_segment(
    eff_name: &str,
    tb_i: usize,
    scale: i32,
    eff: &au2_core::Effect,
    keys: &[au2_core::Frame],
    seg_idx: usize,
    fps: f64,
    trackbar_scripts: &mut Vec<String>,
) -> (i32, i32, u32, i32) {
    let scale_f = (scale.max(1)) as f64;
    let Some(pv) = find_track_param(eff_name, tb_i, eff) else {
        let def = (get_default_trackbar_value(eff_name, tb_i) * scale_f).round() as i32;
        return (def, def, 0, 0);
    };

    match pv {
        au2_core::ParamValue::Text(s) => {
            let v = s
                .split(',')
                .next()
                .and_then(|t| t.trim().parse::<f64>().ok())
                .unwrap_or_else(|| get_default_trackbar_value(eff_name, tb_i));
            let scaled = (v * scale_f).round() as i32;
            (scaled, scaled, 0, 0)
        }
        au2_core::ParamValue::Track(track) => {
            // AviUtl2 の「再生範囲」指定 (秒単位) が入っている場合の変換
            if tb_i == 0
                && matches!(eff_name, "動画ファイル" | "音声ファイル")
                && track
                    .interpolation
                    .as_ref()
                    .is_some_and(|it| matches!(&it.kind, au2_core::MoveKind::Script(s) if s == "再生範囲"))
            {
                let start_sec = track.start_value().max(0.0);
                let val = if eff_name == "動画ファイル" {
                    (start_sec * fps).round() as i32 + 1
                } else {
                    (start_sec * scale_f).round() as i32
                };
                return (val, val, 0, 0);
            }

            let Some(interp) = &track.interpolation else {
                let v = (track.start_value() * scale_f).round() as i32;
                return (v, v, 0, 0);
            };
            if track.keyframes.len() < 2 {
                let v = (track.start_value() * scale_f).round() as i32;
                return (v, v, 0, 0);
            }

            let val_at_idx = |k_idx: usize| -> f64 {
                if let Some(&f) = keys.get(k_idx) {
                    track
                        .keyframes
                        .iter()
                        .find(|k| k.frame == f)
                        .map(|k| k.value)
                        .unwrap_or_else(|| track.evaluate(f))
                } else {
                    track
                        .keyframes
                        .get(k_idx)
                        .or_else(|| track.keyframes.last())
                        .map(|k| k.value)
                        .unwrap_or(0.0)
                }
            };

            let cur = (val_at_idx(seg_idx) * scale_f).round() as i32;
            let next = (val_at_idx(seg_idx + 1) * scale_f).round() as i32;

            let mut easing_flags = 0u32;
            if interp.easing.accel() {
                easing_flags |= 0x40;
            }
            if interp.easing.decel() {
                easing_flags |= 0x20;
            }

            let mut get_or_insert_script = |name: &str| -> u32 {
                if let Some(pos) = trackbar_scripts.iter().position(|s| s == name) {
                    pos as u32
                } else {
                    let idx = trackbar_scripts.len() as u32;
                    trackbar_scripts.push(name.to_string());
                    idx
                }
            };

            let trans_code: u32 = match &interp.kind {
                au2_core::MoveKind::Linear => 1 | easing_flags,
                au2_core::MoveKind::Curve => {
                    if interp.legacy_code == Some(2) {
                        2 | easing_flags
                    } else {
                        let sc_idx = get_or_insert_script("補間移動");
                        0x0F | easing_flags | (sc_idx << 16)
                    }
                }
                au2_core::MoveKind::Instant => 3 | easing_flags,
                au2_core::MoveKind::IgnoreMidpoint => 4 | easing_flags,
                au2_core::MoveKind::MoveAmount => 5 | easing_flags,
                au2_core::MoveKind::Random => 6 | easing_flags,
                au2_core::MoveKind::Other(c) => (c & 0x0F) | easing_flags,
                au2_core::MoveKind::Script(s) => {
                    let sc_idx = get_or_insert_script(s);
                    0x0F | easing_flags | (sc_idx << 16)
                }
            };

            let param = interp
                .param
                .as_deref()
                .and_then(|p| p.trim().parse::<i32>().ok())
                .unwrap_or(0);

            (cur, next, trans_code, param)
        }
    }
}

fn encode_checkbox_value(eff_name: &str, cb_i: usize, eff: &au2_core::Effect) -> i32 {
    let cb_name = get_checkbox_name(eff_name, cb_i);
    let fallback_key = format!("check{}", cb_i);
    for p in &eff.params {
        if cb_name.as_deref() == Some(p.key.as_str()) || p.key == fallback_key {
            return match &p.value {
                au2_core::ParamValue::Text(s) => s.trim().parse::<i32>().unwrap_or(0),
                au2_core::ParamValue::Track(t) => t.start_value().round() as i32,
            };
        }
    }
    0
}

fn parse_rgb_hex(s: &str, default_rgb: [u8; 3]) -> [u8; 3] {
    let trimmed = s.trim().trim_start_matches('#');
    if trimmed.len() >= 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&trimmed[0..2], 16),
            u8::from_str_radix(&trimmed[2..4], 16),
            u8::from_str_radix(&trimmed[4..6], 16),
        ) {
            return [r, g, b];
        }
    }
    default_rgb
}

fn get_param_i32(eff: &au2_core::Effect, key: &str, default_val: i32) -> i32 {
    eff.params
        .iter()
        .find(|p| p.key == key)
        .map(|p| match &p.value {
            au2_core::ParamValue::Text(s) => s.trim().parse::<i32>().unwrap_or(default_val),
            au2_core::ParamValue::Track(t) => t.start_value().round() as i32,
        })
        .unwrap_or(default_val)
}

fn build_ext_data(
    eff_name: &str,
    ext_size: usize,
    implicit_script_name: Option<&str>,
    eff: &au2_core::Effect,
) -> Vec<u8> {
    let mut ext = vec![0u8; ext_size];
    if ext_size == 0 {
        return ext;
    }

    match eff_name {
        "動画ファイル" | "音声ファイル" => {
            if let Some(path) = eff.text("file").or_else(|| eff.text("ファイル")) {
                let sjis = encode_sjis_fixed(path, 260.min(ext_size));
                ext[..sjis.len()].copy_from_slice(&sjis);
            }
        }
        "画像ファイル" => {
            if let Some(path) = eff.text("file").or_else(|| eff.text("ファイル")) {
                if ext_size >= 264 {
                    let sjis = encode_sjis_fixed(path, 260);
                    ext[4..264].copy_from_slice(&sjis);
                }
            }
        }
        "図形" => {
            if ext_size >= 8 {
                let raw_type = eff
                    .text("type")
                    .or_else(|| eff.text("図形の種類"))
                    .unwrap_or("2");
                let fig_type: i32 = match raw_type.trim() {
                    "背景" | "0" => 0,
                    "円" | "1" => 1,
                    "四角形" | "2" => 2,
                    "三角形" | "3" => 3,
                    "五角形" | "4" => 4,
                    "六角形" | "5" => 5,
                    "星型" | "6" => 6,
                    other => other.parse().unwrap_or(2),
                };
                ext[0..4].copy_from_slice(&fig_type.to_le_bytes());
                let rgb = parse_rgb_hex(
                    eff.text("color").or_else(|| eff.text("色")).unwrap_or("ffffff"),
                    [0xFF, 0xFF, 0xFF],
                );
                ext[4..7].copy_from_slice(&rgb);
                if ext_size >= 72 {
                    if let Some(name) = eff.text("name") {
                        let sjis = encode_sjis_fixed(name, 64);
                        ext[8..72].copy_from_slice(&sjis);
                    }
                }
            }
        }
        "テキスト" => {
            if ext_size >= 0x30 + 2048 {
                let raw_type = eff
                    .text("type")
                    .or_else(|| eff.text("文字装飾"))
                    .unwrap_or("0");
                let t_type: u8 = match raw_type.trim() {
                    "標準文字" | "0" => 0,
                    "影付き文字" | "1" => 1,
                    "影付き文字(薄)" | "2" => 2,
                    "縁取り文字" | "3" => 3,
                    "縁取り文字(細)" | "4" => 4,
                    "縁取り文字(太)" | "5" => 3,
                    "縁取り文字(角)" | "6" => 4,
                    other => other.parse().unwrap_or(0),
                };
                let autoadjust = get_param_i32(eff, "autoadjust", 0) as u8;
                let soft = get_param_i32(eff, "soft", 1) as u8;
                let monospace = get_param_i32(eff, "monospace", 0) as u8;
                let raw_align = eff
                    .text("align")
                    .or_else(|| eff.text("文字揃え"))
                    .unwrap_or("0");
                let align: u8 = match raw_align.trim() {
                    "左寄せ[上]" | "0" => 0,
                    "中央揃え[上]" | "1" => 1,
                    "右寄せ[上]" | "2" => 2,
                    "左寄せ[中]" | "3" => 3,
                    "中央揃え[中]" | "4" => 4,
                    "右寄せ[中]" | "5" => 5,
                    "左寄せ[下]" | "6" => 6,
                    "中央揃え[下]" | "7" => 7,
                    "右寄せ[下]" | "8" => 8,
                    other => other.parse().unwrap_or(0),
                };
                let spacing_x = get_param_i32(eff, "spacing_x", get_param_i32(eff, "字間", 0)) as i8 as u8;
                let spacing_y = get_param_i32(eff, "spacing_y", get_param_i32(eff, "行間", 0)) as i8 as u8;
                let precision = get_param_i32(eff, "precision", 1) as u8;

                ext[0] = t_type;
                ext[1] = autoadjust;
                ext[2] = soft;
                ext[3] = monospace;
                ext[4] = align;
                ext[5] = spacing_x;
                ext[6] = spacing_y;
                ext[7] = precision;

                let rgb1 = parse_rgb_hex(
                    eff.text("color").or_else(|| eff.text("文字色")).unwrap_or("ffffff"),
                    [0xFF, 0xFF, 0xFF],
                );
                let rgb2 = parse_rgb_hex(
                    eff.text("color2").or_else(|| eff.text("影・縁色")).unwrap_or("000000"),
                    [0x00, 0x00, 0x00],
                );
                ext[8..11].copy_from_slice(&rgb1);
                ext[12..15].copy_from_slice(&rgb2);

                let font = eff
                    .text("font")
                    .or_else(|| eff.text("フォント"))
                    .unwrap_or("MS UI Gothic");
                let font_sjis = encode_sjis_fixed(font, 32);
                ext[0x10..0x30].copy_from_slice(&font_sjis);

                let raw_text = eff
                    .text("text")
                    .or_else(|| eff.text("テキスト"))
                    .unwrap_or("");
                let decoded_text = if raw_text.len() >= 16
                    && raw_text.len() % 4 == 0
                    && raw_text.chars().all(|c| c.is_ascii_hexdigit())
                {
                    au2_exo::decode_text_hex(raw_text).unwrap_or_else(|| raw_text.to_string())
                } else {
                    raw_text.to_string()
                };
                let mut utf16_pos = 0x30usize;
                for code_unit in decoded_text.encode_utf16() {
                    if utf16_pos + 4 > 0x30 + 2048 {
                        break;
                    }
                    ext[utf16_pos..utf16_pos + 2].copy_from_slice(&code_unit.to_le_bytes());
                    utf16_pos += 2;
                }
            }
        }
        "標準描画" | "拡張描画" | "パーティクル出力" => {
            if ext_size >= 4 {
                let raw_blend = eff
                    .text("blend")
                    .or_else(|| eff.text("合成モード"))
                    .unwrap_or("0");
                let blend: i32 = crate::blend_aup2_to_code(raw_blend).parse().unwrap_or(0);
                ext[0..4].copy_from_slice(&blend.to_le_bytes());
            }
        }
        "シーン" | "シーン(音声)" => {
            if ext_size >= 4 {
                let sc_idx = get_param_i32(eff, "scene", 1);
                ext[0..4].copy_from_slice(&sc_idx.to_le_bytes());
            }
        }
        "時間制御" | "グループ制御" | "カメラ制御" => {
            if ext_size >= 4 {
                let range = get_param_i32(
                    eff,
                    "range",
                    get_param_i32(eff, "対象レイヤー数", 0),
                );
                ext[0..4].copy_from_slice(&range.to_le_bytes());
            }
        }
        "グロー" | "発光" | "拡散光" | "グロー(フィルタ)" | "発光(フィルタ)" | "拡散光(フィルタ)" => {
            if ext_size >= 4 {
                let color_opt = eff.text("color").or_else(|| eff.text("光色"));
                let no_color = get_param_i32(
                    eff,
                    "no_color",
                    if color_opt.is_some_and(|c| c.is_empty()) { 1 } else { 0 },
                ) as u8;
                let rgb = parse_rgb_hex(color_opt.unwrap_or("ffffff"), [0xFF, 0xFF, 0xFF]);
                ext[0..3].copy_from_slice(&rgb);
                ext[3] = no_color;
            }
            if ext_size >= 8 {
                let t = get_param_i32(eff, "type", 0);
                ext[4..8].copy_from_slice(&t.to_le_bytes());
            }
            if ext_size >= 12 {
                let blend = get_param_i32(eff, "blend", 0);
                ext[8..12].copy_from_slice(&blend.to_le_bytes());
            }
        }
        "閃光" | "閃光(フィルタ)" | "ライト" | "ライト(フィルタ)" => {
            if ext_size >= 4 {
                let rgb = parse_rgb_hex(
                    eff.text("color").or_else(|| eff.text("光色")).unwrap_or("ffffff"),
                    [0xFF, 0xFF, 0xFF],
                );
                ext[0..3].copy_from_slice(&rgb);
            }
            if ext_size >= 8 {
                let blend = get_param_i32(eff, "blend", 0);
                ext[4..8].copy_from_slice(&blend.to_le_bytes());
            }
        }
        "シャドー" | "縁取り" | "単色化" | "単色化(フィルタ)" => {
            if ext_size >= 4 {
                let rgb = parse_rgb_hex(
                    eff.text("color")
                        .or_else(|| eff.text("影色"))
                        .or_else(|| eff.text("縁色"))
                        .unwrap_or("000000"),
                    [0x00, 0x00, 0x00],
                );
                ext[0..3].copy_from_slice(&rgb);
            }
            if ext_size >= 264 {
                if let Some(path) = eff.text("file").or_else(|| eff.text("ファイル")) {
                    let sjis = encode_sjis_fixed(path, 260);
                    ext[4..264].copy_from_slice(&sjis);
                }
            }
        }
        "グラデーション" | "グラデーション(フィルタ)" => {
            if ext_size >= 16 {
                let blend = get_param_i32(eff, "blend", 0);
                ext[0..4].copy_from_slice(&blend.to_le_bytes());
                let rgb1 = parse_rgb_hex(eff.text("color").unwrap_or("ffffff"), [0xFF, 0xFF, 0xFF]);
                ext[4..7].copy_from_slice(&rgb1);
                ext[7] = get_param_i32(eff, "no_color", 0) as u8;
                let rgb2 = parse_rgb_hex(eff.text("color2").unwrap_or("000000"), [0x00, 0x00, 0x00]);
                ext[8..11].copy_from_slice(&rgb2);
                ext[11] = get_param_i32(eff, "no_color2", 0) as u8;
                let g_type = get_param_i32(eff, "type", 0);
                ext[12..16].copy_from_slice(&g_type.to_le_bytes());
            }
        }
        "ワイプ" | "マスク" | "ワイプ(フィルタ)" | "ディスプレイスメントマップ" => {
            if ext_size >= 4 {
                let def_t = if eff_name == "ワイプ" || eff_name == "ワイプ(フィルタ)" {
                    0
                } else {
                    1
                };
                let t = get_param_i32(eff, "type", def_t) as i16;
                ext[0..2].copy_from_slice(&t.to_le_bytes());
                if ext_size >= 260 {
                    if let Some(name) = eff.text("name") {
                        let sjis = encode_sjis_fixed(name, 256);
                        ext[4..260].copy_from_slice(&sjis);
                    }
                }
                if ext_size >= 264 {
                    let mode = get_param_i32(eff, "mode", 0);
                    ext[260..264].copy_from_slice(&mode.to_le_bytes());
                }
            }
        }
        "動画ファイル合成" | "画像ファイル合成" => {
            if ext_size >= 4 {
                let mode = get_param_i32(eff, "mode", 0);
                ext[0..4].copy_from_slice(&mode.to_le_bytes());
            }
            if ext_size >= 264 {
                if let Some(path) = eff.text("file").or_else(|| eff.text("ファイル")) {
                    let sjis = encode_sjis_fixed(path, 260);
                    ext[4..264].copy_from_slice(&sjis);
                }
            }
        }
        "音声波形表示" => {
            if ext_size >= 14 {
                let t = get_param_i32(eff, "type", 0) as i16;
                ext[0..2].copy_from_slice(&t.to_le_bytes());
                let rgb = parse_rgb_hex(eff.text("color").unwrap_or("ffffff"), [0xFF, 0xFF, 0xFF]);
                ext[4..7].copy_from_slice(&rgb);
                let sample_n = get_param_i32(eff, "sample_n", 1024) as i16;
                let res_w = get_param_i32(eff, "res_w", 100) as i16;
                let res_h = get_param_i32(eff, "res_h", 100) as i16;
                ext[8..10].copy_from_slice(&sample_n.to_le_bytes());
                ext[10..12].copy_from_slice(&res_w.to_le_bytes());
                ext[12..14].copy_from_slice(&res_h.to_le_bytes());
            }
        }
        "アニメーション効果"
        | "カスタムオブジェクト"
        | "カメラ効果"
        | "スクリプト(カメラ制御)"
        | "シーンチェンジ" => {
            if ext_size >= 516 {
                let raw_name = implicit_script_name
                    .or_else(|| eff.text("name"))
                    .unwrap_or("");
                let explicit_type = eff
                    .text("type")
                    .and_then(|s| s.trim().parse::<i16>().ok());
                let dir = get_param_i32(eff, "filter", 0) as i16;

                let builtin_table: &[&str] = match eff_name {
                    "アニメーション効果" => BUILTIN_ANM_NAMES,
                    "カスタムオブジェクト" => BUILTIN_OBJ_NAMES,
                    "カメラ効果" | "スクリプト(カメラ制御)" => BUILTIN_CAM_NAMES,
                    "シーンチェンジ" => BUILTIN_SCN_TYPES,
                    _ => &[],
                };

                let (script_id, stored_name) = if let Some(pos) =
                    builtin_table.iter().position(|&b| b == raw_name)
                {
                    (pos as i16, "")
                } else if eff_name == "シーンチェンジ" {
                    if let Some(pos) = BUILTIN_SCN_NAMES.iter().position(|&b| b == raw_name) {
                        (pos as i16, "")
                    } else {
                        (explicit_type.unwrap_or(0), raw_name)
                    }
                } else if let Some(t) = explicit_type {
                    if raw_name.is_empty() || builtin_table.get(t as usize).copied() == Some(raw_name) {
                        (t, "")
                    } else {
                        (t, raw_name)
                    }
                } else {
                    (0, raw_name)
                };

                ext[0..2].copy_from_slice(&script_id.to_le_bytes());
                ext[2..4].copy_from_slice(&dir.to_le_bytes());
                if !stored_name.is_empty() {
                    let sjis = encode_sjis_fixed(stored_name, 256);
                    ext[4..260].copy_from_slice(&sjis);
                }
                if let Some(param_str) = eff.text("param") {
                    let sjis = encode_sjis_fixed(param_str, 256);
                    ext[260..516].copy_from_slice(&sjis);
                }
            }
        }
        _ => {
            if ext_size >= 4 {
                if let Some(c) = eff.text("color") {
                    let rgb = parse_rgb_hex(c, [0xFF, 0xFF, 0xFF]);
                    ext[0..3].copy_from_slice(&rgb);
                } else {
                    let t = get_param_i32(eff, "type", 0);
                    ext[0..4].copy_from_slice(&t.to_le_bytes());
                }
            }
        }
    }

    ext
}

