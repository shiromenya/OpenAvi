//! プロジェクト・シーン・レイヤー。

use crate::id::ObjectId;
use crate::object::TimelineObject;
use crate::time::{Frame, Rational};

/// 拡張編集と同じく 1 シーンあたり最大 100 レイヤー。
pub const MAX_LAYERS: usize = 100;

/// プロジェクト（`.aup` に相当）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Project {
    pub name: String,
    /// `scenes[0]` が Root シーン。
    pub scenes: Vec<Scene>,
}

impl Project {
    pub fn new(name: impl Into<String>, root: Scene) -> Self {
        Self { name: name.into(), scenes: vec![root] }
    }
}

/// シーン設定（`.exo` の `[exedit]` セクションに対応）。
#[derive(Debug, Clone, PartialEq)]
pub struct SceneSettings {
    pub width: u32,
    pub height: u32,
    pub frame_rate: Rational,
    /// シーンの長さ（フレーム数）
    pub length: Frame,
    pub audio_rate: u32,
    pub audio_channels: u32,
}

impl Default for SceneSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            frame_rate: Rational::new(30, 1),
            length: 18000, // 30fps で 10 分（必要に応じて自動拡張）
            audio_rate: 48000,
            audio_channels: 2,
        }
    }
}

/// レイヤー。
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: Option<String>,
    pub visible: bool,
    pub locked: bool,
    /// 座標のリンク
    pub coordinate_link: bool,
    /// 上のオブジェクトでクリッピング
    pub clip_above: bool,
    /// 開始フレーム昇順・重なり無しで保持。
    pub objects: Vec<TimelineObject>,
}

impl Default for Layer {
    fn default() -> Self {
        Self { name: None, visible: true, locked: false, coordinate_link: false, clip_above: false, objects: Vec::new() }
    }
}

impl Layer {
    /// `[start, end]` が既存オブジェクト（`ignore` を除く）と重なるか。
    pub fn is_free(&self, start: Frame, end: Frame, ignore: Option<ObjectId>) -> bool {
        self.objects
            .iter()
            .filter(|o| Some(o.id) != ignore)
            .all(|o| end < o.frame_start || start > o.frame_end)
    }

    fn sort(&mut self) {
        self.objects.sort_by_key(|o| o.frame_start);
    }
}

/// シーン（タイムライン 1 枚分）。
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub name: String,
    pub settings: SceneSettings,
    /// `layers[0]` がレイヤー1（最背面）。
    pub layers: Vec<Layer>,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new("Root", SceneSettings::default())
    }
}

impl Scene {
    pub fn new(name: impl Into<String>, settings: SceneSettings) -> Self {
        Self { name: name.into(), settings, layers: (0..MAX_LAYERS).map(|_| Layer::default()).collect() }
    }

    /// オブジェクトを指定レイヤーに追加します（0 始まり）。
    /// 重なりがある場合でも追加はしますが `false` を返します（`.exo` 読込時の寛容性のため）。
    pub fn insert_object(&mut self, layer: usize, obj: TimelineObject) -> bool {
        if layer >= self.layers.len() {
            self.layers.resize_with(layer + 1, Layer::default);
        }
        if obj.frame_end + 1 > self.settings.length {
            self.settings.length = obj.frame_end + 1;
        }
        let l = &mut self.layers[layer];
        let free = l.is_free(obj.frame_start, obj.frame_end, None);
        l.objects.push(obj);
        l.sort();
        free
    }

    /// ID からオブジェクトとそのレイヤー番号を取得します。
    pub fn find(&self, id: ObjectId) -> Option<(usize, &TimelineObject)> {
        self.layers
            .iter()
            .enumerate()
            .find_map(|(li, l)| l.objects.iter().find(|o| o.id == id).map(|o| (li, o)))
    }

    pub fn find_mut(&mut self, id: ObjectId) -> Option<(usize, &mut TimelineObject)> {
        self.layers
            .iter_mut()
            .enumerate()
            .find_map(|(li, l)| l.objects.iter_mut().find(|o| o.id == id).map(|o| (li, o)))
    }

    /// オブジェクトを移動します。移動先が埋まっている場合は何もせず `false`。
    pub fn move_object(&mut self, id: ObjectId, new_layer: usize, new_start: Frame) -> bool {
        let Some((old_layer, obj)) = self.find(id) else { return false };
        if new_layer >= self.layers.len() {
            return false;
        }
        let new_end = new_start + obj.length() - 1;
        if self.layers[new_layer].locked || self.layers[old_layer].locked {
            return false;
        }
        if !self.layers[new_layer].is_free(new_start, new_end, Some(id)) {
            return false;
        }
        let idx = self.layers[old_layer].objects.iter().position(|o| o.id == id).unwrap();
        let mut obj = self.layers[old_layer].objects.remove(idx);
        obj.frame_start = new_start;
        obj.frame_end = new_end;
        if new_end + 1 > self.settings.length {
            self.settings.length = new_end + 1;
        }
        self.layers[new_layer].objects.push(obj);
        self.layers[new_layer].sort();
        true
    }

    /// オブジェクトを削除します。
    pub fn remove_object(&mut self, id: ObjectId) -> Option<TimelineObject> {
        for layer in &mut self.layers {
            if let Some(pos) = layer.objects.iter().position(|o| o.id == id) {
                return Some(layer.objects.remove(pos));
            }
        }
        None
    }

    /// 指定レイヤー・指定開始フレームから配置可能な空きレイヤーを探し、オブジェクトを追加します。
    /// 追加されたオブジェクトの `(layer_index, ObjectId)` を返します。
    pub fn insert_object_at_free_layer(&mut self, preferred_layer: usize, mut obj: TimelineObject) -> (usize, ObjectId) {
        let start = obj.frame_start;
        let len = obj.length();
        if obj.frame_end + 1 > self.settings.length {
            self.settings.length = obj.frame_end + 1;
        }
        // まず希望レイヤーで、後続オブジェクトにぶつかる場合は長さを詰めるか空きレイヤーを探す
        for l_idx in preferred_layer..self.layers.len() {
            let layer = &self.layers[l_idx];
            if layer.locked {
                continue;
            }
            if layer.is_free(start, start + len - 1, None) {
                let id = obj.id;
                self.insert_object(l_idx, obj);
                return (l_idx, id);
            }
            // 開始位置自体は空いているが後ろのオブジェクトに当たる場合、長さを縮めて配置
            if layer.is_free(start, start, None) {
                if let Some(next_start) = layer.objects.iter().filter(|o| o.frame_start > start).map(|o| o.frame_start).min() {
                    obj.frame_end = (next_start - 1).max(start);
                    let id = obj.id;
                    self.insert_object(l_idx, obj);
                    return (l_idx, id);
                }
            }
        }
        let id = obj.id;
        let fallback = preferred_layer.min(self.layers.len().saturating_sub(1));
        self.insert_object(fallback, obj);
        (fallback, id)
    }

    /// 指定フレームに存在するオブジェクトを (レイヤー番号, オブジェクト) で列挙します。
    /// 描画順（レイヤー1 → 100）で返します。
    pub fn objects_at(&self, frame: Frame) -> impl Iterator<Item = (usize, &TimelineObject)> {
        self.layers.iter().enumerate().flat_map(move |(li, l)| {
            l.objects.iter().filter(move |o| o.contains(frame)).map(move |o| (li, o))
        })
    }

    pub fn object_count(&self) -> usize {
        self.layers.iter().map(|l| l.objects.len()).sum()
    }

    /// 最終オブジェクトの終端（オブジェクトが無ければ 0）。
    pub fn content_end(&self) -> Frame {
        self.layers.iter().flat_map(|l| l.objects.iter()).map(|o| o.frame_end + 1).max().unwrap_or(0)
    }

    /// 使用中の最大レイヤー番号 + 1。
    pub fn used_layer_count(&self) -> usize {
        self.layers.iter().rposition(|l| !l.objects.is_empty()).map(|i| i + 1).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_with_collision() {
        let mut s = Scene::default();
        let a = TimelineObject::new(0, 9);
        let b = TimelineObject::new(20, 29);
        let (ia, ib) = (a.id, b.id);
        s.insert_object(0, a);
        s.insert_object(0, b);
        assert!(!s.move_object(ia, 0, 15)); // b と重なる
        assert!(s.move_object(ia, 0, 10));
        assert!(s.move_object(ib, 3, 0));
        assert_eq!(s.find(ib).unwrap().0, 3);
        assert_eq!(s.used_layer_count(), 4);
        assert_eq!(s.objects_at(12).count(), 1);
    }
}
