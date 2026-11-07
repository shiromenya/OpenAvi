//! PSD 立ち絵・レイヤー分離・目パチ・口パク設定UI（PSDToolKit 互換）。

use au2_core::{Effect, Param, ParamValue};
use eframe::egui::{self, CollapsingHeader, DragValue, Grid, RichText, Stroke};
use crate::theme;

fn get_eff_text_param<'a>(eff: &'a Effect, key: &str) -> &'a str {
    eff.text(key).unwrap_or("")
}

fn set_eff_text_param(eff: &mut Effect, key: &str, value: String) {
    if let Some(p) = eff.params.iter_mut().find(|p| p.key == key) {
        p.value = ParamValue::Text(value);
    } else if !value.is_empty() {
        eff.params.push(Param {
            key: key.to_string(),
            value: ParamValue::Text(value),
        });
    }
}

#[derive(Clone, Copy)]
pub(crate) enum PsdPendingAction {
    DecomposeFolders { tight_crop: bool },
    DecomposeLayers { tight_crop: bool },
    ExportPng { by_folders: bool, tight_crop: bool },
}

/// PSD 立ち絵・レイヤー切替・目パチ・口パク設定 UI（PSDToolKit 互換）
pub(crate) fn psd_inspector_ui(
    ui: &mut egui::Ui,
    id: au2_core::ObjectId,
    eff: &mut Effect,
    doc: &au2_media::PsdDocument,
    pending_action: &mut Option<PsdPendingAction>,
) -> bool {
    let mut changed = false;

    let layers_spec = get_eff_text_param(eff, "psd_layers").to_string();
    let blink_spec = get_eff_text_param(eff, "psd_blink").to_string();
    let lipsync_spec = get_eff_text_param(eff, "psd_lipsync").to_string();
    let psd_label = get_eff_text_param(eff, "psd_label").to_string();

    let mut vis_vec = doc.compute_visibility_from_overrides(&layers_spec);
    let mut blink_cfg = au2_media::PsdBlinkConfig::parse(&blink_spec);
    let mut lipsync_cfg = au2_media::PsdLipSyncConfig::parse(&lipsync_spec);
    let is_isolated = au2_media::PsdDocument::is_isolated_spec(&layers_spec);

    let visible_layer_count = doc
        .layers
        .iter()
        .filter(|l| l.kind != au2_media::PsdLayerKind::FolderEnd)
        .count();

    egui::Frame::none()
        .fill(theme::surface_alt())
        .stroke(Stroke::new(1.0_f32, theme::accent().gamma_multiply(0.55)))
        .rounding(theme::rounding::lg())
        .inner_margin(egui::Margin::same(theme::spacing::XL))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("🎨 PSD 立ち絵・レイヤー設定 (PSDToolKit 互換)")
                        .strong()
                        .color(theme::accent_text()),
                );
                ui.label(
                    RichText::new(format!(
                        "({}×{}px / {} レイヤー)",
                        doc.width, doc.height, visible_layer_count
                    ))
                    .weak()
                    .size(theme::typography::BODY_SM),
                );
                if !psd_label.is_empty() {
                    ui.label(
                        RichText::new(format!("✂ 抽出パーツ: {}", psd_label))
                            .size(theme::typography::BODY_SM)
                            .color(theme::ok_text()),
                    );
                }
                if !layers_spec.is_empty() {
                    if ui
                        .small_button("↺ 全体初期状態に戻す")
                        .on_hover_text("パーツ抽出やレイヤー切替を解除し、PSD ファイル保存時の全体初期状態にリセットします")
                        .clicked()
                    {
                        vis_vec = doc.default_visibility();
                        set_eff_text_param(eff, "psd_layers", String::new());
                        set_eff_text_param(eff, "psd_label", String::new());
                        changed = true;
                    }
                }
            });

            // 0. レイヤー・パーツ分解ツールバー
            ui.add_space(theme::spacing::MD);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("✂ メディア分解:").strong().size(theme::typography::BODY));
                if ui
                    .button("📁 パーツ(フォルダ)単位でタイムラインへ分解")
                    .on_hover_text(
                        "眉・目・口・髪・体などのフォルダ単位で別々のレイヤー行へ分解配置します。\n\
                         各パーツの中心座標が自動計算されるため個別回転・揺れエフェクトに最適で、分解後も目パチ・口パク・表情切替がそのまま使えます。",
                    )
                    .clicked()
                {
                    *pending_action = Some(PsdPendingAction::DecomposeFolders { tight_crop: true });
                }
                if ui
                    .button("📑 表示レイヤーを1枚ずつタイムラインへ分解")
                    .on_hover_text(
                        "現在表示中の全ピクセルレイヤーを1枚ずつ別々のレイヤー行へ分解配置します。\n\
                         各レイヤーの描画範囲で自動トリミング＆座標配置され、ブレンドモード（乗算・スクリーン等）も自動引き継ぎされます。",
                    )
                    .clicked()
                {
                    *pending_action = Some(PsdPendingAction::DecomposeLayers { tight_crop: true });
                }
                ui.menu_button("💾 透過PNG書き出し / 詳細...", |ui| {
                    if ui
                        .button("💾 パーツ(フォルダ)単位で透過PNG一括書き出し...")
                        .clicked()
                    {
                        *pending_action = Some(PsdPendingAction::ExportPng {
                            by_folders: true,
                            tight_crop: false,
                        });
                        ui.close_menu();
                    }
                    if ui
                        .button("💾 表示レイヤーを1枚ずつ透過PNG一括書き出し...")
                        .clicked()
                    {
                        *pending_action = Some(PsdPendingAction::ExportPng {
                            by_folders: false,
                            tight_crop: false,
                        });
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .button("📁 パーツ単位でタイムラインへ分解 (元キャンバス余白を維持)")
                        .clicked()
                    {
                        *pending_action =
                            Some(PsdPendingAction::DecomposeFolders { tight_crop: false });
                        ui.close_menu();
                    }
                    if ui
                        .button("📑 表示レイヤーを1枚ずつ分解 (元キャンバス余白を維持)")
                        .clicked()
                    {
                        *pending_action =
                            Some(PsdPendingAction::DecomposeLayers { tight_crop: false });
                        ui.close_menu();
                    }
                });
            });

            // 1. お気に入り (.pfv) プリセット
            if !doc.favorites.is_empty() {
                ui.add_space(theme::spacing::MD);
                ui.label(RichText::new("⭐ お気に入りプリセット (.pfv)").strong().size(theme::typography::BODY));
                ui.horizontal_wrapped(|ui| {
                    for fav in &doc.favorites {
                        if ui
                            .button(format!("⭐ {}", fav.name))
                            .on_hover_text(format!("{} 個のレイヤー切替を一括適用", fav.active_paths.len()))
                            .clicked()
                        {
                            for path in &fav.active_paths {
                                doc.set_layer_path_visible(&mut vis_vec, path, true);
                            }
                            let serialized =
                                doc.encode_visibility_overrides_preserving(&vis_vec, &layers_spec);
                            set_eff_text_param(eff, "psd_layers", serialized);
                            changed = true;
                        }
                    }
                });
            }

            ui.add_space(theme::spacing::MD);

            // 2. 目パチ・口パク 自動アニメーション設定
            CollapsingHeader::new(
                RichText::new("👁 目パチ ・ 👄 口パク 自動アニメーション").strong(),
            )
            .id_salt(("au2-psd-anim", id))
            .default_open(true)
            .show(ui, |ui| {
                // --- 目パチ ---
                let mut blink_changed = false;
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .checkbox(&mut blink_cfg.enabled, RichText::new("👁 自動目パチを有効化").strong())
                        .changed()
                    {
                        if blink_cfg.enabled
                            && blink_cfg.open_path.is_empty()
                            && blink_cfg.close_path.is_empty()
                        {
                            if let Some((ref open_p, ref close_p)) = doc.detected_blink {
                                blink_cfg.open_path = open_p.clone();
                                blink_cfg.close_path = close_p.clone();
                            }
                        }
                        blink_changed = true;
                    }
                    if blink_cfg.enabled {
                        ui.label("間隔(秒):");
                        blink_changed |= ui
                            .add(
                                DragValue::new(&mut blink_cfg.interval_sec)
                                    .speed(0.1)
                                    .range(0.5..=30.0)
                                    .fixed_decimals(1),
                            )
                            .changed();
                        ui.label("閉じF:");
                        blink_changed |= ui
                            .add(DragValue::new(&mut blink_cfg.close_frames).range(1..=30))
                            .changed();
                    }
                });

                if blink_cfg.enabled {
                    Grid::new(("au2-psd-blink-grid", id))
                        .num_columns(2)
                        .spacing([8.0, 4.0])
                        .show(ui, |ui| {
                            ui.label(RichText::new("  開き目レイヤー:").weak());
                            egui::ComboBox::from_id_salt(("au2-psd-blink-open", id))
                                .selected_text(if blink_cfg.open_path.is_empty() {
                                    "(自動検出)"
                                } else {
                                    &blink_cfg.open_path
                                })
                                .width(220.0)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(blink_cfg.open_path.is_empty(), "(自動検出)")
                                        .clicked()
                                    {
                                        blink_cfg.open_path.clear();
                                        blink_changed = true;
                                    }
                                    for l in doc
                                        .layers
                                        .iter()
                                        .rev()
                                        .filter(|l| l.kind == au2_media::PsdLayerKind::Pixel)
                                    {
                                        if ui
                                            .selectable_label(
                                                blink_cfg.open_path == l.path,
                                                &l.path,
                                            )
                                            .clicked()
                                        {
                                            blink_cfg.open_path = l.path.clone();
                                            blink_changed = true;
                                        }
                                    }
                                });
                            ui.end_row();

                            ui.label(RichText::new("  閉じ目レイヤー:").weak());
                            egui::ComboBox::from_id_salt(("au2-psd-blink-closed", id))
                                .selected_text(if blink_cfg.close_path.is_empty() {
                                    "(自動検出)"
                                } else {
                                    &blink_cfg.close_path
                                })
                                .width(220.0)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(blink_cfg.close_path.is_empty(), "(自動検出)")
                                        .clicked()
                                    {
                                        blink_cfg.close_path.clear();
                                        blink_changed = true;
                                    }
                                    for l in doc
                                        .layers
                                        .iter()
                                        .rev()
                                        .filter(|l| l.kind == au2_media::PsdLayerKind::Pixel)
                                    {
                                        if ui
                                            .selectable_label(
                                                blink_cfg.close_path == l.path,
                                                &l.path,
                                            )
                                            .clicked()
                                        {
                                            blink_cfg.close_path = l.path.clone();
                                            blink_changed = true;
                                        }
                                    }
                                });
                            ui.end_row();
                        });
                }

                if blink_changed {
                    set_eff_text_param(eff, "psd_blink", blink_cfg.to_param_string());
                    changed = true;
                }

                ui.add_space(4.0);

                // --- 口パク ---
                let mut lip_changed = false;
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("👄 口パク:").strong());
                    for (mode, label) in [
                        (au2_media::PsdLipSyncMode::Off, "OFF"),
                        (au2_media::PsdLipSyncMode::AudioSync, "🔊 音声連動"),
                        (au2_media::PsdLipSyncMode::Periodic, "⏱ 周期開閉"),
                    ] {
                        if ui.selectable_label(lipsync_cfg.mode == mode, label).clicked() {
                            if lipsync_cfg.mode != mode {
                                lipsync_cfg.mode = mode;
                                match mode {
                                    au2_media::PsdLipSyncMode::AudioSync => {
                                        lipsync_cfg.param = 1.0;
                                    }
                                    au2_media::PsdLipSyncMode::Periodic => {
                                        lipsync_cfg.param = 4.0;
                                    }
                                    au2_media::PsdLipSyncMode::Off => {}
                                }
                            }
                            if mode != au2_media::PsdLipSyncMode::Off
                                && lipsync_cfg.close_path.is_empty()
                                && lipsync_cfg.open_path.is_empty()
                            {
                                if let Some((ref close_p, ref open_p)) = doc.detected_lipsync {
                                    lipsync_cfg.close_path = close_p.clone();
                                    lipsync_cfg.open_path = open_p.clone();
                                }
                            }
                            lip_changed = true;
                        }
                    }

                    if lipsync_cfg.mode == au2_media::PsdLipSyncMode::AudioSync {
                        ui.label("感度:");
                        lip_changed |= ui
                            .add(
                                DragValue::new(&mut lipsync_cfg.param)
                                    .speed(0.05)
                                    .range(0.1..=5.0)
                                    .fixed_decimals(2),
                            )
                            .changed();
                    } else if lipsync_cfg.mode == au2_media::PsdLipSyncMode::Periodic {
                        ui.label("周期F:");
                        lip_changed |= ui
                            .add(
                                DragValue::new(&mut lipsync_cfg.param)
                                    .speed(1.0)
                                    .range(2.0..=60.0)
                                    .fixed_decimals(0),
                            )
                            .changed();
                    }
                });

                if lipsync_cfg.mode != au2_media::PsdLipSyncMode::Off {
                    Grid::new(("au2-psd-lip-grid", id))
                        .num_columns(2)
                        .spacing([8.0, 4.0])
                        .show(ui, |ui| {
                            ui.label(RichText::new("  閉じ口レイヤー:").weak());
                            egui::ComboBox::from_id_salt(("au2-psd-lip-closed", id))
                                .selected_text(if lipsync_cfg.close_path.is_empty() {
                                    "(自動検出)"
                                } else {
                                    &lipsync_cfg.close_path
                                })
                                .width(220.0)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(lipsync_cfg.close_path.is_empty(), "(自動検出)")
                                        .clicked()
                                    {
                                        lipsync_cfg.close_path.clear();
                                        lip_changed = true;
                                    }
                                    for l in doc
                                        .layers
                                        .iter()
                                        .rev()
                                        .filter(|l| l.kind == au2_media::PsdLayerKind::Pixel)
                                    {
                                        if ui
                                            .selectable_label(
                                                lipsync_cfg.close_path == l.path,
                                                &l.path,
                                            )
                                            .clicked()
                                        {
                                            lipsync_cfg.close_path = l.path.clone();
                                            lip_changed = true;
                                        }
                                    }
                                });
                            ui.end_row();

                            ui.label(RichText::new("  開き口レイヤー:").weak());
                            egui::ComboBox::from_id_salt(("au2-psd-lip-open", id))
                                .selected_text(if lipsync_cfg.open_path.is_empty() {
                                    "(自動検出)"
                                } else {
                                    &lipsync_cfg.open_path
                                })
                                .width(220.0)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(lipsync_cfg.open_path.is_empty(), "(自動検出)")
                                        .clicked()
                                    {
                                        lipsync_cfg.open_path.clear();
                                        lip_changed = true;
                                    }
                                    for l in doc
                                        .layers
                                        .iter()
                                        .rev()
                                        .filter(|l| l.kind == au2_media::PsdLayerKind::Pixel)
                                    {
                                        if ui
                                            .selectable_label(
                                                lipsync_cfg.open_path == l.path,
                                                &l.path,
                                            )
                                            .clicked()
                                        {
                                            lipsync_cfg.open_path = l.path.clone();
                                            lip_changed = true;
                                        }
                                    }
                                });
                            ui.end_row();
                        });
                }

                if lip_changed {
                    set_eff_text_param(eff, "psd_lipsync", lipsync_cfg.to_param_string());
                    changed = true;
                }
            });

            // 3. パーツ・表情クイック切替（フォルダごとのワンクリック排他選択）
            let part_groups = au2_media::collect_part_groups(doc);
            if !part_groups.is_empty() {
                ui.add_space(4.0);
                CollapsingHeader::new(
                    RichText::new(format!(
                        "⚡ パーツ・表情クイック切替 ({} グループ)",
                        part_groups.len()
                    ))
                    .strong(),
                )
                .id_salt(("au2-psd-parts", id))
                .default_open(true)
                .show(ui, |ui| {
                    Grid::new(("au2-psd-parts-grid", id))
                        .num_columns(2)
                        .striped(true)
                        .spacing([10.0, 6.0])
                        .show(ui, |ui| {
                            for (folder_id, child_ids) in &part_groups {
                                let folder_label = doc
                                    .layers
                                    .get(*folder_id)
                                    .map(|fl| fl.path.as_str())
                                    .unwrap_or("グループ");
                                ui.label(RichText::new(format!("📁 {}", folder_label)).strong());
                                ui.horizontal_wrapped(|ui| {
                                    for &child_id in child_ids {
                                        let Some(layer) = doc.layers.get(child_id) else {
                                            continue;
                                        };
                                        let is_vis = vis_vec
                                            .get(child_id)
                                            .copied()
                                            .unwrap_or(layer.default_visible);
                                        let btn_label = &layer.display_name;
                                        if ui
                                            .selectable_label(is_vis, btn_label)
                                            .on_hover_text(format!(
                                                "{}\nクリックでこのフォルダ内のパーツを切り替え",
                                                layer.path
                                            ))
                                            .clicked()
                                        {
                                            let others_vis = child_ids.iter().any(|&oid| {
                                                oid != child_id
                                                    && vis_vec.get(oid).copied().unwrap_or(false)
                                            });
                                            let target_vis = if is_vis && !others_vis && !layer.is_radio {
                                                false
                                            } else {
                                                true
                                            };
                                            doc.set_layer_id_visible(
                                                &mut vis_vec,
                                                child_id,
                                                target_vis,
                                                true,
                                            );
                                            let serialized = doc
                                                .encode_visibility_overrides_preserving(&vis_vec, &layers_spec);
                                            set_eff_text_param(eff, "psd_layers", serialized);
                                            changed = true;
                                        }
                                    }
                                });
                                ui.end_row();
                            }
                        });
                });
            }

            // 4. 全レイヤー階層ツリー
            ui.add_space(4.0);
            CollapsingHeader::new(
                RichText::new(format!("🗂 全レイヤー階層ツリー ({} 件)", visible_layer_count)).strong(),
            )
            .id_salt(("au2-psd-tree", id))
            .default_open(part_groups.is_empty())
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("au2-psd-tree-scroll", id))
                    .max_height(260.0)
                    .show(ui, |ui| {
                        for layer in doc.layers.iter().rev() {
                            if layer.kind == au2_media::PsdLayerKind::FolderEnd {
                                continue;
                            }
                            let mut vis = vis_vec
                                .get(layer.id)
                                .copied()
                                .unwrap_or(layer.default_visible);

                            ui.horizontal(|ui| {
                                ui.add_space(layer.depth as f32 * 14.0);
                                let icon = if matches!(layer.kind, au2_media::PsdLayerKind::FolderStart { .. }) {
                                    "📁"
                                } else if layer.is_radio {
                                    "🔘"
                                } else if layer.is_mandatory {
                                    "📌"
                                } else {
                                    "🖼"
                                };
                                let label_text = format!("{} {}", icon, layer.name);
                                if layer.is_mandatory && !is_isolated {
                                    let mut locked = true;
                                    ui.add_enabled(
                                        false,
                                        egui::Checkbox::new(&mut locked, RichText::new(label_text).strong()),
                                    )
                                    .on_hover_text("* 付き必須レイヤー（常時表示）");
                                } else if ui.checkbox(&mut vis, label_text).changed() {
                                    doc.set_layer_id_visible(&mut vis_vec, layer.id, vis, false);
                                    let serialized =
                                        doc.encode_visibility_overrides_preserving(&vis_vec, &layers_spec);
                                    set_eff_text_param(eff, "psd_layers", serialized);
                                    changed = true;
                                }
                                if layer.clipping {
                                    ui.label(RichText::new("⤵クリッピング").weak().size(theme::typography::CAPTION));
                                }
                            });
                        }
                    });
            });
        });

    changed
}


