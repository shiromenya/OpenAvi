//! タイムラインのズーム・座標マッピング・D&D配置に関する単体テスト。

use au2_core::{Frame, MAX_LAYERS};
use eframe::egui::{pos2, vec2, Rect};

use crate::state::EditorState;
use super::types::*;
use super::snap::find_snap_frame;

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
    let frame_under_mouse = (mouse_x - lane_left) / v.px_per_frame + v.scroll_x;
    assert!((frame_under_mouse - 200.0).abs() < 1e-4);
}

#[test]
fn test_timeline_drop_coordinate_and_layer_mapping() {
    let mut st = EditorState::default();
    st.timeline.px_per_frame = 2.0;
    st.timeline.scroll_x = 50.0;
    st.timeline.scroll_y = 10.0;

    let lanes_rect = Rect::from_min_size(pos2(120.0, 50.0), vec2(800.0, 400.0));
    st.timeline.lanes_rect = Some(lanes_rect);

    let mouse_pos = pos2(220.0, 100.0);
    assert!(st.timeline.lanes_rect.is_some_and(|r| r.contains(mouse_pos)));

    let layer_h = LAYER_H_PREVIEW; // 46.0
    let raw_layer = (((mouse_pos.y - lanes_rect.top() + st.timeline.scroll_y) / layer_h).floor().max(0.0) as usize).min(MAX_LAYERS - 1);
    let raw_frame = ((mouse_pos.x - lanes_rect.left()) / st.timeline.px_per_frame + st.timeline.scroll_x).max(0.0) as Frame;

    // (100 - 50 + 10) / 46 = 60 / 46 = 1.304 -> Layer 1
    assert_eq!(raw_layer, 1);
    // (220 - 120) / 2.0 + 50.0 = 100 / 2.0 + 50.0 = 100 フレーム
    assert_eq!(raw_frame, 100);

    let snap_f = find_snap_frame(&st, raw_frame, 12.0, &std::collections::HashSet::new()).unwrap_or(raw_frame);
    assert_eq!(snap_f, 100);
}

#[test]
fn test_load_path_at_places_object_at_exact_layer_and_frame() {
    let mut st = EditorState::default();
    let temp_dir = std::env::temp_dir();
    let txt_file = temp_dir.join("test_drop_object.txt");
    std::fs::write(&txt_file, "テストテキストテロップ").expect("write test txt");

    st.load_path_at(&txt_file, 2, 75);

    let layer = &st.scene.layers[2];
    let placed_obj = layer.objects.iter().find(|o| o.frame_start == 75);
    assert!(placed_obj.is_some(), "Layer 2 の 75 フレーム目にオブジェクトが配置されること");

    let obj = placed_obj.unwrap();
    assert_eq!(obj.frame_start, 75);
    assert_eq!(obj.length(), 60);

    let _ = std::fs::remove_file(txt_file);
}
