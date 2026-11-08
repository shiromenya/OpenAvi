//! アプリケーションの中心的な状態（編集プロジェクト・タイムライン・メディアキャッシュなど）。

pub mod types;
pub mod edit;
pub mod playback;
pub mod project;
pub mod media;
pub mod plugins;

#[cfg(test)]
mod tests;

pub use types::*;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use au2_core::{Frame, ObjectId, Scene, SceneSettings};
use au2_exo::TextEncoding;
use au2_media::{AudioEngine, MediaCache};
use au2_plugin::PluginRegistry;
use eframe::egui::TextureHandle;

/// サンプル（Shift_JIS）。
pub const SAMPLE_EXO: &[u8] = include_bytes!("../../../../samples/sample.exo");

/// OpenAvi アプリケーション全体のコア状態（プロジェクト・タイムライン・メディア・設定）。
///
/// 編集中のプロジェクトデータ、シーンツリー、再生ヘッド（フレーム）、複数選択、
/// Undo/Redo 履歴、メディアキャッシュ、タイムライン表示パラメータを一元管理します。
pub struct EditorState {
    /// 現在編集中のアクティブなシーン（Root またはサブシーン）
    pub scene: Scene,
    /// マルチシーン（Root = 0, Scene 1..N）のマップ
    pub scenes: std::collections::BTreeMap<u32, Scene>,
    /// 現在のプロジェクトファイルパス（`.aup` / `.aup2` / `.exo`）
    pub file_path: Option<PathBuf>,
    /// 外部プロジェクトファイルのテキストエンコーディング（UTF-8 または Shift_JIS）
    pub encoding: TextEncoding,
    /// 現在の再生ヘッド位置（フレーム番号、0 始まり）
    pub frame: Frame,
    /// リアルタイム再生中フラグ
    pub playing: bool,
    /// 再生開始時刻と開始フレーム（ドリフトしないよう経過時間から算出する）
    pub(crate) play_origin: Option<(Instant, Frame)>,
    /// 単一選択中のオブジェクト ID（詳細設定パネルの表示対象）
    pub selected: Option<ObjectId>,
    /// タイムライン上で現在選択されている全オブジェクトの ID 集合（複数選択対応）
    pub selected_objects: HashSet<ObjectId>,
    /// コピーされたタイムラインオブジェクトのクリップボード
    pub clipboard: Vec<ClipboardItem>,
    /// 操作の取り消し・やり直し履歴マネージャー (Undo / Redo)
    pub history: HistoryManager,
    /// ステータスバーに表示する現在の状態メッセージ
    pub status: String,
    /// プロジェクト読み込み時や処理中に発生した警告メッセージ一覧
    pub warnings: Vec<String>,
    /// タイムラインの表示パラメータ（ズーム倍率、スクロール座標等）
    pub timeline: TimelineView,
    /// プラグインおよびスクリプトの登録マネージャー
    pub plugins: PluginRegistry,
    /// 画像・動画・音声の非同期デコードキャッシュ
    pub media: MediaCache,
    /// リアルタイムオーディオ出力エンジン（Windows: winmm / macOS: AudioToolbox / Linux: PipeWire・PulseAudio・ALSA）
    pub audio: AudioEngine,
    /// ソースパネルに登録されたメディア素材一覧
    pub sources: Vec<SourceMediaItem>,
    /// ソースパネルで現在プレビュー選択中の素材インデックス
    pub selected_source: Option<usize>,
    /// 静止画および動画サムネイル (frame 0) 用の永続 GPU テクスチャキャッシュ
    pub(crate) static_textures: HashMap<PathBuf, (TextureHandle, [usize; 2])>,
    /// 動画再生フレーム用の In-Place 更新 GPU テクスチャキャッシュ (Path -> (Handle, 現在フレーム番号, 解像度))
    pub(crate) stream_textures: HashMap<PathBuf, (TextureHandle, u32, [usize; 2])>,
    /// PSD / PSB レイヤー合成状態ごとの GPU テクスチャキャッシュ ((Path, StateHash) -> (Handle, 解像度))
    pub(crate) psd_textures: HashMap<(PathBuf, u64), (TextureHandle, [usize; 2])>,
    /// ファイルパス解決のキャッシュ（毎フレームの OS ディスク IO / exists 呼び出しを削減）
    pub resolved_paths: HashMap<String, Option<PathBuf>>,
    /// ユーザーが「フォルダから一括再リンク」で指定した追加の素材検索フォルダ
    pub extra_search_dirs: Vec<PathBuf>,
    /// FFmpeg（動画・音声デコーダー）のアプリ内ダウンロード状態
    pub ffmpeg_dl: FfmpegDownload,
    /// 未保存の変更が存在するかどうかを示すダーティフラグ
    pub dirty: bool,
    /// プロジェクトの自動保存（オートセーブ）設定
    pub autosave: AutoSaveSettings,
    /// `.auf` / `.auf2` 自動解析＆クロスプラットフォーム移植・変換スタジオの状態
    pub auf_porter: AufPorterModalState,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            scene: Scene::new("Root", SceneSettings::default()),
            scenes: std::collections::BTreeMap::new(),
            file_path: None,
            encoding: TextEncoding::ShiftJis,
            frame: 0,
            playing: false,
            play_origin: None,
            selected: None,
            selected_objects: HashSet::new(),
            clipboard: Vec::new(),
            history: HistoryManager::default(),
            status: "準備完了".into(),
            warnings: Vec::new(),
            timeline: TimelineView::default(),
            plugins: PluginRegistry::new(default_plugin_dir()),
            media: MediaCache::new(),
            audio: AudioEngine::new(),
            sources: initial_sources(),
            selected_source: None,
            static_textures: HashMap::new(),
            stream_textures: HashMap::new(),
            psd_textures: HashMap::new(),
            resolved_paths: HashMap::new(),
            extra_search_dirs: Vec::new(),
            ffmpeg_dl: FfmpegDownload::Idle,
            dirty: false,
            autosave: AutoSaveSettings::default(),
            auf_porter: AufPorterModalState::default(),
        }
    }
}
