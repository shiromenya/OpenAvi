//! ソースパネル（メディアプール・標準オブジェクトパレット・プラグイン一覧）。
//!
//! - 動画・画像・音声素材のサムネイル確認とタイムラインへのワンクリック配置
//! - 標準オブジェクト（テキスト・図形・制御・フィルタ）のワンクリック追加
//! - プラグイン・スクリプトのワンクリックインストール＆選択オブジェクトへの効果適用

use std::path::PathBuf;

use au2_core::{EffectKind, TimelineObject};
use au2_media::MediaKind;
use au2_plugin::{PluginKind, SupportStatus};
use eframe::egui::{self, vec2, CollapsingHeader, RichText, ScrollArea};

use super::{button_grid, small_button, tiny_button};
use crate::state::EditorState;
use crate::theme;

pub fn ui(ui: &mut egui::Ui, st: &mut EditorState) {
    // ---- 1. 上部クイックバー（ファイルから簡単にインストール＆開く） ----
    egui::Frame::none()
        .fill(theme::surface_alt())
        .stroke(egui::Stroke::new(1.0_f32, theme::border()))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            let txt_color = theme::text();
            let items: [(egui::WidgetText, &str); 3] = [
                (
                    RichText::new("📂 開く / 一括取込").strong().color(txt_color).into(),
                    "プロジェクト(.aup/.exo)、動画・画像・音声、プラグイン(.anm/.auf等)を自動判別して開く・インストールします",
                ),
                (
                    RichText::new("🎬 素材追加")
                        .strong()
                        .color(if theme::is_dark() { theme::accent_text() } else { txt_color })
                        .into(),
                    "動画・画像・音声ファイルをソース一覧に追加します",
                ),
                (
                    RichText::new("🧩 プラグイン追加")
                        .strong()
                        .color(if theme::is_dark() { theme::ok_text() } else { txt_color })
                        .into(),
                    ".anm / .obj / .auf 等のスクリプト・プラグインをインストールします",
                ),
            ];
            match button_grid(ui, &items, 130.0) {
                Some(0) => st.smart_open_dialog(),
                Some(1) => st.import_sources_dialog(),
                Some(2) => st.install_plugin_dialog(),
                _ => {}
            }
        });

    // ---- 2. ソースモニター（選択中のメディア素材のプレビュー） ----
    if let Some(sel_idx) = st.selected_source.filter(|&i| i < st.sources.len()) {
        let item = st.sources[sel_idx].clone();
        let egui_ctx = ui.ctx().clone();
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.strong(&item.name);
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
            let place_btn = egui::Button::new(RichText::new("➕ タイムラインに配置").strong())
                .min_size(vec2(ui.available_width(), super::BUTTON_H));
            if ui.add(place_btn).clicked() {
                let layer = st.timeline.context_target.layer;
                let frame = st.frame;
                st.add_media_file(&item.path, layer, frame);
            }
        });
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
                            MediaKind::Image => {
                                if au2_media::is_psd_path(&src.path) {
                                    "🎨"
                                } else {
                                    "🖼"
                                }
                            }
                            MediaKind::Audio => "🔊",
                        };
                        let selected = st.selected_source == Some(idx);
                        ui.horizontal(|ui| {
                            // 右端のボタンを先に確保し、残り幅でファイル名を（省略表示付きで）描画する
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_space(8.0);
                                if small_button(ui, "＋配置").clicked() {
                                    to_place = Some(src.path.clone());
                                }
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    let resp = ui.add_sized(
                                        vec2(ui.available_width(), super::SMALL_BUTTON_H),
                                        egui::SelectableLabel::new(selected, format!("{icon} {}", src.name)),
                                    );
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
                                });
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

                let items: [(egui::WidgetText, &str); 14] = [
                    ("📝 テキスト".into(), ""),
                    ("🎨 PSD立ち絵...".into(), "PSDファイル(.psd/.psb)を選択して立ち絵オブジェクトとして配置します"),
                    ("🔴 図形 (円)".into(), ""),
                    ("🟩 図形 (四角形)".into(), ""),
                    ("⭐ 図形 (星型)".into(), ""),
                    ("🖥 フレームバッファ".into(), ""),
                    ("📊 音声波形表示".into(), ""),
                    ("✨ パーティクル出力".into(), ""),
                    ("🎬 シーン呼出".into(), ""),
                    ("🎨 色調補正".into(), ""),
                    ("🎞 シーンチェンジ".into(), ""),
                    ("⏱ 時間制御".into(), ""),
                    ("📐 グループ制御".into(), ""),
                    ("🎥 カメラ制御".into(), ""),
                ];
                if let Some(i) = button_grid(ui, &items, 130.0) {
                    if i == 1 {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Photoshop PSD / PSB ファイル (*.psd, *.psb)", &["psd", "psb"])
                            .add_filter("すべてのファイル", &["*"])
                            .pick_file()
                        {
                            st.add_media_file(&path, layer, start);
                        }
                    } else {
                        let obj = match i {
                            0 => TimelineObject::new_text(start, end, "サンプルテキスト"),
                            2 => TimelineObject::new_shape(start, end, 1, "e94560"),
                            3 => TimelineObject::new_shape(start, end, 2, "0f9b8e"),
                            4 => TimelineObject::new_shape(start, end, 6, "ffd166"),
                            5 => TimelineObject::new_framebuffer(start, end),
                            6 => TimelineObject::new_audio_waveform(start, end),
                            7 => TimelineObject::new_particle_output(start, end),
                            8 => TimelineObject::new_scene_object(start, end, 1),
                            9 => TimelineObject::new_color_correction(start, end),
                            10 => TimelineObject::new_scene_change(start, end, "発光", 0),
                            11 => TimelineObject::new_time_control(start, end),
                            12 => TimelineObject::new_group_control(start, end),
                            _ => TimelineObject::new_camera_control(start, end),
                        };
                        st.add_object_at(layer, obj);
                    }
                }
            });

        ui.add_space(4.0);

        // ---- 5. プラグイン & スクリプト一覧 ----
        // ソースパネルはタイムラインへの配置・効果適用が目的のため、
        // 配置・適用不可な外部出力プラグインや、x264guiEx などの本家外の第三者プラグインは除外してすっきり表示
        let visible_plugin_indices: Vec<usize> = st
            .plugins
            .plugins
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                let lower = p.file_name.to_lowercase();
                if lower.contains("x264")
                    || lower.contains("guiex")
                    || lower.contains("x265")
                    || lower.contains("qsvencc")
                    || lower.contains("nvenc")
                    || lower.contains("vceencc")
                {
                    return false;
                }
                // ソースパネルでは配置または適用が可能なスクリプト・フィルタを表示
                p.kind.can_place_as_object() || p.kind.can_apply_to_object()
            })
            .map(|(i, _)| i)
            .collect();

        CollapsingHeader::new(
            RichText::new(format!("🧩 プラグイン・スクリプト ({})", visible_plugin_indices.len())).strong(),
        )
        .default_open(true)
        .show(ui, |ui| {
            let mut apply_effect = None;
            let mut place_object = None;
            for &idx in &visible_plugin_indices {
                let p = &st.plugins.plugins[idx];
                let badge_color = match p.status {
                    SupportStatus::Supported => theme::ok_text(),
                    SupportStatus::PartialScript => theme::fg(egui::Color32::from_rgb(240, 200, 90)),
                    _ => theme::text_muted(),
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
                    PluginKind::Mod2Dll => "📦",
                    PluginKind::Aux2Dll => "⚙",
                };
                let group_header = format!("{icon} {} ({})", p.file_name, p.entries.len());
                let header_id = ui.make_persistent_id(format!("src_plugin_grp_{}_{}", idx, p.file_name));
                CollapsingHeader::new(RichText::new(group_header).strong().size(12.5))
                    .id_salt(header_id)
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(format!("種別: {}", p.kind.label()))
                                    .size(11.0)
                                    .color(theme::text_muted()),
                            );
                            ui.colored_label(badge_color, format!("[{}]", p.arch.label()))
                                .on_hover_text(p.arch.description());
                        });
                        for entry in &p.entries {
                            let clean_name = au2_plugin::clean_script_name(&entry.display_name);
                            ui.horizontal(|ui| {
                                ui.add_space(8.0);
                                ui.label(RichText::new(format!("・{clean_name}")).size(12.0));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.add_space(4.0);
                                    if p.kind.can_place_as_object()
                                        && tiny_button(ui, "＋配置")
                                            .on_hover_text("このスクリプト/フィルタを単体オブジェクトとしてタイムラインに配置")
                                            .clicked()
                                    {
                                        place_object = Some((p.kind, entry.clone()));
                                    }
                                    if p.kind.can_apply_to_object()
                                        && st.selected.is_some()
                                        && tiny_button(ui, "＋適用")
                                            .on_hover_text("選択中のオブジェクトにこの効果を追加")
                                            .clicked()
                                    {
                                        apply_effect = Some((p.kind, entry.clone()));
                                    }
                                });
                            });
                        }
                    });
                ui.add_space(1.0);
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
                        let clean_name = au2_plugin::clean_script_name(&entry.display_name);
                        st.status = format!("「{clean_name}」を選択オブジェクトに適用しました");
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
