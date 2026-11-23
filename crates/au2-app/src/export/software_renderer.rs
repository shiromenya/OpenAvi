//! プレビュー描画パイプラインをそのまま利用する CPU ソフトウェアラスタライザ。

use std::collections::HashMap;
use std::sync::Arc;

use au2_core::{EffectKind, Frame, ObjectId, Scene};
use au2_media::RgbaFrame;
use eframe::egui::{self, epaint::ImageDelta, pos2, vec2, Color32, Pos2, Rect, TextureId};

use crate::panels::preview::{render_scene_at, DrawCtx};
use crate::state::EditorState;

struct ExportMediaReq {
    obj_id: ObjectId,
    file_str: String,
    media_frame: u32,
    media_sec: f64,
    rel: u32,
    psd_layers: String,
    psd_blink: String,
    psd_lipsync: String,
}

fn collect_scene_media_requests(
    scene: &Scene,
    frame: Frame,
    fps: f64,
    out: &mut Vec<ExportMediaReq>,
) {
    for (li, obj) in scene.objects_at(frame) {
        if !scene.layers[li].visible {
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
                    out.push(ExportMediaReq {
                        obj_id: obj.id,
                        file_str: file_str.to_string(),
                        media_frame,
                        media_sec,
                        rel,
                        psd_layers: src.text("psd_layers").unwrap_or_default().to_string(),
                        psd_blink: src.text("psd_blink").unwrap_or_default().to_string(),
                        psd_lipsync: src.text("psd_lipsync").unwrap_or_default().to_string(),
                    });
                }
            }
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

        // 1. 現在フレーム（およびサブシーン内）の画像・PSD・動画オブジェクトのピクセルを同期取得
        let mut media_requests: Vec<ExportMediaReq> = Vec::new();
        collect_scene_media_requests(&st.scene, frame, fps, &mut media_requests);
        for sub_scene in st.scenes.values() {
            collect_scene_media_requests(sub_scene, frame, fps, &mut media_requests);
        }

        let mut resolved_textures: HashMap<ObjectId, (TextureId, [usize; 2])> = HashMap::new();
        let mut media_frames: HashMap<TextureId, Arc<RgbaFrame>> = HashMap::new();

        for (idx, req) in media_requests.into_iter().enumerate() {
            if let Some(path) = st.resolve_cached_path(&req.file_str) {
                let rgba_opt = if au2_media::is_psd_path(&path) {
                    let blink = au2_media::PsdBlinkConfig::parse(&req.psd_blink);
                    let lipsync = au2_media::PsdLipSyncConfig::parse(&req.psd_lipsync);
                    let audio_amp = if lipsync.mode == au2_media::PsdLipSyncMode::AudioSync {
                        st.sample_audio_amplitude_at(frame)
                    } else {
                        0.0
                    };
                    st.media
                        .get_psd_composited_frame(&path, &req.psd_layers, &blink, &lipsync, req.rel, fps, audio_amp)
                        .map(|(_, f)| f)
                } else {
                    st.media.get_frame_sync(&path, req.media_frame, req.media_sec)
                };
                if let Some(rgba) = rgba_opt {
                    let tex_id = TextureId::User(0x10000 + idx as u64);
                    let display_size = [rgba.original_width as usize, rgba.original_height as usize];
                    resolved_textures.insert(req.obj_id, (tex_id, display_size));
                    media_frames.insert(tex_id, rgba);
                }
            }
        }

        // シーン内で使われるカスタムフォントを事前登録し、新規登録時は空パスで定義を即時反映させる
        if crate::fonts::prepare_fonts_for_scenes(&self.ctx, &st.scene, &st.scenes) {
            let pre = self.ctx.run(egui::RawInput::default(), |_| {});
            self.sync_textures_delta(&pre.textures_delta);
        }

        let raw_input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32))),
            ..Default::default()
        };

        let has_ffmpeg = st.media.has_ffmpeg();
        let needs_waveform = st.scene.layers.iter().any(|l| {
            l.visible
                && l.objects.iter().any(|o| {
                    o.contains(frame)
                        && o.source()
                            .map(|s| matches!(s.kind, EffectKind::AudioWaveform | EffectKind::Scene))
                            .unwrap_or(false)
                })
        });
        let waveform_bins = if needs_waveform {
            st.sample_audio_waveform_at(frame, 64)
        } else {
            Vec::new()
        };
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
                audio_waveform: &waveform_bins,
                current_scene: Some((scene_ref, frame)),
                layer_limit: None,
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