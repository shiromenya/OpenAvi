//! ソースパネル（メディアプール・標準オブジェクトパレット・プラグイン一覧）。
//!
//! - 動画・画像・音声素材のサムネイル確認とタイムラインへのワンクリック配置
//! - 標準オブジェクト（テキスト・図形・制御・フィルタ）のワンクリック追加
//! - プラグイン・スクリプトのワンクリックインストール＆選択オブジェクトへの効果適用

use std::path::PathBuf;

use au2_core::{EffectKind, TimelineObject};
use au2_media::MediaKind;
use au2_plugin::{PluginKind, SupportStatus};
use eframe::egui::{self, vec2, CollapsingHeader, Color32, RichText, ScrollArea};

use crate::state::EditorState;

pub fn ui(ui: &mut egui::Ui, st: &mut EditorState) {
    // ---- 1. 上部クイックバー（ファイルから簡単にインストール＆開く） ----
    egui::Frame::none()
        .fill(Color32::from_rgb(24, 26, 33))
        .stroke(egui::Stroke::new(1.0_f32, Color32::from_gray(38)))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(8.0, 6.0))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().button_padding = vec2(8.0, 5.0);
                if ui
                    .button(RichText::new("📂 開く / 一括取込").strong())
                    .on_hover_text("プロジェクト(.aup/.exo)、動画・画像・音声、プラグイン(.anm/.auf等)を自動判別して開く・インストールします")
                    .clicked()
                {
                    st.smart_open_dialog();
                }
                if ui
                    .button(RichText::new("🎬 素材追加").color(Color32::from_rgb(147, 197, 253)))
                    .on_hover_text("動画・画像・音声ファイルをソース一覧に追加します")
                    .clicked()
                {
                    st.import_sources_dialog();
                }
                if ui
                    .button(RichText::new("🧩 プラグイン追加").color(Color32::from_rgb(167, 243, 208)))
                    .on_hover_text(".anm / .obj / .auf 等のスクリプト・プラグインをインストールします")
                    .clicked()
                {
                    st.install_plugin_dialog();
                }
            });
        });
    ui.add_space(4.0);

    // ---- 2. ソースモニター（選択中のメディア素材のプレビュー） ----
    if let Some(sel_idx) = st.selected_source.filter(|&i| i < st.sources.len()) {
        let item = st.sources[sel_idx].clone();
        let egui_ctx = ui.ctx().clone();
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.strong(&item.name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("➕ タイムラインに配置").clicked() {
                        let layer = st.timeline.context_target.layer;
                        let frame = st.frame;
                        st.add_media_file(&item.path, layer, frame);
                    }
                });
            });

            if matches!(item.kind, MediaKind::Image | MediaKind::Video) {
                let path_str = item.path.to_string_lossy().into_owned();
                if let Some((tex_id, [w, h])) = st.get_media_texture(&egui_ctx, &path_str, 0, 0.0) {
                    let avail_w = ui.available_width().max(60.0);
                    let max_h = 110.0;
                    let scale = (avail_w / w as f32).min(max_h / h as f32);
                    let disp = vec2(w as f32 * scale, h as f32 * scale);
                    ui.vertical_centered(|ui| {
                        ui.image(egui::load::SizedTexture::new(tex_id, disp));
                    });
                    ui.small(format!("解像度: {w}x{h} / パス: {}", item.path.display()));
                }
            } else {
                ui.small(format!("音声ファイル: {}", item.path.display()));
            }
        });
        ui.add_space(4.0);
    }

    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // ---- 3. メディア素材リスト ----
        CollapsingHeader::new(RichText::new(format!("📁 メディア素材 ({})", st.sources.len())).strong())
            .default_open(true)
            .show(ui, |ui| {
                if st.sources.is_empty() {
                    ui.weak("素材がありません。「🎬 素材追加...」またはドラッグ＆ドロップで追加できます。");
                } else {
                    let mut to_place: Option<PathBuf> = None;
                    for (idx, src) in st.sources.iter().enumerate() {
                        let icon = match src.kind {
                            MediaKind::Video => "🎞",
                            MediaKind::Image => "🖼",
                            MediaKind::Audio => "🔊",
                        };
                        let selected = st.selected_source == Some(idx);
                        ui.horizontal(|ui| {
                            let resp = ui.selectable_label(selected, format!("{icon} {}", src.name));
                            if resp.clicked() {
                                st.selected_source = Some(idx);
                            }
                            if resp.double_clicked() {
                                to_place = Some(src.path.clone());
                            }
                            resp.on_hover_text(format!(
                                "{}\n(クリックでプレビュー、ダブルクリックまたは右のボタンでタイムラインへ配置)",
                                src.path.display()
                            ));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("＋配置").clicked() {
                                    to_place = Some(src.path.clone());
                                }
                            });
                        });
                    }
                    if let Some(path) = to_place {
                        let layer = st.timeline.context_target.layer;
                        let frame = st.frame;
                        st.add_media_file(&path, layer, frame);
                    }
                }
            });

        ui.add_space(4.0);

        // ---- 4. 標準オブジェクト・プリセット ----
        CollapsingHeader::new(RichText::new("✨ 標準オブジェクトの追加").strong())
            .default_open(true)
            .show(ui, |ui| {
                let layer = st.timeline.context_target.layer;
                let start = st.frame;
                let end = start + 149;

                ui.horizontal_wrapped(|ui| {
                    if ui.button("📝 テキスト").clicked() {
                        st.add_object_at(layer, TimelineObject::new_text(start, end, "サンプルテキスト"));
                    }
                    if ui.button("🔴 図形 (円)").clicked() {
                        st.add_object_at(layer, TimelineObject::new_shape(start, end, 1, "e94560"));
                    }
                    if ui.button("🟩 図形 (四角形)").clicked() {
                        st.add_object_at(layer, TimelineObject::new_shape(start, end, 2, "0f9b8e"));
                    }
                    if ui.button("⭐ 図形 (星型)").clicked() {
                        st.add_object_at(layer, TimelineObject::new_shape(start, end, 6, "ffd166"));
                    }
                    if ui.button("🖥 フレームバッファ").clicked() {
                        st.add_object_at(layer, TimelineObject::new_framebuffer(start, end));
                    }
                    if ui.button("📊 音声波形表示").clicked() {
                        st.add_object_at(layer, TimelineObject::new_audio_waveform(start, end));
                    }
                    if ui.button("✨ パーティクル出力").clicked() {
                        st.add_object_at(layer, TimelineObject::new_particle_output(start, end));
                    }
                    if ui.button("🎬 シーン呼出").clicked() {
                        st.add_object_at(layer, TimelineObject::new_scene_object(start, end, 1));
                    }
                    if ui.button("🎨 色調補正").clicked() {
                        st.add_object_at(layer, TimelineObject::new_color_correction(start, end));
                    }
                    if ui.button("🎞 シーンチェンジ").clicked() {
                        st.add_object_at(layer, TimelineObject::new_scene_change(start, end, "発光", 0));
                    }
                    if ui.button("⏱ 時間制御").clicked() {
                        st.add_object_at(layer, TimelineObject::new_time_control(start, end));
                    }
                    if ui.button("📐 グループ制御").clicked() {
                        st.add_object_at(layer, TimelineObject::new_group_control(start, end));
                    }
                    if ui.button("🎥 カメラ制御").clicked() {
                        st.add_object_at(layer, TimelineObject::new_camera_control(start, end));
                    }
                });
            });

        ui.add_space(4.0);

        // ---- 5. プラグイン & スクリプト一覧 ----
        CollapsingHeader::new(
            RichText::new(format!("🧩 プラグイン・スクリプト ({})", st.plugins.plugins.len())).strong(),
        )
        .default_open(true)
        .show(ui, |ui| {
            let mut apply_effect = None;
            let mut place_object = None;
            for p in &st.plugins.plugins {
                let badge_color = match p.status {
                    SupportStatus::Supported => Color32::from_rgb(100, 220, 120),
                    SupportStatus::PartialScript => Color32::from_rgb(240, 200, 90),
                    _ => Color32::from_rgb(170, 170, 185),
                };
                let icon = match p.kind {
                    PluginKind::AnmScript => "✨",
                    PluginKind::ObjScript => "🔷",
                    PluginKind::CamScript => "🎥",
                    PluginKind::ScnScript => "🎬",
                    PluginKind::TraScript => "📈",
                    PluginKind::FilterDll => "🧪",
                    PluginKind::InputDll => "📥",
                    PluginKind::OutputDll => "📤",
                    PluginKind::ColorDll => "🎨",
                };
                ui.horizontal(|ui| {
                    ui.label(format!("{icon} {}", p.file_name));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.colored_label(badge_color, format!("[{}]", p.arch.label()))
                            .on_hover_text(p.arch.description());
                    });
                });
                for entry in &p.entries {
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.label(RichText::new(format!("・{}", entry.display_name)).size(12.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if p.kind.can_place_as_object() {
                                if ui
                                    .small_button("＋配置")
                                    .on_hover_text("このスクリプト/フィルタを単体オブジェクトとしてタイムラインに配置")
                                    .clicked()
                                {
                                    place_object = Some((p.kind, entry.clone()));
                                }
                            }
                            if p.kind.can_apply_to_object() && st.selected.is_some() {
                                if ui
                                    .small_button("＋適用")
                                    .on_hover_text("選択中のオブジェクトにこの効果を追加")
                                    .clicked()
                                {
                                    apply_effect = Some((p.kind, entry.clone()));
                                }
                            }
                        });
                    });
                }
                ui.add_space(2.0);
            }
            if let Some((kind, entry)) = apply_effect {
                if let Some(sel_id) = st.selected {
                    if let Some((_, obj)) = st.scene.find_mut(sel_id) {
                        let insert_pos = obj
                            .effects
                            .iter()
                            .position(|e| {
                                matches!(
                                    e.kind,
                                    EffectKind::StandardDraw
                                        | EffectKind::ExtendedDraw
                                        | EffectKind::StandardPlayback
                                )
                            })
                            .unwrap_or(obj.effects.len());
                        obj.effects.insert(insert_pos, entry.to_effect(kind));
                        st.dirty = true;
                        st.status = format!("「{}」を選択オブジェクトに適用しました", entry.display_name);
                    }
                }
            }
            if let Some((kind, entry)) = place_object {
                let layer = st.timeline.context_target.layer;
                let start = st.frame;
                let end = start + 149;
                let obj = entry.to_timeline_object(kind, start, end);
                st.add_object_at(layer, obj);
            }
        });
    });
}
