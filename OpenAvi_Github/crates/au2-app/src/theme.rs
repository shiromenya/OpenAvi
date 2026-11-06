//! アプリ全体のテーマ（ダーク / ライト）とデザイントークン。
//!
//! - [`apply`] で `egui` のスタイル（配色・余白・ボタン最小サイズ・角丸）を一括設定します。
//! - 各パネルは色をハードコードせず、このモジュールの関数（[`surface`] / [`border`] / [`accent_text`] など）
//!   または [`flip`] / [`fg`]（ダーク前提で書かれた色をライト用に変換）を経由して取得します。
//! - 選択したテーマは `<user_data_dir>/OpenAvi/theme.txt` に保存され、次回起動時に復元されます。

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, ecolor::Hsva, vec2, Color32, Rounding, Stroke};

static DARK: AtomicBool = AtomicBool::new(true);

/// テーマ種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

impl ThemeMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "🌙 ダークモード",
            Self::Light => "☀ ライトモード",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }
}

pub fn mode() -> ThemeMode {
    if DARK.load(Ordering::Relaxed) {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    }
}

pub fn is_dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

/// テーマを切り替えて egui のスタイルへ反映し、設定ファイルへ保存します。
pub fn set_mode(ctx: &egui::Context, m: ThemeMode) {
    set_mode_no_save(ctx, m);
    save(m);
}

/// 設定ファイルへ保存せずにテーマだけ切り替えます（テスト用）。
pub fn set_mode_no_save(ctx: &egui::Context, m: ThemeMode) {
    DARK.store(m == ThemeMode::Dark, Ordering::Relaxed);
    apply(ctx);
}

fn config_path() -> Option<std::path::PathBuf> {
    au2_platform::user_data_dir().map(|p| p.join("OpenAvi").join("theme.txt"))
}

/// 保存済みのテーマを読み込みます（未保存ならダーク）。
pub fn load_saved() -> ThemeMode {
    let Some(path) = config_path() else {
        return ThemeMode::Dark;
    };
    match std::fs::read_to_string(path) {
        Ok(s) if s.trim().eq_ignore_ascii_case("light") => ThemeMode::Light,
        _ => ThemeMode::Dark,
    }
}

fn save(m: ThemeMode) {
    if let Some(path) = config_path() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, if m == ThemeMode::Light { "light" } else { "dark" });
    }
}

/// 起動時に保存済みテーマを読み込んで適用します。
pub fn init(ctx: &egui::Context) {
    DARK.store(load_saved() == ThemeMode::Dark, Ordering::Relaxed);
    apply(ctx);
}

// ---------------------------------------------------------------------------
// パレット
// ---------------------------------------------------------------------------

#[inline]
fn pick(dark: Color32, light: Color32) -> Color32 {
    if is_dark() {
        dark
    } else {
        light
    }
}

/// カード / 入力欄などの面の色。
pub fn surface() -> Color32 {
    pick(Color32::from_rgb(20, 22, 27), Color32::from_rgb(255, 255, 255))
}
/// 少し沈んだ面（リスト行・サブカード）。
pub fn surface_alt() -> Color32 {
    pick(Color32::from_rgb(24, 26, 33), Color32::from_rgb(243, 245, 249))
}
/// 枠線。
pub fn border() -> Color32 {
    pick(Color32::from_gray(44), Color32::from_rgb(208, 213, 224))
}
/// 主要ボタン / 選択色。
pub fn accent() -> Color32 {
    Color32::from_rgb(37, 99, 235)
}
/// 強調テキスト（青系）。
pub fn accent_text() -> Color32 {
    pick(Color32::from_rgb(147, 197, 253), Color32::from_rgb(29, 78, 216))
}
/// 通常テキスト。
pub fn text() -> Color32 {
    pick(Color32::from_gray(225), Color32::from_rgb(10, 12, 16))
}
/// 補助テキスト。
pub fn text_muted() -> Color32 {
    pick(Color32::from_gray(150), Color32::from_gray(100))
}
pub fn danger_text() -> Color32 {
    pick(Color32::from_rgb(255, 120, 120), Color32::from_rgb(185, 28, 28))
}
pub fn ok_text() -> Color32 {
    pick(Color32::from_rgb(100, 220, 120), Color32::from_rgb(21, 128, 61))
}
pub fn warn_text() -> Color32 {
    pick(Color32::from_rgb(255, 210, 120), Color32::from_rgb(146, 64, 14))
}
/// 警告バナーの (背景, 枠線)。
pub fn warn_banner() -> (Color32, Color32) {
    pick2(
        (Color32::from_rgb(48, 38, 16), Color32::from_rgb(230, 170, 60)),
        (Color32::from_rgb(255, 247, 224), Color32::from_rgb(217, 160, 40)),
    )
}
/// エラーバナーの (背景, 枠線)。
pub fn error_banner() -> (Color32, Color32) {
    pick2(
        (Color32::from_rgb(50, 24, 24), Color32::from_rgb(220, 70, 70)),
        (Color32::from_rgb(254, 235, 235), Color32::from_rgb(220, 80, 80)),
    )
}
/// 成功バナーの (背景, 枠線)。
pub fn ok_banner() -> (Color32, Color32) {
    pick2(
        (Color32::from_rgb(18, 44, 28), Color32::from_rgb(60, 180, 100)),
        (Color32::from_rgb(230, 247, 236), Color32::from_rgb(60, 170, 100)),
    )
}
/// モーダル背後の暗幕。
pub fn backdrop() -> Color32 {
    Color32::from_black_alpha(if is_dark() { 160 } else { 110 })
}

fn pick2(dark: (Color32, Color32), light: (Color32, Color32)) -> (Color32, Color32) {
    if is_dark() {
        dark
    } else {
        light
    }
}

/// ダーク前提で書かれた「面・枠・灰色テキスト」の色を、ライトモードでは明度反転して返します。
/// （色相はほぼ保ち、彩度を落とす。ダークモードではそのまま返す）
pub fn flip(c: Color32) -> Color32 {
    if is_dark() {
        return c;
    }
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let mut h = Hsva::from_srgba_unmultiplied([r, g, b, a]);
    h.v = (1.0 - h.v * 0.97).clamp(0.0, 1.0);
    h.s *= 0.35;
    let [r, g, b, a] = h.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// ダーク前提の「明るい強調色テキスト」を、ライトモードでは読みやすいよう暗く鮮やかにして返します。
pub fn fg(c: Color32) -> Color32 {
    if is_dark() {
        return c;
    }
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let mut h = Hsva::from_srgba_unmultiplied([r, g, b, a]);
    h.v = (h.v * 0.55).clamp(0.0, 1.0);
    h.s = (h.s * 1.5).clamp(0.0, 1.0);
    let [r, g, b, a] = h.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

// ---------------------------------------------------------------------------
// egui スタイル
// ---------------------------------------------------------------------------

/// 現在のモードに合わせて egui の Visuals / Style を設定します。
pub fn apply(ctx: &egui::Context) {
    let dark = is_dark();
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };

    let rounding = Rounding::same(6.0);
    let accent = accent();

    if dark {
        v.panel_fill = Color32::from_rgb(20, 21, 25);
        v.window_fill = Color32::from_rgb(26, 28, 34);
        v.extreme_bg_color = Color32::from_rgb(13, 14, 17);
        v.faint_bg_color = Color32::from_rgb(28, 30, 36);
        v.window_stroke = Stroke::new(1.0_f32, Color32::from_gray(50));
        v.override_text_color = Some(Color32::from_gray(225));
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, Color32::from_gray(200));
        v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, Color32::from_gray(225));
        v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
        v.widgets.open.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
        v.widgets.noninteractive.bg_fill = Color32::from_rgb(24, 25, 30);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_gray(38));
        v.widgets.inactive.bg_fill = Color32::from_rgb(38, 42, 52);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(38, 42, 52);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_gray(58));
        v.widgets.hovered.bg_fill = Color32::from_rgb(50, 58, 74);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(50, 58, 74);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(96, 165, 250));
        v.widgets.open.bg_fill = Color32::from_rgb(38, 42, 52);
        v.widgets.open.weak_bg_fill = Color32::from_rgb(38, 42, 52);
        v.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(96, 165, 250));
    } else {
        v.panel_fill = Color32::from_rgb(241, 243, 247);
        v.window_fill = Color32::from_rgb(255, 255, 255);
        v.extreme_bg_color = Color32::from_rgb(255, 255, 255);
        v.faint_bg_color = Color32::from_rgb(235, 238, 244);
        v.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(200, 206, 218));
        v.override_text_color = Some(Color32::from_rgb(10, 12, 16));
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(20, 24, 33));
        v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(10, 12, 16));
        v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::BLACK);
        v.widgets.open.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(10, 12, 16));
        v.widgets.noninteractive.bg_fill = Color32::from_rgb(248, 249, 252);
        v.widgets.noninteractive.weak_bg_fill = Color32::from_rgb(120, 125, 135);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(214, 219, 229));
        v.widgets.inactive.bg_fill = Color32::from_rgb(230, 234, 242);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(230, 234, 242);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(196, 203, 217));
        v.widgets.hovered.bg_fill = Color32::from_rgb(214, 224, 244);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(214, 224, 244);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(59, 130, 246));
        v.widgets.open.bg_fill = Color32::from_rgb(220, 227, 242);
        v.widgets.open.weak_bg_fill = Color32::from_rgb(220, 227, 242);
        v.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(29, 78, 216));
    }
    v.selection.bg_fill = if dark { accent } else { Color32::from_rgb(191, 213, 252) };
    v.widgets.active.bg_fill = if dark { accent } else { Color32::from_rgb(190, 210, 245) };
    v.widgets.active.weak_bg_fill = if dark { accent } else { Color32::from_rgb(190, 210, 245) };
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, if dark { Color32::WHITE } else { Color32::from_rgb(29, 78, 216) });
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, if dark { Color32::WHITE } else { Color32::from_rgb(10, 12, 16) });
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = rounding;
    }
    v.window_rounding = Rounding::same(10.0);
    v.menu_rounding = Rounding::same(8.0);
    v.popup_shadow.color = Color32::from_black_alpha(if dark { 140 } else { 50 });
    v.window_shadow.color = Color32::from_black_alpha(if dark { 160 } else { 60 });
    ctx.set_visuals(v.clone());

    let mut style = (*ctx.style()).clone();
    style.visuals = v;
    style.spacing.item_spacing = vec2(8.0, 8.0);
    // ボタンは十分な高さ・左右余白を確保して、文字が詰まったり押しにくくならないようにする
    style.spacing.button_padding = vec2(12.0, 6.0);
    style.spacing.interact_size = vec2(44.0, 28.0);
    style.spacing.window_margin = egui::Margin::same(12.0);
    style.spacing.menu_margin = egui::Margin::symmetric(8.0, 6.0);
    style.spacing.combo_width = 120.0;
    style.spacing.icon_width = 18.0;
    style.spacing.icon_width_inner = 10.0;
    style.spacing.slider_width = 140.0;
    ctx.set_style(style);
}

/// 主要アクション用ボタン（青塗り・白文字）。
#[allow(dead_code)]
pub fn primary_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.into()).color(Color32::WHITE).strong())
        .fill(accent())
        .min_size(vec2(96.0, 30.0))
}

/// 破壊的アクション用ボタン（赤塗り・白文字）。
#[allow(dead_code)]
pub fn danger_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.into()).color(Color32::WHITE).strong())
        .fill(Color32::from_rgb(200, 50, 50))
        .min_size(vec2(96.0, 30.0))
}
