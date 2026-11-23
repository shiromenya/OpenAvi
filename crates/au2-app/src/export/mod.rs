//! 動画・音声の書き出し（エクスポート / レンダリング）エンジン。
//!
//! - プレビュー描画パイプライン ([`crate::panels::preview::render_scene_at`]) をそのまま利用し、
//!   `egui::Context` のテッセレータ (`tessellate`) が生成した三角形メッシュ・フォントアトラス・メディア画像を
//!   高精度ソフトウェアラスタライザで RGBA8 フレームバッファへ合成します。
//! - シーン内の全音声トラック（音量・左右パン・再生速度・再生位置・音声フェード対応）を 44.1kHz 16-bit ステレオ PCM にミックスします。
//! - FFmpeg パイプライン（MP4 / AVI / MOV / WebM / GIF）およびネイティブ WAV 出力に対応します。


mod audio_mix;
mod session;
mod software_renderer;

pub use audio_mix::mix_scene_audio_to_wav_bytes;
pub use session::ExportSession;
pub use software_renderer::SceneSoftwareRenderer;

use std::path::PathBuf;
use std::process::{Command, Stdio};

use au2_core::{Frame, Scene};

use crate::state::EditorState;

/// 動画・音声の書き出しフォーマット。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// MP4 (H.264 + AAC) — 最も互換性が高い標準動画形式
    Mp4,
    /// AVI (MPEG-4 + PCM) — AviUtl 互換の従来形式
    Avi,
    /// MOV (H.264 + AAC) — Apple / 編集ソフト向け形式
    Mov,
    /// WebM (VP9 + Opus) — Web 向け軽量動画形式
    Webm,
    /// アニメーション GIF
    Gif,
    /// 音声のみ (WAV 44.1kHz 16-bit Stereo PCM, FFmpeg 不要)
    Wav,
}

impl ExportFormat {
    pub const ALL: &'static [ExportFormat] = &[
        ExportFormat::Mp4,
        ExportFormat::Avi,
        ExportFormat::Mov,
        ExportFormat::Webm,
        ExportFormat::Gif,
        ExportFormat::Wav,
    ];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Avi => "avi",
            Self::Mov => "mov",
            Self::Webm => "webm",
            Self::Gif => "gif",
            Self::Wav => "wav",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4 動画 (*.mp4) [推奨・H.264 + AAC]",
            Self::Avi => "AVI 動画 (*.avi) [AviUtl クラシック互換]",
            Self::Mov => "MOV 動画 (*.mov) [QuickTime / H.264]",
            Self::Webm => "WebM 動画 (*.webm) [VP9 + Opus]",
            Self::Gif => "アニメーション GIF (*.gif)",
            Self::Wav => "音声のみ WAV (*.wav) [非圧縮 44.1kHz 16bit]",
        }
    }

    pub fn is_audio_only(self) -> bool {
        matches!(self, Self::Wav)
    }

    pub fn supports_audio(self) -> bool {
        matches!(self, Self::Mp4 | Self::Avi | Self::Mov | Self::Webm | Self::Wav)
    }
}

/// エンコード画質プリセット。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportQuality {
    /// 高画質 (CRF 18)
    High,
    /// 標準画質 (CRF 23)
    Standard,
    /// 軽量・高速 (CRF 28)
    Compact,
}

impl ExportQuality {
    pub fn label(self) -> &'static str {
        match self {
            Self::High => "高画質 (CRF 18 / 推奨)",
            Self::Standard => "標準 (CRF 23 / バランス)",
            Self::Compact => "軽量・高圧縮 (CRF 28)",
        }
    }

    pub fn crf_str(self) -> &'static str {
        match self {
            Self::High => "18",
            Self::Standard => "23",
            Self::Compact => "28",
        }
    }
}

/// 動画エンコード方式（ハードウェアアクセラレーション / CPU）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoEncoder {
    /// 自動判別（検出された最適なハードウェアエンコーダ、なければ CPU）
    #[default]
    Auto,
    /// CPU ソフトウェアエンコーダ (libx264 - 標準・最高画質/互換)
    Cpu,
    /// NVIDIA NVENC (h264_nvenc - GeForce / RTX 高速)
    Nvenc,
    /// Intel QSV (h264_qsv - Quick Sync Video 高速)
    Qsv,
    /// AMD AMF (h264_amf - Radeon 高速)
    Amf,
    /// Apple VideoToolbox (h264_videotoolbox - Mac 高速)
    VideoToolbox,
    /// Linux VA-API (h264_vaapi - Linux オープン規格 高速)
    Vaapi,
    /// Windows Media Foundation (h264_mf - OS標準 高速)
    MediaFoundation,
}

impl VideoEncoder {
    pub const ALL_HARDWARE: &'static [VideoEncoder] = &[
        VideoEncoder::Nvenc,
        VideoEncoder::Qsv,
        VideoEncoder::Amf,
        VideoEncoder::VideoToolbox,
        VideoEncoder::Vaapi,
        VideoEncoder::MediaFoundation,
    ];

    pub fn is_hardware_accelerated(self) -> bool {
        matches!(
            self,
            Self::Nvenc
                | Self::Qsv
                | Self::Amf
                | Self::VideoToolbox
                | Self::Vaapi
                | Self::MediaFoundation
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "自動判別 (最適なエンコーダを自動選択)",
            Self::Cpu => "CPU ソフトウェア (libx264 - 標準・最高互換)",
            Self::Nvenc => "NVIDIA NVENC (GeForce / RTX 高速)",
            Self::Qsv => "Intel QSV (Quick Sync Video 高速)",
            Self::Amf => "AMD AMF (Radeon 高速)",
            Self::VideoToolbox => "Apple VideoToolbox (Mac 高速)",
            Self::Vaapi => "Linux VA-API (オープン規格 高速)",
            Self::MediaFoundation => "Windows Media Foundation (OS標準 高速)",
        }
    }

    pub fn short_name(self) -> &'static str {
        match self {
            Self::Auto => "自動判別",
            Self::Cpu => "CPU (libx264)",
            Self::Nvenc => "NVIDIA NVENC",
            Self::Qsv => "Intel QSV",
            Self::Amf => "AMD AMF",
            Self::VideoToolbox => "Apple VideoToolbox",
            Self::Vaapi => "Linux VA-API",
            Self::MediaFoundation => "Windows Media Foundation",
        }
    }

    pub fn ffmpeg_codec_name(self) -> &'static str {
        match self {
            Self::Auto | Self::Cpu => "libx264",
            Self::Nvenc => "h264_nvenc",
            Self::Qsv => "h264_qsv",
            Self::Amf => "h264_amf",
            Self::VideoToolbox => "h264_videotoolbox",
            Self::Vaapi => "h264_vaapi",
            Self::MediaFoundation => "h264_mf",
        }
    }

    /// 指定された利用可能一覧から、実際に使用するエンコーダを安全に決定します。
    pub fn resolve(self, available: &[VideoEncoder]) -> VideoEncoder {
        match self {
            Self::Auto => available
                .iter()
                .copied()
                .find(|e| e.is_hardware_accelerated())
                .unwrap_or(VideoEncoder::Cpu),
            specific => {
                if specific == VideoEncoder::Cpu || available.contains(&specific) {
                    specific
                } else {
                    VideoEncoder::Cpu
                }
            }
        }
    }
}

/// システムで利用可能なビデオエンコーダ一覧を安全にプローブ・取得します。
pub fn available_video_encoders(ffmpeg_path: Option<&std::path::Path>) -> &'static [VideoEncoder] {
    static ENCODERS: std::sync::OnceLock<Vec<VideoEncoder>> = std::sync::OnceLock::new();
    ENCODERS.get_or_init(|| {
        let mut list = vec![VideoEncoder::Auto, VideoEncoder::Cpu];
        let Some(ffmpeg) = ffmpeg_path else {
            return list;
        };

        // プラットフォームに応じた候補リストを作成
        let mut candidates = Vec::new();
        #[cfg(target_os = "macos")]
        candidates.push(VideoEncoder::VideoToolbox);
        candidates.push(VideoEncoder::Nvenc);
        candidates.push(VideoEncoder::Qsv);
        candidates.push(VideoEncoder::Amf);
        #[cfg(target_os = "windows")]
        candidates.push(VideoEncoder::MediaFoundation);
        #[cfg(target_os = "linux")]
        candidates.push(VideoEncoder::Vaapi);

        for enc in candidates {
            if test_encoder_support(ffmpeg, enc.ffmpeg_codec_name()) {
                list.push(enc);
            }
        }
        list
    })
}

fn test_encoder_support(ffmpeg: &std::path::Path, codec_name: &str) -> bool {
    let mut cmd = Command::new(ffmpeg);
    au2_platform::hide_console_window(&mut cmd);
    cmd.args([
        "-y",
        "-f",
        "lavfi",
        "-i",
        "color=s=64x64:d=0.04",
        "-c:v",
        codec_name,
        "-f",
        "null",
        "-",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());

    match cmd.status() {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

pub fn apply_video_encoder_args(cmd: &mut Command, encoder: VideoEncoder, quality: ExportQuality) {
    match encoder {
        VideoEncoder::Nvenc => {
            let cq = match quality {
                ExportQuality::High => "19",
                ExportQuality::Standard => "23",
                ExportQuality::Compact => "28",
            };
            cmd.args([
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p4",
                "-cq",
                cq,
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
            ]);
        }
        VideoEncoder::Qsv => {
            let gq = match quality {
                ExportQuality::High => "20",
                ExportQuality::Standard => "24",
                ExportQuality::Compact => "29",
            };
            cmd.args([
                "-c:v",
                "h264_qsv",
                "-global_quality",
                gq,
                "-preset",
                "veryfast",
                "-pix_fmt",
                "nv12",
                "-movflags",
                "+faststart",
            ]);
        }
        VideoEncoder::Amf => {
            let qp = match quality {
                ExportQuality::High => "19",
                ExportQuality::Standard => "23",
                ExportQuality::Compact => "28",
            };
            cmd.args([
                "-c:v",
                "h264_amf",
                "-quality",
                "speed",
                "-qp_i",
                qp,
                "-qp_p",
                qp,
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
            ]);
        }
        VideoEncoder::VideoToolbox => {
            let q = match quality {
                ExportQuality::High => "65",
                ExportQuality::Standard => "50",
                ExportQuality::Compact => "35",
            };
            cmd.args([
                "-c:v",
                "h264_videotoolbox",
                "-q:v",
                q,
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
            ]);
        }
        VideoEncoder::Vaapi => {
            let q = match quality {
                ExportQuality::High => "20",
                ExportQuality::Standard => "24",
                ExportQuality::Compact => "29",
            };
            cmd.args([
                "-vaapi_device",
                "/dev/dri/renderD128",
                "-vf",
                "format=nv12,hwupload",
                "-c:v",
                "h264_vaapi",
                "-qp",
                q,
                "-movflags",
                "+faststart",
            ]);
        }
        VideoEncoder::MediaFoundation => {
            let b = match quality {
                ExportQuality::High => "12M",
                ExportQuality::Standard => "6M",
                ExportQuality::Compact => "3M",
            };
            cmd.args([
                "-c:v",
                "h264_mf",
                "-b:v",
                b,
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
            ]);
        }
        VideoEncoder::Cpu | VideoEncoder::Auto => {
            cmd.args([
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                quality.crf_str(),
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
            ]);
        }
    }
}

/// 動画書き出し設定。
#[derive(Debug, Clone)]
pub struct ExportConfig {
    pub output_path: PathBuf,
    pub format: ExportFormat,
    pub quality: ExportQuality,
    pub encoder: VideoEncoder,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub start_frame: Frame,
    pub end_frame: Frame,
    pub include_audio: bool,
}

impl ExportConfig {
    pub fn from_scene(scene: &Scene, default_path: PathBuf) -> Self {
        let content_last = scene.content_end().saturating_sub(1);
        let s = &scene.settings;
        Self {
            output_path: default_path,
            format: ExportFormat::Mp4,
            quality: ExportQuality::High,
            encoder: VideoEncoder::Auto,
            width: ((s.width.max(16) + 1) / 2) * 2,
            height: ((s.height.max(16) + 1) / 2) * 2,
            fps_num: s.frame_rate.num.max(1),
            fps_den: s.frame_rate.den.max(1),
            start_frame: 0,
            end_frame: content_last,
            include_audio: true,
        }
    }

    pub fn fps_f64(&self) -> f64 {
        self.fps_num.max(1) as f64 / self.fps_den.max(1) as f64
    }

    pub fn total_frames(&self) -> u32 {
        if self.end_frame >= self.start_frame {
            self.end_frame - self.start_frame + 1
        } else {
            1
        }
    }
}

/// シーン全体または指定範囲をブロッキングで一括書き出しします（単体テストや CLI バッチ処理用）。
#[allow(dead_code)]
pub fn export_scene_blocking(st: &mut EditorState, config: ExportConfig) -> Result<PathBuf, String> {
    let out_path = config.output_path.clone();
    let mut session = ExportSession::start(st, config)?;
    while !session.step_frames(st, 30)? {}
    Ok(out_path)
}