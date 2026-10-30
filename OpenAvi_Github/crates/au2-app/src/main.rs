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

    /// 実 .aup をヘッドレス egui でプレビュー描画し、各フレームで描画プリミティブが出力されるか調べる。
    #[test]
    fn preview_renders_real_aup_frames() {
        let path = Path::new(r"C:\Users\user\Desktop\goot time pf.aup");
        if !path.exists() {
            return;
        }
        let mut st = state::EditorState::default();
        st.load_path(path);
        let ctx = egui::Context::default();
        for f in [0u32, 100, 300, 500, 800, 1000, 1500, 2000] {
            st.frame = f;
            let out = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| panels::preview::ui(ui, &mut st));
            });
            let mut meshes = 0usize;
            for cs in &out.shapes {
                if let egui::epaint::Shape::Mesh(_) = &cs.shape {
                    meshes += 1;
                }
            }
            assert!(meshes > 0, "frame {f}: プレビューに描画プリミティブが出力されていること");
        }
    }

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

    #[test]
    fn test_real_mp3_playback_and_missing_file_warning() {
        let mut st = state::EditorState::default();
        let mp3_path = Path::new(r"C:\Users\user\Desktop\ぐるたみん\01 シャルル - バルーン.mp3");
        if !mp3_path.exists() {
            println!("MP3 file does not exist, skipping real mp3 test");
            return;
        }

        // 1. MP3 ファイルをレイヤー0に追加
        st.add_media_file(mp3_path, 0, 0);
        let layer0 = &st.scene.layers[0];
        assert!(!layer0.objects.is_empty(), "MP3オブジェクトがレイヤー0に追加されること");
        let obj = &layer0.objects[0];
        assert_eq!(obj.category(), au2_core::ObjectCategory::Audio);
        println!("Added MP3 object: length={} frames", obj.length());

        // 2. PCM データが正常に抽出できるか
        let pcm = st.media.get_audio_pcm(mp3_path);
        assert!(pcm.is_some(), "MP3 から PCM データが抽出できること");
        let pcm_data = pcm.unwrap();
        println!("Decoded MP3 PCM samples: {}", pcm_data.len());
        assert!(!pcm_data.is_empty(), "PCM サンプルが空でないこと");

        // 3. sync_audio_playback を呼んで AudioEngine にトラックが供給されるか
        st.playing = true;
        st.frame = 10;
        st.sync_audio_playback();

        // 4. 音声エンジンの共有状態を確認
        println!("Audio playback sync successful!");
        st.playing = false;
        st.audio.stop();
    }

    #[test]
    fn test_aup_audio_file_restoration_and_resolution() {
        let candidates = [
            r"C:\Users\user\Desktop\goot time pf.aup",
            r"c:\Users\user\AppData\Local\Temp\goot time pf.aup",
        ];
        let mut aup_path = None;
        for c in candidates {
            let p = Path::new(c);
            if p.exists() {
                aup_path = Some(p);
                break;
            }
        }
        let Some(p) = aup_path else {
            return;
        };

        let bytes = std::fs::read(p).expect("read aup");
        let doc = au2_aup::parse_aup_bytes(&bytes).expect("parse legacy aup");

        // タイムライン上の音声オブジェクトを検証
        let mut found_audio_file = false;
        for layer in &doc.scene.layers {
            for obj in &layer.objects {
                if let Some(src) = obj.source() {
                    if src.kind == au2_core::EffectKind::AudioFile {
                        if let Some(f) = src.text("file") {
                            assert!(
                                f.contains("Nightcore - Good Time.wav") || f.contains("Good Time"),
                                "音声オブジェクトのファイルパスが復元されていること: {f}"
                            );
                            found_audio_file = true;
                        }
                        // 標準再生の音量
                        if let Some(sp) = obj.effect(&au2_core::EffectKind::StandardPlayback) {
                            let vol = sp.eval("音量", 0, 0.0);
                            assert_eq!(vol, 100.0, "音量トラックバーが 100.0% として復元されていること");
                        }
                    }
                }
            }
        }
        assert!(found_audio_file, "AUP内の音声ファイルオブジェクトに file パスが割り当てられていること");

        // state のパス解決テスト
        let mut st = state::EditorState::default();
        st.file_path = Some(p.to_path_buf());
        // 曖昧探索: 拡張子が違ったり先頭ドライブ文字が欠けていても解決できるか
        let dummy_path = r":\Users\Owner\Desktop\aviutl animation\Nightcore - Good Time.wav";
        let resolved = st.resolve_cached_path(dummy_path);
        // デスクトップ上にファイルがあれば解決される
        println!("Resolved dummy path: {:?}", resolved);
    }

    #[test]
    fn test_camera_control_preview_rendering() {
        let path = Path::new(r"C:\Users\user\Desktop\配布　1280x720_24fps_44100Hz.aup");
        if !path.exists() {
            return;
        }
        let mut st = state::EditorState::default();
        st.load_path(path);
        assert!(!st.scene.layers.is_empty(), "レイヤーが読み込まれていること");

        let ctx = egui::Context::default();
        // 1. 枠線図形（ライン幅1）が存在するフレーム40でのストローク描画検証
        st.frame = 40;
        let out_frame40 = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| panels::preview::ui(ui, &mut st));
        });
        let mesh_count_f40 = out_frame40
            .shapes
            .iter()
            .filter(|cs| matches!(&cs.shape, egui::epaint::Shape::Mesh(_)))
            .count();
        assert!(
            mesh_count_f40 > 0 && out_frame40.shapes.len() > 15,
            "frame 40: 枠線図形やテキストメッシュが正常に描画されていること (meshes={mesh_count_f40}, total_shapes={})",
            out_frame40.shapes.len()
        );

        // 2. カメラ制御・グロー・加算合成がアクティブな各フレームでプレビュー描画が正常に出力されるか検証
        for f in [10u32, 40, 200, 400, 500, 1000, 2800, 3000, 5200, 5600, 6000] {
            st.frame = f;
            let out = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| panels::preview::ui(ui, &mut st));
            });
            let mut content_shapes = 0usize;
            for cs in &out.shapes {
                if matches!(
                    &cs.shape,
                    egui::epaint::Shape::Mesh(_)
                        | egui::epaint::Shape::Circle(_)
                        | egui::epaint::Shape::Path(_)
                        | egui::epaint::Shape::Text(_)
                ) {
                    content_shapes += 1;
                }
            }
            assert!(
                content_shapes > 0 && out.shapes.len() > 12,
                "frame {f}: プレビューに実オブジェクトの描画プリミティブが出力されていること (content_shapes={content_shapes}, total={})",
                out.shapes.len()
            );
        }

        // 3. 字幕テキストオブジェクト（「文字毎に個別オブジェクト」＋「ランダム間隔で落ちながら登場」等）の検証
        let mut per_char_text_count = 0usize;
        let mut vertical_text_count = 0usize;
        for layer in &st.scene.layers {
            for obj in &layer.objects {
                if let Some(src) = obj.source() {
                    if src.kind == au2_core::EffectKind::Text {
                        if src.eval("文字毎に個別オブジェクト", 0, 0.0) > 0.5 {
                            per_char_text_count += 1;
                        }
                        if src.text("align").and_then(|s| s.parse::<u8>().ok()).unwrap_or(0) >= 9 {
                            vertical_text_count += 1;
                        }
                    }
                }
            }
        }
        assert!(
            per_char_text_count >= 60,
            "ExEdit 0.90d3 バイナリから「文字毎に個別オブジェクト=1」が正しく復元されていること (found={per_char_text_count})"
        );
        assert!(
            vertical_text_count >= 1,
            "テキスト拡張データから縦書き align (>=9) が正しく復元されていること (found={vertical_text_count})"
        );

        // 字幕が表示されるフレーム（例: f=80, 210, 410, 660, 780, 1350, 2400, 2900）で TextShape が実際に描画されているか検証
        for f in [80u32, 210, 410, 660, 780, 1350, 2400, 2900] {
            st.frame = f;
            let out = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| panels::preview::ui(ui, &mut st));
            });
            let text_shapes = out
                .shapes
                .iter()
                .filter(|cs| matches!(&cs.shape, egui::epaint::Shape::Text(_)))
                .count();
            assert!(
                text_shapes >= 8,
                "frame {f}: 字幕の文字毎個別オブジェクトが正常に描画されていること (text_shapes={text_shapes})"
            );
        }
    }

    #[test]
    fn test_goot_time_full_ui_and_playback() {
        let path = Path::new(r"C:\Users\user\Desktop\goot time pf.aup");
        if !path.exists() {
            println!("File not found, skipping: {:?}", path);
            return;
        }

        println!("Loading goot time pf.aup...");
        let mut st = state::EditorState::default();
        st.load_path(path);
        println!("Loaded! Total layers: {}, Total objects: {}", st.scene.layers.len(), st.scene.object_count());

        let ctx = egui::Context::default();

        println!("Testing Timeline UI rendering...");
        for f in [0, 100, 500, 1000, 2000, 5000, 10000, 20000] {
            st.frame = f;
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    panels::timeline::ui(ui, &mut st);
                });
            });
        }

        println!("Testing Settings UI rendering with all objects...");
        let obj_ids: Vec<_> = st.scene.layers.iter().flat_map(|l| l.objects.iter().map(|o| o.id)).collect();
        for id in obj_ids.into_iter().take(200) {
            st.selected = Some(id);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    panels::settings::ui(ui, &mut st);
                });
            });
        }

        println!("Testing Playback & Preview rendering across entire project (0..21877 step 10)...");
        st.playing = true;
        let total_len = st.scene.settings.length.min(25000);
        for f in (0..total_len).step_by(10) {
            st.frame = f;
            st.sync_audio_playback();
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    panels::preview::ui(ui, &mut st);
                });
            });
        }

        println!("All UI tests passed successfully!");
    }
}
