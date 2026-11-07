//! プレビュー画面パネル
//! 
//! - 3Dカメラ投影、テキスト組版、図形、メディア画像/動画、フィルタの合成プレビューを行います。

pub mod camera;
pub mod filters;
pub mod render;
pub mod text;

#[allow(unused_imports)]
pub use camera::*;
#[allow(unused_imports)]
pub(crate) use filters::*;
pub(crate) use render::*;
pub use text::*;

use au2_core::{EffectKind, ObjectId};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke};
use crate::state::EditorState;

pub fn ui(ui: &mut egui::Ui, st: &mut EditorState) {
    let egui_ctx = ui.ctx().clone();

    // 毎フレームのレイアウト幅変動（ジッター）を完全に防止するため、UI幅を整数ピクセルにロック
    let total_w = ui.available_width().floor();
    let total_h = ui.available_height().floor();
    ui.set_max_width(total_w);

    // 下部のシークバー＆再生コントロール領域の高さ（76px 固定）を残して上部にプレビューキャンバスを配置
    let control_height = 78.0_f32 + 8.0;
    let preview_h = (total_h - control_height).max(64.0);
    let preview_size = vec2(total_w.max(64.0), preview_h);

    let (resp, painter) = ui.allocate_painter(preview_size, Sense::click());
    let area = resp.rect;
    painter.rect_filled(area, 0.0, crate::theme::flip(Color32::from_gray(24)));

    let s = &st.scene.settings;
    let (sw, sh) = (s.width.max(1) as f32, s.height.max(1) as f32);
    let scale = (area.width() / sw).min(area.height() / sh).max(0.0001);
    let canvas_w = (sw * scale).floor();
    let canvas_h = (sh * scale).floor();
    let canvas_left = (area.center().x - canvas_w / 2.0).floor();
    let canvas_top = (area.center().y - canvas_h / 2.0).floor();
    let canvas = Rect::from_min_size(pos2(canvas_left, canvas_top), vec2(canvas_w, canvas_h));
    painter.rect_filled(canvas, 0.0, Color32::BLACK);
    let painter = painter.with_clip_rect(canvas.intersect(area));

    let fps = s.frame_rate.as_f64();
    let frame = st.frame;

    // 1. 現在フレーム（およびサブシーン内）に存在する画像・PSD・動画オブジェクトのテクスチャを事前取得
    struct MediaReq {
        id: ObjectId,
        file_str: String,
        media_frame: u32,
        media_sec: f64,
        rel: u32,
        psd_layers: String,
        psd_blink: String,
        psd_lipsync: String,
    }
    let mut media_requests: Vec<MediaReq> = Vec::new();
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
                    let psd_layers = src.text("psd_layers").unwrap_or_default().to_string();
                    let psd_blink = src.text("psd_blink").unwrap_or_default().to_string();
                    let psd_lipsync = src.text("psd_lipsync").unwrap_or_default().to_string();
                    media_requests.push(MediaReq {
                        id: obj.id,
                        file_str: file_str.to_string(),
                        media_frame,
                        media_sec,
                        rel,
                        psd_layers,
                        psd_blink,
                        psd_lipsync,
                    });
                }
            }
        }
    }

    let mut resolved_textures: std::collections::HashMap<ObjectId, (egui::TextureId, [usize; 2])> =
        std::collections::HashMap::new();
    for req in media_requests {
        let lower = req.file_str.to_ascii_lowercase();
        let tex_opt = if lower.ends_with(".psd") || lower.ends_with(".psb") {
            st.get_psd_object_texture(
                &egui_ctx,
                &req.file_str,
                &req.psd_layers,
                &req.psd_blink,
                &req.psd_lipsync,
                req.rel,
                frame,
            )
        } else {
            st.get_media_texture(&egui_ctx, &req.file_str, req.media_frame, req.media_sec)
        };
        if let Some(tex) = tex_opt {
            resolved_textures.insert(req.id, tex);
        }
    }

    let has_ffmpeg = st.media.has_ffmpeg();

    let ctx = DrawCtx {
        painter: &painter,
        canvas,
        scale,
        fps,
        screen_w: sw as f64,
        screen_h: sh as f64,
        textures: &resolved_textures,
        plugins: &st.plugins,
        scenes: &st.scenes,
        has_ffmpeg,
        camera: None,
        group_transform: None,
        scene_depth: 0,
    };

    render_scene_at(&ctx, &st.scene, frame, st.selected);

    let last = st.last_frame();
    let total_digits = ((last + 1).max(1) as f64).log10().floor() as usize + 1;
    let overlay_str = format!(
        "{} / frame {:>width$}",
        st.scene.settings.frame_rate.timecode(frame),
        frame + 1,
        width = total_digits
    );
    draw_fixed_pitch_text(
        &painter,
        canvas.left_top() + vec2(6.0, 4.0),
        Align2::LEFT_TOP,
        &overlay_str,
        FontId::proportional(12.0),
        Color32::from_white_alpha(160),
        7.2,
    );

    // 3. 動画プレビュー下部にシークバーと再生コントロールボタンを配置
    // (間隔は ui.spacing().item_spacing.y で確保)
    bottom_transport_bar(ui, st);
}

/// プロポーショナルフォント（LINE Seed JP 等）でも、1文字ずつ等幅のセル中央に配置することで
/// 秒数やフレーム数の数字切り替え時に横揺れ（ジッター）が起きないように描画します。

fn bottom_transport_bar(ui: &mut egui::Ui, st: &mut EditorState) {
    let last = st.last_frame();
    let mut f = st.frame;

    // トランスポートバー全体のカード（高さ固定。内部は実測ベースのレイアウトで重なりを防ぐ）
    let bar_w = ui.available_width().floor().max(160.0);
    let bar_h = 78.0_f32;
    let (bar_rect, _) = ui.allocate_exact_size(vec2(bar_w, bar_h), Sense::hover());
    ui.painter().rect(
        bar_rect,
        egui::Rounding::same(8.0),
        crate::theme::surface(),
        Stroke::new(1.0_f32, crate::theme::border()),
    );

    let inner = bar_rect.shrink2(vec2(12.0, 8.0));

    // 1段目：シークバー
    let row1_rect = Rect::from_min_size(inner.min, vec2(inner.width(), 24.0));
    let mut row1_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row1_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    row1_ui.spacing_mut().slider_width = (row1_rect.width() - 8.0).floor().max(60.0);
    row1_ui.style_mut().visuals.slider_trailing_fill = true;
    if row1_ui
        .add(egui::Slider::new(&mut f, 0..=last).show_value(false))
        .on_hover_text("シークバー（ドラッグで再生位置を移動）")
        .changed()
    {
        st.set_frame(f);
    }

    // 2段目：タイムコード（左）／再生ボタン群（中央）／解像度（右）
    let row2_h = 30.0_f32;
    let row2_rect = Rect::from_min_size(
        pos2(inner.left().floor(), (inner.bottom() - row2_h).floor()),
        vec2(inner.width().floor(), row2_h),
    );
    let center_y = row2_rect.center().y.floor();

    let rate = st.scene.settings.frame_rate;
    let tc = rate.timecode(st.frame);
    let total_digits = ((last + 1).max(1) as f64).log10().floor() as usize + 1;
    let frame_str = format!("{:>width$} / {} F", st.frame + 1, last + 1, width = total_digits);

    // 左：TC バッジ + タイムコード + フレーム番号（等幅セルで描画して横揺れを防止）
    let tc_badge_rect = Rect::from_min_size(pos2(row2_rect.left(), center_y - 12.0), vec2(30.0, 24.0));
    ui.painter().rect(
        tc_badge_rect,
        egui::Rounding::same(5.0),
        crate::theme::flip(Color32::from_rgb(28, 34, 46)),
        Stroke::new(1.0_f32, crate::theme::flip(Color32::from_rgb(45, 60, 90))),
    );
    ui.painter().text(
        tc_badge_rect.center(),
        Align2::CENTER_CENTER,
        "TC",
        FontId::monospace(10.0),
        crate::theme::accent_text(),
    );
    let tc_text_x = (tc_badge_rect.right() + 8.0).floor();
    let tc_w = tc.chars().count() as f32 * 8.0;
    draw_fixed_pitch_text(
        ui.painter(),
        pos2(tc_text_x, center_y),
        Align2::LEFT_CENTER,
        &tc,
        FontId::proportional(13.0),
        crate::theme::text(),
        8.0,
    );
    let frame_text_x = (tc_text_x + tc_w + 10.0).floor();
    let frame_w = frame_str.chars().count() as f32 * 7.0;
    draw_fixed_pitch_text(
        ui.painter(),
        pos2(frame_text_x, center_y),
        Align2::LEFT_CENTER,
        &frame_str,
        FontId::proportional(11.0),
        crate::theme::text_muted(),
        7.0,
    );
    let left_end = frame_text_x + frame_w;

    // 中央：再生ボタン群（左のテキストと重ならない位置に配置）
    let nav_w = 36.0_f32;
    let play_w = 108.0_f32;
    let gap = 6.0_f32;
    let btn_h = 28.0_f32;
    let group_w = nav_w * 4.0 + play_w + gap * 4.0;
    let mut group_x = (row2_rect.center().x - group_w * 0.5).floor();
    if group_x < left_end + 16.0 {
        group_x = (left_end + 16.0).floor();
    }
    group_x = group_x.min(row2_rect.right() - group_w).max(row2_rect.left());
    let btn_rect = Rect::from_min_size(pos2(group_x, center_y - btn_h * 0.5), vec2(group_w, btn_h));
    let mut btn_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(btn_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    btn_ui.spacing_mut().item_spacing = vec2(gap, 0.0);

    let nav_btn = |ui: &mut egui::Ui, text: &'static str, tip: &'static str| -> egui::Response {
        ui.add_sized(vec2(nav_w, btn_h), egui::Button::new(egui::RichText::new(text).size(13.0)))
            .on_hover_text(tip)
    };

    if nav_btn(&mut btn_ui, "⏮", "先頭フレームへ (Home)").clicked() {
        st.set_frame(0);
    }
    if nav_btn(&mut btn_ui, "◀", "1 コマ戻る (←)").clicked() {
        st.step(-1);
    }

    let (play_label, play_color) = if st.playing {
        ("⏸ 一時停止", Color32::from_rgb(220, 38, 38))
    } else {
        ("▶ 再生", crate::theme::accent())
    };
    let play_btn =
        egui::Button::new(egui::RichText::new(play_label).strong().size(13.0).color(Color32::WHITE)).fill(play_color);
    if btn_ui
        .add_sized(vec2(play_w, btn_h), play_btn)
        .on_hover_text("再生 / 一時停止 (Space)")
        .clicked()
    {
        st.toggle_play();
    }

    if nav_btn(&mut btn_ui, "▶|", "1 コマ進む (→)").clicked() {
        st.step(1);
    }
    if nav_btn(&mut btn_ui, "⏭", "最終フレームへ (End)").clicked() {
        st.set_frame(last);
    }

    // 右端：解像度表示（ボタン群と重ならない余白がある時のみ）
    let s = &st.scene.settings;
    let res_text = format!("{}×{}  {:.0}fps", s.width, s.height, s.frame_rate.as_f64());
    let res_w = res_text.chars().count() as f32 * 7.0;
    if row2_rect.right() - (group_x + group_w) >= res_w + 16.0 {
        ui.painter().text(
            pos2(row2_rect.right().floor(), center_y),
            Align2::RIGHT_CENTER,
            res_text,
            FontId::monospace(11.0),
            crate::theme::text_muted(),
        );
    }
}

