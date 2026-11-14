//! タイムライン右クリック時のコンテキストメニュー。

use au2_core::TimelineObject;
use eframe::egui::{self, Color32};

use crate::state::EditorState;

pub(crate) fn context_menu_ui(ui: &mut egui::Ui, st: &mut EditorState) {
    let target = st.timeline.context_target;
    let start = target.frame;
    let end = start + 149; // 既定 150 フレーム (30fps で 5 秒)

    ui.label(
        egui::RichText::new(format!("Layer {} / Frame {}", target.layer + 1, start + 1))
            .weak()
            .small(),
    );
    ui.separator();

    // 選択範囲 (In / Out 点) の設定
    ui.horizontal(|ui| {
        if ui.button("In 点に設定 (I)").clicked() {
            st.set_in_point(Some(start));
            ui.close_menu();
        }
        if ui.button("Out 点に設定 (O)").clicked() {
            st.set_out_point(Some(start));
            ui.close_menu();
        }
    });
    if st.timeline.in_point.is_some() || st.timeline.out_point.is_some() {
        if ui.button("選択範囲 (In/Out) を解除").clicked() {
            st.clear_range();
            ui.close_menu();
        }
    }
    ui.separator();

    // クリップボードにアイテムがあれば貼り付けを表示
    if !st.clipboard.is_empty() {
        if ui.button(format!("ここに貼り付け ({} 個のオブジェクト)", st.clipboard.len())).clicked() {
            st.paste_clipboard(Some(target.layer), Some(start));
            ui.close_menu();
        }
        ui.separator();
    }

    // 1. オブジェクト上で右クリックされた場合の操作メニュー
    if let Some(obj_id) = target.object_id {
        let raw_file_info: Option<(String, bool)> = st.scene.find(obj_id).and_then(|(_, obj)| {
            let src = obj.source()?;
            let f = src.text("file")?;
            let is_audio = src.kind == au2_core::EffectKind::AudioFile;
            Some((f.to_string(), is_audio))
        });
        let media_file_info = raw_file_info.map(|(f, is_audio)| {
            let is_missing = st.resolve_cached_path(&f).is_none();
            (f, is_audio, is_missing)
        });

        if let Some((old_file, is_audio, is_missing)) = media_file_info {
            let is_psd = au2_media::is_psd_path(std::path::Path::new(&old_file));
            let label = if is_missing {
                egui::RichText::new("⚠ 未検出ファイルを割り当て (MP3/WAV/動画)...")
                    .color(Color32::from_rgb(255, 100, 100))
                    .strong()
            } else {
                egui::RichText::new("メディアファイルを再割り当て...")
            };
            if ui.button(label).clicked() {
                ui.close_menu();
                st.relink_media_dialog(Some(obj_id), &old_file, is_audio);
            }
            if is_psd && !is_missing {
                ui.menu_button("PSDレイヤーごとにメディアを分解", |ui| {
                    if ui
                        .button("パーツ(フォルダ)単位でタイムラインへ分解配置")
                        .clicked()
                    {
                        ui.close_menu();
                        st.decompose_psd_object(obj_id, true, true);
                    }
                    if ui
                        .button("表示レイヤーを1枚ずつタイムラインへ分解配置")
                        .clicked()
                    {
                        ui.close_menu();
                        st.decompose_psd_object(obj_id, false, true);
                    }
                    ui.separator();
                    if ui
                        .button("パーツ(フォルダ)単位で透過PNG一括書き出し...")
                        .clicked()
                    {
                        ui.close_menu();
                        st.export_psd_layers_png_dialog(obj_id, true, false);
                    }
                    if ui
                        .button("表示レイヤーを1枚ずつ透過PNG一括書き出し...")
                        .clicked()
                    {
                        ui.close_menu();
                        st.export_psd_layers_png_dialog(obj_id, false, false);
                    }
                });
            }
            ui.separator();
        }

        if ui.button("コピー (Ctrl+C)").clicked() {
            if !st.selected_objects.contains(&obj_id) {
                st.select_single(obj_id);
            }
            st.copy_selected();
            ui.close_menu();
        }
        if ui.button("複製 (Ctrl+D)").clicked() {
            if !st.selected_objects.contains(&obj_id) {
                st.select_single(obj_id);
            }
            st.duplicate_selected();
            ui.close_menu();
        }

        ui.separator();

        if let Some((_, obj)) = st.scene.find_mut(obj_id) {
            let mut clip = obj.clipping;
            if ui.checkbox(&mut clip, "上のオブジェクトでクリッピング").changed() {
                obj.clipping = clip;
                st.dirty = true;
                st.status = if clip {
                    "上のオブジェクトでクリッピングを有効にしました".into()
                } else {
                    "上のオブジェクトでクリッピングを無効にしました".into()
                };
            }
        }

        if ui.button("◆ 中間点を追加").clicked() {
            st.push_undo("中間点を追加");
            if let Some((_, obj)) = st.scene.find_mut(obj_id) {
                if obj.add_midpoint(start) {
                    st.dirty = true;
                    st.status = format!("フレーム {} に中間点を追加しました", start + 1);
                } else {
                    st.status = "オブジェクト端または既存の中間点位置には追加できません".into();
                }
            }
            ui.close_menu();
        }
        if ui.button("クリック位置で分割").clicked() {
            st.push_undo("オブジェクト分割");
            let split_result = st
                .scene
                .find_mut(obj_id)
                .and_then(|(layer_idx, obj)| obj.split_at(start).map(|right| (layer_idx, right)));
            if let Some((layer_idx, right_obj)) = split_result {
                let right_id = right_obj.id;
                st.scene.insert_object(layer_idx, right_obj);
                st.select_single(right_id);
                st.dirty = true;
                st.status = format!("フレーム {} でオブジェクトを分割しました", start + 1);
            } else {
                st.status = "オブジェクトの先頭フレームでは分割できません".into();
            }
            ui.close_menu();
        }
        if ui.button("再生ヘッド位置で分割 (Ctrl+K)").clicked() {
            if !st.selected_objects.contains(&obj_id) {
                st.select_single(obj_id);
            }
            st.split_at_playhead();
            ui.close_menu();
        }
        if ui.button("削除 (Delete)").clicked() {
            if st.selected_objects.contains(&obj_id) && st.selected_objects.len() > 1 {
                st.delete_selected_objects();
            } else {
                st.push_undo("オブジェクト削除");
                if st.scene.remove_object(obj_id).is_some() {
                    if st.selected == Some(obj_id) {
                        st.selected = None;
                    }
                    st.selected_objects.remove(&obj_id);
                    st.dirty = true;
                    st.status = "オブジェクトを削除しました".into();
                }
            }
            ui.close_menu();
        }
        ui.separator();
    }

    // 2. メディアオブジェクトの追加メニュー（AviUtl 拡張編集互換）
    ui.menu_button("メディアオブジェクトの追加", |ui| {
        if ui.button("動画ファイル...").clicked() {
            ui.close_menu();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("動画ファイル", &["mp4", "avi", "mov", "mkv", "webm", "wmv", "gif"])
                .add_filter("すべてのファイル", &["*"])
                .pick_file()
            {
                let obj = TimelineObject::new_video(start, end, path.to_string_lossy());
                st.add_object_at(target.layer, obj);
            } else {
                let obj = TimelineObject::new_video(start, end, "");
                st.add_object_at(target.layer, obj);
            }
        }
        if ui.button("画像ファイル...").clicked() {
            ui.close_menu();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("画像・PSDファイル", &["png", "jpg", "jpeg", "bmp", "webp", "gif", "psd", "psb"])
                .add_filter("すべてのファイル", &["*"])
                .pick_file()
            {
                let obj = TimelineObject::new_image(start, end, path.to_string_lossy());
                st.add_object_at(target.layer, obj);
            } else {
                let obj = TimelineObject::new_image(start, end, "");
                st.add_object_at(target.layer, obj);
            }
        }
        if ui.button("PSDファイル (立ち絵・レイヤー切替)...").clicked() {
            ui.close_menu();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Photoshop PSD / PSB ファイル (*.psd, *.psb)", &["psd", "psb"])
                .add_filter("すべてのファイル", &["*"])
                .pick_file()
            {
                st.add_media_file(&path, target.layer, start);
            } else {
                let obj = TimelineObject::new_image(start, end, "");
                st.add_object_at(target.layer, obj);
            }
        }
        if ui.button("音声ファイル...").clicked() {
            ui.close_menu();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("音声ファイル", &["wav", "mp3", "ogg", "flac", "aac", "m4a"])
                .add_filter("すべてのファイル", &["*"])
                .pick_file()
            {
                let obj = TimelineObject::new_audio(start, end, path.to_string_lossy());
                st.add_object_at(target.layer, obj);
            } else {
                let obj = TimelineObject::new_audio(start, end, "");
                st.add_object_at(target.layer, obj);
            }
        }
        ui.separator();
        if ui.button("テキスト").clicked() {
            let obj = TimelineObject::new_text(start, end, "サンプルテキスト");
            st.add_object_at(target.layer, obj);
            ui.close_menu();
        }
        ui.menu_button("図形", |ui| {
            if ui.button("円").clicked() {
                st.add_object_at(target.layer, TimelineObject::new_shape(start, end, 1, "e94560"));
                ui.close_menu();
            }
            if ui.button("四角形").clicked() {
                st.add_object_at(target.layer, TimelineObject::new_shape(start, end, 2, "0f9b8e"));
                ui.close_menu();
            }
            if ui.button("三角形").clicked() {
                st.add_object_at(target.layer, TimelineObject::new_shape(start, end, 3, "f5a623"));
                ui.close_menu();
            }
            if ui.button("五角形").clicked() {
                st.add_object_at(target.layer, TimelineObject::new_shape(start, end, 4, "8b5cf6"));
                ui.close_menu();
            }
            if ui.button("六角形").clicked() {
                st.add_object_at(target.layer, TimelineObject::new_shape(start, end, 5, "38bdf8"));
                ui.close_menu();
            }
            if ui.button("星型").clicked() {
                st.add_object_at(target.layer, TimelineObject::new_shape(start, end, 6, "ffd166"));
                ui.close_menu();
            }
            if ui.button("背景").clicked() {
                st.add_object_at(target.layer, TimelineObject::new_shape(start, end, 0, "1a1a2e"));
                ui.close_menu();
            }
        });
        if ui.button("フレームバッファ").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_framebuffer(start, end));
            ui.close_menu();
        }
        if ui.button("音声波形表示").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_audio_waveform(start, end));
            ui.close_menu();
        }
        if ui.button("シーン").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_scene_object(start, end, 1));
            ui.close_menu();
        }
        if ui.button("直前オブジェクト").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_previous_object(start, end));
            ui.close_menu();
        }
        if ui.button("パーティクル出力").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_particle_output(start, end));
            ui.close_menu();
        }
        ui.menu_button("カスタムオブジェクト", |ui| {
            for name in [
                "集中線",
                "走査線",
                "カウンター",
                "レンズフレア",
                "雲",
                "星",
                "雪",
                "雨",
                "ランダム小物配置(カメラ制御)",
                "ライン(移動軌跡)",
                "扇型",
                "多角形",
                "周辺ボケ光量",
                "フレア",
                "水面",
            ] {
                if ui.button(name).clicked() {
                    st.add_object_at(target.layer, TimelineObject::new_custom_object(start, end, name));
                    ui.close_menu();
                }
            }
        });
        ui.separator();
        if ui.button("時間制御").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_time_control(start, end));
            ui.close_menu();
        }
        if ui.button("グループ制御").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_group_control(start, end));
            ui.close_menu();
        }
        if ui.button("カメラ制御").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_camera_control(start, end));
            ui.close_menu();
        }
    });

    // 3. フィルタオブジェクトの追加メニュー（AviUtl 拡張編集互換）
    ui.menu_button("フィルタオブジェクトの追加", |ui| {
        if ui.button("色調補正").clicked() {
            st.add_object_at(target.layer, TimelineObject::new_color_correction(start, end));
            ui.close_menu();
        }
        for fname in [
            "ぼかし",
            "レンズブラー",
            "放射ブラー",
            "方向ブラー",
            "グロー",
            "発光",
            "拡散光",
            "閃光",
            "ライト",
            "モザイク",
            "ラスター",
            "波紋",
            "色ずれ",
            "単色化",
            "グラデーション",
            "ノイズ",
            "ミラー",
            "反転",
        ] {
            if ui.button(fname).clicked() {
                let mut eff = au2_core::Effect::new(fname);
                eff.push_track("強さ", au2_core::Track::constant(60.0, 1));
                eff.push_track("範囲", au2_core::Track::constant(10.0, 0));
                st.add_object_at(target.layer, TimelineObject::new_filter_object(start, end, eff));
                ui.close_menu();
            }
        }
        ui.separator();
        ui.menu_button("シーンチェンジ", |ui| {
            for (sc_name, sc_type) in [
                ("クロスフェード", 0),
                ("ワイプ(円)", 1),
                ("ワイプ(四角)", 2),
                ("ワイプ(時計)", 3),
                ("スライス", 4),
                ("フェードアウトイン", 14),
                ("放射ブラー", 15),
                ("ぼかし", 16),
                ("発光", 0),
                ("レンズブラー", 1),
                ("ドア", 2),
                ("起き上がる", 3),
                ("リール回転", 4),
                ("図形ワイプ", 5),
                ("砕け散る", 8),
                ("ページめくり", 9),
            ] {
                if ui.button(sc_name).clicked() {
                    st.add_object_at(target.layer, TimelineObject::new_scene_change(start, end, sc_name, sc_type));
                    ui.close_menu();
                }
            }
        });
    });

    if ui.button("メディアファイルを選択して配置...").clicked() {
        ui.close_menu();
        st.add_media_dialog(target.layer, start);
    }

    ui.separator();

    // 4. レイヤー操作メニュー
    let layer_state = st.scene.layers.get(target.layer).map(|l| (l.visible, l.locked));
    if let Some((cur_vis, cur_lock)) = layer_state {
        let vis_label = if cur_vis {
            "レイヤーを非表示にする"
        } else {
            "レイヤーを表示する"
        };
        if ui.button(vis_label).clicked() {
            st.push_undo(if cur_vis { format!("Layer {} を非表示", target.layer + 1) } else { format!("Layer {} を表示", target.layer + 1) });
            if let Some(l) = st.scene.layers.get_mut(target.layer) {
                l.visible = !cur_vis;
            }
            st.dirty = true;
            ui.close_menu();
        }
        let lock_label = if cur_lock {
            "レイヤーのロックを解除"
        } else {
            "レイヤーをロックする"
        };
        if ui.button(lock_label).clicked() {
            st.push_undo(if cur_lock { format!("Layer {} のロックを解除", target.layer + 1) } else { format!("Layer {} をロック", target.layer + 1) });
            if let Some(l) = st.scene.layers.get_mut(target.layer) {
                l.locked = !cur_lock;
            }
            st.dirty = true;
            ui.close_menu();
        }
    }

    ui.separator();
    if ui
        .checkbox(&mut st.timeline.show_clip_previews, "クリップ内プレビューを表示")
        .changed()
    {
        ui.close_menu();
    }
}
