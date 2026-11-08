//! フィルタ効果・視覚パラメータ（ブラー、グロー、クリッピング、色調補正等）

use eframe::egui::{self, Color32};

#[derive(Default, Clone, Copy)]
pub(crate) struct FilterVisuals {
    pub(crate) glow_strength: f32,
    pub(crate) glow_radius: f32,
    pub(crate) glow_color: Option<Color32>,
    pub(crate) glow_only: bool,
    pub(crate) inner_hollow_ratio: f32,
    pub(crate) clip_insets: [f32; 4], // top, bottom, left, right
    pub(crate) bevel_width: f32,
    pub(crate) mirror_alpha: f32,
    pub(crate) hsv_shift: (f32, f32, f32), // brightness_delta, hue_deg, sat_mult
    pub(crate) grad_color: Option<(Color32, f32)>,
    pub(crate) chroma_shift: f32,
    pub(crate) chroma_angle: f32,
    pub(crate) border_width: f32,
    pub(crate) border_color: Option<Color32>,
    pub(crate) shadow_offset: Option<egui::Vec2>,
    pub(crate) shadow_alpha: f32,
    pub(crate) tint_color: Option<(Color32, f32)>,
    pub(crate) blend_mode: i32,
    // 極座標変換: Some((center_width, zoom_ratio, rot_deg, swirl))
    pub(crate) polar_transform: Option<(f32, f32, f32, f32)>,
    // 画像ループ: (nx, ny, speed_x, speed_y)
    pub(crate) image_loop: Option<(u32, u32, f32, f32)>,
    // オブジェクト分割: (nx, ny)
    pub(crate) object_split: Option<(u32, u32)>,
    // ラスター波形歪み: Some((width, height, period))
    pub(crate) raster: Option<(f32, f32, f32)>,
    // ノイズ強度: 0.0..=1.0
    pub(crate) noise_strength: f32,
    // 反転フラグ: (flip_v, flip_h, invert_luma, invert_hue, invert_alpha)
    pub(crate) inversions: (bool, bool, bool, bool, bool),
    // 扇クリッピングR: Some((start_deg, end_deg))
    pub(crate) sector_clip: Option<(f32, f32)>,
    // 縦横比オーバーライド (-1.0..=1.0)
    pub(crate) aspect_override: f32,
}
