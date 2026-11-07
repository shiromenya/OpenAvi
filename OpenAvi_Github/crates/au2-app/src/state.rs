//! エディタの状態（UI から独立したアプリケーション状態）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use au2_core::{EffectKind, Frame, ObjectId, Scene, SceneSettings, TimelineObject, Track};
use au2_exo::TextEncoding;
use au2_media::{AudioEngine, AudioTrackItem, MediaCache, MediaKind};
use au2_plugin::PluginRegistry;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

/// 同梱サンプル（Shift_JIS）。
pub const SAMPLE_EXO: &[u8] = include_bytes!("../../../samples/sample.exo");

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
        }
    }
}

pub struct EditorState {
    pub scene: Scene,
    /// マルチシーン（Root = 0, Scene 1..N）のマップ
    pub scenes: std::collections::BTreeMap<u32, Scene>,
    /// 現在のプロジェクトファイルパス（`.aup` または `.exo`）
    pub file_path: Option<PathBuf>,
    pub encoding: TextEncoding,
    pub frame: Frame,
    pub playing: bool,
    /// 再生開始時刻と開始フレーム（ドリフトしないよう経過時間から算出する）
    play_origin: Option<(Instant, Frame)>,
    pub selected: Option<ObjectId>,
    pub status: String,
    pub warnings: Vec<String>,
    pub timeline: TimelineView,
    pub plugins: PluginRegistry,
    pub media: MediaCache,
    /// リアルタイムオーディオ出力エンジン（Windows: winmm / macOS: AudioToolbox / Linux: PipeWire・PulseAudio・ALSA）
    pub audio: AudioEngine,
    /// ソースパネルに登録されたメディア素材一覧
    pub sources: Vec<SourceMediaItem>,
    /// ソースパネルで現在プレビュー選択中の素材インデックス
    pub selected_source: Option<usize>,
    /// 静止画および動画サムネイル (frame 0) 用の永続 GPU テクスチャキャッシュ
    static_textures: HashMap<PathBuf, (TextureHandle, [usize; 2])>,
    /// 動画再生フレーム用の In-Place 更新 GPU テクスチャキャッシュ (Path -> (Handle, 現在フレーム番号, 解像度))
    stream_textures: HashMap<PathBuf, (TextureHandle, u32, [usize; 2])>,
    /// PSD / PSB レイヤー合成状態ごとの GPU テクスチャキャッシュ ((Path, StateHash) -> (Handle, 解像度))
    psd_textures: HashMap<(PathBuf, u64), (TextureHandle, [usize; 2])>,
    /// ファイルパス解決のキャッシュ（毎フレームの OS ディスク IO / exists 呼び出しを削減）
    pub resolved_paths: HashMap<String, Option<PathBuf>>,
    /// ユーザーが「フォルダから一括再リンク」で指定した追加の素材検索フォルダ。
    pub extra_search_dirs: Vec<PathBuf>,
    /// FFmpeg（動画・音声デコーダー）のアプリ内ダウンロード状態
    pub ffmpeg_dl: FfmpegDownload,
    pub dirty: bool,
    /// 自動保存設定
    pub autosave: AutoSaveSettings,
    /// `.auf` / `.auf2` 自動解析＆クロスプラットフォーム移植・変換スタジオの状態
    pub auf_porter: AufPorterModalState,
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

fn resolve_app_path(rel: &str) -> Option<PathBuf> {
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

fn default_plugin_dir() -> PathBuf {
    resolve_app_path("plugins").unwrap_or_else(|| PathBuf::from("plugins"))
}

fn initial_sources() -> Vec<SourceMediaItem> {
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

impl EditorState {
    /// 動画・音声メディアを扱うのに FFmpeg が必要なのに見つかっていない場合 `true`。
    pub fn needs_ffmpeg(&self) -> bool {
        if self.media.has_ffmpeg() {
            return false;
        }
        let in_scene = self.scene.layers.iter().any(|l| {
            l.objects.iter().any(|o| {
                o.source()
                    .is_some_and(|s| matches!(s.kind, EffectKind::VideoFile | EffectKind::AudioFile))
            })
        });
        in_scene
            || self
                .sources
                .iter()
                .any(|s| matches!(s.kind, MediaKind::Video | MediaKind::Audio))
    }

    /// FFmpeg を別スレッドでダウンロードしてユーザーデータ領域（`OpenAvi/bin`）に導入します。
    pub fn start_ffmpeg_download(&mut self) {
        if matches!(self.ffmpeg_dl, FfmpegDownload::Running(_)) {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(au2_media::download_ffmpeg_blocking());
        });
        self.ffmpeg_dl = FfmpegDownload::Running(rx);
        self.status = "FFmpeg をダウンロードしています（約 80MB・数十秒〜数分かかります）...".into();
    }

    /// ダウンロード完了を確認し、成功時は FFmpeg を再読込して動画・音声を即座に使えるようにします。
    pub fn poll_ffmpeg_download(&mut self) {
        let result = match &self.ffmpeg_dl {
            FfmpegDownload::Running(rx) => match rx.try_recv() {
                Ok(r) => Some(r),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(Err("ダウンロード処理が予期せず終了しました".to_string()))
                }
            },
            _ => None,
        };
        match result {
            Some(Ok(dir)) => {
                self.ffmpeg_dl = FfmpegDownload::Idle;
                if self.media.reload_ffmpeg() {
                    self.static_textures.clear();
                    self.stream_textures.clear();
                    self.status = format!("FFmpeg を導入しました ({})。動画・音声が再生できます", dir.display());
                } else {
                    self.ffmpeg_dl = FfmpegDownload::Failed("導入後も FFmpeg を検出できませんでした".into());
                }
            }
            Some(Err(e)) => {
                self.status = e.clone();
                self.ffmpeg_dl = FfmpegDownload::Failed(e);
            }
            None => {}
        }
    }
}

impl EditorState {
    /// 編集可能な最終フレーム。
    pub fn last_frame(&self) -> Frame {
        self.scene.settings.length.max(self.scene.content_end()).max(1) - 1
    }

    pub fn set_frame(&mut self, frame: Frame) {
        if frame >= self.scene.settings.length {
            self.scene.settings.length = (frame + 300).max(self.scene.settings.length);
        }
        self.frame = frame;
        if self.playing {
            self.play_origin = Some((Instant::now(), self.frame));
        } else {
            self.audio.stop();
        }
    }

    pub fn step(&mut self, delta: i64) {
        let f = (self.frame as i64 + delta).max(0) as Frame;
        self.set_frame(f);
    }

    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.play_origin = self.playing.then(|| (Instant::now(), self.frame));
        if !self.playing {
            self.audio.stop();
        }
    }

    /// 再生中なら経過時間からフレームを進め、音声を完全同期再生します（末尾でループ）。
    /// また、設定に応じてプロジェクトの自動保存（オートセーブ）を定期実行します。
    pub fn tick(&mut self) {
        if self.playing {
            if let Some((origin, start)) = self.play_origin {
                let fps = self.scene.settings.frame_rate;
                let mut f = start + fps.seconds_to_frame(origin.elapsed().as_secs_f64());
                if f > self.last_frame() {
                    f = 0;
                    self.play_origin = Some((Instant::now(), 0));
                }
                self.frame = f;
            }
            self.sync_audio_playback();
        } else {
            self.audio.stop();
        }

        // 編集変更がある場合の自動保存チェック
        self.autosave_tick();
    }

    /// 編集変更が発生したことを記録し、自動保存のデバウンスタイマーを開始します。
    #[allow(dead_code)]
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        if self.autosave.dirty_since.is_none() {
            self.autosave.dirty_since = Some(Instant::now());
        }
    }

    /// 自動保存のチェック・実行。
    /// 有効時は、レイヤーやパラメータに変更が生じる（dirty == true）たびに
    /// 操作直後（0.3秒後）に自動でプロジェクトファイルへ即座に上書き保存します。
    pub fn autosave_tick(&mut self) {
        if !self.autosave.enabled || !self.dirty {
            self.autosave.dirty_since = None;
            return;
        }
        let Some(path) = self.file_path.clone() else {
            return;
        };

        // 変更検知の初回時刻を記録
        let dirty_at = *self.autosave.dirty_since.get_or_insert_with(Instant::now);

        // 変更直後 0.3 秒静止（ドラッグや入力完了直後）したら即座に上書き保存
        if dirty_at.elapsed() >= std::time::Duration::from_millis(300) {
            match self.write_to_path(&path) {
                Ok(()) => {
                    self.autosave.auto_save_count += 1;
                    self.autosave.dirty_since = None;
                    let time_str = self.autosave.last_saved_time_str.as_deref().unwrap_or("");
                    let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    self.status = format!("変更を自動保存しました ({fname}) [{time_str}]");
                    log::info!("[自動保存] {time_str}: {}", path.display());
                }
                Err(e) => {
                    self.status = format!("[自動保存失敗] {e}");
                    log::warn!("[自動保存失敗] {e}");
                }
            }
        }
    }

    /// プロジェクトデータを指定のパスへ書き出し、保存ステータスを更新します。
    pub fn write_to_path(&mut self, path: &Path) -> Result<(), String> {
        let is_legacy_aup = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("aup"));
        let is_exo = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("exo"));

        let bytes = if is_legacy_aup {
            au2_aup::write_aup_bytes(&self.scene)
        } else if is_exo {
            au2_exo::write_exo_bytes(&self.scene, self.encoding)
        } else if !self.scenes.is_empty() {
            let mut all_scenes = self.scenes.clone();
            all_scenes.insert(0, self.scene.clone());
            au2_aup::write_aup2_multi_scene_string(&all_scenes, 0, Some(path), self.frame).into_bytes()
        } else {
            au2_aup::write_aup2_bytes(&self.scene, Some(path), self.frame)
        };

        std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        self.dirty = false;
        self.autosave.dirty_since = None;
        self.autosave.last_saved = Some(Instant::now());
        self.autosave.last_saved_time_str = Some(current_time_string());
        Ok(())
    }

    /// 事前設定ダイアログで指定された名前・形式・プリセット・自動保存フラグで新規プロジェクトを作成します。
    pub fn create_new_project_with_config(&mut self, config: NewProjectConfig) -> Result<PathBuf, String> {
        let ext = config.format.extension();
        let file_name = if config.name.to_ascii_lowercase().ends_with(&format!(".{ext}")) {
            config.name.clone()
        } else {
            format!("{}.{ext}", config.name)
        };
        let target_path = config.dir.join(file_name);

        if let Err(e) = std::fs::create_dir_all(&config.dir) {
            return Err(format!("保存先フォルダの作成に失敗しました: {e}"));
        }

        let mut settings = SceneSettings::default();
        settings.width = config.width;
        settings.height = config.height;
        settings.frame_rate = au2_core::Rational::new(config.fps_num, config.fps_den);
        settings.length = config.length_frames;

        let plugins = self.plugins.clone();
        let sources = std::mem::take(&mut self.sources);
        *self = Self {
            scene: Scene::new("Root", settings),
            scenes: std::collections::BTreeMap::new(),
            file_path: Some(target_path.clone()),
            encoding: TextEncoding::Utf8,
            frame: 0,
            playing: false,
            play_origin: None,
            selected: None,
            status: format!("新規プロジェクトを作成しました: {}", target_path.display()),
            warnings: Vec::new(),
            timeline: TimelineView::default(),
            plugins,
            media: MediaCache::new(),
            audio: AudioEngine::new(),
            sources,
            selected_source: None,
            static_textures: HashMap::new(),
            stream_textures: HashMap::new(),
            psd_textures: HashMap::new(),
            resolved_paths: HashMap::new(),
            extra_search_dirs: Vec::new(),
            ffmpeg_dl: FfmpegDownload::Idle,
            dirty: false,
            autosave: AutoSaveSettings {
                enabled: config.autosave_enabled,
                dirty_since: None,
                last_saved: Some(Instant::now()),
                last_saved_time_str: Some(current_time_string()),
                auto_save_count: 0,
            },
            auf_porter: AufPorterModalState::default(),
        };

        if let Err(e) = self.write_to_path(&target_path) {
            return Err(format!("プロジェクト初期ファイルの作成に失敗しました: {e}"));
        }
        self.dirty = false;
        Ok(target_path)
    }
}

/// 日本語の結合濁点・結合半濁点（NFD形式）を合成文字（NFC形式）に結合します。
fn compose_japanese_dakuten(input: &str) -> String {
    let mut chars = input.chars().peekable();
    let mut result = String::with_capacity(input.len());
    while let Some(c) = chars.next() {
        if let Some(&next_c) = chars.peek() {
            if next_c == '\u{3099}' {
                let composed = match c {
                    'か' => 'が', 'き' => 'ぎ', 'く' => 'ぐ', 'け' => 'げ', 'こ' => 'ご',
                    'さ' => 'ざ', 'し' => 'じ', 'す' => 'ず', 'せ' => 'ぜ', 'そ' => 'ぞ',
                    'た' => 'だ', 'ち' => 'ぢ', 'つ' => 'づ', 'て' => 'で', 'と' => 'ど',
                    'は' => 'ば', 'ひ' => 'び', 'ふ' => 'ぶ', 'へ' => 'べ', 'ほ' => 'ぼ',
                    'カ' => 'ガ', 'キ' => 'ギ', 'ク' => 'グ', 'ケ' => 'ゲ', 'コ' => 'ゴ',
                    'サ' => 'ザ', 'シ' => 'ジ', 'ス' => 'ズ', 'セ' => 'ゼ', 'ソ' => 'ゾ',
                    'タ' => 'ダ', 'チ' => 'ヂ', 'ツ' => 'ヅ', 'テ' => 'デ', 'ト' => 'ド',
                    'ハ' => 'バ', 'ヒ' => 'ビ', 'フ' => 'ブ', '\u{30D8}' => '\u{30D9}', 'ホ' => 'ボ',
                    'ウ' => 'ヴ',
                    _ => c,
                };
                if composed != c {
                    result.push(composed);
                    chars.next(); // 濁点消費
                    continue;
                }
            } else if next_c == '\u{309A}' {
                let composed = match c {
                    'は' => 'ぱ', 'ひ' => 'ぴ', 'ふ' => 'ぷ', 'へ' => 'ぺ', 'ほ' => 'ぽ',
                    'ハ' => 'パ', 'ヒ' => 'ピ', 'フ' => 'プ', '\u{30D8}' => '\u{30DA}', 'ホ' => 'ポ',
                    _ => c,
                };
                if composed != c {
                    result.push(composed);
                    chars.next(); // 半濁点消費
                    continue;
                }
            }
        }
        result.push(c);
    }
    result
}

/// 照合用キーを正規化します（全角スペース->半角スペース、連続スペース圧縮、NFC濁点合成、小文字化、トリム）
fn normalize_match_key(input: &str) -> String {
    let composed = compose_japanese_dakuten(input);
    let mut res = String::with_capacity(composed.len());
    let mut last_was_space = false;
    for c in composed.chars() {
        let normalized_char = match c {
            '　' => ' ',
            '_' | '-' => ' ',
            c => c,
        };
        if normalized_char == ' ' {
            if !last_was_space && !res.is_empty() {
                res.push(' ');
                last_was_space = true;
            }
        } else {
            for lc in normalized_char.to_lowercase() {
                res.push(lc);
            }
            last_was_space = false;
        }
    }
    res.trim().to_string()
}

fn scan_subdirs<F: FnMut(PathBuf)>(root: &Path, max_depth: usize, cb: &mut F) {
    if max_depth == 0 {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if file_name.starts_with('.')
                    || file_name.eq_ignore_ascii_case("node_modules")
                    || file_name.eq_ignore_ascii_case("target")
                    || file_name.eq_ignore_ascii_case("appdata")
                    || file_name.eq_ignore_ascii_case("windows")
                    || file_name.starts_with('$')
                {
                    continue;
                }
                cb(path.clone());
                scan_subdirs(&path, max_depth - 1, cb);
            }
        }
    }
}

fn collect_search_dirs(proj_path: Option<&Path>, extra_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut visited = std::collections::HashSet::new();

    let mut add_dir = |d: PathBuf| {
        if d.is_dir() && visited.insert(d.clone()) {
            dirs.push(d);
        }
    };

    // 0. ユーザーが明示指定した素材フォルダ（サブフォルダ 6 階層まで）を最優先
    for extra in extra_dirs {
        add_dir(extra.clone());
        scan_subdirs(extra, 6, &mut add_dir);
    }

    // 1. プロジェクトファイルの親フォルダおよびそのサブフォルダ（深さ3まで）
    if let Some(proj) = proj_path {
        if let Some(parent) = proj.parent() {
            add_dir(parent.to_path_buf());
            scan_subdirs(parent, 3, &mut add_dir);
        }
    }

    // 2. ユーザープロファイル配下の主要フォルダ（Desktop, Downloads, Music, Videos, Documents）
    if let Some(user) = au2_platform::home_dir() {
        let user_path = PathBuf::from(user);
        let desktop = user_path.join("Desktop");
        add_dir(desktop.clone());
        scan_subdirs(&desktop, 3, &mut add_dir);

        let downloads = user_path.join("Downloads");
        add_dir(downloads.clone());
        scan_subdirs(&downloads, 2, &mut add_dir);

        let music = user_path.join("Music");
        add_dir(music.clone());
        scan_subdirs(&music, 2, &mut add_dir);

        let videos = user_path.join("Videos");
        add_dir(videos.clone());
        scan_subdirs(&videos, 2, &mut add_dir);

        let docs = user_path.join("Documents");
        add_dir(docs.clone());
        scan_subdirs(&docs, 2, &mut add_dir);
    }

    // 3. カレントディレクトリ
    if let Ok(cur) = std::env::current_dir() {
        add_dir(cur.clone());
        add_dir(cur.join("samples"));
    }

    dirs
}

impl EditorState {
    /// パス解決をメモリキャッシュ経由で行い、毎フレームの OS ファイルシステム I/O を排除します。
    /// プロジェクトファイルと同じディレクトリやサブディレクトリ、デスクトップ、ダウンロード、
    /// および拡張子違い（.wav と .mp3 等）や濁点・空白揺れも自動でスマート曖昧探索して解決します。
    pub fn resolve_cached_path(&mut self, path_str: &str) -> Option<PathBuf> {
        let raw_trimmed = path_str.trim();
        if raw_trimmed.is_empty() {
            return None;
        }
        if let Some(cached) = self.resolved_paths.get(raw_trimmed) {
            return cached.clone();
        }

        let expanded = au2_aup::expand_aviutl2_path_vars(raw_trimmed);
        let trimmed = expanded.trim();

        // 1. そのまま存在するかチェック（スラッシュ正規化、ドライブ補正）
        let mut p = PathBuf::from(trimmed);
        if !p.exists() {
            let normalized_slashes = trimmed.replace('/', "\\");
            let p_slash = PathBuf::from(&normalized_slashes);
            if p_slash.exists() {
                p = p_slash;
            } else if trimmed.starts_with(":\\") || trimmed.starts_with(":/") {
                let with_c = format!("C{}", trimmed);
                let p_c = PathBuf::from(&with_c);
                if p_c.exists() {
                    p = p_c;
                }
            }
        }

        if !p.exists() {
            let trimmed_path = Path::new(trimmed);
            let orig_file_name = trimmed_path.file_name().map(|n| n.to_string_lossy().into_owned());
            let orig_stem = trimmed_path.file_stem().map(|s| s.to_string_lossy().into_owned());

            let search_dirs = collect_search_dirs(self.file_path.as_deref(), &self.extra_search_dirs);
            let mut found = None;

            // 2. 末尾サブパスによる照合（親フォルダ＋ファイル名の組み合わせ）
            // 例: "創作関係\Aviutl\2018\ココロフロート\ココロフロート.mp3" の末尾 "ココロフロート\ココロフロート.mp3"
            if let Some(parent_dir_name) = trimmed_path.parent().and_then(|p| p.file_name()) {
                if let Some(fname) = &orig_file_name {
                    let sub_rel = Path::new(parent_dir_name).join(fname);
                    for dir in &search_dirs {
                        let cand = dir.join(&sub_rel);
                        if cand.is_file() {
                            found = Some(cand);
                            break;
                        }
                    }
                }
            }

            // 3. 完全同名ファイル名照合（ファイル名が完全一致、または空白・NFC濁点・大文字小文字正規化で一致）
            if found.is_none() {
                if let Some(name) = &orig_file_name {
                    let target_key = normalize_match_key(name);
                    'name_match: for dir in &search_dirs {
                        let cand_direct = dir.join(name);
                        if cand_direct.is_file() {
                            found = Some(cand_direct);
                            break;
                        }
                        if let Ok(entries) = std::fs::read_dir(dir) {
                            for entry in entries.flatten() {
                                let c_path = entry.path();
                                if c_path.is_file() {
                                    if let Some(c_name) = c_path.file_name().and_then(|n| n.to_str()) {
                                        if normalize_match_key(c_name) == target_key {
                                            found = Some(c_path);
                                            break 'name_match;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 4. 拡張子違い（互換メディアフォーマット: wav, mp3, m4a, aac, flac, ogg, wma, mp4, avi, mov, mkv）のファイル名幹（stem）一致
            if found.is_none() {
                if let Some(stem) = &orig_stem {
                    let target_stem_key = normalize_match_key(stem);
                    const COMPAT_EXTS: &[&str] = &[
                        "wav", "mp3", "m4a", "aac", "flac", "ogg", "wma", "mp4", "avi", "mov", "mkv", "webm", "psd", "psb",
                    ];
                    'stem_match: for dir in &search_dirs {
                        for ext in COMPAT_EXTS {
                            let cand = dir.join(format!("{}.{}", stem, ext));
                            if cand.is_file() {
                                found = Some(cand);
                                break 'stem_match;
                            }
                        }
                        if let Ok(entries) = std::fs::read_dir(dir) {
                            for entry in entries.flatten() {
                                let c_path = entry.path();
                                if c_path.is_file() {
                                    if let Some(c_ext) = c_path.extension().and_then(|e| e.to_str()) {
                                        if COMPAT_EXTS.iter().any(|&e| e.eq_ignore_ascii_case(c_ext)) {
                                            if let Some(c_stem) = c_path.file_stem().and_then(|s| s.to_str()) {
                                                if normalize_match_key(c_stem) == target_stem_key {
                                                    found = Some(c_path);
                                                    break 'stem_match;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 5. ファジー部分一致（包含関係: 片方がもう一方を含み、類似度が高いものを選択）
            if found.is_none() {
                if let Some(stem) = &orig_stem {
                    let target_stem_key = normalize_match_key(stem);
                    if target_stem_key.len() >= 3 {
                        let mut best_match: Option<(PathBuf, usize)> = None;
                        for dir in &search_dirs {
                            if let Ok(entries) = std::fs::read_dir(dir) {
                                for entry in entries.flatten() {
                                    let c_path = entry.path();
                                    if c_path.is_file() {
                                        if let Some(c_stem) = c_path.file_stem().and_then(|s| s.to_str()) {
                                            let c_key = normalize_match_key(c_stem);
                                            if c_key.len() >= 3 && (c_key.contains(&target_stem_key) || target_stem_key.contains(&c_key)) {
                                                let diff = (c_key.len() as isize - target_stem_key.len() as isize).unsigned_abs();
                                                if best_match.as_ref().map_or(true, |(_, d)| diff < *d) {
                                                    best_match = Some((c_path, diff));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let Some((best_path, _)) = best_match {
                            found = Some(best_path);
                        }
                    }
                }
            }

            // 6. アプリケーション相対およびサンプル
            if found.is_none() {
                if let Some(resolved) = resolve_app_path(trimmed) {
                    found = Some(resolved);
                } else if trimmed.ends_with("sample.mp4") {
                    found = resolve_app_path("samples/sample_video.mp4");
                }
            }

            if let Some(resolved) = found {
                p = resolved;
            } else {
                self.resolved_paths.insert(raw_trimmed.to_string(), None);
                return None;
            }
        }

        self.resolved_paths.insert(raw_trimmed.to_string(), Some(p.clone()));
        Some(p)
    }

    /// シーン内のすべてのメディアオブジェクトをスキャンし、未検出パスを手元の実在パスに自動再リンクします。
    pub fn auto_relink_missing_media(&mut self) -> usize {
        let mut files_to_resolve = Vec::new();
        for layer in &self.scene.layers {
            for obj in &layer.objects {
                if let Some(src) = obj.source() {
                    if matches!(src.kind, EffectKind::AudioFile | EffectKind::VideoFile | EffectKind::ImageFile) {
                        if let Some(f) = src.text("file").filter(|s| !s.trim().is_empty()) {
                            files_to_resolve.push(f.to_string());
                        }
                    }
                }
            }
        }

        let mut relink_map: HashMap<String, PathBuf> = HashMap::new();
        for f in files_to_resolve {
            if !relink_map.contains_key(&f) {
                if let Some(resolved) = self.resolve_cached_path(&f) {
                    if resolved != Path::new(&f) {
                        relink_map.insert(f, resolved);
                    }
                }
            }
        }

        let mut updated = 0;
        for (old_f, new_p) in &relink_map {
            let new_str = new_p.to_string_lossy().to_string();
            for layer in &mut self.scene.layers {
                for obj in &mut layer.objects {
                    if let Some(src) = obj.source_mut() {
                        if src.text("file") == Some(old_f.as_str()) {
                            if let Some(au2_core::ParamValue::Text(t)) = src.param_mut("file") {
                                *t = new_str.clone();
                                updated += 1;
                            }
                        }
                    }
                }
            }
            self.register_source(new_p);
        }
        if updated > 0 {
            log::info!("未検出メディア {updated} 件を手元の実在ファイルに自動再リンクしました");
        }
        updated
    }

    /// 現在のタイムライン上の音声トラックをオーディオエンジンに同期供給します。
    pub fn sync_audio_playback(&mut self) {
        let fps = self.scene.settings.frame_rate.as_f64();
        let cur_frame = self.frame;

        // 1. 再生対象の候補を収集
        struct Candidate {
            is_audio_obj: bool,
            file_str: String,
            frame_start: Frame,
            frame_end: Frame,
            start_pos: f64,
            volume: f32,
        }
        let mut candidates = Vec::new();

        for layer in &self.scene.layers {
            if !layer.visible {
                continue;
            }
            for obj in &layer.objects {
                let Some(src) = obj.source().filter(|s| s.enabled) else {
                    continue;
                };
                if !matches!(src.kind, EffectKind::AudioFile | EffectKind::VideoFile) {
                    continue;
                }
                let Some(file_str) = src.text("file").filter(|s| !s.trim().is_empty()) else {
                    continue;
                };
                let rel = (cur_frame as i64 - obj.frame_start as i64).max(0) as u32;
                let start_pos = if src.kind == EffectKind::AudioFile {
                    src.eval("再生位置", rel, 0.0).max(0.0)
                } else {
                    ((src.eval("再生位置", rel, 1.0) - 1.0) / fps.max(1.0)).max(0.0)
                };
                let volume = (obj
                    .effect(&EffectKind::StandardPlayback)
                    .map(|e| e.eval("音量", rel, 100.0))
                    .unwrap_or(100.0)
                    / 100.0) as f32;

                candidates.push(Candidate {
                    is_audio_obj: src.kind == EffectKind::AudioFile,
                    file_str: file_str.to_string(),
                    frame_start: obj.frame_start,
                    frame_end: obj.frame_end,
                    start_pos,
                    volume,
                });
            }
        }

        let mut active_tracks = Vec::new();
        let mut registered_keys: std::collections::HashSet<(PathBuf, Frame)> = std::collections::HashSet::new();

        // AudioFile オブジェクトを優先登録し、ペアの VideoFile との2重再生を防止
        candidates.sort_by_key(|c| !c.is_audio_obj);

        for c in candidates {
            let Some(path) = self.resolve_cached_path(&c.file_str) else {
                continue;
            };
            if !registered_keys.insert((path.clone(), c.frame_start)) {
                continue;
            }
            let Some(pcm) = self.media.get_audio_pcm(&path) else {
                continue;
            };
            active_tracks.push(AudioTrackItem {
                audio: pcm,
                start_frame: c.frame_start,
                end_frame: c.frame_end,
                offset_sec: c.start_pos,
                volume: c.volume,
            });
        }

        self.audio.set_active_tracks(active_tracks);
        self.audio.sync_playback(self.playing, self.frame, fps);
    }

    #[allow(dead_code)]
    pub fn new_scene(&mut self) {
        let plugins = self.plugins.clone();
        let sources = std::mem::take(&mut self.sources);
        *self = Self {
            timeline: std::mem::take(&mut self.timeline),
            plugins,
            sources,
            ffmpeg_dl: std::mem::replace(&mut self.ffmpeg_dl, FfmpegDownload::Idle),
            ..Self::default()
        };
        self.status = "新規プロジェクトを作成しました".into();
    }

    /// ソースパネルにメディアファイルを登録します（重複する場合は既存インデックスを返す）。
    pub fn register_source(&mut self, path: &Path) -> Option<usize> {
        let kind = MediaKind::from_path(path)?;
        if let Some(pos) = self.sources.iter().position(|s| s.path == path) {
            self.selected_source = Some(pos);
            return Some(pos);
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let idx = self.sources.len();
        self.sources.push(SourceMediaItem {
            name,
            path: path.to_path_buf(),
            kind,
        });
        self.selected_source = Some(idx);
        Some(idx)
    }

    /// シーン内のオブジェクトが参照している実在メディアファイルをソースパネルに同期登録します。
    pub fn sync_sources_from_scene(&mut self) {
        let mut files = Vec::new();
        for layer in &self.scene.layers {
            for obj in &layer.objects {
                if let Some(src) = obj.source() {
                    if let Some(f) = src.text("file").filter(|s| !s.trim().is_empty()) {
                        files.push(f.to_string());
                    }
                }
            }
        }
        for f in files {
            if let Some(p) = self.resolve_cached_path(&f) {
                let _ = self.register_source(&p);
            }
        }
    }

    /// 画像または動画の指定フレームを GPU テクスチャとしてノンブロッキングで取得します。
    pub fn get_media_texture(
        &mut self,
        ctx: &egui::Context,
        path_str: &str,
        frame_index: u32,
        time_sec: f64,
    ) -> Option<(egui::TextureId, [usize; 2])> {
        let path = self.resolve_cached_path(path_str)?;

        // 静止画の場合は frame_index = 0 で共通化
        let key_frame = match MediaKind::from_path(&path) {
            Some(MediaKind::Image) => 0,
            _ => frame_index,
        };

        // 1. 静止画または動画の先頭サムネイル (frame 0) は static_textures に永続キャッシュ
        if key_frame == 0 {
            if let Some((handle, size)) = self.static_textures.get(&path) {
                return Some((handle.id(), *size));
            }
            let rgba = self.media.get_frame_arc(&path, 0, 0.0)?;
            let tex_size = [rgba.width as usize, rgba.height as usize];
            let display_size = [rgba.original_width as usize, rgba.original_height as usize];
            let color_img = ColorImage::from_rgba_unmultiplied(tex_size, &rgba.pixels);
            let handle = ctx.load_texture(
                format!("media_static:{}", path.display()),
                color_img,
                TextureOptions::LINEAR,
            );
            let id = handle.id();
            self.static_textures.insert(path, (handle, display_size));
            return Some((id, display_size));
        }

        // 2. 動画再生フレーム (frame > 0) はファイルごとに 1 つの TextureHandle を In-Place 更新して再利用
        if let Some((handle, last_frame, size)) = self.stream_textures.get(&path) {
            if *last_frame == key_frame {
                return Some((handle.id(), *size));
            }
        }

        let rgba = self.media.get_frame_arc(&path, key_frame, time_sec)?;
        let tex_size = [rgba.width as usize, rgba.height as usize];
        let display_size = [rgba.original_width as usize, rgba.original_height as usize];
        let color_img = ColorImage::from_rgba_unmultiplied(tex_size, &rgba.pixels);

        if let Some((handle, last_frame, prev_size)) = self.stream_textures.get_mut(&path) {
            handle.set(color_img, TextureOptions::LINEAR);
            *last_frame = key_frame;
            *prev_size = display_size;
            return Some((handle.id(), display_size));
        }

        let handle = ctx.load_texture(
            format!("media_stream:{}", path.display()),
            color_img,
            TextureOptions::LINEAR,
        );
        let id = handle.id();
        self.stream_textures.insert(path, (handle, key_frame, display_size));
        Some((id, display_size))
    }

    /// 指定フレーム位置におけるタイムライン上の音声トラックの振幅（0.0..=1.0）を計算します（PSD 口パク連動用）。
    pub fn sample_audio_amplitude_at(&mut self, abs_frame: Frame) -> f32 {
        let fps = self.scene.settings.frame_rate.as_f64().max(1.0);
        let mut active_audio_items: Vec<(String, f64, f32)> = Vec::new();
        for layer in &self.scene.layers {
            if !layer.visible {
                continue;
            }
            for obj in &layer.objects {
                if !obj.contains(abs_frame) {
                    continue;
                }
                let Some(src) = obj.source().filter(|s| s.enabled) else {
                    continue;
                };
                if !matches!(src.kind, EffectKind::AudioFile | EffectKind::VideoFile) {
                    continue;
                }
                let Some(file_str) = src.text("file").filter(|s| !s.trim().is_empty()) else {
                    continue;
                };
                let rel = abs_frame - obj.frame_start;
                let start_pos = if src.kind == EffectKind::AudioFile {
                    src.eval("再生位置", rel, 0.0).max(0.0)
                } else {
                    ((src.eval("再生位置", rel, 1.0) - 1.0) / fps).max(0.0)
                };
                let speed = (src.eval("再生速度", rel, 100.0) / 100.0).max(0.0);
                let audio_sec = start_pos + (rel as f64 / fps) * speed;
                let volume = (obj
                    .effect(&EffectKind::StandardPlayback)
                    .map(|e| e.eval("音量", rel, 100.0))
                    .unwrap_or(100.0)
                    / 100.0) as f32;
                active_audio_items.push((file_str.to_string(), audio_sec, volume));
            }
        }

        let mut max_amp = 0.0f32;
        let window_stereo_frames = ((44100.0 / fps).round() as usize).clamp(256, 4410);
        for (file_str, audio_sec, volume) in active_audio_items {
            if volume <= 0.001 {
                continue;
            }
            let Some(path) = self.resolve_cached_path(&file_str) else {
                continue;
            };
            let Some(pcm) = self.media.get_audio_pcm(&path) else {
                continue;
            };
            let start_sample_idx = ((audio_sec * 44100.0).round() as usize).saturating_mul(2);
            let end_sample_idx = (start_sample_idx + window_stereo_frames * 2).min(pcm.len());
            if start_sample_idx >= end_sample_idx {
                continue;
            }
            let slice = &pcm[start_sample_idx..end_sample_idx];
            let mut sum_sq = 0.0f64;
            for &s in slice {
                let norm = (s as f64) / 32768.0;
                sum_sq += norm * norm;
            }
            let rms = (sum_sq / (slice.len() as f64)).sqrt() as f32 * volume;
            if rms > max_amp {
                max_amp = rms;
            }
        }
        max_amp.clamp(0.0, 1.0)
    }

    /// PSD / PSB オブジェクトのレイヤー表示設定・目パチ・口パクを反映した GPU テクスチャを取得します。
    pub fn get_psd_object_texture(
        &mut self,
        ctx: &egui::Context,
        file_str: &str,
        overrides: &str,
        blink_str: &str,
        lipsync_str: &str,
        rel_frame: u32,
        abs_frame: Frame,
    ) -> Option<(egui::TextureId, [usize; 2])> {
        let path = self.resolve_cached_path(file_str)?;
        if !au2_media::is_psd_path(&path) {
            return None;
        }
        let fps = self.scene.settings.frame_rate.as_f64().max(1.0);
        let blink = au2_media::PsdBlinkConfig::parse(blink_str);
        let lipsync = au2_media::PsdLipSyncConfig::parse(lipsync_str);
        let audio_amp = if lipsync.mode == au2_media::PsdLipSyncMode::AudioSync {
            self.sample_audio_amplitude_at(abs_frame)
        } else {
            0.0
        };

        let (state_hash, rgba) = self.media.get_psd_composited_frame(
            &path,
            overrides,
            &blink,
            &lipsync,
            rel_frame,
            fps,
            audio_amp,
        )?;

        let cache_key = (path.clone(), state_hash);
        if let Some((handle, size)) = self.psd_textures.get(&cache_key) {
            return Some((handle.id(), *size));
        }

        if self.psd_textures.len() >= 64 {
            self.psd_textures.clear();
        }

        let tex_size = [rgba.width as usize, rgba.height as usize];
        let display_size = [rgba.original_width as usize, rgba.original_height as usize];
        let color_img = ColorImage::from_rgba_unmultiplied(tex_size, &rgba.pixels);
        let handle = ctx.load_texture(
            format!("media_psd:{}:{:x}", path.display(), state_hash),
            color_img,
            TextureOptions::LINEAR,
        );
        let id = handle.id();
        self.psd_textures.insert(cache_key, (handle, display_size));
        Some((id, display_size))
    }

    /// PSD オブジェクトをレイヤー単位またはパーツフォルダ単位で別々のタイムラインオブジェクトに分解・展開します。
    pub fn decompose_psd_object(&mut self, obj_id: ObjectId, by_folders: bool, tight_crop: bool) -> bool {
        let Some((base_layer_idx, orig_obj_ref)) = self.scene.find(obj_id) else {
            return false;
        };
        let orig_obj = orig_obj_ref.clone();
        let Some(src) = orig_obj.source() else {
            return false;
        };
        if src.kind != EffectKind::ImageFile {
            return false;
        }
        let Some(file_str) = src.text("file").map(str::to_string) else {
            return false;
        };
        let layers_spec = src.text("psd_layers").unwrap_or("").to_string();
        let blink_spec = src.text("psd_blink").unwrap_or("").to_string();
        let lipsync_spec = src.text("psd_lipsync").unwrap_or("").to_string();

        let Some(path) = self.resolve_cached_path(&file_str) else {
            self.status = "PSD ファイルが見つからないため分解できません".into();
            return false;
        };
        let Some(doc) = self.media.get_psd_document(&path) else {
            self.status = "PSD データの解析に失敗したため分解できません".into();
            return false;
        };

        let parts = if by_folders {
            doc.decompose_by_folders(&layers_spec, tight_crop)
        } else {
            doc.decompose_by_layers(&layers_spec, tight_crop)
        };
        if parts.is_empty() {
            self.status = "分解可能な表示レイヤーが見つかりませんでした".into();
            return false;
        }

        // 元オブジェクトを削除して、そのレイヤー位置から背面→前面順に各パーツオブジェクトを配置
        let _ = self.scene.remove_object(obj_id);

        let mut cur_target_layer = base_layer_idx;
        let mut first_placed_id = None;
        let mut first_layer = base_layer_idx;
        let mut last_layer = base_layer_idx;
        let part_count = parts.len();

        for part in parts {
            let mut new_obj = orig_obj.clone();
            new_obj.id = ObjectId::new();
            if part.clipping {
                new_obj.clipping = true;
            }

            if let Some(src_eff) = new_obj.source_mut() {
                set_or_remove_text_param(src_eff, "psd_layers", part.psd_layers_param);
                set_or_remove_text_param(src_eff, "psd_label", part.label.clone());
                if part.inherit_blink && !blink_spec.is_empty() {
                    set_or_remove_text_param(src_eff, "psd_blink", blink_spec.clone());
                } else {
                    set_or_remove_text_param(src_eff, "psd_blink", String::new());
                }
                if part.inherit_lipsync && !lipsync_spec.is_empty() {
                    set_or_remove_text_param(src_eff, "psd_lipsync", lipsync_spec.clone());
                } else {
                    set_or_remove_text_param(src_eff, "psd_lipsync", String::new());
                }
            }

            // 描画エフェクトの座標 (X, Y) に切り抜き中心オフセット（拡大率・回転を考慮）を加算し、ブレンドモードを引き継ぐ
            if let Some(draw_eff) = new_obj
                .effects
                .iter_mut()
                .find(|e| matches!(e.kind, EffectKind::StandardDraw | EffectKind::ExtendedDraw))
            {
                let zoom = draw_eff.eval("拡大率", 0, 100.0);
                let rot_deg = draw_eff
                    .track("回転")
                    .or_else(|| draw_eff.track("Z軸回転"))
                    .map(|t| t.evaluate(0))
                    .unwrap_or(0.0);
                let scale = zoom / 100.0;
                let rad = rot_deg.to_radians();
                let dx = part.center_offset.0 * scale;
                let dy = part.center_offset.1 * scale;
                let rot_dx = dx * rad.cos() - dy * rad.sin();
                let rot_dy = dx * rad.sin() + dy * rad.cos();

                if rot_dx.abs() > 1e-4 {
                    if let Some(tx) = draw_eff.track_mut("X") {
                        for kf in &mut tx.keyframes {
                            kf.value = ((kf.value + rot_dx) * 10.0).round() / 10.0;
                        }
                    }
                }
                if rot_dy.abs() > 1e-4 {
                    if let Some(ty) = draw_eff.track_mut("Y") {
                        for kf in &mut ty.keyframes {
                            kf.value = ((kf.value + rot_dy) * 10.0).round() / 10.0;
                        }
                    }
                }
                if part.blend_mode != au2_media::PsdBlendMode::Normal
                    && part.blend_mode != au2_media::PsdBlendMode::PassThrough
                {
                    set_or_remove_text_param(
                        draw_eff,
                        "blend",
                        part.blend_mode.to_aviutl_blend_code().to_string(),
                    );
                }
            }

            let (placed_layer, new_id) = self.scene.insert_object_at_free_layer(cur_target_layer, new_obj);
            if first_placed_id.is_none() {
                first_placed_id = Some(new_id);
                first_layer = placed_layer;
            }
            last_layer = placed_layer;
            cur_target_layer = placed_layer + 1;
        }

        self.selected = first_placed_id;
        self.dirty = true;
        let mode_label = if by_folders { "パーツ(フォルダ)単位" } else { "レイヤー単位" };
        self.status = format!(
            "PSD を{mode_label}で {part_count} 個のオブジェクトに分解し、Layer {}〜{} に配置しました",
            first_layer + 1,
            last_layer + 1
        );
        true
    }

    /// PSD の各パーツ・レイヤーを透過 PNG ファイルとして指定フォルダに一括書き出しするダイアログ。
    pub fn export_psd_layers_png_dialog(&mut self, obj_id: ObjectId, by_folders: bool, tight_crop: bool) {
        let Some((_, orig_obj)) = self.scene.find(obj_id) else {
            return;
        };
        let Some(src) = orig_obj.source() else {
            return;
        };
        let Some(file_str) = src.text("file").map(str::to_string) else {
            return;
        };
        let layers_spec = src.text("psd_layers").unwrap_or("").to_string();
        let Some(path) = self.resolve_cached_path(&file_str) else {
            self.status = "PSD ファイルが見つかりません".into();
            return;
        };
        let Some(doc) = self.media.get_psd_document(&path) else {
            self.status = "PSD ファイルの解析に失敗しました".into();
            return;
        };

        let Some(out_dir) = rfd::FileDialog::new()
            .set_title("PSD レイヤー透過 PNG の書き出し先フォルダを選択")
            .pick_folder()
        else {
            return;
        };

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("psd_layers");
        let target_dir = out_dir.join(format!("{stem}_layers"));
        match doc.export_parts_as_png(&layers_spec, &target_dir, by_folders, tight_crop) {
            Ok(count) => {
                self.status = format!(
                    "PSD の {count} パーツを透過 PNG として「{}」に書き出しました",
                    target_dir.display()
                );
            }
            Err(e) => {
                self.status = format!("PNG 書き出しエラー: {e}");
            }
        }
    }


    /// タイムラインにオブジェクトを追加し、選択状態にします。
    pub fn add_object_at(&mut self, preferred_layer: usize, obj: TimelineObject) {
        let name = obj.display_name();
        if obj.frame_end + 1 > self.scene.settings.length {
            self.scene.settings.length = obj.frame_end + 1;
        }
        let (placed_layer, id) = self.scene.insert_object_at_free_layer(preferred_layer, obj);
        self.selected = Some(id);
        self.dirty = true;
        self.status = format!("レイヤー {} に「{name}」を追加しました", placed_layer + 1);
    }

    /// メディアファイル（画像・動画・音声）をソースパネルに登録しつつ指定レイヤー・指定フレームに追加します。
    pub fn add_media_file(&mut self, path: &Path, preferred_layer: usize, start_frame: Frame) {
        let _ = self.register_source(path);
        let meta = self.media.get_metadata(path);

        // 新規プロジェクト（未保存でメディアがまだ無い状態）の場合、
        // プロジェクト解像度をメディア本来の解像度に自動同期（AviUtl 互換スマート解像度合わせ）
        if self.file_path.is_none() && (self.scene.object_count() == 0 || (self.scene.object_count() <= 1 && !self.dirty)) {
            if meta.width >= 16 && meta.height >= 16 {
                self.scene.settings.width = meta.width;
                self.scene.settings.height = meta.height;
                if meta.fps >= 1.0 {
                    self.scene.settings.frame_rate = au2_core::Rational::new(meta.fps.round() as u32, 1);
                }
            }
        }

        let fps = self.scene.settings.frame_rate.as_f64();
        // メディアファイル（動画・音声・画像）の本来の長さ（フレーム数）を取得（5秒制限を解消）
        let media_len = self.media.get_duration_frames(path, fps).max(1);
        let end_frame = start_frame + media_len - 1;
        if end_frame + 1 > self.scene.settings.length {
            self.scene.settings.length = end_frame + 30;
        }
        let path_str = path.to_string_lossy().into_owned();
        let obj = match MediaKind::from_path(path) {
            Some(MediaKind::Image) => TimelineObject::new_image(start_frame, end_frame, path_str.clone()),
            Some(MediaKind::Video) => TimelineObject::new_video(start_frame, end_frame, path_str.clone()),
            Some(MediaKind::Audio) => TimelineObject::new_audio(start_frame, end_frame, path_str.clone()),
            None => {
                self.status = format!("未対応のメディアファイル形式です: {}", path.display());
                return;
            }
        };
        self.add_object_at(preferred_layer, obj);

        // 動画ファイルで音声ストリームが含まれている場合、AviUtl と同様にペアとなる音声オブジェクトを自動追加
        if matches!(MediaKind::from_path(path), Some(MediaKind::Video)) {
            let _ = self.media.get_frame_arc(path, 0, 0.0);
            if meta.has_audio {
                let _ = self.media.get_audio_pcm(path);
                let mut audio_obj = TimelineObject::new_audio(start_frame, end_frame, path_str);
                if let Some(src) = audio_obj.source_mut() {
                    if let Some(trk) = src.track_mut("動画ファイルと連携") {
                        *trk = Track::constant(1.0, 0);
                    }
                }
                let audio_layer = preferred_layer + 1;
                let _ = self.scene.insert_object_at_free_layer(audio_layer, audio_obj);
            }
        } else if matches!(MediaKind::from_path(path), Some(MediaKind::Audio)) {
            let _ = self.media.get_audio_pcm(path);
        }
    }

    /// メディアファイルを複数選択してタイムライン＆ソースパネルに追加するダイアログ。
    pub fn add_media_dialog(&mut self, preferred_layer: usize, start_frame: Frame) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("動画・画像・PSD・音声ファイルを追加")
            .add_filter(
                "メディアファイル (動画・画像・PSD・音声)",
                &[
                    "mp4", "avi", "mov", "mkv", "webm", "wmv", "png", "jpg", "jpeg", "bmp", "webp",
                    "gif", "psd", "psb", "wav", "mp3", "ogg", "flac",
                ],
            )
            .add_filter("PSD 立ち絵ファイル (*.psd, *.psb)", &["psd", "psb"])
            .add_filter("動画ファイル", &["mp4", "avi", "mov", "mkv", "webm", "wmv"])
            .add_filter("画像ファイル", &["png", "jpg", "jpeg", "bmp", "webp", "gif", "psd", "psb"])
            .add_filter("音声ファイル", &["wav", "mp3", "ogg", "flac"])
            .pick_files()
        {
            for (i, path) in paths.iter().enumerate() {
                self.add_media_file(path, preferred_layer + i, start_frame);
            }
        }
    }

    /// ソースパネルにメディアファイルのみを複数インポートするダイアログ。
    pub fn import_sources_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("ソースパネルにメディア素材を追加")
            .add_filter(
                "メディアファイル (動画・画像・PSD・音声)",
                &[
                    "mp4", "avi", "mov", "mkv", "webm", "wmv", "png", "jpg", "jpeg", "bmp", "webp",
                    "gif", "psd", "psb", "wav", "mp3", "ogg", "flac",
                ],
            )
            .add_filter("PSD 立ち絵ファイル (*.psd, *.psb)", &["psd", "psb"])
            .add_filter("すべてのファイル", &["*"])
            .pick_files()
        {
            let mut count = 0;
            for p in &paths {
                if self.register_source(p).is_some() {
                    count += 1;
                }
            }
            self.status = format!("ソースパネルに {count} 件のメディア素材を追加しました");
        }
    }

    pub fn load_aup2(&mut self, bytes: &[u8], path: Option<&Path>) {
        match au2_aup::parse_aup2_bytes(bytes) {
            Ok(doc) => {
                let name = path.and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
                self.status = format!(
                    "AviUtl2 プロジェクト {} を読み込みました（v{}, {} オブジェクト）",
                    name.as_deref().unwrap_or("無題.aup2"),
                    doc.version,
                    doc.scene.object_count()
                );
                self.scene = doc.scene;
                self.scenes = doc.scenes;
                self.warnings = doc.warnings;
                self.file_path = path.map(Path::to_path_buf);
                self.selected = None;
                self.playing = false;
                self.frame = doc.cursor_frame.min(self.last_frame());
                self.timeline.scroll_x = 0.0;
                self.timeline.scroll_y = 0.0;
                self.dirty = false;
                self.sync_sources_from_scene();
                let auto_relinked = self.auto_relink_missing_media();
                self.sync_audio_playback();
                let missing = self.check_missing_media();
                if !missing.is_empty() {
                    let msg = format!("⚠ プロジェクト内のメディア（音声/動画）が未検出です（{} 件）。手元のファイルを指定してください", missing.len());
                    self.warnings.push(msg.clone());
                    self.status = msg;
                } else if auto_relinked > 0 {
                    self.status = format!("プロジェクトを読み込みました（手元のメディア {auto_relinked} 件を自動照合）");
                }
            }
            Err(e) => {
                self.status = format!(".aup2 の読み込みに失敗しました: {e}");
                log::error!("{}", self.status);
            }
        }
    }

    pub fn load_aup(&mut self, bytes: &[u8], path: Option<&Path>) {
        match au2_aup::parse_aup_bytes(bytes) {
            Ok(doc) => {
                let name = path.and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
                self.status = format!(
                    "プロジェクト {} を読み込みました（AviUtl v{}, {} オブジェクト）",
                    name.as_deref().unwrap_or("無題.aup"),
                    doc.version,
                    doc.scene.object_count()
                );
                self.scene = doc.scene;
                self.scenes = doc.scenes;
                self.warnings = doc.warnings;
                self.file_path = path.map(Path::to_path_buf);
                self.selected = None;
                self.playing = false;
                self.frame = 0;
                self.timeline.scroll_x = 0.0;
                self.timeline.scroll_y = 0.0;
                self.dirty = false;
                self.sync_sources_from_scene();
                let auto_relinked = self.auto_relink_missing_media();
                self.sync_audio_playback();
                let missing = self.check_missing_media();
                if !missing.is_empty() {
                    let msg = format!("⚠ プロジェクト内のメディア（音声/動画）が未検出です（{} 件）。手元のファイルを指定してください", missing.len());
                    self.warnings.push(msg.clone());
                    self.status = msg;
                } else if auto_relinked > 0 {
                    self.status = format!("プロジェクトを読み込みました（手元のメディア {auto_relinked} 件を自動照合）");
                }
            }
            Err(e) => {
                self.status = format!(".aup の読み込みに失敗しました: {e}");
                log::error!("{}", self.status);
            }
        }
    }

    pub fn load_exo(&mut self, bytes: &[u8], path: Option<&Path>) {
        match au2_exo::parse_exo_bytes(bytes) {
            Ok(doc) => {
                let name = path.and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
                self.status = format!(
                    "{} を読み込みました（{} オブジェクト, {:?}, 警告 {} 件）",
                    name.as_deref().unwrap_or("サンプル"),
                    doc.scene.object_count(),
                    doc.encoding,
                    doc.warnings.len()
                );
                for w in &doc.warnings {
                    log::warn!("{w}");
                }
                self.scene = doc.scene;
                self.scenes = std::collections::BTreeMap::new();
                self.scenes.insert(0, self.scene.clone());
                if self.scene.settings.length < 18000 {
                    self.scene.settings.length = self.scene.settings.length.max(self.scene.content_end()).max(18000);
                }
                self.encoding = doc.encoding;
                self.warnings = doc.warnings;
                self.file_path = path.map(Path::to_path_buf);
                self.selected = None;
                self.playing = false;
                self.frame = 0;
                self.timeline.scroll_x = 0.0;
                self.timeline.scroll_y = 0.0;
                self.dirty = false;
                self.sync_sources_from_scene();
                let auto_relinked = self.auto_relink_missing_media();
                self.sync_audio_playback();
                let missing = self.check_missing_media();
                if !missing.is_empty() {
                    let msg = format!("⚠ プロジェクト内のメディア（音声/動画）が未検出です（{} 件）。手元のファイルを指定してください", missing.len());
                    self.warnings.push(msg.clone());
                    self.status = msg;
                } else if auto_relinked > 0 {
                    self.status = format!("オブジェクトファイルを読み込みました（手元のメディア {auto_relinked} 件を自動照合）");
                }
            }
            Err(e) => {
                self.status = format!("読み込みに失敗しました: {e}");
                log::error!("{}", self.status);
            }
        }
    }

    /// 任意のファイルパス（`.aup`, `.exo`, プラグイン, 画像・動画・音声）を自動判別して処理します。
    pub fn load_path(&mut self, path: &Path) {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let lower = ext.to_ascii_lowercase();
            // 1. プラグイン・スクリプト → plugins/ に自動インストール
            if au2_plugin::PluginKind::from_extension(&lower).is_some() {
                match self.plugins.install_file(path) {
                    Ok(name) => self.status = format!("プラグイン `{name}` をインストールしました"),
                    Err(e) => self.status = format!("プラグインのインストールに失敗しました: {e}"),
                }
                return;
            }
            // 2. 画像・動画・音声ファイル → ソースパネルに登録＆タイムラインの現在位置に追加
            if MediaKind::from_path(path).is_some() {
                let preferred_layer = self.timeline.context_target.layer;
                self.add_media_file(path, preferred_layer, self.frame);
                return;
            }
            // 3. AviUtl2 プロジェクトファイル (.aup2)
            if lower == "aup2" {
                match std::fs::read(path) {
                    Ok(bytes) => self.load_aup2(&bytes, Some(path)),
                    Err(e) => self.status = format!("{} を開けません: {e}", path.display()),
                }
                return;
            }
            // 4. 初代 AviUtl プロジェクトファイル (.aup)
            if lower == "aup" {
                match std::fs::read(path) {
                    Ok(bytes) => self.load_aup(&bytes, Some(path)),
                    Err(e) => self.status = format!("{} を開けません: {e}", path.display()),
                }
                return;
            }
            // 5. AviUtl2 エイリアスファイル (.object)
            if lower == "object" {
                match std::fs::read(path) {
                    Ok(bytes) => match au2_aup::parse_object_alias_bytes(&bytes) {
                        Ok(objs) => {
                            let base_layer = self.timeline.context_target.layer;
                            let base_frame = self.frame;
                            let mut count = 0;
                            for (idx, mut obj) in objs.into_iter().enumerate() {
                                let len = obj.length().max(1);
                                obj.frame_start = base_frame + obj.frame_start;
                                obj.frame_end = obj.frame_start + len - 1;
                                self.add_object_at(base_layer + idx, obj);
                                count += 1;
                            }
                            self.sync_sources_from_scene();
                            self.sync_audio_playback();
                            self.status = format!(
                                "AviUtl2 エイリアス `{}` から {} 個のオブジェクトを配置しました",
                                path.file_name().map(|f| f.to_string_lossy()).unwrap_or_default(),
                                count
                            );
                        }
                        Err(e) => self.status = format!(".object の読み込みに失敗しました: {e}"),
                    },
                    Err(e) => self.status = format!("{} を開けません: {e}", path.display()),
                }
                return;
            }
            // 6. 初代 AviUtl エイリアスファイル (.exa)
            if lower == "exa" {
                match std::fs::read(path) {
                    Ok(bytes) => match au2_exo::parse_exa_bytes(&bytes) {
                        Ok(mut obj) => {
                            let base_layer = self.timeline.context_target.layer;
                            let base_frame = self.frame;
                            let len = obj.length().max(1);
                            obj.frame_start = base_frame;
                            obj.frame_end = base_frame + len - 1;
                            self.add_object_at(base_layer, obj);
                            self.sync_sources_from_scene();
                            self.sync_audio_playback();
                            self.status = format!(
                                "拡張編集エイリアス `{}` を配置しました",
                                path.file_name().map(|f| f.to_string_lossy()).unwrap_or_default()
                            );
                        }
                        Err(e) => self.status = format!(".exa の読み込みに失敗しました: {e}"),
                    },
                    Err(e) => self.status = format!("{} を開けません: {e}", path.display()),
                }
                return;
            }
            // 7. テキストファイル (.txt) → テキストオブジェクトとして配置 (exedit.ini 仕様)
            if lower == "txt" {
                match std::fs::read(path) {
                    Ok(bytes) => {
                        let (text, _) = au2_exo::decode_text_bytes(&bytes);
                        let base_layer = self.timeline.context_target.layer;
                        let base_frame = self.frame;
                        let obj = TimelineObject::new_text(base_frame, base_frame + 59, text);
                        self.add_object_at(base_layer, obj);
                        self.status = format!(
                            "テキストファイル `{}` をテキストオブジェクトとして配置しました",
                            path.file_name().map(|f| f.to_string_lossy()).unwrap_or_default()
                        );
                    }
                    Err(e) => self.status = format!("{} を開けません: {e}", path.display()),
                }
                return;
            }
        }
        // 8. 拡張編集オブジェクトファイル (.exo)
        match std::fs::read(path) {
            Ok(bytes) => self.load_exo(&bytes, Some(path)),
            Err(e) => self.status = format!("{} を開けません: {e}", path.display()),
        }
    }

    /// どんなファイル（プロジェクト・動画・画像・音声・プラグイン・スクリプト）でも選んで一括で開く/インストールするスマートダイアログ。
    pub fn smart_open_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("ファイルを開く / 素材・プラグインを取り込む (自動判別)")
            .add_filter(
                "すべての対応ファイル (.aup2, .aup, .exo, .exa, .object, 動画, 画像, PSD, 音声, プラグイン)",
                &[
                    "aup2", "aup", "exo", "exa", "object", "txt", "mp4", "avi", "mov", "mkv", "webm", "wmv", "png",
                    "jpg", "jpeg", "bmp", "webp", "gif", "psd", "psb", "wav", "mp3", "ogg", "flac", "anm", "obj",
                    "cam", "scn", "tra", "auf", "aui", "auo", "auc", "anm2", "obj2", "cam2", "scn2",
                    "tra2", "auf2", "aui2", "auo2", "auc2", "mod2", "aux2",
                ],
            )
            .add_filter(
                "プロジェクト / オブジェクト / エイリアス (.aup2, .aup, .exo, .exa, .object)",
                &["aup2", "aup", "exo", "exa", "object"],
            )
            .add_filter(
                "動画・画像・PSD・音声メディア",
                &[
                    "mp4", "avi", "mov", "mkv", "webm", "wmv", "png", "jpg", "jpeg", "bmp", "webp",
                    "gif", "psd", "psb", "wav", "mp3", "ogg", "flac",
                ],
            )
            .add_filter(
                "初代AviUtl / AviUtl2 プラグイン・スクリプト (.anm, .obj, .auf, .anm2, .auf2, .mod2 等)",
                &[
                    "anm", "obj", "cam", "scn", "tra", "auf", "aui", "auo", "auc", "anm2", "obj2",
                    "cam2", "scn2", "tra2", "auf2", "aui2", "auo2", "auc2", "mod2", "aux2",
                ],
            )
            .add_filter("すべてのファイル", &["*"])
            .pick_files()
        {
            for path in paths {
                self.load_path(&path);
            }
        }
    }

    /// `.aup2` / `.aup` / `.exo` プロジェクトを開くダイアログ。
    pub fn open_project_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("プロジェクトを開く (.aup2 / .aup / .exo)")
            .add_filter("プロジェクト / オブジェクト (.aup2, .aup, .exo)", &["aup2", "aup", "exo"])
            .add_filter("AviUtl2 プロジェクトファイル (.aup2)", &["aup2"])
            .add_filter("初代 AviUtl プロジェクトファイル (.aup)", &["aup"])
            .add_filter("拡張編集オブジェクトファイル (.exo)", &["exo"])
            .add_filter("すべてのファイル", &["*"])
            .pick_file()
        {
            self.load_path(&path);
        }
    }

    /// プロジェクトを保存します（保存先が決まっていれば上書き、未設定ならダイアログ）。
    pub fn save_project(&mut self) -> bool {
        if let Some(path) = self.file_path.clone() {
            match self.write_to_path(&path) {
                Ok(()) => {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    let time_str = self.autosave.last_saved_time_str.as_deref().unwrap_or("");
                    self.status = format!("プロジェクトを {} に保存しました (.{ext}) [{time_str}]", path.display());
                    true
                }
                Err(e) => {
                    self.status = format!("プロジェクト保存に失敗しました: {e}");
                    false
                }
            }
        } else {
            self.save_project_dialog()
        }
    }

    /// `.aup2` (AviUtl2 プロジェクト) または `.aup` として保存するダイアログ。
    pub fn save_project_dialog(&mut self) -> bool {
        let mut dlg = rfd::FileDialog::new()
            .set_title("プロジェクトを保存 (.aup2 / .aup)")
            .add_filter("AviUtl2 プロジェクトファイル (.aup2)", &["aup2"])
            .add_filter("初代 AviUtl プロジェクトファイル (.aup)", &["aup"])
            .set_file_name("project.aup2");
        if let Some(p) = &self.file_path {
            if let Some(dir) = p.parent() {
                dlg = dlg.set_directory(dir);
            }
            if let Some(stem) = p.file_stem() {
                dlg = dlg.set_file_name(format!("{}.aup2", stem.to_string_lossy()));
            }
        }
        if let Some(path) = dlg.save_file() {
            match self.write_to_path(&path) {
                Ok(()) => {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    let time_str = self.autosave.last_saved_time_str.as_deref().unwrap_or("");
                    self.status = format!("プロジェクトを {} に保存しました (.{ext}) [{time_str}]", path.display());
                    self.file_path = Some(path);
                    true
                }
                Err(e) => {
                    self.status = format!("プロジェクト保存に失敗しました: {e}");
                    false
                }
            }
        } else {
            false
        }
    }

    /// `.exo` オブジェクトファイルとしてエクスポートするダイアログ。
    pub fn export_exo_dialog(&mut self) {
        let mut dlg = rfd::FileDialog::new()
            .set_title("オブジェクトファイルのエクスポート (.exo)")
            .add_filter("拡張編集オブジェクトファイル (.exo)", &["exo"])
            .set_file_name("export.exo");
        if let Some(p) = &self.file_path {
            if let Some(dir) = p.parent() {
                dlg = dlg.set_directory(dir);
            }
            if let Some(stem) = p.file_stem() {
                dlg = dlg.set_file_name(format!("{}.exo", stem.to_string_lossy()));
            }
        }
        if let Some(path) = dlg.save_file() {
            let bytes = au2_exo::write_exo_bytes(&self.scene, self.encoding);
            match std::fs::write(&path, bytes) {
                Ok(()) => {
                    self.status = format!("オブジェクトファイルを {} に出力しました (.exo)", path.display());
                }
                Err(e) => self.status = format!(".exo 出力に失敗しました: {e}"),
            }
        }
    }

    /// プラグイン・スクリプトを複数選択して一括インストールするダイアログ。
    pub fn install_plugin_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("プラグイン・スクリプトをインストール (初代AviUtl 32bit / AviUtl2 64bit 両対応)")
            .add_filter(
                "初代AviUtl / AviUtl2 スクリプト・プラグイン",
                &[
                    "anm", "obj", "cam", "scn", "tra", "auf", "aui", "auo", "auc", "anm2", "obj2",
                    "cam2", "scn2", "tra2", "auf2", "aui2", "auo2", "auc2", "mod2", "aux2",
                ],
            )
            .add_filter("すべてのファイル", &["*"])
            .pick_files()
        {
            let mut installed = Vec::new();
            for path in &paths {
                if let Ok(name) = self.plugins.install_file(path) {
                    installed.push(name);
                }
            }
            if !installed.is_empty() {
                self.status = format!(
                    "プラグイン {} 件 ({}) を plugins/ にインストールしました",
                    installed.len(),
                    installed.join(", ")
                );
            }
        }
    }

    /// `.auf` / `.auf2` 自動解析＆クロスプラットフォーム移植・変換スタジオを開きます。
    pub fn open_auf_porter(&mut self, plugin_idx: Option<usize>) {
        if let Some(idx) = plugin_idx {
            self.auf_porter.selected_plugin_idx = idx;
        } else if let Some((idx, _)) = self
            .plugins
            .plugins
            .iter()
            .enumerate()
            .find(|(_, p)| matches!(p.kind, au2_plugin::PluginKind::FilterDll))
        {
            self.auf_porter.selected_plugin_idx = idx;
        }
        self.auf_porter.is_open = true;
    }

    /// 指定した `.auf` / `.auf2` プラグインをクロスプラットフォーム `.anm` スクリプトに変換して `plugins/` に登録します。
    pub fn convert_auf_to_anm(&mut self, plugin_idx: usize) {
        match self.plugins.convert_plugin_to_anm(plugin_idx) {
            Ok(out_path) => {
                let fname = out_path
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.status = format!(
                    "✨ クロスプラットフォーム `.anm` スクリプト `{fname}` を自動生成して登録しました (Windows / Mac / Linux 全OS対応)"
                );
            }
            Err(e) => {
                self.status = format!("⚠ .anm への変換に失敗しました: {e}");
            }
        }
    }

    /// 登録済みの全 `.auf` / `.auf2` プラグインを一括で `.anm` スクリプトに変換します。
    pub fn convert_all_auf_to_anm(&mut self) {
        match self.plugins.convert_all_auf_to_anm() {
            Ok(paths) => {
                self.status = format!(
                    "✨ 全 {} 件の .auf / .auf2 プラグインをクロスプラットフォーム .anm スクリプトへ一括変換しました",
                    paths.len()
                );
            }
            Err(e) => {
                self.status = format!("⚠ 一括変換に失敗しました: {e}");
            }
        }
    }

    /// シーン内の未検出（ファイルが見つからない）メディアオブジェクト一覧を返します。
    pub fn check_missing_media(&mut self) -> Vec<MissingMediaItem> {
        let mut candidates = Vec::new();
        for (layer_idx, layer) in self.scene.layers.iter().enumerate() {
            for obj in &layer.objects {
                if let Some(src) = obj.source() {
                    if matches!(src.kind, EffectKind::AudioFile | EffectKind::VideoFile | EffectKind::ImageFile) {
                        if let Some(file_str) = src.text("file").filter(|s| !s.trim().is_empty()) {
                            let kind_name = match src.kind {
                                EffectKind::AudioFile => "音声 (MP3/WAV)",
                                EffectKind::VideoFile => "動画 (MP4/AVI)",
                                EffectKind::ImageFile => "画像",
                                _ => "メディア",
                            };
                            candidates.push((
                                obj.id,
                                layer_idx,
                                kind_name.to_string(),
                                file_str.to_string(),
                                obj.frame_start,
                                obj.frame_end,
                            ));
                        }
                    }
                }
            }
        }

        let mut missing = Vec::new();
        for (id, layer_idx, kind_name, file_str, start, end) in candidates {
            if self.resolve_cached_path(&file_str).is_none() {
                missing.push(MissingMediaItem {
                    object_id: id,
                    layer_index: layer_idx,
                    kind_name,
                    original_path: file_str,
                    frame_start: start,
                    frame_end: end,
                });
            }
        }
        missing
    }

    /// 未検出メディアや既存オブジェクトの参照ファイルを新しいパスに再設定（再リンク）します。
    /// `target_id` が None の場合、または `old_path` と一致する全オブジェクトを一括更新します。
    pub fn relink_media(&mut self, target_id: Option<ObjectId>, old_path: &str, new_path: &Path) {
        let new_path_str = new_path.to_string_lossy().to_string();
        let mut count = 0;
        for layer in &mut self.scene.layers {
            for obj in &mut layer.objects {
                let is_target = target_id.map_or(false, |id| obj.id == id);
                let has_same_path = obj.source().and_then(|s| s.text("file")).map_or(false, |f| f == old_path);

                if is_target || (target_id.is_none() && has_same_path) || (has_same_path && !old_path.is_empty()) {
                    if let Some(src) = obj.source_mut() {
                        if let Some(au2_core::ParamValue::Text(t)) = src.param_mut("file") {
                            *t = new_path_str.clone();
                        } else {
                            src.push_text("file", new_path_str.clone());
                        }
                        count += 1;
                    }
                }
            }
        }

        // キャッシュクリア＆再キャッシュ
        self.resolved_paths.clear();
        self.resolved_paths.insert(new_path_str.clone(), Some(new_path.to_path_buf()));
        if !old_path.is_empty() {
            self.resolved_paths.insert(old_path.to_string(), Some(new_path.to_path_buf()));
        }
        self.register_source(new_path);
        self.sync_sources_from_scene();
        self.sync_audio_playback();
        self.dirty = true;
        let base_name = new_path.file_name().and_then(|n| n.to_str()).unwrap_or(&new_path_str);
        self.status = format!("メディアファイルを `{base_name}` に再設定しました（{count} 件のオブジェクトを更新）");
    }

    /// 指定フォルダ（サブフォルダ含む）を素材検索先に追加し、未検出メディアを一括で再リンクします。
    /// AviUtl の .aup はメディア本体を内包せず絶対パスしか持たないため、別 PC で開いた場合は
    /// 素材の入ったフォルダを指定してもらうことで、ファイル名から全素材を自動で割り当てます。
    /// 戻り値は `(再リンクできた件数, まだ見つからない素材の件数)`。
    pub fn relink_from_folder(&mut self, folder: &Path) -> (usize, usize) {
        let folder = folder.to_path_buf();
        if !self.extra_search_dirs.contains(&folder) {
            self.extra_search_dirs.insert(0, folder);
        }
        // 未検出として記憶した結果を破棄して再探索
        self.resolved_paths.clear();
        let relinked = self.auto_relink_missing_media();
        self.sync_sources_from_scene();
        self.sync_audio_playback();
        let remaining = self.check_missing_media().len();
        if relinked > 0 {
            self.dirty = true;
        }
        self.status = format!(
            "フォルダから素材を再リンクしました（{relinked} 件を割り当て、未検出 {remaining} 件）"
        );
        (relinked, remaining)
    }

    /// 素材フォルダを選択して未検出メディアを一括再リンクするダイアログを開きます。
    pub fn relink_from_folder_dialog(&mut self) -> bool {
        let mut dlg = rfd::FileDialog::new().set_title("素材（音声・画像・動画）が入っているフォルダを選択");
        if let Some(parent) = self.file_path.as_ref().and_then(|p| p.parent()) {
            if parent.exists() {
                dlg = dlg.set_directory(parent);
            }
        }
        if let Some(dir) = dlg.pick_folder() {
            self.relink_from_folder(&dir);
            true
        } else {
            false
        }
    }

    /// 未検出メディアをユーザー選択ファイルに再リンクするダイアログを開きます。
    pub fn relink_media_dialog(&mut self, target_id: Option<ObjectId>, old_path: &str, is_audio: bool) -> bool {
        let mut dlg = rfd::FileDialog::new()
            .set_title(if is_audio {
                "手元の音声ファイル (MP3 / WAV / M4A 等) を選択して割り当て"
            } else {
                "手元のメディアファイルを選択して割り当て"
            });

        // プロジェクトファイルの親フォルダがあれば最優先で初期ディレクトリに設定
        let mut dir_set = false;
        if let Some(proj) = &self.file_path {
            if let Some(parent) = proj.parent() {
                if parent.exists() {
                    dlg = dlg.set_directory(parent);
                    dir_set = true;
                }
            }
        }
        if !dir_set {
            if let Some(desktop) = au2_platform::desktop_dir() {
                if desktop.exists() {
                    dlg = dlg.set_directory(desktop);
                }
            }
        }

        if is_audio {
            dlg = dlg.add_filter("音声ファイル (*.mp3, *.wav, *.m4a, *.flac, *.ogg, *.aac)", &["mp3", "wav", "m4a", "flac", "ogg", "aac"]);
        } else {
            dlg = dlg.add_filter("メディアファイル", &["mp3", "wav", "m4a", "flac", "ogg", "aac", "mp4", "avi", "mov", "png", "jpg", "jpeg"]);
        }
        dlg = dlg.add_filter("すべてのファイル (*.*)", &["*"]);

        if let Some(p) = dlg.pick_file() {
            self.relink_media(target_id, old_path, &p);
            true
        } else {
            false
        }
    }

    /// メディアキャッシュを破棄し、ソース一覧とオーディオ再生キューを最新状態に再同期します。
    pub fn invalidate_media_cache(&mut self) {
        self.resolved_paths.clear();
        self.sync_sources_from_scene();
        self.sync_audio_playback();
        self.dirty = true;
    }
}

fn set_or_remove_text_param(eff: &mut au2_core::Effect, key: &str, value: String) {
    if value.is_empty() {
        eff.params.retain(|p| p.key != key);
    } else if let Some(p) = eff.params.iter_mut().find(|p| p.key == key) {
        p.value = au2_core::ParamValue::Text(value);
    } else {
        eff.params.push(au2_core::Param {
            key: key.to_string(),
            value: au2_core::ParamValue::Text(value),
        });
    }
}

