//! タイムライン上のクリップ内プレビュー描画（サムネイル・波形・図形・テキスト）およびツールチップ。

use std::collections::HashMap;

use au2_core::{EffectKind, Frame, Rational, TimelineObject};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Shape, Stroke};

use super::types::*;
/// Premiere Pro 風のイン・クリップ・プレビュー（サムネイル帯・音声波形・図形/色見本・テキスト・フィルタ曲線）を描画する。
pub(crate) fn draw_clip_preview(
    painter: &egui::Painter,
    body_rect: Rect,
    lanes: Rect,
    obj: &TimelineObject,
    clip_textures: &HashMap<String, egui::TextureId>,
    clip_waveforms: &HashMap<String, std::sync::Arc<au2_media::AudioWaveformPeaks>>,
    fps: f64,
    ppf: f32,
    dimmed: bool,
) {
    let alpha_mul = if dimmed { 0.45 } else { 1.0 };
    let Some(src) = obj.source() else { return };

    match src.kind {
        EffectKind::VideoFile | EffectKind::ImageFile => {
            let file_str = effect_file_path(src).unwrap_or("");
            let thumb_h = (body_rect.height() - 3.0).max(8.0);
            let thumb_w = (thumb_h * (16.0 / 9.0)).max(20.0);
            let start_x = body_rect.left().max(lanes.left() - thumb_w);
            let end_x = body_rect.right().min(lanes.right() + thumb_w);

            if let Some(&tex_id) = clip_textures.get(file_str) {
                let tint = Color32::from_white_alpha((235.0 * alpha_mul) as u8);
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                let mut x = body_rect.left();
                // 画面外の左側タイルをスキップ
                if x < start_x {
                    let skip = ((start_x - x) / thumb_w).floor();
                    x += skip * thumb_w;
                }
                while x < end_x {
                    let tile_right = (x + thumb_w - 1.0).min(body_rect.right());
                    if tile_right > x + 2.0 {
                        let tile_rect = Rect::from_min_max(
                            pos2(x + 1.0, body_rect.top() + 1.5),
                            pos2(tile_right, body_rect.bottom() - 1.5),
                        );
                        painter.image(tex_id, tile_rect, uv, tint);
                        // フィルムストリップのフレーム区切り線
                        painter.line_segment(
                            [pos2(tile_right, body_rect.top() + 1.0), pos2(tile_right, body_rect.bottom() - 1.0)],
                            Stroke::new(1.0_f32, Color32::from_black_alpha(120)),
                        );
                    }
                    x += thumb_w;
                }
            } else {
                // サムネイル未読込時のフィルムストリップ・プレースホルダー
                let mut x = body_rect.left();
                if x < start_x {
                    let skip = ((start_x - x) / thumb_w).floor();
                    x += skip * thumb_w;
                }
                while x < end_x {
                    let tile_right = (x + thumb_w - 2.0).min(body_rect.right());
                    if tile_right > x + 4.0 {
                        let tile_rect = Rect::from_min_max(
                            pos2(x + 1.5, body_rect.top() + 2.0),
                            pos2(tile_right, body_rect.bottom() - 2.0),
                        );
                        painter.rect_filled(tile_rect, 2.0, Color32::from_black_alpha(70));
                        painter.rect_stroke(tile_rect, 2.0, Stroke::new(1.0_f32, Color32::from_white_alpha(45)));
                    }
                    x += thumb_w;
                }
            }

            // 動画クリップに音声が含まれている場合、下部に波形オーバーレイを描画
            if src.kind == EffectKind::VideoFile {
                if let Some(wf) = clip_waveforms.get(file_str) {
                    let audio_cy = body_rect.bottom() - 6.0;
                    let audio_max_h = 5.0_f32;
                    let mut x = body_rect.left().max(lanes.left());
                    let dt = (2.0 / ppf.max(0.001) as f32 / fps as f32) as f64;
                    let wave_col = Color32::from_rgba_unmultiplied(120, 235, 185, (170.0 * alpha_mul) as u8);
                    while x < body_rect.right().min(lanes.right()) {
                        let rel_frames = ((x - body_rect.left()) / ppf.max(0.001)).max(0.0) as f64;
                        let rel_frame_int = rel_frames.floor() as Frame;
                        let start_pos = ((src.eval("再生位置", rel_frame_int, 1.0) - 1.0) / fps).max(0.0);
                        let speed = (src.eval("再生速度", rel_frame_int, 100.0) / 100.0).max(0.0);
                        let audio_sec = start_pos + (rel_frames / fps) * speed;
                        let peak_amp = wf.sample_range_amplitude(audio_sec, audio_sec + dt.max(0.005));
                        let amp_h = (peak_amp * audio_max_h).clamp(0.5, audio_max_h);
                        painter.line_segment(
                            [pos2(x, audio_cy - amp_h), pos2(x, audio_cy + amp_h)],
                            Stroke::new(1.2_f32, wave_col),
                        );
                        x += 2.0;
                    }
                }
            }
        }
        EffectKind::AudioFile => {
            // Premiere Pro / DaVinci Resolve 風の実オーディオ波形ピーク描画
            let vol = (src.eval("音量", 0, 100.0) / 100.0).clamp(0.0, 2.0) as f32;
            let cy = body_rect.center().y;
            let max_amp = (body_rect.height() * 0.44).min(body_rect.height() * 0.47);

            // センターライン
            painter.line_segment(
                [pos2(body_rect.left().max(lanes.left()), cy), pos2(body_rect.right().min(lanes.right()), cy)],
                Stroke::new(1.0_f32, Color32::from_white_alpha(45)),
            );

            let step = 2.0_f32; // 2px ごとに高密度ピークバーを描画
            let start_x = body_rect.left().max(lanes.left());
            let end_x = body_rect.right().min(lanes.right());
            let wave_color = Color32::from_rgba_unmultiplied(150, 245, 195, (225.0 * alpha_mul) as u8);

            let file_str = effect_file_path(src).unwrap_or("");
            if let Some(wf) = clip_waveforms.get(file_str) {
                // 実音声波形データ（ピークダウンサンプリング）から描画
                let mut x = start_x;
                let dt = (step / ppf.max(0.001) as f32 / fps as f32) as f64;
                while x < end_x {
                    let rel_frames = ((x - body_rect.left()) / ppf.max(0.001)).max(0.0) as f64;
                    let rel_frame_int = rel_frames.floor() as Frame;
                    let start_pos = src.eval("再生位置", rel_frame_int, 0.0).max(0.0);
                    let speed = (src.eval("再生速度", rel_frame_int, 100.0) / 100.0).max(0.0);
                    let audio_sec = start_pos + (rel_frames / fps) * speed;

                    let peak_amp = wf.sample_range_amplitude(audio_sec, audio_sec + dt.max(0.005));
                    let amp_h = (peak_amp * vol * max_amp).clamp(0.5, max_amp);

                    painter.line_segment(
                        [pos2(x, cy - amp_h), pos2(x, cy + amp_h)],
                        Stroke::new(1.4_f32, wave_color),
                    );
                    x += step;
                }
            } else {
                // 波形デコード中または波形未生成時の滑らかな擬似フォールバック
                let seed = obj.id.0 as f32 * 0.73;
                let mut x = start_x + ((body_rect.left() - start_x).rem_euclid(3.0));
                while x < end_x {
                    let rel = (x - body_rect.left()) * 0.14 + seed;
                    let amp = ((rel.sin() * 0.55 + (rel * 2.37).cos() * 0.35 + (rel * 0.41).sin() * 0.25)
                        .abs()
                        .clamp(0.12, 1.0))
                        * max_amp
                        * vol.clamp(0.2, 1.0);
                    painter.line_segment(
                        [pos2(x, cy - amp), pos2(x, cy + amp)],
                        Stroke::new(1.6_f32, wave_color.gamma_multiply(0.7)),
                    );
                    x += 3.0;
                }
            }
        }
        EffectKind::Text => {
            // テキスト内容のリアルタイム・スタイルプレビュー
            let text_val = src
                .text("テキスト")
                .or_else(|| src.text("text"))
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("");
            let display_str = if text_val.is_empty() { "(空のテキスト)" } else { text_val };
            let hex = src
                .text("文字色")
                .or_else(|| src.text("color"))
                .unwrap_or("ffffff");
            let text_col = parse_hex_rgb(hex, alpha_mul);
            let left_x = body_rect.left().max(lanes.left()) + 6.0;
            // カラースウォッチ丸印 + テキスト内容
            let swatch_center = pos2(left_x + 4.0, body_rect.center().y);
            painter.circle_filled(swatch_center, 4.5, text_col);
            painter.circle_stroke(swatch_center, 4.5, Stroke::new(1.0_f32, Color32::from_black_alpha(140)));
            painter.text(
                pos2(left_x + 13.0, body_rect.center().y),
                Align2::LEFT_CENTER,
                format!("“{display_str}”"),
                FontId::proportional(12.0),
                Color32::from_white_alpha((240.0 * alpha_mul) as u8),
            );
        }
        EffectKind::Shape => {
            // 図形ミニアイコン + 実際の図形カラー見本 + サイズ表示
            let shape_name = src
                .text("図形の種類")
                .or_else(|| src.text("type"))
                .unwrap_or("円");
            let hex = src
                .text("色")
                .or_else(|| src.text("color"))
                .unwrap_or("ffffff");
            let shape_col = parse_hex_rgb(hex, alpha_mul);
            let size_px = src.eval("サイズ", 0, 100.0) as i32;

            let left_x = body_rect.left().max(lanes.left()) + 6.0;
            let icon_c = pos2(left_x + 8.0, body_rect.center().y);
            let r = (body_rect.height() * 0.34).clamp(4.0, 9.0);

            // ミニ図形レンダリング
            painter.rect_filled(
                Rect::from_center_size(icon_c, vec2(r * 2.4, r * 2.2)),
                2.5,
                Color32::from_black_alpha(95),
            );
            if shape_name.contains("四角") || shape_name.contains("背景") {
                painter.rect_filled(Rect::from_center_size(icon_c, vec2(r * 1.6, r * 1.4)), 1.5, shape_col);
            } else if shape_name.contains("三角") {
                painter.add(Shape::convex_polygon(
                    vec![
                        pos2(icon_c.x, icon_c.y - r),
                        pos2(icon_c.x + r, icon_c.y + r * 0.85),
                        pos2(icon_c.x - r, icon_c.y + r * 0.85),
                    ],
                    shape_col,
                    Stroke::NONE,
                ));
            } else {
                painter.circle_filled(icon_c, r * 0.85, shape_col);
            }

            painter.text(
                pos2(left_x + 22.0, body_rect.center().y),
                Align2::LEFT_CENTER,
                format!("{shape_name} ({size_px}px / #{hex})"),
                FontId::proportional(11.0),
                Color32::from_white_alpha((225.0 * alpha_mul) as u8),
            );
        }
        _ => {
            // フィルタ・制御・シーン等: エフェクト曲線風ライン + パラメータ要約
            let cy = body_rect.center().y;
            let start_x = body_rect.left().max(lanes.left());
            let end_x = body_rect.right().min(lanes.right());
            painter.line_segment(
                [pos2(start_x, cy), pos2(end_x, cy)],
                Stroke::new(1.0_f32, Color32::from_white_alpha(65)),
            );
            let summary = src.name.as_str();
            painter.text(
                pos2(start_x + 6.0, cy),
                Align2::LEFT_CENTER,
                summary,
                FontId::proportional(10.5),
                Color32::from_white_alpha((205.0 * alpha_mul) as u8),
            );
        }
    }
}

/// クリップにマウスホバーした際、サムネイル拡大プレビューと詳細情報をツールチップ表示する。
pub(crate) fn draw_clip_hover_tooltip(
    painter: &egui::Painter,
    full: Rect,
    hover_pos: Pos2,
    layer_idx: usize,
    obj: &TimelineObject,
    rate: Rational,
    clip_textures: &HashMap<String, egui::TextureId>,
) {
    let has_thumb = obj
        .source()
        .and_then(effect_file_path)
        .is_some_and(|p| clip_textures.contains_key(p));

    let tip_w = 220.0;
    let tip_h = if has_thumb { 166.0 } else { 78.0 };
    let mut tip_pos = hover_pos + vec2(12.0, -tip_h - 10.0);
    if tip_pos.y < full.top() + 4.0 {
        tip_pos.y = hover_pos.y + 16.0;
    }
    if tip_pos.x + tip_w > full.right() - 4.0 {
        tip_pos.x = (full.right() - tip_w - 4.0).max(full.left() + 4.0);
    }
    let tip_rect = Rect::from_min_size(tip_pos, vec2(tip_w, tip_h));

    painter.rect_filled(tip_rect, 5.0, Color32::from_rgba_unmultiplied(20, 22, 27, 242));
    painter.rect_stroke(tip_rect, 5.0, Stroke::new(1.0_f32, Color32::from_gray(95)));

    let mut cur_y = tip_rect.top() + 7.0;
    if has_thumb {
        if let Some(file_str) = obj.source().and_then(effect_file_path) {
            if let Some(&tex_id) = clip_textures.get(file_str) {
                let thumb_rect = Rect::from_min_size(pos2(tip_rect.left() + 8.0, cur_y), vec2(tip_w - 16.0, 88.0));
                painter.rect_filled(thumb_rect, 3.0, Color32::BLACK);
                painter.image(
                    tex_id,
                    thumb_rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                painter.rect_stroke(thumb_rect, 3.0, Stroke::new(1.0_f32, Color32::from_gray(70)));
                cur_y += 94.0;
            }
        }
    }

    let icon = clip_kind_icon(obj);
    painter.text(
        pos2(tip_rect.left() + 8.0, cur_y),
        Align2::LEFT_TOP,
        format!("{icon} {}", obj.display_name()),
        FontId::proportional(12.5),
        Color32::WHITE,
    );
    cur_y += 18.0;

    let dur_frames = obj.frame_end.saturating_sub(obj.frame_start) + 1;
    let info = format!(
        "Layer {}  |  {} - {} ({}f)",
        layer_idx + 1,
        fmt_time(obj.frame_start, rate),
        fmt_time(obj.frame_end, rate),
        dur_frames
    );
    painter.text(
        pos2(tip_rect.left() + 8.0, cur_y),
        Align2::LEFT_TOP,
        info,
        FontId::monospace(10.5),
        Color32::from_gray(190),
    );
    cur_y += 16.0;

    let fx_names: Vec<&str> = obj
        .effects
        .iter()
        .skip(1)
        .filter(|e| !is_render_or_playback(&e.kind))
        .map(|e| e.name.as_str())
        .collect();
    let fx_line = if fx_names.is_empty() {
        format!("種別: {}", obj.source().map(|s| s.name.as_str()).unwrap_or("オブジェクト"))
    } else {
        format!("エフェクト: {}", fx_names.join(", "))
    };
    painter.text(
        pos2(tip_rect.left() + 8.0, cur_y),
        Align2::LEFT_TOP,
        fx_line,
        FontId::proportional(10.5),
        Color32::from_rgb(170, 210, 255),
    );
}

