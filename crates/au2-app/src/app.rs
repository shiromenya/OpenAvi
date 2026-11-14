//! アプリケーション本体：メニュー・ドッキング・ウィンドウモード・プラグイン管理・ショートカット。

use std::path::PathBuf;

use eframe::egui::{
    self, vec2, Color32, Key, KeyboardShortcut, Modifiers, RichText, ViewportBuilder, ViewportClass, ViewportId,
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
    pub(crate) dock: DockState<Tab>,
    pub(crate) layout: LayoutMode,
    pub(crate) st: EditorState,
    pub(crate) gpu_info: String,
    pub(crate) show_plugins: bool,
    pub(crate) show_about: bool,
    pub(crate) show_shortcuts: bool,
    pub(crate) new_project_modal: NewProjectModalState,
    pub(crate) export_modal: ExportModalState,
    pub(crate) last_title: String,
    pub(crate) logo_texture: Option<egui::TextureHandle>,
    pub(crate) allow_close: bool,
}

pub use crate::dialogs::*;

fn default_dock() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Preview]);
    let surface = dock.main_surface_mut();
    let [top, _timeline] = surface.split_below(NodeIndex::root(), 0.60, vec![Tab::Timeline]);
    let [center_left, _settings] = surface.split_right(top, 0.72, vec![Tab::Settings]);
    let [_sources, _preview] = surface.split_left(center_left, 0.30, vec![Tab::Sources]);
    dock
}

const SC_NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const SC_OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SC_IMPORT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::I);
const SC_SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SC_EXPORT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::E);
const SC_UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const SC_REDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
const SC_REDO_SHIFT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
const SC_SELECT_ALL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::A);
const SC_COPY: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::C);
const SC_PASTE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::V);
const SC_DUPLICATE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::D);
const SC_SPLIT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::K);

impl OpenAviApp {
    /// ヘッドレスのスナップショットテスト用に、ウィンドウ無しでアプリ状態を構築します。
    #[cfg(test)]
    pub(crate) fn for_snapshot(st: EditorState) -> Self {
        Self {
            dock: default_dock(),
            layout: LayoutMode::Docked,
            st,
            gpu_info: "snapshot".into(),
            show_plugins: false,
            show_about: false,
            show_shortcuts: false,
            new_project_modal: NewProjectModalState::default(),
            export_modal: ExportModalState::default(),
            last_title: String::new(),
            logo_texture: None,
            allow_close: false,
        }
    }

    /// スナップショット用に各種ダイアログを開きます（1 = 新規プロジェクト, 2 = 書き出し, 3 = プラグイン管理）。
    #[cfg(test)]
    pub(crate) fn snapshot_open_dialog(&mut self, which: u8) {
        match which {
            1 => self.new_project_modal.is_open = true,
            2 => self.export_modal.open_for_state(&self.st),
            3 => self.show_plugins = true,
            _ => {}
        }
    }

    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> Self {
        crate::fonts::install_japanese_font(&cc.egui_ctx);

        // 保存済みのテーマ（ダーク / ライト）を復元して適用
        crate::theme::init(&cc.egui_ctx);

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
        if let Some(path) = initial {
            st.load_path(&path);
        }
        Self {
            dock: default_dock(),
            layout: LayoutMode::Docked,
            st,
            gpu_info,
            show_plugins: false,
            show_about: false,
            show_shortcuts: false,
            new_project_modal: NewProjectModalState::default(),
            export_modal: ExportModalState::default(),
            last_title: String::new(),
            logo_texture: None,
            allow_close: false,
        }
    }
}

/// 全ウィンドウ共通のキーボードショートカット（AviUtl の操作感に準拠）。
fn handle_shortcuts(ctx: &egui::Context, st: &mut EditorState) {
    if ctx.input_mut(|i| i.consume_shortcut(&SC_UNDO)) {
        st.undo();
    }
    if ctx.input_mut(|i| i.consume_shortcut(&SC_REDO) || i.consume_shortcut(&SC_REDO_SHIFT)) {
        st.redo();
    }
    if ctx.input_mut(|i| i.consume_shortcut(&SC_SELECT_ALL)) {
        st.select_all();
    }
    if ctx.input_mut(|i| i.consume_shortcut(&SC_COPY)) {
        st.copy_selected();
    }
    if ctx.input_mut(|i| i.consume_shortcut(&SC_PASTE)) {
        st.paste_clipboard(None, None);
    }
    if ctx.input_mut(|i| i.consume_shortcut(&SC_DUPLICATE)) {
        st.duplicate_selected();
    }
    if ctx.input_mut(|i| i.consume_shortcut(&SC_SPLIT)) {
        st.split_at_playhead();
    }
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
        if i.key_pressed(Key::Escape) {
            st.clear_selection();
        }
        if i.key_pressed(Key::Delete) {
            st.delete_selected_objects();
        }
        if !i.modifiers.command && !i.modifiers.ctrl && !i.modifiers.alt && i.key_pressed(Key::S) {
            st.timeline.snap_enabled = !st.timeline.snap_enabled;
            st.status = if st.timeline.snap_enabled {
                "スナップ（吸着）を有効にしました".into()
            } else {
                "スナップ（吸着）を無効にしました".into()
            };
        }
        // 選択範囲（In点・Out点）ショートカット
        if !i.modifiers.command && !i.modifiers.ctrl && !i.modifiers.alt {
            if i.key_pressed(Key::I) || i.key_pressed(Key::OpenBracket) {
                st.set_in_point(Some(st.frame));
            }
            if i.key_pressed(Key::O) || i.key_pressed(Key::CloseBracket) {
                st.set_out_point(Some(st.frame));
            }
        }
        if (i.modifiers.alt || i.modifiers.shift) && i.key_pressed(Key::X) {
            st.clear_range();
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

impl Drop for OpenAviApp {
    fn drop(&mut self) {
        self.st.audio.stop();
    }
}

impl eframe::App for OpenAviApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_ui(ctx);
    }
}

impl OpenAviApp {
    /// 1 フレーム分の UI 構築（`eframe::Frame` に依存しないため、ヘッドレスのスナップショットテストからも呼べる）。
    pub(crate) fn update_ui(&mut self, ctx: &egui::Context) {
        // システムテーマ（OS 設定）の変化を追従
        crate::theme::update_system_theme(ctx);

        // ウィンドウ閉じる要求 (Alt+F4 や [X] ボタン) を検知
        if ctx.input(|i| i.viewport().close_requested()) {
            if !self.allow_close {
                if self.st.dirty {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    self.handle_close_request(ctx);
                } else {
                    self.st.audio.stop();
                    self.allow_close = true;
                }
            } else {
                self.st.audio.stop();
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

        // 外部ファイル（メディア・PSD・プロジェクト・プラグイン）のドラッグ＆ドロップ直接配置処理
        let dropped_files: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped_files.is_empty() {
            let pointer_pos = ctx.input(|i| i.pointer.hover_pos().or_else(|| i.pointer.interact_pos()));
            let timeline_target = pointer_pos.zip(self.st.timeline.lanes_rect).filter(|(p, lanes)| lanes.contains(*p));

            if let Some((p, lanes)) = timeline_target {
                let layer_h = if self.st.timeline.show_clip_previews {
                    panels::timeline::LAYER_H_PREVIEW
                } else {
                    panels::timeline::LAYER_H_COMPACT
                };
                let drop_layer = (((p.y - lanes.top() + self.st.timeline.scroll_y) / layer_h).floor().max(0.0) as usize).min(au2_core::MAX_LAYERS - 1);
                let raw_frame = ((p.x - lanes.left()) / self.st.timeline.px_per_frame + self.st.timeline.scroll_x).max(0.0) as au2_core::Frame;
                let drop_frame = panels::timeline::find_snap_frame(&self.st, raw_frame, 12.0, &std::collections::HashSet::new()).unwrap_or(raw_frame);

                let mut current_layer = drop_layer;
                for path in &dropped_files {
                    self.st.load_path_at(path, current_layer, drop_frame);
                    // 動画ファイル（音声ストリームあり）の場合は映像+音声で2レイヤー分消費するため、次のレイヤーへ
                    if let Some(au2_media::MediaKind::Video) = au2_media::MediaKind::from_path(path) {
                        let meta = self.st.media.get_metadata(path);
                        if meta.has_audio {
                            current_layer = (current_layer + 2).min(au2_core::MAX_LAYERS - 1);
                            continue;
                        }
                    }
                    current_layer = (current_layer + 1).min(au2_core::MAX_LAYERS - 1);
                }
                self.st.set_frame(drop_frame);
            } else {
                for path in &dropped_files {
                    self.st.load_path(path);
                }
            }
        }

        if ctx.input_mut(|i| i.consume_shortcut(&SC_NEW)) {
            self.new_project_modal.is_open = true;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SC_EXPORT)) {
            self.export_modal.open_for_state(&self.st);
        }

        handle_shortcuts(ctx, &mut self.st);
        self.st.tick();
        if self.st.playing {
            ctx.request_repaint();
        }

        self.menu_bar(ctx);
        self.status_bar(ctx);
        self.ffmpeg_banner(ctx);
        self.missing_media_banner(ctx);

        match self.layout {
            LayoutMode::Docked => {
                egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
                    let mut dock_style = Style::from_egui(ctx.style().as_ref());
                    dock_style.separator.width = 2.0;
                    dock_style.tab_bar.height = 24.0;
                    dock_style.tab.tab_body.inner_margin = egui::Margin::same(6.0);
                    DockArea::new(&mut self.dock)
                        .style(dock_style)
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
        self.auf_porter_window(ctx);

        if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
            self.show_shortcuts = !self.show_shortcuts;
        }
        self.shortcuts_window(ctx);

        if self.show_about {
            let logo_tex = self.ensure_logo_texture(ctx);
            egui::Window::new("OpenAvi について")
                .frame(egui::Frame::window(&ctx.style()).fill(crate::theme::surface()))
                .open(&mut self.show_about)
                .collapsible(false)
                .resizable(false)
                .default_width(320.0)
                .show(ctx, |ui| {
                    if let Some(tex) = &logo_tex {
                        let aspect = tex.aspect_ratio();
                        let display_w = 260.0f32;
                        let display_h = (display_w / aspect).round();
                        ui.vertical_centered(|ui| {
                            ui.image((tex.id(), egui::vec2(display_w, display_h)));
                        });
                        ui.add_space(8.0);
                    } else {
                        ui.heading("OpenAvi");
                    }
                    ui.label(format!("バージョン {}", env!("CARGO_PKG_VERSION")));
                    ui.label("AviUtl 互換の次世代 64bit オープンソース動画編集ソフトウェア");
                    ui.separator();
                    ui.label(format!("GPU レンダラ: {}", self.gpu_info));
                    ui.label("ライセンス: MIT License");
                });
        }

        // 新規プロジェクト事前設定モーダルダイアログ
        self.new_project_dialog(ctx);
        // 動画・音声の書き出しモーダルダイアログ
        self.export_dialog(ctx);
    }
}

const LOGO_PNG: &[u8] = include_bytes!("../../../assets/openavi_logo.png");

impl OpenAviApp {
    fn ensure_logo_texture(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        if self.logo_texture.is_none() {
            if let Ok(img) = image::load_from_memory(LOGO_PNG) {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                self.logo_texture = Some(ctx.load_texture("openavi_logo", color_image, egui::TextureOptions::LINEAR));
            }
        }
        self.logo_texture.clone()
    }

    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                // Word 風の自動保存トグルスイッチ (メニュー左端)
                ui.horizontal(|ui| {
                    ui.add_space(2.0);
                    toggle_switch_ui(ui, &mut self.st.autosave.enabled);
                    let label_color = if self.st.autosave.enabled {
                        crate::theme::accent_text()
                    } else {
                        crate::theme::text_muted()
                    };
                    let text = if self.st.autosave.enabled {
                        RichText::new("自動保存: ON").color(label_color).strong().size(12.0)
                    } else {
                        RichText::new("自動保存: OFF").color(label_color).size(12.0)
                    };
                    let lbl = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                    if lbl.clicked() {
                        self.st.autosave.enabled = !self.st.autosave.enabled;
                    }
                    lbl.on_hover_text("有効時: レイヤーやオブジェクトの変更があるたびに、プロジェクトファイルへ自動上書き保存されます。");
                    ui.add_space(4.0);
                });
                ui.separator();

                ui.menu_button("ファイル", |ui| {
                    if ui
                        .add(
                            egui::Button::new("新規プロジェクト...")
                                .shortcut_text(ctx.format_shortcut(&SC_NEW)),
                        )
                        .clicked()
                    {
                        self.new_project_modal.is_open = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add(
                            egui::Button::new("ファイルを開く / 素材・プラグインを一括取込 (自動判別)...")
                                .shortcut_text(ctx.format_shortcut(&SC_OPEN)),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.st.smart_open_dialog();
                    }
                    if ui
                        .add(
                            egui::Button::new("動画・画像・音声をタイムラインに追加...")
                                .shortcut_text(ctx.format_shortcut(&SC_IMPORT)),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        let layer = self.st.timeline.context_target.layer;
                        let frame = self.st.frame;
                        self.st.add_media_dialog(layer, frame);
                    }
                    if ui.button("プラグイン・スクリプトをインストール (.anm/.obj/.auf等)...").clicked() {
                        ui.close_menu();
                        self.st.install_plugin_dialog();
                    }
                    ui.separator();
                    if ui.button("プロジェクトを開く (.aup2 / .aup / .exo)...").clicked() {
                        ui.close_menu();
                        self.st.open_project_dialog();
                    }
                    if ui
                        .add(
                            egui::Button::new("プロジェクトを保存 (.aup2 / .aup)...")
                                .shortcut_text(ctx.format_shortcut(&SC_SAVE)),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.st.save_project_dialog();
                    }
                    if ui.button("オブジェクトファイルのエクスポート (.exo)...").clicked() {
                        ui.close_menu();
                        self.st.export_exo_dialog();
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("動画・音声の書き出し (MP4 / AVI / GIF / WAV)...")
                                    .strong()
                                    .color(crate::theme::accent_text()),
                            )
                            .shortcut_text(ctx.format_shortcut(&SC_EXPORT)),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.export_modal.open_for_state(&self.st);
                    }
                    ui.separator();
                    if ui.button("サンプルプロジェクトを再読み込み").clicked() {
                        self.st.load_exo(SAMPLE_EXO, None);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("終了").clicked() {
                        ui.close_menu();
                        self.handle_close_request(ctx);
                    }
                });
                ui.menu_button("編集", |ui| {
                    let undo_enabled = self.st.can_undo();
                    let undo_text = if let Some(desc) = self.st.undo_description() {
                        format!("↶ 元に戻す: {desc}")
                    } else {
                        "↶ 元に戻す".to_string()
                    };
                    if ui
                        .add_enabled(
                            undo_enabled,
                            egui::Button::new(undo_text).shortcut_text(ctx.format_shortcut(&SC_UNDO)),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.st.undo();
                    }

                    let redo_enabled = self.st.can_redo();
                    let redo_text = if let Some(desc) = self.st.redo_description() {
                        format!("↷ やり直す: {desc}")
                    } else {
                        "↷ やり直す".to_string()
                    };
                    if ui
                        .add_enabled(
                            redo_enabled,
                            egui::Button::new(redo_text).shortcut_text(ctx.format_shortcut(&SC_REDO)),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.st.redo();
                    }

                    ui.separator();

                    if ui
                        .add(
                            egui::Button::new("すべて選択")
                                .shortcut_text(ctx.format_shortcut(&SC_SELECT_ALL)),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.st.select_all();
                    }

                    let has_selection = self.st.selected.is_some() || !self.st.selected_objects.is_empty();
                    if ui
                        .add_enabled(has_selection, egui::Button::new("選択解除").shortcut_text("Esc"))
                        .clicked()
                    {
                        ui.close_menu();
                        self.st.clear_selection();
                    }

                    if ui
                        .add_enabled(
                            has_selection,
                            egui::Button::new("選択オブジェクトを削除").shortcut_text("Delete"),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.st.delete_selected_objects();
                    }

                    ui.separator();

                    if ui
                        .add_enabled(
                            has_selection,
                            egui::Button::new("コピー").shortcut_text(ctx.format_shortcut(&SC_COPY)),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.st.copy_selected();
                    }

                    let can_paste = !self.st.clipboard.is_empty();
                    if ui
                        .add_enabled(
                            can_paste,
                            egui::Button::new("貼り付け").shortcut_text(ctx.format_shortcut(&SC_PASTE)),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.st.paste_clipboard(None, None);
                    }

                    if ui
                        .add_enabled(
                            has_selection,
                            egui::Button::new("複製").shortcut_text(ctx.format_shortcut(&SC_DUPLICATE)),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.st.duplicate_selected();
                    }

                    if ui
                        .add(
                            egui::Button::new("再生ヘッド位置で分割").shortcut_text(ctx.format_shortcut(&SC_SPLIT)),
                        )
                        .clicked()
                    {
                        ui.close_menu();
                        self.st.split_at_playhead();
                    }

                    ui.separator();

                    if ui.checkbox(&mut self.st.timeline.snap_enabled, "スナップ（吸着）を有効化").clicked() {
                        self.st.status = if self.st.timeline.snap_enabled {
                            "スナップ（吸着）を有効にしました".into()
                        } else {
                            "スナップ（吸着）を無効にしました".into()
                        };
                    }
                });
                ui.menu_button("表示", |ui| {
                    ui.checkbox(
                        &mut self.st.timeline.show_clip_previews,
                        "タイムラインのクリップ内プレビューを表示",
                    );
                    ui.separator();
                    ui.radio_value(&mut self.layout, LayoutMode::Docked, "シングルウィンドウ（ドッキング）");
                    ui.radio_value(&mut self.layout, LayoutMode::Separated, "マルチウィンドウ（AviUtl 風）");
                    ui.separator();
                    let cur_pref = crate::theme::preference();
                    let mut sel_pref = cur_pref;
                    ui.radio_value(&mut sel_pref, crate::theme::ThemePreference::Dark, "ダークモード");
                    ui.radio_value(&mut sel_pref, crate::theme::ThemePreference::Light, "ライトモード");
                    ui.radio_value(&mut sel_pref, crate::theme::ThemePreference::System, "システム設定に従う");
                    if sel_pref != cur_pref {
                        crate::theme::set_preference(ctx, sel_pref);
                    }
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
                    ui.separator();
                    if ui
                        .button(".auf パラメータ抽出・雛形生成 (開発者向け)...")
                        .clicked()
                    {
                        self.st.open_auf_porter(None);
                        ui.close_menu();
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
                    if ui.button("⌨ ショートカット一覧 (F1)").clicked() {
                        self.show_shortcuts = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("OpenAvi について").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });

                // マルチシーン切り替えセレクタ（Root / Scene 1..N）
                if self.st.scenes.len() > 1 {
                    ui.separator();
                    let cur_name = self.st.scene.name.clone();
                    let scene_list: Vec<(u32, String, usize)> = self
                        .st
                        .scenes
                        .iter()
                        .map(|(&id, sc)| (id, sc.name.clone(), sc.object_count()))
                        .collect();
                    let mut switch_to: Option<u32> = None;
                    ui.menu_button(
                        egui::RichText::new(format!("シーン: {}", cur_name))
                            .strong()
                            .color(crate::theme::accent_text()),
                        |ui| {
                            for (sc_id, sc_name, cnt) in scene_list {
                                let is_cur = sc_name == cur_name;
                                if ui
                                    .selectable_label(is_cur, format!("{} ({} オブジェクト)", sc_name, cnt))
                                    .clicked()
                                {
                                    switch_to = Some(sc_id);
                                    ui.close_menu();
                                }
                            }
                        },
                    );
                    if let Some(target_id) = switch_to {
                        // 現在のシーンの変更を scenes マップに書き戻してから切り替え
                        for sc in self.st.scenes.values_mut() {
                            if sc.name == cur_name {
                                *sc = self.st.scene.clone();
                                break;
                            }
                        }
                        if let Some(next_sc) = self.st.scenes.get(&target_id).cloned() {
                            self.st.scene = next_sc;
                            self.st.selected = None;
                            self.st.set_frame(0);
                            self.st.status = format!("シーン「{}」に切り替えました", self.st.scene.name);
                        }
                    }
                }

                // メニューバー右端に「🎬 動画書き出し」と「🌙 / ☀ テーマ切り替え」ボタンを配置
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    let exp_btn = egui::Button::new(
                        RichText::new("動画書き出し")
                            .color(Color32::WHITE)
                            .strong()
                            .size(12.5),
                    )
                    .min_size(vec2(110.0, 26.0))
                    .fill(crate::theme::accent());
                    if ui
                        .add(exp_btn)
                        .on_hover_text("現在のシーンを動画 (MP4 / AVI / MOV / WebM / GIF) または音声 (WAV) として書き出します (Ctrl+E)")
                        .clicked()
                    {
                        self.export_modal.open_for_state(&self.st);
                    }

                    ui.add_space(6.0);
                    let (theme_label, hover_text) = match crate::theme::preference() {
                        crate::theme::ThemePreference::Dark => ("表示: ダーク", "クリックでライトモードに切り替えます"),
                        crate::theme::ThemePreference::Light => ("表示: ライト", "クリックでシステム設定に切り替えます"),
                        crate::theme::ThemePreference::System => {
                            if crate::theme::is_dark() {
                                ("表示: 自動", "システム設定（現在: ダーク）に追従中。クリックでダークモードに固定します")
                            } else {
                                ("表示: 自動", "システム設定（現在: ライト）に追従中。クリックでダークモードに固定します")
                            }
                        }
                    };
                    let next_pref = crate::theme::preference().cycled();
                    let theme_btn = egui::Button::new(RichText::new(theme_label).size(11.5))
                        .min_size(vec2(78.0, 24.0));
                    if ui
                        .add(theme_btn)
                        .on_hover_text(hover_text)
                        .clicked()
                    {
                        crate::theme::set_preference(ctx, next_pref);
                    }
                });
            });
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let s = &self.st.scene.settings;
                let last = self.st.last_frame();
                let digits = ((last + 1).max(1) as f64).log10().floor() as usize + 1;
                ui.monospace(format!(
                    "{} / {}  [{:>w$}/{}]",
                    s.frame_rate.timecode(self.st.frame),
                    s.frame_rate.timecode(last),
                    self.st.frame + 1,
                    last + 1,
                    w = digits
                ));
                ui.separator();
                ui.monospace(format!("{}x{} {:.3}fps", s.width, s.height, s.frame_rate.as_f64()));
                ui.separator();
                if !self.st.warnings.is_empty() {
                    ui.colored_label(crate::theme::warn_text(), format!("⚠ 警告 {}", self.st.warnings.len()))
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

    fn ffmpeg_banner(&mut self, ctx: &egui::Context) {
        self.st.poll_ffmpeg_download();
        let running = matches!(self.st.ffmpeg_dl, crate::state::FfmpegDownload::Running(_));
        if running {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if !running && !self.st.needs_ffmpeg() {
            return;
        }
        let failed = match &self.st.ffmpeg_dl {
            crate::state::FfmpegDownload::Failed(e) => Some(e.clone()),
            _ => None,
        };

        let (banner_bg, banner_stroke) = crate::theme::warn_banner();
        egui::TopBottomPanel::top("ffmpeg_banner")
            .frame(
                egui::Frame::none()
                    .fill(banner_bg)
                    .stroke(egui::Stroke::new(1.0_f32, banner_stroke))
                    .inner_margin(egui::Margin::symmetric(12.0, 6.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if running {
                        ui.spinner();
                        ui.colored_label(
                            crate::theme::warn_text(),
                            "FFmpeg をダウンロード中です（約 80MB）。完了すると動画・音声が自動で使えるようになります...",
                        );
                    } else {
                        ui.colored_label(
                            crate::theme::warn_text(),
                            "⚠ 動画・音声のデコーダー (FFmpeg) が見つかりません。動画の映像と MP3 等の音声を再生するには FFmpeg が必要です",
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let dl_btn = egui::Button::new(
                                egui::RichText::new("FFmpeg を自動ダウンロード")
                                    .color(Color32::WHITE)
                                    .strong(),
                            )
                            .fill(crate::theme::accent());
                            let tip_dir = au2_media::user_ffmpeg_dir()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| "OpenAvi/bin".into());
                            if ui
                                .add(dl_btn)
                                .on_hover_text(format!("お使いの OS 向けの FFmpeg を自動ダウンロードし {tip_dir} に導入します"))
                                .clicked()
                            {
                                self.st.start_ffmpeg_download();
                            }
                            if let Some(e) = &failed {
                                ui.colored_label(crate::theme::danger_text(), format!("失敗: {e}"));
                            }
                        });
                    }
                });
            });
    }

    fn missing_media_banner(&mut self, ctx: &egui::Context) {
        let missing = self.st.check_missing_media();
        if missing.is_empty() {
            return;
        }

        let first = missing[0].clone();
        let count = missing.len();

        let (banner_bg, banner_stroke) = crate::theme::error_banner();
        egui::TopBottomPanel::top("missing_media_banner")
            .frame(
                egui::Frame::none()
                    .fill(banner_bg)
                    .stroke(egui::Stroke::new(1.0_f32, banner_stroke))
                    .inner_margin(egui::Margin::symmetric(12.0, 6.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        crate::theme::danger_text(),
                        format!(
                            "⚠ プロジェクト内のメディア（{}）が見つかりません (リンク切れ: {} 件) - 「{}」",
                            first.kind_name,
                            count,
                            first.original_path
                        ),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let is_audio = first.kind_name.contains("音声");
                        let relink_btn = egui::Button::new(
                            egui::RichText::new("素材フォルダを指定して一括割り当て...")
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(crate::theme::accent());
                        if ui
                            .add(relink_btn)
                            .on_hover_text(
                                "AviUtl のプロジェクト(.aup)は音声・画像・動画の本体を含まず、元の PC 上のパスだけを保存しています。\n素材が入ったフォルダを指定すると、ファイル名から全ての素材を自動で割り当てます。",
                            )
                            .clicked()
                        {
                            self.st.relink_from_folder_dialog();
                        }
                        let single_label = if first.kind_name.contains("画像") {
                            "このファイルだけ選択..."
                        } else if is_audio {
                            "このファイルだけ選択..."
                        } else {
                            "このファイルだけ選択..."
                        };
                        if ui.button(single_label).clicked() {
                            self.st.relink_media_dialog(None, &first.original_path, is_audio);
                        }
                    });
                });
            });
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

    /// アプリケーション終了要求時の保存確認処理（OS 標準のメッセージボックスを表示）。
    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if !self.st.dirty {
            self.st.audio.stop();
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let doc_name = self
            .st
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "無題のプロジェクト".to_string());

        match show_native_unsaved_confirm(&doc_name) {
            ConfirmResult::Save => {
                if self.st.save_project() {
                    self.st.audio.stop();
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            ConfirmResult::DontSave => {
                self.st.dirty = false;
                self.st.audio.stop();
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ConfirmResult::Cancel => {
                // 保存も終了もせず、編集作業を継続
            }
        }
    }

}
