//! 設定ダイアログ（選択中オブジェクトのパラメータ編集）。
//!
//! AviUtl と同様、中間点を持つトラックは「現在フレームを含む区間」の
//! 左（区間開始）・右（区間終了）の値を編集します。
//! また、右上の「＋ フィルタ効果を追加」から内蔵フィルタや plugins/ 内の .anm スクリプトを追加できます。

pub mod psd;
pub mod scene;

pub(crate) use psd::*;
pub(crate) use scene::*;
use au2_core::{Easing, Effect, EffectKind, Frame, Interpolation, MoveKind, Param, ParamValue, Track};
use au2_plugin::PluginKind;
use eframe::egui::{self, CollapsingHeader, Color32, DragValue, Grid, Pos2, RichText, Stroke, TextEdit, Vec2};

use crate::state::EditorState;
use crate::theme;

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

    // PSD / PSB ファイルの場合はレイヤーツリー構造を事前取得
    let current_psd_doc: Option<std::sync::Arc<au2_media::PsdDocument>> = current_file_info
        .as_ref()
        .and_then(|(_, res, _)| res.as_deref())
        .filter(|p| au2_media::is_psd_path(p))
        .and_then(|p| st.media.get_psd_document(p));

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
        .fill(theme::surface_alt())
        .stroke(Stroke::new(1.0_f32, theme::border()))
        .rounding(theme::rounding::lg())
        .inner_margin(egui::Margin::symmetric(theme::spacing::XL, theme::spacing::LG))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new(obj.display_name()).strong().size(theme::typography::HEADING));
                if ui
                    .checkbox(&mut obj.clipping, "上のオブジェクトでクリッピング")
                    .on_hover_text("直上レイヤーのオブジェクトの形状（アルファチャンネル）でこのオブジェクトを切り抜きます")
                    .changed()
                {
                    st.dirty = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button("＋ フィルタ効果を追加", |ui| {
                        // 1. 発光・ブラー・装飾系
                        ui.menu_button("発光・ライト・ブラー", |ui| {
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
                        ui.menu_button("縁取り・シャドー・エッジ", |ui| {
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
                        ui.menu_button("色調・グラデーション・キーイング", |ui| {
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
                        ui.menu_button("フェード・ワイプ・マスク・クリッピング", |ui| {
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
                        ui.menu_button("変形・配置・歪み・基本効果", |ui| {
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
                            ui.menu_button("アニメーション効果 (標準 & スクリプト)", |ui| {
                                for (kind, entry) in &available_scripts {
                                    if *kind == au2_plugin::PluginKind::AnmScript {
                                        let name = au2_plugin::clean_script_name(&entry.display_name);
                                        if ui.button(name).clicked() {
                                            pending_effect = Some(entry.to_effect(*kind));
                                            ui.close_menu();
                                        }
                                    }
                                }
                            });
                            ui.menu_button("カメラ効果 / プラグイン効果 (.auf)", |ui| {
                                for (kind, entry) in &available_scripts {
                                    if *kind != au2_plugin::PluginKind::AnmScript {
                                        let name = au2_plugin::clean_script_name(&entry.display_name);
                                        if ui.button(name).clicked() {
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

            ui.add_space(theme::spacing::SM);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("Layer {}", layer + 1)).size(theme::typography::BODY_SM).color(theme::accent_text()));
                ui.separator();
                ui.label(RichText::new(format!("{} 〜 {} ({}F)", obj.frame_start + 1, obj.frame_end + 1, obj.length())).size(theme::typography::BODY_SM).color(theme::text_muted()));
                if !obj.midpoints.is_empty() {
                    ui.separator();
                    ui.label(RichText::new(format!("中間点: {} / 区間: {}", obj.midpoints.len(), seg + 1)).size(theme::typography::BODY_SM).color(theme::fg(egui::Color32::from_rgb(250, 204, 21))));
                }
                if let Some(g) = obj.group {
                    ui.separator();
                    ui.label(RichText::new(format!("グループ {g}")).size(theme::typography::BODY_SM).color(theme::text_muted()));
                }
            });
            if !obj.contains(frame) {
                ui.add_space(2.0);
                ui.label(RichText::new("※ 現在フレームはオブジェクトの範囲外です").size(10.5).color(theme::warn_text()));
            }
        });

    ui.add_space(theme::spacing::SM);

    let mut changed = false;
    let mut pending_psd_action: Option<PsdPendingAction> = None;
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

    let obj_len = obj.length();

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
                            if key.starts_with("psd_") {
                                continue;
                            }
                            let display_key = match key.as_str() {
                                "type" => match eff.name.as_str() {
                                    "図形" => "図形の種類",
                                    "マスク" => "マスク形状",
                                    "ワイプ" => "ワイプ形状",
                                    "テキスト" => "装飾種別",
                                    _ => "種類・タイプ",
                                },
                                "color" => match eff.name.as_str() {
                                    "図形" => "図形の色",
                                    "テキスト" => "文字色",
                                    _ => "色",
                                },
                                "color2" => "影・縁色",
                                "font" => "フォント",
                                "text" => "テキスト",
                                "file" => "参照ファイル",
                                "line" => "ライン幅",
                                other => other,
                            };
                            ui.label(display_key);
                            changed |= match value {
                                ParamValue::Track(t) => track_editor(ui, t, &keys, seg, rel, obj_len),
                                ParamValue::Text(s) if key == "text" => ui
                                    .add(TextEdit::multiline(s).desired_rows(3).desired_width(f32::INFINITY))
                                    .changed(),
                                ParamValue::Text(s) if key == "type" => {
                                    let mut cur_type = s.trim().parse::<u8>().unwrap_or(0);
                                    let type_labels: &[(u8, &str)] = match eff.name.as_str() {
                                        "図形" => &[
                                            (0, "背景 (全面塗り)"),
                                            (1, "円 (Circle)"),
                                            (2, "四角形 (Square)"),
                                            (3, "三角形 (Triangle)"),
                                            (4, "五角形 (Pentagon)"),
                                            (5, "六角形 (Hexagon)"),
                                            (6, "星型 (Star)"),
                                        ],
                                        "マスク" => &[
                                            (0, "背景"),
                                            (1, "円 (Circle)"),
                                            (2, "四角形 (Square)"),
                                            (3, "三角形 (Triangle)"),
                                            (4, "星型 (Star)"),
                                        ],
                                        "ワイプ" => &[
                                            (0, "円 (Circle)"),
                                            (1, "四角形 (Square)"),
                                            (2, "時計回り (Clockwise)"),
                                            (3, "反時計回り (Counter-CW)"),
                                        ],
                                        _ => &[
                                            (0, "標準 (装飾なし)"),
                                            (1, "影付き (濃)"),
                                            (2, "影付き (薄)"),
                                            (3, "縁取り (標準)"),
                                            (4, "縁取り (細)"),
                                        ],
                                    };
                                    let current_label = type_labels.iter().find(|(t, _)| *t == cur_type).map(|(_, l)| *l).unwrap_or("カスタム");
                                    let mut type_changed = false;
                                    egui::ComboBox::from_id_salt(("au2-type-combo", id, ei))
                                        .selected_text(current_label)
                                        .width(140.0)
                                        .show_ui(ui, |ui| {
                                            for &(val, label) in type_labels {
                                                if ui.selectable_value(&mut cur_type, val, label).clicked() {
                                                    *s = val.to_string();
                                                    type_changed = true;
                                                }
                                            }
                                        });
                                    type_changed
                                }
                                ParamValue::Text(s) if key == "color" || key == "color2" => {
                                    let mut color_changed = false;
                                    ui.horizontal(|ui| {
                                        let mut r = 255u8;
                                        let mut g = 255u8;
                                        let mut b = 255u8;
                                        let clean = s.trim().trim_start_matches('#');
                                        if clean.len() == 6 {
                                            if let Ok(val) = u32::from_str_radix(clean, 16) {
                                                r = ((val >> 16) & 0xFF) as u8;
                                                g = ((val >> 8) & 0xFF) as u8;
                                                b = (val & 0xFF) as u8;
                                            }
                                        }
                                        let mut srgb = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
                                        if ui.color_edit_button_rgb(&mut srgb).changed() {
                                            *s = format!("{:02x}{:02x}{:02x}", (srgb[0] * 255.0) as u8, (srgb[1] * 255.0) as u8, (srgb[2] * 255.0) as u8);
                                            color_changed = true;
                                        }
                                        color_changed |= ui.add(TextEdit::singleline(s).desired_width(55.0)).changed();

                                        let presets = [
                                            ("白", "ffffff"),
                                            ("黒", "000000"),
                                            ("黄", "ffe600"),
                                            ("赤", "ff3333"),
                                            ("青", "3388ff"),
                                            ("緑", "33cc55"),
                                        ];
                                        ui.menu_button("プリセット", |ui| {
                                            for (pname, phex) in presets {
                                                if ui.button(pname).clicked() {
                                                    *s = phex.to_string();
                                                    color_changed = true;
                                                    ui.close_menu();
                                                }
                                            }
                                        });
                                    });
                                    color_changed
                                }
                                ParamValue::Text(s) if key == "font" => {
                                    let mut font_changed = false;
                                    let available_fonts = crate::fonts::list_available_fonts();
                                    ui.horizontal(|ui| {
                                        egui::ComboBox::from_id_salt(("au2-font-combo", id, ei))
                                            .selected_text(if s.is_empty() { "フォント選択" } else { s.as_str() })
                                            .width(130.0)
                                            .show_ui(ui, |ui| {
                                                for f in available_fonts {
                                                    if ui.selectable_label(s == f, f).clicked() {
                                                        *s = f.to_string();
                                                        font_changed = true;
                                                        if crate::fonts::ensure_font_family(ui.ctx(), s) {
                                                            ui.ctx().request_repaint();
                                                        }
                                                    }
                                                }
                                            });
                                        let edit_resp = ui.add(TextEdit::singleline(s).desired_width(90.0));
                                        if edit_resp.changed() {
                                            font_changed = true;
                                        }
                                        if (edit_resp.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)))
                                            && !s.trim().is_empty()
                                        {
                                            if crate::fonts::ensure_font_family(ui.ctx(), s) {
                                                ui.ctx().request_repaint();
                                            }
                                        }
                                    });
                                    font_changed
                                }
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
                                                            "jpeg", "bmp", "webp", "gif", "psd", "psb", "wav", "mp3", "ogg", "flac", "m4a",
                                                        ],
                                                    )
                                                    .add_filter("PSD 立ち絵ファイル (*.psd, *.psb)", &["psd", "psb"])
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
                                                ui.colored_label(theme::danger_text(), "⚠ ファイルが見つかりません（リンク切れ）");
                                                if ui.button("手元のファイルを割り当てる...").clicked() {
                                                    pending_relink = Some((s.clone(), is_audio));
                                                }
                                            });
                                        } else if let Some((_, Some(resolved_path), _)) = resolved_info {
                                            ui.colored_label(
                                                theme::ok_text(),
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

                    if eff.kind == EffectKind::ImageFile {
                        if let Some(ref psd_doc) = current_psd_doc {
                            ui.add_space(6.0);
                            changed |= psd_inspector_ui(ui, id, eff, psd_doc, &mut pending_psd_action);
                        }
                    }
                });
        }
    });

    if let Some((old_p, is_audio)) = pending_relink {
        st.relink_media_dialog(Some(id), &old_p, is_audio);
        changed = true;
    }

    if let Some(action) = pending_psd_action {
        match action {
            PsdPendingAction::DecomposeFolders { tight_crop } => {
                changed |= st.decompose_psd_object(id, true, tight_crop);
            }
            PsdPendingAction::DecomposeLayers { tight_crop } => {
                changed |= st.decompose_psd_object(id, false, tight_crop);
            }
            PsdPendingAction::ExportPng { by_folders, tight_crop } => {
                st.export_psd_layers_png_dialog(id, by_folders, tight_crop);
            }
        }
    }

    if file_changed_flag {
        st.invalidate_media_cache();
    }

    if changed {
        st.begin_continuous_edit("パラメータ変更");
        st.dirty = true;
    }
    if !ui.input(|i| i.pointer.any_down()) {
        st.commit_continuous_edit();
    }
}

/// イージング曲線のミニプレビューグラフを描画する
fn draw_easing_curve_preview(ui: &mut egui::Ui, easing_id: i32, is_selected: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(28.0, 16.0), egui::Sense::hover());
    let painter = ui.painter();

    let bg_color = if is_selected {
        Color32::from_rgb(35, 55, 80)
    } else {
        Color32::from_gray(24)
    };
    painter.rect_filled(rect, 2.0_f32, bg_color);
    painter.rect_stroke(
        rect,
        2.0_f32,
        Stroke::new(1.0_f32, if is_selected { theme::accent_hi() } else { Color32::from_gray(50) }),
    );

    let steps = 14;
    let mut points = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let v = au2_core::track::apply_aviutl_easing_id(easing_id, t).clamp(-0.2, 1.2);
        let px = rect.min.x + 2.0 + (t as f32) * (rect.width() - 4.0);
        let py = rect.max.y - 2.0 - (v as f32) * (rect.height() - 4.0);
        points.push(Pos2::new(px, py));
    }

    let line_color = if is_selected {
        Color32::from_rgb(100, 200, 255)
    } else {
        Color32::from_rgb(220, 190, 90)
    };
    painter.add(egui::epaint::PathShape::line(points, Stroke::new(1.5_f32, line_color)));
}

fn drag(value: &mut f64, precision: u8) -> DragValue<'_> {
    let speed = if precision == 0 { 1.0 } else { 0.1_f64.powi(precision as i32 - 1) };
    DragValue::new(value).speed(speed).fixed_decimals(precision as usize)
}

/// トラックバーの補間方法（移動方法・イージング・加減速）を選択・変更するドロップダウンメニュー。
fn track_interpolation_menu(ui: &mut egui::Ui, t: &mut Track, total_len: Frame) -> bool {
    let mut changed = false;
    let current_label = if let Some(interp) = &t.interpolation {
        let mut l = interp.kind.label();
        if let Some(param) = &interp.param {
            if interp.kind.label().contains("イージング") || interp.kind.label().contains("スクリプト") {
                l = format!("イージング ({param})");
            }
        }
        if interp.easing.accel() && interp.easing.decel() {
            l.push_str(" +加減速");
        } else if interp.easing.accel() {
            l.push_str(" +加速");
        } else if interp.easing.decel() {
            l.push_str(" +減速");
        }
        l
    } else {
        "固定値".into()
    };

    let btn_text = format!("{} ▾", current_label);
    ui.menu_button(btn_text, |ui| {
        ui.set_max_width(220.0);
        // 1. 移動方法の基本
        let is_none = t.interpolation.is_none();
        if ui.selectable_label(is_none, "移動なし (固定値)").clicked() {
            t.set_interpolation(None, total_len);
            changed = true;
            ui.close_menu();
        }
        ui.separator();

        let basic_modes = [
            ("直線移動", MoveKind::Linear),
            ("加減速移動", MoveKind::Other(7)),
            ("曲線移動", MoveKind::Curve),
            ("瞬間移動", MoveKind::Instant),
            ("中間点無視", MoveKind::IgnoreMidpoint),
            ("移動量指定", MoveKind::MoveAmount),
            ("反復移動", MoveKind::Other(8)),
            ("ランダム移動", MoveKind::Random),
        ];

        for (name, kind) in basic_modes {
            let is_sel = t.interpolation.as_ref().map_or(false, |i| i.kind == kind);
            if ui.selectable_label(is_sel, name).clicked() {
                let prev_easing = t.interpolation.as_ref().map(|i| i.easing).unwrap_or(Easing::None);
                t.set_interpolation(
                    Some(Interpolation {
                        kind,
                        easing: prev_easing,
                        param: None,
                        legacy_code: None,
                    }),
                    total_len,
                );
                changed = true;
                ui.close_menu();
            }
        }

        if t.is_animated() {
            ui.separator();
            ui.menu_button("加減速設定 (Ease)", |ui| {
                let cur_easing = t.interpolation.as_ref().map(|i| i.easing).unwrap_or(Easing::None);
                if ui.selectable_label(cur_easing == Easing::None, "等速 (None)").clicked() {
                    if let Some(i) = &mut t.interpolation {
                        i.easing = Easing::None;
                    }
                    changed = true;
                    ui.close_menu();
                }
                if ui.selectable_label(cur_easing == Easing::EaseIn, "＋加速 (Ease In)").clicked() {
                    if let Some(i) = &mut t.interpolation {
                        i.easing = Easing::EaseIn;
                    }
                    changed = true;
                    ui.close_menu();
                }
                if ui.selectable_label(cur_easing == Easing::EaseOut, "＋減速 (Ease Out)").clicked() {
                    if let Some(i) = &mut t.interpolation {
                        i.easing = Easing::EaseOut;
                    }
                    changed = true;
                    ui.close_menu();
                }
                if ui.selectable_label(cur_easing == Easing::EaseInOut, "＋加減速 (Ease In-Out)").clicked() {
                    if let Some(i) = &mut t.interpolation {
                        i.easing = Easing::EaseInOut;
                    }
                    changed = true;
                    ui.close_menu();
                }
            });

            ui.menu_button("イージング・プリセット (AviUtl 1〜41)", |ui| {
                ui.set_max_height(260.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let easings = [
                        (1, "1: Quad In"), (2, "2: Quad Out"), (3, "3: Quad In-Out"),
                        (4, "4: Cubic In"), (5, "5: Cubic Out"), (6, "6: Cubic In-Out"),
                        (7, "7: Quart In"), (8, "8: Quart Out"), (9, "9: Quart In-Out"),
                        (10, "10: Quint In"), (11, "11: Quint Out"), (12, "12: Quint In-Out"),
                        (13, "13: Sine In"), (14, "14: Sine Out"), (15, "15: Sine In-Out"),
                        (16, "16: Exp In"), (17, "17: Exp Out"), (18, "18: Exp In-Out"),
                        (19, "19: Circ In"), (20, "20: Circ Out"), (21, "21: Circ In-Out"),
                        (22, "22: Elastic In"), (23, "23: Elastic Out"), (24, "24: Elastic In-Out"),
                        (25, "25: Back In"), (26, "26: Back Out"), (27, "27: Back In-Out"),
                        (28, "28: Bounce In"), (29, "29: Bounce Out"), (30, "30: Bounce In-Out"),
                    ];
                    for (id, desc) in easings {
                        let is_sel = t.interpolation.as_ref().is_some_and(|i| {
                            i.kind == MoveKind::Script("イージング".into()) && i.param.as_deref() == Some(&id.to_string())
                        });
                        ui.horizontal(|ui| {
                            draw_easing_curve_preview(ui, id, is_sel);
                            if ui.selectable_label(is_sel, desc).clicked() {
                                t.set_interpolation(
                                    Some(Interpolation {
                                        kind: MoveKind::Script("イージング".into()),
                                        easing: Easing::None,
                                        param: Some(id.to_string()),
                                        legacy_code: None,
                                    }),
                                    total_len,
                                );
                                changed = true;
                                ui.close_menu();
                            }
                        });
                    }
                });
            });
        }
    });

    changed
}

/// トラックバー 1 本分の編集 UI。値や移動方法が変更されたら `true`。
fn track_editor(ui: &mut egui::Ui, t: &mut Track, keys: &[Frame], seg: usize, rel: Frame, total_len: Frame) -> bool {
    let p = t.precision;
    let mut changed = false;
    if !t.is_animated() {
        let mut val = t.keyframes.first().map(|k| k.value).unwrap_or(0.0);
        ui.horizontal(|ui| {
            changed |= ui.add(drag(&mut val, p)).changed();
            changed |= track_interpolation_menu(ui, t, total_len);
        });
        if changed && !t.keyframes.is_empty() {
            t.keyframes[0].value = val;
        }
        return changed;
    }

    let find = |f: Frame| t.keyframes.iter().position(|k| k.frame == f);
    let li = keys.get(seg).and_then(|&f| find(f)).unwrap_or(0);
    let ri = keys.get(seg + 1).and_then(|&f| find(f)).unwrap_or(t.keyframes.len() - 1);
    let (mut a, mut b) = (t.keyframes[li].value, t.keyframes[ri].value);
    let current = t.evaluate(rel);

    ui.horizontal(|ui| {
        changed |= ui.add(drag(&mut a, p)).changed();
        ui.label("→");
        changed |= ui.add(drag(&mut b, p)).changed();
        changed |= track_interpolation_menu(ui, t, total_len);
        ui.small(format!("({current:.prec$})", prec = p as usize))
            .on_hover_text(format!("現在フレームでの評価値: {current:.prec$}", prec = p as usize));
    });
    if changed && t.keyframes.len() > ri {
        t.keyframes[li].value = a;
        t.keyframes[ri].value = b;
    }
    changed
}

