#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! OpenAvi デスクトップアプリケーション。

mod app;
mod fonts;
mod panels;
mod state;

use std::sync::Arc;

use eframe::egui;

/// ウィンドウおよびタスクバー用のアプリアイコン (64x64 RGBA) を生成します。
fn build_app_icon() -> Arc<egui::IconData> {
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
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_video_longer_than_5s_and_audio_integration() {
        let mut st = state::EditorState::default();
        let video_path = Path::new("samples/sample_with_audio_10s.mp4");
        if !video_path.exists() {
            return;
        }

        // 1. 動画をレイヤー0 に配置
        st.add_media_file(video_path, 0, 0);

        // 動画の長さが 5秒 (150フレーム) ではなく、実際の 10秒 (300フレーム) になっているか検証
        let layer0 = &st.scene.layers[0];
        assert!(!layer0.objects.is_empty(), "レイヤー0に動画オブジェクトが追加されていること");
        let video_obj = &layer0.objects[0];
        let video_len = video_obj.length();
        assert!(video_len >= 290, "10秒動画が150フレーム(5秒)で切られず、本来の長さ(約300フレーム)になっていること");

        // 2. 音声オブジェクトがペアで追加されているか検証
        let layer1 = &st.scene.layers[1];
        assert!(!layer1.objects.is_empty(), "音声付き動画の場合、ペアの音声オブジェクトが自動配置されること");
        let audio_obj = &layer1.objects[0];
        assert_eq!(audio_obj.category(), au2_core::ObjectCategory::Audio);
        assert_eq!(audio_obj.length(), video_len, "音声オブジェクトの長さが動画オブジェクトと一致していること");

        // 3. 5秒以降 (例えば frame 210 = 7.0秒) のフレームが正常に取得できるか検証
        let ctx = eframe::egui::Context::default();
        let path_str = video_path.to_string_lossy().to_string();
        let tex_result = st.get_media_texture(&ctx, &path_str, 210, 7.0);
        assert!(tex_result.is_some(), "5秒以降(7.0秒)の動画フレームが正常にデコードされてテクスチャ取得できること");
        let (_, size) = tex_result.unwrap();
        assert_eq!(size, [320, 180]);

        // 4. 音声 PCM データが正常に抽出できるか検証
        let pcm = st.media.get_audio_pcm(video_path);
        assert!(pcm.is_some(), "動画から PCM 音声データが取得できること");
        let pcm_data = pcm.unwrap();
        assert!(pcm_data.len() >= 800000, "10秒分のステレオPCMサンプルが含まれていること");

        // 5. 再生パフォーマンス検証: 300フレーム連続で再生ループを回した際、メインスレッドが一切ブロックされないこと
        st.playing = true;
        let bench_start = std::time::Instant::now();
        for f in 0..300 {
            st.frame = f;
            st.sync_audio_playback();
            // プレビュー描画のテクスチャ取得
            let _ = st.get_media_texture(&ctx, &path_str, f, f as f64 / 30.0);
            // タイムラインのサムネイル取得 (frame 0)
            let _ = st.get_media_texture(&ctx, &path_str, 0, 0.0);
        }
        let elapsed_ms = bench_start.elapsed().as_secs_f64() * 1000.0;
        let avg_ms_per_frame = elapsed_ms / 300.0;
        println!("300フレーム再生ループ合計: {elapsed_ms:.2} ms (1フレーム平均: {avg_ms_per_frame:.3} ms)");
        assert!(
            avg_ms_per_frame < 2.0,
            "1フレームあたりのメインスレッド処理時間が 2.0ms 未満 (60fps=16.6ms に対して十分な余裕) であること: 実測 {avg_ms_per_frame:.3} ms"
        );
        st.playing = false;
        st.audio.stop();
    }
}
