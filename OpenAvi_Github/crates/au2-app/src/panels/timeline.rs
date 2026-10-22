//! タイムライン（拡張編集ウィンドウ）。
//!
//! 1 フレーム単位で操作できるよう、すべてのヒットテスト・スナップはフレーム整数に丸めて行います。
//! 操作：
//! - ルーラー / 空き領域のクリック・ドラッグ … 再生位置の移動
//! - オブジェクトのクリック … 選択（設定ダイアログに表示）
//! - オブジェクトのドラッグ … 移動（レイヤー間移動含む、重なる位置には移動しない）
//! - **右クリック（空きレーン / オブジェクト上）** … **拡張編集コンテキストメニュー**
//!   - メディアオブジェクトの追加（動画・画像・音声・テキスト・図形・フレームバッファ・グループ制御・カメラ制御）
//!   - フィルタオブジェクトの追加（色調補正）
//!   - 選択オブジェクトへの操作（中間点を追加・分割・削除）
//!   - レイヤーの表示 / 非表示・ロック切替
//! - レイヤー名のクリック … 表示 / 非表示の切り替え
//! - ホイール … 縦スクロール、Shift+ホイール … 横スクロール、Ctrl+ホイール … 拡大縮小

use au2_core::{Frame, ObjectCategory, ObjectId, Rational, TimelineObject, MAX_LAYERS};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Sense, Shape, Stroke};

use crate::state::{ContextTarget, EditorState, TimelineDrag};

const HEADER_W: f32 = 84.0;
const RULER_H: f32 = 22.0;
const LAYER_H: f32 = 24.0;

fn category_color(c: ObjectCategory) -> Color32 {
    match c {
        ObjectCategory::Media => Color32::from_rgb(72, 112, 196),
        ObjectCategory::Audio => Color32::from_rgb(180, 70, 80),
        ObjectCategory::Filter => Color32::from_rgb(70, 150, 90),
        ObjectCategory::Control => Color32::from_rgb(190, 150, 60),
    }
}

fn fmt_time(frame: Frame, rate: Rational) -> String {
    let s = rate.frame_to_seconds(frame);
    format!("{:02}:{:05.2}", (s / 60.0).floor() as u64, s % 60.0)
}

pub fn ui(ui: &mut egui::Ui, st: &mut EditorState) {
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let full = resp.rect;
    let ruler = Rect::from_min_max(pos2(full.left() + HEADER_W, full.top()), pos2(full.right(), full.top() + RULER_H));
    let headers = Rect::from_min_max(pos2(full.left(), full.top() + RULER_H), pos2(full.left() + HEADER_W, full.bottom()));
    let lanes = Rect::from_min_max(pos2(full.left() + HEADER_W, full.top() + RULER_H), full.max);

    // ---- スクロール / ズーム ----
    if resp.hovered() {
        let (zoom, scroll, shift) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.modifiers.shift));
        let v = &mut st.timeline;
        if zoom != 1.0 {
            if let Some(p) = resp.hover_pos() {
                let anchor = (p.x - lanes.left()) / v.px_per_frame + v.scroll_x;
                v.px_per_frame = (v.px_per_frame * zoom).clamp(0.05, 48.0);
                v.scroll_x = anchor - (p.x - lanes.left()) / v.px_per_frame;
            }
        } else {
            let (dx, dy) = if shift && scroll.x == 0.0 { (scroll.y, 0.0) } else { (scroll.x, scroll.y) };
            v.scroll_x -= dx / v.px_per_frame;
            v.scroll_y -= dy;
        }
    }
    {
        let v = &mut st.timeline;
        let max_y = (MAX_LAYERS as f32 * LAYER_H - lanes.height()).max(0.0);
        v.scroll_y = v.scroll_y.clamp(0.0, max_y);
        v.scroll_x = v.scroll_x.max(0.0);
    }

    // 再生中は再生位置を追従
    if st.playing {
        let v = &mut st.timeline;
        let px = lanes.left() + (st.frame as f32 - v.scroll_x) * v.px_per_frame;
        if px > lanes.right() - 16.0 || px < lanes.left() {
            v.scroll_x = (st.frame as f32 - 8.0 / v.px_per_frame).max(0.0);
        }
    }

    let ppf = st.timeline.px_per_frame;
    let scroll_x = st.timeline.scroll_x;
    let scroll_y = st.timeline.scroll_y;
    let fx = |f: f32| lanes.left() + (f - scroll_x) * ppf;
    let xf = |x: f32| ((x - lanes.left()) / ppf + scroll_x).max(0.0) as Frame;
    let row_top = |li: usize| lanes.top() + li as f32 * LAYER_H - scroll_y;
    let y_layer = |y: f32| (((y - lanes.top() + scroll_y) / LAYER_H).floor().max(0.0) as usize).min(MAX_LAYERS - 1);

    let hit = |st: &EditorState, p: egui::Pos2| -> Option<(ObjectId, Frame)> {
        if !lanes.contains(p) {
            return None;
        }
        let f = xf(p.x);
        st.scene.layers.get(y_layer(p.y))?.objects.iter().find(|o| o.contains(f)).map(|o| (o.id, o.frame_start))
    };

    // ---- 右クリック位置の記録 ----
    if resp.secondary_clicked() || ui.input(|i| i.pointer.secondary_pressed()) {
        if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
            if full.contains(p) {
                let layer = y_layer(p.y);
                let frame = if p.x >= lanes.left() { xf(p.x) } else { st.frame };
                let obj_id = hit(st, p).map(|(id, _)| id);
                if let Some(id) = obj_id {
                    st.selected = Some(id);
                }
                if p.x >= lanes.left() {
                    st.set_frame(frame);
                }
                st.timeline.context_target = ContextTarget {
                    layer,
                    frame,
                    object_id: obj_id,
                };
            }
        }
    }

    // ---- 左ドラッグ・クリック操作 ----
    if resp.drag_started_by(egui::PointerButton::Primary) {
        if let Some(p) = ui.input(|i| i.pointer.press_origin()) {
            st.timeline.drag = if ruler.contains(p) {
                Some(TimelineDrag::Scrub)
            } else if let Some((id, start)) = hit(st, p) {
                st.selected = Some(id);
                Some(TimelineDrag::Move { id, grab_offset: xf(p.x) as i64 - start as i64 })
            } else if lanes.contains(p) {
                Some(TimelineDrag::Scrub)
            } else {
                None
            };
        }
    }
    if resp.dragged_by(egui::PointerButton::Primary) {
        if let Some(p) = resp.interact_pointer_pos() {
            match st.timeline.drag {
                Some(TimelineDrag::Scrub) => st.set_frame(xf(p.x)),
                Some(TimelineDrag::Move { id, grab_offset }) => {
                    let new_start = (xf(p.x) as i64 - grab_offset).max(0) as Frame;
                    let new_layer = y_layer(p.y);
                    if let Some((cur_layer, obj)) = st.scene.find(id) {
                        if (cur_layer, obj.frame_start) != (new_layer, new_start)
                            && st.scene.move_object(id, new_layer, new_start)
                        {
                            st.dirty = true;
                        }
                    }
                }
                None => {}
            }
        }
    }
    if resp.drag_stopped() {
        st.timeline.drag = None;
    }
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            if headers.contains(p) {
                let l = &mut st.scene.layers[y_layer(p.y)];
                l.visible = !l.visible;
            } else if ruler.contains(p) {
                st.set_frame(xf(p.x));
            } else if let Some((id, _)) = hit(st, p) {
                st.selected = Some(id);
            } else if lanes.contains(p) {
                st.selected = None;
                st.set_frame(xf(p.x));
            }
        }
    }

    // ---- 右クリック コンテキストメニュー（AviUtl 拡張編集スタイル） ----
    resp.context_menu(|ui| {
        context_menu_ui(ui, st);
    });

    // ---- 描画 ----
    painter.rect_filled(full, 0.0, Color32::from_gray(28));
    let first_row = (scroll_y / LAYER_H).floor() as usize;
    let rows = (lanes.height() / LAYER_H).ceil() as usize + 1;
    let visible_rows = first_row..(first_row + rows).min(MAX_LAYERS);

    // レーン背景
    let lane_painter = painter.with_clip_rect(lanes);
    for li in visible_rows.clone() {
        let y = row_top(li);
        let r = Rect::from_min_size(pos2(lanes.left(), y), vec2(lanes.width(), LAYER_H));
        let base = if li % 2 == 0 { 34 } else { 38 };
        lane_painter.rect_filled(r, 0.0, Color32::from_gray(base));
    }
    // シーン長より後ろを暗く
    let end_x = fx(st.scene.settings.length as f32);
    if end_x < lanes.right() {
        lane_painter.rect_filled(
            Rect::from_min_max(pos2(end_x.max(lanes.left()), lanes.top()), lanes.max),
            0.0,
            Color32::from_black_alpha(90),
        );
    }

    // オブジェクト
    for li in visible_rows.clone() {
        let layer = &st.scene.layers[li];
        let y = row_top(li);
        for obj in &layer.objects {
            let (x0, x1) = (fx(obj.frame_start as f32), fx(obj.frame_end as f32 + 1.0));
            if x1 < lanes.left() || x0 > lanes.right() {
                continue;
            }
            let r = Rect::from_min_max(pos2(x0, y + 2.0), pos2(x1.max(x0 + 2.0), y + LAYER_H - 2.0));
            let selected = st.selected == Some(obj.id);
            let mut color = category_color(obj.category());
            if selected {
                color = color.linear_multiply(1.35);
            }
            if !layer.visible || obj.source().is_some_and(|s| !s.enabled) {
                color = color.gamma_multiply(0.35);
            }
            lane_painter.rect_filled(r, 3.0, color);
            if selected {
                lane_painter.rect_stroke(r, 3.0, Stroke::new(1.5_f32, Color32::WHITE));
            }
            // 中間点
            for &m in &obj.midpoints {
                let mx = fx((obj.frame_start + m) as f32);
                lane_painter.line_segment([pos2(mx, r.top()), pos2(mx, r.bottom())], Stroke::new(1.0_f32, Color32::from_white_alpha(140)));
                let c = pos2(mx, r.center().y);
                lane_painter.add(Shape::convex_polygon(
                    vec![c + vec2(0.0, -4.0), c + vec2(4.0, 0.0), c + vec2(0.0, 4.0), c + vec2(-4.0, 0.0)],
                    Color32::WHITE,
                    Stroke::NONE,
                ));
            }
            let text_clip = r.shrink2(vec2(3.0, 0.0)).intersect(lanes);
            if text_clip.width() > 8.0 {
                painter.with_clip_rect(text_clip).text(
                    pos2(r.left().max(lanes.left()) + 5.0, r.center().y),
                    Align2::LEFT_CENTER,
                    obj.display_name(),
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );
            }
        }
    }

    // ルーラー
    painter.rect_filled(ruler, 0.0, Color32::from_gray(46));
    let rate = st.scene.settings.frame_rate;
    let fps = rate.as_f64().round().max(1.0) as u32;
    let candidates = [1, 2, 5, 10, fps / 2, fps, fps * 2, fps * 5, fps * 10, fps * 30, fps * 60, fps * 300, fps * 600];
    let step = candidates.into_iter().filter(|&c| c > 0).find(|&c| c as f32 * ppf >= 70.0).unwrap_or(fps * 600);
    let minor = if step % 5 == 0 && (step / 5) as f32 * ppf >= 6.0 { step / 5 } else { step };
    let ruler_painter = painter.with_clip_rect(ruler);
    let first_f = (scroll_x as u32 / minor) * minor;
    let mut f = first_f;
    while fx(f as f32) <= ruler.right() {
        let x = fx(f as f32);
        let major = f % step == 0;
        let h = if major { RULER_H * 0.6 } else { RULER_H * 0.25 };
        ruler_painter.line_segment([pos2(x, ruler.bottom() - h), pos2(x, ruler.bottom())], Stroke::new(1.0_f32, Color32::from_gray(150)));
        if major {
            ruler_painter.text(pos2(x + 3.0, ruler.top() + 1.0), Align2::LEFT_TOP, fmt_time(f, rate), FontId::monospace(10.0), Color32::from_gray(200));
        }
        f += minor;
    }

    // レイヤーヘッダ
    let header_painter = painter.with_clip_rect(headers);
    header_painter.rect_filled(headers, 0.0, Color32::from_gray(42));
    for li in visible_rows {
        let y = row_top(li);
        let r = Rect::from_min_size(pos2(headers.left(), y), vec2(HEADER_W, LAYER_H));
        let layer = &st.scene.layers[li];
        let color = if layer.visible { Color32::from_gray(220) } else { Color32::from_gray(90) };
        let base_name = layer.name.clone().unwrap_or_else(|| format!("Layer {}", li + 1));
        let label = if layer.locked { format!("🔒 {base_name}") } else { base_name };
        header_painter.text(r.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(12.0), color);
        header_painter.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, Color32::from_gray(30)));
    }
    painter.rect_filled(Rect::from_min_max(full.min, pos2(headers.right(), ruler.bottom())), 0.0, Color32::from_gray(50));
    painter.text(
        full.min + vec2(6.0, RULER_H / 2.0),
        Align2::LEFT_CENTER,
        format!("x{:.2}", ppf),
        FontId::monospace(10.0),
        Color32::from_gray(160),
    );

    // 再生位置
    let px = fx(st.frame as f32);
    if px >= lanes.left() && px <= lanes.right() {
        painter.line_segment([pos2(px, ruler.top()), pos2(px, lanes.bottom())], Stroke::new(1.5_f32, Color32::from_rgb(255, 64, 64)));
        painter.add(Shape::convex_polygon(
            vec![pos2(px - 5.0, ruler.top()), pos2(px + 5.0, ruler.top()), pos2(px, ruler.top() + 8.0)],
            Color32::from_rgb(255, 64, 64),
            Stroke::NONE,
        ));
    }
}

/// タイムライン右クリック時のコンテキストメニュー。
fn context_menu_ui(ui: &mut egui::Ui, st: &mut EditorState) {
    let target = st.timeline.context_target;
    let start = target.frame;
    let end = start + 149; // 既定 150 フレーム (30fps で 5 秒)

    ui.label(
        egui::RichText::new(format!("Layer {} / Frame {}", target.layer + 1, start + 1))
            .weak()
            .small(),
    );
    ui.separator();

    // 1. オブジェクト上で右クリックされた場合の操作メニュー
    if let Some(obj_id) = target.object_id {
        if ui.button("◆ 中間点を追加").clicked() {
            if let Some((_, obj)) = st.scene.find_mut(obj_id) {
                if obj.add_midpoint(start) {
                    st.dirty = true;
                    st.status = format!("フレーム {} に中間点を追加しました", start + 1);
                } else {
                    st.status = "オブジェクト端または既存の中間点位置には追加できません".into();
                }
            }
            ui.close_menu();
        }
        if ui.button("✂ ここで分割").clicked() {
            let split_result = st
                .scene
                .find_mut(obj_id)
                .and_then(|(layer_idx, obj)| obj.split_at(start).map(|right| (layer_idx, right)));
            if let Some((layer_idx, right_obj)) = split_result {
                let right_id = right_obj.id;
                st.scene.insert_object(layer_idx, right_obj);
                st.selected = Some(right_id);
                st.dirty = true;
                st.status = format!("フレーム {} でオブジェクトを分割しました", start + 1);
            } else {
                st.status = "オブジェクトの先頭フレームでは分割できません".into();
            }
            ui.close_menu();
        }
        if ui.button("🗑 削除 (Delete)").clicked() {
            if st.scene.remove_object(obj_id).is_some() {
                if st.selected == Some(obj_id) {
                    st.selected = None;
                }
                st.dirty = true;
                st.status = "オブジェクトを削除しました".into();
            }
            ui.close_menu();
        }
        ui.separator();
    }

    // 2. メディアオブジェクトの追加メニュー（AviUtl 拡張編集互換）
    ui.menu_button("メディアオブジェクトの追加", |ui| {
        if ui.button("動画ファイル...").clicked() {
            ui.close_menu();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("動画ファイル", &["mp4", "avi", "mov", "mkv", "webm", "wmv", "gif"])
                .add_filter("すべてのファイル", &["*"])
                .pick_file()
            {
                let obj = TimelineObject::new_video(start, end, path.to_string_lossy());
                st.add_object_at(target.layer, obj);
            } else {
                let obj = TimelineObject::new_video(start, end, "");
                st.add_object_at(target.layer, obj);
            }
        }
        if ui.button("画像ファイル...").clicked() {
            ui.close_menu();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("画像ファイル", &["png", "jpg", "jpeg", "bmp", "webp", "gif"])
                .add_filter("すべてのファイル", &["*"])
                .pick_file()
            {
                let obj = TimelineObject::new_image(start, end, path.to_string_lossy());
                st.add_object_at(target.layer, obj);
            } else {
                let obj = TimelineObject::new_image(start, end, "");
                st.add_object_at(target.layer, obj);
            }
        }
        if ui.button("音声ファイル...").clicked() {
            ui.close_menu();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("音声ファイル", &["wav", "mp3", "ogg", "flac", "aac", "m4a"])
                .add_filter("すべてのファイル", &["*"])
                .pick_file()
            {
                let obj = TimelineObject::new_audio(start, end, path.to_string_lossy());
                st.add_object_at(target.layer, obj);
            } else {
                let obj = TimelineObject::new_audio(start, end, "");
                st.add_object_at(target.layer, obj);
            }
        }
        ui.separator();
        if ui.button("テキスト").clicked() {
            let obj = TimelineObject::new_text(start, end, "サンプルテキスト");
            st.add_object_at(target.layer, obj);
            ui.close_menu();
        }
        ui.menu_button("図形", |ui| {
            if ui.button("円").clicked() {
                st.add_object_at(target.layer, TimelineObject::new_shape(start, end, 1, "e94560"));
                ui.close_menu();
            }
            if ui.button("四角形").clicked() {
                st.add_object_at(target.layer, TimelineObject::new_shape(start, end, 2, "0f9b8e"));
                ui.close_menu();
            }
            if ui.button("三角形").clicked() {
                st.add_object_at(target.layer, TimelineObject::new_shape(start, end, 3, "f5a623"));
                ui.close_menu();
            }
            if ui.button("星型").clicked() {
                st.add_object_at(target.layer, TimelineObject::new_shape(start, end, 6, "ffd166"));
                ui.close_menu();
            }
            if ui.button("背景").clicked() {
                st.add_object_at(target.layer, TimelineObject::new_shape(start, end, 0, "1a1a2e"));
                ui.close_menu();
            }
        });
        if ui.button("フレームバッファ").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_framebuffer(start, end));
            ui.close_menu();
        }
        ui.separator();
        if ui.button("グループ制御").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_group_control(start, end));
            ui.close_menu();
        }
        if ui.button("カメラ制御").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_camera_control(start, end));
            ui.close_menu();
        }
    });

    // 3. フィルタオブジェクトの追加メニュー
    ui.menu_button("フィルタオブジェクトの追加", |ui| {
        if ui.button("色調補正").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_color_correction(start, end));
            ui.close_menu();
        }
    });

    if ui.button("📂 メディアファイルを選択して配置...").clicked() {
        ui.close_menu();
        st.add_media_dialog(target.layer, start);
    }

    ui.separator();

    // 4. レイヤー操作メニュー
    if let Some(layer) = st.scene.layers.get_mut(target.layer) {
        let vis_label = if layer.visible {
            "👁 レイヤーを非表示にする"
        } else {
            "👁 レイヤーを表示する"
        };
        if ui.button(vis_label).clicked() {
            layer.visible = !layer.visible;
            ui.close_menu();
        }
        let lock_label = if layer.locked {
            "🔓 レイヤーのロックを解除"
        } else {
            "🔒 レイヤーをロックする"
        };
        if ui.button(lock_label).clicked() {
            layer.locked = !layer.locked;
            ui.close_menu();
        }
    }
}
