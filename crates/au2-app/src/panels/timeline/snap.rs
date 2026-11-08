//! タイムライン上のスナップ（吸着）フレーム候補の探索処理。

use au2_core::{Frame, ObjectId};

use crate::state::EditorState;

/// オブジェクト境界や再生ヘッドへのスナップ（吸着）フレーム候補を探索します。
pub fn find_snap_frame(
    st: &EditorState,
    candidate_frame: Frame,
    threshold_px: f32,
    ignore_ids: &std::collections::HashSet<ObjectId>,
) -> Option<Frame> {
    if !st.timeline.snap_enabled {
        return None;
    }
    let ppf = st.timeline.px_per_frame;
    let threshold_frames = (threshold_px / ppf).max(1.0).ceil() as Frame;

    let mut best_snap: Option<(Frame, Frame)> = None;

    let mut check_target = |target: Frame| {
        let diff = (candidate_frame as i64 - target as i64).abs() as Frame;
        if diff <= threshold_frames {
            if let Some((best_diff, _)) = best_snap {
                if diff < best_diff {
                    best_snap = Some((diff, target));
                }
            } else {
                best_snap = Some((diff, target));
            }
        }
    };

    // 1. 0 フレーム
    check_target(0);
    // 2. 現在の再生ヘッド位置
    check_target(st.frame);

    // 3. 他オブジェクトの境界（始点および終点 + 1）
    for layer in &st.scene.layers {
        for obj in &layer.objects {
            if ignore_ids.contains(&obj.id) {
                continue;
            }
            check_target(obj.frame_start);
            check_target(obj.frame_end + 1);
        }
    }

    best_snap.map(|(_, target)| target)
}
