//! オブジェクト単位の変換・スクリプト評価・フィルタオブジェクト描画。

use std::f32::consts::{PI, TAU};

use au2_core::{EffectKind, Frame, ObjectCategory, TimelineObject};
use au2_plugin::LuaObjState;
use eframe::egui::{pos2, vec2, Color32, Pos2, Rect, Stroke};

use crate::panels::parse_color;
use super::super::camera::{aviutl_interpolation, aviutl_rand, rotate_3d};
use super::super::filters::FilterVisuals;
use super::super::text::{layout_aviutl_text, u32_to_color32, TextCharLayout};
use super::instance::draw_single_instance;
use super::primitives::fan;
use super::DrawCtx;

pub(super) fn draw_object(ctx: &DrawCtx, layer_idx: usize, obj: &TimelineObject, rel: Frame, selected: bool) -> Option<Rect> {
    let Some(src) = obj.source() else { return None };
    if !src.enabled {
        return None;
    }
    match obj.category() {
        ObjectCategory::Media => {}
        ObjectCategory::Filter => {
            draw_filter_object(ctx, obj, rel);
            return Some(ctx.canvas);
        }
        _ => return None,
    }

    // テキストの「文字毎に個別オブジェクト (`check0` = 1)」が有効な場合、
    // 各文字を個別のサブオブジェクト (`obj.index = 0..N-1`, `obj.num = N`) としてエフェクト評価・描画する
    if matches!(src.kind, EffectKind::Text) {
        let is_split = src.eval("文字毎に個別オブジェクト", rel, src.eval("check0", rel, 0.0)) >= 0.5;
        if is_split {
            let (chars_layout, _) = layout_aviutl_text(ctx, src, rel);
            if chars_layout.is_empty() {
                return None;
            }
            let display_on_moving = src.eval("移動座標上に表示する", rel, src.eval("check1", rel, 0.0)) >= 0.5;
            let raw_size = src.eval("サイズ", rel, 34.0).max(1.0);
            let total_num = chars_layout.len() as i64;
            let mut combined: Option<Rect> = None;
            for (idx, item) in chars_layout.iter().enumerate() {
                if item.ch == " " || item.ch == "　" {
                    continue;
                }
                let init_ox = if display_on_moving { 0.0 } else { item.local_x as f64 };
                let init_oy = if display_on_moving { 0.0 } else { item.local_y as f64 };
                let single_char = TextCharLayout {
                    ch: item.ch.clone(),
                    local_x: 0.0,
                    local_y: 0.0,
                    color_override: item.color_override,
                    color2_override: item.color2_override,
                    size_scale: item.size_scale,
                    font_override: item.font_override.clone(),
                };
                if let Some(b) = draw_object_sub(
                    ctx,
                    layer_idx,
                    obj,
                    src,
                    rel,
                    idx as i64,
                    total_num,
                    init_ox,
                    init_oy,
                    raw_size,
                    Some(&single_char),
                    false,
                ) {
                    combined = Some(combined.map(|prev| prev.union(b)).unwrap_or(b));
                }
            }
            if selected {
                if let Some(b) = combined {
                    ctx.painter.rect_stroke(b.expand(2.0), 0.0, Stroke::new(1.0_f32, Color32::from_rgb(255, 200, 0)));
                }
            }
            return combined;
        }
    }

    let approx_size = src.eval("サイズ", rel, 100.0).max(16.0);
    draw_object_sub(
        ctx,
        layer_idx,
        obj,
        src,
        rel,
        0,
        1,
        0.0,
        0.0,
        approx_size,
        None,
        selected,
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_object_sub(
    ctx: &DrawCtx,
    layer_idx: usize,
    obj: &TimelineObject,
    src: &au2_core::Effect,
    rel: Frame,
    sub_index: i64,
    sub_num: i64,
    init_ox: f64,
    init_oy: f64,
    approx_size: f64,
    single_text_char: Option<&TextCharLayout>,
    selected: bool,
) -> Option<Rect> {
    // 標準描画 / 拡張描画パラメータ
    let draw = obj
        .effect(&EffectKind::StandardDraw)
        .or_else(|| obj.effect(&EffectKind::ExtendedDraw));
    let ev = |key: &str, default: f64| draw.map(|d| d.eval(key, rel, default)).unwrap_or(default);
    let base_x = ev("X", 0.0);
    let base_y = ev("Y", 0.0);
    let base_z = ev("Z", 0.0);
    let base_zoom = ev("拡大率", 100.0) / 100.0;
    let base_rot = ev("回転", 0.0);
    let base_alpha = (1.0 - ev("透明度", 0.0) / 100.0).clamp(0.0, 1.0);
    let base_rx = ev("X軸回転", 0.0);
    let base_ry = ev("Y軸回転", 0.0);
    let base_rz = if base_rot != 0.0 { base_rot } else { ev("Z軸回転", 0.0) };
    let base_cx = ev("中心X", 0.0);
    let base_cy = ev("中心Y", 0.0);
    let base_cz = ev("中心Z", 0.0);

    let time_sec = rel as f64 / ctx.fps.max(1.0);
    let total_sec = obj.length() as f64 / ctx.fps.max(1.0);

    // Lua スクリプト実行ステートの初期化
    let mut lua_st = LuaObjState {
        x: base_x,
        y: base_y,
        z: base_z,
        ox: init_ox,
        oy: init_oy,
        rx: base_rx,
        ry: base_ry,
        rz: base_rz,
        cx: base_cx,
        cy: base_cy,
        cz: base_cz,
        zoom: 1.0,
        alpha: base_alpha,
        w: approx_size,
        h: approx_size,
        time: time_sec,
        totaltime: total_sec,
        frame: rel as i64,
        totalframe: obj.length() as i64,
        layer: (layer_idx + 1) as i64,
        index: sub_index,
        num: sub_num,
        screen_w: ctx.screen_w,
        screen_h: ctx.screen_h,
        framerate: ctx.fps,
        ..Default::default()
    };

    let mut fx = FilterVisuals::default();
    if let Some(b) = draw.and_then(|d| d.text("blend")).and_then(|b| b.parse::<i32>().ok()) {
        fx.blend_mode = b;
    }

    // フィルタ効果・スクリプト (.anm / .obj / .cam / .scn) および内蔵エフェクトの評価
    for eff in &obj.effects {
        if !eff.enabled {
            continue;
        }
        if matches!(
            eff.kind,
            EffectKind::StandardDraw | EffectKind::ExtendedDraw | EffectKind::StandardPlayback
        ) {
            continue;
        }

        // 1) プラグインレジストリ内の Lua スクリプト (.anm/.obj/.cam/.scn) を実行
        let is_builtin_filter = matches!(
            eff.name.as_str(),
            "フェード"
                | "振動"
                | "縁取り"
                | "シャドー"
                | "ドロップシャドウ"
                | "単色化"
                | "グロー"
                | "発光"
                | "拡散光"
                | "閃光"
                | "色ズレ"
                | "ワイプ"
                | "マスク"
                | "クリッピング"
                | "斜めクリッピング"
                | "色調補正"
                | "グラデーション"
                | "凸エッジ"
                | "ミラー"
                | "リサイズ"
                | "ローテーション"
                | "反転"
                | "極座標変換"
                | "ディスプレイスメントマップ"
                | "ノイズ"
                | "モザイク"
                | "ぼかし"
                | "方向ブラー"
                | "レンズブラー"
                | "放射ブラー"
                | "座標"
                | "拡大率"
                | "透明度"
                | "回転"
                | "エッジ抽出"
                | "ルミナンスキー"
        ) && eff.text("name").is_none();

        let ran_lua = if is_builtin_filter {
            false
        } else {
            ctx.plugins.execute_effect(eff, rel, &mut lua_st)
        };

        // 2) 内蔵フィルタの視覚効果評価
        match eff.name.as_str() {
            "フェード" => {
                let fin = eff.eval("イン", rel, 0.0) * ctx.fps;
                let fout = eff.eval("アウト", rel, 0.0) * ctx.fps;
                let remain = (obj.length() - 1 - rel) as f64;
                if fin > 0.0 {
                    lua_st.alpha *= (rel as f64 / fin).clamp(0.0, 1.0);
                }
                if fout > 0.0 {
                    lua_st.alpha *= (remain / fout).clamp(0.0, 1.0);
                }
            }
            "振動" => {
                let ax = eff.eval("X", rel, 15.0);
                let ay = eff.eval("Y", rel, 15.0);
                let az = eff.eval("Z", rel, 0.0);
                let period = eff.eval("周期", rel, 1.0).max(0.1);
                let phase = (rel as f64 / period) * (TAU as f64);
                lua_st.ox += phase.sin() * ax;
                lua_st.oy += (phase * 1.3).cos() * ay;
                lua_st.oz += (phase * 0.7).sin() * az;
            }
            "リサイズ" => {
                let z = eff.eval("拡大率", rel, 100.0) / 100.0;
                lua_st.zoom *= z.max(0.0);
            }
            "ローテーション" => {
                let deg = eff.eval("90度回転", rel, 0.0);
                lua_st.rz += deg * 90.0;
            }
            "グロー" | "発光" | "拡散光" | "閃光" => {
                let strength = eff.eval("強さ", rel, 60.0) as f32;
                let diffusion = eff.eval("拡散", rel, 30.0) as f32;
                let blur = eff.eval("ぼかし", rel, 1.0) as f32;
                fx.glow_strength = (fx.glow_strength + strength / 85.0).min(8.0);
                fx.glow_radius = (fx.glow_radius + diffusion + blur * 0.5).max(8.0);
                if let Some(c) = eff.text("color").and_then(parse_color) {
                    fx.glow_color = Some(c);
                }
                if eff.eval("光成分のみ", rel, eff.eval("check0", rel, 0.0)) >= 0.5 {
                    fx.glow_only = true;
                }
            }
            "ワイプ" => {
                // AviUtl のワイプ: イン(秒) で 0→100% 出現、アウト(秒) で中央からくり抜かれて消滅（または枠線化）
                let win = eff.eval("イン", rel, 0.0) * ctx.fps;
                let wout = eff.eval("アウト", rel, 0.0) * ctx.fps;
                let remain = (obj.length() - 1 - rel) as f64;
                let rev_in = eff.eval("反転(イン)", rel, eff.eval("check0", rel, 0.0)) >= 0.5;
                let rev_out = eff.eval("反転(アウト)", rel, eff.eval("check1", rel, 0.0)) >= 0.5;

                // イン進行度 (0.0 -> 1.0)
                if win > 0.0 && (rel as f64) < win {
                    let p = ((rel as f64 + 0.5) / win).clamp(0.0, 1.0) as f32;
                    if rev_in {
                        fx.inner_hollow_ratio = fx.inner_hollow_ratio.max(1.0 - p);
                    } else {
                        lua_st.zoom *= p as f64;
                    }
                }
                // アウト進行度: 残り時間が wout 以下のとき、中央からくり抜いて枠線（フレーム）のように消えていく
                if wout > 0.0 && remain < wout {
                    let p = (1.0 - (remain / wout).clamp(0.0, 1.0)) as f32;
                    if rev_out {
                        lua_st.zoom *= (1.0 - p) as f64;
                    } else {
                        fx.inner_hollow_ratio = fx.inner_hollow_ratio.max(p.clamp(0.0, 0.98));
                    }
                }
            }
            "マスク" => {
                let m_size = eff.eval("サイズ", rel, 100.0) as f32;
                let invert = eff.eval("マスクの反転", rel, eff.eval("check0", rel, 0.0)) >= 0.5;
                let fit = eff.eval("元のサイズに合わせる", rel, eff.eval("check1", rel, 0.0)) >= 0.5;
                let obj_sz = approx_size as f32;
                if invert {
                    // マスクの反転: 図形の中央をくり抜いて中抜きフレーム（枠線）にする
                    let ratio = if fit {
                        (m_size / 100.0).clamp(0.0, 0.98)
                    } else if obj_sz > 1.0 {
                        (m_size / obj_sz).clamp(0.0, 0.98)
                    } else {
                        0.75
                    };
                    fx.inner_hollow_ratio = fx.inner_hollow_ratio.max(ratio);
                } else if !fit && obj_sz > 1.0 && m_size < obj_sz {
                    let clip_half = ((obj_sz - m_size) * 0.5).max(0.0);
                    for inset in &mut fx.clip_insets {
                        *inset = (*inset).max(clip_half);
                    }
                }
            }
            "クリッピング" => {
                let top = eff.eval("上", rel, 0.0) as f32;
                let bottom = eff.eval("下", rel, 0.0) as f32;
                let left = eff.eval("左", rel, 0.0) as f32;
                let right = eff.eval("右", rel, 0.0) as f32;
                fx.clip_insets[0] += top.max(0.0);
                fx.clip_insets[1] += bottom.max(0.0);
                fx.clip_insets[2] += left.max(0.0);
                fx.clip_insets[3] += right.max(0.0);
                if eff.eval("中心の位置を変更", rel, eff.eval("check0", rel, 0.0)) >= 0.5 {
                    lua_st.ox += ((left - right) * 0.5) as f64;
                    lua_st.oy += ((top - bottom) * 0.5) as f64;
                }
            }
            "斜めクリッピング" => {
                let width = eff.eval("幅", rel, 0.0) as f32;
                if width < 0.0 {
                    // 負の幅は帯状の切り抜き
                    let band = (-width).max(2.0);
                    let cut = ((approx_size as f32 - band) * 0.5).max(0.0);
                    fx.clip_insets[0] += cut;
                    fx.clip_insets[1] += cut;
                }
            }
            "色調補正" | "拡張色調補正" => {
                let br = (eff.eval("明るさ", rel, 100.0) as f32 - 100.0) / 100.0;
                let hue = eff.eval("色相", rel, 0.0) as f32;
                let sat = (eff.eval("彩度", rel, 100.0) as f32 / 100.0).max(0.0);
                fx.hsv_shift.0 += br;
                fx.hsv_shift.1 += hue;
                fx.hsv_shift.2 = if fx.hsv_shift.2 == 0.0 { sat } else { fx.hsv_shift.2 * sat };
            }
            "グラデーション" | "グラデーションマップ" => {
                let strength = (eff.eval("強さ", rel, 100.0) / 100.0).clamp(0.0, 1.0) as f32;
                let col1 = eff
                    .text("color")
                    .or_else(|| eff.text("開始色"))
                    .or_else(|| eff.text("色1"))
                    .and_then(parse_color);
                let col2 = eff
                    .text("color2")
                    .or_else(|| eff.text("終了色"))
                    .or_else(|| eff.text("色2"))
                    .and_then(parse_color);
                if let Some(c) = col1.or(col2) {
                    fx.grad_color = Some((c, strength * 0.65));
                }
            }
            "凸エッジ" => {
                let w = eff.eval("幅", rel, 5.0) as f32;
                fx.bevel_width = w.max(1.0);
            }
            "ミラー" => {
                let tr = eff.eval("透明度", rel, 30.0) as f32;
                fx.mirror_alpha = (1.0 - tr / 100.0).clamp(0.0, 1.0);
            }
            s if s.contains("色収差") || s.contains("色ズレ") => {
                let shift = eff.eval("ズレ幅", rel, 8.0) as f32;
                let ang = (eff.eval("角度", rel, 0.0) as f32).to_radians();
                fx.chroma_shift = shift;
                fx.chroma_angle = ang;
            }
            "縁取り" => {
                fx.border_width = eff.eval("サイズ", rel, 4.0) as f32;
                fx.border_color = eff
                    .text("color")
                    .or_else(|| eff.text("縁色"))
                    .and_then(parse_color)
                    .or(Some(Color32::WHITE));
            }
            "シャドー" | "ドロップシャドウ" => {
                let dx = eff.eval("X", rel, 6.0) as f32;
                let dy = eff.eval("Y", rel, 6.0) as f32;
                let density = (eff.eval("濃さ", rel, 60.0) / 100.0).clamp(0.0, 1.0) as f32;
                fx.shadow_offset = Some(vec2(dx, dy));
                fx.shadow_alpha = density;
            }
            "単色化" => {
                let strength = (eff.eval("強さ", rel, 100.0) / 100.0).clamp(0.0, 1.0) as f32;
                let col = eff
                    .text("color")
                    .or_else(|| eff.text("色"))
                    .and_then(parse_color)
                    .unwrap_or(Color32::from_rgb(255, 200, 120));
                fx.tint_color = Some((col, strength));
            }
            "座標" => {
                lua_st.ox += eff.eval("X", rel, 0.0);
                lua_st.oy += eff.eval("Y", rel, 0.0);
                lua_st.oz += eff.eval("Z", rel, 0.0);
            }
            "拡大率" => {
                let z = eff.eval("拡大率", rel, 100.0) / 100.0;
                let sx = eff.eval("X", rel, 100.0) / 100.0;
                let sy = eff.eval("Y", rel, 100.0) / 100.0;
                lua_st.zoom *= (z * ((sx + sy) * 0.5)).max(0.0);
            }
            "透明度" => {
                let tr = (eff.eval("透明度", rel, 0.0) / 100.0).clamp(0.0, 1.0);
                lua_st.alpha *= 1.0 - tr;
            }
            "回転" => {
                lua_st.rx += eff.eval("X", rel, eff.eval("X軸回転", rel, 0.0));
                lua_st.ry += eff.eval("Y", rel, eff.eval("Y軸回転", rel, 0.0));
                lua_st.rz += eff.eval("Z", rel, eff.eval("Z軸回転", rel, 0.0));
            }
            "極座標変換" => {
                let cw = eff.eval("中心幅", rel, 0.0) as f32;
                let zm = (eff.eval("拡大率", rel, 100.0) / 100.0).max(0.05) as f32;
                let rot = eff.eval("回転", rel, 0.0) as f32;
                let swirl = eff.eval("渦巻", rel, 0.0) as f32;
                fx.polar_transform = Some((cw, zm, rot, swirl));
            }
            "画像ループ" => {
                let nx = (eff.eval("横回数", rel, 1.0).round() as u32).clamp(1, 16);
                let ny = (eff.eval("縦回数", rel, 1.0).round() as u32).clamp(1, 16);
                let sx = eff.eval("速度X", rel, 0.0) as f32;
                let sy = eff.eval("速度Y", rel, 0.0) as f32;
                if nx > 1 || ny > 1 || sx.abs() > 0.1 || sy.abs() > 0.1 {
                    fx.image_loop = Some((nx, ny, sx, sy));
                }
            }
            "オブジェクト分割" => {
                let nx = (eff.eval("横分割数", rel, 1.0).round() as u32).clamp(1, 16);
                let ny = (eff.eval("縦分割数", rel, 1.0).round() as u32).clamp(1, 16);
                if nx > 1 || ny > 1 {
                    fx.object_split = Some((nx, ny));
                }
            }
            "ラスター" => {
                let rw = eff.eval("横幅", rel, 100.0) as f32;
                let rh = eff.eval("高さ", rel, 100.0) as f32;
                let rp = eff.eval("周期", rel, 1.0) as f32;
                fx.raster = Some((rw, rh, rp));
                let phase = (time_sec as f32) * rp.max(0.1) * TAU;
                lua_st.ox += (phase.sin() * rw * 0.15) as f64;
            }
            "ノイズ" => {
                let ns = (eff.eval("強さ", rel, 50.0) / 100.0).clamp(0.0, 1.0) as f32;
                fx.noise_strength = fx.noise_strength.max(ns);
            }
            "ぼかし" | "レンズブラー" | "放射ブラー" | "方向ブラー" | "モーションブラー" => {
                let rng = eff.eval("範囲", rel, eff.eval("間隔", rel, 5.0)) as f32;
                if rng > 0.5 {
                    fx.glow_strength = (fx.glow_strength + (rng / 60.0).min(1.5)).min(8.0);
                    fx.glow_radius = (fx.glow_radius + rng * 0.8).max(6.0);
                }
            }
            "反転" => {
                let fv = eff.eval("上下反転", rel, eff.eval("check0", rel, 0.0)) >= 0.5;
                let fh = eff.eval("左右反転", rel, eff.eval("check1", rel, 0.0)) >= 0.5;
                let il = eff.eval("輝度反転", rel, eff.eval("check2", rel, 0.0)) >= 0.5;
                let ih = eff.eval("色相反転", rel, eff.eval("check3", rel, 0.0)) >= 0.5;
                let ia = eff.eval("透明度反転", rel, eff.eval("check4", rel, 0.0)) >= 0.5;
                if fv {
                    lua_st.rx += 180.0;
                }
                if fh {
                    lua_st.ry += 180.0;
                }
                fx.inversions = (
                    fx.inversions.0 || fv,
                    fx.inversions.1 || fh,
                    fx.inversions.2 || il,
                    fx.inversions.3 || ih,
                    fx.inversions.4 || ia,
                );
            }
            "エッジ抽出" => {
                let str_val = eff.eval("強さ", rel, 100.0) as f32;
                if str_val > 10.0 {
                    fx.inner_hollow_ratio = fx.inner_hollow_ratio.max(0.85);
                    if let Some(c) = eff.text("color").and_then(parse_color) {
                        fx.tint_color = Some((c, 0.9));
                    }
                }
            }
            "ルミナンスキー" => {
                let thresh = eff.eval("基準輝度", rel, 2000.0) as f32;
                if thresh > 3800.0 {
                    lua_st.alpha *= 0.85;
                }
            }
            "ディスプレイスメントマップ" => {
                let dx = eff.eval("変形X", rel, 0.0);
                let dy = eff.eval("変形Y", rel, 0.0);
                let shift = (dx.hypot(dy) * 0.15) as f32;
                if shift > 0.5 {
                    fx.chroma_shift = fx.chroma_shift.max(shift.min(18.0));
                }
            }
            _ => {
                if !ran_lua && eff.name == "アニメーション効果" {
                    let script_name = eff.text("name").unwrap_or_default();
                    let sec = script_name.split('@').next().unwrap_or(script_name);
                    match sec {
                        "揺れ" | "振動" | "震える" => {
                            let ax = eff.eval("X振幅", rel, eff.eval("track0", rel, 25.0));
                            let ay = eff.eval("Y振幅", rel, eff.eval("track1", rel, 15.0));
                            let interval = eff.eval("間隔", rel, eff.eval("track2", rel, 0.0));
                            let angle = eff.eval("角度", rel, eff.eval("track1", rel, 0.0));
                            if interval <= 0.0 {
                                lua_st.ox += aviutl_rand(-ax, ax, 0, (time_sec * 100.0) as i64);
                                lua_st.oy += aviutl_rand(-ay, ay, 1, (time_sec * 100.0) as i64);
                            } else {
                                let t = time_sec * 100.0 / interval;
                                let p = t.floor() as i64;
                                let t_frac = t - (p as f64);
                                let x0 = aviutl_rand(-1000.0, 1000.0, 0, p) / 1000.0 * ax;
                                let x1 = aviutl_rand(-1000.0, 1000.0, 0, p + 1) / 1000.0 * ax;
                                let x2 = aviutl_rand(-1000.0, 1000.0, 0, p + 2) / 1000.0 * ax;
                                let x3 = aviutl_rand(-1000.0, 1000.0, 0, p + 3) / 1000.0 * ax;

                                let y0 = aviutl_rand(-1000.0, 1000.0, 1, p) / 1000.0 * ay;
                                let y1 = aviutl_rand(-1000.0, 1000.0, 1, p + 1) / 1000.0 * ay;
                                let y2 = aviutl_rand(-1000.0, 1000.0, 1, p + 2) / 1000.0 * ay;
                                let y3 = aviutl_rand(-1000.0, 1000.0, 1, p + 3) / 1000.0 * ay;

                                lua_st.ox += aviutl_interpolation(t_frac, x0, x1, x2, x3);
                                lua_st.oy += aviutl_interpolation(t_frac, y0, y1, y2, y3);
                                if angle != 0.0 {
                                    let r0 = aviutl_rand(-1000.0, 1000.0, 2, p) / 1000.0 * angle;
                                    let r1 = aviutl_rand(-1000.0, 1000.0, 2, p + 1) / 1000.0 * angle;
                                    let r2 = aviutl_rand(-1000.0, 1000.0, 2, p + 2) / 1000.0 * angle;
                                    let r3 = aviutl_rand(-1000.0, 1000.0, 2, p + 3) / 1000.0 * angle;
                                    lua_st.rz += aviutl_interpolation(t_frac, r0, r1, r2, r3);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Lua スクリプト内で `obj.effect(...)` が呼ばれた場合の反映
    for fcall in &lua_st.filter_calls {
        match fcall.name.as_str() {
            "グロー" | "発光" | "拡散光" | "閃光" => {
                let s = fcall.params.get("強さ").map(|v| v.as_num() as f32).unwrap_or(60.0);
                let d = fcall.params.get("拡散").map(|v| v.as_num() as f32).unwrap_or(30.0);
                fx.glow_strength = (fx.glow_strength + s / 85.0).min(8.0);
                fx.glow_radius = (fx.glow_radius + d).max(8.0);
                if let Some(c) = fcall.params.get("color").map(|v| u32_to_color32(v.as_num() as u32)) {
                    fx.glow_color = Some(c);
                }
            }
            "縁取り" => {
                fx.border_width = fcall.params.get("サイズ").map(|v| v.as_num() as f32).unwrap_or(4.0);
                fx.border_color = fcall
                    .params
                    .get("color")
                    .map(|v| u32_to_color32(v.as_num() as u32))
                    .or(Some(Color32::WHITE));
            }
            "色ズレ" | "色収差" => {
                fx.chroma_shift = fcall.params.get("ズレ幅").map(|v| v.as_num() as f32).unwrap_or(8.0);
                if let Some(ang) = fcall.params.get("角度") {
                    fx.chroma_angle = (ang.as_num() as f32).to_radians();
                }
            }
            "ぼかし" | "レンズブラー" | "放射ブラー" | "方向ブラー" => {
                let rng = fcall.params.get("範囲").map(|v| v.as_num() as f32).unwrap_or(10.0);
                if rng > 0.5 {
                    fx.glow_strength = (fx.glow_strength + (rng / 60.0).min(1.5)).min(8.0);
                    fx.glow_radius = (fx.glow_radius + rng * 0.8).max(6.0);
                }
            }
            "色調補正" | "拡張色調補正" => {
                let br = (fcall.params.get("明るさ").map(|v| v.as_num() as f32).unwrap_or(100.0) - 100.0) / 100.0;
                let hue = fcall.params.get("色相").map(|v| v.as_num() as f32).unwrap_or(0.0);
                let sat = (fcall.params.get("彩度").map(|v| v.as_num() as f32).unwrap_or(100.0) / 100.0).max(0.0);
                fx.hsv_shift.0 += br;
                fx.hsv_shift.1 += hue;
                fx.hsv_shift.2 = if fx.hsv_shift.2 == 0.0 { sat } else { fx.hsv_shift.2 * sat };
            }
            "シャドー" | "ドロップシャドウ" => {
                let dx = fcall.params.get("X").map(|v| v.as_num() as f32).unwrap_or(6.0);
                let dy = fcall.params.get("Y").map(|v| v.as_num() as f32).unwrap_or(6.0);
                let density = (fcall.params.get("濃さ").map(|v| v.as_num() as f32).unwrap_or(60.0) / 100.0).clamp(0.0, 1.0);
                fx.shadow_offset = Some(vec2(dx, dy));
                fx.shadow_alpha = density;
            }
            "単色化" => {
                let strength = (fcall.params.get("強さ").map(|v| v.as_num() as f32).unwrap_or(100.0) / 100.0).clamp(0.0, 1.0);
                let col = fcall
                    .params
                    .get("color")
                    .map(|v| u32_to_color32(v.as_num() as u32))
                    .unwrap_or(Color32::from_rgb(255, 200, 120));
                fx.tint_color = Some((col, strength));
            }
            "ミラー" => {
                let tr = fcall.params.get("透明度").map(|v| v.as_num() as f32).unwrap_or(30.0);
                fx.mirror_alpha = (1.0 - tr / 100.0).clamp(0.0, 1.0);
            }
            "ラスター" => {
                let rw = fcall.params.get("横幅").map(|v| v.as_num() as f32).unwrap_or(50.0);
                let rh = fcall.params.get("高さ").map(|v| v.as_num() as f32).unwrap_or(50.0);
                let rp = fcall.params.get("周期").map(|v| v.as_num() as f32).unwrap_or(1.0);
                fx.raster = Some((rw, rh, rp));
            }
            "ノイズ" => {
                let ns = (fcall.params.get("強さ").map(|v| v.as_num() as f32).unwrap_or(40.0) / 100.0).clamp(0.0, 1.0);
                fx.noise_strength = fx.noise_strength.max(ns);
            }
            "極座標変換" => {
                let cw = fcall.params.get("中心幅").map(|v| v.as_num() as f32).unwrap_or(0.0);
                let zm = (fcall.params.get("拡大率").map(|v| v.as_num() as f32).unwrap_or(100.0) / 100.0).max(0.05);
                let rot = fcall.params.get("回転").map(|v| v.as_num() as f32).unwrap_or(0.0);
                let swirl = fcall.params.get("渦巻").map(|v| v.as_num() as f32).unwrap_or(2.0);
                fx.polar_transform = Some((cw, zm, rot, swirl));
            }
            "クリッピング" => {
                fx.clip_insets[0] += fcall.params.get("上").map(|v| v.as_num() as f32).unwrap_or(0.0).max(0.0);
                fx.clip_insets[1] += fcall.params.get("下").map(|v| v.as_num() as f32).unwrap_or(0.0).max(0.0);
                fx.clip_insets[2] += fcall.params.get("左").map(|v| v.as_num() as f32).unwrap_or(0.0).max(0.0);
                fx.clip_insets[3] += fcall.params.get("右").map(|v| v.as_num() as f32).unwrap_or(0.0).max(0.0);
            }
            "マスク" => {
                let sz = fcall.params.get("サイズ").map(|v| v.as_num() as f32).unwrap_or(100.0);
                let inv = fcall.params.get("マスクの反転").map(|v| v.as_num()).unwrap_or(0.0) >= 0.5;
                if inv {
                    fx.inner_hollow_ratio = fx.inner_hollow_ratio.max((sz / (approx_size as f32).max(1.0)).clamp(0.1, 0.96));
                }
            }
            "扇クリッピングR" => {
                let end_deg = fcall.params.get("終了角").map(|v| v.as_num() as f32).unwrap_or(360.0);
                let start_deg = fcall.params.get("開始角").map(|v| v.as_num() as f32).unwrap_or(0.0);
                fx.sector_clip = Some((start_deg, end_deg));
            }
            _ => {}
        }
    }

    if lua_st.aspect.abs() > 0.001 {
        fx.aspect_override = lua_st.aspect as f32;
    }

    let gt = ctx.group_transform.unwrap_or_default();
    let alpha = (lua_st.alpha * gt.alpha).clamp(0.0, 1.0) as f32;
    if alpha <= 0.0 && lua_st.draw_calls.is_empty() && lua_st.poly_calls.is_empty() {
        return None;
    }

    // グループ制御の 3D 回転・スケーリング・平行移動を合成
    let local_pos = [
        (base_x + lua_st.ox - lua_st.cx) * gt.zoom,
        (base_y + lua_st.oy - lua_st.cy) * gt.zoom,
        (base_z + lua_st.oz - lua_st.cz) * gt.zoom,
    ];
    let rot_pos = rotate_3d(local_pos, gt.rx, gt.ry, gt.rz);
    let raw_x = gt.ox + rot_pos[0];
    let raw_y = gt.oy + rot_pos[1];
    let raw_z = gt.oz + rot_pos[2];
    let raw_zoom = (base_zoom * lua_st.zoom * gt.zoom).max(0.0);
    let raw_rx = lua_st.rx + gt.rx;
    let raw_ry = lua_st.ry + gt.ry;
    let raw_rz = lua_st.rz + gt.rz;

    let (x, y, zoom, angle) = if obj.camera_target {
        if let Some(ref cam) = ctx.camera {
            if let Some((proj_x, proj_y, scale_3d, _)) = cam.project([raw_x, raw_y, raw_z]) {
                (
                    proj_x,
                    proj_y,
                    (raw_zoom * scale_3d as f64) as f32,
                    ((raw_rz - cam.roll) as f32).to_radians(),
                )
            } else {
                return None;
            }
        } else {
            (
                raw_x as f32,
                raw_y as f32,
                raw_zoom as f32,
                (raw_rz as f32).to_radians(),
            )
        }
    } else {
        (
            raw_x as f32,
            raw_y as f32,
            raw_zoom as f32,
            (raw_rz as f32).to_radians(),
        )
    };

    // `obj.drawpoly(...)` が呼ばれている場合はポリゴン群を描画
    let mut poly_bounds: Option<Rect> = None;
    for poly in &lua_st.poly_calls {
        let pts: Vec<Pos2> = poly
            .pts
            .iter()
            .map(|&(px, py, pz)| {
                let p_rot = rotate_3d([px * gt.zoom, py * gt.zoom, pz * gt.zoom], raw_rx, raw_ry, raw_rz);
                let wx = raw_x + p_rot[0];
                let wy = raw_y + p_rot[1];
                let wz = raw_z + p_rot[2];
                if obj.camera_target {
                    if let Some(ref cam) = ctx.camera {
                        if let Some((proj_x, proj_y, _, _)) = cam.project([wx, wy, wz]) {
                            return ctx.canvas.center() + vec2(proj_x, proj_y) * ctx.scale;
                        }
                    }
                }
                ctx.canvas.center() + vec2(wx as f32, wy as f32) * ctx.scale
            })
            .collect();
        let col = poly
            .color
            .map(u32_to_color32)
            .or_else(|| src.text("color").and_then(parse_color))
            .unwrap_or(Color32::WHITE)
            .gamma_multiply((poly.alpha as f32 * alpha).clamp(0.0, 1.0));
        let c = Pos2::new(
            pts.iter().map(|p| p.x).sum::<f32>() / 4.0,
            pts.iter().map(|p| p.y).sum::<f32>() / 4.0,
        );
        fan(ctx.painter, c, &pts, col);
        let r = Rect::from_points(&pts);
        poly_bounds = Some(poly_bounds.map(|b| b.union(r)).unwrap_or(r));
    }

    // `obj.draw(...)` がスクリプト内で複数回呼ばれている場合（円形配置・砕け散る・ランダム配置等）は各インスタンスを描画
    if !lua_st.draw_calls.is_empty() {
        let mut combined_bounds: Option<Rect> = poly_bounds;
        for dc in &lua_st.draw_calls {
            let dc_rot = rotate_3d([dc.ox * gt.zoom, dc.oy * gt.zoom, dc.oz * gt.zoom], gt.rx, gt.ry, gt.rz);
            let inst_raw_x = raw_x + dc_rot[0];
            let inst_raw_y = raw_y + dc_rot[1];
            let inst_raw_z = raw_z + dc_rot[2];
            let inst_raw_zoom = (raw_zoom * dc.zoom).max(0.0);
            let inst_raw_rx = raw_rx + dc.rx;
            let inst_raw_ry = raw_ry + dc.ry;
            let inst_raw_rz = raw_rz + dc.rz;

            let (inst_x, inst_y, inst_zoom, inst_angle) = if obj.camera_target {
                if let Some(ref cam) = ctx.camera {
                    if let Some((proj_x, proj_y, scale_3d, _)) = cam.project([inst_raw_x, inst_raw_y, inst_raw_z]) {
                        (
                            proj_x,
                            proj_y,
                            (inst_raw_zoom * scale_3d as f64) as f32,
                            ((inst_raw_rz - cam.roll) as f32).to_radians(),
                        )
                    } else {
                        continue;
                    }
                } else {
                    (
                        inst_raw_x as f32,
                        inst_raw_y as f32,
                        inst_raw_zoom as f32,
                        (inst_raw_rz as f32).to_radians(),
                    )
                }
            } else {
                (
                    inst_raw_x as f32,
                    inst_raw_y as f32,
                    inst_raw_zoom as f32,
                    (inst_raw_rz as f32).to_radians(),
                )
            };

            let inst_alpha = (dc.alpha * gt.alpha).clamp(0.0, 1.0) as f32;
            let prim = dc.primitive.as_ref().or(lua_st.loaded_primitive.as_ref());
            let inst_transform_3d = if obj.camera_target {
                Some((
                    [inst_raw_x, inst_raw_y, inst_raw_z],
                    [inst_raw_rx, inst_raw_ry, inst_raw_rz],
                    inst_raw_zoom as f32,
                ))
            } else {
                None
            };
            let mut inst_fx = fx;
            if let Some(clip) = dc.clip {
                inst_fx.clip_insets[0] += clip[0] as f32;
                inst_fx.clip_insets[1] += clip[1] as f32;
                inst_fx.clip_insets[2] += clip[2] as f32;
                inst_fx.clip_insets[3] += clip[3] as f32;
            }
            if let Some(b) = draw_single_instance(
                ctx,
                obj,
                src,
                rel,
                inst_x,
                inst_y,
                inst_zoom,
                inst_angle,
                inst_alpha,
                prim,
                &inst_fx,
                inst_transform_3d,
                single_text_char,
                selected,
            ) {
                combined_bounds = Some(combined_bounds.map(|prev| prev.union(b)).unwrap_or(b));
            }
        }
        if selected {
            if let Some(b) = combined_bounds {
                ctx.painter.rect_stroke(b.expand(2.0), 0.0, Stroke::new(1.0_f32, Color32::from_rgb(255, 200, 0)));
            }
        }
        return combined_bounds;
    }

    if !lua_st.poly_calls.is_empty() {
        return poly_bounds;
    }

    let transform_3d = if obj.camera_target {
        Some(([raw_x, raw_y, raw_z], [raw_rx, raw_ry, raw_rz], raw_zoom as f32))
    } else {
        None
    };

    // 「画像ループ」フィルタが有効な場合は nx × ny タイル状に展開して描画
    if let Some((nx, ny, sx, sy)) = fx.image_loop {
        if nx > 1 || ny > 1 {
            let tile_step = (approx_size as f32 * (raw_zoom as f32)).max(24.0);
            let scroll_x = (time_sec as f32 * sx) % tile_step;
            let scroll_y = (time_sec as f32 * sy) % tile_step;
            let mut loop_bounds: Option<Rect> = None;
            let mut tile_fx = fx;
            tile_fx.image_loop = None;
            for iy in 0..ny {
                for ix in 0..nx {
                    let ox_tile = (ix as f32 - (nx as f32 - 1.0) * 0.5) * tile_step + scroll_x;
                    let oy_tile = (iy as f32 - (ny as f32 - 1.0) * 0.5) * tile_step + scroll_y;
                    if let Some(b) = draw_single_instance(
                        ctx,
                        obj,
                        src,
                        rel,
                        x + ox_tile,
                        y + oy_tile,
                        zoom,
                        angle,
                        alpha,
                        lua_st.loaded_primitive.as_ref(),
                        &tile_fx,
                        transform_3d,
                        single_text_char,
                        false,
                    ) {
                        loop_bounds = Some(loop_bounds.map(|prev| prev.union(b)).unwrap_or(b));
                    }
                }
            }
            if selected {
                if let Some(b) = loop_bounds {
                    ctx.painter.rect_stroke(b.expand(2.0), 0.0, Stroke::new(1.0_f32, Color32::from_rgb(255, 200, 0)));
                }
            }
            return loop_bounds;
        }
    }

    let bounds = draw_single_instance(
        ctx,
        obj,
        src,
        rel,
        x,
        y,
        zoom,
        angle,
        alpha,
        lua_st.loaded_primitive.as_ref(),
        &fx,
        transform_3d,
        single_text_char,
        selected,
    );

    if selected {
        if let Some(b) = bounds {
            ctx.painter.rect_stroke(b.expand(2.0), 0.0, Stroke::new(1.0_f32, Color32::from_rgb(255, 200, 0)));
        }
    }
    bounds
}

/// フィルタオブジェクトのプレビュー（下位レイヤー全体に掛かる：色調補正・32bit/64bit `.auf` フィルタ・シーンチェンジ等）。
fn draw_filter_object(ctx: &DrawCtx, obj: &TimelineObject, rel: Frame) {
    let Some(src) = obj.source() else { return };
    if src.name == "色調補正" {
        let b = src.eval("明るさ", rel, 100.0) as f32;
        let overlay = if b >= 100.0 {
            Color32::from_white_alpha((((b - 100.0) / 100.0).min(1.0) * 160.0) as u8)
        } else {
            Color32::from_black_alpha((((100.0 - b) / 100.0).min(1.0) * 255.0) as u8)
        };
        ctx.painter.rect_filled(ctx.canvas, 0.0, overlay);
    } else if src.name.contains("グロー") || src.name.contains("発光") || src.name.contains("拡散光") || src.name.contains("閃光") {
        let strength = (src.eval("強さ", rel, 60.0) / 200.0).clamp(0.0, 1.0) as f32;
        let overlay = Color32::from_rgba_unmultiplied(255, 248, 215, (strength * 85.0) as u8);
        ctx.painter.rect_filled(ctx.canvas, 0.0, overlay);
    } else if src.name.contains("色収差") || src.name.contains("色ズレ") || src.name.contains("色ずれ") {
        let shift = (src.eval("ズレ幅", rel, src.eval("ずれ幅", rel, 8.0)) as f32 * ctx.scale).max(1.0);
        let r_rect = Rect::from_min_max(ctx.canvas.left_top(), pos2(ctx.canvas.left() + shift * 2.0, ctx.canvas.bottom()));
        let b_rect = Rect::from_min_max(pos2(ctx.canvas.right() - shift * 2.0, ctx.canvas.top()), ctx.canvas.right_bottom());
        ctx.painter.rect_filled(r_rect, 0.0, Color32::from_rgba_unmultiplied(255, 40, 40, 85));
        ctx.painter.rect_filled(b_rect, 0.0, Color32::from_rgba_unmultiplied(40, 180, 255, 85));
    } else if src.name == "シーンチェンジ" {
        let progress = (rel as f32 / (obj.length().max(1) as f32)).clamp(0.0, 1.0);
        // ピーク波形（トランジションの中間 p=0.5 で最大 1.0 になり、両端で 0.0 になる sin カーブ）
        let wave = (progress * PI).sin().clamp(0.0, 1.0);
        let sc_name = src.text("name").unwrap_or_default();
        let sc_type = src.text("type").and_then(|t| t.parse::<i32>().ok()).unwrap_or(0);

        match sc_name {
            "発光" => {
                let flash = (wave * 235.0) as u8;
                ctx.painter.rect_filled(ctx.canvas, 0.0, Color32::from_white_alpha(flash));
            }
            "フェードアウトイン" => {
                let fade = (wave * 250.0) as u8;
                ctx.painter.rect_filled(ctx.canvas, 0.0, Color32::from_black_alpha(fade));
            }
            "レンズブラー" | "放射ブラー" | "ぼかし" => {
                let bloom = (wave * 115.0) as u8;
                ctx.painter.rect_filled(ctx.canvas, 0.0, Color32::from_rgba_unmultiplied(230, 240, 255, bloom));
            }
            "リール回転" | "スライス" | "押し出し(横)" | "押し出し(縦)" => {
                // リール回転・スライス時は画面を横切るモーションブラー帯を描画
                let bands = 5;
                let h = ctx.canvas.height() / bands as f32;
                let alpha_b = (wave * 95.0) as u8;
                for i in 0..bands {
                    let y0 = ctx.canvas.top() + ((i as f32 + progress * 3.0) % bands as f32) * h;
                    let r = Rect::from_min_max(pos2(ctx.canvas.left(), y0), pos2(ctx.canvas.right(), (y0 + h * 0.35).min(ctx.canvas.bottom())));
                    ctx.painter.rect_filled(r, 0.0, Color32::from_white_alpha(alpha_b));
                }
            }
            "ワイプ(円)" | "図形ワイプ" => {
                let max_r = ctx.canvas.width().hypot(ctx.canvas.height()) * 0.55;
                let r = max_r * (1.0 - wave * 0.85);
                ctx.painter.circle_stroke(ctx.canvas.center(), r, Stroke::new((max_r - r).max(1.0), Color32::from_black_alpha((wave * 200.0) as u8)));
            }
            "砕け散る" => {
                let count = 16;
                for i in 0..count {
                    let rx = aviutl_rand(-0.45, 0.45, i, 1) as f32 * ctx.canvas.width();
                    let ry = (aviutl_rand(-0.45, 0.45, i, 2) as f32 + progress * 0.4) * ctx.canvas.height();
                    let sz = (24.0 * (1.0 - progress * 0.7) * ctx.scale).max(4.0);
                    let p = ctx.canvas.center() + vec2(rx, ry);
                    ctx.painter.rect_filled(
                        Rect::from_center_size(p, vec2(sz, sz)),
                        0.0,
                        Color32::from_white_alpha(((1.0 - progress) * 140.0) as u8),
                    );
                }
            }
            _ => {
                if sc_type == 14 {
                    let fade = (wave * 250.0) as u8;
                    ctx.painter.rect_filled(ctx.canvas, 0.0, Color32::from_black_alpha(fade));
                } else {
                    let fade = (wave * 140.0) as u8;
                    ctx.painter.rect_filled(ctx.canvas, 0.0, Color32::from_black_alpha(fade));
                }
            }
        }
    }
}