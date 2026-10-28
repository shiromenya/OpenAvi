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

    // 毎フレームのレイアウト幅変動（ジッター）を完全に防止するため、UI幅を整数ピクセルにロック
    let total_w = ui.available_width().floor();
    let total_h = ui.available_height().floor();
    ui.set_max_width(total_w);

    // 下部のシークバー＆再生コントロール領域の高さ（マージン込みで約 84px）を残して上部にプレビューキャンバスを配置
    let control_height = 84.0_f32;
    let preview_h = (total_h - control_height).max(64.0);
    let preview_size = vec2(total_w.max(64.0), preview_h);

    let (resp, painter) = ui.allocate_painter(preview_size, Sense::click());
    let area = resp.rect;
    painter.rect_filled(area, 0.0, Color32::from_gray(24));

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

    let has_ffmpeg = st.media.has_ffmpeg();

    // 2. カメラ制御オブジェクトの検出とパラメータ構築
    let mut active_camera = None;
    let mut camera_layer = None;
    for (li, obj) in st.scene.objects_at(frame) {
        if !st.scene.layers[li].visible {
            continue;
        }
        if let Some(cam_eff) = obj.effects.iter().find(|e| e.name == "カメラ制御" || matches!(e.kind, EffectKind::Camera)) {
            if !cam_eff.enabled {
                continue;
            }
            let rel = frame - obj.frame_start;
            let cx = cam_eff.eval("X", rel, 0.0);
            let cy = cam_eff.eval("Y", rel, 0.0);
            let cz = cam_eff.eval("Z", rel, -1024.0);
            let mut tx = cam_eff.eval("目標X", rel, 0.0);
            let mut ty = cam_eff.eval("目標Y", rel, 0.0);
            let mut tz = cam_eff.eval("目標Z", rel, 0.0);
            let target_layer = cam_eff.eval("目標レイヤー", rel, 0.0) as usize;
            if target_layer > 0 && target_layer <= st.scene.layers.len() {
                let target_li = target_layer - 1;
                if let Some(t_obj) = st.scene.layers[target_li].objects.iter().find(|o| o.contains(frame)) {
                    let t_rel = frame - t_obj.frame_start;
                    if let Some(t_draw) = t_obj.effect(&EffectKind::StandardDraw).or_else(|| t_obj.effect(&EffectKind::ExtendedDraw)) {
                        tx = t_draw.eval("X", t_rel, 0.0);
                        ty = t_draw.eval("Y", t_rel, 0.0);
                        tz = t_draw.eval("Z", t_rel, 0.0);
                    }
                }
            }
            let roll = cam_eff.eval("傾き", rel, 0.0);
            let fov = cam_eff.eval("視野角", rel, 0.0);

            let d = if fov > 0.1 {
                (sh as f64 / 2.0) / (fov.to_radians() / 2.0).tan()
            } else {
                1024.0
            };

            let mut cam_params = CameraParams {
                x: cx,
                y: cy,
                z: cz,
                tx,
                ty,
                tz,
                rz: roll,
                ux: 0.0,
                uy: -1.0,
                uz: 0.0,
                d,
            };

            // カメラ制御オブジェクト自身に含まれるカメラ効果を適用
            for eff in &obj.effects {
                if (eff.name == "カメラ効果" || eff.name == "スクリプト(カメラ制御)") && eff.enabled {
                    let eff_sub_name = eff.text("name").unwrap_or("手ぶれ");
                    let tr0 = eff.eval("track0", rel, 0.0);
                    let tr1 = eff.eval("track1", rel, 0.0);
                    let tr2 = eff.eval("track2", rel, 0.0);
                    let tr3 = eff.eval("track3", rel, 0.0);
                    apply_camera_effect(&mut cam_params, eff_sub_name, tr0, tr1, tr2, tr3, rel as f64 / fps, sh as f64);
                }
            }

            // タイムライン上で同時にアクティブな別レイヤーの「カメラ効果」オブジェクトもすべて適用
            for (other_li, other_obj) in st.scene.objects_at(frame) {
                if other_obj.id == obj.id || !st.scene.layers[other_li].visible {
                    continue;
                }
                for eff in &other_obj.effects {
                    if (eff.name == "カメラ効果" || eff.name == "スクリプト(カメラ制御)") && eff.enabled {
                        let o_rel = frame - other_obj.frame_start;
                        let eff_sub_name = eff.text("name").unwrap_or("手ぶれ");
                        let tr0 = eff.eval("track0", o_rel, 0.0);
                        let tr1 = eff.eval("track1", o_rel, 0.0);
                        let tr2 = eff.eval("track2", o_rel, 0.0);
                        let tr3 = eff.eval("track3", o_rel, 0.0);
                        apply_camera_effect(&mut cam_params, eff_sub_name, tr0, tr1, tr2, tr3, o_rel as f64 / fps, sh as f64);
                    }
                }
            }

            active_camera = Some(Camera3D::from_params(&cam_params, fov));
            camera_layer = Some(li);
            break;
        }
    }

    // 3. レイヤー順 / 3D深度順に合成描画
    let ctx = DrawCtx {
        painter: &painter,
        canvas,
        scale,
        fps,
        screen_w: sw as f64,
        screen_h: sh as f64,
        textures: &resolved_textures,
        plugins: &st.plugins,
        has_ffmpeg,
        camera: active_camera,
    };

    let mut visible_objs: Vec<(usize, &TimelineObject, Frame, f64)> = Vec::new();
    for (li, obj) in st.scene.objects_at(frame) {
        if !st.scene.layers[li].visible {
            continue;
        }
        let rel = frame - obj.frame_start;
        let depth = if obj.camera_target {
            if let Some(ref cam) = active_camera {
                let draw = obj
                    .effect(&EffectKind::StandardDraw)
                    .or_else(|| obj.effect(&EffectKind::ExtendedDraw));
                let bx = draw.map(|d| d.eval("X", rel, 0.0)).unwrap_or(0.0);
                let by = draw.map(|d| d.eval("Y", rel, 0.0)).unwrap_or(0.0);
                let bz = draw.map(|d| d.eval("Z", rel, 0.0)).unwrap_or(0.0);
                let vx = bx - cam.eye[0];
                let vy = by - cam.eye[1];
                let vz = bz - cam.eye[2];
                vx * cam.forward[0] + vy * cam.forward[1] + vz * cam.forward[2]
            } else {
                0.0
            }
        } else {
            0.0
        };
        visible_objs.push((li, obj, rel, depth));
    }

    let cam_layer = camera_layer.unwrap_or(usize::MAX);
    if active_camera.is_some() {
        visible_objs.sort_by(|a, b| {
            let a_cam = a.1.camera_target;
            let b_cam = b.1.camera_target;
            let a_group = if a_cam {
                1u8 // 3D空間
            } else if a.0 < cam_layer {
                0u8 // 2D背景
            } else {
                2u8 // 2D前面オーバーレイ
            };
            let b_group = if b_cam {
                1u8
            } else if b.0 < cam_layer {
                0u8
            } else {
                2u8
            };

            if a_group != b_group {
                a_group.cmp(&b_group)
            } else if a_group == 1 {
                // 3D 同士: カメラから遠い順（depth 降順）。depth が同じならレイヤー昇順
                b.3.total_cmp(&a.3).then_with(|| a.0.cmp(&b.0))
            } else {
                // 2D 同士: レイヤー昇順
                a.0.cmp(&b.0)
            }
        });
    }

    for (li, obj, rel, _) in visible_objs {
        draw_object(&ctx, li, obj, rel, st.selected == Some(obj.id));
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
            // 親 Ui へのフィードバックループ（毎フレームの幅伸縮・振動）を完全に遮断
            ui.set_max_width(ui.available_width().floor());

            // 1段目：シークバー
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 8.0).floor().max(80.0);
                if ui
                    .add(egui::Slider::new(&mut f, 0..=last).show_value(false))
                    .on_hover_text("シークバー（ドラッグで再生位置を移動）")
                    .changed()
                {
                    st.set_frame(f);
                }
            });

            ui.add_space(3.0);

            // 2段目：プロ仕様 3 カラムレイアウト（秒数・フレーム変化時も再生ボタンが一切揺れない完全固定幅設計）
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 0.0);

                let rate = st.scene.settings.frame_rate;
                let tc = rate.timecode(st.frame);

                // タイムコード＆フレーム数表示エリアを固定幅 (190px) に固定し、数字が変わっても右側のボタン群が一切揺れないようにする
                ui.allocate_ui_with_layout(
                    vec2(190.0, 24.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
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
                    },
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

                // 再生ボタンも固定幅 (96px) の領域に配置し、「再生」「一時停止」の文字数差でも右側ボタンがブレない
                ui.allocate_ui_with_layout(
                    vec2(96.0, 24.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let (play_label, play_color) = if st.playing {
                            ("⏸ 一時停止", Color32::from_rgb(220, 38, 38))
                        } else {
                            ("▶ 再生", Color32::from_rgb(37, 99, 235))
                        };
                        let play_btn = egui::Button::new(
                            egui::RichText::new(play_label).strong().size(12.0).color(Color32::WHITE),
                        )
                        .fill(play_color)
                        .min_size(vec2(96.0, 24.0));
                        if ui.add(play_btn).on_hover_text("再生 / 一時停止 (Space)").clicked() {
                            st.toggle_play();
                        }
                    },
                );

                if nav_btn(ui, "▶|", "1 コマ進む (→)").clicked() {
                    st.step(1);
                }
                if nav_btn(ui, "⏭", "最終フレームへ (End)").clicked() {
                    st.set_frame(last);
                }

                // 右端の解像度表示：十分な余白（110px 以上）がある時のみ描画し、左側のボタン群と決して衝突させない
                if ui.available_width() >= 110.0 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let s = &st.scene.settings;
                        ui.label(
                            egui::RichText::new(format!("{}×{}  {:.0}fps", s.width, s.height, s.frame_rate.as_f64()))
                                .size(11.0)
                                .color(Color32::from_gray(145)),
                        );
                    });
                }
            });
        });
}

#[derive(Clone, Copy, Debug)]
pub struct CameraParams {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub tx: f64,
    pub ty: f64,
    pub tz: f64,
    pub rz: f64,
    pub ux: f64,
    pub uy: f64,
    pub uz: f64,
    pub d: f64,
}

/// AviUtl 互換の決定論的疑似乱数生成器 (obj.rand 互換)
pub fn aviutl_rand(st_num: f64, ed_num: f64, seed: i32, frame: i64) -> f64 {
    let mut h = (frame as u64).wrapping_mul(0x9E3779B97F4A7C15)
        ^ ((seed as u64).wrapping_mul(0xBF58476D1CE4E5B9));
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D049BB133111EB);
    h ^= h >> 31;
    let norm = (h as f64) / (u64::MAX as f64);
    st_num + norm * (ed_num - st_num)
}

/// AviUtl 互換の Catmull-Rom スプライン補間 (obj.interpolation 互換)
pub fn aviutl_interpolation(t: f64, p0: f64, p1: f64, p2: f64, p3: f64) -> f64 {
    0.5 * (
        (2.0 * p1)
            + (-p0 + p2) * t
            + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
            + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t
    )
}

/// カメラ効果（@手ぶれ、@目標中心回転、@目標サイズ固定視野角 等）をカメラパラメータに適用します。
pub fn apply_camera_effect(
    cam: &mut CameraParams,
    eff_name: &str,
    track0: f64,
    track1: f64,
    track2: f64,
    _track3: f64,
    time_sec: f64,
    screen_h: f64,
) {
    let sec = eff_name.split('@').next().unwrap_or(eff_name);
    match sec {
        "手ぶれ" | "手ブレ" => {
            if track2 <= 0.0 {
                cam.tx += aviutl_rand(-track0, track0, 0, (time_sec * 100.0) as i64);
                cam.ty += aviutl_rand(-track0, track0, 1, (time_sec * 100.0) as i64);
                cam.tz += aviutl_rand(-track0, track0, 2, (time_sec * 100.0) as i64);
                cam.rz += aviutl_rand(-track1, track1, 3, (time_sec * 100.0) as i64);
            } else {
                let t = time_sec * 100.0 / track2;
                let p = t.floor() as i64;
                let t_frac = t - (p as f64);

                let x0 = aviutl_rand(-1000.0, 1000.0, 0, p) / 1000.0 * track0;
                let x1 = aviutl_rand(-1000.0, 1000.0, 0, p + 1) / 1000.0 * track0;
                let x2 = aviutl_rand(-1000.0, 1000.0, 0, p + 2) / 1000.0 * track0;
                let x3 = aviutl_rand(-1000.0, 1000.0, 0, p + 3) / 1000.0 * track0;

                let y0 = aviutl_rand(-1000.0, 1000.0, 1, p) / 1000.0 * track0;
                let y1 = aviutl_rand(-1000.0, 1000.0, 1, p + 1) / 1000.0 * track0;
                let y2 = aviutl_rand(-1000.0, 1000.0, 1, p + 2) / 1000.0 * track0;
                let y3 = aviutl_rand(-1000.0, 1000.0, 1, p + 3) / 1000.0 * track0;

                let z0 = aviutl_rand(-1000.0, 1000.0, 2, p) / 1000.0 * track0;
                let z1 = aviutl_rand(-1000.0, 1000.0, 2, p + 1) / 1000.0 * track0;
                let z2 = aviutl_rand(-1000.0, 1000.0, 2, p + 2) / 1000.0 * track0;
                let z3 = aviutl_rand(-1000.0, 1000.0, 2, p + 3) / 1000.0 * track0;

                let r0 = aviutl_rand(-1000.0, 1000.0, 3, p) / 1000.0 * track1;
                let r1 = aviutl_rand(-1000.0, 1000.0, 3, p + 1) / 1000.0 * track1;
                let r2 = aviutl_rand(-1000.0, 1000.0, 3, p + 2) / 1000.0 * track1;
                let r3 = aviutl_rand(-1000.0, 1000.0, 3, p + 3) / 1000.0 * track1;

                cam.tx += aviutl_interpolation(t_frac, x0, x1, x2, x3);
                cam.ty += aviutl_interpolation(t_frac, y0, y1, y2, y3);
                cam.tz += aviutl_interpolation(t_frac, z0, z1, z2, z3);
                cam.rz += aviutl_interpolation(t_frac, r0, r1, r2, r3);
            }
        }
        "目標中心回転" => {
            let x = cam.x - cam.tx;
            let y = cam.y - cam.ty;
            let z = cam.z - cam.tz;

            let vx = cam.ux;
            let vy = cam.uy;
            let vz = cam.uz;
            let r_h = track0.to_radians();
            let s_h = r_h.sin();
            let c_h = r_h.cos();
            let m00 = vx * vx * (1.0 - c_h) + c_h;
            let m01 = vx * vy * (1.0 - c_h) - vz * s_h;
            let m02 = vz * vx * (1.0 - c_h) + vy * s_h;
            let m10 = vx * vy * (1.0 - c_h) + vz * s_h;
            let m11 = vy * vy * (1.0 - c_h) + c_h;
            let m12 = vy * vz * (1.0 - c_h) - vx * s_h;
            let m20 = vz * vx * (1.0 - c_h) - vy * s_h;
            let m21 = vy * vz * (1.0 - c_h) + vx * s_h;
            let m22 = vz * vz * (1.0 - c_h) + c_h;

            let x2 = m00 * x + m01 * y + m02 * z;
            let y2 = m10 * x + m11 * y + m12 * z;
            let z2 = m20 * x + m21 * y + m22 * z;

            let l2 = (x2 * x2 + y2 * y2 + z2 * z2).sqrt();
            if l2 > 1e-6 {
                let n_vx = x2 / l2;
                let n_vy = y2 / l2;
                let n_vz = z2 / l2;

                let cr_x = n_vy * cam.uz - n_vz * cam.uy;
                let cr_y = n_vz * cam.ux - n_vx * cam.uz;
                let cr_z = n_vx * cam.uy - n_vy * cam.ux;
                let cr_l = (cr_x * cr_x + cr_y * cr_y + cr_z * cr_z).sqrt();
                if cr_l > 1e-6 {
                    let ax_x = cr_x / cr_l;
                    let ax_y = cr_y / cr_l;
                    let ax_z = cr_z / cr_l;

                    let r_v = track1.to_radians();
                    let s_v = r_v.sin();
                    let c_v = r_v.cos();

                    let vm00 = ax_x * ax_x * (1.0 - c_v) + c_v;
                    let vm01 = ax_x * ax_y * (1.0 - c_v) - ax_z * s_v;
                    let vm02 = ax_z * ax_x * (1.0 - c_v) + ax_y * s_v;
                    let vm10 = ax_x * ax_y * (1.0 - c_v) + ax_z * s_v;
                    let vm11 = ax_y * ax_y * (1.0 - c_v) + c_v;
                    let vm12 = ax_y * ax_z * (1.0 - c_v) - ax_x * s_v;
                    let vm20 = ax_z * ax_x * (1.0 - c_v) - ax_y * s_v;
                    let vm21 = ax_y * ax_z * (1.0 - c_v) + ax_x * s_v;
                    let vm22 = ax_z * ax_z * (1.0 - c_v) + c_v;

                    cam.x = cam.tx + vm00 * x2 + vm01 * y2 + vm02 * z2;
                    cam.y = cam.ty + vm10 * x2 + vm11 * y2 + vm12 * z2;
                    cam.z = cam.tz + vm20 * x2 + vm21 * y2 + vm22 * z2;

                    let ux = cam.ux;
                    let uy = cam.uy;
                    let uz = cam.uz;
                    cam.ux = vm00 * ux + vm01 * uy + vm02 * uz;
                    cam.uy = vm10 * ux + vm11 * uy + vm12 * uz;
                    cam.uz = vm20 * ux + vm21 * uy + vm22 * uz;
                }
            }
        }
        "目標サイズ固定視野角" => {
            let x = cam.tx - cam.x;
            let y = cam.ty - cam.y;
            let z = cam.tz - cam.z;
            let l = (x * x + y * y + z * z).sqrt();
            if l > 1e-6 && cam.d > 1e-6 {
                let norm_x = x / l;
                let norm_y = y / l;
                let norm_z = z / l;
                let r = l / cam.d;
                let mut p = (screen_h / (cam.d * 2.0)).atan() * 360.0 / std::f64::consts::PI;
                p = (p + track0).clamp(1.0, 120.0);
                cam.d = screen_h / ((p * std::f64::consts::PI / 360.0).tan() * 2.0);
                let new_l = r * cam.d;
                cam.x = cam.tx - new_l * norm_x;
                cam.y = cam.ty - new_l * norm_y;
                cam.z = cam.tz - new_l * norm_z;
            }
        }
        _ => {}
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Camera3D {
    pub eye: [f64; 3],
    #[allow(dead_code)]
    pub target: [f64; 3],
    pub roll: f64,
    #[allow(dead_code)]
    pub fov: f64,
    pub focal_length: f64,
    pub forward: [f64; 3],
    pub right_prime: [f64; 3],
    pub up_prime: [f64; 3],
}

impl Camera3D {
    pub fn from_params(p: &CameraParams, fov: f64) -> Self {
        let dx = p.tx - p.x;
        let dy = p.ty - p.y;
        let dz = p.tz - p.z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let forward = if dist < 1e-6 {
            [0.0, 0.0, 1.0]
        } else {
            [dx / dist, dy / dist, dz / dist]
        };

        // 上方向ベクトル
        let up_norm = (p.ux * p.ux + p.uy * p.uy + p.uz * p.uz).sqrt();
        let cam_up = if up_norm < 1e-6 {
            if forward[1].abs() > 0.999 {
                [0.0, 0.0, 1.0]
            } else {
                [0.0, -1.0, 0.0]
            }
        } else {
            [p.ux / up_norm, p.uy / up_norm, p.uz / up_norm]
        };

        // right = cross(forward, cam_up)
        let rx = forward[1] * cam_up[2] - forward[2] * cam_up[1];
        let ry = forward[2] * cam_up[0] - forward[0] * cam_up[2];
        let rz = forward[0] * cam_up[1] - forward[1] * cam_up[0];
        let r_len = (rx * rx + ry * ry + rz * rz).sqrt();
        let right = if r_len < 1e-6 {
            [1.0, 0.0, 0.0]
        } else {
            [rx / r_len, ry / r_len, rz / r_len]
        };

        // up = cross(right, forward)
        let ux = right[1] * forward[2] - right[2] * forward[1];
        let uy = right[2] * forward[0] - right[0] * forward[2];
        let uz = right[0] * forward[1] - right[1] * forward[0];
        let up = [ux, uy, uz];

        // ロール回転（傾き）
        let roll_rad = p.rz.to_radians();
        let cos_r = roll_rad.cos();
        let sin_r = roll_rad.sin();
        let right_prime = [
            right[0] * cos_r + up[0] * sin_r,
            right[1] * cos_r + up[1] * sin_r,
            right[2] * cos_r + up[2] * sin_r,
        ];
        let up_prime = [
            -right[0] * sin_r + up[0] * cos_r,
            -right[1] * sin_r + up[1] * cos_r,
            -right[2] * sin_r + up[2] * cos_r,
        ];

        Self {
            eye: [p.x, p.y, p.z],
            target: [p.tx, p.ty, p.tz],
            roll: p.rz,
            fov,
            focal_length: p.d,
            forward,
            right_prime,
            up_prime,
        }
    }

    #[allow(dead_code)]
    pub fn new(eye: [f64; 3], target: [f64; 3], roll: f64, fov: f64) -> Self {
        let dx = target[0] - eye[0];
        let dy = target[1] - eye[1];
        let dz = target[2] - eye[2];
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let forward = if dist < 1e-6 {
            [0.0, 0.0, 1.0]
        } else {
            [dx / dist, dy / dist, dz / dist]
        };

        // ワールドの上方向：Y軸下向きが画面下なので、上方向は [0.0, -1.0, 0.0]
        let world_up = if forward[1].abs() > 0.999 {
            [0.0, 0.0, 1.0]
        } else {
            [0.0, -1.0, 0.0]
        };

        // right = cross(forward, world_up)
        let rx = forward[1] * world_up[2] - forward[2] * world_up[1];
        let ry = forward[2] * world_up[0] - forward[0] * world_up[2];
        let rz = forward[0] * world_up[1] - forward[1] * world_up[0];
        let r_len = (rx * rx + ry * ry + rz * rz).sqrt();
        let right = if r_len < 1e-6 {
            [1.0, 0.0, 0.0]
        } else {
            [rx / r_len, ry / r_len, rz / r_len]
        };

        // up = cross(right, forward)
        let ux = right[1] * forward[2] - right[2] * forward[1];
        let uy = right[2] * forward[0] - right[0] * forward[2];
        let uz = right[0] * forward[1] - right[1] * forward[0];
        let up = [ux, uy, uz];

        // ロール回転（傾き）
        let roll_rad = roll.to_radians();
        let cos_r = roll_rad.cos();
        let sin_r = roll_rad.sin();
        let right_prime = [
            right[0] * cos_r + up[0] * sin_r,
            right[1] * cos_r + up[1] * sin_r,
            right[2] * cos_r + up[2] * sin_r,
        ];
        let up_prime = [
            -right[0] * sin_r + up[0] * cos_r,
            -right[1] * sin_r + up[1] * cos_r,
            -right[2] * sin_r + up[2] * cos_r,
        ];

        // 焦点距離（AviUtl基準: 視野角0時は焦点距離1024.0）
        let focal_length = if fov > 0.1 {
            (1024.0 / 2.0) / (fov.to_radians() / 2.0).tan()
        } else {
            1024.0
        };

        Self {
            eye,
            target,
            roll,
            fov,
            focal_length,
            forward,
            right_prime,
            up_prime,
        }
    }

    /// 3D ワールド座標 [x, y, z] をカメラ座標系に変換し、透視投影した結果 (screen_x, screen_y, scale_3d, z_cam) を返す
    pub fn project(&self, p: [f64; 3]) -> Option<(f32, f32, f32, f64)> {
        let vx = p[0] - self.eye[0];
        let vy = p[1] - self.eye[1];
        let vz = p[2] - self.eye[2];

        let x_cam = vx * self.right_prime[0] + vy * self.right_prime[1] + vz * self.right_prime[2];
        let y_cam = -(vx * self.up_prime[0] + vy * self.up_prime[1] + vz * self.up_prime[2]);
        let z_cam = vx * self.forward[0] + vy * self.forward[1] + vz * self.forward[2];

        if z_cam <= 1.0 {
            // カメラの背面または近すぎるクリップ面
            return None;
        }

        let scale_3d = (self.focal_length / z_cam) as f32;
        let x_proj = (x_cam as f32) * scale_3d;
        let y_proj = (y_cam as f32) * scale_3d;

        Some((x_proj, y_proj, scale_3d, z_cam))
    }
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
    has_ffmpeg: bool,
    camera: Option<Camera3D>,
}

#[derive(Default, Clone, Copy)]
struct FilterVisuals {
    glow_strength: f32,
    glow_radius: f32,
    glow_color: Option<Color32>,
    chroma_shift: f32,
    chroma_angle: f32,
    border_width: f32,
    border_color: Option<Color32>,
    shadow_offset: Option<egui::Vec2>,
    shadow_alpha: f32,
    tint_color: Option<(Color32, f32)>,
    blend_mode: i32,
}

fn rotate_3d(p: [f64; 3], rx_deg: f64, ry_deg: f64, rz_deg: f64) -> [f64; 3] {
    let rx = rx_deg.to_radians();
    let ry = ry_deg.to_radians();
    let rz = rz_deg.to_radians();

    // 1. X軸回転
    let cos_x = rx.cos();
    let sin_x = rx.sin();
    let x1 = p[0];
    let y1 = p[1] * cos_x - p[2] * sin_x;
    let z1 = p[1] * sin_x + p[2] * cos_x;

    // 2. Y軸回転
    let cos_y = ry.cos();
    let sin_y = ry.sin();
    let x2 = x1 * cos_y + z1 * sin_y;
    let y2 = y1;
    let z2 = -x1 * sin_y + z1 * cos_y;

    // 3. Z軸回転
    let cos_z = rz.cos();
    let sin_z = rz.sin();
    let x3 = x2 * cos_z - y2 * sin_z;
    let y3 = x2 * sin_z + y2 * cos_z;
    let z3 = z2;

    [x3, y3, z3]
}

fn closed_stroke(painter: &egui::Painter, pts: &[Pos2], stroke: Stroke) {
    if pts.len() < 2 {
        return;
    }
    let n = pts.len();
    for i in 0..n {
        painter.line_segment([pts[i], pts[(i + 1) % n]], stroke);
    }
}

fn project_3d_points(
    cam: &Camera3D,
    canvas: Rect,
    scale: f32,
    world_pos: [f64; 3],
    rot: [f64; 3],
    local_pts: &[[f32; 2]],
    zoom: f32,
) -> Option<Vec<Pos2>> {
    let mut out = Vec::with_capacity(local_pts.len());
    for &[lx, ly] in local_pts {
        let p_rot = rotate_3d([(lx * zoom) as f64, (ly * zoom) as f64, 0.0], rot[0], rot[1], rot[2]);
        let pw = [world_pos[0] + p_rot[0], world_pos[1] + p_rot[1], world_pos[2] + p_rot[2]];
        if let Some((sx, sy, _, _)) = cam.project(pw) {
            out.push(canvas.center() + vec2(sx, sy) * scale);
        } else {
            return None;
        }
    }
    Some(out)
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
    let base_rx = ev("X軸回転", 0.0);
    let base_ry = ev("Y軸回転", 0.0);
    let base_rz = if base_rot != 0.0 { base_rot } else { ev("Z軸回転", 0.0) };

    let time_sec = rel as f64 / ctx.fps.max(1.0);
    let total_sec = obj.length() as f64 / ctx.fps.max(1.0);

    // Lua スクリプト実行ステートの初期化
    let mut lua_st = LuaObjState {
        x: base_x,
        y: base_y,
        z: base_z,
        rz: base_rz,
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
    if let Some(b) = draw.and_then(|d| d.text("blend")).and_then(|b| b.parse::<i32>().ok()) {
        fx.blend_mode = b;
    }

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

        // 1) プラグインレジストリ内の Lua スクリプト (.anm/.obj/.cam/.scn) または外部プラグインを実行
        let is_builtin_filter = matches!(eff.name.as_str(), "フェード" | "振動" | "縁取り" | "シャドー" | "ドロップシャドウ" | "単色化") && eff.text("name").is_none();
        let ran_lua = if is_builtin_filter {
            false
        } else {
            ctx.plugins.execute_effect(eff, rel, &mut lua_st)
        };

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
            let diffusion = eff.eval("拡散", rel, 30.0) as f32;
            fx.glow_strength = (fx.glow_strength + strength / 100.0).min(6.0);
            fx.glow_radius = (fx.glow_radius + diffusion).max(10.0);
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
            let script_name = eff.text("name").unwrap_or_default();
            let sec = script_name.split('@').next().unwrap_or(script_name);
            match sec {
                "揺れ" | "振動" | "震える" => {
                    let ax = eff.eval("X振幅", rel, eff.eval("track0", rel, 25.0));
                    let ay = eff.eval("Y振幅", rel, eff.eval("track0", rel, 15.0));
                    let interval = eff.eval("間隔", rel, eff.eval("track2", rel, 0.0));
                    let angle = eff.eval("角度", rel, eff.eval("track1", rel, 0.0));
                    if interval <= 0.0 {
                        lua_st.ox += aviutl_rand(-ax, ax, 0, (time_sec * 100.0) as i64);
                        lua_st.oy += aviutl_rand(-ay, ay, 1, (time_sec * 100.0) as i64);
                        if angle != 0.0 {
                            lua_st.rz += aviutl_rand(-angle, angle, 2, (time_sec * 100.0) as i64);
                        }
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
                "振り子" => {
                    let speed = eff.eval("速さ", rel, eff.eval("track0", rel, 2.0));
                    let angle = eff.eval("角度", rel, eff.eval("track1", rel, 30.0));
                    lua_st.rz += (time_sec * speed * std::f64::consts::PI).sin() * angle;
                }
                "円形配置" => {
                    let circumference = eff.eval("円周", rel, eff.eval("track0", rel, 100.0));
                    let radius = eff.eval("半径", rel, eff.eval("track1", rel, 100.0));
                    let rot = eff.eval("回転", rel, eff.eval("track2", rel, 0.0));
                    let shift = eff.eval("ずらし", rel, eff.eval("track3", rel, 0.0));
                    let num = eff.eval("数", rel, 8.0).round().max(1.0) as usize;
                    for i in 1..=num {
                        let r = ((i as f64 - (num as f64 + 1.0) / 2.0) * circumference / 50.0 / (num as f64) + rot / 180.0) * std::f64::consts::PI;
                        let x = r.sin() * radius;
                        let y = -r.cos() * radius;
                        let deg = r.to_degrees();
                        let z = shift * deg / 360.0;
                        lua_st.draw_calls.push(au2_plugin::LuaDrawCall {
                            ox: x,
                            oy: y,
                            oz: z,
                            zoom: 1.0,
                            alpha: 1.0,
                            rx: 0.0,
                            ry: 0.0,
                            rz: deg,
                            color: None,
                            primitive: None,
                        });
                    }
                }
                "点滅" => {
                    let period = eff.eval("周期", rel, eff.eval("track0", rel, 0.5)).max(0.01);
                    let min_t = (eff.eval("最小透明度", rel, eff.eval("track1", rel, 60.0)) / 100.0).clamp(0.0, 1.0);
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
                let d = fcall.params.get("拡散").map(|v| v.as_num() as f32).unwrap_or(30.0);
                fx.glow_strength = (fx.glow_strength + s / 100.0).min(6.0);
                fx.glow_radius = (fx.glow_radius + d).max(10.0);
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

    let raw_x = base_x + lua_st.ox - lua_st.cx;
    let raw_y = base_y + lua_st.oy - lua_st.cy;
    let raw_z = base_z + lua_st.oz - lua_st.cz;
    let raw_zoom = (base_zoom * lua_st.zoom).max(0.0);
    let raw_rz = lua_st.rz;

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
                return;
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
    for poly in &lua_st.poly_calls {
        let pts: Vec<Pos2> = poly
            .pts
            .iter()
            .map(|&(px, py, pz)| {
                if obj.camera_target {
                    if let Some(ref cam) = ctx.camera {
                        if let Some((proj_x, proj_y, _, _)) = cam.project([raw_x + px, raw_y + py, raw_z + pz]) {
                            return ctx.canvas.center() + vec2(proj_x, proj_y) * ctx.scale;
                        }
                    }
                }
                ctx.canvas.center() + vec2(x + px as f32, y + py as f32) * ctx.scale
            })
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
            let inst_raw_x = raw_x + dc.ox;
            let inst_raw_y = raw_y + dc.oy;
            let inst_raw_z = raw_z + dc.oz;
            let inst_raw_zoom = (base_zoom * dc.zoom).max(0.0);
            let inst_raw_rz = dc.rz;

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

            let inst_alpha = dc.alpha.clamp(0.0, 1.0) as f32;
            let prim = dc.primitive.as_ref().or(lua_st.loaded_primitive.as_ref());
            let inst_transform_3d = if obj.camera_target {
                Some(([inst_raw_x, inst_raw_y, inst_raw_z], [base_rx, base_ry, inst_raw_rz]))
            } else {
                None
            };
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
                inst_transform_3d,
                selected,
            );
        }
        if selected {
            if let Some(b) = last_bounds {
                ctx.painter.rect_stroke(b.expand(2.0), 0.0, Stroke::new(1.0_f32, Color32::from_rgb(255, 200, 0)));
            }
        }
        return;
    }

    let transform_3d = if obj.camera_target {
        Some(([raw_x, raw_y, raw_z], [base_rx, base_ry, raw_rz]))
    } else {
        None
    };

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
        selected,
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
    transform_3d: Option<([f64; 3], [f64; 3])>,
    selected: bool,
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
        // グローまたは加算合成（1, 11, 12）時の発光ハイライト強調
        let is_glow_or_add = fx.glow_strength > 0.01 || fx.blend_mode == 1 || fx.blend_mode == 11 || fx.blend_mode == 12;
        let final_base = if is_glow_or_add {
            let boost = (fx.glow_strength * 0.25 + if fx.blend_mode == 1 { 0.4 } else { 0.18 }).min(0.7);
            Color32::from_rgba_unmultiplied(
                (base.r() as f32 * (1.0 - boost) + 255.0 * boost).min(255.0) as u8,
                (base.g() as f32 * (1.0 - boost) + 255.0 * boost).min(255.0) as u8,
                (base.b() as f32 * (1.0 - boost) + 255.0 * boost).min(255.0) as u8,
                base.a(),
            )
        } else {
            base
        };
        final_base.gamma_multiply(alpha)
    };

    // ドロップシャドウの背面描画
    if let Some(sh_off) = fx.shadow_offset {
        let sh_center = center + sh_off * ctx.scale;
        let sh_col = Color32::from_black_alpha((fx.shadow_alpha * alpha * 180.0) as u8);
        ctx.painter.circle_filled(sh_center, 45.0 * k, sh_col);
    }

    // グロー (32bit .auf / 内蔵グロー) の背面ハロー描画（多層拡散ブルーム）
    if fx.glow_strength > 0.01 {
        let diff_r = fx.glow_radius * k;
        let halo_r = 75.0 * k * (1.0 + fx.glow_strength * 0.25) + diff_r;
        let glow_alpha = (fx.glow_strength * alpha * 45.0).clamp(10.0, 160.0) as u8;
        let glow_col = fx.glow_color.unwrap_or(Color32::from_rgba_unmultiplied(255, 235, 170, glow_alpha));
        ctx.painter.circle_filled(center, halo_r, glow_col.gamma_multiply(0.35));
        ctx.painter.circle_filled(center, halo_r * 0.6, glow_col.gamma_multiply(0.65));
        ctx.painter.circle_filled(center, halo_r * 0.3, Color32::from_white_alpha((fx.glow_strength * alpha * 35.0).min(180.0) as u8));
    }

    // カスタムオブジェクト (.obj) 等で `obj.load("figure", ...)` や `obj.mes(...)` が実行された場合
    if let Some(prim) = loaded_prim {
        match prim {
            LuaLoadedPrimitive::Figure { fig_type, color, size, line_width } => {
                let sz = (*size as f32 * k).max(1.0);
                let col = apply_tint(u32_to_color32(*color));
                let is_stroke = *line_width > 0.0 && *line_width < *size && *line_width < 3999.0;
                let stroke_w = (*line_width as f32 * k).max(1.0);
                let stroke = Stroke::new(stroke_w, col);

                if fx.border_width > 0.0 {
                    let bcol = fx.border_color.unwrap_or(Color32::WHITE).gamma_multiply(alpha);
                    ctx.painter.circle_filled(center, sz / 2.0 + fx.border_width * ctx.scale, bcol);
                }
                match fig_type.as_str() {
                    "四角形" => {
                        let pts = polygon(center, sz * std::f32::consts::FRAC_1_SQRT_2, 4, angle + PI / 4.0);
                        if is_stroke {
                            closed_stroke(ctx.painter, &pts, stroke);
                        } else {
                            fan(ctx.painter, center, &pts, col);
                        }
                    }
                    "三角形" => {
                        let pts = polygon(center, sz / 2.0, 3, angle);
                        if is_stroke {
                            closed_stroke(ctx.painter, &pts, stroke);
                        } else {
                            fan(ctx.painter, center, &pts, col);
                        }
                    }
                    "五角形" => {
                        let pts = polygon(center, sz / 2.0, 5, angle);
                        if is_stroke {
                            closed_stroke(ctx.painter, &pts, stroke);
                        } else {
                            fan(ctx.painter, center, &pts, col);
                        }
                    }
                    "六角形" => {
                        let pts = polygon(center, sz / 2.0, 6, angle);
                        if is_stroke {
                            closed_stroke(ctx.painter, &pts, stroke);
                        } else {
                            fan(ctx.painter, center, &pts, col);
                        }
                    }
                    "星型" => {
                        let pts = star(center, sz / 2.0, angle);
                        if is_stroke {
                            closed_stroke(ctx.painter, &pts, stroke);
                        } else {
                            fan(ctx.painter, center, &pts, col);
                        }
                    }
                    _ => {
                        if is_stroke {
                            ctx.painter.circle_stroke(center, (sz - stroke_w).max(0.0) / 2.0, stroke);
                        } else {
                            ctx.painter.circle_filled(center, sz / 2.0, col);
                        }
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
            let raw_size = src.eval("サイズ", rel, 100.0) as f32;
            let size = (raw_size * k).max(1.0);
            let line_width = src.eval("ライン幅", rel, 0.0) as f32;
            let aspect = src.eval("縦横比", rel, 0.0) as f32 / 100.0;
            let (w_mult, h_mult) = if aspect >= 0.0 { (1.0 + aspect, 1.0) } else { (1.0, 1.0 - aspect) };
            let color = apply_tint(src.text("color").and_then(parse_color).unwrap_or(Color32::WHITE));
            let ty: u32 = src.text("type").and_then(|t| t.trim().parse().ok()).unwrap_or(1);
            let is_stroke = line_width > 0.0 && line_width < raw_size && line_width < 3999.0;
            let stroke_w = (line_width * k).max(1.0);
            let stroke = Stroke::new(stroke_w, color);

            if fx.border_width > 0.0 && ty != 0 {
                let bcol = fx.border_color.unwrap_or(Color32::WHITE).gamma_multiply(alpha);
                ctx.painter.circle_filled(center, size / 2.0 + fx.border_width * ctx.scale, bcol);
            }
            if fx.chroma_shift > 0.5 && ty != 0 {
                let shift_vec = vec2(fx.chroma_angle.cos(), fx.chroma_angle.sin()) * fx.chroma_shift * ctx.scale;
                ctx.painter.circle_filled(center - shift_vec, size / 2.0, Color32::from_rgba_unmultiplied(255, 40, 40, (alpha * 130.0) as u8));
                ctx.painter.circle_filled(center + shift_vec, size / 2.0, Color32::from_rgba_unmultiplied(40, 180, 255, (alpha * 130.0) as u8));
            }

            // ローカル頂点列を生成（縦横比を反映）
            let local_pts: Vec<[f32; 2]> = match ty {
                0 => {
                    // 背景
                    let hw = ctx.screen_w as f32 / 2.0;
                    let hh = ctx.screen_h as f32 / 2.0;
                    vec![[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]]
                }
                1 => {
                    // 円 (24分割の多角形近似で3D透視投影にも美しく対応)
                    let rx = (raw_size * w_mult) / 2.0;
                    let ry = (raw_size * h_mult) / 2.0;
                    (0..24).map(|i| {
                        let a = TAU * (i as f32) / 24.0;
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

            // 頂点をスクリーン座標に変換（3Dカメラ投影 or 2D平面変換）
            let screen_pts: Option<Vec<Pos2>> = if let (Some(cam), Some((w_pos, rot))) = (ctx.camera, transform_3d) {
                project_3d_points(&cam, ctx.canvas, ctx.scale, w_pos, rot, &local_pts, zoom / ctx.scale)
            } else {
                let rot = Rot2::from_angle(angle);
                Some(local_pts.iter().map(|&[lx, ly]| center + rot * vec2(lx * k, ly * k)).collect())
            };

            let Some(pts) = screen_pts else {
                return None;
            };

            if is_stroke && ty != 0 {
                closed_stroke(ctx.painter, &pts, stroke);
            } else {
                fan(ctx.painter, center, &pts, color);
            }

            Some(Rect::from_center_size(center, vec2(size * w_mult, size * h_mult)))
        }
        EffectKind::Text => {
            // epaint のフォントアトラスは巨大フォント（>100pt）でアトラス境界違反・クラッシュを起こすため、
            // フォントのラスタライズサイズは最大 96.0 に抑え、それ以上の超巨大テキストはメッシュ頂点スケーリングで安全に描画する
            let max_font_size = 96.0_f32;
            let raw_size = (src.eval("サイズ", rel, 34.0) as f32 * k).max(1.0);
            let (font_size, scale_up) = if raw_size > max_font_size {
                (max_font_size, (raw_size / max_font_size).min(100.0))
            } else {
                (raw_size, 1.0)
            };
            let color = apply_tint(src.text("color").and_then(parse_color).unwrap_or(Color32::WHITE));
            let text = src.text("text").unwrap_or_default().replace("\r\n", "\n");

            let render_safe_text = |painter: &egui::Painter, target_center: Pos2, galley: std::sync::Arc<egui::epaint::Galley>, col: Color32, text_angle: f32| {
                let unscaled_half = galley.size() / 2.0;
                let unscaled_pos = target_center - Rot2::from_angle(text_angle) * unscaled_half;
                let shape = egui::epaint::TextShape::new(unscaled_pos, galley, col).with_angle(text_angle);
                if scale_up > 1.001 {
                    let clipped = egui::epaint::ClippedShape {
                        clip_rect: egui::Rect::EVERYTHING,
                        shape: egui::Shape::Text(shape),
                    };
                    let primitives = painter.ctx().tessellate(vec![clipped], painter.ctx().pixels_per_point());
                    for prim in primitives {
                        if let egui::epaint::Primitive::Mesh(mut mesh) = prim.primitive {
                            for v in &mut mesh.vertices {
                                v.pos = target_center + (v.pos - target_center) * scale_up;
                            }
                            painter.add(egui::Shape::Mesh(mesh));
                        }
                    }
                } else {
                    painter.add(shape);
                }
            };

            if fx.chroma_shift > 0.5 {
                let shift_vec = vec2(fx.chroma_angle.cos(), fx.chroma_angle.sin()) * fx.chroma_shift * ctx.scale;
                let r_col = Color32::from_rgba_unmultiplied(255, 50, 50, (alpha * 150.0) as u8);
                let b_col = Color32::from_rgba_unmultiplied(50, 180, 255, (alpha * 150.0) as u8);
                let g1 = ctx.painter.layout(text.clone(), FontId::proportional(font_size), r_col, f32::INFINITY);
                let g2 = ctx.painter.layout(text.clone(), FontId::proportional(font_size), b_col, f32::INFINITY);
                render_safe_text(ctx.painter, center - shift_vec, g1, r_col, angle);
                render_safe_text(ctx.painter, center + shift_vec, g2, b_col, angle);
            }
            let galley = ctx.painter.layout(text, FontId::proportional(font_size), color, f32::INFINITY);
            let scaled_size = galley.size() * scale_up;
            let r = Rect::from_center_size(center, scaled_size);
            render_safe_text(ctx.painter, center, galley, color, angle);
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
                let placeholder_col = apply_tint(Color32::from_rgba_unmultiplied(40, 50, 70, 35));
                fan(ctx.painter, center, &corners, placeholder_col);

                let is_video = matches!(src.kind, EffectKind::VideoFile);
                if is_video && !ctx.has_ffmpeg {
                    // FFmpeg が見つからない場合は選択中かどうかにかかわらず親切に案内
                    ctx.painter.text(
                        center,
                        Align2::CENTER_CENTER,
                        format!("{}\n【動画デコーダー (FFmpeg) 未検出】\n画面上部の「FFmpeg を自動ダウンロード」を押してください", obj.display_name()),
                        FontId::proportional((13.0 * zoom.max(0.5)).min(22.0)),
                        Color32::from_rgb(255, 140, 140),
                    );
                } else if selected {
                    ctx.painter.text(
                        center,
                        Align2::CENTER_CENTER,
                        format!("{}\n{}", src.name, obj.display_name()),
                        FontId::proportional((14.0 * zoom.max(0.5)).min(28.0)),
                        apply_tint(Color32::LIGHT_GRAY),
                    );
                }
                Some(r)
            }
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
            fan(ctx.painter, center, &corners, col);
            if selected {
                ctx.painter.text(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aviutl_camera_effects_math() {
        let mut cam = CameraParams {
            x: 0.0,
            y: 0.0,
            z: -1024.0,
            tx: 0.0,
            ty: 0.0,
            tz: 0.0,
            rz: 0.0,
            ux: 0.0,
            uy: -1.0,
            uz: 0.0,
            d: 1024.0,
        };

        // 1. 手ぶれテスト
        apply_camera_effect(&mut cam, "手ぶれ", 50.0, 10.0, 20.0, 0.0, 0.5, 720.0);
        assert!(cam.tx.abs() > 0.0 || cam.ty.abs() > 0.0 || cam.tz.abs() > 0.0, "手ぶれで注視点座標が変化すること");
        assert!(cam.rz.abs() > 0.0, "手ぶれでロール角が変化すること");

        // 2. 目標中心回転テスト (水平90度回転)
        let mut rot_cam = CameraParams {
            x: 0.0,
            y: 0.0,
            z: -1000.0,
            tx: 0.0,
            ty: 0.0,
            tz: 0.0,
            rz: 0.0,
            ux: 0.0,
            uy: -1.0,
            uz: 0.0,
            d: 1024.0,
        };
        apply_camera_effect(&mut rot_cam, "目標中心回転", 90.0, 0.0, 0.0, 0.0, 0.0, 720.0);
        assert!((rot_cam.x.abs() - 1000.0).abs() < 1.0, "水平90度回転でX座標が±1000近傍になること: got {}", rot_cam.x);
        assert!(rot_cam.z.abs() < 1.0, "水平90度回転でZ座標が0近傍になること: got {}", rot_cam.z);

        // 3. 目標サイズ固定視野角テスト (ドリーズーム)
        let mut fov_cam = CameraParams {
            x: 0.0,
            y: 0.0,
            z: -1000.0,
            tx: 0.0,
            ty: 0.0,
            tz: 0.0,
            rz: 0.0,
            ux: 0.0,
            uy: -1.0,
            uz: 0.0,
            d: 1000.0,
        };
        apply_camera_effect(&mut fov_cam, "目標サイズ固定視野角", 20.0, 0.0, 0.0, 0.0, 0.0, 720.0);
        assert!((fov_cam.z - (-1000.0)).abs() > 10.0, "視野角変更に伴いカメラ距離が再計算されること: got {}", fov_cam.z);
    }

    #[test]
    fn test_aviutl_rand_and_interpolation() {
        let r1 = aviutl_rand(-10.0, 10.0, 1, 100);
        let r2 = aviutl_rand(-10.0, 10.0, 1, 100);
        assert_eq!(r1, r2, "同一シード・フレームで決定論的に同じ乱数が得られること");
        assert!(r1 >= -10.0 && r1 <= 10.0, "範囲内であること");

        // Catmull-Rom スプライン補間: t=0 で p1, t=1 で p2 を通ること
        let val_0 = aviutl_interpolation(0.0, 10.0, 20.0, 30.0, 40.0);
        let val_1 = aviutl_interpolation(1.0, 10.0, 20.0, 30.0, 40.0);
        assert!((val_0 - 20.0).abs() < 1e-6, "t=0 で p1 (20.0) を通ること: got {}", val_0);
        assert!((val_1 - 30.0).abs() < 1e-6, "t=1 で p2 (30.0) を通ること: got {}", val_1);
    }
}


