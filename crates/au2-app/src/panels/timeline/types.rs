//! タイムラインの定数・色・ズーム計算・アイコン定義。

use au2_core::{Effect, EffectKind, Frame, ObjectCategory, Rational, TimelineObject};
use eframe::egui::Color32;

use crate::state::EditorState;

pub(crate) const HEADER_W: f32 = 114.0;
pub(crate) const RULER_H: f32 = 22.0;
pub(crate) const H_SCROLLBAR_H: f32 = 18.0;
pub(crate) const ZOOM_BAR_W: f32 = 270.0;
pub(crate) const MIN_PPF: f32 = 0.05;
pub(crate) const MAX_PPF: f32 = 48.0;

/// Premiere Pro 風のインクリップ・サムネイル／波形／図形プレビュー表示時のレイヤー高さ
pub const LAYER_H_PREVIEW: f32 = 46.0;
/// プレビュー OFF 時（従来の AviUtl 風コンパクト表示）のレイヤー高さ
pub const LAYER_H_COMPACT: f32 = 24.0;
pub(crate) const CLIP_HEADER_H: f32 = 15.0;

/// タイムライン末尾の作業用余白フレーム数（60fps換算で約1分間分）
pub(crate) const TIMELINE_TAIL_MARGIN_FRAMES: f32 = 3600.0;
/// タイムライン表示幅に応じた最小作業余白フレーム数（約20秒分）
pub(crate) const TIMELINE_MIN_SCROLLABLE_FRAMES: f32 = 1200.0;
/// 水平スクロールバーのつまみ（サム）の最小表示幅 (px)
pub(crate) const SCROLLBAR_MIN_THUMB_W: f32 = 28.0;
/// ドラッグ移動時に自動スクロールが開始されるレーン端からのマージン (px)
pub(crate) const EDGE_AUTOSCROLL_MARGIN_PX: f32 = 24.0;
/// ドラッグ移動時の自動スクロール基準速度 (px/frame相当)
pub(crate) const EDGE_AUTOSCROLL_SPEED_PX: f32 = 16.0;
/// 再生位置がタイムライン表示枠の右端から外れたと判定するマージン (px)
pub(crate) const PLAYHEAD_FOLLOW_MARGIN_PX: f32 = 16.0;
/// 再生追従スクロール時にヘッドの手前に確保する余白 (px)
pub(crate) const PLAYHEAD_FOLLOW_LEAD_PX: f32 = 8.0;

pub(crate) fn ppf_to_slider_t(ppf: f32) -> f32 {
    let p = ppf.clamp(MIN_PPF, MAX_PPF);
    ((p / MIN_PPF).ln() / (MAX_PPF / MIN_PPF).ln()).clamp(0.0, 1.0)
}

pub(crate) fn slider_t_to_ppf(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    MIN_PPF * (MAX_PPF / MIN_PPF).powf(t)
}

pub(crate) fn zoom_at(v: &mut crate::state::TimelineView, lane_left: f32, center_x: f32, factor: f32) {
    let anchor = ((center_x - lane_left) / v.px_per_frame + v.scroll_x).max(0.0);
    v.px_per_frame = (v.px_per_frame * factor).clamp(MIN_PPF, MAX_PPF);
    v.scroll_x = (anchor - (center_x - lane_left) / v.px_per_frame).max(0.0);
}

pub(crate) fn fit_timeline_to_view(st: &mut EditorState, view_w: f32) {
    let end_f = (st.scene.content_end().max(180) + 30) as f32;
    let target_ppf = (view_w / end_f).clamp(MIN_PPF, MAX_PPF);
    st.timeline.px_per_frame = target_ppf;
    st.timeline.scroll_x = 0.0;
}

pub(crate) fn category_color(c: ObjectCategory) -> Color32 {
    match c {
        ObjectCategory::Media => Color32::from_rgb(38, 114, 120),
        ObjectCategory::Audio => Color32::from_rgb(148, 66, 76),
        ObjectCategory::Filter => Color32::from_rgb(56, 118, 82),
        ObjectCategory::Control => Color32::from_rgb(148, 110, 48),
    }
}

/// オブジェクト種別に応じた落ち着いたプロ編集ソフト風クリップ色を返す。
pub(crate) fn clip_color(obj: &TimelineObject) -> Color32 {
    match obj.source().map(|s| &s.kind) {
        Some(EffectKind::VideoFile) => Color32::from_rgb(30, 116, 122),
        Some(EffectKind::ImageFile) => Color32::from_rgb(52, 96, 134),
        Some(EffectKind::Text) => Color32::from_rgb(92, 76, 128),
        Some(EffectKind::Shape) => Color32::from_rgb(54, 90, 108),
        Some(EffectKind::AudioFile) => Color32::from_rgb(148, 66, 76),
        _ => category_color(obj.category()),
    }
}

pub(crate) fn fmt_time(frame: Frame, rate: Rational) -> String {
    let s = rate.frame_to_seconds(frame);
    format!("{:02}:{:05.2}", (s / 60.0).floor() as u64, s % 60.0)
}

/// エフェクトからファイルパス文字列 (`file` または `ファイル`) を取得する。
pub(crate) fn effect_file_path(eff: &Effect) -> Option<&str> {
    eff.text("file").or_else(|| eff.text("ファイル"))
}

/// 標準描画・拡張描画・標準再生など末尾の描画/再生プロパティかどうか。
pub(crate) fn is_render_or_playback(kind: &EffectKind) -> bool {
    matches!(
        kind,
        EffectKind::StandardDraw | EffectKind::ExtendedDraw | EffectKind::StandardPlayback
    )
}

/// クリップ種別に応じたアイコン文字を返す。
pub(crate) fn clip_kind_icon(obj: &TimelineObject) -> &'static str {
    if obj.camera_target {
        return "◎";
    }
    match obj.source().map(|s| &s.kind) {
        Some(EffectKind::VideoFile) => "▶",
        Some(EffectKind::ImageFile) => {
            if obj
                .source()
                .and_then(effect_file_path)
                .is_some_and(|p| p.to_ascii_lowercase().ends_with(".psd") || p.to_ascii_lowercase().ends_with(".psb"))
            {
                "◈"
            } else {
                "▣"
            }
        }
        Some(EffectKind::AudioFile) => "♪",
        Some(EffectKind::Text) => "T",
        Some(EffectKind::Shape) => "◆",
        Some(EffectKind::FrameBuffer) => "⧉",
        Some(EffectKind::GroupControl) => "▤",
        Some(EffectKind::Camera) => "◎",
        Some(EffectKind::TimeControl) => "◷",
        Some(EffectKind::AudioWaveform) => "∿",
        _ => "▫",
    }
}

pub(crate) fn parse_hex_rgb(hex: &str, alpha_mul: f32) -> Color32 {
    let clean = hex.trim().trim_start_matches('#');
    let u = u32::from_str_radix(clean, 16).unwrap_or(0xffffff);
    let r = ((u >> 16) & 0xff) as u8;
    let g = ((u >> 8) & 0xff) as u8;
    let b = (u & 0xff) as u8;
    let a = (255.0 * alpha_mul.clamp(0.0, 1.0)) as u8;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}
