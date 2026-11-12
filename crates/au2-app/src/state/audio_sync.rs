//! 再生中の音声トラック収集とオーディオエンジンへの同期供給。
//!
//! 毎フレーム全レイヤー・全オブジェクトを走査して「パス解決 → PCM 取得 → トラック一覧再構築 →
//! エンジンのロック差し替え」を行うと、数千オブジェクト級のプロジェクトで再生負荷になります。
//! そこで以下のように責務を分けています。
//!
//! - **構成（どのファイルをどの区間で鳴らすか）**: シーンの構成シグネチャが変わったときだけ再構築します。
//! - **値（再生位置・音量キーフレームの現在値）**: 毎フレーム評価し、変化があったときだけエンジンへ反映します。
//!
//! 再生結果（鳴る音・音量フェードへの追従）は従来と同一です。

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

use au2_core::{EffectKind, Frame, TimelineObject};
use au2_media::AudioTrackItem;

use super::EditorState;

/// キャッシュされた 1 本分の再生対象トラック（構成情報＋直近にエンジンへ渡した値）。
pub(crate) struct AudioSyncEntry {
    layer_idx: usize,
    obj_idx: usize,
    frame_start: Frame,
    frame_end: Frame,
    audio: Arc<Vec<i16>>,
    last_offset_sec: f64,
    last_volume: f32,
}

/// 音声トラック構成のキャッシュ。`signature` が一致する間は再構築しません。
pub(crate) struct AudioSyncCache {
    signature: u64,
    entries: Vec<AudioSyncEntry>,
}

impl AudioSyncCache {
    /// キャッシュされている再生対象トラック数。
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

fn is_playable_kind(kind: &EffectKind) -> bool {
    matches!(kind, EffectKind::AudioFile | EffectKind::VideoFile)
}

/// オブジェクトの現在フレームにおける `(再生位置[秒], 音量[0.0..])` を評価します。
/// 再生対象でない場合は `None`。
fn playback_params(obj: &TimelineObject, fps: f64, cur_frame: Frame) -> Option<(f64, f32)> {
    let src = obj.source().filter(|s| s.enabled)?;
    if !is_playable_kind(&src.kind) {
        return None;
    }
    let rel = (cur_frame as i64 - obj.frame_start as i64).max(0) as u32;
    let start_pos = if src.kind == EffectKind::AudioFile {
        src.eval("再生位置", rel, 0.0).max(0.0)
    } else {
        ((src.eval("再生位置", rel, 1.0) - 1.0) / fps.max(1.0)).max(0.0)
    };
    let volume = (obj
        .effect(&EffectKind::StandardPlayback)
        .map(|e| e.eval("音量", rel, 100.0))
        .unwrap_or(100.0)
        / 100.0) as f32;
    Some((start_pos, volume))
}

impl EditorState {
    /// 音声トラック構成のキャッシュを破棄します（次回の同期で必ず再構築）。
    /// パス解決キャッシュの破棄や FFmpeg 再読込など、シーン外の要因で結果が変わる場合に呼びます。
    pub(crate) fn invalidate_audio_sync_cache(&mut self) {
        self.audio_sync_cache = None;
    }

    /// 再生対象オブジェクトの構成（位置・区間・ファイル名・有効/表示状態・FPS）から安価なシグネチャを作ります。
    fn audio_sync_signature(&self, fps: f64) -> u64 {
        let mut h = DefaultHasher::new();
        fps.to_bits().hash(&mut h);
        for (li, layer) in self.scene.layers.iter().enumerate() {
            if !layer.visible {
                continue;
            }
            for (oi, obj) in layer.objects.iter().enumerate() {
                let Some(src) = obj.source().filter(|s| s.enabled) else {
                    continue;
                };
                if !is_playable_kind(&src.kind) {
                    continue;
                }
                let Some(file_str) = src.text("file").filter(|s| !s.trim().is_empty()) else {
                    continue;
                };
                (
                    li,
                    oi,
                    obj.id,
                    obj.frame_start,
                    obj.frame_end,
                    src.kind == EffectKind::AudioFile,
                    file_str,
                )
                    .hash(&mut h);
            }
        }
        h.finish()
    }

    /// 現在のシーンから再生対象トラックの構成を構築します（パス解決・PCM 取得を含む）。
    fn build_audio_sync_entries(&mut self, fps: f64) -> Vec<AudioSyncEntry> {
        struct Candidate {
            is_audio_obj: bool,
            file_str: String,
            layer_idx: usize,
            obj_idx: usize,
            frame_start: Frame,
            frame_end: Frame,
        }
        let mut candidates = Vec::new();

        for (li, layer) in self.scene.layers.iter().enumerate() {
            if !layer.visible {
                continue;
            }
            for (oi, obj) in layer.objects.iter().enumerate() {
                let Some(src) = obj.source().filter(|s| s.enabled) else {
                    continue;
                };
                if !is_playable_kind(&src.kind) {
                    continue;
                }
                let Some(file_str) = src.text("file").filter(|s| !s.trim().is_empty()) else {
                    continue;
                };
                candidates.push(Candidate {
                    is_audio_obj: src.kind == EffectKind::AudioFile,
                    file_str: file_str.to_string(),
                    layer_idx: li,
                    obj_idx: oi,
                    frame_start: obj.frame_start,
                    frame_end: obj.frame_end,
                });
            }
        }

        // AudioFile オブジェクトを優先登録し、ペアの VideoFile との 2 重再生を防止
        candidates.sort_by_key(|c| !c.is_audio_obj);

        let cur_frame = self.frame;
        let mut registered_keys: HashSet<(PathBuf, Frame)> = HashSet::new();
        let mut entries = Vec::new();
        for c in candidates {
            let Some(path) = self.resolve_cached_path(&c.file_str) else {
                continue;
            };
            if !registered_keys.insert((path.clone(), c.frame_start)) {
                continue;
            }
            let Some(pcm) = self.media.get_audio_pcm(&path) else {
                continue;
            };
            let Some((offset, volume)) = self.scene.layers[c.layer_idx]
                .objects
                .get(c.obj_idx)
                .and_then(|o| playback_params(o, fps, cur_frame))
            else {
                continue;
            };
            entries.push(AudioSyncEntry {
                layer_idx: c.layer_idx,
                obj_idx: c.obj_idx,
                frame_start: c.frame_start,
                frame_end: c.frame_end,
                audio: pcm,
                last_offset_sec: offset,
                last_volume: volume,
            });
        }
        entries
    }

    /// 現在のタイムライン上の音声トラックをオーディオエンジンに同期供給します。
    ///
    /// トラック構成はキャッシュし、シーン構成が変わったときだけ再構築します。
    /// 再生位置・音量は毎フレーム評価し、変化があったときだけエンジンに反映します。
    pub fn sync_audio_playback(&mut self) {
        let fps = self.scene.settings.frame_rate.as_f64();
        let signature = self.audio_sync_signature(fps);

        let mut changed = false;
        if !matches!(&self.audio_sync_cache, Some(c) if c.signature == signature) {
            let entries = self.build_audio_sync_entries(fps);
            self.audio_sync_cache = Some(AudioSyncCache { signature, entries });
            changed = true;
        } else if let Some(cache) = self.audio_sync_cache.as_mut() {
            let cur_frame = self.frame;
            for e in cache.entries.iter_mut() {
                let Some((offset, volume)) = self
                    .scene
                    .layers
                    .get(e.layer_idx)
                    .and_then(|l| l.objects.get(e.obj_idx))
                    .and_then(|o| playback_params(o, fps, cur_frame))
                else {
                    continue;
                };
                if offset != e.last_offset_sec || volume != e.last_volume {
                    e.last_offset_sec = offset;
                    e.last_volume = volume;
                    changed = true;
                }
            }
        }

        if changed {
            if let Some(cache) = &self.audio_sync_cache {
                let tracks = cache
                    .entries
                    .iter()
                    .map(|e| AudioTrackItem {
                        audio: Arc::clone(&e.audio),
                        start_frame: e.frame_start,
                        end_frame: e.frame_end,
                        offset_sec: e.last_offset_sec,
                        volume: e.last_volume,
                    })
                    .collect();
                self.audio.set_active_tracks(tracks);
            }
        }
        self.audio.sync_playback(self.playing, self.frame, fps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_sync_cache_reused_and_invalidated() {
        let mut st = EditorState::default();
        st.sync_audio_playback();
        let sig1 = st.audio_sync_cache.as_ref().map(|c| c.signature);
        assert!(sig1.is_some(), "初回同期でキャッシュが作成されること");
        assert_eq!(st.audio_sync_cache.as_ref().map(|c| c.len()), Some(0));

        // 構成が変わらなければ同じシグネチャのまま再利用される
        st.frame = 10;
        st.sync_audio_playback();
        assert_eq!(st.audio_sync_cache.as_ref().map(|c| c.signature), sig1);

        // FPS が変わると構成シグネチャが変わり再構築される
        let before = st.audio_sync_signature(30.0);
        let after = st.audio_sync_signature(60.0);
        assert_ne!(before, after);

        st.invalidate_audio_sync_cache();
        assert!(st.audio_sync_cache.is_none());
    }
}
