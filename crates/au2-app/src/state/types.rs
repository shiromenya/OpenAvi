//! 状態管理に関連する補助構造体・列挙型・プリセット定義。

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use au2_core::{Frame, ObjectId, Scene, TimelineObject};
use au2_media::MediaKind;
use eframe::egui;

/// ソースパネルに登録されたメディア素材 1 件。
#[derive(Clone, Debug)]
pub struct SourceMediaItem {
    pub name: String,
    pub path: PathBuf,
    pub kind: MediaKind,
}

/// プロジェクト内で未検出（リンク切れ）となっているメディア素材情報。
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct MissingMediaItem {
    pub object_id: ObjectId,
    pub layer_index: usize,
    pub kind_name: String,
    pub original_path: String,
    pub frame_start: Frame,
    pub frame_end: Frame,
}

/// タイムライン右クリック時のコンテキストターゲット。
#[derive(Clone, Copy, Debug)]
pub struct ContextTarget {
    pub layer: usize,
    pub frame: Frame,
    pub object_id: Option<ObjectId>,
}

/// タイムラインの表示状態。
pub struct TimelineView {
    /// 1 フレームあたりのピクセル数
    pub px_per_frame: f32,
    /// 左端のフレーム位置（小数可）
    pub scroll_x: f32,
    /// 縦スクロール量（ピクセル）
    pub scroll_y: f32,
    pub drag: Option<TimelineDrag>,
    /// 右クリックされた座標に対応するレイヤー・フレーム・オブジェクト情報
    pub context_target: ContextTarget,
    /// Premiere Pro 風のクリップ内ビジュアルプレビュー表示の ON/OFF（既定: ON）
    pub show_clip_previews: bool,
    /// オブジェクト境界や再生ヘッドへの吸着（スナップ）有効/無効
    pub snap_enabled: bool,
    /// 直近に吸着（スナップ）が発生したフレーム位置（ガイド線表示用）
    pub active_snap_line: Option<Frame>,
    /// 現在選択されているアクティブなレイヤー番号
    pub selected_layer: usize,
    /// タイムラインのレーン表示領域（外部D&Dやヒットテスト判定用）
    pub lanes_rect: Option<egui::Rect>,
}

#[derive(Clone, Copy, Debug)]
pub enum TimelineDrag {
    /// 再生位置のスクラブ
    Scrub,
    /// オブジェクト移動。`grab_offset` は掴んだ位置とオブジェクト先頭の差（フレーム）
    Move { id: ObjectId, grab_offset: i64 },
    /// オブジェクト左端ドラッグ（開始フレームの伸縮）
    ResizeLeft { id: ObjectId },
    /// オブジェクト右端ドラッグ（終了フレームの伸縮・長さ変更）
    ResizeRight { id: ObjectId },
    /// 中クリックやドラッグによるタイムライン自由パン（縦横スクロール）
    Pan { start_scroll_x: f32, start_scroll_y: f32, start_pos: egui::Pos2 },
    /// 下部横スクロールバーのドラッグ
    ScrollbarThumb { grab_rel: f32 },
    /// 右下タイムライン横縮尺（ズーム）スライダーのドラッグ
    ZoomSlider,
    /// タイムライン上の矩形範囲選択（ラバーバンド選択）
    BoxSelect { start_pos: egui::Pos2, current_pos: egui::Pos2 },
}

impl Default for TimelineView {
    fn default() -> Self {
        Self {
            px_per_frame: 4.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            drag: None,
            context_target: ContextTarget {
                layer: 0,
                frame: 0,
                object_id: None,
            },
            show_clip_previews: true,
            snap_enabled: true,
            active_snap_line: None,
            selected_layer: 0,
            lanes_rect: None,
        }
    }
}

/// プロジェクトの編集履歴（Undo/Redo 用スナップショット）。
#[derive(Clone, Debug)]
pub struct HistorySnapshot {
    pub scene: Scene,
    pub scenes: std::collections::BTreeMap<u32, Scene>,
    pub selected: Option<ObjectId>,
    pub selected_objects: HashSet<ObjectId>,
    pub description: String,
}

/// Undo / Redo の履歴スタック管理。
#[derive(Debug, Clone)]
pub struct HistoryManager {
    pub undo_stack: Vec<HistorySnapshot>,
    pub redo_stack: Vec<HistorySnapshot>,
    pub max_depth: usize,
    /// スライダーやドラッグ移動などの連続操作中に、操作開始前の状態を一時退避するスナップショット
    pub pending_continuous_base: Option<HistorySnapshot>,
}

impl Default for HistoryManager {
    fn default() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_depth: 60,
            pending_continuous_base: None,
        }
    }
}

/// タイムラインオブジェクトのクリップボード保管データ。
/// 複数選択されたクリップ群の相対的なレイヤー関係や開始時間差を正確に保持します。
#[derive(Debug, Clone)]
pub struct ClipboardItem {
    /// コピー元選択集合の最小レイヤーに対する相対レイヤー番号
    pub rel_layer: usize,
    /// コピー元選択集合の最小開始フレームに対する相対フレームオフセット
    pub rel_frame_start: Frame,
    /// クリップの長さ (frame_end - frame_start)
    pub duration: Frame,
    /// オブジェクトのディープコピー（エフェクトや設定をすべて保持）
    pub object: TimelineObject,
}

/// `.auf` / `.auf2` 移植スタジオの生成コード表示タブ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AufPorterTab {
    AnmLua,
    WgslShader,
    RustCode,
    JsonSpec,
}

/// `.auf` / `.auf2` 自動解析＆クロスプラットフォーム移植・変換モーダルダイアログの状態。
#[derive(Debug, Clone)]
pub struct AufPorterModalState {
    pub is_open: bool,
    pub selected_plugin_idx: usize,
    pub active_tab: AufPorterTab,
}

impl Default for AufPorterModalState {
    fn default() -> Self {
        Self {
            is_open: false,
            selected_plugin_idx: 0,
            active_tab: AufPorterTab::AnmLua,
        }
    }
}

/// プロジェクトの自動保存（オートセーブ）設定。
#[derive(Debug, Clone)]
pub struct AutoSaveSettings {
    /// 自動保存が有効かどうか
    pub enabled: bool,
    /// 変更発生時のタイムスタンプ（操作完了後の即時上書き保存用デバウンス）
    pub dirty_since: Option<Instant>,
    /// 直近に保存された時刻
    pub last_saved: Option<Instant>,
    /// 直近の保存成功時刻の文字列表現 (例: "14:25:30")
    pub last_saved_time_str: Option<String>,
    /// セッション中の自動保存回数
    pub auto_save_count: usize,
}

impl Default for AutoSaveSettings {
    fn default() -> Self {
        Self {
            enabled: false, // デフォルトではオフ
            dirty_since: None,
            last_saved: None,
            last_saved_time_str: None,
            auto_save_count: 0,
        }
    }
}

/// 新規プロジェクト作成時のファイル保存形式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectFormat {
    /// AviUtl2 プロジェクト (*.aup2)
    Aup2,
    /// 初代 AviUtl プロジェクト (*.aup)
    Aup,
    /// 拡張編集オブジェクトファイル (*.exo)
    Exo,
}

impl ProjectFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            ProjectFormat::Aup2 => "aup2",
            ProjectFormat::Aup => "aup",
            ProjectFormat::Exo => "exo",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ProjectFormat::Aup2 => "AviUtl2 プロジェクト (*.aup2) [推奨・高速・完全対応]",
            ProjectFormat::Aup => "初代 AviUtl プロジェクト (*.aup) [クラシック互換]",
            ProjectFormat::Exo => "拡張編集オブジェクトファイル (*.exo)",
        }
    }
}

/// 解像度・フレームレートのプリセット。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectPreset {
    Fhd60p,
    Fhd30p,
    Hd720p,
    ShortsVertical,
    Uhd4k,
    Custom,
}

impl ProjectPreset {
    pub fn label(&self) -> &'static str {
        match self {
            ProjectPreset::Fhd60p => "Full HD 1080p 60fps (1920×1080, YouTube / Twitch 推奨)",
            ProjectPreset::Fhd30p => "Full HD 1080p 30fps (1920×1080, 標準動画)",
            ProjectPreset::Hd720p => "HD 720p 30fps (1280×720, 軽量)",
            ProjectPreset::ShortsVertical => "縦型ショート 60fps (1080×1920, TikTok / YouTube Shorts / Reels)",
            ProjectPreset::Uhd4k => "4K UHD 60fps (3840×2160, 超高精細)",
            ProjectPreset::Custom => "カスタム設定",
        }
    }

    pub fn apply(&self, width: &mut u32, height: &mut u32, fps_num: &mut u32, fps_den: &mut u32) {
        match self {
            ProjectPreset::Fhd60p => {
                *width = 1920;
                *height = 1080;
                *fps_num = 60;
                *fps_den = 1;
            }
            ProjectPreset::Fhd30p => {
                *width = 1920;
                *height = 1080;
                *fps_num = 30;
                *fps_den = 1;
            }
            ProjectPreset::Hd720p => {
                *width = 1280;
                *height = 720;
                *fps_num = 30;
                *fps_den = 1;
            }
            ProjectPreset::ShortsVertical => {
                *width = 1080;
                *height = 1920;
                *fps_num = 60;
                *fps_den = 1;
            }
            ProjectPreset::Uhd4k => {
                *width = 3840;
                *height = 2160;
                *fps_num = 60;
                *fps_den = 1;
            }
            ProjectPreset::Custom => {}
        }
    }
}

/// 新規プロジェクト作成時の設定情報。
#[derive(Debug, Clone)]
pub struct NewProjectConfig {
    pub name: String,
    pub dir: PathBuf,
    pub format: ProjectFormat,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub length_frames: u32,
    pub autosave_enabled: bool,
}

/// 日本標準時 (JST = UTC+9) の現在時刻文字列 "HH:MM:SS" を取得します。
pub fn current_time_string() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let total_secs = dur.as_secs();
    let jst_secs = total_secs + 9 * 3600;
    let sec = jst_secs % 60;
    let min = (jst_secs / 60) % 60;
    let hour = (jst_secs / 3600) % 24;
    format!("{:02}:{:02}:{:02}", hour, min, sec)
}

/// デフォルトの動画プロジェクト保存先ディレクトリ。
pub fn default_projects_dir() -> PathBuf {
    if let Some(user) = au2_platform::home_dir() {
        let p = PathBuf::from(&user).join("Videos").join("OpenAvi");
        if p.exists() || std::fs::create_dir_all(&p).is_ok() {
            return p;
        }
        let doc = PathBuf::from(&user).join("Documents").join("OpenAvi");
        if doc.exists() || std::fs::create_dir_all(&doc).is_ok() {
            return doc;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// FFmpeg アプリ内ダウンロードの進行状態。
pub enum FfmpegDownload {
    Idle,
    Running(std::sync::mpsc::Receiver<Result<PathBuf, String>>),
    Failed(String),
}

pub(crate) fn resolve_app_path(rel: &str) -> Option<PathBuf> {
    let local = PathBuf::from(rel);
    if local.exists() {
        return Some(local);
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut cur = exe.parent();
        for _ in 0..4 {
            if let Some(dir) = cur {
                let candidate = dir.join(rel);
                if candidate.exists() {
                    return Some(candidate);
                }
                cur = dir.parent();
            }
        }
    }
    None
}

pub(crate) fn default_plugin_dir() -> PathBuf {
    resolve_app_path("plugins").unwrap_or_else(|| PathBuf::from("plugins"))
}

pub(crate) fn initial_sources() -> Vec<SourceMediaItem> {
    let mut list = Vec::new();
    for rel in ["samples/sample_video.mp4", "samples/sample_image.png"] {
        if let Some(p) = resolve_app_path(rel) {
            if let Some(kind) = MediaKind::from_path(&p) {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| rel.to_string());
                list.push(SourceMediaItem { name, path: p, kind });
            }
        }
    }
    list
}
