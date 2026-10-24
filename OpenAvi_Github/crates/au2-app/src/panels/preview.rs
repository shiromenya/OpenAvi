//! プレビュー画面。
//!
//! - 図形・テキスト・画像ファイル・動画ファイル・カスタムオブジェクト・フィルタオブジェクトを合成描画します。
//! - 初代 AviUtl (32-bit) および AviUtl2 (64-bit) の拡張編集 Lua スクリプト（`.anm`, `.obj`, `.cam`, `.scn`）と
//!   ネイティブフィルタプラグイン（`.auf` 32bit/64bit）をリアルタイム評価してプレビューに反映します。

use std::f32::consts::{PI, TAU};

use au2_core::{EffectKind, Frame, ObjectCategory, ObjectId, TimelineObject};
use au2_plugin::{LuaLoadedPrimitive, LuaObjState, PluginRegistry};
use eframe::egui::{self, emath::Rot2, pos2, vec2, Align2, Color32, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke};

use super::parse_color;
use crate::state::EditorState;

pub fn ui(ui: &mut egui::Ui, st: &mut EditorState) {
    let egui_ctx = ui.ctx().clone();

    // 下部のシークバー＆再生コントロール領域の高さ（マージン込みで約 82px）を残して上部にプレビューキャンバスを配置
    let control_height = 84.0;
    let avail = ui.available_size();
    let preview_size = vec2(avail.x.max(64.0), (avail.y - control_height).max(64.0));

    let (resp, painter) = ui.allocate_painter(preview_size, Sense::click());
    let area = resp.rect;
    painter.rect_filled(area, 0.0, Color32::from_gray(24));

    let s = &st.scene.settings;
    let (sw, sh) = (s.width.max(1) as f32, s.height.max(1) as f32);
    let scale = (area.width() / sw).min(area.height() / sh).max(0.0001);
    let canvas = Rect::from_center_size(area.center(), vec2(sw * scale, sh * scale));
    painter.rect_filled(canvas, 0.0, Color32::BLACK);
    let painter = painter.with_clip_rect(canvas.intersect(area));

    let fps = s.frame_rate.as_f64();
    let frame = st.frame;

    // 1. 現在フレームに存在する画像・動画オブジェクトのテクスチャを事前取得
    let mut media_requests: Vec<(ObjectId, String, u32, f64)> = Vec::new();
    for (li, obj) in st.scene.objects_at(frame) {
        if !st.scene.layers[li].visible {
            continue;
        }
        if let Some(src) = obj.source().filter(|s| s.enabled) {
            if matches!(src.kind, EffectKind::ImageFile | EffectKind::VideoFile) {
                if let Some(file_str) = src.text("file").filter(|f| !f.trim().is_empty()) {
                    let rel = frame - obj.frame_start;
                    let start_pos = (src.eval("再生位置", rel, 1.0) - 1.0).max(0.0);
                    let speed = (src.eval("再生速度", rel, 100.0) / 100.0).max(0.0);
                    let media_frame = (start_pos + rel as f64 * speed).floor() as u32;
                    let media_sec = media_frame as f64 / fps.max(1.0);
                    media_requests.push((obj.id, file_str.to_string(), media_frame, media_sec));
                }
            }
        }
    }

    let mut resolved_textures: std::collections::HashMap<ObjectId, (egui::TextureId, [usize; 2])> =
        std::collections::HashMap::new();
    for (id, file_str, media_frame, media_sec) in media_requests {
        if let Some(tex) = st.get_media_texture(&egui_ctx, &file_str, media_frame, media_sec) {
            resolved_textures.insert(id, tex);
        }
    }

    // 2. レイヤー順に合成描画
    let ctx = DrawCtx {
        painter: &painter,
        canvas,
        scale,
        fps,
        screen_w: sw as f64,
        screen_h: sh as f64,
        textures: &resolved_textures,
        plugins: &st.plugins,
    };
    for (li, obj) in st.scene.objects_at(frame) {
        if !st.scene.layers[li].visible {
            continue;
        }
        draw_object(&ctx, li, obj, frame - obj.frame_start, st.selected == Some(obj.id));
    }

    painter.text(
        canvas.left_top() + vec2(6.0, 4.0),
        Align2::LEFT_TOP,
        format!("{} / frame {}", st.scene.settings.frame_rate.timecode(frame), frame + 1),
        FontId::monospace(12.0),
        Color32::from_white_alpha(160),
    );

    // 3. 動画プレビュー下部にシークバーと再生コントロールボタンを配置
    ui.add_space(4.0);
    bottom_transport_bar(ui, st);
}

fn bottom_transport_bar(ui: &mut egui::Ui, st: &mut EditorState) {
    let last = st.last_frame();
    let mut f = st.frame;

    egui::Frame::none()
        .fill(Color32::from_rgb(20, 22, 27))
        .stroke(Stroke::new(1.0_f32, Color32::from_gray(38)))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(10.0, 6.0))
        .show(ui, |ui| {
            // 1段目：シークバー
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 8.0).max(80.0);
                if ui
                    .add(egui::Slider::new(&mut f, 0..=last).show_value(false))
                    .on_hover_text("シークバー（ドラッグで再生位置を移動）")
                    .changed()
                {
                    st.set_frame(f);
                }
            });

            ui.add_space(3.0);

            // 2段目：プロ仕様 3 カラムレイアウト（すべて高さ 24px で垂直中央に統一）
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 0.0);

                let rate = st.scene.settings.frame_rate;
                let tc = rate.timecode(st.frame);

                let tc_badge = egui::Button::new(
                    egui::RichText::new("TC")
                        .monospace()
                        .size(10.0)
                        .strong()
                        .color(Color32::from_rgb(147, 197, 253)),
                )
                .fill(Color32::from_rgb(28, 34, 46))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 60, 90)))
                .min_size(vec2(26.0, 24.0));
                ui.add(tc_badge);

                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(&tc)
                        .monospace()
                        .size(13.0)
                        .strong()
                        .color(Color32::from_rgb(240, 244, 252)),
                );
                ui.label(
                    egui::RichText::new(format!("({} / {} F)", st.frame + 1, last + 1))
                        .monospace()
                        .size(11.0)
                        .color(Color32::from_gray(140)),
                );

                ui.separator();

                let nav_btn = |ui: &mut egui::Ui, text: &'static str, tip: &'static str| -> egui::Response {
                    ui.add(
                        egui::Button::new(egui::RichText::new(text).size(12.0).color(Color32::from_gray(225)))
                            .min_size(vec2(28.0, 24.0)),
                    )
                    .on_hover_text(tip)
                };

                if nav_btn(ui, "⏮", "先頭フレームへ (Home)").clicked() {
                    st.set_frame(0);
                }
                if nav_btn(ui, "◀", "1 コマ戻る (←)").clicked() {
                    st.step(-1);
                }

                let (play_label, play_color) = if st.playing {
                    ("⏸ 一時停止", Color32::from_rgb(220, 38, 38))
                } else {
                    ("▶ 再生", Color32::from_rgb(37, 99, 235))
                };
                let play_btn = egui::Button::new(egui::RichText::new(play_label).strong().size(12.0).color(Color32::WHITE))
                    .fill(play_color)
                    .min_size(vec2(86.0, 24.0));
                if ui.add(play_btn).on_hover_text("再生 / 一時停止 (Space)").clicked() {
                    st.toggle_play();
                }

                if nav_btn(ui, "▶|", "1 コマ進む (→)").clicked() {
                    st.step(1);
                }
                if nav_btn(ui, "⏭", "最終フレームへ (End)").clicked() {
                    st.set_frame(last);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let s = &st.scene.settings;
                    ui.label(
                        egui::RichText::new(format!("{}×{}  {:.0}fps", s.width, s.height, s.frame_rate.as_f64()))
                            .size(11.0)
                            .color(Color32::from_gray(145)),
                    );
                });
            });
        });
}

struct DrawCtx<'a> {
    painter: &'a egui::Painter,
    canvas: Rect,
    scale: f32,
    fps: f64,
    screen_w: f64,
    screen_h: f64,
    textures: &'a std::collections::HashMap<ObjectId, (egui::TextureId, [usize; 2])>,
    plugins: &'a PluginRegistry,
}

#[derive(Default, Clone, Copy)]
struct FilterVisuals {
    glow_strength: f32,
    chroma_shift: f32,
    chroma_angle: f32,
    border_width: f32,
    border_color: Option<Color32>,
    shadow_offset: Option<egui::Vec2>,
    shadow_alpha: f32,
    tint_color: Option<(Color32, f32)>,
}

fn u32_to_color32(rgb: u32) -> Color32 {
    let r = ((rgb >> 16) & 0xFF) as u8;
    let g = ((rgb >> 8) & 0xFF) as u8;
    let b = (rgb & 0xFF) as u8;
    Color32::from_rgb(r, g, b)
}

fn draw_object(ctx: &DrawCtx, layer_idx: usize, obj: &TimelineObject, rel: Frame, selected: bool) {
    let Some(src) = obj.source() else { return };
    if !src.enabled {
        return;
    }
    match obj.category() {
        ObjectCategory::Media => {}
        ObjectCategory::Filter => {
            draw_filter_object(ctx, obj, rel);
            return;
        }
        _ => return,
    }

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

    let time_sec = rel as f64 / ctx.fps.max(1.0);
    let total_sec = obj.length() as f64 / ctx.fps.max(1.0);

    // Lua スクリプト実行ステートの初期化
    let mut lua_st = LuaObjState {
        x: base_x,
        y: base_y,
        z: base_z,
        rz: base_rot,
        zoom: 1.0,
        alpha: base_alpha,
        time: time_sec,
        totaltime: total_sec,
        frame: rel as i64,
        totalframe: obj.length() as i64,
        layer: (layer_idx + 1) as i64,
        screen_w: ctx.screen_w,
        screen_h: ctx.screen_h,
        framerate: ctx.fps,
        ..Default::default()
    };

    let mut fx = FilterVisuals::default();

    // フィルタ効果・スクリプト (.anm / .obj / .cam / .scn) および .auf プラグインの評価
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
        let ran_lua = ctx.plugins.execute_effect(eff, rel, &mut lua_st);

        // 2) 内蔵フィルタおよび .auf (32bit / 64bit) フィルタプラグインの視覚効果評価
        if eff.name == "フェード" {
            let fin = eff.eval("イン", rel, 0.0) * ctx.fps;
            let fout = eff.eval("アウト", rel, 0.0) * ctx.fps;
            let remain = (obj.length() - 1 - rel) as f64;
            if fin > 0.0 {
                lua_st.alpha *= (rel as f64 / fin).min(1.0);
            }
            if fout > 0.0 {
                lua_st.alpha *= (remain / fout).min(1.0);
            }
        } else if eff.name == "振動" {
            let ax = eff.eval("X", rel, 15.0);
            let ay = eff.eval("Y", rel, 15.0);
            let period = eff.eval("周期", rel, 1.0).max(0.1);
            let phase = (rel as f64 / period) * (TAU as f64);
            lua_st.ox += phase.sin() * ax;
            lua_st.oy += (phase * 1.3).cos() * ay;
        } else if eff.name.contains("グロー") || eff.name.contains("発光") || eff.name.contains("拡散光") || eff.name.contains("閃光") {
            let strength = eff.eval("強さ", rel, 60.0) as f32;
            fx.glow_strength = (fx.glow_strength + strength / 100.0).min(2.5);
        } else if eff.name.contains("色収差") || eff.name.contains("色ズレ") {
            let shift = eff.eval("ズレ幅", rel, 8.0) as f32;
            let ang = (eff.eval("角度", rel, 0.0) as f32).to_radians();
            fx.chroma_shift = shift;
            fx.chroma_angle = ang;
        } else if eff.name == "縁取り" {
            fx.border_width = eff.eval("サイズ", rel, 4.0) as f32;
            fx.border_color = eff.text("color").and_then(parse_color).or(Some(Color32::WHITE));
        } else if eff.name == "シャドー" || eff.name == "ドロップシャドウ" {
            let dx = eff.eval("X", rel, 6.0) as f32;
            let dy = eff.eval("Y", rel, 6.0) as f32;
            let density = (eff.eval("濃さ", rel, 60.0) / 100.0).clamp(0.0, 1.0) as f32;
            fx.shadow_offset = Some(vec2(dx, dy));
            fx.shadow_alpha = density;
        } else if eff.name == "単色化" {
            let strength = (eff.eval("強さ", rel, 100.0) / 100.0).clamp(0.0, 1.0) as f32;
            let col = eff.text("color").and_then(parse_color).unwrap_or(Color32::from_rgb(255, 200, 120));
            fx.tint_color = Some((col, strength));
        } else if !ran_lua && eff.name == "アニメーション効果" {
            // スクリプトファイル未存在時のフォールバック評価
            let script_name = eff.text("name").unwrap_or_default();
            let sec = script_name.split('@').next().unwrap_or(script_name);
            match sec {
                "揺れ" | "振動" => {
                    let ax = eff.eval("X振幅", rel, 25.0);
                    let ay = eff.eval("Y振幅", rel, 15.0);
                    let period = eff.eval("周期", rel, 0.4).max(0.01);
                    let phase = (time_sec / period) * (TAU as f64);
                    lua_st.ox += phase.sin() * ax;
                    lua_st.oy += (phase * 1.3).cos() * ay;
                }
                "点滅" => {
                    let period = eff.eval("周期", rel, 0.5).max(0.01);
                    let min_t = (eff.eval("最小透明度", rel, 60.0) / 100.0).clamp(0.0, 1.0);
                    let w = (((time_sec / period) * (TAU as f64)).sin() + 1.0) * 0.5;
                    lua_st.alpha *= 1.0 - min_t * w;
                }
                "回転" => {
                    let deg_per_sec = eff.eval("角速度", rel, 90.0);
                    lua_st.rz += time_sec * deg_per_sec;
                }
                _ => {}
            }
        }
    }

    // Lua スクリプト内で `obj.effect(...)` が呼ばれた場合の反映
    for fcall in &lua_st.filter_calls {
        match fcall.name.as_str() {
            "グロー" | "発光" | "拡散光" | "閃光" => {
                let s = fcall.params.get("強さ").map(|v| v.as_num() as f32).unwrap_or(60.0);
                fx.glow_strength = (fx.glow_strength + s / 100.0).min(2.5);
            }
            "縁取り" => {
                fx.border_width = fcall.params.get("サイズ").map(|v| v.as_num() as f32).unwrap_or(4.0);
                fx.border_color = fcall
                    .params
                    .get("color")
                    .map(|v| u32_to_color32(v.as_num() as u32))
                    .or(Some(Color32::WHITE));
            }
            "色ズレ" => {
                fx.chroma_shift = fcall.params.get("ズレ幅").map(|v| v.as_num() as f32).unwrap_or(8.0);
            }
            _ => {}
        }
    }

    let alpha = lua_st.alpha.clamp(0.0, 1.0) as f32;
    if alpha <= 0.0 && lua_st.draw_calls.is_empty() && lua_st.poly_calls.is_empty() {
        return;
    }

    let x = (base_x + lua_st.ox - lua_st.cx) as f32;
    let y = (base_y + lua_st.oy - lua_st.cy) as f32;
    let zoom = (base_zoom * lua_st.zoom).max(0.0) as f32;
    let angle = (lua_st.rz as f32).to_radians();

    // `obj.drawpoly(...)` が呼ばれている場合はポリゴン群を描画
    for poly in &lua_st.poly_calls {
        let pts: Vec<Pos2> = poly
            .pts
            .iter()
            .map(|&(px, py, _)| ctx.canvas.center() + vec2(x + px as f32, y + py as f32) * ctx.scale)
            .collect();
        let col = poly.color.map(u32_to_color32).unwrap_or(Color32::WHITE).gamma_multiply(poly.alpha as f32);
        let c = Pos2::new(
            pts.iter().map(|p| p.x).sum::<f32>() / 4.0,
            pts.iter().map(|p| p.y).sum::<f32>() / 4.0,
        );
        fan(ctx.painter, c, &pts, col);
    }

    // `obj.draw(...)` がスクリプト内で複数回呼ばれている場合（円形複製やパーティクル等）は各インスタンスを描画
    if !lua_st.draw_calls.is_empty() {
        let mut last_bounds = None;
        for dc in &lua_st.draw_calls {
            let inst_x = (base_x + dc.ox) as f32;
            let inst_y = (base_y + dc.oy) as f32;
            let inst_zoom = (base_zoom * dc.zoom).max(0.0) as f32;
            let inst_angle = (dc.rz as f32).to_radians();
            let inst_alpha = dc.alpha.clamp(0.0, 1.0) as f32;
            let prim = dc.primitive.as_ref().or(lua_st.loaded_primitive.as_ref());
            last_bounds = draw_single_instance(
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
                &fx,
            );
        }
        if selected {
            if let Some(b) = last_bounds {
                ctx.painter.rect_stroke(b.expand(2.0), 0.0, Stroke::new(1.0_f32, Color32::from_rgb(255, 200, 0)));
            }
        }
        return;
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
    );

    if selected {
        if let Some(b) = bounds {
            ctx.painter.rect_stroke(b.expand(2.0), 0.0, Stroke::new(1.0_f32, Color32::from_rgb(255, 200, 0)));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_single_instance(
    ctx: &DrawCtx,
    obj: &TimelineObject,
    src: &au2_core::Effect,
    rel: Frame,
    x: f32,
    y: f32,
    zoom: f32,
    angle: f32,
    alpha: f32,
    loaded_prim: Option<&LuaLoadedPrimitive>,
    fx: &FilterVisuals,
) -> Option<Rect> {
    let center = ctx.canvas.center() + vec2(x, y) * ctx.scale;
    let k = ctx.scale * zoom;
    let apply_tint = |c: Color32| -> Color32 {
        let base = if let Some((tc, s)) = fx.tint_color {
            let inv = 1.0 - s;
            Color32::from_rgba_unmultiplied(
                (c.r() as f32 * inv + tc.r() as f32 * s) as u8,
                (c.g() as f32 * inv + tc.g() as f32 * s) as u8,
                (c.b() as f32 * inv + tc.b() as f32 * s) as u8,
                c.a(),
            )
        } else {
            c
        };
        base.gamma_multiply(alpha)
    };

    // ドロップシャドウの背面描画
    if let Some(sh_off) = fx.shadow_offset {
        let sh_center = center + sh_off * ctx.scale;
        let sh_col = Color32::from_black_alpha((fx.shadow_alpha * alpha * 180.0) as u8);
        ctx.painter.circle_filled(sh_center, 45.0 * k, sh_col);
    }

    // グロー (32bit .auf / 内蔵グロー) の背面ハロー描画
    if fx.glow_strength > 0.01 {
        let glow_alpha = (fx.glow_strength * alpha * 90.0).min(180.0) as u8;
        let glow_col = Color32::from_rgba_unmultiplied(255, 240, 180, glow_alpha);
        ctx.painter.circle_filled(center, 85.0 * k * (1.0 + fx.glow_strength * 0.35), glow_col);
    }

    // カスタムオブジェクト (.obj) 等で `obj.load("figure", ...)` や `obj.mes(...)` が実行された場合
    if let Some(prim) = loaded_prim {
        match prim {
            LuaLoadedPrimitive::Figure { fig_type, color, size, .. } => {
                let sz = (*size as f32 * k).max(1.0);
                let col = apply_tint(u32_to_color32(*color));
                if fx.border_width > 0.0 {
                    let bcol = fx.border_color.unwrap_or(Color32::WHITE).gamma_multiply(alpha);
                    ctx.painter.circle_filled(center, sz / 2.0 + fx.border_width * ctx.scale, bcol);
                }
                match fig_type.as_str() {
                    "四角形" => {
                        fan(
                            ctx.painter,
                            center,
                            &polygon(center, sz * std::f32::consts::FRAC_1_SQRT_2, 4, angle + PI / 4.0),
                            col,
                        );
                    }
                    "三角形" => {
                        fan(ctx.painter, center, &polygon(center, sz / 2.0, 3, angle), col);
                    }
                    "五角形" => {
                        fan(ctx.painter, center, &polygon(center, sz / 2.0, 5, angle), col);
                    }
                    "六角形" => {
                        fan(ctx.painter, center, &polygon(center, sz / 2.0, 6, angle), col);
                    }
                    "星型" => {
                        fan(ctx.painter, center, &star(center, sz / 2.0, angle), col);
                    }
                    _ => {
                        ctx.painter.circle_filled(center, sz / 2.0, col);
                    }
                }
                return Some(Rect::from_center_size(center, vec2(sz, sz)));
            }
            LuaLoadedPrimitive::Text { text, size, color, .. } => {
                let sz = (*size as f32 * k).max(1.0);
                let col = apply_tint(u32_to_color32(*color));
                let galley = ctx.painter.layout(text.clone(), FontId::proportional(sz), col, f32::INFINITY);
                let half = galley.size() / 2.0;
                let pos = center - Rot2::from_angle(angle) * half;
                let r = Rect::from_center_size(center, galley.size());
                ctx.painter.add(egui::epaint::TextShape::new(pos, galley, col).with_angle(angle));
                return Some(r);
            }
            LuaLoadedPrimitive::Image { .. } => {}
        }
    }

    match src.kind {
        EffectKind::Shape => {
            let size = src.eval("サイズ", rel, 100.0) as f32 * k;
            let color = apply_tint(src.text("color").and_then(parse_color).unwrap_or(Color32::WHITE));
            let ty: u32 = src.text("type").and_then(|t| t.trim().parse().ok()).unwrap_or(1);
            if fx.border_width > 0.0 && ty != 0 {
                let bcol = fx.border_color.unwrap_or(Color32::WHITE).gamma_multiply(alpha);
                ctx.painter.circle_filled(center, size / 2.0 + fx.border_width * ctx.scale, bcol);
            }
            if fx.chroma_shift > 0.5 && ty != 0 {
                let shift_vec = vec2(fx.chroma_angle.cos(), fx.chroma_angle.sin()) * fx.chroma_shift * ctx.scale;
                ctx.painter.circle_filled(center - shift_vec, size / 2.0, Color32::from_rgba_unmultiplied(255, 40, 40, (alpha * 130.0) as u8));
                ctx.painter.circle_filled(center + shift_vec, size / 2.0, Color32::from_rgba_unmultiplied(40, 180, 255, (alpha * 130.0) as u8));
            }
            match ty {
                0 => {
                    ctx.painter.rect_filled(ctx.canvas, 0.0, color);
                    None
                }
                1 => {
                    ctx.painter.circle_filled(center, size / 2.0, color);
                    Some(Rect::from_center_size(center, vec2(size, size)))
                }
                2 => {
                    fan(ctx.painter, center, &polygon(center, size * std::f32::consts::FRAC_1_SQRT_2, 4, angle + PI / 4.0), color);
                    Some(Rect::from_center_size(center, vec2(size, size)))
                }
                6 => {
                    fan(ctx.painter, center, &star(center, size / 2.0, angle), color);
                    Some(Rect::from_center_size(center, vec2(size, size)))
                }
                n => {
                    let sides = (n as usize).clamp(3, 12);
                    fan(ctx.painter, center, &polygon(center, size / 2.0, sides, angle), color);
                    Some(Rect::from_center_size(center, vec2(size, size)))
                }
            }
        }
        EffectKind::Text => {
            let size = (src.eval("サイズ", rel, 34.0) as f32 * k).max(1.0);
            let color = apply_tint(src.text("color").and_then(parse_color).unwrap_or(Color32::WHITE));
            let text = src.text("text").unwrap_or_default().replace("\r\n", "\n");
            if fx.chroma_shift > 0.5 {
                let shift_vec = vec2(fx.chroma_angle.cos(), fx.chroma_angle.sin()) * fx.chroma_shift * ctx.scale;
                let r_col = Color32::from_rgba_unmultiplied(255, 50, 50, (alpha * 150.0) as u8);
                let b_col = Color32::from_rgba_unmultiplied(50, 180, 255, (alpha * 150.0) as u8);
                let g1 = ctx.painter.layout(text.clone(), FontId::proportional(size), r_col, f32::INFINITY);
                let g2 = ctx.painter.layout(text.clone(), FontId::proportional(size), b_col, f32::INFINITY);
                let half = g1.size() / 2.0;
                let pos = center - Rot2::from_angle(angle) * half;
                ctx.painter.add(egui::epaint::TextShape::new(pos - shift_vec, g1, r_col).with_angle(angle));
                ctx.painter.add(egui::epaint::TextShape::new(pos + shift_vec, g2, b_col).with_angle(angle));
            }
            let galley = ctx.painter.layout(text, FontId::proportional(size), color, f32::INFINITY);
            let half = galley.size() / 2.0;
            let pos = center - Rot2::from_angle(angle) * half;
            let r = Rect::from_center_size(center, galley.size());
            ctx.painter.add(egui::epaint::TextShape::new(pos, galley, color).with_angle(angle));
            Some(r)
        }
        EffectKind::ImageFile | EffectKind::VideoFile => {
            if let Some(&(tex_id, [tw, th])) = ctx.textures.get(&obj.id) {
                let size = vec2(tw as f32 * k, th as f32 * k);
                let r = Rect::from_center_size(center, size);
                if fx.chroma_shift > 0.5 {
                    let shift_vec = vec2(fx.chroma_angle.cos(), fx.chroma_angle.sin()) * fx.chroma_shift * ctx.scale;
                    draw_textured_quad(
                        ctx.painter,
                        tex_id,
                        r.translate(-shift_vec),
                        angle,
                        Color32::from_rgba_unmultiplied(255, 60, 60, (alpha * 120.0) as u8),
                    );
                    draw_textured_quad(
                        ctx.painter,
                        tex_id,
                        r.translate(shift_vec),
                        angle,
                        Color32::from_rgba_unmultiplied(60, 180, 255, (alpha * 120.0) as u8),
                    );
                }
                draw_textured_quad(ctx.painter, tex_id, r, angle, apply_tint(Color32::WHITE));
                Some(r)
            } else {
                let size = vec2(ctx.canvas.width(), ctx.canvas.height()) * zoom;
                let r = Rect::from_center_size(center, size);
                let corners = rotated_rect(r, angle);
                fan(ctx.painter, center, &corners, apply_tint(Color32::from_rgb(60, 70, 90)));
                ctx.painter.text(
                    center,
                    Align2::CENTER_CENTER,
                    format!("{}\n{}", src.name, obj.display_name()),
                    FontId::proportional((14.0 * zoom.max(0.5)).min(28.0)),
                    apply_tint(Color32::LIGHT_GRAY),
                );
                Some(r)
            }
        }
        _ => {
            let size = vec2(ctx.canvas.width(), ctx.canvas.height()) * zoom;
            let r = Rect::from_center_size(center, size);
            let corners = rotated_rect(r, angle);
            fan(ctx.painter, center, &corners, apply_tint(Color32::from_rgb(60, 70, 90)));
            ctx.painter.text(
                center,
                Align2::CENTER_CENTER,
                format!("{}\n{}", src.name, obj.display_name()),
                FontId::proportional((14.0 * zoom.max(0.5)).min(28.0)),
                apply_tint(Color32::LIGHT_GRAY),
            );
            Some(r)
        }
    }
}

/// 回転・透明度付きのテクスチャ矩形を描画します。
fn draw_textured_quad(painter: &egui::Painter, tex_id: egui::TextureId, rect: Rect, angle: f32, tint: Color32) {
    let corners = rotated_rect(rect, angle);
    let uvs = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
    let mut mesh = Mesh::with_texture(tex_id);
    for (p, uv) in corners.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex { pos: p, uv, color: tint });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
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
    } else if src.name.contains("グロー") || src.name.contains("発光") {
        let strength = (src.eval("強さ", rel, 60.0) / 200.0).clamp(0.0, 1.0) as f32;
        let overlay = Color32::from_rgba_unmultiplied(255, 245, 200, (strength * 95.0) as u8);
        ctx.painter.rect_filled(ctx.canvas, 0.0, overlay);
    } else if src.name.contains("色収差") || src.name.contains("色ズレ") {
        let shift = (src.eval("ズレ幅", rel, 8.0) as f32 * ctx.scale).max(1.0);
        let r_rect = Rect::from_min_max(ctx.canvas.left_top(), pos2(ctx.canvas.left() + shift * 2.0, ctx.canvas.bottom()));
        let b_rect = Rect::from_min_max(pos2(ctx.canvas.right() - shift * 2.0, ctx.canvas.top()), ctx.canvas.right_bottom());
        ctx.painter.rect_filled(r_rect, 0.0, Color32::from_rgba_unmultiplied(255, 40, 40, 90));
        ctx.painter.rect_filled(b_rect, 0.0, Color32::from_rgba_unmultiplied(40, 180, 255, 90));
    } else if src.name == "シーンチェンジ" {
        let progress = (rel as f32 / (obj.length().max(1) as f32)).clamp(0.0, 1.0);
        let overlay = Color32::from_black_alpha((progress * 220.0) as u8);
        ctx.painter.rect_filled(ctx.canvas, 0.0, overlay);
    }
}

fn polygon(center: Pos2, radius: f32, sides: usize, angle: f32) -> Vec<Pos2> {
    (0..sides)
        .map(|i| {
            let a = angle - PI / 2.0 + TAU * i as f32 / sides as f32;
            center + vec2(a.cos(), a.sin()) * radius
        })
        .collect()
}

fn star(center: Pos2, radius: f32, angle: f32) -> Vec<Pos2> {
    (0..10)
        .map(|i| {
            let r = if i % 2 == 0 { radius } else { radius * 0.4 };
            let a = angle - PI / 2.0 + TAU * i as f32 / 10.0;
            center + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}

fn rotated_rect(r: Rect, angle: f32) -> Vec<Pos2> {
    let rot = Rot2::from_angle(angle);
    let c = r.center();
    [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]
        .into_iter()
        .map(|p| c + rot * (p - c))
        .collect()
}

/// 中心から見て星形（凹形含む）の多角形を三角形ファンで塗りつぶします。
fn fan(painter: &egui::Painter, center: Pos2, pts: &[Pos2], color: Color32) {
    if pts.len() < 3 {
        return;
    }
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, color);
    for &p in pts {
        mesh.colored_vertex(p, color);
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(Shape::mesh(mesh));
}
