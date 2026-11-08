//! シーン全体設定ダイアログ（未選択時表示）。

use eframe::egui::{self, CollapsingHeader, DragValue, Grid, RichText, Stroke};
use crate::state::EditorState;
use crate::theme;

pub(crate) fn scene_settings_ui(ui: &mut egui::Ui, st: &mut EditorState) {
    egui::Frame::none()
        .fill(theme::surface_alt())
        .stroke(Stroke::new(1.0_f32, theme::border()))
        .rounding(theme::rounding::lg())
        .inner_margin(egui::Margin::symmetric(theme::spacing::XL, theme::spacing::LG))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new(format!("🎬 シーン設定: {}", st.scene.name)).strong().size(theme::typography::HEADING));
            });
            ui.add_space(theme::spacing::XS);
            ui.label(RichText::new("タイムライン上のオブジェクトをクリックすると、そのオブジェクトの個別パラメータを編集できます。").weak().size(theme::typography::BODY_SM));
        });

    ui.add_space(theme::spacing::MD);

    let obj_count = st.scene.object_count();
    let s = &mut st.scene.settings;
    egui::Frame::none()
        .fill(theme::surface())
        .stroke(Stroke::new(1.0_f32, theme::border()))
        .rounding(theme::rounding::lg())
        .inner_margin(egui::Margin::same(theme::spacing::XL))
        .show(ui, |ui| {
            Grid::new("au2-scene-settings").num_columns(2).striped(true).spacing(theme::spacing::GRID_FORM).show(ui, |ui| {
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
                    ui.label(RichText::new(format!("= {:.3} fps", s.frame_rate.as_f64())).color(theme::accent_text()));
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
