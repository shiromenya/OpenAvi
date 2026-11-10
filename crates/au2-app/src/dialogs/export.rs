//! 動画・音声書き出し（エクスポート）モーダルダイアログ。

use std::path::PathBuf;

use eframe::egui::{self, vec2, Color32, Key, RichText, Stroke};

use crate::app::OpenAviApp;
use crate::export::{available_video_encoders, ExportConfig, ExportFormat, ExportQuality, ExportSession, VideoEncoder};
use crate::state::{default_projects_dir, EditorState};

/// 動画・音声書き出し（エクスポート）モーダルダイアログの状態。
pub struct ExportModalState {
    pub is_open: bool,
    pub config: ExportConfig,
    pub use_content_range: bool,
    pub session: Option<ExportSession>,
    pub finished_path: Option<PathBuf>,
    pub error_message: Option<String>,
}

impl Default for ExportModalState {
    fn default() -> Self {
        let default_dir = au2_platform::desktop_dir().unwrap_or_else(default_projects_dir);
        Self {
            is_open: false,
            config: ExportConfig {
                output_path: default_dir.join("output.mp4"),
                format: ExportFormat::Mp4,
                quality: ExportQuality::High,
                encoder: VideoEncoder::Auto,
                width: 1920,
                height: 1080,
                fps_num: 30,
                fps_den: 1,
                start_frame: 0,
                end_frame: 299,
                include_audio: true,
            },
            use_content_range: true,
            session: None,
            finished_path: None,
            error_message: None,
        }
    }
}

impl ExportModalState {
    pub fn open_for_state(&mut self, st: &EditorState) {
        let base_dir = st
            .file_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .or_else(au2_platform::desktop_dir)
            .unwrap_or_else(default_projects_dir);
        let stem = st
            .file_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "output".to_string());
        let ext = self.config.format.extension();
        let default_path = base_dir.join(format!("{stem}.{ext}"));

        let mut cfg = ExportConfig::from_scene(&st.scene, default_path);
        cfg.format = self.config.format;
        cfg.quality = self.config.quality;
        cfg.encoder = self.config.encoder;
        cfg.include_audio = self.config.include_audio;
        if let (Some(in_f), Some(out_f)) = (st.timeline.in_point, st.timeline.out_point) {
            cfg.start_frame = in_f.min(out_f);
            cfg.end_frame = in_f.max(out_f);
        } else if !self.use_content_range {
            cfg.end_frame = st.last_frame();
        }
        self.config = cfg;
        self.session = None;
        self.finished_path = None;
        self.error_message = None;
        self.is_open = true;
    }
}

/// 残り秒数を「1時間02分」「3分05秒」「12秒」の形式に整形します。
pub fn format_remaining(secs: f32) -> String {
    let s = secs.max(0.0).ceil() as u64;
    if s >= 3600 {
        format!("{}時間{:02}分", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}分{:02}秒", s / 60, s % 60)
    } else {
        format!("{s}秒")
    }
}

impl OpenAviApp {
    /// 動画・音声の書き出し（エクスポート / レンダリング）設定＆進捗モーダルダイアログ。
    pub fn export_dialog(&mut self, ctx: &egui::Context) {
        if !self.export_modal.is_open {
            return;
        }

        // 書き出しセッションが実行中なら 1 ティックにつき数フレーム進める
        if let Some(mut session) = self.export_modal.session.take() {
            match session.step_frames(&mut self.st, 1000) {
                Ok(true) => {
                    let out_path = session.config.output_path.clone();
                    let elapsed = session.started_at.elapsed().as_secs_f32();
                    self.st.status = format!(
                        "動画を書き出しました: {} ({elapsed:.1}秒)",
                        out_path.display()
                    );
                    self.export_modal.finished_path = Some(out_path);
                    self.export_modal.error_message = None;
                }
                Ok(false) => {
                    self.export_modal.session = Some(session);
                    ctx.request_repaint();
                }
                Err(e) => {
                    self.export_modal.error_message = Some(e);
                }
            }
        }

        egui::Area::new(egui::Id::new("export_modal_backdrop"))
            .interactable(true)
            .order(egui::Order::Middle)
            .fixed_pos([0.0, 0.0])
            .show(ctx, |ui| {
                let screen_rect = ctx.screen_rect();
                ui.painter().rect_filled(screen_rect, 0.0, crate::theme::backdrop());
            });

        let mut close_modal = false;
        let mut do_start = false;
        let mut do_cancel = false;
        let is_running = self.export_modal.session.is_some();
        let has_ffmpeg = self.st.media.has_ffmpeg();

        egui::Window::new("🎬 動画・音声の書き出し (エクスポート)")
            .id(egui::Id::new("export_modal_window"))
            .frame(egui::Frame::window(&ctx.style()).fill(crate::theme::surface()))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .fixed_size([560.0, 500.0])
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 9.0);

                ui.horizontal(|ui| {
                    ui.label(RichText::new("🎞").size(26.0).color(crate::theme::accent_text()));
                    ui.vertical(|ui| {
                        ui.strong(RichText::new("動画・音声ファイルの書き出し").size(15.0));
                        ui.weak("現在のタイムライン（図形・テキスト・画像・動画・音声・スクリプト効果）をファイルに出力します。");
                    });
                });

                ui.separator();

                ui.add_enabled_ui(!is_running, |ui| {
                    // 1. 出力形式と出力先ファイルパス
                    egui::Frame::none()
                        .fill(crate::theme::surface_alt())
                        .stroke(Stroke::new(1.0_f32, crate::theme::border()))
                        .rounding(egui::Rounding::same(6.0))
                        .inner_margin(egui::Margin::same(10.0))
                        .show(ui, |ui| {
                            egui::Grid::new("export-format-grid")
                                .num_columns(2)
                                .spacing([12.0, 8.0])
                                .show(ui, |ui| {
                                    ui.label(RichText::new("出力フォーマット:").strong());
                                    let prev_fmt = self.export_modal.config.format;
                                    egui::ComboBox::from_id_salt("export-format-combo")
                                        .selected_text(self.export_modal.config.format.label())
                                        .width(340.0)
                                        .show_ui(ui, |ui| {
                                            for &fmt in ExportFormat::ALL {
                                                ui.selectable_value(
                                                    &mut self.export_modal.config.format,
                                                    fmt,
                                                    fmt.label(),
                                                );
                                            }
                                        });
                                    if self.export_modal.config.format != prev_fmt {
                                        let new_ext = self.export_modal.config.format.extension();
                                        self.export_modal.config.output_path.set_extension(new_ext);
                                    }
                                    ui.end_row();

                                    ui.label(RichText::new("出力先ファイル:").strong());
                                    ui.horizontal(|ui| {
                                        let mut path_str = self.export_modal.config.output_path.display().to_string();
                                        if ui
                                            .add(
                                                egui::TextEdit::singleline(&mut path_str)
                                                    .desired_width(270.0),
                                            )
                                            .changed()
                                        {
                                            self.export_modal.config.output_path = PathBuf::from(path_str);
                                        }
                                        if ui.button("📁 参照...").clicked() {
                                            let ext = self.export_modal.config.format.extension();
                                            let mut dlg = rfd::FileDialog::new()
                                                .set_title("書き出し先のファイルを指定")
                                                .add_filter(self.export_modal.config.format.label(), &[ext]);
                                            if let Some(parent) = self.export_modal.config.output_path.parent() {
                                                if parent.exists() {
                                                    dlg = dlg.set_directory(parent);
                                                }
                                            }
                                            if let Some(fname) = self
                                                .export_modal
                                                .config
                                                .output_path
                                                .file_name()
                                                .and_then(|n| n.to_str())
                                            {
                                                dlg = dlg.set_file_name(fname);
                                            }
                                            if let Some(picked) = dlg.save_file() {
                                                self.export_modal.config.output_path = picked;
                                            }
                                        }
                                    });
                                    ui.end_row();
                                });
                        });

                    // 2. 解像度・フレームレート・画質・フレーム範囲
                    egui::Frame::none()
                        .fill(crate::theme::surface_alt())
                        .stroke(Stroke::new(1.0_f32, crate::theme::border()))
                        .rounding(egui::Rounding::same(6.0))
                        .inner_margin(egui::Margin::same(10.0))
                        .show(ui, |ui| {
                            egui::Grid::new("export-video-params-grid")
                                .num_columns(2)
                                .spacing([12.0, 7.0])
                                .show(ui, |ui| {
                                    if !self.export_modal.config.format.is_audio_only() {
                                        ui.label("出力解像度:");
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::DragValue::new(&mut self.export_modal.config.width)
                                                    .range(16..=4096)
                                                    .speed(2),
                                            );
                                            ui.label("×");
                                            ui.add(
                                                egui::DragValue::new(&mut self.export_modal.config.height)
                                                    .range(16..=4096)
                                                    .speed(2),
                                            );
                                            ui.label(RichText::new("px").weak());
                                            if ui.small_button("シーン設定と同じ").clicked() {
                                                self.export_modal.config.width =
                                                    ((self.st.scene.settings.width.max(16) + 1) / 2) * 2;
                                                self.export_modal.config.height =
                                                    ((self.st.scene.settings.height.max(16) + 1) / 2) * 2;
                                            }
                                            if ui.small_button("1080p").clicked() {
                                                self.export_modal.config.width = 1920;
                                                self.export_modal.config.height = 1080;
                                            }
                                            if ui.small_button("720p").clicked() {
                                                self.export_modal.config.width = 1280;
                                                self.export_modal.config.height = 720;
                                            }
                                        });
                                        ui.end_row();

                                        ui.label("フレームレート:");
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::DragValue::new(&mut self.export_modal.config.fps_num)
                                                    .range(1..=240),
                                            );
                                            ui.label("/");
                                            ui.add(
                                                egui::DragValue::new(&mut self.export_modal.config.fps_den)
                                                    .range(1..=1001),
                                            );
                                            let fps = self.export_modal.config.fps_f64();
                                            ui.label(
                                                RichText::new(format!("= {fps:.2} fps"))
                                                    .color(crate::theme::accent_text()),
                                            );
                                            if ui.small_button("60fps").clicked() {
                                                self.export_modal.config.fps_num = 60;
                                                self.export_modal.config.fps_den = 1;
                                            }
                                            if ui.small_button("30fps").clicked() {
                                                self.export_modal.config.fps_num = 30;
                                                self.export_modal.config.fps_den = 1;
                                            }
                                            if ui.small_button("24fps").clicked() {
                                                self.export_modal.config.fps_num = 24;
                                                self.export_modal.config.fps_den = 1;
                                            }
                                        });
                                        ui.end_row();

                                        ui.label("画質設定:");
                                        ui.horizontal(|ui| {
                                            for q in [
                                                ExportQuality::High,
                                                ExportQuality::Standard,
                                                ExportQuality::Compact,
                                            ] {
                                                ui.radio_value(&mut self.export_modal.config.quality, q, q.label());
                                            }
                                        });
                                        ui.end_row();

                                        if matches!(
                                            self.export_modal.config.format,
                                            ExportFormat::Mp4 | ExportFormat::Mov
                                        ) {
                                            ui.label("エンコーダ:");
                                            ui.horizontal(|ui| {
                                                let available = available_video_encoders(self.st.media.ffmpeg_path());
                                                let resolved = self.export_modal.config.encoder.resolve(available);

                                                egui::ComboBox::from_id_salt("export-video-encoder-combo")
                                                    .selected_text(self.export_modal.config.encoder.label())
                                                    .show_ui(ui, |ui| {
                                                        ui.selectable_value(
                                                            &mut self.export_modal.config.encoder,
                                                            VideoEncoder::Auto,
                                                            VideoEncoder::Auto.label(),
                                                        );
                                                        ui.separator();
                                                        ui.selectable_value(
                                                            &mut self.export_modal.config.encoder,
                                                            VideoEncoder::Cpu,
                                                            VideoEncoder::Cpu.label(),
                                                        );
                                                        for enc in VideoEncoder::ALL_HARDWARE {
                                                            let is_avail = available.contains(enc);
                                                            let label = if is_avail {
                                                                format!("{} [利用可能]", enc.label())
                                                            } else {
                                                                format!("{} (非対応環境)", enc.label())
                                                            };
                                                            if is_avail {
                                                                ui.selectable_value(
                                                                    &mut self.export_modal.config.encoder,
                                                                    *enc,
                                                                    label,
                                                                );
                                                            } else {
                                                                ui.add_enabled(false, egui::Button::new(label));
                                                            }
                                                        }
                                                    });

                                                if resolved.is_hardware_accelerated() {
                                                    let tip = format!("⚡ GPU 高速エンコード有効 ({})", resolved.short_name());
                                                    ui.label(
                                                        RichText::new(tip)
                                                            .color(crate::theme::ok_text())
                                                            .strong()
                                                            .size(11.5),
                                                    );
                                                } else {
                                                    ui.label(
                                                        RichText::new("🛡️ CPU 安全エンコード (互換性重視)")
                                                            .color(crate::theme::text_muted())
                                                            .size(11.5),
                                                    );
                                                }
                                            });
                                            ui.end_row();
                                        }
                                    }

                                    ui.label("書き出し範囲:");
                                    ui.vertical(|ui| {
                                        let content_end = self.st.scene.content_end().saturating_sub(1);
                                        let scene_last = self.st.last_frame();
                                        ui.horizontal(|ui| {
                                            ui.label("開始フレーム:");
                                            let mut start_1 = self.export_modal.config.start_frame + 1;
                                            if ui
                                                .add(egui::DragValue::new(&mut start_1).range(1..=(scene_last + 1)))
                                                .changed()
                                            {
                                                self.export_modal.config.start_frame = start_1.saturating_sub(1);
                                                if self.export_modal.config.end_frame < self.export_modal.config.start_frame {
                                                    self.export_modal.config.end_frame = self.export_modal.config.start_frame;
                                                }
                                            }
                                            ui.label("〜 終了フレーム:");
                                            let mut end_1 = self.export_modal.config.end_frame + 1;
                                            if ui
                                                .add(
                                                    egui::DragValue::new(&mut end_1)
                                                        .range((self.export_modal.config.start_frame + 1)..=(scene_last + 10000)),
                                                )
                                                .changed()
                                            {
                                                self.export_modal.config.end_frame = end_1.saturating_sub(1);
                                            }
                                        });
                                        ui.horizontal(|ui| {
                                            if let (Some(inf), Some(outf)) = (self.st.timeline.in_point, self.st.timeline.out_point) {
                                                let (sf, ef) = (inf.min(outf), inf.max(outf));
                                                let btn = egui::Button::new(
                                                    RichText::new(format!("📍 選択範囲 ({}〜{}F)", sf + 1, ef + 1))
                                                        .color(Color32::WHITE)
                                                        .strong(),
                                                ).fill(crate::theme::accent());
                                                if ui.add(btn).clicked() {
                                                    self.export_modal.config.start_frame = sf;
                                                    self.export_modal.config.end_frame = ef;
                                                }
                                            }
                                            if ui
                                                .small_button(format!("オブジェクト終端まで (1〜{}F)", content_end + 1))
                                                .clicked()
                                            {
                                                self.export_modal.config.start_frame = 0;
                                                self.export_modal.config.end_frame = content_end;
                                            }
                                            if ui
                                                .small_button(format!("シーン全体 (1〜{}F)", scene_last + 1))
                                                .clicked()
                                            {
                                                self.export_modal.config.start_frame = 0;
                                                self.export_modal.config.end_frame = scene_last;
                                            }
                                            let total_f = self.export_modal.config.total_frames();
                                            let dur_sec = total_f as f64 / self.export_modal.config.fps_f64().max(1.0);
                                            ui.weak(format!("(計 {total_f} フレーム / 約 {dur_sec:.1} 秒)"));
                                        });
                                    });
                                    ui.end_row();

                                    if !self.export_modal.config.format.is_audio_only()
                                        && self.export_modal.config.format.supports_audio()
                                    {
                                        ui.label("音声出力:");
                                        ui.checkbox(
                                            &mut self.export_modal.config.include_audio,
                                            "音声トラックを合成して動画に含める (44.1kHz ステレオ)",
                                        );
                                        ui.end_row();
                                    }
                                });
                        });
                });

                // 3. FFmpeg 未導入警告 / 進捗バー / 完了通知 / エラー表示
                if !has_ffmpeg && !self.export_modal.config.format.is_audio_only() {
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            crate::theme::warn_text(),
                            "⚠ 動画書き出しには FFmpeg が必要です。",
                        );
                        if ui.button("⬇ FFmpeg を自動ダウンロード").clicked() {
                            self.st.start_ffmpeg_download();
                        }
                    });
                }

                if let Some(session) = &self.export_modal.session {
                    let ratio = session.progress_ratio();
                    let done = session.current_frame.saturating_sub(session.config.start_frame);
                    let total = session.config.total_frames();
                    let eta_text = match session.eta_seconds() {
                        Some(s) => format!("残り約 {}", format_remaining(s)),
                        None => "残り時間を計算中...".to_string(),
                    };
                    ui.add(
                        egui::ProgressBar::new(ratio)
                            .show_percentage()
                            .animate(true)
                            .text(format!(
                                "レンダリング中... {done} / {total} フレーム ({:.0}%) [{eta_text}]",
                                ratio * 100.0
                            )),
                    );
                } else if let Some(done_path) = &self.export_modal.finished_path {
                    let (ok_bg, ok_stroke) = crate::theme::ok_banner();
                    egui::Frame::none()
                        .fill(ok_bg)
                        .stroke(Stroke::new(1.0_f32, ok_stroke))
                        .rounding(egui::Rounding::same(6.0))
                        .inner_margin(egui::Margin::same(8.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.colored_label(
                                    crate::theme::ok_text(),
                                    format!("✅ 書き出しが完了しました: {}", done_path.display()),
                                );
                                if let Some(_parent) = done_path.parent() {
                                    if ui.small_button("📂 保存先フォルダを開く").clicked() {
                                        au2_platform::reveal_in_file_manager(done_path);
                                    }
                                }
                            });
                        });
                }

                if let Some(err) = &self.export_modal.error_message {
                    ui.colored_label(crate::theme::danger_text(), format!("⚠ {err}"));
                }

                ui.separator();

                // 下部アクションボタン
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if is_running {
                        let cancel_btn = egui::Button::new(
                            RichText::new("⏹ 書き出しを中止")
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .min_size(vec2(130.0, 32.0))
                        .fill(Color32::from_rgb(185, 45, 45));
                        if ui.add(cancel_btn).clicked() {
                            do_cancel = true;
                        }
                    } else {
                        let can_export = has_ffmpeg || self.export_modal.config.format.is_audio_only();
                        let start_btn = egui::Button::new(
                            RichText::new("🎬 書き出しを開始")
                                .color(Color32::WHITE)
                                .strong()
                                .size(13.5),
                        )
                        .min_size(vec2(150.0, 32.0))
                        .fill(crate::theme::accent());

                        if ui.add_enabled(can_export, start_btn).clicked() {
                            do_start = true;
                        }
                        let close_btn = egui::Button::new("閉じる").min_size(vec2(80.0, 32.0));
                        if ui.add(close_btn).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            close_modal = true;
                        }
                    }
                });
            });

        if do_start {
            // 再生中なら停止してから書き出し開始
            if self.st.playing {
                self.st.toggle_play();
            }
            self.export_modal.finished_path = None;
            self.export_modal.error_message = None;
            match ExportSession::start(&mut self.st, self.export_modal.config.clone()) {
                Ok(session) => {
                    self.export_modal.session = Some(session);
                    ctx.request_repaint();
                }
                Err(e) => {
                    self.export_modal.error_message = Some(e);
                }
            }
        }

        if do_cancel {
            if let Some(mut session) = self.export_modal.session.take() {
                session.cancel();
            }
            self.st.status = "動画の書き出しをキャンセルしました".into();
        }

        if close_modal {
            self.export_modal.is_open = false;
            self.export_modal.error_message = None;
        }
    }
}
