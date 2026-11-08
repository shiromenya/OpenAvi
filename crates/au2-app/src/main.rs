#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! # OpenAvi デスクトップアプリケーション
//!
//! AviUtl / AviUtl2 互換の次世代オープンソース動画編集ソフトウェア。
//!
//! ## 主な特徴
//! - **完全互換性**: `.aup2` / `.aup` / `.exo` / `.exa` の完全な読み書きと相互移行
//! - **高速プレビュー**: wgpu / egui による 60fps ハードウェアアクセラレーション描画
//! - **快適な編集機能**: スナップ吸着タイムライン、直感的なD&D配置、PSD立ち絵（目パチ口パク）対応
//! - **クロスプラットフォーム**: Windows / macOS / Linux 対応のポータブル設計

mod app;
mod dialogs;
mod export;
mod fonts;
mod panels;
mod state;
mod theme;

use std::sync::Arc;

use eframe::egui;

/// ウィンドウおよびタスクバー用のアプリアイコンを生成します。
/// `assets/openavi_icon.png` のロゴ画像を読み込み、失敗時はフォールバックします。
fn build_app_icon() -> Arc<egui::IconData> {
    const ICON_PNG: &[u8] = include_bytes!("../../../assets/openavi_icon.png");
    if let Ok(img) = image::load_from_memory(ICON_PNG) {
        let rgba_img = img.to_rgba8();
        let (w, h) = rgba_img.dimensions();
        return Arc::new(egui::IconData {
            rgba: rgba_img.into_raw(),
            width: w,
            height: h,
        });
    }

    let w = 64u32;
    let h = 64u32;
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 - 31.5;
            let fy = y as f32 - 31.5;
            // 角丸矩形マスク
            let dx = (fx.abs() - 20.0).max(0.0);
            let dy = (fy.abs() - 20.0).max(0.0);
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= 10.0 {
                // 中央の再生トライアングル判定
                let tx = fx + 4.0;
                let ty = fy;
                let in_play = tx >= -8.0 && tx <= 14.0 && ty.abs() <= (14.0 - tx) * 0.62;
                let idx = ((y * w + x) * 4) as usize;
                if in_play {
                    rgba[idx] = 255;
                    rgba[idx + 1] = 255;
                    rgba[idx + 2] = 255;
                    rgba[idx + 3] = 255;
                } else {
                    let t = (x + y) as f32 / (w + h) as f32;
                    rgba[idx] = (26.0 + t * 207.0) as u8;
                    rgba[idx + 1] = (35.0 + t * 34.0) as u8;
                    rgba[idx + 2] = (75.0 + (1.0 - t) * 110.0) as u8;
                    rgba[idx + 3] = if dist > 9.0 {
                        ((10.0 - dist) * 255.0) as u8
                    } else {
                        255
                    };
                }
            }
        }
    }
    Arc::new(egui::IconData {
        rgba,
        width: w,
        height: h,
    })
}

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // 第 1 引数に .aup2 / .aup / .exo / メディアファイルを渡すと起動時に読み込む
    let initial = std::env::args_os().nth(1).map(std::path::PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OpenAvi")
            .with_icon(build_app_icon())
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([800.0, 500.0])
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "OpenAvi",
        options,
        Box::new(move |cc| Ok(Box::new(app::OpenAviApp::new(cc, initial)))),
    )
}

#[cfg(test)]
mod tests;
