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

    let Some((layer, obj)) = st.scene.find_mut(id) else {
        st.selected = None;
        return;
    };

    let rel = frame.saturating_sub(obj.frame_start).min(obj.length() - 1);
    let keys = obj.key_frames();
    let seg = keys.partition_point(|&k| k <= rel).saturating_sub(1).min(keys.len().saturating_sub(2));

    let mut pending_effect: Option<Effect> = None;

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
                        ui.label(RichText::new("標準内蔵フィルタ").weak().small());
                        if ui.button("フェード").clicked() {
                            let mut eff = Effect::new("フェード");
                            eff.push_track("イン", Track::constant(0.50, 2));
                            eff.push_track("アウト", Track::constant(0.50, 2));
                            pending_effect = Some(eff);
                            ui.close_menu();
                        }
                        if ui.button("縁取り").clicked() {
                            let mut eff = Effect::new("縁取り");
                            eff.push_track("サイズ", Track::constant(4.0, 0));
                            eff.push_text("color", "ffffff");
                            pending_effect = Some(eff);
                            ui.close_menu();
                        }
                        if ui.button("ドロップシャドウ").clicked() {
                            let mut eff = Effect::new("ドロップシャドウ");
                            eff.push_track("X", Track::constant(8.0, 0));
                            eff.push_track("Y", Track::constant(8.0, 0));
                            eff.push_track("濃さ", Track::constant(60.0, 1));
                            pending_effect = Some(eff);
                            ui.close_menu();
                        }
                        if ui.button("グロー (発光)").clicked() {
                            let mut eff = Effect::new("グロー");
                            eff.push_track("強さ", Track::constant(60.0, 1));
                            pending_effect = Some(eff);
                            ui.close_menu();
                        }
                        if ui.button("色ズレ (色収差)").clicked() {
                            let mut eff = Effect::new("色ズレ");
                            eff.push_track("ズレ幅", Track::constant(8.0, 1));
                            eff.push_track("角度", Track::constant(0.0, 1));
                            pending_effect = Some(eff);
                            ui.close_menu();
                        }
                        if ui.button("単色化").clicked() {
                            let mut eff = Effect::new("単色化");
                            eff.push_track("強さ", Track::constant(80.0, 1));
                            eff.push_text("color", "ffcc66");
                            pending_effect = Some(eff);
                            ui.close_menu();
                        }
                        if ui.button("振動").clicked() {
                            let mut eff = Effect::new("振動");
                            eff.push_track("X", Track::constant(15.0, 1));
                            eff.push_track("Y", Track::constant(15.0, 1));
                            eff.push_track("周期", Track::constant(1.0, 1));
                            pending_effect = Some(eff);
                            ui.close_menu();
                        }
                        if !available_scripts.is_empty() {
                            ui.separator();
                            ui.label(RichText::new("スクリプト & .auf プラグイン (32bit/64bit)").weak().small());
                            for (kind, entry) in &available_scripts {
                                if ui.button(&entry.display_name).clicked() {
                                    pending_effect = Some(entry.to_effect(*kind));
                                    ui.close_menu();
                                }
                            }
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
                                    ui.horizontal(|ui| {
                                        file_changed |= ui.add(TextEdit::singleline(s).desired_width(150.0)).changed();
                                        if ui.button("参照...").clicked() {
                                            if let Some(p) = rfd::FileDialog::new()
                                                .add_filter(
                                                    "メディアファイル",
                                                    &[
                                                        "mp4", "avi", "mov", "mkv", "webm", "wmv", "png", "jpg",
                                                        "jpeg", "bmp", "webp", "gif", "wav", "mp3", "ogg", "flac",
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
