//! アプリケーション本体：メニュー・ドッキング・ウィンドウモード・プラグイン管理・ショートカット。

use std::path::PathBuf;

use au2_plugin::SupportStatus;
use eframe::egui::{
    self, vec2, Color32, Key, KeyboardShortcut, Modifiers, RichText, Stroke, ViewportBuilder, ViewportClass, ViewportId,
};
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};

use crate::panels;
use crate::state::{EditorState, SAMPLE_EXO};

/// ドッキング可能なパネル。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Sources,
    Preview,
    Timeline,
    Settings,
}

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Tab::Sources => "ソース（素材・プラグイン）",
            Tab::Preview => "プレビュー",
            Tab::Timeline => "タイムライン（拡張編集）",
            Tab::Settings => "設定ダイアログ",
        }
    }
}

/// ウィンドウ構成。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// シングルウィンドウ（ドッキング）
    Docked,
    /// AviUtl 風の分離ウィンドウ（メイン＝プレビュー、ソース・タイムライン・設定ダイアログは別 OS ウィンドウ）
    Separated,
}

pub struct OpenAviApp {
    dock: DockState<Tab>,
    layout: LayoutMode,
    st: EditorState,
    gpu_info: String,
    show_plugins: bool,
    show_about: bool,
    show_unsaved_confirm: bool,
    last_title: String,
}

fn default_dock() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Preview]);
    let surface = dock.main_surface_mut();
    let [top, _timeline] = surface.split_below(NodeIndex::root(), 0.60, vec![Tab::Timeline]);
    let [center_left, _settings] = surface.split_right(top, 0.72, vec![Tab::Settings]);
    let [_sources, _preview] = surface.split_left(center_left, 0.30, vec![Tab::Sources]);
    dock
}

const SC_OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SC_IMPORT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::I);
const SC_SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);

impl OpenAviApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> Self {
        crate::fonts::install_japanese_font(&cc.egui_ctx);

        // プロ向け動画編集ソフトに最適化した洗練されたダークテーマ
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(20, 21, 25);
        visuals.window_fill = Color32::from_rgb(26, 28, 34);
        visuals.extreme_bg_color = Color32::from_rgb(13, 14, 17);
        visuals.faint_bg_color = Color32::from_rgb(28, 30, 36);
        visuals.selection.bg_fill = Color32::from_rgb(37, 99, 235);
        visuals.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(96, 165, 250));

        let rounding = egui::Rounding::same(4.0);
        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(24, 25, 30);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_gray(38));
        visuals.widgets.noninteractive.rounding = rounding;

        visuals.widgets.inactive.bg_fill = Color32::from_rgb(34, 37, 46);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_gray(48));
        visuals.widgets.inactive.rounding = rounding;

        visuals.widgets.hovered.bg_fill = Color32::from_rgb(46, 52, 65);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(96, 165, 250));
        visuals.widgets.hovered.rounding = rounding;

        visuals.widgets.active.bg_fill = Color32::from_rgb(37, 99, 235);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
        visuals.widgets.active.rounding = rounding;

        visuals.widgets.open.bg_fill = Color32::from_rgb(38, 42, 52);
        visuals.widgets.open.rounding = rounding;
        cc.egui_ctx.set_visuals(visuals);

        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.button_padding = vec2(9.0, 5.0);
        style.spacing.window_margin = egui::Margin::same(8.0);
        cc.egui_ctx.set_style(style);

        let gpu_info = cc
            .wgpu_render_state
            .as_ref()
            .map(|rs| {
                let info = rs.adapter.get_info();
                format!("{} / {:?}", info.name, info.backend)
            })
            .unwrap_or_else(|| "GPU 情報なし".into());
        log::info!("GPU: {gpu_info}");

        let mut st = EditorState::default();
        match initial {
            Some(path) => st.load_path(&path),
            None => st.load_exo(SAMPLE_EXO, None),
        }
        Self {
            dock: default_dock(),
            layout: LayoutMode::Docked,
            st,
            gpu_info,
            show_plugins: false,
            show_about: false,
            show_unsaved_confirm: false,
            last_title: String::new(),
        }
    }
}

/// 全ウィンドウ共通のキーボードショートカット（AviUtl の操作感に準拠）。
fn handle_shortcuts(ctx: &egui::Context, st: &mut EditorState) {
    if ctx.input_mut(|i| i.consume_shortcut(&SC_OPEN)) {
        st.smart_open_dialog();
    }
    if ctx.input_mut(|i| i.consume_shortcut(&SC_IMPORT)) {
        let layer = st.timeline.context_target.layer;
        let frame = st.frame;
        st.add_media_dialog(layer, frame);
    }
    if ctx.input_mut(|i| i.consume_shortcut(&SC_SAVE)) {
        st.save_project_dialog();
    }
    if ctx.wants_keyboard_input() {
        return;
    }
    ctx.input(|i| {
        let big = if i.modifiers.shift { 10 } else { 1 };
        if i.key_pressed(Key::Space) {
            st.toggle_play();
        }
        if i.key_pressed(Key::ArrowRight) {
            st.step(big);
        }
        if i.key_pressed(Key::ArrowLeft) {
            st.step(-big);
        }
        if i.key_pressed(Key::Home) {
            st.set_frame(0);
        }
        if i.key_pressed(Key::End) {
            let last = st.last_frame();
            st.set_frame(last);
        }
        if i.key_pressed(Key::Delete) {
            if let Some(id) = st.selected.take() {
                if st.scene.remove_object(id).is_some() {
                    st.dirty = true;
                    st.status = "選択オブジェクトを削除しました".into();
                }
            }
        }
    });
}

fn panel_ui(tab: Tab, ui: &mut egui::Ui, st: &mut EditorState) {
    match tab {
        Tab::Sources => panels::sources::ui(ui, st),
        Tab::Preview => panels::preview::ui(ui, st),
        Tab::Timeline => panels::timeline::ui(ui, st),
        Tab::Settings => panels::settings::ui(ui, st),
    }
}

struct Viewer<'a> {
    st: &'a mut EditorState,
}

impl TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        panel_ui(*tab, ui, self.st);
    }

    fn closeable(&mut self, _tab: &mut Tab) -> bool {
        false
    }

    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        [false, false]
    }
}

impl eframe::App for OpenAviApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ウィンドウ閉じる要求 (Alt+F4 や [X] ボタン) を検知し、未保存の変更がある場合は確認ダイアログを表示
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.st.dirty {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.show_unsaved_confirm = true;
            }
        }

        // タイトルバーにファイル名と編集状態 (*) を反映
        let doc_name = self
            .st
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "無題".to_string());
        let current_title = format!("OpenAvi - [{}{}]", doc_name, if self.st.dirty { " *" } else { "" });
        if current_title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(current_title.clone()));
            self.last_title = current_title;
        }

        // ドラッグ＆ドロップで .aup / .exo / メディア / プラグインを一括処理
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        for path in dropped {
            self.st.load_path(&path);
        }

        handle_shortcuts(ctx, &mut self.st);
        self.st.tick();
        if self.st.playing {
            ctx.request_repaint();
        }

        self.menu_bar(ctx);
        self.status_bar(ctx);

        match self.layout {
            LayoutMode::Docked => {
                egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
                    DockArea::new(&mut self.dock)
                        .style(Style::from_egui(ctx.style().as_ref()))
                        .show_inside(ui, &mut Viewer { st: &mut self.st });
                });
            }
            LayoutMode::Separated => {
                egui::CentralPanel::default().show(ctx, |ui| panels::preview::ui(ui, &mut self.st));
                if self.separated_windows(ctx) {
                    self.layout = LayoutMode::Docked;
                }
            }
        }

        self.plugins_window(ctx);

        if self.show_about {
            egui::Window::new("OpenAvi について")
                .open(&mut self.show_about)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.heading("OpenAvi");
                    ui.label(format!("バージョン {}", env!("CARGO_PKG_VERSION")));
                    ui.label("AviUtl 互換の次世代 64bit オープンソース動画編集ソフトウェア");
                    ui.separator();
                    ui.label(format!("GPU レンダラ: {}", self.gpu_info));
                    ui.label("ライセンス: MIT License");
                });
        }

        // 保存確認モーダルダイアログ
        self.unsaved_confirm_dialog(ctx);
    }
}

impl OpenAviApp {
    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("ファイル", |ui| {
                    if ui.button("📄 新規プロジェクト").clicked() {
                        self.st.new_scene();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add(
                            egui::Button::new("📂 ファイルを開く / 素材・プラグインを一括取込 (自動判別)...")
                                .shortcut_text(ctx.format_shortcut(&SC_OPEN)),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.st.smart_open_dialog();
                    }
                    if ui
                        .add(
                            egui::Button::new("🎬 動画・画像・音声をタイムラインに追加...")
                                .shortcut_text(ctx.format_shortcut(&SC_IMPORT)),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        let layer = self.st.timeline.context_target.layer;
                        let frame = self.st.frame;
                        self.st.add_media_dialog(layer, frame);
                    }
                    if ui.button("🧩 プラグイン・スクリプトをインストール (.anm/.obj/.auf等)...").clicked() {
                        ui.close_menu();
                        self.st.install_plugin_dialog();
                    }
                    ui.separator();
                    if ui.button("📁 プロジェクトを開く (.aup2 / .aup / .exo)...").clicked() {
                        ui.close_menu();
                        self.st.open_project_dialog();
                    }
                    if ui
                        .add(
                            egui::Button::new("💾 プロジェクトを保存 (.aup2 / .aup)...")
                                .shortcut_text(ctx.format_shortcut(&SC_SAVE)),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.st.save_project_dialog();
                    }
                    if ui.button("📤 オブジェクトファイルのエクスポート (.exo)...").clicked() {
                        ui.close_menu();
                        self.st.export_exo_dialog();
                    }
                    ui.separator();
                    if ui.button("🔄 サンプルプロジェクトを再読み込み").clicked() {
                        self.st.load_exo(SAMPLE_EXO, None);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("終了").clicked() {
                        if self.st.dirty {
                            self.show_unsaved_confirm = true;
                        } else {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                });
                ui.menu_button("表示", |ui| {
                    ui.checkbox(
                        &mut self.st.timeline.show_clip_previews,
                        "🖼 タイムラインのクリップ内プレビューを表示",
                    );
                    ui.separator();
                    ui.radio_value(&mut self.layout, LayoutMode::Docked, "シングルウィンドウ（ドッキング）");
                    ui.radio_value(&mut self.layout, LayoutMode::Separated, "マルチウィンドウ（AviUtl 風）");
                    ui.separator();
                    if ui.button("レイアウトを初期化").clicked() {
                        self.dock = default_dock();
                        self.layout = LayoutMode::Docked;
                        ui.close_menu();
                    }
                });
                ui.menu_button("プラグイン", |ui| {
                    if ui.button("プラグイン・スクリプト管理一覧...").clicked() {
                        self.st.plugins.reload();
                        self.show_plugins = true;
                        ui.close_menu();
                    }
                    if ui.button("プラグイン・スクリプトをインストール...").clicked() {
                        ui.close_menu();
                        self.st.install_plugin_dialog();
                        self.show_plugins = true;
                    }
                });
                ui.menu_button("再生", |ui| {
                    let label = if self.st.playing { "停止 (Space)" } else { "再生 (Space)" };
                    if ui.button(label).clicked() {
                        self.st.toggle_play();
                        ui.close_menu();
                    }
                    if ui.button("先頭へ (Home)").clicked() {
                        self.st.set_frame(0);
                        ui.close_menu();
                    }
                });
                ui.menu_button("ヘルプ", |ui| {
                    if ui.button("OpenAvi について").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });
            });
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let s = &self.st.scene.settings;
                ui.label(format!(
                    "{} / {}  [{}/{}]",
                    s.frame_rate.timecode(self.st.frame),
                    s.frame_rate.timecode(self.st.last_frame()),
                    self.st.frame + 1,
                    self.st.last_frame() + 1
                ));
                ui.separator();
                ui.label(format!("{}x{} {:.3}fps", s.width, s.height, s.frame_rate.as_f64()));
                ui.separator();
                if !self.st.warnings.is_empty() {
                    ui.colored_label(Color32::YELLOW, format!("⚠ 警告 {}", self.st.warnings.len()))
                        .on_hover_text(self.st.warnings.join("\n"));
                    ui.separator();
                }
                ui.label(&self.st.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(&self.gpu_info);
                });
            });
        });
    }

    fn plugins_window(&mut self, ctx: &egui::Context) {
        if !self.show_plugins {
            return;
        }
        let mut open = self.show_plugins;
        egui::Window::new("プラグイン・スクリプト管理")
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
                                            SupportStatus::Supported => Color32::from_rgb(100, 220, 120),
                                            SupportStatus::PartialScript => Color32::from_rgb(240, 200, 90),
                                            _ => Color32::from_rgb(180, 180, 195),
                                        };
                                        ui.label(RichText::new(p.status.summary()).color(color));
                                        if !p.description.is_empty() {
                                            ui.small(RichText::new(&p.description).weak());
                                        }
                                        if !p.exports.is_empty() {
                                            ui.small(
                                                RichText::new(format!("エクスポートシンボル: {}", p.exports.join(", ")))
                                                    .color(Color32::from_rgb(147, 197, 253)),
                                            );
                                        }
                                        for e in &p.entries {
                                            let tracks: Vec<&str> = e.tracks.iter().map(|t| t.name.as_str()).collect();
                                            ui.small(format!(
                                                "・{} (トラック: [{}])",
                                                e.display_name,
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

    /// 分離モードのサブウィンドウを描画します。いずれかが閉じられたら `true`。
    fn separated_windows(&mut self, ctx: &egui::Context) -> bool {
        let mut closed = false;
        for (tab, size) in [
            (Tab::Sources, [360.0, 640.0]),
            (Tab::Timeline, [1200.0, 380.0]),
            (Tab::Settings, [400.0, 640.0]),
        ] {
            let st = &mut self.st;
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of(("openavi-viewport", tab.title())),
                ViewportBuilder::default().with_title(tab.title()).with_inner_size(size),
                |ctx, class| {
                    if class == ViewportClass::Embedded {
                        egui::Window::new(tab.title()).default_size(size).show(ctx, |ui| panel_ui(tab, ui, st));
                    } else {
                        handle_shortcuts(ctx, st);
                        egui::CentralPanel::default().show(ctx, |ui| panel_ui(tab, ui, st));
                        if ctx.input(|i| i.viewport().close_requested()) {
                            closed = true;
                        }
                    }
                },
            );
        }
        closed
    }

    /// 未保存の変更があるままアプリを終了しようとした際の確認ダイアログ。
    fn unsaved_confirm_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_unsaved_confirm {
            return;
        }

        // 半透明の暗い背景オーバーレイ（画面全体のクリックをガード）
        egui::Area::new(egui::Id::new("unsaved_backdrop"))
            .interactable(true)
            .order(egui::Order::Middle)
            .fixed_pos([0.0, 0.0])
            .show(ctx, |ui| {
                let screen_rect = ctx.screen_rect();
                ui.painter().rect_filled(screen_rect, 0.0, Color32::from_black_alpha(150));
            });

        let mut close_modal = false;
        let mut do_exit = false;

        let doc_name = self
            .st
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "無題のプロジェクト".to_string());

        egui::Window::new("保存の確認")
            .id(egui::Id::new("unsaved_confirm_window"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .fixed_size([440.0, 150.0])
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 12.0);

                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").size(32.0).color(Color32::from_rgb(250, 204, 21)));
                    ui.vertical(|ui| {
                        ui.strong(RichText::new("変更内容が保存されていません").size(15.0));
                        ui.label(format!("「{}」への変更を保存しますか？", doc_name));
                        ui.weak("保存せずに終了すると、編集内容はすべて失われます。");
                    });
                });

                ui.separator();

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // キャンセル
                    if ui.button("キャンセル").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        close_modal = true;
                    }

                    // 保存しない
                    if ui.add(egui::Button::new("保存しない").fill(Color32::from_rgb(180, 50, 50))).clicked() {
                        self.st.dirty = false;
                        close_modal = true;
                        do_exit = true;
                    }

                    // 保存する
                    let save_btn = egui::Button::new(RichText::new("💾 保存する").color(Color32::WHITE).strong())
                        .fill(Color32::from_rgb(37, 99, 235));
                    if ui.add(save_btn).clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                        if self.st.save_project() {
                            close_modal = true;
                            do_exit = true;
                        }
                    }
                });
            });

        if close_modal {
            self.show_unsaved_confirm = false;
        }
        if do_exit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
