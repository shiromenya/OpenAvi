//! アプリケーション本体：メニュー・ドッキング・ウィンドウモード・プラグイン管理・ショートカット。

use std::path::PathBuf;

use au2_plugin::SupportStatus;
use eframe::egui::{
    self, vec2, Color32, Key, KeyboardShortcut, Modifiers, RichText, Stroke, ViewportBuilder, ViewportClass, ViewportId,
};
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};

use crate::export::{ExportConfig, ExportFormat, ExportQuality, ExportSession};
use crate::panels;
use crate::state::{
    default_projects_dir, EditorState, NewProjectConfig, ProjectFormat, ProjectPreset, SAMPLE_EXO,
};

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
    new_project_modal: NewProjectModalState,
    export_modal: ExportModalState,
    last_title: String,
}

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
        cfg.include_audio = self.config.include_audio;
        if !self.use_content_range {
            cfg.end_frame = st.last_frame();
        }
        self.config = cfg;
        self.session = None;
        self.finished_path = None;
        self.error_message = None;
        self.is_open = true;
    }
}

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

/// Word / モダン OS 風の美しいカプセル型トグルスイッチ UI。
fn toggle_switch_ui(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let desired_size = vec2(32.0, 18.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
        let bg_fill = if *on {
            Color32::from_rgb(37, 99, 235) // Office / Word 風の鮮やかなブルー
        } else {
            Color32::from_rgb(60, 64, 75)
        };
        let stroke_color = if response.hovered() {
            Color32::from_rgb(147, 197, 253)
        } else {
            Color32::from_gray(80)
        };
        ui.painter().rect(
            rect,
            rect.height() * 0.5,
            bg_fill,
            Stroke::new(1.0_f32, stroke_color),
        );
        let radius = 0.5 * rect.height() - 2.5;
        let x = egui::lerp(
            (rect.left() + radius + 3.0)..=(rect.right() - radius - 3.0),
            how_on,
        );
        let center = egui::pos2(x, rect.center().y);
        ui.painter().circle_filled(center, radius, Color32::WHITE);
    }
    response
}

/// OS 標準の保存確認ダイアログの選択結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmResult {
    /// 変更を保存して終了
    Save,
    /// 変更を保存せずに終了
    DontSave,
    /// キャンセル（終了を中止して作業を継続）
    Cancel,
}

/// Windows / OS 標準の保存確認ダイアログ（「保存して終了」「保存せず終了」「キャンセル」）を表示します。
#[cfg(windows)]
pub fn show_native_unsaved_confirm(doc_name: &str) -> ConfirmResult {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicIsize, Ordering};

    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: isize, lpText: *const u16, lpCaption: *const u16, uType: u32) -> i32;
        fn SetWindowsHookExW(
            idHook: i32,
            lpfn: unsafe extern "system" fn(i32, usize, isize) -> isize,
            hmod: isize,
            dwThreadId: u32,
        ) -> isize;
        fn UnhookWindowsHookEx(hhk: isize) -> i32;
        fn CallNextHookEx(hhk: isize, nCode: i32, wParam: usize, lParam: isize) -> isize;
        fn SetDlgItemTextW(hDlg: isize, nIDDlgItem: i32, lpString: *const u16) -> i32;
        fn GetDlgItem(hDlg: isize, nIDDlgItem: i32) -> isize;
        fn GetWindowRect(hWnd: isize, lpRect: *mut [i32; 4]) -> i32;
        fn SetWindowPos(hWnd: isize, hWndInsertAfter: isize, X: i32, Y: i32, cx: i32, cy: i32, uFlags: u32) -> i32;
        fn MapWindowPoints(hWndFrom: isize, hWndTo: isize, lpPoints: *mut [i32; 2], cPoints: u32) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThreadId() -> u32;
    }

    static HOOK: AtomicIsize = AtomicIsize::new(0);

    unsafe extern "system" fn cbt_hook_proc(n_code: i32, w_param: usize, l_param: isize) -> isize {
        const HCBT_ACTIVATE: i32 = 5;
        const IDYES: i32 = 6;
        const IDNO: i32 = 7;
        const IDCANCEL: i32 = 2;
        const SWP_NOZORDER: u32 = 0x0004;
        const SWP_NOACTIVATE: u32 = 0x0010;

        if n_code == HCBT_ACTIVATE {
            let hwnd = w_param as isize;

            let to_w = |s: &str| -> Vec<u16> {
                OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
            };

            let s_yes = to_w("保存して終了(&S)");
            let s_no = to_w("保存せず終了(&N)");
            let s_cancel = to_w("キャンセル");

            // ボタンの文面を確実に「保存して終了」「保存せず終了」「キャンセル」に変更
            SetDlgItemTextW(hwnd, IDYES, s_yes.as_ptr());
            SetDlgItemTextW(hwnd, IDNO, s_no.as_ptr());
            SetDlgItemTextW(hwnd, IDCANCEL, s_cancel.as_ptr());

            // ボタン幅を 112px に拡張して日本語テキストが綺麗に収まるよう等間隔に配置
            let h_yes = GetDlgItem(hwnd, IDYES);
            let h_no = GetDlgItem(hwnd, IDNO);
            let h_cancel = GetDlgItem(hwnd, IDCANCEL);

            if h_yes != 0 && h_no != 0 && h_cancel != 0 {
                let mut r_dlg = [0i32; 4];
                let mut r_yes = [0i32; 4];
                let mut r_cancel = [0i32; 4];
                GetWindowRect(hwnd, &mut r_dlg);
                GetWindowRect(h_yes, &mut r_yes);
                GetWindowRect(h_cancel, &mut r_cancel);

                let dlg_width = r_dlg[2] - r_dlg[0];
                let btn_w = 112i32;
                let btn_h = (r_yes[3] - r_yes[1]).max(26);
                let gap = 10i32;
                let total_btns_w = btn_w * 3 + gap * 2;

                let required_dlg_w = (total_btns_w + 50).max(dlg_width);
                if required_dlg_w > dlg_width {
                    let dlg_h = r_dlg[3] - r_dlg[1];
                    SetWindowPos(hwnd, 0, r_dlg[0] - (required_dlg_w - dlg_width) / 2, r_dlg[1], required_dlg_w, dlg_h, SWP_NOZORDER | SWP_NOACTIVATE);
                }

                let mut pt_yes = [r_yes[0], r_yes[1]];
                MapWindowPoints(0, hwnd, &mut pt_yes, 1);
                let btn_y = pt_yes[1];

                let mut pt_cancel = [r_cancel[2], r_cancel[1]];
                MapWindowPoints(0, hwnd, &mut pt_cancel, 1);
                let right_edge = pt_cancel[0].max(required_dlg_w - 24);

                let x_cancel = right_edge - btn_w;
                let x_no = x_cancel - gap - btn_w;
                let x_yes = x_no - gap - btn_w;

                SetWindowPos(h_yes, 0, x_yes, btn_y, btn_w, btn_h, SWP_NOZORDER | SWP_NOACTIVATE);
                SetWindowPos(h_no, 0, x_no, btn_y, btn_w, btn_h, SWP_NOZORDER | SWP_NOACTIVATE);
                SetWindowPos(h_cancel, 0, x_cancel, btn_y, btn_w, btn_h, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }

        let hhk = HOOK.load(Ordering::SeqCst);
        CallNextHookEx(hhk, n_code, w_param, l_param)
    }

    let to_wide = |s: &str| -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    };

    let title = to_wide("OpenAvi");
    let instruction = to_wide(&format!("「{doc_name}」への変更内容を保存しますか？\n\n保存せずに終了すると、編集内容はすべて失われます。"));

    const WH_CBT: i32 = 5;
    const MB_YESNOCANCEL: u32 = 0x00000003;
    const MB_ICONWARNING: u32 = 0x00000030;
    const MB_TOPMOST: u32 = 0x00040000;
    const IDYES: i32 = 6;
    const IDNO: i32 = 7;

    // 現在のスレッドに CBT フックを設置してボタンテキストを確実に変更
    let thread_id = unsafe { GetCurrentThreadId() };
    let hhk = unsafe { SetWindowsHookExW(WH_CBT, cbt_hook_proc, 0, thread_id) };
    HOOK.store(hhk, Ordering::SeqCst);

    let ret = unsafe { MessageBoxW(0, instruction.as_ptr(), title.as_ptr(), MB_YESNOCANCEL | MB_ICONWARNING | MB_TOPMOST) };

    if hhk != 0 {
        unsafe { UnhookWindowsHookEx(hhk) };
        HOOK.store(0, Ordering::SeqCst);
    }

    match ret {
        IDYES => ConfirmResult::Save,
        IDNO => ConfirmResult::DontSave,
        _ => ConfirmResult::Cancel,
    }
}

#[cfg(not(windows))]
pub fn show_native_unsaved_confirm(doc_name: &str) -> ConfirmResult {
    let res = rfd::MessageDialog::new()
        .set_title("OpenAvi")
        .set_description(&format!("「{doc_name}」への変更内容を保存しますか？"))
        .set_buttons(rfd::MessageButtons::YesNoCancel)
        .show();
    match res {
        rfd::MessageDialogResult::Yes => ConfirmResult::Save,
        rfd::MessageDialogResult::No => ConfirmResult::DontSave,
        _ => ConfirmResult::Cancel,
    }
}

/// 残り秒数を「1時間02分」「3分05秒」「12秒」の形式に整形します。
fn format_remaining(secs: f32) -> String {
    let s = secs.max(0.0).ceil() as u64;
    if s >= 3600 {
        format!("{}時間{:02}分", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}分{:02}秒", s / 60, s % 60)
    } else {
        format!("{s}秒")
    }
}

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
            new_project_modal: NewProjectModalState::default(),
            export_modal: ExportModalState::default(),
            last_title: String::new(),
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
            new_project_modal: NewProjectModalState::default(),
            export_modal: ExportModalState::default(),
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
        self.update_ui(ctx);
    }
}

impl OpenAviApp {
    /// 1 フレーム分の UI 構築（`eframe::Frame` に依存しないため、ヘッドレスのスナップショットテストからも呼べる）。
    pub(crate) fn update_ui(&mut self, ctx: &egui::Context) {
        // ウィンドウ閉じる要求 (Alt+F4 や [X] ボタン) を検知
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.st.dirty {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.handle_close_request();
            } else {
                self.st.audio.stop();
                std::process::exit(0);
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
                .frame(egui::Frame::window(&ctx.style()).fill(crate::theme::surface()))
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

        // 新規プロジェクト事前設定モーダルダイアログ
        self.new_project_dialog(ctx);
        // 動画・音声の書き出しモーダルダイアログ
        self.export_dialog(ctx);
    }
}

impl OpenAviApp {
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
                    if ui.add(egui::Label::new(text).sense(egui::Sense::click())).clicked() {
                        self.st.autosave.enabled = !self.st.autosave.enabled;
                    }
                    if self.st.autosave.enabled {
                        ui.weak("（編集時に即時上書き）")
                            .on_hover_text("有効時: レイヤーやオブジェクトの変更があるたびに、プロジェクトファイルへ自動上書き保存されます。");
                    }
                    ui.add_space(6.0);
                });
                ui.separator();

                ui.menu_button("ファイル", |ui| {
                    if ui
                        .add(
                            egui::Button::new("📄 新規プロジェクト...")
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
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("🎬 動画・音声の書き出し (MP4 / AVI / GIF / WAV)...")
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
                    if ui.button("🔄 サンプルプロジェクトを再読み込み").clicked() {
                        self.st.load_exo(SAMPLE_EXO, None);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("終了").clicked() {
                        ui.close_menu();
                        if self.st.dirty {
                            self.handle_close_request();
                        } else {
                            self.st.audio.stop();
                            std::process::exit(0);
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
                    let cur_mode = crate::theme::mode();
                    let mut sel_mode = cur_mode;
                    ui.radio_value(&mut sel_mode, crate::theme::ThemeMode::Dark, "🌙 ダークモード");
                    ui.radio_value(&mut sel_mode, crate::theme::ThemeMode::Light, "☀ ライトモード");
                    if sel_mode != cur_mode {
                        crate::theme::set_mode(ctx, sel_mode);
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
                        egui::RichText::new(format!("🎬 シーン: {}", cur_name))
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
                        RichText::new("🎬 動画書き出し")
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
                    let theme_label = if crate::theme::is_dark() { "🌙 ダーク" } else { "☀ ライト" };
                    let next_mode = crate::theme::mode().toggled();
                    let theme_btn = egui::Button::new(RichText::new(theme_label).size(12.0))
                        .min_size(vec2(76.0, 26.0));
                    if ui
                        .add(theme_btn)
                        .on_hover_text(format!("クリックで{}に切り替えます", next_mode.label()))
                        .clicked()
                    {
                        crate::theme::set_mode(ctx, next_mode);
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
                                egui::RichText::new("⬇ FFmpeg を自動ダウンロード")
                                    .color(Color32::WHITE)
                                    .strong(),
                            )
                            .fill(crate::theme::accent());
                            if ui
                                .add(dl_btn)
                                .on_hover_text("gyan.dev 配布の FFmpeg をダウンロードし %LOCALAPPDATA%\\OpenAvi\\bin に導入します")
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
                            egui::RichText::new("📁 素材フォルダを指定して一括割り当て...")
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
                            "🖼 このファイルだけ選択..."
                        } else if is_audio {
                            "🎵 このファイルだけ選択..."
                        } else {
                            "🎞 このファイルだけ選択..."
                        };
                        if ui.button(single_label).clicked() {
                            self.st.relink_media_dialog(None, &first.original_path, is_audio);
                        }
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
    fn handle_close_request(&mut self) {
        if !self.st.dirty {
            self.st.audio.stop();
            std::process::exit(0);
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
                    std::process::exit(0);
                }
            }
            ConfirmResult::DontSave => {
                self.st.dirty = false;
                self.st.audio.stop();
                std::process::exit(0);
            }
            ConfirmResult::Cancel => {
                // 保存も終了もせず、編集作業を継続
            }
        }
    }

    /// 新規プロジェクト立ち上げ時の名前・形式・プリセット・自動保存の事前設定ダイアログ。
    fn new_project_dialog(&mut self, ctx: &egui::Context) {
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

    /// 動画・音声の書き出し（エクスポート / レンダリング）設定＆進捗モーダルダイアログ。
    fn export_dialog(&mut self, ctx: &egui::Context) {
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
                                        #[cfg(windows)]
                                        {
                                            let _ = std::process::Command::new("explorer")
                                                .arg("/select,")
                                                .arg(done_path)
                                                .spawn();
                                        }
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
