//! 単一インスタンス（図形・テキスト・画像・動画・音声波形・パーティクル等）の描画処理。

use std::f32::consts::{PI, TAU};

use au2_core::{EffectKind, Frame, TimelineObject};
use au2_plugin::LuaLoadedPrimitive;
use eframe::egui::{self, emath::Rot2, pos2, vec2, Align2, Color32, FontId, Mesh, Pos2, Rect, Shape, Stroke};

use crate::panels::parse_color;
use super::super::camera::{aviutl_rand, project_3d_points, rotate_3d};
use super::super::filters::FilterVisuals;
use super::super::text::{layout_aviutl_text, u32_to_color32, TextCharLayout};
use super::primitives::{closed_stroke, draw_shape_mesh, draw_textured_quad_uv, fan, rotated_rect};
use super::{render_scene_at, DrawCtx};

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_single_instance(
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
    transform_3d: Option<([f64; 3], [f64; 3], f32)>,
    single_text_char: Option<&TextCharLayout>,
    selected: bool,
) -> Option<Rect> {
    let center = ctx.canvas.center() + vec2(x, y) * ctx.scale;
    let k = ctx.scale * zoom;

    // クリッピング（上・下・左・右）が設定されている場合は描画ペインターのクリップ矩形を絞り込む
    let has_clip = fx.clip_insets.iter().any(|&v| v > 0.1);
    let clipped_painter = if has_clip {
        let approx_r = (src.eval("サイズ", rel, 200.0) as f32 * k * 0.5).max(50.0 * k);
        let clip_rect = Rect::from_min_max(
            pos2(
                center.x - (approx_r - fx.clip_insets[2] * k).max(0.0),
                center.y - (approx_r - fx.clip_insets[0] * k).max(0.0),
            ),
            pos2(
                center.x + (approx_r - fx.clip_insets[3] * k).max(0.0),
                center.y + (approx_r - fx.clip_insets[1] * k).max(0.0),
            ),
        );
        Some(ctx.painter.with_clip_rect(clip_rect.intersect(ctx.canvas)))
    } else {
        None
    };
    let painter = clipped_painter.as_ref().unwrap_or(ctx.painter);

    let apply_tint = |c: Color32| -> Color32 {
        let mut r = c.r() as f32;
        let mut g = c.g() as f32;
        let mut b = c.b() as f32;

        // 輝度反転 / 色相反転
        if fx.inversions.2 || fx.inversions.3 {
            r = 255.0 - r;
            g = 255.0 - g;
            b = 255.0 - b;
        }

        // 色調補正（明るさ・彩度）
        if fx.hsv_shift.0.abs() > 0.01 {
            let delta = fx.hsv_shift.0 * 255.0;
            r = (r + delta).clamp(0.0, 255.0);
            g = (g + delta).clamp(0.0, 255.0);
            b = (b + delta).clamp(0.0, 255.0);
        }
        if fx.hsv_shift.2 > 0.0 && (fx.hsv_shift.2 - 1.0).abs() > 0.01 {
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;
            let sat = fx.hsv_shift.2;
            r = (luma + (r - luma) * sat).clamp(0.0, 255.0);
            g = (luma + (g - luma) * sat).clamp(0.0, 255.0);
            b = (luma + (b - luma) * sat).clamp(0.0, 255.0);
        }

        // 単色化
        if let Some((tc, s)) = fx.tint_color {
            let inv = 1.0 - s;
            r = r * inv + tc.r() as f32 * s;
            g = g * inv + tc.g() as f32 * s;
            b = b * inv + tc.b() as f32 * s;
        }

        // グラデーション
        if let Some((gc, s)) = fx.grad_color {
            let inv = 1.0 - s;
            r = r * inv + gc.r() as f32 * s;
            g = g * inv + gc.g() as f32 * s;
            b = b * inv + gc.b() as f32 * s;
        }

        // グローまたは加算合成（1, 11, 12）時の発光ハイライト強調
        let is_glow_or_add = fx.glow_strength > 0.01 || fx.blend_mode == 1 || fx.blend_mode == 11 || fx.blend_mode == 12;
        if is_glow_or_add {
            let boost = (fx.glow_strength * 0.22 + if fx.blend_mode == 1 { 0.35 } else { 0.15 }).min(0.68);
            r = (r * (1.0 - boost) + 255.0 * boost).min(255.0);
            g = (g * (1.0 - boost) + 255.0 * boost).min(255.0);
            b = (b * (1.0 - boost) + 255.0 * boost).min(255.0);
        }

        let eff_alpha = if fx.inversions.4 { (1.0 - alpha).clamp(0.0, 1.0) } else { alpha };
        Color32::from_rgba_unmultiplied(r as u8, g as u8, b as u8, c.a()).gamma_multiply(eff_alpha)
    };

    // ドロップシャドウの背面描画（テキストはグリフ単位の正確なシャドーを描画するため除外）
    if !matches!(src.kind, EffectKind::Text) {
        if let Some(sh_off) = fx.shadow_offset {
            let sh_center = center + sh_off * ctx.scale;
            let sh_col = Color32::from_black_alpha((fx.shadow_alpha * alpha * 180.0) as u8);
            painter.circle_filled(sh_center, 45.0 * k, sh_col);
        }
    }

    // カスタムオブジェクト (.obj) 等で `obj.load("figure", ...)` や `obj.mes(...)` が実行された場合
    if let Some(prim) = loaded_prim {
        match prim {
            LuaLoadedPrimitive::Figure { fig_type, color, size, line_width } => {
                let raw_sz = *size as f32;
                let sz = (raw_sz * k).max(1.0);
                let col = apply_tint(u32_to_color32(*color));
                let is_stroke = *line_width > 0.0 && (*line_width as f32) < raw_sz * 0.5 && *line_width < 3999.0;
                let inner_ratio = if is_stroke {
                    ((raw_sz - (*line_width as f32) * 2.0).max(0.0) / raw_sz.max(1.0)).max(fx.inner_hollow_ratio)
                } else {
                    fx.inner_hollow_ratio
                };
                let (w_mult, h_mult) = if fx.aspect_override.abs() > 0.001 {
                    let a = fx.aspect_override.clamp(-0.98, 0.98);
                    if a >= 0.0 {
                        (1.0 - a, 1.0)
                    } else {
                        (1.0, 1.0 + a)
                    }
                } else {
                    (1.0, 1.0)
                };

                let local_pts: Vec<[f32; 2]> = match fig_type.as_str() {
                    "四角形" => {
                        let hw = raw_sz * 0.5 * w_mult;
                        let hh = raw_sz * 0.5 * h_mult;
                        vec![[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]]
                    }
                    "三角形" => (0..3).map(|i| {
                        let a = -PI / 2.0 + TAU * (i as f32) / 3.0;
                        [a.cos() * raw_sz * 0.5 * w_mult, a.sin() * raw_sz * 0.5 * h_mult]
                    }).collect(),
                    "五角形" => (0..5).map(|i| {
                        let a = -PI / 2.0 + TAU * (i as f32) / 5.0;
                        [a.cos() * raw_sz * 0.5 * w_mult, a.sin() * raw_sz * 0.5 * h_mult]
                    }).collect(),
                    "六角形" => (0..6).map(|i| {
                        let a = -PI / 2.0 + TAU * (i as f32) / 6.0;
                        [a.cos() * raw_sz * 0.5 * w_mult, a.sin() * raw_sz * 0.5 * h_mult]
                    }).collect(),
                    "星型" => (0..10).map(|i| {
                        let r = if i % 2 == 0 { raw_sz * 0.5 } else { raw_sz * 0.2 };
                        let a = -PI / 2.0 + TAU * (i as f32) / 10.0;
                        [a.cos() * r * w_mult, a.sin() * r * h_mult]
                    }).collect(),
                    _ => (0..24).map(|i| {
                        let a = TAU * (i as f32) / 24.0;
                        [a.cos() * raw_sz * 0.5 * w_mult, a.sin() * raw_sz * 0.5 * h_mult]
                    }).collect(),
                };

                let outer_screen = if let (Some(cam), Some((w_pos, rot, r_zoom))) = (ctx.camera, transform_3d) {
                    project_3d_points(&cam, ctx.canvas, ctx.scale, w_pos, rot, &local_pts, r_zoom)
                } else {
                    let rot = Rot2::from_angle(angle);
                    Some(local_pts.iter().map(|&[lx, ly]| center + rot * vec2(lx * k, ly * k)).collect())
                };
                let Some(outer_pts) = outer_screen else { return None };

                let inner_screen = if inner_ratio > 0.01 {
                    let inner_local: Vec<[f32; 2]> = local_pts.iter().map(|&[lx, ly]| [lx * inner_ratio, ly * inner_ratio]).collect();
                    if let (Some(cam), Some((w_pos, rot, r_zoom))) = (ctx.camera, transform_3d) {
                        project_3d_points(&cam, ctx.canvas, ctx.scale, w_pos, rot, &inner_local, r_zoom)
                    } else {
                        let rot = Rot2::from_angle(angle);
                        Some(inner_local.iter().map(|&[lx, ly]| center + rot * vec2(lx * k, ly * k)).collect())
                    }
                } else {
                    None
                };

                draw_shape_mesh(painter, center, &outer_pts, inner_screen.as_deref(), col, fx, ctx.scale, alpha);
                return Some(Rect::from_center_size(center, vec2(sz, sz)));
            }
            LuaLoadedPrimitive::Text { text, size, color, .. } => {
                let sz = (*size as f32 * k).clamp(1.0, 96.0);
                let col = apply_tint(u32_to_color32(*color));
                let galley = painter.layout(text.clone(), FontId::proportional(sz), col, f32::INFINITY);
                let half = galley.size() / 2.0;
                let pos = center - Rot2::from_angle(angle) * half;
                let r = Rect::from_center_size(center, galley.size());
                painter.add(egui::epaint::TextShape::new(pos, galley, col).with_angle(angle));
                return Some(r);
            }
            LuaLoadedPrimitive::Image { .. } => {}
        }
    }

    match src.kind {
        EffectKind::Shape => {
            let raw_size = src.eval("サイズ", rel, 100.0) as f32;
            let size = (raw_size * k).max(1.0);
            let line_width = src.eval("ライン幅", rel, 4000.0) as f32;
            let aspect = src.eval("縦横比", rel, 0.0) as f32 / 100.0;
            let (w_mult, h_mult) = if aspect >= 0.0 { (1.0 - aspect, 1.0) } else { (1.0, 1.0 + aspect) };
            let color = apply_tint(src.text("color").and_then(parse_color).unwrap_or(Color32::WHITE));
            let ty: u32 = src.text("type").and_then(|t| t.trim().parse().ok()).unwrap_or(1);

            // 極座標変換フィルタが適用されている場合: 直線や四角形を円環（リング・渦巻）に変換
            let effective_ty = if fx.polar_transform.is_some() && ty != 0 { 1 } else { ty };
            let polar_inner = fx.polar_transform.map(|(cw, zm, _, _)| {
                if cw > 1.0 {
                    (cw / (raw_size * zm + cw).max(1.0)).clamp(0.15, 0.92)
                } else {
                    0.55
                }
            }).unwrap_or(0.0);

            // AviUtl の図形: ライン幅 < サイズ/2 の場合は枠線（中抜き）図形、さらにワイプや反転マスク・極座標変換による中抜き比率も統合
            let is_stroke = effective_ty != 0 && line_width > 0.0 && line_width < raw_size * 0.5 && line_width < 3999.0;
            let stroke_inner_ratio = if is_stroke {
                ((raw_size - line_width * 2.0).max(0.0) / raw_size.max(1.0)).clamp(0.02, 0.98)
            } else {
                0.0
            };
            let inner_ratio = stroke_inner_ratio.max(fx.inner_hollow_ratio).max(polar_inner);

            if fx.border_width > 0.0 && effective_ty != 0 {
                let bcol = fx.border_color.unwrap_or(Color32::WHITE).gamma_multiply(alpha);
                painter.circle_stroke(
                    center,
                    size * 0.5 + fx.border_width * ctx.scale * 0.5,
                    Stroke::new((fx.border_width * ctx.scale).max(1.0), bcol),
                );
            }

            // ローカル頂点列を生成（縦横比を反映）
            let local_pts: Vec<[f32; 2]> = match effective_ty {
                0 => {
                    // 背景
                    let hw = ctx.screen_w as f32 / 2.0;
                    let hh = ctx.screen_h as f32 / 2.0;
                    vec![[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]]
                }
                1 => {
                    // 円 (32分割の多角形近似で3D透視投影・中抜きリング・扇クリッピングRにも滑らかに対応)
                    let rx = (raw_size * w_mult) / 2.0;
                    let ry = (raw_size * h_mult) / 2.0;
                    if let Some((start_deg, end_deg)) = fx.sector_clip {
                        let span = (end_deg - start_deg).clamp(-360.0, 360.0);
                        if span.abs() < 0.5 {
                            return None;
                        }
                        if span.abs() < 359.5 {
                            let s_rad = (start_deg - 90.0).to_radians();
                            let e_rad = (end_deg - 90.0).to_radians();
                            let steps = 28usize;
                            let r_in_ratio = inner_ratio;
                            let mut outer_arc = Vec::with_capacity(steps + 1);
                            let mut inner_arc = Vec::with_capacity(steps + 1);
                            for i in 0..=steps {
                                let t = i as f32 / steps as f32;
                                let a = s_rad + (e_rad - s_rad) * t;
                                outer_arc.push([a.cos() * rx, a.sin() * ry]);
                                inner_arc.push([a.cos() * rx * r_in_ratio, a.sin() * ry * r_in_ratio]);
                            }
                            let proj_pts = |pts_2d: &[[f32; 2]]| -> Option<Vec<Pos2>> {
                                if let (Some(cam), Some((w_pos, rot, r_zoom))) = (ctx.camera, transform_3d) {
                                    project_3d_points(&cam, ctx.canvas, ctx.scale, w_pos, rot, pts_2d, r_zoom)
                                } else {
                                    let rot = Rot2::from_angle(angle);
                                    Some(pts_2d.iter().map(|&[lx, ly]| center + rot * vec2(lx * k, ly * k)).collect())
                                }
                            };
                            if let (Some(out_s), Some(in_s)) = (proj_pts(&outer_arc), proj_pts(&inner_arc)) {
                                let mut mesh = Mesh::default();
                                let n_v = out_s.len();
                                for i in 0..n_v {
                                    mesh.colored_vertex(out_s[i], color);
                                    mesh.colored_vertex(in_s[i], color);
                                }
                                for i in 0..(n_v - 1) {
                                    let o0 = (i * 2) as u32;
                                    let i0 = (i * 2 + 1) as u32;
                                    let o1 = ((i + 1) * 2) as u32;
                                    let i1 = ((i + 1) * 2 + 1) as u32;
                                    mesh.add_triangle(o0, o1, i0);
                                    mesh.add_triangle(i0, o1, i1);
                                }
                                painter.add(Shape::mesh(mesh));
                                return Some(Rect::from_points(&out_s));
                            }
                            return None;
                        }
                    }
                    (0..32).map(|i| {
                        let a = TAU * (i as f32) / 32.0;
                        [a.cos() * rx, a.sin() * ry]
                    }).collect()
                }
                2 => {
                    // 四角形
                    let hw = (raw_size * w_mult) / 2.0;
                    let hh = (raw_size * h_mult) / 2.0;
                    vec![[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]]
                }
                3 => {
                    // 三角形
                    let r = raw_size / 2.0;
                    (0..3).map(|i| {
                        let a = -PI / 2.0 + TAU * (i as f32) / 3.0;
                        [a.cos() * r * w_mult, a.sin() * r * h_mult]
                    }).collect()
                }
                4 => {
                    // 五角形
                    let r = raw_size / 2.0;
                    (0..5).map(|i| {
                        let a = -PI / 2.0 + TAU * (i as f32) / 5.0;
                        [a.cos() * r * w_mult, a.sin() * r * h_mult]
                    }).collect()
                }
                5 => {
                    // 六角形
                    let r = raw_size / 2.0;
                    (0..6).map(|i| {
                        let a = -PI / 2.0 + TAU * (i as f32) / 6.0;
                        [a.cos() * r * w_mult, a.sin() * r * h_mult]
                    }).collect()
                }
                6 => {
                    // 星型
                    let r = raw_size / 2.0;
                    (0..10).map(|i| {
                        let rad = if i % 2 == 0 { r } else { r * 0.4 };
                        let a = -PI / 2.0 + TAU * (i as f32) / 10.0;
                        [a.cos() * rad * w_mult, a.sin() * rad * h_mult]
                    }).collect()
                }
                n => {
                    let sides = (n as usize).clamp(3, 24);
                    let r = raw_size / 2.0;
                    (0..sides).map(|i| {
                        let a = -PI / 2.0 + TAU * (i as f32) / sides as f32;
                        [a.cos() * r * w_mult, a.sin() * r * h_mult]
                    }).collect()
                }
            };

            // 外周頂点をスクリーン座標に変換（3Dカメラ投影 or 2D平面変換）
            let screen_pts: Option<Vec<Pos2>> = if let (Some(cam), Some((w_pos, rot, r_zoom))) = (ctx.camera, transform_3d) {
                project_3d_points(&cam, ctx.canvas, ctx.scale, w_pos, rot, &local_pts, r_zoom)
            } else {
                let rot = Rot2::from_angle(angle);
                Some(local_pts.iter().map(|&[lx, ly]| center + rot * vec2(lx * k, ly * k)).collect())
            };

            let Some(pts) = screen_pts else {
                return None;
            };

            // 内周頂点（枠線またはワイプ・反転マスクで中抜きされる場合）
            let inner_screen_pts: Option<Vec<Pos2>> = if ty != 0 && inner_ratio > 0.01 {
                let inner_local: Vec<[f32; 2]> = local_pts
                    .iter()
                    .map(|&[lx, ly]| [lx * inner_ratio, ly * inner_ratio])
                    .collect();
                if let (Some(cam), Some((w_pos, rot, r_zoom))) = (ctx.camera, transform_3d) {
                    project_3d_points(&cam, ctx.canvas, ctx.scale, w_pos, rot, &inner_local, r_zoom)
                } else {
                    let rot = Rot2::from_angle(angle);
                    Some(inner_local.iter().map(|&[lx, ly]| center + rot * vec2(lx * k, ly * k)).collect())
                }
            } else {
                None
            };

            // 色ズレ（色収差）
            if fx.chroma_shift > 0.5 && ty != 0 {
                let shift_vec = vec2(fx.chroma_angle.cos(), fx.chroma_angle.sin()) * fx.chroma_shift * ctx.scale;
                let r_pts: Vec<Pos2> = pts.iter().map(|&p| p - shift_vec).collect();
                let b_pts: Vec<Pos2> = pts.iter().map(|&p| p + shift_vec).collect();
                let r_in: Option<Vec<Pos2>> = inner_screen_pts.as_ref().map(|ip| ip.iter().map(|&p| p - shift_vec).collect());
                let b_in: Option<Vec<Pos2>> = inner_screen_pts.as_ref().map(|ip| ip.iter().map(|&p| p + shift_vec).collect());
                let mut no_glow_fx = *fx;
                no_glow_fx.glow_strength = 0.0;
                draw_shape_mesh(
                    painter,
                    center - shift_vec,
                    &r_pts,
                    r_in.as_deref(),
                    Color32::from_rgba_unmultiplied(255, 50, 50, (alpha * 130.0) as u8),
                    &no_glow_fx,
                    ctx.scale,
                    alpha,
                );
                draw_shape_mesh(
                    painter,
                    center + shift_vec,
                    &b_pts,
                    b_in.as_deref(),
                    Color32::from_rgba_unmultiplied(50, 180, 255, (alpha * 130.0) as u8),
                    &no_glow_fx,
                    ctx.scale,
                    alpha,
                );
            }

            draw_shape_mesh(
                painter,
                center,
                &pts,
                inner_screen_pts.as_deref(),
                color,
                fx,
                ctx.scale,
                alpha,
            );

            // ミラー反射
            if fx.mirror_alpha > 0.01 && ty != 0 {
                let mirror_offset = vec2(0.0, size * h_mult);
                let m_pts: Vec<Pos2> = pts.iter().map(|&p| pos2(p.x, center.y * 2.0 - p.y) + mirror_offset).collect();
                let m_in: Option<Vec<Pos2>> = inner_screen_pts
                    .as_ref()
                    .map(|ip| ip.iter().map(|&p| pos2(p.x, center.y * 2.0 - p.y) + mirror_offset).collect());
                let mut m_fx = *fx;
                m_fx.glow_strength *= 0.4;
                draw_shape_mesh(
                    painter,
                    center + mirror_offset,
                    &m_pts,
                    m_in.as_deref(),
                    color.gamma_multiply(fx.mirror_alpha * 0.5),
                    &m_fx,
                    ctx.scale,
                    alpha * fx.mirror_alpha * 0.5,
                );
            }

            Some(Rect::from_points(&pts))
        }
        EffectKind::Text => {
            // epaint のフォントアトラスは巨大フォント（>100pt）でアトラス境界違反・クラッシュを起こすため、
            // フォントのラスタライズサイズは最大 96.0 に抑え、それ以上の超巨大テキストはメッシュ頂点スケーリングで安全に描画する
            // 高解像度ディスプレイ・巨大テロップ・拡大時でもボケないよう、フォントラスタライズ上限を 512.0pt に拡大
            let max_font_size = 512.0_f32;
            let raw_canvas_size = src.eval("サイズ", rel, 34.0).max(1.0) as f32;
            let scaled_raw_size = (raw_canvas_size * k).max(1.0);
            let (font_size, scale_up) = if scaled_raw_size > max_font_size {
                (max_font_size, (scaled_raw_size / max_font_size).min(100.0))
            } else {
                (scaled_raw_size, 1.0)
            };

            let base_color = src.text("color").and_then(parse_color).unwrap_or(Color32::WHITE);
            let color2 = src.text("color2").and_then(parse_color).unwrap_or(Color32::BLACK);
            let font_family = crate::fonts::resolve_bound_font_family(painter.ctx(), src.text("font"));
            let font_id = FontId::new(font_size, font_family);

            let text_type = src
                .text("type")
                .and_then(|s| s.trim().parse::<u8>().ok())
                .unwrap_or(0);

            // 文字レイアウトの取得（個別オブジェクト時は単一文字、非分割時は全文レイアウト）
            let (char_items, mirror_bottom_y): (Vec<TextCharLayout>, f32) = if let Some(single) = single_text_char {
                (vec![single.clone()], raw_canvas_size * 0.5)
            } else {
                layout_aviutl_text(ctx, src, rel)
            };

            if char_items.is_empty() {
                return None;
            }

            // 3D回転（X軸回転・Y軸回転）が設定されているか判定
            let use_3d_mesh = if let (Some((_, rot_3d, _)), Some(_)) = (transform_3d, &ctx.camera) {
                rot_3d[0].abs() > 0.1 || rot_3d[1].abs() > 0.1
            } else {
                false
            };

            // ローカルキャンバス座標 `local_off` にある1つの Galley を、3D透視変換または2D回転スケーリングで描画するクロージャ
            let draw_galley_at = |p: &egui::Painter,
                                  local_off: egui::Vec2,
                                  galley: std::sync::Arc<egui::epaint::Galley>,
                                  col: Color32,
                                  flip_y_around: Option<f32>,
                                  char_scale_up: f32|
             -> Option<Rect> {
                let half = galley.size() * 0.5;
                if use_3d_mesh || char_scale_up > 1.001 || flip_y_around.is_some() {
                    let shape = egui::epaint::TextShape::new(Pos2::ZERO, galley.clone(), col);
                    let clipped = egui::epaint::ClippedShape {
                        clip_rect: egui::Rect::EVERYTHING,
                        shape: egui::Shape::Text(shape),
                    };
                    let primitives = p.ctx().tessellate(vec![clipped], p.ctx().pixels_per_point());
                    let inv_k = 1.0 / k.max(0.0001);
                    let rot2 = Rot2::from_angle(angle);
                    let mut bounds: Option<Rect> = None;
                    for prim in primitives {
                        if let egui::epaint::Primitive::Mesh(mut mesh) = prim.primitive {
                            let mut all_valid = true;
                            for v in &mut mesh.vertices {
                                v.color = col;
                                let gx = (v.pos.x - half.x) * char_scale_up * inv_k;
                                let gy = (v.pos.y - half.y) * char_scale_up * inv_k;
                                let lx = local_off.x + gx;
                                let mut ly = local_off.y + gy;
                                if let Some(my) = flip_y_around {
                                    ly = my * 2.0 - ly;
                                }
                                if use_3d_mesh {
                                    if let (Some((w_pos, rot_3d, r_zoom)), Some(ref cam)) = (transform_3d, &ctx.camera) {
                                        let p_rot = rotate_3d(
                                            [(lx * r_zoom) as f64, (ly * r_zoom) as f64, 0.0],
                                            rot_3d[0],
                                            rot_3d[1],
                                            rot_3d[2],
                                        );
                                        let pw = [
                                            w_pos[0] + p_rot[0],
                                            w_pos[1] + p_rot[1],
                                            w_pos[2] + p_rot[2],
                                        ];
                                        if let Some((sx, sy, _, _)) = cam.project(pw) {
                                            v.pos = ctx.canvas.center() + vec2(sx, sy) * ctx.scale;
                                        } else {
                                            all_valid = false;
                                            break;
                                        }
                                    }
                                } else {
                                    v.pos = center + rot2 * (vec2(lx, ly) * k);
                                }
                            }
                            if all_valid && !mesh.vertices.is_empty() {
                                let mut min_p = mesh.vertices[0].pos;
                                let mut max_p = mesh.vertices[0].pos;
                                for v in &mesh.vertices {
                                    min_p = min_p.min(v.pos);
                                    max_p = max_p.max(v.pos);
                                }
                                let r = Rect::from_min_max(min_p, max_p);
                                bounds = Some(bounds.map(|b| b.union(r)).unwrap_or(r));
                                p.add(egui::Shape::Mesh(mesh));
                            }
                        }
                    }
                    bounds
                } else {
                    let rot2 = Rot2::from_angle(angle);
                    let scaled_off: egui::Vec2 = local_off * k;
                    let target_center = center + (rot2 * scaled_off);
                    let unscaled_pos = target_center - (rot2 * half);
                    let r = Rect::from_center_size(target_center, galley.size());
                    p.add(egui::epaint::TextShape::new(unscaled_pos, galley, col).with_angle(angle));
                    Some(r)
                }
            };

            let mut total_bounds: Option<Rect> = None;
            let _ = (font_id, scale_up);

            for item in &char_items {
                if item.ch == " " || item.ch == "　" {
                    continue;
                }
                let item_off = vec2(item.local_x, item.local_y);
                let main_col = apply_tint(item.color_override.unwrap_or(base_color));
                let item_color2 = item.color2_override.unwrap_or(color2);
                let item_scale = item.size_scale.clamp(0.1, 20.0);
                let item_canvas_size = (raw_canvas_size * item_scale).max(1.0);
                let item_scaled_raw = (item_canvas_size * k).max(1.0);
                let (item_font_size, item_scale_up) = if item_scaled_raw > max_font_size {
                    (max_font_size, (item_scaled_raw / max_font_size).min(100.0))
                } else {
                    (item_scaled_raw, 1.0)
                };
                let item_family = if let Some(ref f_ov) = item.font_override {
                    crate::fonts::resolve_bound_font_family(painter.ctx(), Some(f_ov.as_str()))
                } else {
                    crate::fonts::resolve_bound_font_family(painter.ctx(), src.text("font"))
                };
                let item_font_id = FontId::new(item_font_size, item_family);
                let galley = painter.layout_no_wrap(item.ch.clone(), item_font_id, main_col);

                // 1. グロー効果 (`fx.glow_strength`)
                if fx.glow_strength > 0.01 {
                    let glow_alpha = (fx.glow_strength * alpha * 38.0).clamp(8.0, 140.0) as u8;
                    let gcol = fx
                        .glow_color
                        .unwrap_or(Color32::from_rgb(
                            main_col.r().saturating_add(50),
                            main_col.g().saturating_add(50),
                            main_col.b().saturating_add(50),
                        ));
                    let gc = Color32::from_rgba_unmultiplied(gcol.r(), gcol.g(), gcol.b(), glow_alpha);
                    let gr = (fx.glow_radius * 0.12).clamp(1.5, 8.0);
                    for (dx, dy) in [(-gr, 0.0), (gr, 0.0), (0.0, -gr), (0.0, gr)] {
                        let _ = draw_galley_at(painter, item_off + vec2(dx, dy), galley.clone(), gc.gamma_multiply(0.45), None, item_scale_up);
                    }
                }
                if fx.glow_only {
                    continue;
                }

                // 2. シャドーフィルタ (`fx.shadow_offset`)
                if let Some(sh_off) = fx.shadow_offset {
                    let sh_col = Color32::from_black_alpha((fx.shadow_alpha * alpha * 200.0) as u8);
                    let _ = draw_galley_at(painter, item_off + sh_off, galley.clone(), sh_col, None, item_scale_up);
                }

                // 3. 文字装飾 (type = 1..4: 影付き / 縁取り) または 縁取りフィルタ (fx.border_width)
                if text_type == 1 || text_type == 2 {
                    // 多層ソフトドロップシャドウ（自然で上品なテロップ影）
                    let sh_a = if text_type == 1 { 0.85 } else { 0.45 };
                    let d = (item_canvas_size * 0.075).clamp(2.0, 64.0);
                    let base_sc = item_color2.gamma_multiply(alpha * sh_a);
                    let _ = draw_galley_at(painter, item_off + vec2(d, d), galley.clone(), base_sc, None, item_scale_up);

                    // 微細ブラー層（ソフトエッジ）
                    let blur_sc = item_color2.gamma_multiply(alpha * sh_a * 0.35);
                    let blur_r = (d * 0.45).clamp(1.0, 24.0);
                    for (bx, by) in [(-blur_r, 0.0), (blur_r, 0.0), (0.0, -blur_r), (0.0, blur_r)] {
                        let _ = draw_galley_at(painter, item_off + vec2(d + bx, d + by), galley.clone(), blur_sc, None, item_scale_up);
                    }
                } else if text_type == 3 || text_type == 4 || fx.border_width > 0.5 {
                    // 高品質・角丸円周サンプリング縁取り（16〜24方向＋内側補間）
                    let bw = if fx.border_width > 0.5 {
                        fx.border_width
                    } else if text_type == 3 {
                        (item_canvas_size * 0.075).clamp(2.0, 64.0)
                    } else {
                        (item_canvas_size * 0.040).clamp(1.2, 32.0)
                    };
                    let bc = if fx.border_width > 0.5 {
                        fx.border_color.unwrap_or(item_color2)
                    } else {
                        item_color2
                    }
                    .gamma_multiply(alpha);

                    // 太さに応じて 16 または 24 方向を全周サンプリング（滑らかな角丸縁取り）
                    let samples = if bw >= 12.0 { 32 } else if bw >= 3.5 { 24 } else { 16 };
                    let step_angle = std::f32::consts::TAU / (samples as f32);

                    // 太い縁取りの時は内側の隙間を埋めるため中間半径もサンプリング
                    if bw >= 8.0 {
                        let r1 = bw * 0.33;
                        let r2 = bw * 0.66;
                        for i in 0..samples {
                            let angle = i as f32 * step_angle;
                            let (sa, ca) = (angle.sin(), angle.cos());
                            let _ = draw_galley_at(painter, item_off + vec2(ca * r1, sa * r1), galley.clone(), bc, None, item_scale_up);
                            let _ = draw_galley_at(painter, item_off + vec2(ca * r2, sa * r2), galley.clone(), bc, None, item_scale_up);
                        }
                    } else if bw >= 2.5 {
                        let inner_r = bw * 0.55;
                        for i in 0..samples {
                            let angle = i as f32 * step_angle;
                            let dx = inner_r * angle.cos();
                            let dy = inner_r * angle.sin();
                            let _ = draw_galley_at(painter, item_off + vec2(dx, dy), galley.clone(), bc, None, item_scale_up);
                        }
                    }
                    // 外周サンプリング
                    for i in 0..samples {
                        let angle = i as f32 * step_angle;
                        let dx = bw * angle.cos();
                        let dy = bw * angle.sin();
                        let _ = draw_galley_at(painter, item_off + vec2(dx, dy), galley.clone(), bc, None, item_scale_up);
                    }
                }

                // 4. 色ズレ (`fx.chroma_shift`)
                if fx.chroma_shift > 0.5 {
                    let shift_canvas = vec2(fx.chroma_angle.cos(), fx.chroma_angle.sin()) * fx.chroma_shift;
                    let r_col = Color32::from_rgba_unmultiplied(255, 50, 50, (alpha * 150.0) as u8);
                    let b_col = Color32::from_rgba_unmultiplied(50, 180, 255, (alpha * 150.0) as u8);
                    let _ = draw_galley_at(painter, item_off - shift_canvas, galley.clone(), r_col, None, item_scale_up);
                    let _ = draw_galley_at(painter, item_off + shift_canvas, galley.clone(), b_col, None, item_scale_up);
                }

                // 5. 凸エッジ (`fx.bevel_width`)
                if fx.bevel_width > 0.5 {
                    let bd = (fx.bevel_width * 0.35).clamp(0.8, 3.5);
                    let hi_col = Color32::from_white_alpha((alpha * 110.0) as u8);
                    let lo_col = Color32::from_black_alpha((alpha * 140.0) as u8);
                    let _ = draw_galley_at(painter, item_off + vec2(bd, bd), galley.clone(), lo_col, None, item_scale_up);
                    let _ = draw_galley_at(painter, item_off - vec2(bd * 0.8, bd * 0.8), galley.clone(), hi_col, None, item_scale_up);
                }

                // 6. 本体文字描画
                if let Some(r) = draw_galley_at(painter, item_off, galley.clone(), main_col, None, item_scale_up) {
                    total_bounds = Some(total_bounds.map(|b| b.union(r)).unwrap_or(r));
                }

                // 7. ミラー反射 (`fx.mirror_alpha`)
                if fx.mirror_alpha > 0.01 {
                    let m_col = main_col.gamma_multiply(fx.mirror_alpha * 0.45);
                    let _ = draw_galley_at(painter, item_off, galley, m_col, Some(mirror_bottom_y), item_scale_up);
                }
            }

            total_bounds
        }
        EffectKind::Scene => {
            // サブシーンオブジェクト: Scene 1..49 を再帰的にレンダリング
            let scene_id = src
                .text("scene")
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or(1);
            if let Some(sub_scene) = ctx.scenes.get(&scene_id) {
                let start_pos = src.eval("再生位置", rel, 0.0).max(0.0) as Frame;
                let speed = src.eval("再生速度", rel, 100.0) / 100.0;
                let is_loop = src.eval("ループ再生", rel, src.eval("check0", rel, 0.0)) >= 0.5;
                let raw_sub_frame = start_pos + ((rel as f64) * speed.max(0.0)).round() as Frame;
                let sub_len = sub_scene
                    .layers
                    .iter()
                    .flat_map(|l| &l.objects)
                    .map(|o| o.frame_end)
                    .max()
                    .unwrap_or(1)
                    .max(1);
                let sub_frame = if is_loop { raw_sub_frame % sub_len } else { raw_sub_frame };

                let mut sub_gt = ctx.group_transform.unwrap_or_default();
                sub_gt.ox = x as f64;
                sub_gt.oy = y as f64;
                sub_gt.zoom = zoom as f64;
                sub_gt.rz = angle.to_degrees() as f64;
                sub_gt.alpha = alpha as f64;

                let sub_ctx = DrawCtx {
                    painter,
                    canvas: ctx.canvas,
                    scale: ctx.scale,
                    fps: ctx.fps,
                    screen_w: ctx.screen_w,
                    screen_h: ctx.screen_h,
                    textures: ctx.textures,
                    plugins: ctx.plugins,
                    scenes: ctx.scenes,
                    has_ffmpeg: ctx.has_ffmpeg,
                    camera: ctx.camera,
                    group_transform: Some(sub_gt),
                    scene_depth: ctx.scene_depth + 1,
                    audio_waveform: ctx.audio_waveform,
                    current_scene: Some((sub_scene, sub_frame)),
                    layer_limit: None,
                };
                render_scene_at(&sub_ctx, sub_scene, sub_frame, None);
            }
            Some(ctx.canvas)
        }
        EffectKind::ParticleOutput => {
            let freq = src.eval("出力頻度", rel, 10.0).clamp(1.0, 60.0) as usize;
            let speed = src.eval("出力速度", rel, 200.0) as f32;
            let col = apply_tint(src.text("color").and_then(parse_color).unwrap_or(Color32::WHITE));
            let time_s = rel as f32 / (ctx.fps.max(1.0) as f32);
            for i in 0..freq {
                let seed = (i as i32) * 13 + 7;
                let vx = aviutl_rand(-1.0, 1.0, seed, 1) as f32 * speed * 0.5;
                let vy = aviutl_rand(-1.0, 1.0, seed, 2) as f32 * speed * 0.5;
                let life = ((time_s + (i as f32) * 0.13) % 1.5) / 1.5;
                let p = center + vec2(vx * life, vy * life) * k;
                let p_col = col.gamma_multiply(1.0 - life);
                painter.circle_filled(p, (6.0 * k * (1.0 - life * 0.5)).max(1.5), p_col);
            }
            Some(Rect::from_center_size(center, vec2(160.0 * k, 160.0 * k)))
        }
        EffectKind::AudioWaveform => {
            let w = src.eval("横幅", rel, 240.0) as f32 * k;
            let h = src.eval("高さ", rel, 100.0) as f32 * k;
            let vol_scale = (src.eval("音量", rel, 100.0) / 100.0).clamp(0.0, 5.0) as f32;
            let wf_type = src
                .text("type")
                .and_then(|s| s.trim().parse::<u8>().ok())
                .unwrap_or(0);
            let col = apply_tint(src.text("color").and_then(parse_color).unwrap_or(Color32::WHITE));
            let bars = if !ctx.audio_waveform.is_empty() {
                ctx.audio_waveform.len()
            } else {
                32usize
            };
            let step_x = w / bars.max(1) as f32;
            let bar_w = step_x.max(1.5);
            let mut prev_pt: Option<Pos2> = None;
            for i in 0..bars {
                let norm_amp = if !ctx.audio_waveform.is_empty() {
                    (ctx.audio_waveform[i] * vol_scale).clamp(0.0, 1.0)
                } else {
                    let phase = (rel as f32 * 0.18) + (i as f32 * 0.35);
                    ((phase.sin() * 0.35 + (phase * 2.3).cos() * 0.25).abs() + 0.08).clamp(0.05, 0.9)
                        * vol_scale.min(1.5)
                };
                let amp = norm_amp * h * 0.5;
                let bx = center.x - w * 0.5 + (i as f32 + 0.5) * step_x;
                match wf_type {
                    1 | 3 => {
                        let sign = if i % 2 == 0 { -1.0 } else { 1.0 };
                        let pt = pos2(bx, center.y + sign * amp);
                        if let Some(p0) = prev_pt {
                            painter.line_segment([p0, pt], Stroke::new((1.8 * k).max(1.2), col));
                        }
                        prev_pt = Some(pt);
                    }
                    2 => {
                        let bottom_y = center.y + h * 0.5;
                        let top_y = (bottom_y - amp * 1.8).max(center.y - h * 0.5);
                        painter.line_segment(
                            [pos2(bx, bottom_y), pos2(bx, top_y)],
                            Stroke::new(bar_w * 0.72, col),
                        );
                    }
                    _ => {
                        painter.line_segment(
                            [pos2(bx, center.y - amp), pos2(bx, center.y + amp)],
                            Stroke::new(bar_w * 0.72, col),
                        );
                    }
                }
            }
            Some(Rect::from_center_size(center, vec2(w, h)))
        }
        EffectKind::ImageFile | EffectKind::VideoFile => {
            if let Some(&(tex_id, [tw, th])) = ctx.textures.get(&obj.id) {
                let size = vec2(tw as f32 * k, th as f32 * k);
                let r = Rect::from_center_size(center, size);
                let fh = fx.inversions.1;
                let fv = fx.inversions.0;
                if fx.chroma_shift > 0.5 {
                    let shift_vec = vec2(fx.chroma_angle.cos(), fx.chroma_angle.sin()) * fx.chroma_shift * ctx.scale;
                    draw_textured_quad_uv(
                        painter,
                        tex_id,
                        r.translate(-shift_vec),
                        angle,
                        Color32::from_rgba_unmultiplied(255, 60, 60, (alpha * 120.0) as u8),
                        fh,
                        fv,
                    );
                    draw_textured_quad_uv(
                        painter,
                        tex_id,
                        r.translate(shift_vec),
                        angle,
                        Color32::from_rgba_unmultiplied(60, 180, 255, (alpha * 120.0) as u8),
                        fh,
                        fv,
                    );
                }
                draw_textured_quad_uv(painter, tex_id, r, angle, apply_tint(Color32::WHITE), fh, fv);
                Some(r)
            } else {
                // 画像・動画ファイルがローカルに存在しない場合の軽量スマート描画（画面全体の塗り潰しを防止し、キャラクター立ち絵や映像配置を視認可能にする）
                let file_stem = src
                    .text("file")
                    .and_then(|p| std::path::Path::new(p).file_stem())
                    .and_then(|s| s.to_str())
                    .unwrap_or("Media");
                let is_video = matches!(src.kind, EffectKind::VideoFile);
                let (base_w, base_h) = if is_video { (360.0, 202.0) } else { (220.0, 320.0) };
                let size = vec2(base_w * k, base_h * k);
                let r = Rect::from_center_size(center, size);
                let corners = rotated_rect(r, angle);
                let fill_col = apply_tint(Color32::from_rgba_unmultiplied(65, 85, 120, 42));
                let stroke_col = apply_tint(Color32::from_rgba_unmultiplied(150, 190, 255, 130));
                fan(painter, center, &corners, fill_col);
                closed_stroke(painter, &corners, Stroke::new((1.5 * ctx.scale).max(1.0), stroke_col));

                let label = if fx.inversions.1 {
                    format!("{} [反転]", file_stem)
                } else {
                    file_stem.to_string()
                };
                painter.text(
                    center,
                    Align2::CENTER_CENTER,
                    label,
                    FontId::proportional((13.0 * k.max(0.4)).clamp(9.0, 22.0)),
                    apply_tint(Color32::from_rgba_unmultiplied(235, 245, 255, 210)),
                );
                Some(r)
            }
        }
        EffectKind::CustomObject => {
            if selected {
                let r = Rect::from_center_size(center, vec2(64.0 * ctx.scale, 64.0 * ctx.scale));
                painter.text(
                    center,
                    Align2::CENTER_CENTER,
                    obj.display_name(),
                    FontId::proportional(12.0),
                    apply_tint(Color32::LIGHT_GRAY),
                );
                Some(r)
            } else {
                None
            }
        }
        EffectKind::FrameBuffer => {
            let clear_fb = src.eval("フレームバッファをクリア", rel, src.eval("check0", rel, 0.0)) >= 0.5;
            if clear_fb {
                painter.rect_filled(ctx.canvas, 0.0, Color32::BLACK);
            }
            let has_transform = x.abs() > 0.5
                || y.abs() > 0.5
                || (zoom - 1.0).abs() > 0.005
                || angle.abs() > 0.005
                || alpha < 0.995;
            if (clear_fb || has_transform) && alpha > 0.005 && ctx.scene_depth < 4 {
                if let Some((cur_scene, cur_frame)) = ctx.current_scene {
                    let fb_layer_idx = cur_scene.find(obj.id).map(|(li, _)| li).unwrap_or(0);
                    if fb_layer_idx > 0 {
                        let mut fb_gt = ctx.group_transform.unwrap_or_default();
                        fb_gt.ox = x as f64;
                        fb_gt.oy = y as f64;
                        fb_gt.zoom = zoom as f64;
                        fb_gt.rz = angle.to_degrees() as f64;
                        fb_gt.alpha = alpha as f64;
                        let fb_ctx = DrawCtx {
                            painter,
                            canvas: ctx.canvas,
                            scale: ctx.scale,
                            fps: ctx.fps,
                            screen_w: ctx.screen_w,
                            screen_h: ctx.screen_h,
                            textures: ctx.textures,
                            plugins: ctx.plugins,
                            scenes: ctx.scenes,
                            has_ffmpeg: ctx.has_ffmpeg,
                            camera: ctx.camera,
                            group_transform: Some(fb_gt),
                            scene_depth: ctx.scene_depth + 1,
                            audio_waveform: ctx.audio_waveform,
                            current_scene: Some((cur_scene, cur_frame)),
                            layer_limit: Some(fb_layer_idx),
                        };
                        render_scene_at(&fb_ctx, cur_scene, cur_frame, None);
                    }
                }
            }
            if fx.hsv_shift.0.abs() > 0.01 {
                let b_delta = fx.hsv_shift.0;
                let ov = if b_delta > 0.0 {
                    Color32::from_white_alpha((b_delta.min(1.0) * alpha * 150.0) as u8)
                } else {
                    Color32::from_black_alpha(((-b_delta).min(1.0) * alpha * 220.0) as u8)
                };
                painter.rect_filled(ctx.canvas, 0.0, ov);
            }
            if let Some((tcol, t_str)) = fx.tint_color {
                let a = (t_str * alpha * 140.0).clamp(0.0, 220.0) as u8;
                painter.rect_filled(
                    ctx.canvas,
                    0.0,
                    Color32::from_rgba_unmultiplied(tcol.r(), tcol.g(), tcol.b(), a),
                );
            }
            if fx.glow_strength > 0.01 {
                let gcol = fx.glow_color.unwrap_or(Color32::from_rgb(255, 248, 220));
                let a = (fx.glow_strength * alpha * 75.0).clamp(0.0, 180.0) as u8;
                painter.rect_filled(
                    ctx.canvas,
                    0.0,
                    Color32::from_rgba_unmultiplied(gcol.r(), gcol.g(), gcol.b(), a),
                );
            }
            if fx.chroma_shift > 0.5 {
                let shift = (fx.chroma_shift * ctx.scale).clamp(1.0, 24.0);
                let r_rect = Rect::from_min_max(ctx.canvas.left_top(), pos2(ctx.canvas.left() + shift * 1.5, ctx.canvas.bottom()));
                let b_rect = Rect::from_min_max(pos2(ctx.canvas.right() - shift * 1.5, ctx.canvas.top()), ctx.canvas.right_bottom());
                painter.rect_filled(r_rect, 0.0, Color32::from_rgba_unmultiplied(255, 60, 60, (alpha * 50.0) as u8));
                painter.rect_filled(b_rect, 0.0, Color32::from_rgba_unmultiplied(60, 180, 255, (alpha * 50.0) as u8));
            }
            Some(ctx.canvas)
        }
        _ => {
            let sz = (src.eval("サイズ", rel, 100.0) as f32 * k).clamp(16.0 * ctx.scale, ctx.canvas.width());
            let r = Rect::from_center_size(center, vec2(sz, sz));
            let corners = rotated_rect(r, angle);
            let col = apply_tint(
                src.text("color")
                    .and_then(parse_color)
                    .unwrap_or(Color32::from_rgba_unmultiplied(100, 130, 180, 70)),
            );
            fan(painter, center, &corners, col);
            if selected {
                painter.text(
                    center,
                    Align2::CENTER_CENTER,
                    format!("{}\n{}", src.name, obj.display_name()),
                    FontId::proportional((13.0 * zoom.max(0.5)).min(24.0)),
                    apply_tint(Color32::WHITE),
                );
            }
            Some(r)
        }
    }
}