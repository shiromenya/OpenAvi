//! UI をブロックせずに段階的に動画・音声を書き出すエクスポートセッション管理。

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::thread::JoinHandle;
use std::time::Instant;

use au2_core::Frame;

use crate::state::EditorState;

use super::{
    apply_video_encoder_args, available_video_encoders, mix_scene_audio_to_wav_bytes, ExportConfig,
    ExportFormat, ExportQuality, SceneSoftwareRenderer, VideoEncoder,
};

/// UI をブロックせずに毎ティック数フレームずつ書き出しを進める段階的エクスポートセッション。
pub struct ExportSession {
    pub config: ExportConfig,
    pub current_frame: Frame,
    pub started_at: Instant,
    renderer: SceneSoftwareRenderer,
    child: Option<Child>,
    tx: Option<SyncSender<Vec<u8>>>,
    writer: Option<JoinHandle<Result<(), String>>>,
    temp_audio_path: Option<PathBuf>,
}

impl ExportSession {
    /// 書き出しセッションを開始します。
    pub fn start(st: &mut EditorState, config: ExportConfig) -> Result<Self, String> {
        if let Some(parent) = config.output_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| format!("出力先フォルダを作成できません: {e}"))?;
            }
        }

        // 音声のみ (WAV) の場合は FFmpeg 不要で即座に準備完了
        if config.format.is_audio_only() {
            return Ok(Self {
                current_frame: config.start_frame,
                config,
                started_at: Instant::now(),
                renderer: SceneSoftwareRenderer::new(),
                child: None,
                tx: None,
                writer: None,
                temp_audio_path: None,
            });
        }

        let ffmpeg = st
            .media
            .ffmpeg_path()
            .ok_or_else(|| "FFmpeg が見つかりません。上部の黄色いバナーから FFmpeg を自動導入してください。".to_string())?
            .to_path_buf();

        // 偶数解像度に正規化 (yuv420p 要件)
        let w = ((config.width.max(16) + 1) / 2) * 2;
        let h = ((config.height.max(16) + 1) / 2) * 2;
        let fps = config.fps_f64();

        // 音声トラックをミックスして一時 WAV ファイルを作成
        let mut temp_audio_path = None;
        if config.include_audio && config.format.supports_audio() {
            let wav_bytes = mix_scene_audio_to_wav_bytes(st, config.start_frame, config.end_frame, fps);
            let tmp_path = std::env::temp_dir().join(format!("openavi_export_audio_{}.wav", std::process::id()));
            if std::fs::write(&tmp_path, wav_bytes).is_ok() {
                temp_audio_path = Some(tmp_path);
            }
        }

        let fps_arg = format!("{}/{}", config.fps_num.max(1), config.fps_den.max(1));
        let size_arg = format!("{w}x{h}");

        let mut cmd = Command::new(&ffmpeg);
        au2_platform::hide_console_window(&mut cmd);
        cmd.args([
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &size_arg,
            "-r",
            &fps_arg,
            "-i",
            "-",
        ]);

        if let Some(audio_path) = &temp_audio_path {
            cmd.arg("-i").arg(audio_path);
        }

        let available = available_video_encoders(Some(&ffmpeg));
        let resolved_encoder = config.encoder.resolve(available);

        match config.format {
            ExportFormat::Mp4 | ExportFormat::Mov => {
                apply_video_encoder_args(&mut cmd, resolved_encoder, config.quality);
                if temp_audio_path.is_some() {
                    cmd.args(["-c:a", "aac", "-b:a", "192k", "-shortest"]);
                }
            }
            ExportFormat::Avi => {
                let qscale = match config.quality {
                    ExportQuality::High => "2",
                    ExportQuality::Standard => "5",
                    ExportQuality::Compact => "9",
                };
                cmd.args(["-c:v", "mpeg4", "-q:v", qscale, "-pix_fmt", "yuv420p"]);
                if temp_audio_path.is_some() {
                    cmd.args(["-c:a", "pcm_s16le", "-shortest"]);
                }
            }
            ExportFormat::Webm => {
                cmd.args([
                    "-c:v",
                    "libvpx-vp9",
                    "-crf",
                    config.quality.crf_str(),
                    "-b:v",
                    "0",
                    "-deadline",
                    "realtime",
                    "-cpu-used",
                    "4",
                    "-pix_fmt",
                    "yuv420p",
                ]);
                if temp_audio_path.is_some() {
                    cmd.args(["-c:a", "libopus", "-b:a", "160k", "-shortest"]);
                }
            }
            ExportFormat::Gif => {
                // GIF は高発色ディザリング付き
                cmd.args(["-vf", "split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse"]);
            }
            ExportFormat::Wav => {}
        }

        cmd.args(["-loglevel", "error"])
            .arg(&config.output_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if resolved_encoder.is_hardware_accelerated() => {
                // GPU エンコード起動失敗時、安全に CPU エンコーダ (libx264) へフォールバックして再試行
                let mut fallback_cmd = Command::new(&ffmpeg);
                au2_platform::hide_console_window(&mut fallback_cmd);
                fallback_cmd.args([
                    "-y",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "rgba",
                    "-s",
                    &size_arg,
                    "-r",
                    &fps_arg,
                    "-i",
                    "-",
                ]);
                if let Some(audio_path) = &temp_audio_path {
                    fallback_cmd.arg("-i").arg(audio_path);
                }
                apply_video_encoder_args(&mut fallback_cmd, VideoEncoder::Cpu, config.quality);
                if temp_audio_path.is_some() {
                    fallback_cmd.args(["-c:a", "aac", "-b:a", "192k", "-shortest"]);
                }
                fallback_cmd
                    .args(["-loglevel", "error"])
                    .arg(&config.output_path)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped());
                fallback_cmd
                    .spawn()
                    .map_err(|fe| format!("FFmpeg プロセスの起動に失敗しました (GPU: {e}, CPU: {fe})"))?
            }
            Err(e) => return Err(format!("FFmpeg プロセスの起動に失敗しました: {e}")),
        };
        let stdin = child.stdin.take();
        let (tx, writer) = match stdin {
            Some(s) => {
                let (tx, h) = spawn_frame_writer(s);
                (Some(tx), Some(h))
            }
            None => (None, None),
        };

        let mut normalized_config = config;
        normalized_config.width = w;
        normalized_config.height = h;

        Ok(Self {
            current_frame: normalized_config.start_frame,
            config: normalized_config,
            started_at: Instant::now(),
            renderer: SceneSoftwareRenderer::new(),
            child: Some(child),
            tx,
            writer,
            temp_audio_path,
        })
    }

    /// 指定フレーム数だけ書き出しを進めます。
    /// 完了した場合は `Ok(true)`、継続中は `Ok(false)` を返します。
    pub fn step_frames(&mut self, st: &mut EditorState, max_frames: u32) -> Result<bool, String> {
        if self.config.format.is_audio_only() {
            let fps = self.config.fps_f64();
            let wav_bytes = mix_scene_audio_to_wav_bytes(st, self.config.start_frame, self.config.end_frame, fps);
            std::fs::write(&self.config.output_path, wav_bytes)
                .map_err(|e| format!("WAV ファイルの書き出しに失敗しました: {e}"))?;
            self.current_frame = self.config.end_frame + 1;
            return Ok(true);
        }

        let fps = self.config.fps_f64();
        let w = self.config.width;
        let h = self.config.height;

        let t0 = Instant::now();
        for _ in 0..max_frames.max(1) {
            if self.current_frame > self.config.end_frame {
                return self.finish_encoding();
            }

            let rgba = self.renderer.render_frame(st, self.current_frame, w, h, fps);
            if let Some(tx) = self.tx.as_ref() {
                if tx.send(rgba).is_err() {
                    // 書き込みスレッドが異常終了（FFmpeg が先に終了した等）
                    let stderr_msg = self.collect_stderr();
                    self.cleanup_temp();
                    return Err(format!("FFmpeg へのフレーム書き込みエラー: {stderr_msg}"));
                }
            }
            self.current_frame += 1;
            // UI の応答性を保つため、1 ティックあたり約 40ms で打ち切る
            if t0.elapsed().as_millis() >= 40 {
                break;
            }
        }

        if self.current_frame > self.config.end_frame {
            self.finish_encoding()
        } else {
            Ok(false)
        }
    }

    pub fn progress_ratio(&self) -> f32 {
        let total = self.config.total_frames().max(1) as f32;
        let done = self.current_frame.saturating_sub(self.config.start_frame) as f32;
        (done / total).clamp(0.0, 1.0)
    }

    pub fn cancel(&mut self) {
        self.tx.take();
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        self.cleanup_temp();
        let _ = std::fs::remove_file(&self.config.output_path);
    }

    /// 残り時間の推定（秒）。開始直後でフレーム数が少なく推定できない場合は `None`。
    pub fn eta_seconds(&self) -> Option<f32> {
        let total = self.config.total_frames();
        let done = self.current_frame.saturating_sub(self.config.start_frame);
        if done < 2 {
            return None;
        }
        let elapsed = self.started_at.elapsed().as_secs_f32();
        let per_frame = elapsed / done as f32;
        Some((per_frame * total.saturating_sub(done) as f32).max(0.0))
    }

    fn finish_encoding(&mut self) -> Result<bool, String> {
        // 送信側を閉じて書き込みスレッドを終了させ、FFmpeg に EOF を通知
        self.tx.take();
        let write_result = match self.writer.take() {
            Some(w) => w.join().unwrap_or_else(|_| Err("書き込みスレッドが異常終了しました".to_string())),
            None => Ok(()),
        };
        if let Some(child) = self.child.take() {
            let output = child
                .wait_with_output()
                .map_err(|e| format!("FFmpeg の終了待機に失敗しました: {e}"))?;
            self.cleanup_temp();
            if !output.status.success() {
                let err_str = String::from_utf8_lossy(&output.stderr);
                return Err(format!("FFmpeg エンコード失敗: {}", err_str.trim()));
            }
        } else {
            self.cleanup_temp();
        }
        write_result?;
        Ok(true)
    }

    fn collect_stderr(&mut self) -> String {
        self.tx.take();
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        if let Some(child) = self.child.take() {
            if let Ok(out) = child.wait_with_output() {
                return String::from_utf8_lossy(&out.stderr).trim().to_string();
            }
        }
        String::new()
    }

    fn cleanup_temp(&mut self) {
        if let Some(p) = self.temp_audio_path.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

impl Drop for ExportSession {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        self.cleanup_temp();
    }
}

/// FFmpeg の標準入力へフレームを書き込む専用スレッドを起動します。
/// 映像のレンダリングとエンコード（パイプ書き込み）を並行させてスループットを上げます。
fn spawn_frame_writer(mut stdin: ChildStdin) -> (SyncSender<Vec<u8>>, JoinHandle<Result<(), String>>) {
    let (tx, rx) = sync_channel::<Vec<u8>>(6);
    let handle = std::thread::Builder::new()
        .name("openavi-export-writer".into())
        .spawn(move || {
            while let Ok(buf) = rx.recv() {
                stdin
                    .write_all(&buf)
                    .map_err(|e| format!("FFmpeg へのフレーム書き込みエラー: {e}"))?;
            }
            Ok(())
        })
        .expect("spawn export writer thread");
    (tx, handle)
}