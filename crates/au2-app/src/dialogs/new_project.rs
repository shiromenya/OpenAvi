//! 新規プロジェクト事前設定モーダルダイアログ。

use std::path::PathBuf;

use eframe::egui::{self, vec2, Color32, Key, RichText, Stroke};

use crate::app::OpenAviApp;
use crate::state::{default_projects_dir, NewProjectConfig, ProjectFormat, ProjectPreset};

/// 新規プロジェクト作成モーダルダイアログの状態。
#[derive(Debug, Clone)]
pub struct NewProjectModalState {
    pub is_open: bool,
    pub name: String,
    pub dir: PathBuf,
    pub format: ProjectFormat,
    pub preset: ProjectPreset,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub length_seconds: u32,
    pub autosave_enabled: bool,
    pub error_message: Option<String>,
}

impl Default for NewProjectModalState {
    fn default() -> Self {
        Self {
            is_open: false,
            name: "新しい動画プロジェクト".into(),
            dir: default_projects_dir(),
            format: ProjectFormat::Aup2,
            preset: ProjectPreset::Fhd60p,
            width: 1920,
            height: 1080,
            fps_num: 60,
            fps_den: 1,
            length_seconds: 60,
            autosave_enabled: false,
            error_message: None,
        }
    }
}

impl OpenAviApp {
    /// 新規プロジェクト立ち上げ時の名前・形式・プリセット・自動保存の事前設定ダイアログ。
    pub fn new_project_dialog(&mut self, ctx: &egui::Context) {
        if !self.new_project_modal.is_open {
            return;
        }

        // 半透明の背景オーバーレイ（画面全体のクリックをガード）
        egui::Area::new(egui::Id::new("new_project_backdrop"))
            .interactable(true)
            .order(egui::Order::Middle)
            .fixed_pos([0.0, 0.0])
            .show(ctx, |ui| {
                let screen_rect = ctx.screen_rect();
                ui.painter().rect_filled(screen_rect, 0.0, crate::theme::backdrop());
            });

        let mut close_modal = false;
        let mut do_create = false;

        egui::Window::new("📄 新規プロジェクトの作成")
            .id(egui::Id::new("new_project_modal_window"))
            .frame(egui::Frame::window(&ctx.style()).fill(crate::theme::surface()))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .fixed_size([540.0, 520.0])
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 10.0);

                ui.horizontal(|ui| {
                    ui.label(RichText::new("🎬").size(28.0).color(crate::theme::accent_text()));
                    ui.vertical(|ui| {
                        ui.strong(RichText::new("新規プロジェクトの設定").size(15.0));
                        ui.weak("プロジェクト名とファイル形式、および自動保存を設定して編集を開始します。");
                    });
                });

                ui.separator();

                // 1. プロジェクト基本情報（名前・保存先フォルダー・形式）
                egui::Frame::none()
                    .fill(crate::theme::surface_alt())
                    .stroke(Stroke::new(1.0_f32, crate::theme::border()))
                    .rounding(egui::Rounding::same(6.0))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        egui::Grid::new("new-proj-basic-grid")
                            .num_columns(2)
                            .spacing([12.0, 8.0])
                            .show(ui, |ui| {
                                ui.label(RichText::new("プロジェクト名:").strong());
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.new_project_modal.name)
                                            .hint_text("プロジェクト名を入力")
                                            .desired_width(280.0),
                                    );
                                });
                                ui.end_row();

                                ui.label(RichText::new("保存先フォルダー:").strong());
                                ui.horizontal(|ui| {
                                    let dir_str = self.new_project_modal.dir.display().to_string();
                                    ui.monospace(if dir_str.len() > 32 {
                                        format!("...{}", &dir_str[dir_str.len() - 30..])
                                    } else {
                                        dir_str
                                    }).on_hover_text(self.new_project_modal.dir.display().to_string());

                                    if ui.button("📁 選択...").clicked() {
                                        if let Some(folder) = rfd::FileDialog::new()
                                            .set_directory(&self.new_project_modal.dir)
                                            .set_title("保存先フォルダーを選択")
                                            .pick_folder()
                                        {
                                            self.new_project_modal.dir = folder;
                                        }
                                    }
                                });
                                ui.end_row();

                                ui.label(RichText::new("ファイル形式:").strong());
                                ui.vertical(|ui| {
                                    ui.radio_value(
                                        &mut self.new_project_modal.format,
                                        ProjectFormat::Aup2,
                                        ProjectFormat::Aup2.label(),
                                    );
                                    ui.radio_value(
                                        &mut self.new_project_modal.format,
                                        ProjectFormat::Aup,
                                        ProjectFormat::Aup.label(),
                                    );
                                    ui.radio_value(
                                        &mut self.new_project_modal.format,
                                        ProjectFormat::Exo,
                                        ProjectFormat::Exo.label(),
                                    );
                                });
                                ui.end_row();
                            });

                        // 保存されるフルパスのプレビュー表示
                        let ext = self.new_project_modal.format.extension();
                        let stem = self.new_project_modal.name.trim();
                        let safe_name = if stem.is_empty() { "無題" } else { stem };
                        let full_file = if safe_name.to_ascii_lowercase().ends_with(&format!(".{ext}")) {
                            safe_name.to_string()
                        } else {
                            format!("{safe_name}.{ext}")
                        };
                        let target_display = self.new_project_modal.dir.join(&full_file);
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("保存先パス:").weak().size(11.0));
                            ui.label(RichText::new(target_display.display().to_string()).color(crate::theme::accent_text()).size(11.0));
                        });
                    });

                // 2. 映像フォーマット（プリセット & 解像度・フレームレート）
                egui::Frame::none()
                    .fill(crate::theme::surface_alt())
                    .stroke(Stroke::new(1.0_f32, crate::theme::border()))
                    .rounding(egui::Rounding::same(6.0))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("映像フォーマット:").strong());
                            ui.menu_button(self.new_project_modal.preset.label(), |ui| {
                                for &p in &[
                                    ProjectPreset::Fhd60p,
                                    ProjectPreset::Fhd30p,
                                    ProjectPreset::Hd720p,
                                    ProjectPreset::ShortsVertical,
                                    ProjectPreset::Uhd4k,
                                ] {
                                    if ui.button(p.label()).clicked() {
                                        self.new_project_modal.preset = p;
                                        p.apply(
                                            &mut self.new_project_modal.width,
                                            &mut self.new_project_modal.height,
                                            &mut self.new_project_modal.fps_num,
                                            &mut self.new_project_modal.fps_den,
                                        );
                                        ui.close_menu();
                                    }
                                }
                            });
                        });

                        ui.add_space(4.0);
                        egui::Grid::new("new-proj-video-grid")
                            .num_columns(2)
                            .spacing([12.0, 6.0])
                            .show(ui, |ui| {
                                ui.label("解像度 (幅×高):");
                                ui.horizontal(|ui| {
                                    if ui.add(egui::DragValue::new(&mut self.new_project_modal.width).range(16..=8192)).changed() {
                                        self.new_project_modal.preset = ProjectPreset::Custom;
                                    }
                                    ui.label("×");
                                    if ui.add(egui::DragValue::new(&mut self.new_project_modal.height).range(16..=8192)).changed() {
                                        self.new_project_modal.preset = ProjectPreset::Custom;
                                    }
                                    ui.label(RichText::new("px").weak());
                                });
                                ui.end_row();

                                ui.label("フレームレート:");
                                ui.horizontal(|ui| {
                                    if ui.add(egui::DragValue::new(&mut self.new_project_modal.fps_num).range(1..=240_000)).changed() {
                                        self.new_project_modal.preset = ProjectPreset::Custom;
                                    }
                                    ui.label("/");
                                    if ui.add(egui::DragValue::new(&mut self.new_project_modal.fps_den).range(1..=1001)).changed() {
                                        self.new_project_modal.preset = ProjectPreset::Custom;
                                    }
                                    let fps_val = self.new_project_modal.fps_num as f64 / self.new_project_modal.fps_den.max(1) as f64;
                                    ui.label(RichText::new(format!("= {:.3} fps", fps_val)).color(crate::theme::accent_text()));
                                });
                                ui.end_row();

                                ui.label("初期の長さ:");
                                ui.horizontal(|ui| {
                                    ui.add(egui::DragValue::new(&mut self.new_project_modal.length_seconds).range(1..=36000).suffix(" 秒"));
                                    let fps_val = self.new_project_modal.fps_num as f64 / self.new_project_modal.fps_den.max(1) as f64;
                                    let total_frames = (self.new_project_modal.length_seconds as f64 * fps_val).round() as u64;
                                    ui.label(RichText::new(format!("(約 {} フレーム)", total_frames)).weak());
                                });
                                ui.end_row();
                            });
                    });

                // 3. 💾 自動保存設定
                egui::Frame::none()
                    .fill(crate::theme::surface_alt())
                    .stroke(Stroke::new(1.0_f32, crate::theme::border()))
                    .rounding(egui::Rounding::same(6.0))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.new_project_modal.autosave_enabled, RichText::new("💾 編集時の自動保存を有効にする").strong());
                            if self.new_project_modal.autosave_enabled {
                                ui.label(RichText::new("推奨").color(crate::theme::ok_text()).size(11.0));
                            }
                        });
                        ui.add_space(2.0);
                        ui.weak("※ 有効時、レイヤーやオブジェクトに変更が加わるたびに自動でプロジェクトへ即座に上書き保存されます。");
                        ui.weak("※ 画面上部のトグルスイッチからいつでもワンクリックでオン/オフを切り替えられます。");
                    });

                // エラーメッセージがある場合は表示
                if let Some(err) = &self.new_project_modal.error_message {
                    ui.colored_label(crate::theme::danger_text(), format!("⚠ {err}"));
                }

                ui.separator();

                // 下部アクションボタン
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let create_btn = egui::Button::new(
                        RichText::new("🚀 作成して編集を開始")
                            .color(Color32::WHITE)
                            .strong()
                            .size(13.5),
                    )
                    .min_size(vec2(170.0, 32.0))
                    .fill(crate::theme::accent());

                    if ui.add(create_btn).clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                        do_create = true;
                    }

                    let cancel_btn = egui::Button::new("キャンセル").min_size(vec2(90.0, 32.0));
                    if ui.add(cancel_btn).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        close_modal = true;
                    }
                });
            });

        if do_create {
            let name = self.new_project_modal.name.trim();
            if name.is_empty() {
                self.new_project_modal.error_message = Some("プロジェクト名を入力してください".to_string());
            } else {
                let fps_val = self.new_project_modal.fps_num as f64 / self.new_project_modal.fps_den.max(1) as f64;
                let length_frames = (self.new_project_modal.length_seconds as f64 * fps_val).round().max(1.0) as u32;

                let config = NewProjectConfig {
                    name: name.to_string(),
                    dir: self.new_project_modal.dir.clone(),
                    format: self.new_project_modal.format,
                    width: self.new_project_modal.width,
                    height: self.new_project_modal.height,
                    fps_num: self.new_project_modal.fps_num,
                    fps_den: self.new_project_modal.fps_den,
                    length_frames,
                    autosave_enabled: self.new_project_modal.autosave_enabled,
                };

                match self.st.create_new_project_with_config(config) {
                    Ok(_) => {
                        self.new_project_modal.is_open = false;
                        self.new_project_modal.error_message = None;
                    }
                    Err(e) => {
                        self.new_project_modal.error_message = Some(e);
                    }
                }
            }
        }

        if close_modal {
            self.new_project_modal.is_open = false;
            self.new_project_modal.error_message = None;
        }
    }
}
