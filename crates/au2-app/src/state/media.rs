//! メディアファイル探索・パス解決・テクスチャキャッシュ・音声波形・PSD・素材再リンク・タイムライン配置処理。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use au2_core::{EffectKind, Frame, ObjectId, TimelineObject, Track};
use au2_media::MediaKind;
use eframe::egui::{self, ColorImage, TextureOptions};

use super::{EditorState, MissingMediaItem, SourceMediaItem};

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
                if let Some(resolved) = super::types::resolve_app_path(trimmed) {
                    found = Some(resolved);
                } else if trimmed.ends_with("sample.mp4") {
                    found = super::types::resolve_app_path("samples/sample_video.mp4");
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

        // 1. 静止画または動画の先頭サムネイル (frame 0) は static_textures に LRU キャッシュ
        if key_frame == 0 {
            self.texture_seq = self.texture_seq.wrapping_add(1);
            let cur_seq = self.texture_seq;
            if let Some((handle, size, seq)) = self.static_textures.get_mut(&path) {
                *seq = cur_seq;
                return Some((handle.id(), *size));
            }
            // 上限 256 件を超えたら最も古い 32 件を自動解放して VRAM/メモリを最適化
            if self.static_textures.len() >= 256 {
                let mut entries: Vec<(PathBuf, u64)> = self.static_textures.iter().map(|(p, (_, _, s))| (p.clone(), *s)).collect();
                entries.sort_by_key(|e| e.1);
                for (old_path, _) in entries.into_iter().take(32) {
                    self.static_textures.remove(&old_path);
                }
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
            self.static_textures.insert(path, (handle, display_size, cur_seq));
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

    /// 指定フレーム位置におけるタイムライン上の音声トラックの波形配列（`bins` 個の振幅 `0.0..=1.0`）を取得します（音声波形表示オブジェクト連動用）。
    /// アクティブな音声・動画クリップが存在しない場合は空の `Vec` を返します。
    pub fn sample_audio_waveform_at(&mut self, abs_frame: Frame, bins: usize) -> Vec<f32> {
        let bins = bins.max(1);
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
        if active_audio_items.is_empty() {
            return Vec::new();
        }

        let mut out = vec![0.0f32; bins];
        let mut any_pcm = false;
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
            any_pcm = true;
            let start_sample_idx = ((audio_sec * 44100.0).round() as usize).saturating_mul(2);
            let total_samples = window_stereo_frames * 2;
            let end_sample_idx = (start_sample_idx + total_samples).min(pcm.len());
            if start_sample_idx >= end_sample_idx {
                continue;
            }
            let slice = &pcm[start_sample_idx..end_sample_idx];
            let chunk_len = (slice.len() / bins).max(1);
            for (b, slot) in out.iter_mut().enumerate() {
                let c_start = (b * chunk_len).min(slice.len());
                let c_end = ((b + 1) * chunk_len).min(slice.len());
                if c_start >= c_end {
                    continue;
                }
                let mut peak = 0.0f32;
                for &s in &slice[c_start..c_end] {
                    let v = (s as f32 / 32768.0).abs() * volume;
                    if v > peak {
                        peak = v;
                    }
                }
                *slot = (*slot + peak).clamp(0.0, 1.0);
            }
        }
        if any_pcm {
            out
        } else {
            Vec::new()
        }
    }

    /// メディアファイルのオーディオ波形ピークデータ（タイムライン描画用）を取得します。
    pub fn get_audio_waveform(&mut self, path_str: &str) -> Option<std::sync::Arc<au2_media::AudioWaveformPeaks>> {
        let path = self.resolve_cached_path(path_str)?;
        self.media.get_audio_waveform(&path)
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

        self.texture_seq = self.texture_seq.wrapping_add(1);
        let cur_seq = self.texture_seq;

        let cache_key = (path.clone(), state_hash);
        if let Some((handle, size, seq)) = self.psd_textures.get_mut(&cache_key) {
            *seq = cur_seq;
            return Some((handle.id(), *size));
        }

        // 上限 128 件を超えたら最も古い 32 件を自動解放（全消去によるフレームドロップを完全防止）
        if self.psd_textures.len() >= 128 {
            let mut entries: Vec<((PathBuf, u64), u64)> = self.psd_textures.iter().map(|(k, (_, _, s))| (k.clone(), *s)).collect();
            entries.sort_by_key(|e| e.1);
            for (old_key, _) in entries.into_iter().take(32) {
                self.psd_textures.remove(&old_key);
            }
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
        self.psd_textures.insert(cache_key, (handle, display_size, cur_seq));
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

        self.push_undo("PSD レイヤー分解");

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
        self.push_undo(format!("オブジェクト「{name}」を追加"));
        if obj.frame_end + 1 > self.scene.settings.length {
            self.scene.settings.length = obj.frame_end + 1;
        }
        let (placed_layer, id) = self.scene.insert_object_at_free_layer(preferred_layer, obj);
        self.select_single(id);
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
        self.invalidate_audio_sync_cache();
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
        self.invalidate_audio_sync_cache();
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
        self.invalidate_audio_sync_cache();
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
