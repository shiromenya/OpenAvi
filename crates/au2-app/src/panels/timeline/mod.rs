//! タイムラインパネル（拡張編集ウィンドウ）のメインUI・イベントループ。

pub mod types;
pub mod snap;
pub mod render;
pub mod context_menu;
#[cfg(test)]
mod tests;

pub use types::*;
pub use snap::*;

use std::collections::HashMap;

use au2_core::{EffectKind, Frame, ObjectId, MAX_LAYERS};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Sense, Shape, Stroke};

use crate::state::{ContextTarget, EditorState, TimelineDrag};
use crate::theme::{self, flip};
use self::render::{draw_clip_hover_tooltip, draw_clip_preview};
use self::context_menu::context_menu_ui;

pub fn ui(ui: &mut egui::Ui, st: &mut EditorState) {
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let full = resp.rect;
    let ruler = Rect::from_min_max(pos2(full.left() + HEADER_W, full.top()), pos2(full.right(), full.top() + RULER_H));
    let headers = Rect::from_min_max(pos2(full.left(), full.top() + RULER_H), pos2(full.left() + HEADER_W, full.bottom() - H_SCROLLBAR_H));
    let lanes = Rect::from_min_max(pos2(full.left() + HEADER_W, full.top() + RULER_H), pos2(full.right(), full.bottom() - H_SCROLLBAR_H));
    st.timeline.lanes_rect = Some(lanes);
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
    let btn_snap = Rect::from_min_size(pos2(zoom_bar.left() + 4.0, zoom_bar.top() + 2.0), vec2(50.0, H_SCROLLBAR_H - 4.0));
    let btn_fit = Rect::from_min_size(pos2(btn_snap.right() + 4.0, zoom_bar.top() + 2.0), vec2(40.0, H_SCROLLBAR_H - 4.0));
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
    let total_scrollable_frames = (content_frames + TIMELINE_TAIL_MARGIN_FRAMES).max(st.timeline.scroll_x + visible_frames + TIMELINE_MIN_SCROLLABLE_FRAMES);
    let max_scroll_x = (total_scrollable_frames - visible_frames).max(1.0);
    let thumb_w = ((visible_frames / total_scrollable_frames) * h_scrollbar.width()).clamp(SCROLLBAR_MIN_THUMB_W, h_scrollbar.width());
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
        if px > lanes.right() - PLAYHEAD_FOLLOW_MARGIN_PX || px < lanes.left() {
            v.scroll_x = (st.frame as f32 - PLAYHEAD_FOLLOW_LEAD_PX / v.px_per_frame).max(0.0);
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
            st.timeline.drag = if btn_snap.contains(p) {
                st.timeline.snap_enabled = !st.timeline.snap_enabled;
                st.status = if st.timeline.snap_enabled {
                    "🧲 スナップ（吸着）を有効にしました".into()
                } else {
                    "🧲 スナップ（吸着）を無効にしました".into()
                };
                None
            } else if btn_fit.contains(p) {
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
                let shift = ui.input(|i| i.modifiers.shift);
                match hk {
                    HitKind::LeftEdge(id) => {
                        if shift {
                            st.toggle_select(id);
                        } else if !st.selected_objects.contains(&id) {
                            st.select_single(id);
                        }
                        st.begin_continuous_edit("クリップ左端トリム");
                        Some(TimelineDrag::ResizeLeft { id })
                    }
                    HitKind::RightEdge(id) => {
                        if shift {
                            st.toggle_select(id);
                        } else if !st.selected_objects.contains(&id) {
                            st.select_single(id);
                        }
                        st.begin_continuous_edit("クリップ右端トリム");
                        Some(TimelineDrag::ResizeRight { id })
                    }
                    HitKind::Body(id, start) => {
                        if shift {
                            st.toggle_select(id);
                        } else if !st.selected_objects.contains(&id) {
                            st.select_single(id);
                        }
                        let desc = if st.selected_objects.len() > 1 {
                            "複数クリップの移動"
                        } else {
                            "クリップの移動"
                        };
                        st.begin_continuous_edit(desc);
                        Some(TimelineDrag::Move { id, grab_offset: xf(p.x) as i64 - start as i64 })
                    }
                }
            } else if headers.contains(p) {
                // レイヤーヘッダー領域のクリック（可視トグル・ロックトグル・レイヤー選択）
                let li = y_layer(p.y);
                if li < MAX_LAYERS {
                    let y = lanes.top() + li as f32 * layer_h - scroll_y;
                    let r = Rect::from_min_max(pos2(headers.left(), y), pos2(headers.right(), y + layer_h));
                    let vis_btn_rect = Rect::from_min_size(pos2(r.right() - 38.0, r.center().y - 8.0), vec2(17.0, 16.0));
                    let lock_btn_rect = Rect::from_min_size(pos2(r.right() - 19.0, r.center().y - 8.0), vec2(17.0, 16.0));

                    if vis_btn_rect.contains(p) {
                        let cur_vis = st.scene.layers[li].visible;
                        st.push_undo(if cur_vis { format!("Layer {} を非表示", li + 1) } else { format!("Layer {} を表示", li + 1) });
                        st.scene.layers[li].visible = !cur_vis;
                        st.dirty = true;
                        st.status = if !cur_vis { format!("Layer {} を表示しました", li + 1) } else { format!("Layer {} を非表示にしました", li + 1) };
                    } else if lock_btn_rect.contains(p) {
                        let cur_lock = st.scene.layers[li].locked;
                        st.push_undo(if cur_lock { format!("Layer {} のロックを解除", li + 1) } else { format!("Layer {} をロック", li + 1) });
                        st.scene.layers[li].locked = !cur_lock;
                        st.dirty = true;
                        st.status = if !cur_lock { format!("Layer {} をロックしました", li + 1) } else { format!("Layer {} のロックを解除しました", li + 1) };
                    } else {
                        st.timeline.selected_layer = li;
                        st.status = format!("Layer {} を選択しました", li + 1);
                    }
                }
                None
            } else if lanes.contains(p) {
                Some(TimelineDrag::BoxSelect { start_pos: p, current_pos: p })
            } else {
                None
            };
        }
    }
    if let Some(drag_mode) = st.timeline.drag {
        match drag_mode {
            TimelineDrag::BoxSelect { start_pos, current_pos: _ } => {
                if let Some(p) = resp.interact_pointer_pos() {
                    st.timeline.drag = Some(TimelineDrag::BoxSelect { start_pos, current_pos: p });
                    let sel_rect = Rect::from_two_pos(start_pos, p);
                    let mut hits = std::collections::HashSet::new();
                    for li in 0..MAX_LAYERS {
                        let y0 = lanes.top() + li as f32 * layer_h - scroll_y;
                        let y1 = y0 + layer_h;
                        if y1 < lanes.top() || y0 > lanes.bottom() {
                            continue;
                        }
                        if let Some(layer) = st.scene.layers.get(li) {
                            for obj in &layer.objects {
                                let x0 = fx(obj.frame_start as f32);
                                let x1 = fx(obj.frame_end as f32 + 1.0);
                                let obj_rect = Rect::from_min_max(pos2(x0, y0 + 2.0), pos2(x1.max(x0 + 3.0), y1 - 2.0));
                                if sel_rect.intersects(obj_rect) {
                                    hits.insert(obj.id);
                                }
                            }
                        }
                    }
                    st.selected_objects = hits;
                    st.selected = st.selected_objects.iter().next().copied();
                }
            }
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
                    if p.x > lanes.right() - EDGE_AUTOSCROLL_MARGIN_PX {
                        st.timeline.scroll_x += EDGE_AUTOSCROLL_SPEED_PX / ppf;
                    } else if p.x < lanes.left() + EDGE_AUTOSCROLL_MARGIN_PX {
                        st.timeline.scroll_x = (st.timeline.scroll_x - EDGE_AUTOSCROLL_SPEED_PX / ppf).max(0.0);
                    }
                    let mut new_start = (xf(p.x) as i64 - grab_offset).max(0) as Frame;
                    let new_layer = y_layer(p.y);

                    if let Some((cur_layer, obj)) = st.scene.find(id) {
                        let obj_len = obj.length();
                        let cur_start = obj.frame_start;
                        let is_multi = st.selected_objects.contains(&id) && st.selected_objects.len() > 1;
                        let ignore_ids: std::collections::HashSet<ObjectId> = if is_multi {
                            st.selected_objects.clone()
                        } else {
                            std::iter::once(id).collect()
                        };

                        // スナップ吸着判定（始点または終点）
                        let snap_start = find_snap_frame(st, new_start, 9.0, &ignore_ids);
                        let snap_end = find_snap_frame(st, new_start + obj_len, 9.0, &ignore_ids);
                        if let Some(s) = snap_start {
                            new_start = s;
                            st.timeline.active_snap_line = Some(s);
                        } else if let Some(e) = snap_end {
                            new_start = e.saturating_sub(obj_len);
                            st.timeline.active_snap_line = Some(e);
                        } else {
                            st.timeline.active_snap_line = None;
                        }

                        if is_multi {
                            let delta_frame = new_start as i64 - cur_start as i64;
                            let delta_layer = new_layer as isize - cur_layer as isize;
                            if delta_frame != 0 || delta_layer != 0 {
                                let mut valid = true;
                                let mut items_to_move = Vec::new();
                                for &sel_id in &st.selected_objects {
                                    if let Some((l, o)) = st.scene.find(sel_id) {
                                        let tgt_l = l as isize + delta_layer;
                                        let tgt_s = o.frame_start as i64 + delta_frame;
                                        let tgt_e = o.frame_end as i64 + delta_frame;
                                        if tgt_l < 0 || tgt_l >= MAX_LAYERS as isize || tgt_s < 0 {
                                            valid = false;
                                            break;
                                        }
                                        items_to_move.push((sel_id, l, o.clone(), tgt_l as usize, tgt_s as Frame, tgt_e as Frame));
                                    }
                                }

                                if valid {
                                    for (_, _, _, tgt_l, tgt_s, tgt_e) in &items_to_move {
                                        if let Some(target_layer) = st.scene.layers.get(*tgt_l) {
                                            for other in &target_layer.objects {
                                                if !st.selected_objects.contains(&other.id) {
                                                    if !(*tgt_e < other.frame_start || *tgt_s > other.frame_end) {
                                                        valid = false;
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                        if !valid {
                                            break;
                                        }
                                    }
                                }

                                if valid {
                                    for (sel_id, _, _, _, _, _) in &items_to_move {
                                        let _ = st.scene.remove_object(*sel_id);
                                    }
                                    for (_, _, mut o, tgt_l, tgt_s, tgt_e) in items_to_move {
                                        o.frame_start = tgt_s;
                                        o.frame_end = tgt_e;
                                        st.scene.insert_object(tgt_l, o);
                                    }
                                    st.dirty = true;
                                }
                            }
                        } else {
                            if (cur_layer, cur_start) != (new_layer, new_start)
                                && st.scene.move_object(id, new_layer, new_start)
                            {
                                st.dirty = true;
                            }
                        }
                    }
                }
            }
            TimelineDrag::ResizeLeft { id } => {
                if let Some(p) = resp.interact_pointer_pos() {
                    let mut new_start = xf(p.x);
                    let ignore_ids: std::collections::HashSet<ObjectId> = std::iter::once(id).collect();
                    if let Some(snap_f) = find_snap_frame(st, new_start, 9.0, &ignore_ids) {
                        new_start = snap_f;
                        st.timeline.active_snap_line = Some(snap_f);
                    } else {
                        st.timeline.active_snap_line = None;
                    }
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
                    let mut new_end = xf(p.x);
                    let ignore_ids: std::collections::HashSet<ObjectId> = std::iter::once(id).collect();
                    if let Some(snap_f) = find_snap_frame(st, new_end + 1, 9.0, &ignore_ids) {
                        new_end = snap_f.saturating_sub(1);
                        st.timeline.active_snap_line = Some(snap_f);
                    } else {
                        st.timeline.active_snap_line = None;
                    }
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
        st.commit_continuous_edit();
        st.timeline.drag = None;
        st.timeline.active_snap_line = None;
    }
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let shift = ui.input(|i| i.modifiers.shift);
            if btn_snap.contains(p) {
                st.timeline.snap_enabled = !st.timeline.snap_enabled;
                st.status = if st.timeline.snap_enabled {
                    "🧲 スナップ（吸着）を有効にしました".into()
                } else {
                    "🧲 スナップ（吸着）を無効にしました".into()
                };
            } else if corner.contains(p) {
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
                if shift {
                    st.toggle_select(id);
                } else {
                    st.select_single(id);
                }
            } else if lanes.contains(p) {
                st.clear_selection();
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
    painter.rect_filled(full, 0.0, flip(Color32::from_gray(28)));
    let first_row = (scroll_y / layer_h).floor() as usize;
    let rows = (lanes.height() / layer_h).ceil() as usize + 1;
    let visible_rows = first_row..(first_row + rows).min(MAX_LAYERS);

    // レーン背景（ライトモード／ダークモードそれぞれで明確なコントラスト比を持つ交互色分け）
    let lane_painter = painter.with_clip_rect(lanes);
    let is_dark = theme::is_dark();
    for li in visible_rows.clone() {
        let y = row_top(li);
        let r = Rect::from_min_size(pos2(lanes.left(), y), vec2(lanes.width(), layer_h));
        let row_bg = if is_dark {
            if li % 2 == 0 {
                Color32::from_rgb(23, 25, 31)
            } else {
                Color32::from_rgb(31, 34, 43)
            }
        } else {
            if li % 2 == 0 {
                Color32::from_rgb(255, 255, 255)
            } else {
                Color32::from_rgb(233, 238, 247)
            }
        };
        lane_painter.rect_filled(r, 0.0, row_bg);
        // 各レーンの下辺に薄い区切り線
        let divider_color = if is_dark {
            Color32::from_gray(38)
        } else {
            Color32::from_rgb(216, 222, 234)
        };
        lane_painter.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, divider_color));
    }
    // シーン長より後ろを暗く
    let end_x = fx(st.scene.settings.length as f32);
    if end_x < lanes.right() {
        lane_painter.rect_filled(
            Rect::from_min_max(pos2(end_x.max(lanes.left()), lanes.top()), lanes.max),
            0.0,
            Color32::from_black_alpha(if theme::is_dark() { 90 } else { 40 }),
        );
    }

    // オブジェクト用サムネイルおよびオーディオ波形の事前取得 (プレビュー ON 時のみ)
    let mut clip_textures: HashMap<String, egui::TextureId> = HashMap::new();
    let mut clip_waveforms: HashMap<String, std::sync::Arc<au2_media::AudioWaveformPeaks>> = HashMap::new();
    if show_previews {
        let egui_ctx = ui.ctx().clone();
        let mut media_paths: Vec<String> = Vec::new();
        let mut audio_paths: Vec<String> = Vec::new();
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
                    if matches!(src.kind, EffectKind::AudioFile | EffectKind::VideoFile) {
                        if let Some(p) = effect_file_path(src).filter(|s| !s.is_empty()) {
                            let s = p.to_string();
                            if !audio_paths.contains(&s) {
                                audio_paths.push(s);
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
        for path_str in audio_paths {
            if let Some(wf) = st.get_audio_waveform(&path_str) {
                clip_waveforms.insert(path_str, wf);
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
            let selected = st.selected == Some(obj.id) || st.selected_objects.contains(&obj.id);
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
                    let fps = st.scene.settings.frame_rate.as_f64().max(1.0);
                    draw_clip_preview(&body_painter, body_rect, lanes, obj, &clip_textures, &clip_waveforms, fps, ppf, dimmed);
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

                if obj.clipping {
                    lane_painter.line_segment(
                        [pos2(r.left() + 1.5, r.top()), pos2(r.left() + 1.5, r.bottom())],
                        Stroke::new(3.0_f32, Color32::from_rgb(255, 75, 75)),
                    );
                }

                // ヘッダーラベル + アイコン + fx バッジ（高さを正確に統一）
                let text_clip = header_rect.shrink2(vec2(3.0, 0.0)).intersect(lanes);
                if text_clip.width() > 8.0 {
                    let hp = painter.with_clip_rect(text_clip);
                    let icon = clip_kind_icon(obj);
                    let left_x = header_rect.left().max(lanes.left()) + if obj.clipping { 8.0 } else { 5.0 };
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
                    } else if obj.clipping {
                        format!("↳ {}", obj.display_name())
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
                if obj.clipping {
                    lane_painter.line_segment(
                        [pos2(r.left() + 1.5, r.top()), pos2(r.left() + 1.5, r.bottom())],
                        Stroke::new(3.0_f32, Color32::from_rgb(255, 75, 75)),
                    );
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
                    } else if obj.clipping {
                        format!("↳ {}", obj.display_name())
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
    painter.rect_filled(ruler, 0.0, flip(Color32::from_rgb(26, 28, 35)));
    painter.line_segment([ruler.left_bottom(), ruler.right_bottom()], Stroke::new(1.0_f32, theme::border()));
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
        ruler_painter.line_segment([pos2(x, ruler.bottom() - h), pos2(x, ruler.bottom())], Stroke::new(1.0_f32, flip(Color32::from_gray(120))));
        if major {
            ruler_painter.text(pos2(x + 3.0, ruler.top() + 2.0), Align2::LEFT_TOP, fmt_time(f, rate), FontId::monospace(10.0), theme::text_muted());
        }
        f += minor;
    }

    // レイヤーヘッダ（レーン背景と連動した交互色分け）
    let header_painter = painter.with_clip_rect(headers);
    for li in visible_rows {
        let y = row_top(li);
        let r = Rect::from_min_size(pos2(headers.left(), y), vec2(HEADER_W, layer_h));
        let h_row_bg = if is_dark {
            if li % 2 == 0 {
                Color32::from_rgb(26, 28, 36)
            } else {
                Color32::from_rgb(33, 36, 46)
            }
        } else {
            if li % 2 == 0 {
                Color32::from_rgb(250, 251, 254)
            } else {
                Color32::from_rgb(235, 239, 247)
            }
        };
        header_painter.rect_filled(r, 0.0, h_row_bg);

        let is_selected_layer = st.timeline.selected_layer == li;
        if is_selected_layer {
            header_painter.rect_filled(r, 0.0, if is_dark { Color32::from_rgb(42, 48, 65) } else { Color32::from_rgb(220, 230, 248) });
        }

        let layer = &st.scene.layers[li];
        let color = if layer.visible { theme::text() } else { theme::text_muted() };
        let base_name = layer.name.clone().unwrap_or_else(|| format!("Layer {}", li + 1));

        // レイヤーバッジ [L1]
        let badge_rect = Rect::from_min_size(pos2(r.left() + 5.0, r.center().y - 8.0), vec2(24.0, 16.0));
        let badge_bg = if is_selected_layer {
            Color32::from_rgb(59, 130, 246)
        } else if is_dark {
            Color32::from_rgb(38, 42, 54)
        } else {
            Color32::from_rgb(218, 226, 242)
        };
        header_painter.rect_filled(badge_rect, 3.0, badge_bg);
        header_painter.text(badge_rect.center(), Align2::CENTER_CENTER, format!("L{}", li + 1), FontId::monospace(10.0), if is_selected_layer { Color32::WHITE } else { theme::accent_text() });

        // レイヤー名（ボタンと重ならないようクリップ）
        let name_clip = Rect::from_min_max(pos2(badge_rect.right() + 6.0, r.top()), pos2(r.right() - 40.0, r.bottom()));
        header_painter.with_clip_rect(name_clip).text(pos2(badge_rect.right() + 6.0, r.center().y), Align2::LEFT_CENTER, base_name, FontId::proportional(11.5), color);

        // 状態操作ボタン (右端：👁️ 表示/非表示トグル ＆ 🔒 ロックトグル)
        let vis_btn_rect = Rect::from_min_size(pos2(r.right() - 38.0, r.center().y - 8.0), vec2(17.0, 16.0));
        let lock_btn_rect = Rect::from_min_size(pos2(r.right() - 19.0, r.center().y - 8.0), vec2(17.0, 16.0));
        let vis_hovered = hover_pos.is_some_and(|p| vis_btn_rect.contains(p));
        let lock_hovered = hover_pos.is_some_and(|p| lock_btn_rect.contains(p));

        if vis_hovered {
            header_painter.rect_filled(vis_btn_rect, 2.5, Color32::from_white_alpha(35));
        }
        if lock_hovered {
            header_painter.rect_filled(lock_btn_rect, 2.5, Color32::from_white_alpha(35));
        }

        // 👁️ 表示/非表示アイコン
        let vis_icon = if layer.visible { "👁" } else { "🚫" };
        let vis_color = if layer.visible {
            if vis_hovered { Color32::WHITE } else { theme::text_muted() }
        } else {
            Color32::from_rgb(255, 90, 90)
        };
        header_painter.text(vis_btn_rect.center(), Align2::CENTER_CENTER, vis_icon, FontId::proportional(11.0), vis_color);

        // 🔒 ロックアイコン
        let lock_icon = if layer.locked { "🔒" } else { "🔓" };
        let lock_color = if layer.locked {
            Color32::from_rgb(250, 204, 21)
        } else if lock_hovered {
            Color32::WHITE
        } else {
            Color32::from_white_alpha(40)
        };
        header_painter.text(lock_btn_rect.center(), Align2::CENTER_CENTER, lock_icon, FontId::proportional(10.5), lock_color);
        header_painter.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, theme::border()));
    }

    // 左上コーナー：クリップ内プレビュー ON/OFF スタイリッシュピルボタン
    let corner_hovered = hover_pos.is_some_and(|p| corner.contains(p));
    let corner_bg = if is_dark {
        Color32::from_rgb(22, 24, 30)
    } else {
        Color32::from_rgb(241, 244, 250)
    };
    painter.rect_filled(corner, 0.0, corner_bg);
    painter.line_segment([corner.right_top(), corner.right_bottom()], Stroke::new(1.0_f32, theme::border()));
    painter.line_segment([corner.left_bottom(), corner.right_bottom()], Stroke::new(1.0_f32, theme::border()));

    let btn_rect = corner.shrink2(vec2(3.0, 2.5));
    let (btn_fill, btn_stroke, text_color) = if show_previews {
        if corner_hovered {
            (Color32::from_rgb(45, 110, 245), Stroke::new(1.0_f32, Color32::from_rgb(147, 197, 253)), Color32::WHITE)
        } else {
            (Color32::from_rgb(37, 99, 235), Stroke::new(1.0_f32, Color32::from_rgb(96, 165, 250)), Color32::WHITE)
        }
    } else if corner_hovered {
        (flip(Color32::from_rgb(52, 56, 68)), Stroke::new(1.0_f32, theme::border()), theme::text())
    } else {
        (flip(Color32::from_rgb(38, 41, 50)), Stroke::new(1.0_f32, theme::border()), theme::text_muted())
    };
    painter.rect_filled(btn_rect, 4.0, btn_fill);
    painter.rect_stroke(btn_rect, 4.0, btn_stroke);

    let toggle_label = if show_previews {
        "🖼 プレビュー ON"
    } else {
        "🖼 プレビュー OFF"
    };
    painter.text(
        btn_rect.center(),
        Align2::CENTER_CENTER,
        toggle_label,
        FontId::proportional(10.5),
        text_color,
    );

    // スナップ（吸着）ガイド線
    if let Some(snap_f) = st.timeline.active_snap_line {
        let sx = fx(snap_f as f32);
        if sx >= lanes.left() && sx <= lanes.right() {
            painter.line_segment(
                [pos2(sx, ruler.top()), pos2(sx, lanes.bottom())],
                Stroke::new(2.0_f32, Color32::from_rgb(0, 225, 255)),
            );
            painter.add(Shape::convex_polygon(
                vec![pos2(sx - 4.0, ruler.top()), pos2(sx + 4.0, ruler.top()), pos2(sx, ruler.top() + 7.0)],
                Color32::from_rgb(0, 225, 255),
                Stroke::NONE,
            ));
        }
    }

    // タイムライン上の矩形選択範囲（ラバーバンド枠）
    if let Some(TimelineDrag::BoxSelect { start_pos, current_pos }) = st.timeline.drag {
        let sel_rect = Rect::from_two_pos(start_pos, current_pos);
        painter.rect_filled(sel_rect, 2.0, Color32::from_rgba_unmultiplied(40, 140, 255, 45));
        painter.rect_stroke(sel_rect, 2.0, Stroke::new(1.5_f32, Color32::from_rgb(80, 180, 255)));
    }

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

    // ---- 外部ファイル（エクスプローラー等）のドラッグ＆ドロップ・ホバープレビュー描画 ----
    let hovered_files = ui.input(|i| i.raw.hovered_files.clone());
    if !hovered_files.is_empty() {
        if let Some(hp) = hover_pos {
            if lanes.contains(hp) {
                let hover_layer = y_layer(hp.y);
                let raw_f = xf(hp.x);
                let snap_f = find_snap_frame(st, raw_f, 12.0, &std::collections::HashSet::new()).unwrap_or(raw_f);

                let default_duration = (st.scene.settings.frame_rate.as_f64() * 5.0).round().max(30.0) as Frame; // デフォルト5秒分
                let sx0 = fx(snap_f as f32);
                let sx1 = fx((snap_f + default_duration) as f32).max(sx0 + 36.0);

                let num_items = hovered_files.len().min(8);
                for idx in 0..num_items {
                    let cur_layer = (hover_layer + idx).min(MAX_LAYERS - 1);
                    let ly_top = lanes.top() - scroll_y + (cur_layer as f32) * layer_h;
                    let ly_bottom = ly_top + layer_h;

                    // 可視領域内の場合のみ描画
                    if ly_bottom >= lanes.top() && ly_top <= lanes.bottom() {
                        let drop_rect = Rect::from_min_max(
                            pos2(sx0.max(lanes.left()), ly_top + 1.0),
                            pos2(sx1.min(lanes.right()), ly_bottom - 1.0),
                        );
                        if drop_rect.width() > 2.0 {
                            painter.rect_filled(drop_rect, 3.5, Color32::from_rgba_unmultiplied(46, 134, 222, 90));
                            painter.rect_stroke(drop_rect, 3.5, Stroke::new(1.8_f32, Color32::from_rgb(56, 189, 248)));
                        }
                    }
                }

                // スナップ吸着ライン（縦線）
                if sx0 >= lanes.left() && sx0 <= lanes.right() {
                    painter.line_segment(
                        [pos2(sx0, lanes.top()), pos2(sx0, lanes.bottom())],
                        Stroke::new(2.0_f32, Color32::from_rgb(56, 189, 248)),
                    );
                }

                // ガイドバッジ（ツールチップ）
                let badge_pos = pos2(
                    (sx0 + 8.0).clamp(lanes.left() + 8.0, lanes.right() - 200.0),
                    (hp.y - 28.0).max(lanes.top() + 4.0),
                );
                let label_text = if hovered_files.len() == 1 {
                    format!("📥 Layer {} : {}F に配置", hover_layer + 1, snap_f)
                } else {
                    format!("📥 Layer {}〜 : {}F に配置 ({}件)", hover_layer + 1, snap_f, hovered_files.len())
                };
                let galley = painter.layout_no_wrap(label_text, FontId::proportional(11.0), Color32::WHITE);
                let badge_rect = Rect::from_min_size(badge_pos, vec2(galley.size().x + 16.0, 22.0));
                painter.rect_filled(badge_rect, 4.0, Color32::from_rgba_unmultiplied(15, 23, 42, 230));
                painter.rect_stroke(badge_rect, 4.0, Stroke::new(1.0_f32, Color32::from_rgb(56, 189, 248)));
                painter.galley(pos2(badge_pos.x + 8.0, badge_pos.y + 4.0), galley, Color32::WHITE);
            }
        }
    }

    // 下部水平スクロールバー
    painter.rect_filled(h_scrollbar, 0.0, flip(Color32::from_gray(24)));
    painter.line_segment([h_scrollbar.left_top(), h_scrollbar.right_top()], Stroke::new(1.0_f32, theme::border()));
    let thumb_hovered = hover_pos.is_some_and(|p| thumb_rect.contains(p));
    let thumb_bg = if matches!(st.timeline.drag, Some(TimelineDrag::ScrollbarThumb { .. })) {
        theme::accent()
    } else if thumb_hovered {
        flip(Color32::from_gray(120))
    } else {
        flip(Color32::from_gray(75))
    };
    painter.rect_filled(thumb_rect, 3.0, thumb_bg);
    painter.rect_stroke(thumb_rect, 3.0, Stroke::new(1.0_f32, theme::border()));

    // 右下ズームツールバー
    painter.rect_filled(zoom_bar, 0.0, flip(Color32::from_rgb(24, 26, 32)));
    painter.line_segment([zoom_bar.left_top(), zoom_bar.right_top()], Stroke::new(1.0_f32, theme::border()));
    painter.line_segment([zoom_bar.left_top(), zoom_bar.left_bottom()], Stroke::new(1.0_f32, theme::border()));

    // [🧲] スナップ（吸着）切り替えボタン
    let snap_hovered = hover_pos.is_some_and(|p| btn_snap.contains(p));
    let snap_on = st.timeline.snap_enabled;
    let snap_bg = if snap_on {
        if snap_hovered { Color32::from_rgb(32, 108, 196) } else { Color32::from_rgb(24, 88, 164) }
    } else {
        if snap_hovered { flip(Color32::from_gray(56)) } else { flip(Color32::from_gray(38)) }
    };
    painter.rect_filled(btn_snap, 3.0, snap_bg);
    painter.rect_stroke(
        btn_snap,
        3.0,
        Stroke::new(1.0_f32, if snap_on { Color32::from_rgb(80, 180, 255) } else { theme::border() }),
    );
    let snap_label = if snap_on { "🧲 ON" } else { "🧲 OFF" };
    painter.text(
        btn_snap.center(),
        Align2::CENTER_CENTER,
        snap_label,
        FontId::proportional(10.0),
        if snap_on { Color32::WHITE } else { theme::text_muted() },
    );

    // [⊡ 全体] ボタン (Fit)
    let fit_hovered = hover_pos.is_some_and(|p| btn_fit.contains(p));
    let fit_bg = if fit_hovered { flip(Color32::from_gray(56)) } else { flip(Color32::from_gray(38)) };
    painter.rect_filled(btn_fit, 3.0, fit_bg);
    painter.rect_stroke(btn_fit, 3.0, Stroke::new(1.0_f32, if fit_hovered { flip(Color32::from_gray(90)) } else { theme::border() }));
    painter.text(
        btn_fit.center(),
        Align2::CENTER_CENTER,
        "⊡ 全体",
        FontId::proportional(10.0),
        if fit_hovered { theme::text() } else { theme::text_muted() },
    );

    // [-] ボタン
    let minus_hovered = hover_pos.is_some_and(|p| btn_minus.contains(p));
    let minus_bg = if minus_hovered { flip(Color32::from_gray(56)) } else { flip(Color32::from_gray(38)) };
    painter.rect_filled(btn_minus, 3.0, minus_bg);
    painter.rect_stroke(btn_minus, 3.0, Stroke::new(1.0_f32, if minus_hovered { flip(Color32::from_gray(90)) } else { theme::border() }));
    painter.text(
        btn_minus.center(),
        Align2::CENTER_CENTER,
        "−",
        FontId::proportional(11.0),
        if minus_hovered { theme::text() } else { theme::text_muted() },
    );

    // スライダートラック (溝)
    painter.rect_filled(slider_track, 2.0, flip(Color32::from_gray(18)));
    painter.rect_stroke(slider_track, 2.0, Stroke::new(1.0_f32, theme::border()));

    // スライダーつまみ (Knob)
    let is_zoom_dragging = matches!(st.timeline.drag, Some(TimelineDrag::ZoomSlider));
    let knob_hovered = hover_pos.is_some_and(|p| knob_rect.contains(p)) || is_zoom_dragging;
    let knob_bg = if is_zoom_dragging {
        theme::accent()
    } else if knob_hovered {
        flip(Color32::from_gray(150))
    } else {
        flip(Color32::from_gray(100))
    };
    painter.rect_filled(knob_rect, 2.5, knob_bg);
    painter.rect_stroke(knob_rect, 2.5, Stroke::new(1.0_f32, theme::border()));

    // [+] ボタン
    let plus_hovered = hover_pos.is_some_and(|p| btn_plus.contains(p));
    let plus_bg = if plus_hovered { flip(Color32::from_gray(56)) } else { flip(Color32::from_gray(38)) };
    painter.rect_filled(btn_plus, 3.0, plus_bg);
    painter.rect_stroke(btn_plus, 3.0, Stroke::new(1.0_f32, if plus_hovered { flip(Color32::from_gray(90)) } else { theme::border() }));
    painter.text(
        btn_plus.center(),
        Align2::CENTER_CENTER,
        "+",
        FontId::proportional(11.0),
        if plus_hovered { theme::text() } else { theme::text_muted() },
    );

    // 倍率テキスト (例: 100%)
    let zoom_pct = (ppf * 100.0).round() as i64;
    painter.text(
        pos2(lbl_percent.right() - 2.0, lbl_percent.center().y),
        Align2::RIGHT_CENTER,
        format!("{}%", zoom_pct),
        FontId::monospace(9.5),
        theme::text_muted(),
    );

    // 左下角
    painter.rect_filled(corner_bl, 0.0, flip(Color32::from_rgb(24, 26, 32)));
    painter.line_segment([corner_bl.left_top(), corner_bl.right_top()], Stroke::new(1.0_f32, theme::border()));
    painter.line_segment([corner_bl.right_top(), corner_bl.right_bottom()], Stroke::new(1.0_f32, theme::border()));
}

