//! EditorState の単体テスト群。

use super::*;
use au2_core::TimelineObject;

#[test]
fn test_undo_redo_object_add_and_delete() {
    let mut st = EditorState::default();
    assert_eq!(st.scene.object_count(), 0);
    assert!(!st.can_undo());

    // オブジェクトを追加
    let obj = TimelineObject::new(0, 30);
    st.add_object_at(0, obj);
    assert_eq!(st.scene.object_count(), 1);
    assert!(st.can_undo());
    assert_eq!(st.undo_description().unwrap(), "オブジェクト「(空のオブジェクト)」を追加");

    // Undo
    assert!(st.undo());
    assert_eq!(st.scene.object_count(), 0);
    assert!(st.can_redo());
    assert_eq!(st.redo_description().unwrap(), "オブジェクト「(空のオブジェクト)」を追加");

    // Redo
    assert!(st.redo());
    assert_eq!(st.scene.object_count(), 1);
    assert!(st.selected.is_some());

    // 削除
    let count = st.delete_selected_objects();
    assert_eq!(count, 1);
    assert_eq!(st.scene.object_count(), 0);

    // 削除の Undo
    assert!(st.undo());
    assert_eq!(st.scene.object_count(), 1);
}

#[test]
fn test_multi_select_and_batch_delete_with_undo() {
    let mut st = EditorState::default();
    let obj1 = TimelineObject::new(0, 20);
    let obj2 = TimelineObject::new(30, 50);
    let id1 = obj1.id;
    let id2 = obj2.id;
    st.scene.insert_object(0, obj1);
    st.scene.insert_object(1, obj2);
    assert_eq!(st.scene.object_count(), 2);

    // 複数選択
    st.select_single(id1);
    assert_eq!(st.selected_objects.len(), 1);
    st.toggle_select(id2);
    assert_eq!(st.selected_objects.len(), 2);

    // 一括削除
    let deleted = st.delete_selected_objects();
    assert_eq!(deleted, 2);
    assert_eq!(st.scene.object_count(), 0);
    assert!(st.selected_objects.is_empty());

    // Undo で 2 件とも復元
    assert!(st.undo());
    assert_eq!(st.scene.object_count(), 2);
    assert!(st.scene.find(id1).is_some());
    assert!(st.scene.find(id2).is_some());
}

#[test]
fn test_continuous_edit_commit() {
    let mut st = EditorState::default();
    let obj = TimelineObject::new(10, 40);
    let id = obj.id;
    st.scene.insert_object(0, obj);

    // ドラッグ開始
    st.begin_continuous_edit("クリップ移動");
    assert!(!st.can_undo()); // コミット前はまだ Undo スタックに積まれない

    // 移動実行
    assert!(st.scene.move_object(id, 0, 50));
    assert_eq!(st.scene.find(id).unwrap().1.frame_start, 50);

    // ドラッグ終了（コミット）
    st.commit_continuous_edit();
    assert!(st.can_undo());
    assert_eq!(st.undo_description().unwrap(), "クリップ移動");

    // Undo で移動前の 10 フレームに戻る
    assert!(st.undo());
    assert_eq!(st.scene.find(id).unwrap().1.frame_start, 10);

    // Redo で再度 50 フレームへ移動
    assert!(st.redo());
    assert_eq!(st.scene.find(id).unwrap().1.frame_start, 50);
}

#[test]
fn test_copy_and_paste_with_undo() {
    let mut st = EditorState::default();
    let obj1 = TimelineObject::new(10, 30);
    let obj2 = TimelineObject::new(20, 50);
    let id1 = obj1.id;
    let id2 = obj2.id;
    st.scene.insert_object(0, obj1);
    st.scene.insert_object(1, obj2);

    // 2つのオブジェクトを選択してコピー
    st.select_single(id1);
    st.toggle_select(id2);
    assert_eq!(st.copy_selected(), 2);
    assert_eq!(st.clipboard.len(), 2);

    // フレーム 100 に貼り付け
    let pasted = st.paste_clipboard(Some(2), Some(100));
    assert_eq!(pasted.len(), 2);
    assert_eq!(st.scene.object_count(), 4);

    // 貼り付けられたオブジェクトの相対関係（レイヤー差・フレーム差）の検証
    let p1 = st.scene.find(pasted[0]).unwrap();
    let p2 = st.scene.find(pasted[1]).unwrap();
    assert_eq!(p1.0, 2); // layer 2
    assert_eq!(p1.1.frame_start, 100);
    assert_eq!(p1.1.frame_end, 120);

    assert_eq!(p2.0, 3); // layer 3 (相対差 +1)
    assert_eq!(p2.1.frame_start, 110); // 100 + (20 - 10)
    assert_eq!(p2.1.frame_end, 140);

    // Undo で貼り付けが取り消される
    assert!(st.undo());
    assert_eq!(st.scene.object_count(), 2);
    assert!(st.scene.find(pasted[0]).is_none());

    // Redo で再度貼り付けられる
    assert!(st.redo());
    assert_eq!(st.scene.object_count(), 4);
}

#[test]
fn test_split_at_playhead_with_undo() {
    let mut st = EditorState::default();
    let obj = TimelineObject::new(0, 60);
    let id = obj.id;
    st.scene.insert_object(0, obj);
    st.select_single(id);

    // 再生ヘッドを 30 フレームに設定して分割
    st.set_frame(30);
    assert_eq!(st.split_at_playhead(), 1);
    assert_eq!(st.scene.object_count(), 2);

    // 左側 (0..=29) と右側 (30..=60) に分割されていることを確認
    let left = st.scene.find(id).unwrap().1;
    assert_eq!(left.frame_start, 0);
    assert_eq!(left.frame_end, 29);

    let right_id = st.selected.unwrap();
    assert_ne!(right_id, id);
    let right = st.scene.find(right_id).unwrap().1;
    assert_eq!(right.frame_start, 30);
    assert_eq!(right.frame_end, 60);

    // Undo で結合状態に戻る
    assert!(st.undo());
    assert_eq!(st.scene.object_count(), 1);
    let restored = st.scene.find(id).unwrap().1;
    assert_eq!(restored.frame_start, 0);
    assert_eq!(restored.frame_end, 60);
}
