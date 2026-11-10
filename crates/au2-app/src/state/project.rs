//! プロジェクト作成・入出力（AUP2 / AUP / EXO）・ダイアログ処理。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use au2_core::{Frame, Scene, SceneSettings, TimelineObject};
use au2_exo::TextEncoding;
use au2_media::{AudioEngine, MediaCache, MediaKind};

use super::{
    current_time_string, AufPorterModalState, AutoSaveSettings, EditorState, FfmpegDownload,
    HistoryManager, NewProjectConfig, TimelineView,
};

impl EditorState {
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
            selected_objects: HashSet::new(),
            clipboard: Vec::new(),
            history: HistoryManager::default(),
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
            texture_seq: 0,
            preview_drag: None,
            preview_snap_x: false,
            preview_snap_y: false,
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
                self.selected_objects.clear();
                self.history = HistoryManager::default();
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
                self.selected_objects.clear();
                self.history = HistoryManager::default();
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
                self.selected_objects.clear();
                self.history = HistoryManager::default();
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

    /// 任意のファイルパス（`.aup`, `.exo`, プラグイン, 画像・動画・音声）を指定のレイヤー・フレーム位置に配置します。
    pub fn load_path_at(&mut self, path: &Path, preferred_layer: usize, start_frame: Frame) {
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
            // 2. 画像・動画・音声ファイル → ソースパネルに登録＆タイムラインの指定位置に追加
            if MediaKind::from_path(path).is_some() {
                self.add_media_file(path, preferred_layer, start_frame);
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
                            let mut count = 0;
                            for (idx, mut obj) in objs.into_iter().enumerate() {
                                let len = obj.length().max(1);
                                obj.frame_start = start_frame + obj.frame_start;
                                obj.frame_end = obj.frame_start + len - 1;
                                self.add_object_at(preferred_layer + idx, obj);
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
                            let len = obj.length().max(1);
                            obj.frame_start = start_frame;
                            obj.frame_end = start_frame + len - 1;
                            self.add_object_at(preferred_layer, obj);
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
                        let obj = TimelineObject::new_text(start_frame, start_frame + 59, text);
                        self.add_object_at(preferred_layer, obj);
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

    /// 任意のファイルパス（`.aup`, `.exo`, プラグイン, 画像・動画・音声）を自動判別して処理します。
    pub fn load_path(&mut self, path: &Path) {
        let preferred_layer = self.timeline.context_target.layer;
        let start_frame = self.frame;
        self.load_path_at(path, preferred_layer, start_frame);
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
}
