//! 設定ダイアログ（選択中オブジェクトのパラメータ編集）。
//!
//! AviUtl と同様、中間点を持つトラックは「現在フレームを含む区間」の
//! 左（区間開始）・右（区間終了）の値を編集します。
//! また、右上の「＋ フィルタ効果を追加」から内蔵フィルタや `plugins/` 内の `.anm` スクリプトを追加できます。

use au2_core::{Effect, EffectKind, Frame, Param, ParamValue, Track};
use au2_plugin::PluginKind;
use eframe::egui::{self, CollapsingHeader, Color32, DragValue, Grid, RichText, Stroke, TextEdit};

use crate::state::EditorState;

pub fn ui(ui: &mut egui::Ui, st: &mut EditorState) {
    let Some(id) = st.selected else {
        scene_settings_ui(ui, st);
        return;
    };
    let frame = st.frame;

    // プラグイン一覧から追加可能なスクリプト効果・フィルタプラグイン (32bit/64bit .auf) を収集
    let available_scripts: Vec<(PluginKind, au2_plugin::ScriptEntry)> = st
        .plugins
        .plugins
        .iter()
        .filter(|p| p.kind.can_apply_to_object())
        .flat_map(|p| p.entries.iter().cloned().map(move |e| (p.kind, e)))
        .collect();

    // 対象オブジェクトの file パラメータを取得＆事前解決
    let current_file_info: Option<(String, Option<std::path::PathBuf>, bool)> = {
        let raw = if let Some((_, obj)) = st.scene.find_mut(id) {
            obj.source().and_then(|s| s.text("file")).map(|f| {
                let is_audio = obj.source().map_or(false, |s| s.kind == EffectKind::AudioFile);
                (f.to_string(), is_audio)
            })
        } else {
            None
        };
        raw.map(|(f, is_audio)| {
            let res = st.resolve_cached_path(&f);
            (f, res, is_audio)
        })
    };

    let Some((layer, obj)) = st.scene.find_mut(id) else {
        st.selected = None;
        return;
    };

    let rel = frame.saturating_sub(obj.frame_start).min(obj.length() - 1);
    let keys = obj.key_frames();
    let seg = keys.partition_point(|&k| k <= rel).saturating_sub(1).min(keys.len().saturating_sub(2));

    let mut pending_effect: Option<Effect> = None;
    let mut pending_relink: Option<(String, bool)> = None;
    let mut file_changed_flag = false;

    egui::Frame::none()
        .fill(Color32::from_rgb(24, 26, 33))
        .stroke(Stroke::new(1.0_f32, Color32::from_gray(40)))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new(obj.display_name()).strong().size(15.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button("＋ フィルタ効果を追加", |ui| {
                        // 1. 発光・ブラー・装飾系
                        ui.menu_button("✨ 発光・ライト・ブラー", |ui| {
                            if ui.button("グロー").clicked() {
                                let mut eff = Effect::new("グロー");
                                eff.push_track("強さ", Track::constant(60.0, 1));
                                eff.push_track("拡散", Track::constant(30.0, 0));
                                eff.push_track("しきい値", Track::constant(50.0, 1));
                                eff.push_track("ぼかし", Track::constant(1.0, 0));
                                eff.push_track("光成分のみ", Track::constant(0.0, 0));
                                eff.push_text("color", "");
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("発光").clicked() {
                                let mut eff = Effect::new("発光");
                                eff.push_track("強さ", Track::constant(100.0, 1));
                                eff.push_track("拡散", Track::constant(50.0, 0));
                                eff.push_track("しきい値", Track::constant(70.0, 1));
                                eff.push_track("拡散速度", Track::constant(0.0, 0));
                                eff.push_track("光成分のみ", Track::constant(0.0, 0));
                                eff.push_text("color", "");
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("拡散光").clicked() {
                                let mut eff = Effect::new("拡散光");
                                eff.push_track("強さ", Track::constant(50.0, 1));
                                eff.push_track("拡散", Track::constant(30.0, 0));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("閃光").clicked() {
                                let mut eff = Effect::new("閃光");
                                eff.push_track("強さ", Track::constant(60.0, 1));
                                eff.push_track("X", Track::constant(0.0, 1));
                                eff.push_track("Y", Track::constant(0.0, 1));
                                eff.push_text("color", "ffffff");
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("ライト").clicked() {
                                let mut eff = Effect::new("ライト");
                                eff.push_track("強さ", Track::constant(100.0, 1));
                                eff.push_track("拡散", Track::constant(60.0, 0));
                                eff.push_track("比率", Track::constant(0.0, 1));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            ui.separator();
                            for blur_name in ["ぼかし", "境界ぼかし", "レンズブラー", "放射ブラー", "方向ブラー", "モーションブラー"] {
                                if ui.button(blur_name).clicked() {
                                    let mut eff = Effect::new(blur_name);
                                    eff.push_track("範囲", Track::constant(10.0, 0));
                                    pending_effect = Some(eff);
                                    ui.close_menu();
                                }
                            }
                        });

                        // 2. 装飾・縁取り・シャドー系
                        ui.menu_button("🖋 縁取り・シャドー・エッジ", |ui| {
                            if ui.button("縁取り").clicked() {
                                let mut eff = Effect::new("縁取り");
                                eff.push_track("サイズ", Track::constant(4.0, 0));
                                eff.push_track("ぼかし", Track::constant(5.0, 0));
                                eff.push_text("color", "ffffff");
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("シャドー").clicked() {
                                let mut eff = Effect::new("シャドー");
                                eff.push_track("X", Track::constant(6.0, 1));
                                eff.push_track("Y", Track::constant(6.0, 1));
                                eff.push_track("濃さ", Track::constant(60.0, 1));
                                eff.push_track("拡散", Track::constant(5.0, 0));
                                eff.push_text("color", "000000");
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("凸エッジ").clicked() {
                                let mut eff = Effect::new("凸エッジ");
                                eff.push_track("幅", Track::constant(5.0, 0));
                                eff.push_track("高さ", Track::constant(100.0, 2));
                                eff.push_track("角度", Track::constant(-45.0, 1));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("エッジ抽出").clicked() {
                                let mut eff = Effect::new("エッジ抽出");
                                eff.push_track("強さ", Track::constant(100.0, 1));
                                eff.push_track("しきい値", Track::constant(0.0, 1));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("シャープ").clicked() {
                                let mut eff = Effect::new("シャープ");
                                eff.push_track("強さ", Track::constant(100.0, 0));
                                eff.push_track("範囲", Track::constant(3.0, 0));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                        });

                        // 3. 色調・グラデーション・キーイング系
                        ui.menu_button("🎨 色調・グラデーション・キーイング", |ui| {
                            if ui.button("色調補正").clicked() {
                                let mut eff = Effect::new("色調補正");
                                eff.push_track("明るさ", Track::constant(100.0, 1));
                                eff.push_track("コントラスト", Track::constant(100.0, 1));
                                eff.push_track("色相", Track::constant(0.0, 1));
                                eff.push_track("輝度", Track::constant(100.0, 1));
                                eff.push_track("彩度", Track::constant(100.0, 1));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("グラデーション").clicked() {
                                let mut eff = Effect::new("グラデーション");
                                eff.push_track("強さ", Track::constant(100.0, 1));
                                eff.push_track("中心X", Track::constant(0.0, 0));
                                eff.push_track("中心Y", Track::constant(0.0, 0));
                                eff.push_track("角度", Track::constant(0.0, 1));
                                eff.push_track("幅", Track::constant(100.0, 0));
                                eff.push_text("color", "ffffff");
                                eff.push_text("color2", "000000");
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("単色化").clicked() {
                                let mut eff = Effect::new("単色化");
                                eff.push_track("強さ", Track::constant(100.0, 1));
                                eff.push_track("輝度を保持する", Track::constant(1.0, 0));
                                eff.push_text("color", "ffcc66");
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("色ずれ (色収差)").clicked() {
                                let mut eff = Effect::new("色ずれ");
                                eff.push_track("ずれ幅", Track::constant(5.0, 0));
                                eff.push_track("角度", Track::constant(0.0, 1));
                                eff.push_track("強さ", Track::constant(100.0, 0));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("反転").clicked() {
                                let mut eff = Effect::new("反転");
                                eff.push_track("上下反転", Track::constant(0.0, 0));
                                eff.push_track("左右反転", Track::constant(0.0, 0));
                                eff.push_track("輝度反転", Track::constant(0.0, 0));
                                eff.push_track("色相反転", Track::constant(0.0, 0));
                                eff.push_track("透明度反転", Track::constant(0.0, 0));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            ui.separator();
                            for key_name in ["クロマキー", "カラーキー", "ルミナンスキー", "特定色域変換", "拡張色設定"] {
                                if ui.button(key_name).clicked() {
                                    let mut eff = Effect::new(key_name);
                                    eff.push_track("しきい値", Track::constant(50.0, 0));
                                    pending_effect = Some(eff);
                                    ui.close_menu();
                                }
                            }
                        });

                        // 4. クリッピング・マスク・ワイプ・フェード
                        ui.menu_button("✂ フェード・ワイプ・マスク・クリッピング", |ui| {
                            if ui.button("フェード").clicked() {
                                let mut eff = Effect::new("フェード");
                                eff.push_track("イン", Track::constant(0.50, 2));
                                eff.push_track("アウト", Track::constant(0.50, 2));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("ワイプ").clicked() {
                                let mut eff = Effect::new("ワイプ");
                                eff.push_track("イン", Track::constant(0.50, 2));
                                eff.push_track("アウト", Track::constant(0.50, 2));
                                eff.push_track("ぼかし", Track::constant(10.0, 0));
                                eff.push_track("反転(イン)", Track::constant(0.0, 0));
                                eff.push_track("反転(アウト)", Track::constant(0.0, 0));
                                eff.push_text("type", "0");
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("マスク").clicked() {
                                let mut eff = Effect::new("マスク");
                                eff.push_track("X", Track::constant(0.0, 1));
                                eff.push_track("Y", Track::constant(0.0, 1));
                                eff.push_track("回転", Track::constant(0.0, 2));
                                eff.push_track("サイズ", Track::constant(100.0, 0));
                                eff.push_track("縦横比", Track::constant(0.0, 1));
                                eff.push_track("ぼかし", Track::constant(0.0, 0));
                                eff.push_track("マスクの反転", Track::constant(0.0, 0));
                                eff.push_track("元のサイズに合わせる", Track::constant(0.0, 0));
                                eff.push_text("type", "1");
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("クリッピング").clicked() {
                                let mut eff = Effect::new("クリッピング");
                                eff.push_track("上", Track::constant(0.0, 0));
                                eff.push_track("下", Track::constant(0.0, 0));
                                eff.push_track("左", Track::constant(0.0, 0));
                                eff.push_track("右", Track::constant(0.0, 0));
                                eff.push_track("中心の位置を変更", Track::constant(0.0, 0));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("斜めクリッピング").clicked() {
                                let mut eff = Effect::new("斜めクリッピング");
                                eff.push_track("中心X", Track::constant(0.0, 0));
                                eff.push_track("中心Y", Track::constant(0.0, 0));
                                eff.push_track("角度", Track::constant(0.0, 1));
                                eff.push_track("ぼかし", Track::constant(1.0, 0));
                                eff.push_track("幅", Track::constant(0.0, 0));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                        });

                        // 5. 変形・配置・歪み・基本効果
                        ui.menu_button("📐 変形・配置・歪み・基本効果", |ui| {
                            if ui.button("振動").clicked() {
                                let mut eff = Effect::new("振動");
                                eff.push_track("X", Track::constant(15.0, 1));
                                eff.push_track("Y", Track::constant(15.0, 1));
                                eff.push_track("Z", Track::constant(0.0, 1));
                                eff.push_track("周期", Track::constant(1.0, 0));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("極座標変換").clicked() {
                                let mut eff = Effect::new("極座標変換");
                                eff.push_track("中心幅", Track::constant(0.0, 0));
                                eff.push_track("拡大率", Track::constant(100.0, 1));
                                eff.push_track("回転", Track::constant(0.0, 1));
                                eff.push_track("渦巻", Track::constant(0.0, 2));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("画像ループ").clicked() {
                                let mut eff = Effect::new("画像ループ");
                                eff.push_track("横回数", Track::constant(3.0, 0));
                                eff.push_track("縦回数", Track::constant(3.0, 0));
                                eff.push_track("速度X", Track::constant(0.0, 1));
                                eff.push_track("速度Y", Track::constant(0.0, 1));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("オブジェクト分割").clicked() {
                                let mut eff = Effect::new("オブジェクト分割");
                                eff.push_track("横分割数", Track::constant(2.0, 0));
                                eff.push_track("縦分割数", Track::constant(2.0, 0));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("ミラー").clicked() {
                                let mut eff = Effect::new("ミラー");
                                eff.push_track("透明度", Track::constant(30.0, 1));
                                eff.push_track("減衰", Track::constant(50.0, 1));
                                eff.push_track("境目調整", Track::constant(0.0, 0));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("ラスター").clicked() {
                                let mut eff = Effect::new("ラスター");
                                eff.push_track("横幅", Track::constant(50.0, 0));
                                eff.push_track("高さ", Track::constant(50.0, 0));
                                eff.push_track("周期", Track::constant(1.0, 2));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("波紋").clicked() {
                                let mut eff = Effect::new("波紋");
                                eff.push_track("中心X", Track::constant(0.0, 0));
                                eff.push_track("中心Y", Track::constant(0.0, 0));
                                eff.push_track("幅", Track::constant(30.0, 0));
                                eff.push_track("高さ", Track::constant(20.0, 0));
                                eff.push_track("速度", Track::constant(100.0, 0));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("ディスプレイスメントマップ").clicked() {
                                let mut eff = Effect::new("ディスプレイスメントマップ");
                                eff.push_track("変形X", Track::constant(20.0, 1));
                                eff.push_track("変形Y", Track::constant(20.0, 1));
                                eff.push_track("サイズ", Track::constant(200.0, 0));
                                eff.push_track("ぼかし", Track::constant(5.0, 0));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("ノイズ").clicked() {
                                let mut eff = Effect::new("ノイズ");
                                eff.push_track("強さ", Track::constant(50.0, 1));
                                eff.push_track("速度X", Track::constant(0.0, 1));
                                eff.push_track("速度Y", Track::constant(0.0, 1));
                                eff.push_track("変化速度", Track::constant(10.0, 1));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("モザイク").clicked() {
                                let mut eff = Effect::new("モザイク");
                                eff.push_track("サイズ", Track::constant(16.0, 0));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("座標").clicked() {
                                let mut eff = Effect::new("座標");
                                eff.push_track("X", Track::constant(0.0, 1));
                                eff.push_track("Y", Track::constant(0.0, 1));
                                eff.push_track("Z", Track::constant(0.0, 1));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("拡大率").clicked() {
                                let mut eff = Effect::new("拡大率");
                                eff.push_track("拡大率", Track::constant(100.0, 2));
                                eff.push_track("X", Track::constant(100.0, 2));
                                eff.push_track("Y", Track::constant(100.0, 2));
                                eff.push_track("Z", Track::constant(100.0, 2));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("透明度").clicked() {
                                let mut eff = Effect::new("透明度");
                                eff.push_track("透明度", Track::constant(0.0, 1));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("回転").clicked() {
                                let mut eff = Effect::new("回転");
                                eff.push_track("X", Track::constant(0.0, 2));
                                eff.push_track("Y", Track::constant(0.0, 2));
                                eff.push_track("Z", Track::constant(0.0, 2));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("リサイズ").clicked() {
                                let mut eff = Effect::new("リサイズ");
                                eff.push_track("拡大率", Track::constant(100.0, 2));
                                eff.push_track("X", Track::constant(100.0, 2));
                                eff.push_track("Y", Track::constant(100.0, 2));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("ローテーション").clicked() {
                                let mut eff = Effect::new("ローテーション");
                                eff.push_track("90度回転", Track::constant(1.0, 0));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                            if ui.button("領域拡張").clicked() {
                                let mut eff = Effect::new("領域拡張");
                                eff.push_track("上", Track::constant(20.0, 0));
                                eff.push_track("下", Track::constant(20.0, 0));
                                eff.push_track("左", Track::constant(20.0, 0));
                                eff.push_track("右", Track::constant(20.0, 0));
                                pending_effect = Some(eff);
                                ui.close_menu();
                            }
                        });

                        // 6. アニメーション効果 / スクリプト / .auf プラグイン
                        if !available_scripts.is_empty() {
                            ui.separator();
                            ui.menu_button("🎬 アニメーション効果 (標準 & スクリプト)", |ui| {
                                for (kind, entry) in &available_scripts {
                                    if *kind == au2_plugin::PluginKind::AnmScript {
                                        if ui.button(&entry.display_name).clicked() {
                                            pending_effect = Some(entry.to_effect(*kind));
                                            ui.close_menu();
                                        }
                                    }
                                }
                            });
                            ui.menu_button("🎥 カメラ効果 / 外部プラグイン (.auf)", |ui| {
                                for (kind, entry) in &available_scripts {
                                    if *kind != au2_plugin::PluginKind::AnmScript {
                                        if ui.button(&entry.display_name).clicked() {
                                            pending_effect = Some(entry.to_effect(*kind));
                                            ui.close_menu();
                                        }
                                    }
                                }
                            });
                        }
                    });
                });
            });

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("Layer {}", layer + 1)).size(11.0).color(Color32::from_rgb(147, 197, 253)));
                ui.separator();
                ui.label(RichText::new(format!("{} 〜 {} ({}F)", obj.frame_start + 1, obj.frame_end + 1, obj.length())).size(11.0).color(Color32::from_gray(185)));
                if !obj.midpoints.is_empty() {
                    ui.separator();
                    ui.label(RichText::new(format!("中間点: {} / 区間: {}", obj.midpoints.len(), seg + 1)).size(11.0).color(Color32::from_rgb(250, 204, 21)));
                }
                if let Some(g) = obj.group {
                    ui.separator();
                    ui.label(RichText::new(format!("グループ {g}")).size(11.0).color(Color32::from_gray(170)));
                }
            });
            if !obj.contains(frame) {
                ui.add_space(2.0);
                ui.label(RichText::new("※ 現在フレームはオブジェクトの範囲外です").size(10.5).color(Color32::from_rgb(250, 204, 21)));
            }
        });

    ui.add_space(4.0);

    let mut changed = false;
    if let Some(eff) = pending_effect {
        // 標準描画 / 拡張描画 / 標準再生 の直前に挿入する
        let insert_pos = obj
            .effects
            .iter()
            .position(|e| matches!(e.kind, EffectKind::StandardDraw | EffectKind::ExtendedDraw | EffectKind::StandardPlayback))
            .unwrap_or(obj.effects.len());
        obj.effects.insert(insert_pos, eff);
        changed = true;
    }

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (ei, eff) in obj.effects.iter_mut().enumerate() {
            let header_title = match eff.text("name").filter(|n| !n.is_empty()) {
                Some(sub) => format!("{} ({sub})", eff.name),
                None => eff.name.clone(),
            };
            CollapsingHeader::new(RichText::new(header_title).strong())
                .id_salt(("au2-effect", id, ei))
                .default_open(true)
                .show(ui, |ui| {
                    changed |= ui.checkbox(&mut eff.enabled, "有効").changed();
                    Grid::new(("au2-params", id, ei)).num_columns(2).striped(true).spacing([10.0, 4.0]).show(ui, |ui| {
                        for Param { key, value } in eff.params.iter_mut() {
                            ui.label(key.as_str());
                            changed |= match value {
                                ParamValue::Track(t) => track_editor(ui, t, &keys, seg, rel),
                                ParamValue::Text(s) if key == "text" => ui
                                    .add(TextEdit::multiline(s).desired_rows(3).desired_width(f32::INFINITY))
                                    .changed(),
                                ParamValue::Text(s) if key == "file" => {
                                    let mut file_changed = false;
                                    let resolved_info = current_file_info.as_ref().filter(|(f, _, _)| f == s);
                                    let is_missing = !s.trim().is_empty()
                                        && resolved_info.map_or(false, |(_, res, _)| res.is_none());
                                    let is_audio = resolved_info.map_or(false, |(_, _, is_aud)| *is_aud);

                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            file_changed |= ui.add(TextEdit::singleline(s).desired_width(150.0)).changed();
                                            if ui.button("参照...").clicked() {
                                                if let Some(p) = rfd::FileDialog::new()
                                                    .add_filter(
                                                        "メディアファイル",
                                                        &[
                                                            "mp4", "avi", "mov", "mkv", "webm", "wmv", "png", "jpg",
                                                            "jpeg", "bmp", "webp", "gif", "wav", "mp3", "ogg", "flac", "m4a",
                                                        ],
                                                    )
                                                    .add_filter("すべてのファイル", &["*"])
                                                    .pick_file()
                                                {
                                                    *s = p.to_string_lossy().into_owned();
                                                    file_changed = true;
                                                }
                                            }
                                        });

                                        if is_missing {
                                            ui.horizontal(|ui| {
                                                ui.colored_label(Color32::from_rgb(255, 90, 90), "⚠ ファイルが見つかりません（リンク切れ）");
                                                if ui.button("🎵 手元のファイルを割り当てる...").clicked() {
                                                    pending_relink = Some((s.clone(), is_audio));
                                                }
                                            });
                                        } else if let Some((_, Some(resolved_path), _)) = resolved_info {
                                            ui.colored_label(
                                                Color32::from_rgb(110, 220, 110),
                                                format!("✓ 検出: {}", resolved_path.file_name().and_then(|n| n.to_str()).unwrap_or("")),
                                            );
                                        }
                                    });
                                    if file_changed {
                                        file_changed_flag = true;
                                    }
                                    file_changed
                                }
                                ParamValue::Text(s) => ui.add(TextEdit::singleline(s).desired_width(180.0)).changed(),
                            };
                            ui.end_row();
                        }
                    });
                });
        }
    });

    if let Some((old_p, is_audio)) = pending_relink {
        st.relink_media_dialog(Some(id), &old_p, is_audio);
        changed = true;
    }

    if file_changed_flag {
        st.invalidate_media_cache();
    }

    if changed {
        st.dirty = true;
    }
}

fn drag(value: &mut f64, precision: u8) -> DragValue<'_> {
    let speed = if precision == 0 { 1.0 } else { 0.1_f64.powi(precision as i32 - 1) };
    DragValue::new(value).speed(speed).fixed_decimals(precision as usize)
}

/// トラックバー 1 本分の編集 UI。値が変更されたら `true`。
fn track_editor(ui: &mut egui::Ui, t: &mut Track, keys: &[Frame], seg: usize, rel: Frame) -> bool {
    let p = t.precision;
    if !t.is_animated() {
        let Some(k) = t.keyframes.first_mut() else { return false };
        return ui.add(drag(&mut k.value, p)).changed();
    }

    let find = |f: Frame| t.keyframes.iter().position(|k| k.frame == f);
    let li = keys.get(seg).and_then(|&f| find(f)).unwrap_or(0);
    let ri = keys.get(seg + 1).and_then(|&f| find(f)).unwrap_or(t.keyframes.len() - 1);
    let (mut a, mut b) = (t.keyframes[li].value, t.keyframes[ri].value);
    let current = t.evaluate(rel);
    let interp = t.interpolation.clone().unwrap();

    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= ui.add(drag(&mut a, p)).changed();
        ui.label("→");
        changed |= ui.add(drag(&mut b, p)).changed();
        let mut label = interp.kind.label();
        if interp.easing.accel() {
            label.push_str(" +加速");
        }
        if interp.easing.decel() {
            label.push_str(" +減速");
        }
        ui.small(label).on_hover_text(format!("現在値: {current:.prec$}", prec = p as usize));
    });
    if changed {
        t.keyframes[li].value = a;
        t.keyframes[ri].value = b;
    }
    changed
}

fn scene_settings_ui(ui: &mut egui::Ui, st: &mut EditorState) {
    egui::Frame::none()
        .fill(Color32::from_rgb(24, 26, 33))
        .stroke(Stroke::new(1.0_f32, Color32::from_gray(40)))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new(format!("🎬 シーン設定: {}", st.scene.name)).strong().size(15.0));
            });
            ui.add_space(2.0);
            ui.label(RichText::new("タイムライン上のオブジェクトをクリックすると、そのオブジェクトの個別パラメータを編集できます。").weak().size(11.0));
        });

    ui.add_space(6.0);

    let obj_count = st.scene.object_count();
    let s = &mut st.scene.settings;
    egui::Frame::none()
        .fill(Color32::from_rgb(20, 22, 27))
        .stroke(Stroke::new(1.0_f32, Color32::from_gray(36)))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            Grid::new("au2-scene-settings").num_columns(2).striped(true).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label(RichText::new("解像度 (幅×高)").strong());
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut s.width).range(1..=8192));
                    ui.label("×");
                    ui.add(DragValue::new(&mut s.height).range(1..=8192));
                    ui.label(RichText::new("px").weak());
                    ui.menu_button("プリセット", |ui| {
                        if ui.button("1920×1080 (FHD 16:9)").clicked() {
                            s.width = 1920;
                            s.height = 1080;
                            ui.close_menu();
                        }
                        if ui.button("1280×720 (HD 16:9)").clicked() {
                            s.width = 1280;
                            s.height = 720;
                            ui.close_menu();
                        }
                        if ui.button("3840×2160 (4K 16:9)").clicked() {
                            s.width = 3840;
                            s.height = 2160;
                            ui.close_menu();
                        }
                        if ui.button("1080×1920 (縦動画 9:16)").clicked() {
                            s.width = 1080;
                            s.height = 1920;
                            ui.close_menu();
                        }
                    });
                });
                ui.end_row();

                ui.label(RichText::new("フレームレート").strong());
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut s.frame_rate.num).range(1..=240_000));
                    ui.label("/");
                    ui.add(DragValue::new(&mut s.frame_rate.den).range(1..=1001));
                    ui.label(RichText::new(format!("= {:.3} fps", s.frame_rate.as_f64())).color(Color32::from_rgb(147, 197, 253)));
                });
                ui.end_row();

                ui.label(RichText::new("総フレーム数").strong());
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(&mut s.length).range(1..=10_000_000));
                    let dur_sec = s.frame_rate.frame_to_seconds(s.length);
                    ui.label(RichText::new(format!("(約 {:.1} 秒)", dur_sec)).weak());
                });
                ui.end_row();

                ui.label(RichText::new("音声フォーマット").strong());
                ui.label(format!("{} Hz / {} ch", s.audio_rate, s.audio_channels));
                ui.end_row();

                ui.label(RichText::new("配置オブジェクト数").strong());
                ui.label(format!("{} 個", obj_count));
                ui.end_row();

                ui.label(RichText::new("プロジェクトファイル").strong());
                ui.label(st.file_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "未保存 (新規プロジェクト)".into()));
                ui.end_row();

                ui.label(RichText::new("テキスト文字コード").strong());
                ui.label(format!("{:?}", st.encoding));
                ui.end_row();
            });
        });

    if !st.warnings.is_empty() {
        ui.separator();
        CollapsingHeader::new(format!("読み込み時の警告 ({})", st.warnings.len())).show(ui, |ui| {
            for w in &st.warnings {
                ui.label(w);
            }
        });
    }
}
