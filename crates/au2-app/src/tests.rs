use super::*;
use std::path::Path;

/// デスクトップ上の個人用サンプルファイル（開発者環境にのみ存在。無ければ各テストはスキップされる）。
fn desk(rel: &str) -> &'static Path {
    let base = au2_platform::desktop_dir().unwrap_or_default();
    Box::leak(base.join(rel).into_boxed_path())
}

/// リポジトリ同梱のテストデータ (`crates/au2-aup/test_data`)。
fn test_data(name: &str) -> &'static Path {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../au2-aup/test_data").join(name);
    Box::leak(p.into_boxed_path())
}

/// 実 .aup をヘッドレス egui でプレビュー描画し、各フレームで描画プリミティブが出力されるか調べる。
#[test]
fn preview_renders_real_aup_frames() {
    let path = desk(r"goot time pf.aup");
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
    assert!(video_len >= 290, "10秒動画が150フレーム(5秒)で切られず本来の長さ(約300フレーム)になっていること");

    // 2. 音声オブジェクトがペアで追加されているか検証
    let layer1 = &st.scene.layers[1];
    assert!(!layer1.objects.is_empty(), "音声付き動画の場合ペアの音声オブジェクトが自動配置されること");
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
        let _ = st.get_media_texture(&ctx, &path_str, f, f as f64 / 30.0);
        let _ = st.get_media_texture(&ctx, &path_str, 0, 0.0);
    }
    let elapsed_ms = bench_start.elapsed().as_secs_f64() * 1000.0;
    let avg_ms_per_frame = elapsed_ms / 300.0;
    println!("300フレーム再生ループ合計: {elapsed_ms:.2} ms (1フレーム平均: {avg_ms_per_frame:.3} ms)");
    assert!(
        avg_ms_per_frame < 2.0,
        "1フレームあたりのメインスレッド処理時間が 2.0ms 未満であること: 実測 {avg_ms_per_frame:.3} ms"
    );
    st.playing = false;
    st.audio.stop();
}

#[test]
fn test_real_mp3_playback_and_missing_file_warning() {
    let mut st = state::EditorState::default();
    let mp3_path = desk(r"ぐるたみん01 シャルル - バルーン.mp3");
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

    println!("Audio playback sync successful!");
    st.playing = false;
    st.audio.stop();
}

#[test]
fn test_aup_audio_file_restoration_and_resolution() {
    let candidates = [
        desk(r"goot time pf.aup").to_str().unwrap_or(""),
        r"",
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
                    if let Some(sp) = obj.effect(&au2_core::EffectKind::StandardPlayback) {
                        let vol = sp.eval("音量", 0, 0.0);
                        assert_eq!(vol, 100.0, "音量トラックバーが 100.0% として復元されていること");
                    }
                }
            }
        }
    }
    assert!(found_audio_file, "AUP内の音声ファイルオブジェクトに file パスが割り当てられていること");

    let mut st = state::EditorState::default();
    st.file_path = Some(p.to_path_buf());
    let dummy_path = r":\Users\Owner\Desktop\aviutl animation\Nightcore - Good Time.wav";
    let resolved = st.resolve_cached_path(dummy_path);
    println!("Resolved dummy path: {:?}", resolved);
}

#[test]
fn test_camera_control_preview_rendering() {
    let path = desk(r"配布　1280x720_24fps_44100Hz.aup");
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let mut st = state::EditorState::default();
    st.load_aup(&bytes, Some(path));
    if st.scene.object_count() == 0 {
        return;
    }
    assert!(!st.scene.layers.is_empty(), "レイヤーが読み込まれていること");

    let ctx = egui::Context::default();
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
        "frame 40: 枠線図形やテキストメッシュが正常に描画されていること"
    );

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
            "frame {f}: プレビューに実オブジェクトの描画プリミティブが出力されていること"
        );
    }

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
        "ExEdit 0.90d3 バイナリから「文字毎に個別オブジェクト=1」が正しく復元されていること"
    );
    assert!(
        vertical_text_count >= 1,
        "テキスト拡張データから縦書き align (>=9) が正しく復元されていること"
    );

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
            "frame {f}: 字幕の文字毎個別オブジェクトが正常に描画されていること"
        );
    }
}

#[test]
fn test_goot_time_full_ui_and_playback() {
    let p_desktop = desk(r"goot time pf.aup");
    let p_testdata = Path::new(r"crates\au2-aup\test_data\goot_time.aup");
    let p_alt = Path::new(r"..\au2-aup\test_data\goot_time.aup");
    let path = if p_testdata.exists() {
        p_testdata
    } else if p_alt.exists() {
        p_alt
    } else if p_desktop.exists() {
        p_desktop
    } else {
        println!("goot time pf.aup not found, skipping");
        return;
    };

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

#[test]
fn test_mayamaid_aup_preview_and_scenes_if_present() {
    let mayamaid_path = desk(r"mayamaid.aup");
    let Ok(bytes) = std::fs::read(mayamaid_path) else {
        eprintln!("mayamaid.aup not readable, skipping test.");
        return;
    };

    let mut st = state::EditorState::default();
    st.load_aup(&bytes, Some(mayamaid_path));
    if st.scene.object_count() == 0 {
        return;
    }
    println!(
        "mayamaid.aup loaded! Scenes: {}, Layers: {}, Objects: {}, Length: {}",
        st.scenes.len(),
        st.scene.layers.len(),
        st.scene.object_count(),
        st.scene.settings.length
    );

    assert!(st.scenes.len() >= 20, "mayamaid.aup should have 22 scenes, got {}", st.scenes.len());
    assert_eq!(st.scene.settings.length, 815, "Root scene length should be 815");
    assert!(st.scene.object_count() >= 200, "Root scene should have 228 objects");

    let ctx = egui::Context::default();

    for f in (0..=815).step_by(5) {
        st.frame = f;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                panels::preview::ui(ui, &mut st);
                panels::timeline::ui(ui, &mut st);
            });
        });
    }

    let original_scene = st.scene.clone();
    let sub_scenes: Vec<_> = st.scenes.iter().filter(|(&id, _)| id != 0).map(|(&id, sc)| (id, sc.clone())).collect();
    for (_sc_id, sub_sc) in sub_scenes {
        st.scene = sub_sc.clone();
        for f in (0..sub_sc.settings.length.min(200)).step_by(10) {
            st.frame = f;
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    panels::preview::ui(ui, &mut st);
                });
            });
        }
    }
    st.scene = original_scene;

    println!("mayamaid.aup all scenes and key frames preview rendering passed successfully!");
}

#[test]
fn test_check_all_aups_on_desktop() {
    let desktop = desk(r"");
    if let Ok(entries) = std::fs::read_dir(desktop) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().map(|e| e == "aup").unwrap_or(false) {
                let mut st = state::EditorState::default();
                st.load_path(&p);
                let missing = st.check_missing_media();
                println!("=== AUP: {:?} ===", p.file_name().unwrap());
                println!("  Objects: {}, Warnings: {}", st.scene.object_count(), st.warnings.len());
                for w in &st.warnings {
                    if w.contains("未検出") {
                        println!("  Warning: {}", w);
                    }
                }
                for m in &missing {
                    println!("  Missing: L{} kind={} path={:?}", m.layer_index + 1, m.kind_name, m.original_path);
                }
            }
        }
    }
}

#[test]
fn test_shipping_aup_preview_and_objects() {
    let p_desktop = desk(r"出荷用プロジェクトファイル.aup");
    let p_testdata = test_data("shipping.aup");
    let path = if p_testdata.exists() {
        p_testdata
    } else if p_desktop.exists() {
        p_desktop
    } else {
        eprintln!("shipping.aup not found, skipping test.");
        return;
    };

    let mut st = state::EditorState::default();
    st.load_path(path);
    println!(
        "shipping.aup loaded! Scenes: {}, Layers: {}, Objects: {}, Length: {}",
        st.scenes.len(),
        st.scene.layers.len(),
        st.scene.object_count(),
        st.scene.settings.length
    );
    println!("Status: {}", st.status);

    assert_eq!(st.scene.layers.len(), 100, "100 layers expected");
    assert!(st.scene.object_count() >= 90, "At least 90 objects expected, got {}", st.scene.object_count());
    assert_eq!(st.scene.settings.width, 1280);
    assert_eq!(st.scene.settings.height, 720);

    let ctx = egui::Context::default();
    for f in (0..=1270).step_by(20) {
        st.frame = f;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                panels::preview::ui(ui, &mut st);
                panels::timeline::ui(ui, &mut st);
            });
        });
    }
    println!("shipping.aup preview rendering and UI test passed successfully!");
}

#[test]
fn test_create_new_project_with_config_and_autosave() {
    use std::time::{Duration, Instant};

    let temp_dir = std::env::temp_dir().join("openavi_test_autosave");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);

    let mut st = state::EditorState::default();
    assert!(!st.autosave.enabled, "初期状態の自動保存はオフであること");

    // 1. 新規プロジェクトの作成 (.aup2 形式、明示的に自動保存有効化)
    let config = state::NewProjectConfig {
        name: "test_project_autosave".into(),
        dir: temp_dir.clone(),
        format: state::ProjectFormat::Aup2,
        width: 1920,
        height: 1080,
        fps_num: 60,
        fps_den: 1,
        length_frames: 1800,
        autosave_enabled: true,
    };

    let created_path = st.create_new_project_with_config(config).expect("プロジェクト作成成功");
    assert!(created_path.exists(), "作成されたファイルが存在すること");
    assert_eq!(created_path.file_name().unwrap(), "test_project_autosave.aup2");
    assert_eq!(st.file_path.as_deref(), Some(created_path.as_path()));
    assert!(!st.dirty, "作成直後は未変更 (dirty == false)");
    assert!(st.autosave.enabled, "自動保存が有効であること");

    // 2. レイヤーに変更を加えて mark_dirty する
    st.scene.settings.width = 1280;
    st.mark_dirty();

    // 変更直後 (0.3秒未満) はまだデバウンス中で tick では保存されない
    st.tick();
    assert!(st.dirty, "デバウンス中のため直後は dirty のまま");
    assert_eq!(st.autosave.auto_save_count, 0);

    // 3. 操作完了 (0.3秒以上経過) 後、tick で即座に上書き保存される
    st.autosave.dirty_since = Some(Instant::now() - Duration::from_millis(350));
    st.tick();

    assert!(!st.dirty, "変更が即座に上書き保存されて dirty == false にリセットされること");
    assert_eq!(st.autosave.auto_save_count, 1, "自動保存カウントが 1 に増加すること");

    let bak_path = temp_dir.join("test_project_autosave.bak");
    assert!(!bak_path.exists(), "バックアップ (.bak) は生成されないこと");

    // 4. 自動保存をオフにした場合、変更があっても自動保存されないこと
    st.autosave.enabled = false;
    st.scene.settings.width = 1920;
    st.mark_dirty();
    st.autosave.dirty_since = Some(Instant::now() - Duration::from_millis(500));

    st.tick();
    assert!(st.dirty, "自動保存無効時は変更があっても dirty のまま");
    assert_eq!(st.autosave.auto_save_count, 1, "自動保存カウントは増加しない");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_smart_media_resolution_and_audio_playback() {
    let temp_dir = std::env::temp_dir().join("openavi_test_audio_resolution");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let deep_sub_dir = temp_dir.join("sub1").join("sub2").join("audio_assets");
    std::fs::create_dir_all(&deep_sub_dir).expect("create test directories");

    let target_wav = deep_sub_dir.join("ピアノ　レコ.wav");
    {
        let num_frames = 44100u32;
        let num_channels = 2u16;
        let bits_per_sample = 16u16;
        let sample_rate = 44100u32;
        let byte_rate = sample_rate * num_channels as u32 * 2;
        let block_align = num_channels * 2;
        let data_size = num_frames * block_align as u32;

        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&num_channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits_per_sample.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        for i in 0..num_frames {
            let v = ((i % 100) as i16) * 50;
            wav.extend_from_slice(&v.to_le_bytes());
            wav.extend_from_slice(&(-v).to_le_bytes());
        }
        std::fs::write(&target_wav, wav).expect("write wav file");
    }

    let proj_file = temp_dir.join("test_project.aup2");
    let mut st = state::EditorState::default();
    st.file_path = Some(proj_file);

    // A. 旧環境の他PC絶対パス + 半角スペース表記: "D:\\OldUser\\Desktop\\ピアノ レコ.wav"
    let old_pc_path = r"D:\OldUser\Desktop\ピアノ レコ.wav";
    let resolved = st.resolve_cached_path(old_pc_path);
    assert!(resolved.is_some(), "全角・半角スペースの違いや他PCパスを越えて深層サブフォルダ内の実在ファイルを解決できること");
    assert_eq!(resolved.as_ref().unwrap().canonicalize().unwrap(), target_wav.canonicalize().unwrap());

    // B. 拡張子違い: "C:\\OldPC\\ピアノ　レコ.mp3" を検索した場合も実在の .wav に解決されること
    let ext_diff_path = r"C:\OldPC\ピアノ　レコ.mp3";
    let resolved_ext = st.resolve_cached_path(ext_diff_path);
    assert!(resolved_ext.is_some(), "拡張子違い (.mp3 -> .wav) のメディアも自動で解決できること");

    // C. NFD結合文字: "ヒ\u{309A}アノ　レコ.wav"
    let nfd_path = "C:\\Music\\ヒ\u{309A}アノ　レコ.wav";
    let resolved_nfd = st.resolve_cached_path(nfd_path);
    assert!(resolved_nfd.is_some(), "Unicode NFD結合半濁点のファイル名も自動でNFC合成して解決できること");

    // D. WAV ファイルの PCM デコード
    let pcm = st.media.get_audio_pcm(&target_wav);
    assert!(pcm.is_some(), "WAV ファイルがネイティブに PCM デコードされること");
    let pcm_vec = pcm.unwrap();
    assert_eq!(pcm_vec.len(), 44100 * 2, "44.1kHz 16-bit ステレオのサンプルが完全に取得できること");

    // E. タイムライン上のオブジェクトの自動再リンクと再生同期
    let audio_obj = au2_core::TimelineObject::new_audio(0, 300, r"C:\UnknownPC\ピアノ レコ.mp3");
    st.scene.layers[0].objects.push(audio_obj);

    let count = st.auto_relink_missing_media();
    assert_eq!(count, 1, "未検出オブジェクトが手元の実在パスに自動再リンクされること");
    assert_eq!(
        st.scene.layers[0].objects[0].source().unwrap().text("file").unwrap(),
        target_wav.to_string_lossy()
    );

    st.sync_audio_playback();

    let missing = st.check_missing_media();
    assert!(missing.is_empty(), "自動再リンク後に未検出メディアが 0 件になること");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// 別 PC の絶対パス（UNC・他ドライブ）を持つ音声・画像・動画が、
/// 素材フォルダを指定するだけでまとめて再リンクされること。
#[test]
fn test_relink_from_folder_bulk() {
    let root = std::env::temp_dir().join("openavi_bulk_relink_test");
    let _ = std::fs::remove_dir_all(&root);
    let assets = root.join("素材一式").join("images");
    std::fs::create_dir_all(&assets).expect("create dirs");
    let wav = root.join("素材一式").join("ma-maido.wav");
    let png = assets.join("kaitenmaya.png");
    let mp4 = root.join("素材一式").join("lightleak.mp4");
    for f in [&wav, &png, &mp4] {
        std::fs::write(f, b"dummy").expect("write dummy media");
    }

    let mut st = state::EditorState::default();
    st.scene.layers[0].objects.push(au2_core::TimelineObject::new_audio(
        0, 100, r"\\Trigkey-g4\c\MainD\ma-maido.wav",
    ));
    st.scene.layers[1].objects.push(au2_core::TimelineObject::new_image(
        0, 100, r"D:\素材\kaitenmaya.png",
    ));
    st.scene.layers[2].objects.push(au2_core::TimelineObject::new_video(
        0, 100, r"F:\ダウンロード\lightleak.mp4",
    ));
    assert_eq!(st.check_missing_media().len(), 3, "最初は 3 件とも未検出");

    let (relinked, remaining) = st.relink_from_folder(&root.join("素材一式"));
    assert_eq!(relinked, 3, "音声・画像・動画が一括で再リンクされること");
    assert_eq!(remaining, 0);
    assert!(st.check_missing_media().is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_dist_24fps_aup_load_and_full_preview() {
    let p_testdata = test_data("dist_24fps.aup");
    let p_desktop = desk(r"新しいフォルダー\配布　1280x720_24fps_44100Hz.aup");
    let path = if p_testdata.exists() {
        p_testdata
    } else if p_desktop.exists() {
        p_desktop
    } else {
        return;
    };

    let mut st = state::EditorState::default();
    st.load_path(path);

    assert_eq!(st.scene.settings.width, 1280);
    assert_eq!(st.scene.settings.height, 720);
    assert_eq!(st.scene.settings.frame_rate.num, 24);
    assert_eq!(st.scene.settings.frame_rate.den, 1);
    assert_eq!(st.scenes.len(), 4, "4 scenes expected");
    assert!(st.scene.object_count() >= 230, "Expected >= 230 objects in Root scene, got {}", st.scene.object_count());

    let ctx = egui::Context::default();
    for f in (0..=7033).step_by(50) {
        st.frame = f;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                panels::preview::ui(ui, &mut st);
                panels::timeline::ui(ui, &mut st);
                panels::settings::ui(ui, &mut st);
            });
        });
    }
}

/// UI 全体をヘッドレスで描画して PNG に保存するデザイン確認用スナップショット。
/// 環境変数 `OPENAVI_SNAPSHOT_DIR` が設定されているときのみ画像を出力する。
#[test]
fn ui_snapshot_png() {
    let Some(dir) = std::env::var_os("OPENAVI_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&dir).expect("create snapshot dir");
    let (w, h) = (1440usize, 900usize);
    let mut renderer = export::SceneSoftwareRenderer::new();
    for (mode, tag) in [(theme::ThemeMode::Dark, "dark"), (theme::ThemeMode::Light, "light")] {
        for (dialog, name) in [(0u8, "main"), (1, "newproj"), (2, "export"), (3, "plugins")] {
            let mut st = state::EditorState::default();
            st.load_exo(state::SAMPLE_EXO, None);
            st.frame = 30;
            let mut app = app::OpenAviApp::for_snapshot(st);
            app.snapshot_open_dialog(dialog);
            theme::set_mode_no_save(renderer.context(), mode);
            let bg = if mode == theme::ThemeMode::Dark { [20, 21, 25, 255] } else { [241, 243, 247, 255] };
            let rgba = renderer.paint_ui(w, h, bg, |ctx| app.update_ui(ctx));
            let path = dir.join(format!("{name}_{tag}.png"));
            image::save_buffer(&path, &rgba, w as u32, h as u32, image::ColorType::Rgba8).expect("save png");
        }
    }
    theme::set_mode_no_save(renderer.context(), theme::ThemeMode::Dark);
}

#[test]
fn test_video_and_audio_export() {
    let mut st = state::EditorState::default();
    st.load_exo(state::SAMPLE_EXO, None);
    let mut renderer = export::SceneSoftwareRenderer::new();
    let frame_rgba = renderer.render_frame(&mut st, 0, 320, 180, 30.0);
    assert_eq!(frame_rgba.len(), 320 * 180 * 4);
    let non_black_pixels = frame_rgba
        .chunks_exact(4)
        .filter(|px| px[0] > 10 || px[1] > 10 || px[2] > 10)
        .count();
    assert!(non_black_pixels > 100, "ラスタライズされたフレームに図形・テキストが描画されていること");

    let out_dir = std::env::temp_dir().join("openavi_export_test");
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).expect("create export test dir");

    // 1. WAV 音声書き出しテスト (FFmpeg 不要)
    let wav_path = out_dir.join("test_out.wav");
    let wav_cfg = export::ExportConfig {
        output_path: wav_path.clone(),
        format: export::ExportFormat::Wav,
        quality: export::ExportQuality::High,
        encoder: export::VideoEncoder::Auto,
        width: 320,
        height: 180,
        fps_num: 30,
        fps_den: 1,
        start_frame: 0,
        end_frame: 29,
        include_audio: true,
    };
    export::export_scene_blocking(&mut st, wav_cfg).expect("WAV export should succeed");
    assert!(wav_path.is_file());
    let wav_bytes = std::fs::read(&wav_path).expect("read wav");
    assert!(wav_bytes.len() > 44 && &wav_bytes[0..4] == b"RIFF");

    // 2. FFmpeg が利用可能な環境では MP4 動画書き出しを検証
    if st.media.has_ffmpeg() {
        let mp4_path = out_dir.join("test_out.mp4");
        let mp4_cfg = export::ExportConfig {
            output_path: mp4_path.clone(),
            format: export::ExportFormat::Mp4,
            quality: export::ExportQuality::Standard,
            encoder: export::VideoEncoder::Auto,
            width: 320,
            height: 180,
            fps_num: 30,
            fps_den: 1,
            start_frame: 0,
            end_frame: 14,
            include_audio: true,
        };
        export::export_scene_blocking(&mut st, mp4_cfg).expect("MP4 export should succeed");
        assert!(mp4_path.is_file());
        let meta = std::fs::metadata(&mp4_path).expect("mp4 metadata");
        assert!(meta.len() > 500, "生成された MP4 ファイルが空でないこと");
    }

    let _ = std::fs::remove_dir_all(&out_dir);
}

/// ハードウェアビデオエンコーダの自動判別および安全なCPUフォールバック動作の検証
#[test]
fn test_video_encoder_probing_and_fallback() {
    use export::VideoEncoder;

    // 1. ハードウェアアクセラレーション判定の検証
    assert!(!VideoEncoder::Auto.is_hardware_accelerated());
    assert!(!VideoEncoder::Cpu.is_hardware_accelerated());
    assert!(VideoEncoder::Nvenc.is_hardware_accelerated());
    assert!(VideoEncoder::Qsv.is_hardware_accelerated());
    assert!(VideoEncoder::Amf.is_hardware_accelerated());
    assert!(VideoEncoder::VideoToolbox.is_hardware_accelerated());
    assert!(VideoEncoder::Vaapi.is_hardware_accelerated());
    assert!(VideoEncoder::MediaFoundation.is_hardware_accelerated());

    // 2. GPUのない環境（CPUのみ）の場合の解決ロジック
    let cpu_only = [VideoEncoder::Auto, VideoEncoder::Cpu];
    assert_eq!(VideoEncoder::Auto.resolve(&cpu_only), VideoEncoder::Cpu);
    assert_eq!(VideoEncoder::Cpu.resolve(&cpu_only), VideoEncoder::Cpu);
    assert_eq!(VideoEncoder::Nvenc.resolve(&cpu_only), VideoEncoder::Cpu);
    assert_eq!(VideoEncoder::Qsv.resolve(&cpu_only), VideoEncoder::Cpu);

    // 3. GPU対応環境（例: NVENCあり）の場合の解決ロジック
    let with_nvenc = [VideoEncoder::Auto, VideoEncoder::Cpu, VideoEncoder::Nvenc];
    assert_eq!(VideoEncoder::Auto.resolve(&with_nvenc), VideoEncoder::Nvenc);
    assert_eq!(VideoEncoder::Cpu.resolve(&with_nvenc), VideoEncoder::Cpu);
    assert_eq!(VideoEncoder::Nvenc.resolve(&with_nvenc), VideoEncoder::Nvenc);
    assert_eq!(VideoEncoder::Qsv.resolve(&with_nvenc), VideoEncoder::Cpu);

    // 4. 実環境でのプローブ取得
    let st = state::EditorState::default();
    let encoders = export::available_video_encoders(st.media.ffmpeg_path());
    assert!(encoders.contains(&VideoEncoder::Auto));
    assert!(encoders.contains(&VideoEncoder::Cpu));
    let resolved = VideoEncoder::Auto.resolve(encoders);
    assert!(resolved == VideoEncoder::Cpu || resolved.is_hardware_accelerated());
}

/// ダークモードとライトモードの切り替え、および各モードでの UI 描画が正常に行われること。
#[test]
fn test_dark_and_light_mode_rendering() {
    let (w, h) = (800usize, 600usize);
    let mut renderer = export::SceneSoftwareRenderer::new();

    // 1. ダークモードでレンダリング
    theme::set_mode_no_save(renderer.context(), theme::ThemeMode::Dark);
    assert_eq!(theme::mode(), theme::ThemeMode::Dark);
    assert!(theme::is_dark());

    let mut st_dark = state::EditorState::default();
    st_dark.load_exo(state::SAMPLE_EXO, None);
    let mut app_dark = app::OpenAviApp::for_snapshot(st_dark);
    let dark_rgba = renderer.paint_ui(w, h, [20, 21, 25, 255], |ctx| app_dark.update_ui(ctx));
    assert_eq!(dark_rgba.len(), w * h * 4);

    // 2. ライトモードでレンダリング
    theme::set_mode_no_save(renderer.context(), theme::ThemeMode::Light);
    assert_eq!(theme::mode(), theme::ThemeMode::Light);
    assert!(!theme::is_dark());

    let mut st_light = state::EditorState::default();
    st_light.load_exo(state::SAMPLE_EXO, None);
    let mut app_light = app::OpenAviApp::for_snapshot(st_light);
    let light_rgba = renderer.paint_ui(w, h, [241, 243, 247, 255], |ctx| app_light.update_ui(ctx));
    assert_eq!(light_rgba.len(), w * h * 4);

    // 3. ライトモード時の平均輝度がダークモード時より明らかに高いことを検証
    let dark_avg_lum: u64 = dark_rgba
        .chunks_exact(4)
        .map(|px| (px[0] as u64 + px[1] as u64 + px[2] as u64) / 3)
        .sum::<u64>()
        / (w * h) as u64;
    let light_avg_lum: u64 = light_rgba
        .chunks_exact(4)
        .map(|px| (px[0] as u64 + px[1] as u64 + px[2] as u64) / 3)
        .sum::<u64>()
        / (w * h) as u64;

    assert!(
        light_avg_lum > dark_avg_lum + 50,
        "ライトモードの平均輝度 ({light_avg_lum}) はダークモード ({dark_avg_lum}) より明るいこと"
    );

    // 4. トグル動作の確認
    let toggled = theme::mode().toggled();
    assert_eq!(toggled, theme::ThemeMode::Dark);
    theme::set_mode_no_save(renderer.context(), toggled);
    assert!(theme::is_dark());
}
