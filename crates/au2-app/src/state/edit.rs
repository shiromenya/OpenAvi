//! 編集履歴（Undo/Redo）・選択管理・クリップボード・複製・分割・削除処理。

use au2_core::{Frame, ObjectId};

use super::{ClipboardItem, EditorState, HistorySnapshot};

impl EditorState {
    /// 現在のシーンおよび選択状態のスナップショットを作成します。
    pub fn capture_snapshot(&self, description: impl Into<String>) -> HistorySnapshot {
        HistorySnapshot {
            scene: self.scene.clone(),
            scenes: self.scenes.clone(),
            selected: self.selected,
            selected_objects: self.selected_objects.clone(),
            description: description.into(),
        }
    }

    /// 単発の変更操作（オブジェクトの追加・削除・分割など）の直前にスナップショットを記録します。
    pub fn push_undo(&mut self, description: impl Into<String>) {
        let snap = self.capture_snapshot(description);
        self.history.redo_stack.clear();
        self.history.undo_stack.push(snap);
        if self.history.undo_stack.len() > self.history.max_depth {
            self.history.undo_stack.remove(0);
        }
        self.dirty = true;
    }

    /// ドラッグ移動・伸縮・スライダー操作などの連続操作を開始する際に、変更前の状態を退避します。
    pub fn begin_continuous_edit(&mut self, description: impl Into<String>) {
        if self.history.pending_continuous_base.is_none() {
            self.history.pending_continuous_base = Some(self.capture_snapshot(description));
        }
    }

    /// 連続操作（ドラッグ移動・スライダーなど）が完了した際、実際に状態が変化していれば 1 件の Undo 履歴としてコミットします。
    pub fn commit_continuous_edit(&mut self) {
        if let Some(base) = self.history.pending_continuous_base.take() {
            if base.scene != self.scene || base.scenes != self.scenes {
                self.history.redo_stack.clear();
                self.history.undo_stack.push(base);
                if self.history.undo_stack.len() > self.history.max_depth {
                    self.history.undo_stack.remove(0);
                }
                self.dirty = true;
            }
        }
    }

    /// 連続操作をキャンセル（変更前にロールバック）します。
    #[allow(dead_code)]
    pub fn cancel_continuous_edit(&mut self) {
        if let Some(base) = self.history.pending_continuous_base.take() {
            self.scene = base.scene;
            self.scenes = base.scenes;
            self.selected = base.selected;
            self.selected_objects = base.selected_objects;
        }
    }

    /// 直前の操作を取り消します (Undo)。成功時 true。
    pub fn undo(&mut self) -> bool {
        self.history.pending_continuous_base = None;
        if let Some(prev) = self.history.undo_stack.pop() {
            let desc = prev.description.clone();
            let current = self.capture_snapshot(desc.clone());
            self.history.redo_stack.push(current);
            if self.history.redo_stack.len() > self.history.max_depth {
                self.history.redo_stack.remove(0);
            }
            self.scene = prev.scene;
            self.scenes = prev.scenes;
            self.selected = prev.selected;
            self.selected_objects = prev.selected_objects;
            self.dirty = true;
            self.status = format!("「{desc}」を取り消しました (Undo)");
            true
        } else {
            false
        }
    }

    /// 取り消した操作をやり直します (Redo)。成功時 true。
    pub fn redo(&mut self) -> bool {
        self.history.pending_continuous_base = None;
        if let Some(next) = self.history.redo_stack.pop() {
            let desc = next.description.clone();
            let current = self.capture_snapshot(desc.clone());
            self.history.undo_stack.push(current);
            if self.history.undo_stack.len() > self.history.max_depth {
                self.history.undo_stack.remove(0);
            }
            self.scene = next.scene;
            self.scenes = next.scenes;
            self.selected = next.selected;
            self.selected_objects = next.selected_objects;
            self.dirty = true;
            self.status = format!("「{desc}」をやり直しました (Redo)");
            true
        } else {
            false
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.history.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.history.redo_stack.is_empty()
    }

    pub fn undo_description(&self) -> Option<&str> {
        self.history.undo_stack.last().map(|s| s.description.as_str())
    }

    pub fn redo_description(&self) -> Option<&str> {
        self.history.redo_stack.last().map(|s| s.description.as_str())
    }

    /// 単一オブジェクトを選択状態にします。
    pub fn select_single(&mut self, id: ObjectId) {
        self.selected = Some(id);
        self.selected_objects.clear();
        self.selected_objects.insert(id);
    }

    /// オブジェクトの選択状態を反転（トグル）します（Shift+クリック用）。
    pub fn toggle_select(&mut self, id: ObjectId) {
        if self.selected_objects.contains(&id) {
            self.selected_objects.remove(&id);
            if self.selected == Some(id) {
                self.selected = self.selected_objects.iter().next().copied();
            }
        } else {
            self.selected_objects.insert(id);
            self.selected = Some(id);
        }
    }

    /// タイムライン上の全オブジェクトを選択します (Ctrl+A)。
    pub fn select_all(&mut self) {
        self.selected_objects.clear();
        for layer in &self.scene.layers {
            for obj in &layer.objects {
                self.selected_objects.insert(obj.id);
            }
        }
        self.selected = self.selected_objects.iter().next().copied();
        self.status = format!("{} 個のオブジェクトを選択しました", self.selected_objects.len());
    }

    /// オブジェクト選択をすべて解除します (Esc)。
    pub fn clear_selection(&mut self) {
        self.selected = None;
        self.selected_objects.clear();
    }

    /// 選択中のすべてのオブジェクトを一括削除します（Undo 履歴に記録）。
    pub fn delete_selected_objects(&mut self) -> usize {
        let targets: Vec<ObjectId> = if !self.selected_objects.is_empty() {
            self.selected_objects.iter().copied().collect()
        } else if let Some(id) = self.selected {
            vec![id]
        } else {
            Vec::new()
        };

        if targets.is_empty() {
            return 0;
        }

        self.push_undo(format!("{} 個のオブジェクトを削除", targets.len()));
        let mut count = 0;
        for id in &targets {
            if self.scene.remove_object(*id).is_some() {
                count += 1;
            }
        }
        self.clear_selection();
        self.dirty = true;
        self.status = format!("{} 個のオブジェクトを削除しました", count);
        count
    }

    /// 選択中のオブジェクト群をクリップボードにコピーします (Ctrl+C)。
    pub fn copy_selected(&mut self) -> usize {
        let target_ids: Vec<ObjectId> = if !self.selected_objects.is_empty() {
            self.selected_objects.iter().copied().collect()
        } else if let Some(id) = self.selected {
            vec![id]
        } else {
            Vec::new()
        };

        if target_ids.is_empty() {
            return 0;
        }

        let mut items = Vec::new();
        let mut min_layer = usize::MAX;
        let mut min_frame = Frame::MAX;

        for id in &target_ids {
            if let Some((layer_idx, obj)) = self.scene.find(*id) {
                if layer_idx < min_layer {
                    min_layer = layer_idx;
                }
                if obj.frame_start < min_frame {
                    min_frame = obj.frame_start;
                }
                items.push((layer_idx, obj.clone()));
            }
        }

        if items.is_empty() {
            return 0;
        }

        // 常にレイヤー昇順・開始フレーム昇順で決定論的に整列
        items.sort_by_key(|&(l, ref o)| (l, o.frame_start));

        self.clipboard = items
            .into_iter()
            .map(|(layer_idx, obj)| ClipboardItem {
                rel_layer: layer_idx.saturating_sub(min_layer),
                rel_frame_start: obj.frame_start.saturating_sub(min_frame),
                duration: obj.frame_end.saturating_sub(obj.frame_start),
                object: obj,
            })
            .collect();

        let count = self.clipboard.len();
        self.status = format!("{} 個のオブジェクトをクリップボードにコピーしました", count);
        count
    }

    /// クリップボード内のオブジェクト群をタイムラインへ貼り付けます (Ctrl+V)。
    /// 貼り付け先レイヤー・フレームが未指定の場合は、現在の選択レイヤー（または最上段）と再生ヘッド位置へ貼り付けます。
    pub fn paste_clipboard(&mut self, target_layer: Option<usize>, target_frame: Option<Frame>) -> Vec<ObjectId> {
        if self.clipboard.is_empty() {
            self.status = "クリップボードが空です".into();
            return Vec::new();
        }

        let base_layer = target_layer.unwrap_or_else(|| {
            self.selected
                .and_then(|id| self.scene.find(id).map(|(l, _)| l))
                .unwrap_or(0)
        });
        let base_frame = target_frame.unwrap_or(self.frame);

        self.push_undo(format!("{} 個のオブジェクトを貼り付け", self.clipboard.len()));

        let mut pasted_ids = Vec::new();
        for item in &self.clipboard {
            let mut new_obj = item.object.clone();
            new_obj.id = ObjectId::new();
            new_obj.frame_start = base_frame + item.rel_frame_start;
            new_obj.frame_end = new_obj.frame_start + item.duration;

            let layer_idx = (base_layer + item.rel_layer).min(au2_core::MAX_LAYERS - 1);
            pasted_ids.push(new_obj.id);
            self.scene.insert_object(layer_idx, new_obj);
        }

        self.selected_objects = pasted_ids.iter().copied().collect();
        self.selected = pasted_ids.first().copied();
        self.dirty = true;
        self.sync_sources_from_scene();
        self.sync_audio_playback();
        self.status = format!("{} 個のオブジェクトを貼り付けました", pasted_ids.len());
        pasted_ids
    }

    /// 選択中のオブジェクトを直後のフレームへ即座に複製します (Ctrl+D)。
    pub fn duplicate_selected(&mut self) -> usize {
        let count = self.copy_selected();
        if count == 0 {
            return 0;
        }

        let target_ids: Vec<ObjectId> = if !self.selected_objects.is_empty() {
            self.selected_objects.iter().copied().collect()
        } else if let Some(id) = self.selected {
            vec![id]
        } else {
            Vec::new()
        };

        let mut max_end = self.frame;
        let mut min_layer = 0;
        for id in &target_ids {
            if let Some((l, obj)) = self.scene.find(*id) {
                if obj.frame_end > max_end {
                    max_end = obj.frame_end;
                }
                min_layer = l;
            }
        }

        let pasted = self.paste_clipboard(Some(min_layer), Some(max_end + 1));
        let num = pasted.len();
        if num > 0 {
            self.status = format!("{} 個のオブジェクトを複製しました", num);
        }
        num
    }

    /// 再生ヘッド（現在のフレーム位置）にある選択オブジェクトを即座に分割します (Ctrl+K)。
    /// 何も選択されていない場合は、再生ヘッド位置を通過しているオブジェクトを分割します。
    pub fn split_at_playhead(&mut self) -> usize {
        let cur_frame = self.frame;

        let candidates: Vec<(usize, ObjectId)> = if !self.selected_objects.is_empty() {
            self.selected_objects
                .iter()
                .filter_map(|&id| {
                    let (layer_idx, obj) = self.scene.find(id)?;
                    if cur_frame > obj.frame_start && cur_frame <= obj.frame_end {
                        Some((layer_idx, id))
                    } else {
                        None
                    }
                })
                .collect()
        } else if let Some(id) = self.selected {
            if let Some((layer_idx, obj)) = self.scene.find(id) {
                if cur_frame > obj.frame_start && cur_frame <= obj.frame_end {
                    vec![(layer_idx, id)]
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            let mut list = Vec::new();
            for (li, layer) in self.scene.layers.iter().enumerate() {
                if !layer.visible {
                    continue;
                }
                for obj in &layer.objects {
                    if cur_frame > obj.frame_start && cur_frame <= obj.frame_end {
                        list.push((li, obj.id));
                    }
                }
            }
            list
        };

        if candidates.is_empty() {
            self.status = "再生ヘッド位置で分割できるオブジェクトがありません".into();
            return 0;
        }

        self.push_undo(format!("{} 個のオブジェクトを分割", candidates.len()));

        let mut right_ids = Vec::new();
        for (layer_idx, obj_id) in candidates {
            let split_result = self
                .scene
                .find_mut(obj_id)
                .and_then(|(_, obj)| obj.split_at(cur_frame));
            if let Some(right_obj) = split_result {
                let right_id = right_obj.id;
                right_ids.push(right_id);
                self.scene.insert_object(layer_idx, right_obj);
            }
        }

        let count = right_ids.len();
        if !right_ids.is_empty() {
            self.selected_objects = right_ids.iter().copied().collect();
            self.selected = right_ids.first().copied();
            self.dirty = true;
            self.status = format!("フレーム {} で {} 個のオブジェクトを分割しました", cur_frame + 1, count);
        }
        count
    }
}
