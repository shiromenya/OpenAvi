//! エディタの状態（UI から独立したアプリケーション状態）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use au2_core::{Frame, ObjectId, Scene, SceneSettings, TimelineObject};
use au2_exo::TextEncoding;
use au2_media::{MediaCache, MediaKind};
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
}

#[derive(Clone, Copy, Debug)]
pub enum TimelineDrag {
    /// 再生位置のスクラブ
    Scrub,
    /// オブジェクト移動。`grab_offset` は掴んだ位置とオブジェクト先頭の差（フレーム）
    Move { id: ObjectId, grab_offset: i64 },
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
        }
    }
}

pub struct EditorState {
    pub scene: Scene,
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
    /// ソースパネルに登録されたメディア素材一覧
    pub sources: Vec<SourceMediaItem>,
    /// ソースパネルで現在プレビュー選択中の素材インデックス
    pub selected_source: Option<usize>,
    /// デコード済み RGBA フレームの GPU テクスチャキャッシュ
    textures: HashMap<(PathBuf, u32), (TextureHandle, [usize; 2])>,
    pub dirty: bool,
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
            sources: initial_sources(),
            selected_source: None,
            textures: HashMap::new(),
            dirty: false,
        }
    }
}

impl EditorState {
    /// 編集可能な最終フレーム。
    pub fn last_frame(&self) -> Frame {
        self.scene.settings.length.max(self.scene.content_end()).max(1) - 1
    }

    pub fn set_frame(&mut self, frame: Frame) {
        self.frame = frame.min(self.last_frame());
        if self.playing {
            self.play_origin = Some((Instant::now(), self.frame));
        }
    }

    pub fn step(&mut self, delta: i64) {
        let f = (self.frame as i64 + delta).max(0) as Frame;
        self.set_frame(f);
    }

    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.play_origin = self.playing.then(|| (Instant::now(), self.frame));
    }

    /// 再生中なら経過時間からフレームを進めます（末尾でループ）。
    pub fn tick(&mut self) {
        let Some((origin, start)) = self.play_origin.filter(|_| self.playing) else { return };
        let fps = self.scene.settings.frame_rate;
        let mut f = start + fps.seconds_to_frame(origin.elapsed().as_secs_f64());
        if f > self.last_frame() {
            f = 0;
            self.play_origin = Some((Instant::now(), 0));
        }
        self.frame = f;
    }

    pub fn new_scene(&mut self) {
        let plugins = self.plugins.clone();
        let sources = std::mem::take(&mut self.sources);
        *self = Self {
            timeline: std::mem::take(&mut self.timeline),
            plugins,
            sources,
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
        let mut paths = Vec::new();
        for layer in &self.scene.layers {
            for obj in &layer.objects {
                if let Some(src) = obj.source() {
                    if let Some(f) = src.text("file") {
                        let p = PathBuf::from(f);
                        if p.exists() {
                            paths.push(p);
                        }
                    }
                }
            }
        }
        for p in paths {
            let _ = self.register_source(&p);
        }
    }

    /// 画像または動画の指定フレームを GPU テクスチャとして取得します。
    pub fn get_media_texture(
        &mut self,
        ctx: &egui::Context,
        path_str: &str,
        frame_index: u32,
        time_sec: f64,
    ) -> Option<(egui::TextureId, [usize; 2])> {
        let path = PathBuf::from(path_str);
        // 静止画の場合は frame_index = 0 で共通化
        let key_frame = match MediaKind::from_path(&path) {
            Some(MediaKind::Image) => 0,
            _ => frame_index,
        };
        let key = (path.clone(), key_frame);
        if let Some((tex, size)) = self.textures.get(&key) {
            return Some((tex.id(), *size));
        }

        let rgba = self.media.get_frame(&path, key_frame, time_sec)?;
        let size = [rgba.width as usize, rgba.height as usize];
        let color_img = ColorImage::from_rgba_unmultiplied(size, &rgba.pixels);

        if self.textures.len() >= 120 {
            self.textures.clear();
        }
        let handle = ctx.load_texture(
            format!("media:{}:{key_frame}", path.display()),
            color_img,
            TextureOptions::LINEAR,
        );
        let id = handle.id();
        self.textures.insert(key, (handle, size));
        Some((id, size))
    }

    /// タイムラインにオブジェクトを追加し、選択状態にします。
    pub fn add_object_at(&mut self, preferred_layer: usize, obj: TimelineObject) {
        let name = obj.display_name();
        let (placed_layer, id) = self.scene.insert_object_at_free_layer(preferred_layer, obj);
        self.selected = Some(id);
        self.dirty = true;
        self.status = format!("レイヤー {} に「{name}」を追加しました", placed_layer + 1);
    }

    /// メディアファイル（画像・動画・音声）をソースパネルに登録しつつ指定レイヤー・指定フレームに追加します。
    pub fn add_media_file(&mut self, path: &Path, preferred_layer: usize, start_frame: Frame) {
        let _ = self.register_source(path);
        let default_len = 150;
        let end_frame = start_frame + default_len - 1;
        let path_str = path.to_string_lossy().into_owned();
        let obj = match MediaKind::from_path(path) {
            Some(MediaKind::Image) => TimelineObject::new_image(start_frame, end_frame, path_str),
            Some(MediaKind::Video) => TimelineObject::new_video(start_frame, end_frame, path_str),
            Some(MediaKind::Audio) => TimelineObject::new_audio(start_frame, end_frame, path_str),
            None => {
                self.status = format!("未対応のメディアファイル形式です: {}", path.display());
                return;
            }
        };
        self.add_object_at(preferred_layer, obj);
    }

    /// メディアファイルを複数選択してタイムライン＆ソースパネルに追加するダイアログ。
    pub fn add_media_dialog(&mut self, preferred_layer: usize, start_frame: Frame) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("動画・画像・音声ファイルを追加")
            .add_filter(
                "メディアファイル (動画・画像・音声)",
                &[
                    "mp4", "avi", "mov", "mkv", "webm", "wmv", "png", "jpg", "jpeg", "bmp", "webp",
                    "gif", "wav", "mp3", "ogg", "flac",
                ],
            )
            .add_filter("動画ファイル", &["mp4", "avi", "mov", "mkv", "webm", "wmv"])
            .add_filter("画像ファイル", &["png", "jpg", "jpeg", "bmp", "webp", "gif"])
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
                "メディアファイル (動画・画像・音声)",
                &[
                    "mp4", "avi", "mov", "mkv", "webm", "wmv", "png", "jpg", "jpeg", "bmp", "webp",
                    "gif", "wav", "mp3", "ogg", "flac",
                ],
            )
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
                self.warnings = doc.warnings;
                self.file_path = path.map(Path::to_path_buf);
                self.selected = None;
                self.playing = false;
                self.frame = doc.cursor_frame.min(self.last_frame());
                self.timeline.scroll_x = 0.0;
                self.timeline.scroll_y = 0.0;
                self.dirty = false;
                self.sync_sources_from_scene();
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
                self.warnings = doc.warnings;
                self.file_path = path.map(Path::to_path_buf);
                self.selected = None;
                self.playing = false;
                self.frame = 0;
                self.timeline.scroll_x = 0.0;
                self.timeline.scroll_y = 0.0;
                self.dirty = false;
                self.sync_sources_from_scene();
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
        }
        // 5. 拡張編集オブジェクトファイル (.exo)
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
                "すべての対応ファイル (.aup2, .aup, .exo, 動画, 画像, 音声, プラグイン)",
                &[
                    "aup2", "aup", "exo", "mp4", "avi", "mov", "mkv", "webm", "wmv", "png", "jpg",
                    "jpeg", "bmp", "webp", "gif", "wav", "mp3", "ogg", "flac", "anm", "obj", "cam",
                    "scn", "tra", "auf", "aui", "auo", "auc",
                ],
            )
            .add_filter("プロジェクト / オブジェクト (.aup2, .aup, .exo)", &["aup2", "aup", "exo"])
            .add_filter(
                "動画・画像・音声メディア",
                &[
                    "mp4", "avi", "mov", "mkv", "webm", "wmv", "png", "jpg", "jpeg", "bmp", "webp",
                    "gif", "wav", "mp3", "ogg", "flac",
                ],
            )
            .add_filter(
                "AviUtl プラグイン / スクリプト (.anm, .obj, .auf 等)",
                &["anm", "obj", "cam", "scn", "tra", "auf", "aui", "auo", "auc"],
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

    /// `.aup2` (AviUtl2 プロジェクト) または `.aup` として保存するダイアログ。
    pub fn save_project_dialog(&mut self) {
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
            let is_legacy_aup = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("aup"));
            let bytes = if is_legacy_aup {
                au2_aup::write_aup_bytes(&self.scene)
            } else {
                au2_aup::write_aup2_bytes(&self.scene, Some(&path), self.frame)
            };
            match std::fs::write(&path, bytes) {
                Ok(()) => {
                    let kind_label = if is_legacy_aup { ".aup" } else { ".aup2" };
                    self.status = format!("プロジェクトを {} に保存しました ({kind_label})", path.display());
                    self.file_path = Some(path);
                    self.dirty = false;
                }
                Err(e) => self.status = format!("プロジェクト保存に失敗しました: {e}"),
            }
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
            .set_title("プラグイン・スクリプトをインストール")
            .add_filter(
                "AviUtl スクリプト / プラグイン",
                &["anm", "obj", "cam", "scn", "tra", "auf", "aui", "auo", "auc"],
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
}
