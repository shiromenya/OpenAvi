//! 動画・音声の書き出し（エクスポート / レンダリング）エンジン。
//!
//! - プレビュー描画パイプライン ([`crate::panels::preview::render_scene_at`]) をそのまま利用し、
//!   `egui::Context` のテッセレータ (`tessellate`) が生成した三角形メッシュ・フォントアトラス・メディア画像を
//!   高精度ソフトウェアラスタライザで RGBA8 フレームバッファへ合成します。
//! - シーン内の全音声トラック（音量・左右パン・再生速度・再生位置・音声フェード対応）を 44.1kHz 16-bit ステレオ PCM にミックスします。
//! - FFmpeg パイプライン（MP4 / AVI / MOV / WebM / GIF）およびネイティブ WAV 出力に対応します。

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::thread::JoinHandle;
use std::sync::Arc;
use std::time::Instant;

use au2_core::{EffectKind, Frame, ObjectId, Scene};
use au2_media::audio::{CHANNELS, SAMPLE_RATE};
use au2_media::RgbaFrame;
use eframe::egui::{self, epaint::ImageDelta, pos2, vec2, Color32, Pos2, Rect, TextureId};

use crate::panels::preview::{render_scene_at, DrawCtx};
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

/// 動画書き出し設定。
#[derive(Debug, Clone)]
pub struct ExportConfig {
    pub output_path: PathBuf,
    pub format: ExportFormat,
    pub quality: ExportQuality,
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

/// CPU ラスタライザ用の RGBA テクスチャ。
#[derive(Clone)]
struct SoftwareTexture {
    width: usize,
    height: usize,
    /// RGBA8 unmultiplied (width * height * 4)
    pixels: Vec<u8>,
}

impl SoftwareTexture {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![0u8; width * height * 4],
        }
    }

    fn from_image_data(data: &egui::ImageData) -> Self {
        match data {
            egui::ImageData::Color(c) => {
                let mut pixels = Vec::with_capacity(c.pixels.len() * 4);
                for p in &c.pixels {
                    let [r, g, b, a] = p.to_srgba_unmultiplied();
                    pixels.extend_from_slice(&[r, g, b, a]);
                }
                Self {
                    width: c.width(),
                    height: c.height(),
                    pixels,
                }
            }
            egui::ImageData::Font(f) => {
                let mut pixels = Vec::with_capacity(f.pixels.len() * 4);
                for c in f.srgba_pixels(None) {
                    let [r, g, b, a] = c.to_srgba_unmultiplied();
                    pixels.extend_from_slice(&[r, g, b, a]);
                }
                Self {
                    width: f.width(),
                    height: f.height(),
                    pixels,
                }
            }
        }
    }

    fn apply_delta(&mut self, delta: &ImageDelta) {
        let patch = Self::from_image_data(&delta.image);
        if let Some([ox, oy]) = delta.pos {
            for y in 0..patch.height {
                let dy = oy + y;
                if dy >= self.height {
                    break;
                }
                for x in 0..patch.width {
                    let dx = ox + x;
                    if dx >= self.width {
                        break;
                    }
                    let src_idx = (y * patch.width + x) * 4;
                    let dst_idx = (dy * self.width + dx) * 4;
                    self.pixels[dst_idx..dst_idx + 4].copy_from_slice(&patch.pixels[src_idx..src_idx + 4]);
                }
            }
        } else {
            *self = patch;
        }
    }

    /// バイリニア補間付きで UV (0.0..=1.0) の RGBA (0.0..=255.0) をサンプリングします。
    #[inline]
    fn sample_bilinear(&self, u: f32, v: f32) -> [f32; 4] {
        if self.width == 0 || self.height == 0 {
            return [255.0, 255.0, 255.0, 255.0];
        }
        let fx = (u.clamp(0.0, 1.0) * self.width as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
        let fy = (v.clamp(0.0, 1.0) * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let x0 = fx.floor() as usize;
        let y0 = fy.floor() as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;

        let get = |x: usize, y: usize| -> [f32; 4] {
            let idx = (y * self.width + x) * 4;
            [
                self.pixels[idx] as f32,
                self.pixels[idx + 1] as f32,
                self.pixels[idx + 2] as f32,
                self.pixels[idx + 3] as f32,
            ]
        };

        let c00 = get(x0, y0);
        let c10 = get(x1, y0);
        let c01 = get(x0, y1);
        let c11 = get(x1, y1);

        let mut out = [0.0; 4];
        for i in 0..4 {
            let top = c00[i] + (c10[i] - c00[i]) * tx;
            let bot = c01[i] + (c11[i] - c01[i]) * tx;
            out[i] = top + (bot - top) * ty;
        }
        out
    }
}

/// シーン描画を RGBA8 バッファへラスタライズするレンダラ。
pub struct SceneSoftwareRenderer {
    ctx: egui::Context,
    textures: HashMap<TextureId, SoftwareTexture>,
}

impl Default for SceneSoftwareRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneSoftwareRenderer {
    pub fn new() -> Self {
        let ctx = egui::Context::default();
        crate::fonts::install_japanese_font(&ctx);
        // 初回フォントアトラス初期化のため空フレームを1回実行
        let mut renderer = Self {
            ctx,
            textures: HashMap::new(),
        };
        let out = renderer.ctx.run(egui::RawInput::default(), |_| {});
        renderer.sync_textures_delta(&out.textures_delta);
        renderer
    }

    fn sync_textures_delta(&mut self, delta: &egui::TexturesDelta) {
        for (id, img_delta) in &delta.set {
            self.textures
                .entry(*id)
                .or_insert_with(|| SoftwareTexture::new(1, 1))
                .apply_delta(img_delta);
        }
        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    /// 指定フレームを `(width x height)` の RGBA8 ピクセル配列へレンダリングします。
    pub fn render_frame(
        &mut self,
        st: &mut EditorState,
        frame: Frame,
        width: u32,
        height: u32,
        fps: f64,
    ) -> Vec<u8> {
        let w = width.max(2) as usize;
        let h = height.max(2) as usize;
        let sw = st.scene.settings.width.max(1) as f32;
        let sh = st.scene.settings.height.max(1) as f32;
        let scale = (w as f32 / sw).min(h as f32 / sh).max(0.0001);
        let canvas_w = (sw * scale).round();
        let canvas_h = (sh * scale).round();
        let canvas_left = ((w as f32 - canvas_w) * 0.5).floor();
        let canvas_top = ((h as f32 - canvas_h) * 0.5).floor();
        let canvas = Rect::from_min_size(pos2(canvas_left, canvas_top), vec2(canvas_w, canvas_h));

        // 1. 現在フレーム（およびサブシーン内）の画像・動画オブジェクトのピクセルを同期取得
        let mut media_requests: Vec<(ObjectId, String, u32, f64)> = Vec::new();
        for (li, obj) in st.scene.objects_at(frame) {
            if !st.scene.layers[li].visible {
                continue;
            }
            if let Some(src) = obj.source().filter(|s| s.enabled) {
                if matches!(src.kind, EffectKind::ImageFile | EffectKind::VideoFile) {
                    if let Some(file_str) = src.text("file").filter(|f| !f.trim().is_empty()) {
                        let rel = frame - obj.frame_start;
                        let start_pos = (src.eval("再生位置", rel, 1.0) - 1.0).max(0.0);
                        let speed = (src.eval("再生速度", rel, 100.0) / 100.0).max(0.0);
                        let media_frame = (start_pos + rel as f64 * speed).floor() as u32;
                        let media_sec = media_frame as f64 / fps.max(1.0);
                        media_requests.push((obj.id, file_str.to_string(), media_frame, media_sec));
                    }
                }
            }
        }
        // サブシーン内のメディアも収集
        for sub_scene in st.scenes.values() {
            for (li, obj) in sub_scene.objects_at(frame) {
                if !sub_scene.layers[li].visible {
                    continue;
                }
                if let Some(src) = obj.source().filter(|s| s.enabled) {
                    if matches!(src.kind, EffectKind::ImageFile | EffectKind::VideoFile) {
                        if let Some(file_str) = src.text("file").filter(|f| !f.trim().is_empty()) {
                            let rel = frame - obj.frame_start;
                            let start_pos = (src.eval("再生位置", rel, 1.0) - 1.0).max(0.0);
                            let speed = (src.eval("再生速度", rel, 100.0) / 100.0).max(0.0);
                            let media_frame = (start_pos + rel as f64 * speed).floor() as u32;
                            let media_sec = media_frame as f64 / fps.max(1.0);
                            media_requests.push((obj.id, file_str.to_string(), media_frame, media_sec));
                        }
                    }
                }
            }
        }

        let mut resolved_textures: HashMap<ObjectId, (TextureId, [usize; 2])> = HashMap::new();
        let mut media_frames: HashMap<TextureId, Arc<RgbaFrame>> = HashMap::new();

        for (idx, (obj_id, file_str, media_frame, media_sec)) in media_requests.into_iter().enumerate() {
            if let Some(path) = st.resolve_cached_path(&file_str) {
                if let Some(rgba) = st.media.get_frame_sync(&path, media_frame, media_sec) {
                    let tex_id = TextureId::User(0x10000 + idx as u64);
                    let display_size = [rgba.original_width as usize, rgba.original_height as usize];
                    resolved_textures.insert(obj_id, (tex_id, display_size));
                    media_frames.insert(tex_id, rgba);
                }
            }
        }

        let raw_input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32))),
            ..Default::default()
        };

        let has_ffmpeg = st.media.has_ffmpeg();
        let scene_ref = &st.scene;
        let plugins_ref = &st.plugins;
        let scenes_ref = &st.scenes;

        let full_output = self.ctx.run(raw_input, |ctx| {
            let layer_id = egui::LayerId::background();
            let painter = egui::Painter::new(ctx.clone(), layer_id, canvas);
            painter.rect_filled(canvas, 0.0, Color32::BLACK);
            let draw_ctx = DrawCtx {
                painter: &painter,
                canvas,
                scale,
                fps,
                screen_w: sw as f64,
                screen_h: sh as f64,
                textures: &resolved_textures,
                plugins: plugins_ref,
                scenes: scenes_ref,
                has_ffmpeg,
                camera: None,
                group_transform: None,
                scene_depth: 0,
            };
            render_scene_at(&draw_ctx, scene_ref, frame, None);
        });

        self.sync_textures_delta(&full_output.textures_delta);
        let primitives = self.ctx.tessellate(full_output.shapes, 1.0);
        self.rasterize_primitives(&primitives, &media_frames, w, h, [0, 0, 0, 255])
    }

    /// UI（メニュー・パネル等）をヘッドレスで描画して RGBA8 画像として返します（デザイン確認用スナップショット）。
    #[cfg(test)]
    pub fn paint_ui(&mut self, w: usize, h: usize, bg: [u8; 4], mut run: impl FnMut(&egui::Context)) -> Vec<u8> {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32))),
            ..Default::default()
        };
        // レイアウトを安定させるため 2 パス実行する
        let first = self.ctx.run(raw.clone(), |c| run(c));
        self.sync_textures_delta(&first.textures_delta);
        let out = self.ctx.run(raw, |c| run(c));
        self.sync_textures_delta(&out.textures_delta);
        let prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        self.rasterize_primitives(&prims, &HashMap::new(), w, h, bg)
    }

    #[cfg(test)]
    pub fn context(&self) -> &egui::Context {
        &self.ctx
    }

    /// 三角形メッシュ列を RGBA8 フレームバッファへラスタライズします。
    fn rasterize_primitives(
        &self,
        primitives: &[egui::ClippedPrimitive],
        media_frames: &HashMap<TextureId, Arc<RgbaFrame>>,
        w: usize,
        h: usize,
        bg: [u8; 4],
    ) -> Vec<u8> {
        // 2. RGBA8 フレームバッファの初期化
        let mut fb = vec![0u8; w * h * 4];
        for px in fb.chunks_exact_mut(4) {
            px.copy_from_slice(&bg);
        }

        // 3. 各 ClippedPrimitive (Mesh) をラスタライズ。
        //    フレームバッファを水平バンドに分割し、複数スレッドで並列に処理する
        //    （各バンドは全プリミティブを元の順序で描画するため、見た目は逐次処理と完全に同一）。
        let textures_ref = &self.textures;
        let raster_band = |band: &mut [u8], y0: i32, y1: i32| {
        for prim in primitives {
            let egui::epaint::Primitive::Mesh(mesh) = &prim.primitive else {
                continue;
            };
            let clip_min_x = (prim.clip_rect.min.x.floor() as i32).clamp(0, w as i32);
            let clip_min_y = (prim.clip_rect.min.y.floor() as i32).clamp(0, h as i32).max(y0);
            let clip_max_x = (prim.clip_rect.max.x.ceil() as i32).clamp(0, w as i32);
            let clip_max_y = (prim.clip_rect.max.y.ceil() as i32).clamp(0, h as i32).min(y1);
            if clip_min_x >= clip_max_x || clip_min_y >= clip_max_y {
                continue;
            }

            let media_tex = media_frames.get(&mesh.texture_id);
            let sw_tex = textures_ref.get(&mesh.texture_id);

            for tri in mesh.indices.chunks_exact(3) {
                let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
                if i0 >= mesh.vertices.len() || i1 >= mesh.vertices.len() || i2 >= mesh.vertices.len() {
                    continue;
                }
                let v0 = &mesh.vertices[i0];
                let v1 = &mesh.vertices[i1];
                let v2 = &mesh.vertices[i2];

                rasterize_triangle(
                    band,
                    w,
                    y0,
                    [clip_min_x, clip_min_y, clip_max_x, clip_max_y],
                    v0,
                    v1,
                    v2,
                    sw_tex,
                    media_tex.map(|a| a.as_ref()),
                );
            }
        }
        };

        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(1, 16);
        // 小さな画像ではスレッド起動コストが勝つため単一スレッドで処理
        if threads <= 1 || h < 64 {
            raster_band(&mut fb[..], 0, h as i32);
        } else {
            // 負荷分散のためスレッド数の 3 倍程度の細かいバンドに分割し、各スレッドが順に担当する
            let band_h = (h + threads * 3 - 1) / (threads * 3);
            let bands: Vec<(usize, &mut [u8])> = fb
                .chunks_mut(band_h * w * 4)
                .enumerate()
                .map(|(i, s)| (i * band_h, s))
                .collect();
            let queue = std::sync::Mutex::new(bands.into_iter());
            let raster_ref = &raster_band;
            std::thread::scope(|scope| {
                for _ in 0..threads {
                    scope.spawn(|| loop {
                        let next = queue.lock().ok().and_then(|mut q| q.next());
                        let Some((y0, slice)) = next else { break };
                        let rows = slice.len() / (w * 4);
                        raster_ref(slice, y0 as i32, (y0 + rows) as i32);
                    });
                }
            });
        }

        fb
    }
}

/// 1つの三角形を重心座標補間とアルファブレンディングでフレームバッファにラスタライズします。
#[allow(clippy::too_many_arguments)]
fn rasterize_triangle(
    fb: &mut [u8],
    fb_w: usize,
    band_y0: i32,
    clip: [i32; 4],
    v0: &egui::epaint::Vertex,
    v1: &egui::epaint::Vertex,
    v2: &egui::epaint::Vertex,
    sw_tex: Option<&SoftwareTexture>,
    media_tex: Option<&RgbaFrame>,
) {
    let p0 = v0.pos;
    let p1 = v1.pos;
    let p2 = v2.pos;

    let min_x = (p0.x.min(p1.x).min(p2.x).floor() as i32).max(clip[0]);
    let min_y = (p0.y.min(p1.y).min(p2.y).floor() as i32).max(clip[1]);
    let max_x = (p0.x.max(p1.x).max(p2.x).ceil() as i32).min(clip[2]);
    let max_y = (p0.y.max(p1.y).max(p2.y).ceil() as i32).min(clip[3]);

    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let area = (p1.x - p0.x) * (p2.y - p0.y) - (p1.y - p0.y) * (p2.x - p0.x);
    if area.abs() < 1e-5 {
        return;
    }
    let inv_area = 1.0 / area;

    let c0 = v0.color.to_srgba_unmultiplied();
    let c1 = v1.color.to_srgba_unmultiplied();
    let c2 = v2.color.to_srgba_unmultiplied();

    let c0_f = [c0[0] as f32, c0[1] as f32, c0[2] as f32, c0[3] as f32];
    let c1_f = [c1[0] as f32, c1[1] as f32, c1[2] as f32, c1[3] as f32];
    let c2_f = [c2[0] as f32, c2[1] as f32, c2[2] as f32, c2[3] as f32];

    // 単色ポリゴン（UV が white UV 付近で全頂点同一色・テクスチャがフォントアトラスの白ピクセル）の高速判定
    let is_solid_white_uv = media_tex.is_none()
        && v0.uv.x <= 0.002
        && v0.uv.y <= 0.002
        && v1.uv.x <= 0.002
        && v1.uv.y <= 0.002
        && v2.uv.x <= 0.002
        && v2.uv.y <= 0.002;

    for y in min_y..max_y {
        let py = y as f32 + 0.5;
        let row_offset = ((y - band_y0) as usize) * fb_w * 4;
        for x in min_x..max_x {
            let px = x as f32 + 0.5;

            let w0 = ((p1.x - px) * (p2.y - py) - (p1.y - py) * (p2.x - px)) * inv_area;
            let w1 = ((p2.x - px) * (p0.y - py) - (p2.y - py) * (p0.x - px)) * inv_area;
            let w2 = 1.0 - w0 - w1;

            if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                continue;
            }

            let vr = w0 * c0_f[0] + w1 * c1_f[0] + w2 * c2_f[0];
            let vg = w0 * c0_f[1] + w1 * c1_f[1] + w2 * c2_f[1];
            let vb = w0 * c0_f[2] + w1 * c1_f[2] + w2 * c2_f[2];
            let va = w0 * c0_f[3] + w1 * c1_f[3] + w2 * c2_f[3];

            let (sr, sg, sb, sa) = if is_solid_white_uv {
                (vr, vg, vb, va)
            } else {
                let u = w0 * v0.uv.x + w1 * v1.uv.x + w2 * v2.uv.x;
                let v = w0 * v0.uv.y + w1 * v1.uv.y + w2 * v2.uv.y;

                let tex_rgba = if let Some(m) = media_tex {
                    sample_rgba_frame(m, u, v)
                } else if let Some(t) = sw_tex {
                    t.sample_bilinear(u, v)
                } else {
                    [255.0, 255.0, 255.0, 255.0]
                };

                (
                    (vr * tex_rgba[0]) * (1.0 / 255.0),
                    (vg * tex_rgba[1]) * (1.0 / 255.0),
                    (vb * tex_rgba[2]) * (1.0 / 255.0),
                    (va * tex_rgba[3]) * (1.0 / 255.0),
                )
            };

            if sa <= 0.5 {
                continue;
            }

            let idx = row_offset + (x as usize) * 4;
            if sa >= 254.5 {
                fb[idx] = sr.clamp(0.0, 255.0) as u8;
                fb[idx + 1] = sg.clamp(0.0, 255.0) as u8;
                fb[idx + 2] = sb.clamp(0.0, 255.0) as u8;
                fb[idx + 3] = 255;
            } else {
                let alpha = (sa * (1.0 / 255.0)).clamp(0.0, 1.0);
                let inv_alpha = 1.0 - alpha;
                fb[idx] = (sr * alpha + fb[idx] as f32 * inv_alpha).clamp(0.0, 255.0) as u8;
                fb[idx + 1] = (sg * alpha + fb[idx + 1] as f32 * inv_alpha).clamp(0.0, 255.0) as u8;
                fb[idx + 2] = (sb * alpha + fb[idx + 2] as f32 * inv_alpha).clamp(0.0, 255.0) as u8;
                fb[idx + 3] = 255;
            }
        }
    }
}

#[inline]
fn sample_rgba_frame(frame: &RgbaFrame, u: f32, v: f32) -> [f32; 4] {
    let w = frame.width as usize;
    let h = frame.height as usize;
    if w == 0 || h == 0 || frame.pixels.len() < w * h * 4 {
        return [255.0, 255.0, 255.0, 255.0];
    }
    let fx = (u.clamp(0.0, 1.0) * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let fy = (v.clamp(0.0, 1.0) * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let x0 = fx.floor() as usize;
    let y0 = fy.floor() as usize;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;

    let get = |x: usize, y: usize| -> [f32; 4] {
        let idx = (y * w + x) * 4;
        [
            frame.pixels[idx] as f32,
            frame.pixels[idx + 1] as f32,
            frame.pixels[idx + 2] as f32,
            frame.pixels[idx + 3] as f32,
        ]
    };

    let c00 = get(x0, y0);
    let c10 = get(x1, y0);
    let c01 = get(x0, y1);
    let c11 = get(x1, y1);

    let mut out = [0.0; 4];
    for i in 0..4 {
        let top = c00[i] + (c10[i] - c00[i]) * tx;
        let bot = c01[i] + (c11[i] - c01[i]) * tx;
        out[i] = top + (bot - top) * ty;
    }
    out
}

/// シーンの指定フレーム範囲 `[start_frame..=end_frame]` の音声を 44.1kHz 16-bit ステレオ PCM にミックスし、
/// RIFF WAVE バイト列として返します。
/// （音量・左右パン・再生位置・再生速度・音声フェードに対応）
pub fn mix_scene_audio_to_wav_bytes(
    st: &mut EditorState,
    start_frame: Frame,
    end_frame: Frame,
    fps: f64,
) -> Vec<u8> {
    let fps = fps.max(1.0);
    let total_frames = end_frame.saturating_sub(start_frame) + 1;
    let duration_sec = total_frames as f64 / fps;
    let total_stereo_frames = ((duration_sec * SAMPLE_RATE as f64).round() as usize).max(1);
    let mut mix_buf = vec![0.0f32; total_stereo_frames * (CHANNELS as usize)];

    struct AudioClip {
        is_audio_obj: bool,
        file_str: String,
        frame_start: Frame,
        frame_end: Frame,
        obj_clone: au2_core::TimelineObject,
    }

    let mut candidates = Vec::new();
    for layer in &st.scene.layers {
        if !layer.visible {
            continue;
        }
        for obj in &layer.objects {
            if obj.frame_end < start_frame || obj.frame_start > end_frame {
                continue;
            }
            let Some(src) = obj.source().filter(|s| s.enabled) else {
                continue;
            };
            if !matches!(src.kind, EffectKind::AudioFile | EffectKind::VideoFile) {
                continue;
            }
            let Some(file_str) = src.text("file").filter(|s| !s.trim().is_empty()) else {
                continue;
            };
            candidates.push(AudioClip {
                is_audio_obj: src.kind == EffectKind::AudioFile,
                file_str: file_str.to_string(),
                frame_start: obj.frame_start,
                frame_end: obj.frame_end,
                obj_clone: obj.clone(),
            });
        }
    }

    // AudioFile オブジェクトを優先登録し、ペアの VideoFile との2重再生を防止
    candidates.sort_by_key(|c| !c.is_audio_obj);
    let mut registered_keys: std::collections::HashSet<(PathBuf, Frame)> = std::collections::HashSet::new();

    for clip in candidates {
        let Some(path) = st.resolve_cached_path(&clip.file_str) else {
            continue;
        };
        if !registered_keys.insert((path.clone(), clip.frame_start)) {
            continue;
        };
        let Some(pcm) = st.media.get_audio_pcm(&path) else {
            continue;
        };
        if pcm.is_empty() {
            continue;
        }

        let src_total_frames = pcm.len() / 2;
        let Some(src_eff) = clip.obj_clone.source() else {
            continue;
        };

        let clip_start_f = clip.frame_start.max(start_frame);
        let clip_end_f = clip.frame_end.min(end_frame);
        let out_sample_start = (((clip_start_f - start_frame) as f64 / fps) * SAMPLE_RATE as f64).round() as usize;
        let out_sample_end = (((clip_end_f + 1 - start_frame) as f64 / fps) * SAMPLE_RATE as f64)
            .round()
            .min(total_stereo_frames as f64) as usize;

        let base_start_sec = if clip.is_audio_obj {
            src_eff.eval("再生位置", 0, 0.0).max(0.0)
        } else {
            ((src_eff.eval("再生位置", 0, 1.0) - 1.0) / fps).max(0.0)
        };
        let speed = (src_eff.eval("再生速度", 0, 100.0) / 100.0).max(0.0);

        // 音声フェード
        let (fade_in_sec, fade_out_sec) = clip
            .obj_clone
            .effects
            .iter()
            .find(|e| e.enabled && e.name == "音声フェード")
            .map(|e| (e.eval("イン", 0, 0.0).max(0.0), e.eval("アウト", 0, 0.0).max(0.0)))
            .unwrap_or((0.0, 0.0));

        let obj_dur_sec = (clip.frame_end.saturating_sub(clip.frame_start) + 1) as f64 / fps;

        for out_idx in out_sample_start..out_sample_end {
            let global_sec = out_idx as f64 / SAMPLE_RATE as f64 + (start_frame as f64 / fps);
            let rel_sec = (global_sec - clip.frame_start as f64 / fps).max(0.0);
            let rel_frame = (rel_sec * fps).floor() as u32;

            let src_sec = base_start_sec + rel_sec * speed;
            let src_sample_idx = (src_sec * SAMPLE_RATE as f64).floor() as usize;
            if src_sample_idx >= src_total_frames {
                break;
            }

            let vol = (clip
                .obj_clone
                .effect(&EffectKind::StandardPlayback)
                .map(|e| e.eval("音量", rel_frame, 100.0))
                .unwrap_or(100.0)
                / 100.0)
                .clamp(0.0, 5.0) as f32;

            let pan = (clip
                .obj_clone
                .effect(&EffectKind::StandardPlayback)
                .map(|e| e.eval("左右", rel_frame, 0.0))
                .unwrap_or(0.0)
                / 100.0)
                .clamp(-1.0, 1.0) as f32;

            let mut fade_gain = 1.0f32;
            if fade_in_sec > 0.001 && rel_sec < fade_in_sec {
                fade_gain *= (rel_sec / fade_in_sec).clamp(0.0, 1.0) as f32;
            }
            let remain_sec = (obj_dur_sec - rel_sec).max(0.0);
            if fade_out_sec > 0.001 && remain_sec < fade_out_sec {
                fade_gain *= (remain_sec / fade_out_sec).clamp(0.0, 1.0) as f32;
            }

            let gain_l = vol * fade_gain * (1.0 - pan.max(0.0));
            let gain_r = vol * fade_gain * (1.0 + pan.min(0.0));

            let l = pcm[src_sample_idx * 2] as f32 / 32768.0;
            let r = pcm[src_sample_idx * 2 + 1] as f32 / 32768.0;

            mix_buf[out_idx * 2] += l * gain_l;
            mix_buf[out_idx * 2 + 1] += r * gain_r;
        }
    }

    // RIFF WAVE (44.1kHz 16-bit Stereo PCM) エンコード
    let data_bytes_len = (mix_buf.len() * 2) as u32;
    let riff_size = 36 + data_bytes_len;
    let byte_rate = SAMPLE_RATE * (CHANNELS as u32) * 2;
    let block_align = CHANNELS * 2;

    let mut wav = Vec::with_capacity(44 + data_bytes_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM = 1
    wav.extend_from_slice(&CHANNELS.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes()); // 16 bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes_len.to_le_bytes());

    for sample in mix_buf {
        let s = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        wav.extend_from_slice(&s.to_le_bytes());
    }

    wav
}

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

        match config.format {
            ExportFormat::Mp4 | ExportFormat::Mov => {
                cmd.args([
                    "-c:v",
                    "libx264",
                    "-preset",
                    "veryfast",
                    "-crf",
                    config.quality.crf_str(),
                    "-pix_fmt",
                    "yuv420p",
                    "-movflags",
                    "+faststart",
                ]);
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

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("FFmpeg プロセスの起動に失敗しました: {e}"))?;
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

/// シーン全体または指定範囲をブロッキングで一括書き出しします（単体テストや CLI バッチ処理用）。
#[allow(dead_code)]
pub fn export_scene_blocking(st: &mut EditorState, config: ExportConfig) -> Result<PathBuf, String> {
    let out_path = config.output_path.clone();
    let mut session = ExportSession::start(st, config)?;
    while !session.step_frames(st, 30)? {}
    Ok(out_path)
}
