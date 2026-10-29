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

use std::collections::HashMap;

use au2_core::{Effect, EffectKind, Frame, ObjectCategory, ObjectId, Rational, TimelineObject, MAX_LAYERS};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke};

use crate::state::{ContextTarget, EditorState, TimelineDrag};

const HEADER_W: f32 = 88.0;
const RULER_H: f32 = 22.0;
const H_SCROLLBAR_H: f32 = 18.0;
const ZOOM_BAR_W: f32 = 200.0;
const MIN_PPF: f32 = 0.05;
const MAX_PPF: f32 = 48.0;

fn ppf_to_slider_t(ppf: f32) -> f32 {
    let p = ppf.clamp(MIN_PPF, MAX_PPF);
    ((p / MIN_PPF).ln() / (MAX_PPF / MIN_PPF).ln()).clamp(0.0, 1.0)
}

fn slider_t_to_ppf(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    MIN_PPF * (MAX_PPF / MIN_PPF).powf(t)
}

fn zoom_at(v: &mut crate::state::TimelineView, lane_left: f32, center_x: f32, factor: f32) {
    let anchor = ((center_x - lane_left) / v.px_per_frame + v.scroll_x).max(0.0);
    v.px_per_frame = (v.px_per_frame * factor).clamp(MIN_PPF, MAX_PPF);
    v.scroll_x = (anchor - (center_x - lane_left) / v.px_per_frame).max(0.0);
}

fn fit_timeline_to_view(st: &mut EditorState, view_w: f32) {
    let end_f = (st.scene.content_end().max(180) + 30) as f32;
    let target_ppf = (view_w / end_f).clamp(MIN_PPF, MAX_PPF);
    st.timeline.px_per_frame = target_ppf;
    st.timeline.scroll_x = 0.0;
}

/// Premiere Pro 風のインクリップ・サムネイル／波形／図形プレビュー表示時のレイヤー高さ
const LAYER_H_PREVIEW: f32 = 46.0;
/// プレビュー OFF 時（従来の AviUtl 風コンパクト表示）のレイヤー高さ
const LAYER_H_COMPACT: f32 = 24.0;
const CLIP_HEADER_H: f32 = 15.0;

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
    let headers = Rect::from_min_max(pos2(full.left(), full.top() + RULER_H), pos2(full.left() + HEADER_W, full.bottom() - H_SCROLLBAR_H));
    let lanes = Rect::from_min_max(pos2(full.left() + HEADER_W, full.top() + RULER_H), pos2(full.right(), full.bottom() - H_SCROLLBAR_H));
    let zoom_bar = Rect::from_min_max(
        pos2((full.right() - ZOOM_BAR_W).max(full.left() + HEADER_W + 80.0), full.bottom() - H_SCROLLBAR_H),
        full.max,
    );
    let h_scrollbar = Rect::from_min_max(
        pos2(full.left() + HEADER_W, full.bottom() - H_SCROLLBAR_H),
        pos2(zoom_bar.left(), full.bottom()),
    );
    let corner_bl = Rect::from_min_max(pos2(full.left(), full.bottom() - H_SCROLLBAR_H), pos2(full.left() + HEADER_W, full.bottom()));
    let corner = Rect::from_min_max(full.min, pos2(headers.right(), ruler.bottom()));

    // ズームバー内のUIコンポーネント配置
    let btn_fit = Rect::from_min_size(pos2(zoom_bar.left() + 4.0, zoom_bar.top() + 2.0), vec2(44.0, H_SCROLLBAR_H - 4.0));
    let btn_minus = Rect::from_min_size(pos2(btn_fit.right() + 4.0, zoom_bar.top() + 2.0), vec2(16.0, H_SCROLLBAR_H - 4.0));
    let btn_plus = Rect::from_min_size(pos2(zoom_bar.right() - 48.0, zoom_bar.top() + 2.0), vec2(16.0, H_SCROLLBAR_H - 4.0));
    let slider_track = Rect::from_min_max(
        pos2(btn_minus.right() + 4.0, zoom_bar.top() + 7.0),
        pos2(btn_plus.left() - 4.0, zoom_bar.bottom() - 7.0),
    );
    let lbl_percent = Rect::from_min_max(pos2(btn_plus.right() + 2.0, zoom_bar.top()), pos2(zoom_bar.right() - 2.0, zoom_bar.bottom()));

    let ppf = st.timeline.px_per_frame;
    let current_t = ppf_to_slider_t(ppf);
    let knob_x = slider_track.left() + current_t * slider_track.width();
    let knob_rect = Rect::from_center_size(pos2(knob_x, slider_track.center().y), vec2(10.0, 14.0));

    let layer_h = if st.timeline.show_clip_previews {
        LAYER_H_PREVIEW
    } else {
        LAYER_H_COMPACT
    };
    let max_y = (MAX_LAYERS as f32 * layer_h - lanes.height()).max(0.0);

    // 水平スクロール可能範囲の動的計算（現在の長さ＋常に先へスクロール可能な余白）
    let content_frames = (st.scene.settings.length.max(st.scene.content_end()) as f32).max(1800.0);
    let visible_frames = (lanes.width() / ppf).max(1.0);
    let total_scrollable_frames = (content_frames + 3600.0).max(st.timeline.scroll_x + visible_frames + 1200.0);
    let max_scroll_x = (total_scrollable_frames - visible_frames).max(1.0);
    let thumb_w = ((visible_frames / total_scrollable_frames) * h_scrollbar.width()).clamp(28.0, h_scrollbar.width());
    let track_free = (h_scrollbar.width() - thumb_w).max(1.0);
    let scroll_ratio = (st.timeline.scroll_x / max_scroll_x).clamp(0.0, 1.0);
    let thumb_left = h_scrollbar.left() + scroll_ratio * track_free;
    let thumb_rect = Rect::from_min_size(pos2(thumb_left, h_scrollbar.top() + 2.0), vec2(thumb_w, H_SCROLLBAR_H - 4.0));

    // ---- 中クリック (ホイールクリック) / Alt+ドラッグによる縦横パン操作 ----
    let middle_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle));
    let alt_down = ui.input(|i| i.modifiers.alt && i.pointer.button_down(egui::PointerButton::Primary));
    if (middle_down || alt_down) && st.timeline.drag.is_none() {
        if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
            if full.contains(p) {
                st.timeline.drag = Some(TimelineDrag::Pan {
                    start_scroll_x: st.timeline.scroll_x,
                    start_scroll_y: st.timeline.scroll_y,
                    start_pos: p,
                });
            }
        }
    }

    // ---- キーボードショートカットによるズーム操作 (Ctrl + +, Ctrl + -, Ctrl + 0) ----
    let (ctrl_plus, ctrl_minus, ctrl_zero) = ui.input(|i| {
        let ctrl = i.modifiers.command || i.modifiers.ctrl;
        let plus = ctrl && (i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals));
        let minus = ctrl && i.key_pressed(egui::Key::Minus);
        let zero = ctrl && i.key_pressed(egui::Key::Num0);
        (plus, minus, zero)
    });
    if ctrl_plus {
        let center = (lanes.left() + (st.frame as f32 - st.timeline.scroll_x) * st.timeline.px_per_frame).clamp(lanes.left(), lanes.right());
        zoom_at(&mut st.timeline, lanes.left(), center, 1.25);
    } else if ctrl_minus {
        let center = (lanes.left() + (st.frame as f32 - st.timeline.scroll_x) * st.timeline.px_per_frame).clamp(lanes.left(), lanes.right());
        zoom_at(&mut st.timeline, lanes.left(), center, 0.8);
    } else if ctrl_zero {
        fit_timeline_to_view(st, lanes.width());
    }

    // ---- スクロール / ズーム ----
    if resp.hovered() {
        let (zoom, scroll, shift, ctrl) = ui.input(|i| {
            (
                i.zoom_delta(),
                i.smooth_scroll_delta,
                i.modifiers.shift,
                i.modifiers.command || i.modifiers.ctrl,
            )
        });
        let v = &mut st.timeline;
        if zoom != 1.0 {
            if let Some(p) = resp.hover_pos() {
                zoom_at(v, lanes.left(), p.x, zoom);
            }
        } else if ctrl && (scroll.y != 0.0 || scroll.x != 0.0) {
            // Ctrl + ホイールによるカーソル位置中心のタイムライン横拡大縮小
            let raw_delta = if scroll.y.abs() > scroll.x.abs() { scroll.y } else { scroll.x };
            let factor = (1.0 + raw_delta * 0.003).clamp(0.2, 5.0);
            if let Some(p) = resp.hover_pos() {
                zoom_at(v, lanes.left(), p.x, factor);
            } else {
                let center_x = lanes.center().x;
                zoom_at(v, lanes.left(), center_x, factor);
            }
        } else {
            let hover_p = resp.hover_pos();
            let is_over_ruler_or_scroll = hover_p.is_some_and(|p| ruler.contains(p) || h_scrollbar.contains(p));
            // Shift+ホイール、水平ホイール/タッチパッド、またはルーラー／スクロールバー上のホイールで横スクロール
            if shift || scroll.x != 0.0 || is_over_ruler_or_scroll {
                let raw_h = if scroll.x != 0.0 { scroll.x } else { scroll.y };
                v.scroll_x = (v.scroll_x - (raw_h * 2.5) / v.px_per_frame).max(0.0);
            } else if scroll.y != 0.0 {
                v.scroll_y = (v.scroll_y - scroll.y).clamp(0.0, max_y);
            }
        }
    }
    {
        let v = &mut st.timeline;
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
    let y_layer = |y: f32| (((y - lanes.top() + scroll_y) / layer_h).floor().max(0.0) as usize).min(MAX_LAYERS - 1);

    enum HitKind {
        Body(ObjectId, Frame),
        LeftEdge(ObjectId),
        RightEdge(ObjectId),
    }

    let hit_detail = |st: &EditorState, p: egui::Pos2| -> Option<HitKind> {
        if !lanes.contains(p) {
            return None;
        }
        let f = xf(p.x);
        let layer = st.scene.layers.get(y_layer(p.y))?;
        for o in &layer.objects {
            if o.contains(f) {
                let x0 = fx(o.frame_start as f32);
                let x1 = fx(o.frame_end as f32 + 1.0);
                let edge_threshold = 8.0f32;
                if (p.x - x0).abs() <= edge_threshold && x1 - x0 > edge_threshold * 2.0 {
                    return Some(HitKind::LeftEdge(o.id));
                } else if (p.x - x1).abs() <= edge_threshold && x1 - x0 > edge_threshold * 2.0 {
                    return Some(HitKind::RightEdge(o.id));
                } else {
                    return Some(HitKind::Body(o.id, o.frame_start));
                }
            }
        }
        None
    };

    let hit = |st: &EditorState, p: egui::Pos2| -> Option<(ObjectId, Frame)> {
        match hit_detail(st, p) {
            Some(HitKind::Body(id, s)) => Some((id, s)),
            Some(HitKind::LeftEdge(id)) | Some(HitKind::RightEdge(id)) => {
                let s = st.scene.find(id).map(|(_, o)| o.frame_start).unwrap_or(0);
                Some((id, s))
            }
            None => None,
        }
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
            st.timeline.drag = if btn_fit.contains(p) {
                fit_timeline_to_view(st, lanes.width());
                None
            } else if btn_minus.contains(p) {
                let center = (lanes.left() + (st.frame as f32 - st.timeline.scroll_x) * st.timeline.px_per_frame).clamp(lanes.left(), lanes.right());
                zoom_at(&mut st.timeline, lanes.left(), center, 0.8);
                None
            } else if btn_plus.contains(p) {
                let center = (lanes.left() + (st.frame as f32 - st.timeline.scroll_x) * st.timeline.px_per_frame).clamp(lanes.left(), lanes.right());
                zoom_at(&mut st.timeline, lanes.left(), center, 1.25);
                None
            } else if slider_track.expand2(vec2(4.0, 4.0)).contains(p) {
                let t = ((p.x - slider_track.left()) / slider_track.width()).clamp(0.0, 1.0);
                let new_ppf = slider_t_to_ppf(t);
                let center = (lanes.left() + (st.frame as f32 - st.timeline.scroll_x) * st.timeline.px_per_frame).clamp(lanes.left(), lanes.right());
                let anchor = ((center - lanes.left()) / st.timeline.px_per_frame + st.timeline.scroll_x).max(0.0);
                st.timeline.px_per_frame = new_ppf;
                st.timeline.scroll_x = (anchor - (center - lanes.left()) / new_ppf).max(0.0);
                Some(TimelineDrag::ZoomSlider)
            } else if h_scrollbar.contains(p) {
                if thumb_rect.contains(p) {
                    Some(TimelineDrag::ScrollbarThumb { grab_rel: (p.x - thumb_left) / thumb_w.max(1.0) })
                } else if p.x < thumb_left {
                    st.timeline.scroll_x = (st.timeline.scroll_x - visible_frames * 0.75).max(0.0);
                    None
                } else {
                    st.timeline.scroll_x = (st.timeline.scroll_x + visible_frames * 0.75).min(max_scroll_x);
                    None
                }
            } else if ruler.contains(p) {
                Some(TimelineDrag::Scrub)
            } else if let Some(hk) = hit_detail(st, p) {
                match hk {
                    HitKind::LeftEdge(id) => {
                        st.selected = Some(id);
                        Some(TimelineDrag::ResizeLeft { id })
                    }
                    HitKind::RightEdge(id) => {
                        st.selected = Some(id);
                        Some(TimelineDrag::ResizeRight { id })
                    }
                    HitKind::Body(id, start) => {
                        st.selected = Some(id);
                        Some(TimelineDrag::Move { id, grab_offset: xf(p.x) as i64 - start as i64 })
                    }
                }
            } else if lanes.contains(p) {
                Some(TimelineDrag::Scrub)
            } else {
                None
            };
        }
    }
    if let Some(drag_mode) = st.timeline.drag {
        match drag_mode {
            TimelineDrag::ZoomSlider => {
                if ui.input(|i| i.pointer.primary_down()) {
                    if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                        let t = ((p.x - slider_track.left()) / slider_track.width()).clamp(0.0, 1.0);
                        let new_ppf = slider_t_to_ppf(t);
                        let center = (lanes.left() + (st.frame as f32 - st.timeline.scroll_x) * st.timeline.px_per_frame).clamp(lanes.left(), lanes.right());
                        let anchor = ((center - lanes.left()) / st.timeline.px_per_frame + st.timeline.scroll_x).max(0.0);
                        st.timeline.px_per_frame = new_ppf;
                        st.timeline.scroll_x = (anchor - (center - lanes.left()) / new_ppf).max(0.0);
                        ui.ctx().request_repaint();
                    }
                } else {
                    st.timeline.drag = None;
                }
            }
            TimelineDrag::Pan { start_scroll_x, start_scroll_y, start_pos } => {
                if middle_down || alt_down {
                    if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                        let dx = (p.x - start_pos.x) / ppf;
                        let dy = p.y - start_pos.y;
                        st.timeline.scroll_x = (start_scroll_x - dx).max(0.0);
                        st.timeline.scroll_y = (start_scroll_y - dy).clamp(0.0, max_y);
                    }
                } else {
                    st.timeline.drag = None;
                }
            }
            TimelineDrag::ScrollbarThumb { grab_rel } => {
                if ui.input(|i| i.pointer.primary_down()) {
                    if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                        let desired_left = p.x - grab_rel * thumb_w;
                        let rel_x = (desired_left - h_scrollbar.left()).clamp(0.0, track_free);
                        st.timeline.scroll_x = (rel_x / track_free) * max_scroll_x;
                    }
                } else {
                    st.timeline.drag = None;
                }
            }
            TimelineDrag::Scrub => {
                if let Some(p) = resp.interact_pointer_pos() {
                    let frame = xf(p.x);
                    st.set_frame(frame);

                    // 赤い縦線（再生位置）をドラッグした際、画面端に近づくか見切れたらタイムラインを自動で一緒にスクロール
                    let margin = 32.0;
                    if p.x > lanes.right() - margin {
                        let overflow = (p.x - (lanes.right() - margin)).max(1.0);
                        let speed = (overflow * 0.75).clamp(2.0, 36.0);
                        st.timeline.scroll_x += speed / ppf;
                        ui.ctx().request_repaint();
                    } else if p.x < lanes.left() + margin {
                        let overflow = ((lanes.left() + margin) - p.x).max(1.0);
                        let speed = (overflow * 0.75).clamp(2.0, 36.0);
                        st.timeline.scroll_x = (st.timeline.scroll_x - speed / ppf).max(0.0);
                        ui.ctx().request_repaint();
                    }
                }
            }
            TimelineDrag::Move { id, grab_offset } => {
                if let Some(p) = resp.interact_pointer_pos() {
                    // 端に近づいた時の自動横スクロール
                    if p.x > lanes.right() - 24.0 {
                        st.timeline.scroll_x += 16.0 / ppf;
                    } else if p.x < lanes.left() + 24.0 {
                        st.timeline.scroll_x = (st.timeline.scroll_x - 16.0 / ppf).max(0.0);
                    }
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
            }
            TimelineDrag::ResizeLeft { id } => {
                if let Some(p) = resp.interact_pointer_pos() {
                    let new_start = xf(p.x);
                    if let Some((_, obj)) = st.scene.find_mut(id) {
                        if new_start <= obj.frame_end {
                            obj.frame_start = new_start;
                            st.dirty = true;
                        }
                    }
                }
            }
            TimelineDrag::ResizeRight { id } => {
                if let Some(p) = resp.interact_pointer_pos() {
                    let new_end = xf(p.x);
                    let mut updated_end = None;
                    if let Some((_, obj)) = st.scene.find_mut(id) {
                        if new_end >= obj.frame_start {
                            obj.frame_end = new_end;
                            updated_end = Some(new_end);
                            st.dirty = true;
                        }
                    }
                    if let Some(end) = updated_end {
                        if end + 1 > st.scene.settings.length {
                            st.scene.settings.length = end + 30;
                        }
                    }
                }
            }
        }
    }
    if resp.drag_stopped() {
        st.timeline.drag = None;
    }
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            if corner.contains(p) {
                st.timeline.show_clip_previews = !st.timeline.show_clip_previews;
            } else if h_scrollbar.contains(p) {
                if p.x < thumb_left {
                    st.timeline.scroll_x = (st.timeline.scroll_x - visible_frames * 0.75).max(0.0);
                } else if p.x > thumb_rect.right() {
                    st.timeline.scroll_x = (st.timeline.scroll_x + visible_frames * 0.75).min(max_scroll_x);
                }
            } else if headers.contains(p) {
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

    let show_previews = st.timeline.show_clip_previews;
    let layer_h = if show_previews { LAYER_H_PREVIEW } else { LAYER_H_COMPACT };
    let row_top = |li: usize| lanes.top() + li as f32 * layer_h - scroll_y;

    // ---- 描画 ----
    painter.rect_filled(full, 0.0, Color32::from_gray(28));
    let first_row = (scroll_y / layer_h).floor() as usize;
    let rows = (lanes.height() / layer_h).ceil() as usize + 1;
    let visible_rows = first_row..(first_row + rows).min(MAX_LAYERS);

    // レーン背景
    let lane_painter = painter.with_clip_rect(lanes);
    for li in visible_rows.clone() {
        let y = row_top(li);
        let r = Rect::from_min_size(pos2(lanes.left(), y), vec2(lanes.width(), layer_h));
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

    // オブジェクト用サムネイルの事前取得 (プレビュー ON 時のみ)
    let mut clip_textures: HashMap<String, egui::TextureId> = HashMap::new();
    if show_previews {
        let egui_ctx = ui.ctx().clone();
        let mut media_paths: Vec<String> = Vec::new();
        for li in visible_rows.clone() {
            for obj in &st.scene.layers[li].objects {
                let (x0, x1) = (fx(obj.frame_start as f32), fx(obj.frame_end as f32 + 1.0));
                if x1 < lanes.left() || x0 > lanes.right() {
                    continue;
                }
                if let Some(src) = obj.source() {
                    if matches!(src.kind, EffectKind::VideoFile | EffectKind::ImageFile) {
                        if let Some(p) = effect_file_path(src).filter(|s| !s.is_empty()) {
                            let s = p.to_string();
                            if !media_paths.contains(&s) {
                                media_paths.push(s);
                            }
                        }
                    }
                }
            }
        }
        for path_str in media_paths {
            if let Some((tex_id, _)) = st.get_media_texture(&egui_ctx, &path_str, 0, 0.0) {
                clip_textures.insert(path_str, tex_id);
            }
        }
    }

    // オブジェクト描画 + ホバー検出
    let hover_pos = resp.hover_pos();
    let mut hovered_clip: Option<(usize, ObjectId)> = None;

    // 未検出メディアを持つオブジェクトの ID セットを事前抽出
    let missing_obj_ids: std::collections::HashSet<ObjectId> = st
        .check_missing_media()
        .into_iter()
        .map(|m| m.object_id)
        .collect();

    for li in visible_rows.clone() {
        let layer = &st.scene.layers[li];
        let y = row_top(li);
        for obj in &layer.objects {
            let (x0, x1) = (fx(obj.frame_start as f32), fx(obj.frame_end as f32 + 1.0));
            if x1 < lanes.left() || x0 > lanes.right() {
                continue;
            }
            let r = Rect::from_min_max(pos2(x0, y + 2.0), pos2(x1.max(x0 + 3.0), y + layer_h - 2.0));
            if let Some(hp) = hover_pos {
                if lanes.contains(hp) && r.contains(hp) {
                    hovered_clip = Some((li, obj.id));
                    let edge_t = 8.0f32;
                    if (hp.x - x0).abs() <= edge_t || (hp.x - x1).abs() <= edge_t {
                        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeHorizontal);
                    }
                }
            }

            let is_missing = missing_obj_ids.contains(&obj.id);
            let selected = st.selected == Some(obj.id);
            let dimmed = !layer.visible || obj.source().is_some_and(|s| !s.enabled);
            let mut base_color = if is_missing {
                Color32::from_rgb(180, 50, 50)
            } else {
                category_color(obj.category())
            };
            if selected {
                base_color = base_color.linear_multiply(1.3);
            }
            if dimmed {
                base_color = base_color.gamma_multiply(0.35);
            }

            if show_previews {
                // --- Premiere Pro 風の2段表示（ヘッダー + ボディプレビュー） ---
                let body_bg = base_color.gamma_multiply(0.45);
                lane_painter.rect_filled(r, 3.5, body_bg);

                let header_bottom = (r.top() + CLIP_HEADER_H).min(r.bottom());
                let header_rect = Rect::from_min_max(r.min, pos2(r.right(), header_bottom));
                let body_rect = Rect::from_min_max(pos2(r.left(), header_bottom), r.max);

                lane_painter.rect_filled(header_rect, 3.5, base_color);

                let body_clip = body_rect.shrink(1.0).intersect(lanes);
                if body_clip.width() > 4.0 && body_clip.height() > 6.0 {
                    let body_painter = painter.with_clip_rect(body_clip);
                    draw_clip_preview(&body_painter, body_rect, lanes, obj, &clip_textures, dimmed);
                }

                lane_painter.line_segment(
                    [pos2(r.left(), header_bottom), pos2(r.right(), header_bottom)],
                    Stroke::new(1.0_f32, Color32::from_black_alpha(90)),
                );

                // 中間点マーカー
                for &m in &obj.midpoints {
                    let mx = fx((obj.frame_start + m) as f32);
                    lane_painter.line_segment(
                        [pos2(mx, r.top()), pos2(mx, r.bottom())],
                        Stroke::new(1.0_f32, Color32::from_white_alpha(160)),
                    );
                    let c = pos2(mx, header_rect.center().y);
                    lane_painter.add(Shape::convex_polygon(
                        vec![c + vec2(0.0, -3.5), c + vec2(3.5, 0.0), c + vec2(0.0, 3.5), c + vec2(-3.5, 0.0)],
                        Color32::WHITE,
                        Stroke::NONE,
                    ));
                }

                if selected {
                    lane_painter.rect_stroke(r, 3.5, Stroke::new(1.6_f32, Color32::WHITE));
                } else {
                    lane_painter.rect_stroke(r, 3.5, Stroke::new(1.0_f32, Color32::from_black_alpha(110)));
                }

                // ヘッダーラベル + アイコン + fx バッジ（高さを正確に統一）
                let text_clip = header_rect.shrink2(vec2(3.0, 0.0)).intersect(lanes);
                if text_clip.width() > 8.0 {
                    let hp = painter.with_clip_rect(text_clip);
                    let icon = clip_kind_icon(obj);
                    let left_x = header_rect.left().max(lanes.left()) + 5.0;
                    let cy = header_rect.center().y;
                    hp.text(
                        pos2(left_x, cy),
                        Align2::LEFT_CENTER,
                        icon,
                        FontId::proportional(11.0),
                        Color32::WHITE,
                    );
                    let disp_text = if is_missing {
                        format!("⚠ 未検出: {}", obj.display_name())
                    } else {
                        obj.display_name().to_string()
                    };
                    let text_color = if is_missing {
                        Color32::from_rgb(255, 170, 170)
                    } else {
                        Color32::WHITE
                    };
                    hp.text(
                        pos2(left_x + 15.0, cy),
                        Align2::LEFT_CENTER,
                        disp_text,
                        FontId::proportional(11.0),
                        text_color,
                    );
                    let extra_fx = obj.effects.iter().skip(1).filter(|e| !is_render_or_playback(&e.kind)).count();
                    if extra_fx > 0 && text_clip.width() > 54.0 {
                        let badge_text = format!("fx+{extra_fx}");
                        let bx = header_rect.right().min(lanes.right()) - 4.0;
                        hp.text(
                            pos2(bx, cy),
                            Align2::RIGHT_CENTER,
                            badge_text,
                            FontId::monospace(9.5),
                            Color32::from_rgb(255, 230, 120),
                        );
                    }
                }
            } else {
                // --- 従来の AviUtl 風コンパクト1段表示 ---
                lane_painter.rect_filled(r, 3.0, base_color);
                if selected {
                    lane_painter.rect_stroke(r, 3.0, Stroke::new(1.5_f32, Color32::WHITE));
                }
                for &m in &obj.midpoints {
                    let mx = fx((obj.frame_start + m) as f32);
                    lane_painter.line_segment(
                        [pos2(mx, r.top()), pos2(mx, r.bottom())],
                        Stroke::new(1.0_f32, Color32::from_white_alpha(140)),
                    );
                    let c = pos2(mx, r.center().y);
                    lane_painter.add(Shape::convex_polygon(
                        vec![c + vec2(0.0, -4.0), c + vec2(4.0, 0.0), c + vec2(0.0, 4.0), c + vec2(-4.0, 0.0)],
                        Color32::WHITE,
                        Stroke::NONE,
                    ));
                }
                let text_clip = r.shrink2(vec2(3.0, 0.0)).intersect(lanes);
                if text_clip.width() > 8.0 {
                    let disp_text = if is_missing {
                        format!("⚠ 未検出: {}", obj.display_name())
                    } else {
                        obj.display_name().to_string()
                    };
                    let text_color = if is_missing {
                        Color32::from_rgb(255, 170, 170)
                    } else {
                        Color32::WHITE
                    };
                    painter.with_clip_rect(text_clip).text(
                        pos2(r.left().max(lanes.left()) + 5.0, r.center().y),
                        Align2::LEFT_CENTER,
                        disp_text,
                        FontId::proportional(12.0),
                        text_color,
                    );
                }
            }
        }
    }

    // クリップホバー時のフローティング・プレビューツールチップ (プレビュー ON 時のみ)
    if show_previews && st.timeline.drag.is_none() {
        if let (Some(hp), Some((li, obj_id))) = (hover_pos, hovered_clip) {
            if let Some(obj) = st.scene.layers[li].objects.iter().find(|o| o.id == obj_id) {
                draw_clip_hover_tooltip(&painter, full, hp, li, obj, st.scene.settings.frame_rate, &clip_textures);
            }
        }
    }

    // ルーラー
    painter.rect_filled(ruler, 0.0, Color32::from_rgb(26, 28, 35));
    painter.line_segment([ruler.left_bottom(), ruler.right_bottom()], Stroke::new(1.0_f32, Color32::from_gray(44)));
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
        ruler_painter.line_segment([pos2(x, ruler.bottom() - h), pos2(x, ruler.bottom())], Stroke::new(1.0_f32, Color32::from_gray(120)));
        if major {
            ruler_painter.text(pos2(x + 3.0, ruler.top() + 2.0), Align2::LEFT_TOP, fmt_time(f, rate), FontId::monospace(10.0), Color32::from_gray(190));
        }
        f += minor;
    }

    // レイヤーヘッダ
    let header_painter = painter.with_clip_rect(headers);
    header_painter.rect_filled(headers, 0.0, Color32::from_rgb(24, 26, 32));
    for li in visible_rows {
        let y = row_top(li);
        let r = Rect::from_min_size(pos2(headers.left(), y), vec2(HEADER_W, layer_h));
        let layer = &st.scene.layers[li];
        let color = if layer.visible { Color32::from_gray(225) } else { Color32::from_gray(100) };
        let base_name = layer.name.clone().unwrap_or_else(|| format!("Layer {}", li + 1));

        // レイヤーバッジ [L1]
        let badge_rect = Rect::from_min_size(pos2(r.left() + 5.0, r.center().y - 8.0), vec2(22.0, 16.0));
        header_painter.rect_filled(badge_rect, 3.0, Color32::from_rgb(34, 38, 48));
        header_painter.text(badge_rect.center(), Align2::CENTER_CENTER, format!("L{}", li + 1), FontId::monospace(10.0), Color32::from_rgb(175, 195, 225));

        // レイヤー名
        header_painter.text(pos2(badge_rect.right() + 5.0, r.center().y), Align2::LEFT_CENTER, base_name, FontId::proportional(11.5), color);

        // 状態アイコン (右端)
        if layer.locked {
            header_painter.text(pos2(r.right() - 5.0, r.center().y), Align2::RIGHT_CENTER, "🔒", FontId::proportional(10.0), Color32::from_rgb(250, 204, 21));
        } else if !layer.visible {
            header_painter.text(pos2(r.right() - 5.0, r.center().y), Align2::RIGHT_CENTER, "🚫", FontId::proportional(10.0), Color32::from_gray(120));
        }
        header_painter.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, Color32::from_gray(35)));
    }

    // 左上コーナー：クリップ内プレビュー ON/OFF スタイリッシュピルボタン
    let corner_hovered = hover_pos.is_some_and(|p| corner.contains(p));
    painter.rect_filled(corner, 0.0, Color32::from_rgb(22, 24, 30));
    painter.line_segment([corner.right_top(), corner.right_bottom()], Stroke::new(1.0_f32, Color32::from_gray(40)));
    painter.line_segment([corner.left_bottom(), corner.right_bottom()], Stroke::new(1.0_f32, Color32::from_gray(40)));

    let btn_rect = corner.shrink2(vec2(4.0, 3.0));
    let (btn_fill, btn_stroke, text_color) = if show_previews {
        if corner_hovered {
            (Color32::from_rgb(45, 110, 245), Stroke::new(1.0_f32, Color32::from_rgb(147, 197, 253)), Color32::WHITE)
        } else {
            (Color32::from_rgb(37, 99, 235), Stroke::new(1.0_f32, Color32::from_rgb(96, 165, 250)), Color32::WHITE)
        }
    } else if corner_hovered {
        (Color32::from_rgb(52, 56, 68), Stroke::new(1.0_f32, Color32::from_gray(120)), Color32::from_gray(225))
    } else {
        (Color32::from_rgb(38, 41, 50), Stroke::new(1.0_f32, Color32::from_gray(65)), Color32::from_gray(175))
    };
    painter.rect_filled(btn_rect, 4.0, btn_fill);
    painter.rect_stroke(btn_rect, 4.0, btn_stroke);

    let toggle_label = if show_previews {
        "🖼 プレビュー: ON"
    } else {
        "🖼 プレビュー: OFF"
    };
    painter.text(
        btn_rect.center(),
        Align2::CENTER_CENTER,
        toggle_label,
        FontId::proportional(10.0),
        text_color,
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

    // 下部水平スクロールバー
    painter.rect_filled(h_scrollbar, 0.0, Color32::from_gray(24));
    painter.line_segment([h_scrollbar.left_top(), h_scrollbar.right_top()], Stroke::new(1.0_f32, Color32::from_gray(40)));
    let thumb_hovered = hover_pos.is_some_and(|p| thumb_rect.contains(p));
    let thumb_bg = if matches!(st.timeline.drag, Some(TimelineDrag::ScrollbarThumb { .. })) {
        Color32::from_rgb(37, 99, 235)
    } else if thumb_hovered {
        Color32::from_gray(120)
    } else {
        Color32::from_gray(75)
    };
    painter.rect_filled(thumb_rect, 3.0, thumb_bg);
    painter.rect_stroke(thumb_rect, 3.0, Stroke::new(1.0_f32, Color32::from_white_alpha(45)));

    // 右下ズームツールバー
    painter.rect_filled(zoom_bar, 0.0, Color32::from_rgb(24, 26, 32));
    painter.line_segment([zoom_bar.left_top(), zoom_bar.right_top()], Stroke::new(1.0_f32, Color32::from_gray(40)));
    painter.line_segment([zoom_bar.left_top(), zoom_bar.left_bottom()], Stroke::new(1.0_f32, Color32::from_gray(40)));

    // [⊡ 全体] ボタン (Fit)
    let fit_hovered = hover_pos.is_some_and(|p| btn_fit.contains(p));
    let fit_bg = if fit_hovered { Color32::from_gray(56) } else { Color32::from_gray(38) };
    painter.rect_filled(btn_fit, 3.0, fit_bg);
    painter.rect_stroke(btn_fit, 3.0, Stroke::new(1.0_f32, if fit_hovered { Color32::from_gray(90) } else { Color32::from_gray(50) }));
    painter.text(
        btn_fit.center(),
        Align2::CENTER_CENTER,
        "⊡ 全体",
        FontId::proportional(10.0),
        if fit_hovered { Color32::WHITE } else { Color32::from_gray(210) },
    );

    // [-] ボタン
    let minus_hovered = hover_pos.is_some_and(|p| btn_minus.contains(p));
    let minus_bg = if minus_hovered { Color32::from_gray(56) } else { Color32::from_gray(38) };
    painter.rect_filled(btn_minus, 3.0, minus_bg);
    painter.rect_stroke(btn_minus, 3.0, Stroke::new(1.0_f32, if minus_hovered { Color32::from_gray(90) } else { Color32::from_gray(50) }));
    painter.text(
        btn_minus.center(),
        Align2::CENTER_CENTER,
        "−",
        FontId::proportional(11.0),
        if minus_hovered { Color32::WHITE } else { Color32::from_gray(210) },
    );

    // スライダートラック (溝)
    painter.rect_filled(slider_track, 2.0, Color32::from_gray(18));
    painter.rect_stroke(slider_track, 2.0, Stroke::new(1.0_f32, Color32::from_gray(42)));

    // スライダーつまみ (Knob)
    let is_zoom_dragging = matches!(st.timeline.drag, Some(TimelineDrag::ZoomSlider));
    let knob_hovered = hover_pos.is_some_and(|p| knob_rect.contains(p)) || is_zoom_dragging;
    let knob_bg = if is_zoom_dragging {
        Color32::from_rgb(59, 130, 246)
    } else if knob_hovered {
        Color32::from_gray(150)
    } else {
        Color32::from_gray(100)
    };
    painter.rect_filled(knob_rect, 2.5, knob_bg);
    painter.rect_stroke(knob_rect, 2.5, Stroke::new(1.0_f32, Color32::from_white_alpha(50)));

    // [+] ボタン
    let plus_hovered = hover_pos.is_some_and(|p| btn_plus.contains(p));
    let plus_bg = if plus_hovered { Color32::from_gray(56) } else { Color32::from_gray(38) };
    painter.rect_filled(btn_plus, 3.0, plus_bg);
    painter.rect_stroke(btn_plus, 3.0, Stroke::new(1.0_f32, if plus_hovered { Color32::from_gray(90) } else { Color32::from_gray(50) }));
    painter.text(
        btn_plus.center(),
        Align2::CENTER_CENTER,
        "+",
        FontId::proportional(11.0),
        if plus_hovered { Color32::WHITE } else { Color32::from_gray(210) },
    );

    // 倍率テキスト (例: 100%)
    let zoom_pct = (ppf * 100.0).round() as i64;
    painter.text(
        pos2(lbl_percent.right() - 2.0, lbl_percent.center().y),
        Align2::RIGHT_CENTER,
        format!("{}%", zoom_pct),
        FontId::monospace(9.5),
        Color32::from_gray(185),
    );

    // 左下角
    painter.rect_filled(corner_bl, 0.0, Color32::from_rgb(24, 26, 32));
    painter.line_segment([corner_bl.left_top(), corner_bl.right_top()], Stroke::new(1.0_f32, Color32::from_gray(40)));
    painter.line_segment([corner_bl.right_top(), corner_bl.right_bottom()], Stroke::new(1.0_f32, Color32::from_gray(40)));
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
        let raw_file_info: Option<(String, bool)> = st.scene.find(obj_id).and_then(|(_, obj)| {
            let src = obj.source()?;
            let f = src.text("file")?;
            let is_audio = src.kind == au2_core::EffectKind::AudioFile;
            Some((f.to_string(), is_audio))
        });
        let media_file_info = raw_file_info.map(|(f, is_audio)| {
            let is_missing = st.resolve_cached_path(&f).is_none();
            (f, is_audio, is_missing)
        });

        if let Some((old_file, is_audio, is_missing)) = media_file_info {
            let label = if is_missing {
                egui::RichText::new("⚠ 未検出ファイルを割り当て (MP3/WAV/動画)...")
                    .color(Color32::from_rgb(255, 100, 100))
                    .strong()
            } else {
                egui::RichText::new("🎵 メディアファイルを再割り当て...")
            };
            if ui.button(label).clicked() {
                ui.close_menu();
                st.relink_media_dialog(Some(obj_id), &old_file, is_audio);
            }
            ui.separator();
        }

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
            if ui.button("五角形").clicked() {
                st.add_object_at(target.layer, TimelineObject::new_shape(start, end, 4, "8b5cf6"));
                ui.close_menu();
            }
            if ui.button("六角形").clicked() {
                st.add_object_at(target.layer, TimelineObject::new_shape(start, end, 5, "38bdf8"));
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
        if ui.button("音声波形表示").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_audio_waveform(start, end));
            ui.close_menu();
        }
        if ui.button("シーン").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_scene_object(start, end, 1));
            ui.close_menu();
        }
        if ui.button("直前オブジェクト").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_previous_object(start, end));
            ui.close_menu();
        }
        if ui.button("パーティクル出力").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_particle_output(start, end));
            ui.close_menu();
        }
        ui.menu_button("カスタムオブジェクト", |ui| {
            for name in [
                "集中線",
                "走査線",
                "カウンター",
                "レンズフレア",
                "雲",
                "星",
                "雪",
                "雨",
                "ランダム小物配置(カメラ制御)",
                "ライン(移動軌跡)",
                "扇型",
                "多角形",
                "周辺ボケ光量",
                "フレア",
                "水面",
            ] {
                if ui.button(name).clicked() {
                    st.add_object_at(target.layer, TimelineObject::new_custom_object(start, end, name));
                    ui.close_menu();
                }
            }
        });
        ui.separator();
        if ui.button("時間制御").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_time_control(start, end));
            ui.close_menu();
        }
        if ui.button("グループ制御").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_group_control(start, end));
            ui.close_menu();
        }
        if ui.button("カメラ制御").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_camera_control(start, end));
            ui.close_menu();
        }
    });

    // 3. フィルタオブジェクトの追加メニュー（AviUtl 拡張編集互換）
    ui.menu_button("フィルタオブジェクトの追加", |ui| {
        if ui.button("色調補正").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_color_correction(start, end));
            ui.close_menu();
        }
        for fname in [
            "ぼかし",
            "レンズブラー",
            "放射ブラー",
            "方向ブラー",
            "グロー",
            "発光",
            "拡散光",
            "閃光",
            "ライト",
            "モザイク",
            "ラスター",
            "波紋",
            "色ずれ",
            "単色化",
            "グラデーション",
            "ノイズ",
            "ミラー",
            "反転",
        ] {
            if ui.button(fname).clicked() {
                let mut eff = au2_core::Effect::new(fname);
                eff.push_track("強さ", au2_core::Track::constant(60.0, 1));
                eff.push_track("範囲", au2_core::Track::constant(10.0, 0));
                st.add_object_at(target.layer, TimelineObject::new_filter_object(start, end, eff));
                ui.close_menu();
            }
        }
        ui.separator();
        ui.menu_button("シーンチェンジ", |ui| {
            for (sc_name, sc_type) in [
                ("クロスフェード", 0),
                ("ワイプ(円)", 1),
                ("ワイプ(四角)", 2),
                ("ワイプ(時計)", 3),
                ("スライス", 4),
                ("フェードアウトイン", 14),
                ("放射ブラー", 15),
                ("ぼかし", 16),
                ("発光", 0),
                ("レンズブラー", 1),
                ("ドア", 2),
                ("起き上がる", 3),
                ("リール回転", 4),
                ("図形ワイプ", 5),
                ("砕け散る", 8),
                ("ページめくり", 9),
            ] {
                if ui.button(sc_name).clicked() {
                    st.add_object_at(target.layer, TimelineObject::new_scene_change(start, end, sc_name, sc_type));
                    ui.close_menu();
                }
            }
        });
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

    ui.separator();
    if ui
        .checkbox(&mut st.timeline.show_clip_previews, "🖼 クリップ内プレビューを表示")
        .changed()
    {
        ui.close_menu();
    }
}

/// エフェクトからファイルパス文字列 (`file` または `ファイル`) を取得する。
fn effect_file_path(eff: &Effect) -> Option<&str> {
    eff.text("file").or_else(|| eff.text("ファイル"))
}

/// 標準描画・拡張描画・標準再生など末尾の描画/再生プロパティかどうか。
fn is_render_or_playback(kind: &EffectKind) -> bool {
    matches!(
        kind,
        EffectKind::StandardDraw | EffectKind::ExtendedDraw | EffectKind::StandardPlayback
    )
}

/// クリップ種別に応じたアイコン文字を返す。
fn clip_kind_icon(obj: &TimelineObject) -> &'static str {
    match obj.source().map(|s| &s.kind) {
        Some(EffectKind::VideoFile) => "🎬",
        Some(EffectKind::ImageFile) => "🖼",
        Some(EffectKind::AudioFile) => "🔊",
        Some(EffectKind::Text) => "Ｔ",
        Some(EffectKind::Shape) => "◆",
        Some(EffectKind::Scene) => "🎞",
        Some(EffectKind::FrameBuffer) => "▣",
        Some(EffectKind::GroupControl) => "⚙",
        Some(EffectKind::Camera) => "🎥",
        _ => match obj.category() {
            ObjectCategory::Media => "🎬",
            ObjectCategory::Audio => "🔊",
            ObjectCategory::Filter => "✨",
            ObjectCategory::Control => "⚙",
        },
    }
}

/// Premiere Pro 風のイン・クリップ・プレビュー（サムネイル帯・音声波形・図形/色見本・テキスト・フィルタ曲線）を描画する。
fn draw_clip_preview(
    painter: &egui::Painter,
    body_rect: Rect,
    lanes: Rect,
    obj: &TimelineObject,
    clip_textures: &HashMap<String, egui::TextureId>,
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
        }
        EffectKind::AudioFile => {
            // Premiere Pro 風のオーディオ波形バー描画
            let vol = (src.eval("音量", 0, 100.0) / 100.0).clamp(0.1, 1.5) as f32;
            let cy = body_rect.center().y;
            let max_amp = (body_rect.height() * 0.44 * vol).min(body_rect.height() * 0.47);
            painter.line_segment(
                [pos2(body_rect.left().max(lanes.left()), cy), pos2(body_rect.right().min(lanes.right()), cy)],
                Stroke::new(1.0_f32, Color32::from_white_alpha(45)),
            );
            let step = 3.0_f32;
            let start_x = body_rect.left().max(lanes.left());
            let end_x = body_rect.right().min(lanes.right());
            let seed = obj.id.0 as f32 * 0.73;
            let wave_color = Color32::from_rgba_unmultiplied(150, 245, 195, (210.0 * alpha_mul) as u8);
            let mut x = start_x + ((body_rect.left() - start_x).rem_euclid(step));
            while x < end_x {
                let rel = (x - body_rect.left()) * 0.14 + seed;
                let amp = ((rel.sin() * 0.55 + (rel * 2.37).cos() * 0.35 + (rel * 0.41).sin() * 0.25)
                    .abs()
                    .clamp(0.12, 1.0))
                    * max_amp;
                painter.line_segment(
                    [pos2(x, cy - amp), pos2(x, cy + amp)],
                    Stroke::new(1.6_f32, wave_color),
                );
                x += step;
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
fn draw_clip_hover_tooltip(
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

/// 6桁の16進カラー文字列 (`"ff8800"` など) を `Color32` に変換する。
fn parse_hex_rgb(hex: &str, alpha_mul: f32) -> Color32 {
    let clean = hex.trim().trim_start_matches('#');
    if clean.len() >= 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).unwrap_or(255);
        let g = u8::from_str_radix(&clean[2..4], 16).unwrap_or(255);
        let b = u8::from_str_radix(&clean[4..6], 16).unwrap_or(255);
        Color32::from_rgba_unmultiplied(r, g, b, (255.0 * alpha_mul) as u8)
    } else {
        Color32::from_white_alpha((255.0 * alpha_mul) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zoom_mapping_and_bounds() {
        assert!((ppf_to_slider_t(MIN_PPF) - 0.0).abs() < 1e-5);
        assert!((ppf_to_slider_t(MAX_PPF) - 1.0).abs() < 1e-5);
        assert!((slider_t_to_ppf(0.0) - MIN_PPF).abs() < 1e-5);
        assert!((slider_t_to_ppf(1.0) - MAX_PPF).abs() < 1e-3);

        let mid_t = 0.5;
        let mid_ppf = slider_t_to_ppf(mid_t);
        assert!(mid_ppf > MIN_PPF && mid_ppf < MAX_PPF);
        assert!((ppf_to_slider_t(mid_ppf) - mid_t).abs() < 1e-5);

        let mut v = crate::state::TimelineView::default();
        v.px_per_frame = 1.0;
        v.scroll_x = 100.0;
        let lane_left = 100.0;
        let mouse_x = 200.0; // lane_left + 100px -> frame 200
        zoom_at(&mut v, lane_left, mouse_x, 2.0);
        assert_eq!(v.px_per_frame, 2.0);
        // After 2x zoom, mouse_x should still point to frame 200
        let frame_under_mouse = (mouse_x - lane_left) / v.px_per_frame + v.scroll_x;
        assert!((frame_under_mouse - 200.0).abs() < 1e-4);
    }
}
