//! プレビュー画面。
//!
//! - 図形・テキスト・画像ファイル・動画ファイル・フィルタオブジェクトを合成描画します。
//! - 画像ファイル（`.png`, `.jpg`, `.bmp`, `.webp`, `.gif`）および動画ファイル（`.mp4`, `.avi`, `.mov`, `.mkv` 等）は
//!   [`au2_media::MediaCache`] を通じてデコードされ、GPU テクスチャとして回転・拡大率・透明度付きで描画されます。

use std::f32::consts::{PI, TAU};

use au2_core::{EffectKind, Frame, ObjectCategory, ObjectId, TimelineObject};
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
        textures: &resolved_textures,
    };
    for (li, obj) in st.scene.objects_at(frame) {
        if !st.scene.layers[li].visible {
            continue;
        }
        draw_object(&ctx, obj, frame - obj.frame_start, st.selected == Some(obj.id));
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

                // 左側：タイムコードバッジ [TC] + タイムコード (ベースライン・高さを完璧に統一)
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

                // 中央：再生・コマ送りボタン群（すべて高さ 24px で統一）
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

                // 再生 / 一時停止ボタン（アクティブ時は鮮やかなブルーで強調、高さ 24px）
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

                // 右側：解像度とフレームレート情報
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
    textures: &'a std::collections::HashMap<ObjectId, (egui::TextureId, [usize; 2])>,
}

fn draw_object(ctx: &DrawCtx, obj: &TimelineObject, rel: Frame, selected: bool) {
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

    // 標準描画パラメータ
    let draw = obj.effect(&EffectKind::StandardDraw);
    let ev = |key: &str, default: f64| draw.map(|d| d.eval(key, rel, default)).unwrap_or(default);
    let (mut x, mut y) = (ev("X", 0.0) as f32, ev("Y", 0.0) as f32);
    let zoom = (ev("拡大率", 100.0) / 100.0) as f32;
    let mut angle = (ev("回転", 0.0) as f32).to_radians();
    let mut alpha = (1.0 - ev("透明度", 0.0) / 100.0).clamp(0.0, 1.0) as f32;

    let time_sec = rel as f64 / ctx.fps.max(1.0);

    // フィルタ効果・アニメーション効果スクリプトの評価
    for eff in &obj.effects {
        if !eff.enabled {
            continue;
        }
        if eff.name == "フェード" {
            let fin = eff.eval("イン", rel, 0.0) * ctx.fps;
            let fout = eff.eval("アウト", rel, 0.0) * ctx.fps;
            let remain = (obj.length() - 1 - rel) as f64;
            if fin > 0.0 {
                alpha *= (rel as f64 / fin).min(1.0) as f32;
            }
            if fout > 0.0 {
                alpha *= (remain / fout).min(1.0) as f32;
            }
        } else if eff.name == "アニメーション効果" {
            let script_name = eff.text("name").unwrap_or_default();
            let sec = script_name.split('@').next().unwrap_or(script_name);
            match sec {
                "揺れ" | "振動" => {
                    let ax = eff.eval("X振幅", rel, 25.0) as f32;
                    let ay = eff.eval("Y振幅", rel, 15.0) as f32;
                    let period = eff.eval("周期", rel, 0.4).max(0.01) as f32;
                    let phase = (time_sec as f32 / period) * TAU;
                    x += phase.sin() * ax;
                    y += (phase * 1.3).cos() * ay;
                }
                "点滅" => {
                    let period = eff.eval("周期", rel, 0.5).max(0.01) as f32;
                    let min_t = (eff.eval("最小透明度", rel, 60.0) / 100.0).clamp(0.0, 1.0) as f32;
                    let w = (((time_sec as f32 / period) * TAU).sin() + 1.0) * 0.5;
                    alpha *= 1.0 - min_t * w;
                }
                "回転" => {
                    let deg_per_sec = eff.eval("角速度", rel, 90.0) as f32;
                    angle += (time_sec as f32 * deg_per_sec).to_radians();
                }
                _ => {}
            }
        }
    }

    if alpha <= 0.0 {
        return;
    }

    let center = ctx.canvas.center() + vec2(x, y) * ctx.scale;
    let k = ctx.scale * zoom;
    let with_alpha = |c: Color32| c.gamma_multiply(alpha);

    let bounds = match src.kind {
        EffectKind::Shape => {
            let size = src.eval("サイズ", rel, 100.0) as f32 * k;
            let color = with_alpha(src.text("color").and_then(parse_color).unwrap_or(Color32::WHITE));
            let ty: u32 = src.text("type").and_then(|t| t.trim().parse().ok()).unwrap_or(1);
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
            let color = with_alpha(src.text("color").and_then(parse_color).unwrap_or(Color32::WHITE));
            let text = src.text("text").unwrap_or_default().replace("\r\n", "\n");
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
                draw_textured_quad(ctx.painter, tex_id, r, angle, with_alpha(Color32::WHITE));
                Some(r)
            } else {
                // ファイル未指定または存在しない場合はプレースホルダを描画
                let size = vec2(ctx.canvas.width(), ctx.canvas.height()) * zoom;
                let r = Rect::from_center_size(center, size);
                let corners = rotated_rect(r, angle);
                fan(ctx.painter, center, &corners, with_alpha(Color32::from_rgb(60, 70, 90)));
                ctx.painter.text(
                    center,
                    Align2::CENTER_CENTER,
                    format!("{}\n{}", src.name, obj.display_name()),
                    FontId::proportional((14.0 * zoom.max(0.5)).min(28.0)),
                    with_alpha(Color32::LIGHT_GRAY),
                );
                Some(r)
            }
        }
        _ => {
            let size = vec2(ctx.canvas.width(), ctx.canvas.height()) * zoom;
            let r = Rect::from_center_size(center, size);
            let corners = rotated_rect(r, angle);
            fan(ctx.painter, center, &corners, with_alpha(Color32::from_rgb(60, 70, 90)));
            ctx.painter.text(
                center,
                Align2::CENTER_CENTER,
                format!("{}\n{}", src.name, obj.display_name()),
                FontId::proportional((14.0 * zoom.max(0.5)).min(28.0)),
                with_alpha(Color32::LIGHT_GRAY),
            );
            Some(r)
        }
    };

    if selected {
        if let Some(b) = bounds {
            ctx.painter.rect_stroke(b.expand(2.0), 0.0, Stroke::new(1.0_f32, Color32::from_rgb(255, 200, 0)));
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

/// フィルタオブジェクトの簡易プレビュー（下位レイヤー全体に掛かる）。
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
