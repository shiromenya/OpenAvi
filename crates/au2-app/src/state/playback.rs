//! 再生ヘッド・タイムライン進行・タイマ・自動保存・FFmpeg管理処理。

use std::time::Instant;

use au2_core::{EffectKind, Frame};
use au2_media::MediaKind;

use super::{EditorState, FfmpegDownload};

impl EditorState {
    /// 動画・音声メディアを扱うのに FFmpeg が必要なのに見つかっていない場合 `true`。
    pub fn needs_ffmpeg(&self) -> bool {
        if self.media.has_ffmpeg() {
            return false;
        }
        let in_scene = self.scene.layers.iter().any(|l| {
            l.objects.iter().any(|o| {
                o.source()
                    .is_some_and(|s| matches!(s.kind, EffectKind::VideoFile | EffectKind::AudioFile))
            })
        });
        in_scene
            || self
                .sources
                .iter()
                .any(|s| matches!(s.kind, MediaKind::Video | MediaKind::Audio))
    }

    /// FFmpeg を別スレッドでダウンロードしてユーザーデータ領域（`OpenAvi/bin`）に導入します。
    pub fn start_ffmpeg_download(&mut self) {
        if matches!(self.ffmpeg_dl, FfmpegDownload::Running(_)) {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(au2_media::download_ffmpeg_blocking());
        });
        self.ffmpeg_dl = FfmpegDownload::Running(rx);
        self.status = "FFmpeg をダウンロードしています（約 80MB・数十秒〜数分かかります）...".into();
    }

    /// ダウンロード完了を確認し、成功時は FFmpeg を再読込して動画・音声を即座に使えるようにします。
    pub fn poll_ffmpeg_download(&mut self) {
        let result = match &self.ffmpeg_dl {
            FfmpegDownload::Running(rx) => match rx.try_recv() {
                Ok(r) => Some(r),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(Err("ダウンロード処理が予期せず終了しました".to_string()))
                }
            },
            _ => None,
        };
        match result {
            Some(Ok(dir)) => {
                self.ffmpeg_dl = FfmpegDownload::Idle;
                if self.media.reload_ffmpeg() {
                    self.static_textures.clear();
                    self.stream_textures.clear();
                    self.status = format!("FFmpeg を導入しました ({})。動画・音声が再生できます", dir.display());
                } else {
                    self.ffmpeg_dl = FfmpegDownload::Failed("導入後も FFmpeg を検出できませんでした".into());
                }
            }
            Some(Err(e)) => {
                self.status = e.clone();
                self.ffmpeg_dl = FfmpegDownload::Failed(e);
            }
            None => {}
        }
    }

    /// 編集可能な最終フレーム。
    pub fn last_frame(&self) -> Frame {
        self.scene.settings.length.max(self.scene.content_end()).max(1) - 1
    }

    pub fn set_frame(&mut self, frame: Frame) {
        if frame >= self.scene.settings.length {
            self.scene.settings.length = (frame + 300).max(self.scene.settings.length);
        }
        self.frame = frame;
        if self.playing {
            self.play_origin = Some((Instant::now(), self.frame));
        } else {
            self.audio.stop();
        }
    }

    pub fn step(&mut self, delta: i64) {
        let f = (self.frame as i64 + delta).max(0) as Frame;
        self.set_frame(f);
    }

    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.play_origin = self.playing.then(|| (Instant::now(), self.frame));
        if !self.playing {
            self.audio.stop();
        }
    }

    /// 再生中なら経過時間からフレームを進め、音声を完全同期再生します（末尾でループ）。
    /// また、設定に応じてプロジェクトの自動保存（オートセーブ）を定期実行します。
    pub fn tick(&mut self) {
        if self.playing {
            if let Some((origin, start)) = self.play_origin {
                let fps = self.scene.settings.frame_rate;
                let mut f = start + fps.seconds_to_frame(origin.elapsed().as_secs_f64());
                if f > self.last_frame() {
                    f = 0;
                    self.play_origin = Some((Instant::now(), 0));
                }
                self.frame = f;
            }
            self.sync_audio_playback();
        } else {
            self.audio.stop();
        }

        // 編集変更がある場合の自動保存チェック
        self.autosave_tick();
    }

    /// 編集変更が発生したことを記録し、自動保存のデバウンスタイマーを開始します。
    #[allow(dead_code)]
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        if self.autosave.dirty_since.is_none() {
            self.autosave.dirty_since = Some(Instant::now());
        }
    }

    /// 自動保存のチェック・実行。
    /// 有効時は、レイヤーやパラメータに変更が生じる（dirty == true）たびに
    /// 操作直後（0.3秒後）に自動でプロジェクトファイルへ即座に上書き保存します。
    pub fn autosave_tick(&mut self) {
        if !self.autosave.enabled || !self.dirty {
            self.autosave.dirty_since = None;
            return;
        }
        let Some(path) = self.file_path.clone() else {
            return;
        };

        // 変更検知の初回時刻を記録
        let dirty_at = *self.autosave.dirty_since.get_or_insert_with(Instant::now);

        // 変更直後 0.3 秒静止（ドラッグや入力完了直後）したら即座に上書き保存
        if dirty_at.elapsed() >= std::time::Duration::from_millis(300) {
            match self.write_to_path(&path) {
                Ok(()) => {
                    self.autosave.auto_save_count += 1;
                    self.autosave.dirty_since = None;
                    let time_str = self.autosave.last_saved_time_str.as_deref().unwrap_or("");
                    let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    self.status = format!("変更を自動保存しました ({fname}) [{time_str}]");
                    log::info!("[自動保存] {time_str}: {}", path.display());
                }
                Err(e) => {
                    self.status = format!("[自動保存失敗] {e}");
                    log::warn!("[自動保存失敗] {e}");
                }
            }
        }
    }
}
