//! プラグイン管理＆.auf移植スタジオウィンドウ。

use au2_plugin::SupportStatus;
use eframe::egui::{self, vec2, Color32, RichText, Stroke};

use crate::app::OpenAviApp;

impl OpenAviApp {
    /// プラグイン管理ウィンドウを描画します。
    pub fn plugins_window(&mut self, ctx: &egui::Context) {
        if !self.show_plugins {
            return;
        }
        let mut open = self.show_plugins;
        egui::Window::new("プラグイン・スクリプト管理")
            .frame(egui::Frame::window(&ctx.style()).fill(crate::theme::surface()))
            .open(&mut open)
            .default_size([680.0, 420.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("検索フォルダ: {}", self.st.plugins.plugin_dir.display()));
                    if ui.button("再スキャン").clicked() {
                        self.st.plugins.reload();
                    }
                    if ui.button("ファイルを追加インストール...").clicked() {
                        self.st.install_plugin_dialog();
                    }
                    if ui
                        .button(RichText::new("🔄 .auf 仕様解析 & 移植スタジオ").strong())
                        .clicked()
                    {
                        self.st.open_auf_porter(None);
                    }
                });
                ui.weak("※ ウィンドウに .anm / .obj / .auf 等のファイルをドラッグ＆ドロップしてもインストールできます。");
                ui.separator();

                if self.st.plugins.plugins.is_empty() {
                    ui.label("プラグインまたはスクリプトがまだ登録されていません。");
                } else {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        egui::Grid::new("openavi-plugin-grid")
                            .striped(true)
                            .num_columns(4)
                            .spacing([12.0, 6.0])
                            .show(ui, |ui| {
                                ui.strong("ファイル名");
                                ui.strong("種別");
                                ui.strong("アーキテクチャ");
                                ui.strong("対応状況 / 含まれるエントリ");
                                ui.end_row();

                                for p in &self.st.plugins.plugins {
                                    ui.monospace(&p.file_name);
                                    ui.label(p.kind.label());
                                    ui.label(p.arch.label());
                                    ui.vertical(|ui| {
                                        let color = match p.status {
                                            SupportStatus::Supported => crate::theme::ok_text(),
                                            SupportStatus::PartialScript => crate::theme::warn_text(),
                                            _ => crate::theme::text_muted(),
                                        };
                                        ui.label(RichText::new(p.status.summary()).color(color));
                                        if !p.description.is_empty() {
                                            ui.small(RichText::new(&p.description).weak());
                                        }
                                        if !p.exports.is_empty() {
                                            ui.small(
                                                RichText::new(format!("エクスポートシンボル: {}", p.exports.join(", ")))
                                                    .color(crate::theme::accent_text()),
                                            );
                                        }
                                        for e in &p.entries {
                                            let clean_entry_name = au2_plugin::clean_script_name(&e.display_name);
                                            let tracks: Vec<&str> = e.tracks.iter().map(|t| t.name.as_str()).collect();
                                            ui.small(format!(
                                                "・{} (トラック: [{}])",
                                                clean_entry_name,
                                                tracks.join(", ")
                                            ));
                                        }
                                    });
                                    ui.end_row();
                                }
                            });
                    });
                }
            });
        self.show_plugins = open;
    }

    /// `.auf` / `.auf2` バイナリ構造の自動解析と、クロスプラットフォーム `.anm` (Lua) / WGSL シェーダー / Rust コード自動生成スタジオ。
    pub fn auf_porter_window(&mut self, ctx: &egui::Context) {
        if !self.st.auf_porter.is_open {
            return;
        }
        let mut is_open = self.st.auf_porter.is_open;

        egui::Window::new("🔄 .auf / .auf2 仕様解析 & クロスプラットフォーム移植スタジオ")
            .frame(egui::Frame::window(&ctx.style()).fill(crate::theme::surface()))
            .open(&mut is_open)
            .default_size([780.0, 620.0])
            .resizable(true)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 6.0);

                // 上部ツールバー：対象プラグイン選択・外部ファイル読込・一括変換
                egui::Frame::none()
                    .fill(crate::theme::surface_alt())
                    .stroke(Stroke::new(1.0_f32, crate::theme::border()))
                    .rounding(egui::Rounding::same(6.0))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new("解析対象プラグイン:").strong());

                            let plugin_count = self.st.plugins.plugins.len();
                            if plugin_count > 0 {
                                let cur_idx = self.st.auf_porter.selected_plugin_idx.min(plugin_count - 1);
                                self.st.auf_porter.selected_plugin_idx = cur_idx;
                                let cur_label = format!(
                                    "{} [{}]",
                                    self.st.plugins.plugins[cur_idx].file_name,
                                    self.st.plugins.plugins[cur_idx].arch.label()
                                );
                                egui::ComboBox::from_id_salt("auf_porter_plugin_combo")
                                    .selected_text(cur_label)
                                    .width(260.0)
                                    .show_ui(ui, |ui| {
                                        for (idx, p) in self.st.plugins.plugins.iter().enumerate() {
                                            let prefix = if matches!(p.kind, au2_plugin::PluginKind::FilterDll) {
                                                "🧪 [ネイティブ .auf] "
                                            } else {
                                                "✨ "
                                            };
                                            let lbl = format!("{prefix}{} ({})", p.file_name, p.arch.label());
                                            ui.selectable_value(&mut self.st.auf_porter.selected_plugin_idx, idx, lbl);
                                        }
                                    });
                            }

                            if ui
                                .button("📂 外部の .auf / .auf2 を選択...")
                                .on_hover_text("PC 上の任意の .auf / .auf2 ファイルを選択してインストール＆仕様解析します")
                                .clicked()
                            {
                                if let Some(path) = rfd::FileDialog::new()
                                    .set_title("解析・移植する .auf / .auf2 プラグインを選択")
                                    .add_filter("AviUtl フィルタプラグイン (*.auf, *.auf2, *.mod2, *.aux2)", &["auf", "auf2", "mod2", "aux2", "aui", "auc"])
                                    .add_filter("すべてのファイル", &["*"])
                                    .pick_file()
                                {
                                    if let Ok(fname) = self.st.plugins.install_file(&path) {
                                        if let Some((idx, _)) = self
                                            .st
                                            .plugins
                                            .plugins
                                            .iter()
                                            .enumerate()
                                            .find(|(_, p)| p.file_name == fname)
                                        {
                                            self.st.auf_porter.selected_plugin_idx = idx;
                                        }
                                        self.st.status = format!("`{fname}` を読み込み、PE構造とパラメータ仕様を解析しました");
                                    }
                                }
                            }

                            if ui
                                .button(
                                    RichText::new("⚡ 全 .auf を一括で .anm へ変換")
                                        .color(crate::theme::ok_text())
                                        .strong(),
                                 )
                                .on_hover_text("登録されているすべての .auf / .auf2 プラグインを一括で Mac / Linux / Windows 互換の .anm スクリプトに変換します")
                                .clicked()
                            {
                                self.st.convert_all_auf_to_anm();
                            }
                        });
                    });

                let Some(desc) = self
                    .st
                    .plugins
                    .plugins
                    .get(self.st.auf_porter.selected_plugin_idx)
                    .cloned()
                else {
                    ui.label("解析可能なプラグインがありません。");
                    return;
                };

                let report = au2_plugin::AufPortingReport::analyze(&desc);

                // 1. PE バイナリ構造・SDK世代・移植難易度診断レポート
                egui::Frame::none()
                    .fill(crate::theme::surface_alt())
                    .stroke(Stroke::new(1.0_f32, crate::theme::border()))
                    .rounding(egui::Rounding::same(6.0))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(RichText::new(format!("🔍 バイナリ仕様診断: {}", desc.file_name)).size(13.5));
                            ui.separator();
                            ui.colored_label(crate::theme::ok_text(), report.difficulty.badge_label());
                        });
                        ui.small(report.difficulty.description());
                        ui.add_space(4.0);

                        egui::Grid::new("auf_report_meta_grid")
                            .num_columns(2)
                            .spacing([12.0, 4.0])
                            .show(ui, |ui| {
                                ui.label(RichText::new("アーキテクチャ / SDK:").weak());
                                ui.monospace(format!("{} — {}", desc.arch.label(), report.sdk_generation));
                                ui.end_row();

                                ui.label(RichText::new("エクスポート関数:").weak());
                                ui.monospace(if report.exports.is_empty() {
                                    "なし (セクション構造体スキャン)".to_string()
                                } else {
                                    report.exports.join(", ")
                                });
                                ui.end_row();

                                ui.label(RichText::new("インポート DLL:").weak());
                                ui.monospace(if report.imported_dlls.is_empty() {
                                    "外部Win32 GUI依存なし (純粋構造体)".to_string()
                                } else {
                                    report.imported_dlls.join(", ")
                                });
                                ui.end_row();

                                ui.label(RichText::new("抽出パラメータ総数:").weak());
                                ui.label(format!(
                                    "{} エントリ / トラックバー {} 本 / チェックボックス {} 個 / ダイアログ項目 {} 個",
                                    desc.entries.len(),
                                    report.total_tracks,
                                    report.total_checks,
                                    report.total_dialogs
                                ));
                                ui.end_row();
                            });

                        // 抽出されたトラックバー・チェックボックス一覧
                        ui.add_space(4.0);
                        egui::CollapsingHeader::new(
                            RichText::new("📋 抽出された FILTER_DLL / FILTER_PLUGIN_TABLE パラメータ定義一覧").strong(),
                        )
                        .default_open(true)
                        .show(ui, |ui| {
                            for (e_idx, entry) in desc.entries.iter().enumerate() {
                                let cat = report
                                    .entry_categories
                                    .get(e_idx)
                                    .map(|(_, c)| c.label())
                                    .unwrap_or("汎用");
                                ui.horizontal(|ui| {
                                    ui.strong(format!("■ {}", entry.display_name));
                                    ui.colored_label(crate::theme::accent_text(), format!("[自動推論: {cat}]"));
                                });
                                for t in &entry.tracks {
                                    ui.small(format!(
                                        "  ・[Track {}] 「{}」 : 最小={} / 最大={} / 初期値={} (精度={})",
                                        t.index, t.name, t.min, t.max, t.default, t.precision
                                    ));
                                }
                                for (ci, c) in entry.checks.iter().enumerate() {
                                    ui.small(format!(
                                        "  ・[Check {}] 「{}」 : 初期値={}",
                                        ci,
                                        c.name,
                                        if c.default { "ON (1)" } else { "OFF (0)" }
                                    ));
                                }
                                for d in &entry.dialogs {
                                    ui.small(format!(
                                        "  ・[Dialog] 「{}」 (var: {}) : 初期値={}",
                                        d.label, d.var_name, d.default_value
                                    ));
                                }
                            }
                        });
                    });

                // 2. ワンクリック変換アクションバー & 生成コード切替タブ
                let generated_code = match self.st.auf_porter.active_tab {
                    crate::state::AufPorterTab::AnmLua => au2_plugin::generate_anm_script(&desc),
                    crate::state::AufPorterTab::WgslShader => au2_plugin::generate_wgsl_shader(&desc),
                    crate::state::AufPorterTab::RustCode => au2_plugin::generate_rust_effect_code(&desc),
                    crate::state::AufPorterTab::JsonSpec => au2_plugin::generate_json_spec(&desc),
                };

                ui.horizontal_wrapped(|ui| {
                    let convert_btn = egui::Button::new(
                        RichText::new("🚀 ワンクリックで .anm スクリプトに変換してプラグイン登録")
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(crate::theme::accent());
                    if ui
                        .add(convert_btn)
                        .on_hover_text("生成されたクロスプラットフォーム .anm (Lua) スクリプトを plugins/ フォルダに書き出し、Mac / Linux / Windows で即座に使えるようにします")
                        .clicked()
                    {
                        let idx = self.st.auf_porter.selected_plugin_idx;
                        self.st.convert_auf_to_anm(idx);
                    }

                    if ui.button("📋 コードをコピー").clicked() {
                        ui.ctx().copy_text(generated_code.clone());
                        self.st.status = "生成された移植コードをクリップボードにコピーしました".into();
                    }

                    if ui.button("💾 ファイルに保存...").clicked() {
                        let stem = std::path::Path::new(&desc.file_name)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("ported_filter");
                        let (ext, filter_name) = match self.st.auf_porter.active_tab {
                            crate::state::AufPorterTab::AnmLua => ("anm", "AviUtl アニメーション効果スクリプト (*.anm)"),
                            crate::state::AufPorterTab::WgslShader => ("wgsl", "WebGPU WGSL シェーダー (*.wgsl)"),
                            crate::state::AufPorterTab::RustCode => ("rs", "Rust ソースコード (*.rs)"),
                            crate::state::AufPorterTab::JsonSpec => ("json", "JSON 仕様書 (*.json)"),
                        };
                        if let Some(save_path) = rfd::FileDialog::new()
                            .set_title("自動生成コードを保存")
                            .add_filter(filter_name, &[ext])
                            .set_directory(&self.st.plugins.plugin_dir)
                            .set_file_name(format!("{stem}_ported.{ext}"))
                            .save_file()
                        {
                            match std::fs::write(&save_path, &generated_code) {
                                Ok(()) => {
                                    if ext == "anm" {
                                        self.st.plugins.reload();
                                    }
                                    self.st.status = format!("移植コードを {} に保存しました", save_path.display());
                                }
                                Err(e) => {
                                    self.st.status = format!("保存に失敗しました: {e}");
                                }
                            }
                        }
                    }
                });

                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut self.st.auf_porter.active_tab,
                        crate::state::AufPorterTab::AnmLua,
                        "✨ 1. クロスプラットフォーム .anm (Lua)",
                    );
                    ui.selectable_value(
                        &mut self.st.auf_porter.active_tab,
                        crate::state::AufPorterTab::WgslShader,
                        "🎨 2. WGSL (WebGPU シェーダー)",
                    );
                    ui.selectable_value(
                        &mut self.st.auf_porter.active_tab,
                        crate::state::AufPorterTab::RustCode,
                        "🦀 3. Rust ネイティブコード",
                    );
                    ui.selectable_value(
                        &mut self.st.auf_porter.active_tab,
                        crate::state::AufPorterTab::JsonSpec,
                        "📄 4. JSON 構造体ダンプ",
                    );
                });

                egui::ScrollArea::vertical()
                    .id_salt("auf_porter_code_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut code_view = generated_code;
                        ui.add(
                            egui::TextEdit::multiline(&mut code_view)
                                .font(egui::TextStyle::Monospace)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(16),
                        );
                    });
            });

        self.st.auf_porter.is_open = is_open;
    }
}
