//! # au2-media
//!
//! OpenAvi の画像・動画デコード、完全非同期バックグラウンド・動画ストリーミングキャッシュ、
//! および Windows ネイティブオーディオ再生パイプライン。
//!
//! - **画像ファイル**: `.png`, `.jpg`, `.jpeg`, `.bmp`, `.webp`, `.gif` をネイティブデコード。
//! - **動画ファイル**: バックグラウンドスレッドで `ffmpeg` 連続パイプを駆動し、メインスレッドの待ち時間 0ms で 60fps 滑らかな再生を実現。
//! - **音声ファイル / 動画音声**: 44.1kHz 16-bit ステレオ PCM にデコードし、低遅延リアルタイム出力。

pub mod audio;

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub use audio::{decode_audio_pcm, AudioEngine, AudioTrackItem, DecodedAudio};
use image::AnimationDecoder;

/// RGBA8 ピクセルバッファ。
#[derive(Debug, Clone)]
pub struct RgbaFrame {
    pub width: u32,
    pub height: u32,
    pub original_width: u32,
    pub original_height: u32,
    pub pixels: Vec<u8>,
}

/// ファイル拡張子からメディア種別を判別します。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

impl MediaKind {
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "png" | "jpg" | "jpeg" | "bmp" | "webp" => Some(Self::Image),
            "gif" | "mp4" | "avi" | "mov" | "mkv" | "webm" | "wmv" | "m4v" => Some(Self::Video),
            "wav" | "mp3" | "ogg" | "flac" | "aac" | "m4a" => Some(Self::Audio),
            _ => None,
        }
    }
}

/// メディアファイルのメタデータ情報。
#[derive(Debug, Clone, Default)]
pub struct MediaMetadata {
    pub width: u32,
    pub height: u32,
    pub duration_sec: f64,
    pub fps: f64,
    pub has_audio: bool,
    pub has_video: bool,
}

/// バックグラウンド連続動画デコーダとメインスレッド間で共有されるフレームバッファ。
struct VideoStreamShared {
    /// 先頭フレーム (frame 0)。タイムラインやソースパネルのサムネイル用に永久保持する。
    first_frame: Option<Arc<RgbaFrame>>,
    /// デコード済みフレーム (30fps 換算の frame_index -> Arc<RgbaFrame>)
    frames: HashMap<u32, Arc<RgbaFrame>>,
    /// 現在メインスレッドが再生・表示しようとしているフレーム番号
    target_frame: u32,
    /// ワーカースレッド停止フラグ
    stop: bool,
}

struct VideoStreamHandle {
    shared: Arc<(Mutex<VideoStreamShared>, Condvar)>,
}

impl Drop for VideoStreamHandle {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.shared;
        if let Ok(mut st) = lock.lock() {
            st.stop = true;
            cvar.notify_all();
        }
    }
}

/// 静止画または動画のキャッシュエントリ。
enum CachedAsset {
    Static(Arc<RgbaFrame>),
    AnimatedGif(Vec<Arc<RgbaFrame>>),
    VideoStream(VideoStreamHandle),
    Failed,
}

/// メディアデコーダ＆キャッシュ。
pub struct MediaCache {
    assets: HashMap<PathBuf, CachedAsset>,
    metadata_cache: HashMap<PathBuf, MediaMetadata>,
    /// 音声キャッシュ (`None` の場合もキャッシュして毎フレームの失敗再試行を完全に防止)
    audio_cache: HashMap<PathBuf, Option<Arc<Vec<i16>>>>,
    ffmpeg_bin: Option<PathBuf>,
    ffprobe_bin: Option<PathBuf>,
}

impl Default for MediaCache {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaCache {
    pub fn new() -> Self {
        let (ffmpeg_bin, ffprobe_bin) = locate_ffmpeg_tools();
        if let Some(p) = &ffmpeg_bin {
            log::info!("FFmpeg 検出: {}", p.display());
        }
        Self {
            assets: HashMap::new(),
            metadata_cache: HashMap::new(),
            audio_cache: HashMap::new(),
            ffmpeg_bin,
            ffprobe_bin,
        }
    }

    pub fn has_ffmpeg(&self) -> bool {
        self.ffmpeg_bin.is_some()
    }

    /// FFmpeg を再探索し、検出できた場合は失敗キャッシュを破棄して動画・音声を再デコード可能にします。
    /// （アプリ内からの自動ダウンロード完了後などに呼び出します）
    pub fn reload_ffmpeg(&mut self) -> bool {
        let (ffmpeg_bin, ffprobe_bin) = locate_ffmpeg_tools();
        self.ffmpeg_bin = ffmpeg_bin;
        self.ffprobe_bin = ffprobe_bin;
        self.assets.retain(|_, a| !matches!(a, CachedAsset::Failed));
        self.metadata_cache.clear();
        self.audio_cache.retain(|_, v| v.is_some());
        self.ffmpeg_bin.is_some()
    }

    /// メディアのメタデータ（解像度・長さ・ストリーム有無）を取得します（キャッシュ対応）。
    pub fn get_metadata(&mut self, path: &Path) -> MediaMetadata {
        if let Some(meta) = self.metadata_cache.get(path) {
            return meta.clone();
        }

        let meta = match MediaKind::from_path(path) {
            Some(MediaKind::Image) => {
                if let Ok(dim) = image::image_dimensions(path) {
                    MediaMetadata {
                        width: dim.0,
                        height: dim.1,
                        duration_sec: 5.0,
                        fps: 30.0,
                        has_audio: false,
                        has_video: false,
                    }
                } else {
                    MediaMetadata {
                        width: 640,
                        height: 360,
                        duration_sec: 5.0,
                        fps: 30.0,
                        has_audio: false,
                        has_video: false,
                    }
                }
            }
            Some(MediaKind::Audio) => {
                if let Some(probe) = &self.ffprobe_bin {
                    let mut meta = probe_media_metadata(probe, path).unwrap_or(MediaMetadata {
                        width: 0,
                        height: 0,
                        duration_sec: 180.0,
                        fps: 30.0,
                        has_audio: true,
                        has_video: false,
                    });
                    meta.has_audio = true;
                    meta
                } else {
                    MediaMetadata {
                        width: 0,
                        height: 0,
                        duration_sec: 180.0,
                        fps: 30.0,
                        has_audio: true,
                        has_video: false,
                    }
                }
            }
            _ => {
                if let Some(probe) = &self.ffprobe_bin {
                    probe_media_metadata(probe, path).unwrap_or(MediaMetadata {
                        width: 640,
                        height: 360,
                        duration_sec: 5.0,
                        fps: 30.0,
                        has_audio: false,
                        has_video: true,
                    })
                } else {
                    MediaMetadata {
                        width: 640,
                        height: 360,
                        duration_sec: 5.0,
                        fps: 30.0,
                        has_audio: false,
                        has_video: true,
                    }
                }
            }
        };

        self.metadata_cache.insert(path.to_path_buf(), meta.clone());
        meta
    }

    /// 指定 FPS におけるメディアの総フレーム数を取得します。
    pub fn get_duration_frames(&mut self, path: &Path, fps: f64) -> u32 {
        let meta = self.get_metadata(path);
        if meta.duration_sec > 0.0 {
            (meta.duration_sec * fps.max(1.0)).round().max(1.0) as u32
        } else {
            (5.0 * fps.max(1.0)).round().max(1.0) as u32
        }
    }

    /// メディアから抽出された PCM 音声データを取得します（音声なしファイルもキャッシュして再実行を防止）。
    pub fn get_audio_pcm(&mut self, path: &Path) -> Option<Arc<Vec<i16>>> {
        if let Some(cached) = self.audio_cache.get(path) {
            return cached.clone();
        }
        // メタデータで音声ストリームが無いことが分かっている場合は ffmpeg を起動せず即座に None をキャッシュ
        let meta = self.get_metadata(path);
        if matches!(MediaKind::from_path(path), Some(MediaKind::Video)) && !meta.has_audio {
            self.audio_cache.insert(path.to_path_buf(), None);
            return None;
        }
        let Some(ffmpeg) = self.ffmpeg_bin.as_ref() else {
            self.audio_cache.insert(path.to_path_buf(), None);
            return None;
        };
        let result = decode_audio_pcm(ffmpeg, path).map(|d| d.samples);
        self.audio_cache.insert(path.to_path_buf(), result.clone());
        result
    }

    /// 静止画または動画の指定フレームの [`Arc<RgbaFrame>`] をノンブロッキングで取得します。
    pub fn get_frame_arc(&mut self, path: &Path, frame_index: u32, _time_sec: f64) -> Option<Arc<RgbaFrame>> {
        if !self.assets.contains_key(path) {
            if !path.exists() {
                return None;
            }
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();

            let asset = match ext.as_str() {
                "gif" => load_gif_or_static(path),
                "png" | "jpg" | "jpeg" | "bmp" | "webp" => load_static_image(path),
                _ => self.init_video_stream(path),
            };
            self.assets.insert(path.to_path_buf(), asset);
        }

        match self.assets.get(path)? {
            CachedAsset::Static(frame) => Some(Arc::clone(frame)),
            CachedAsset::AnimatedGif(frames) if !frames.is_empty() => {
                Some(Arc::clone(&frames[(frame_index as usize) % frames.len()]))
            }
            CachedAsset::VideoStream(handle) => {
                let (lock, cvar) = &*handle.shared;
                let mut st = lock.lock().ok()?;

                // frame_index == 0 はサムネイル描画で毎フレーム呼ばれるため、target_frame を動かさず first_frame を即返す
                if frame_index == 0 {
                    if let Some(first) = &st.first_frame {
                        return Some(Arc::clone(first));
                    }
                } else {
                    st.target_frame = frame_index;
                    cvar.notify_all();
                }

                // 1. 要求フレームがすでにバックグラウンドでデコード済みなら即座に返す (0.00ms)
                if let Some(f) = st.frames.get(&frame_index) {
                    return Some(Arc::clone(f));
                }

                // 2. まだデコードが追いついていない場合、最も近いデコード済みフレームをノンブロッキングで返す
                if let Some((_, nearest)) = st
                    .frames
                    .iter()
                    .min_by_key(|(&k, _)| (k as i64 - frame_index as i64).abs())
                {
                    return Some(Arc::clone(nearest));
                }

                st.first_frame.clone()
            }
            _ => None,
        }
    }

    /// 互換性のための参照取得 API。
    pub fn get_frame(&mut self, path: &Path, frame_index: u32, time_sec: f64) -> Option<Arc<RgbaFrame>> {
        self.get_frame_arc(path, frame_index, time_sec)
    }

    fn init_video_stream(&mut self, path: &Path) -> CachedAsset {
        let Some(ffmpeg) = self.ffmpeg_bin.clone() else {
            return CachedAsset::Failed;
        };
        let meta = self.get_metadata(path);
        let orig_w = meta.width.max(2);
        let orig_h = meta.height.max(2);

        // デコード解像度: プレビューの滑らかさとパフォーマンスを考慮し、最大 1920px にクランプ（4K等も軽快再生）
        let (decode_w, decode_h) = if orig_w > 1920 {
            let sc_h = (orig_h as f64 * 1920.0 / orig_w as f64).round() as u32;
            (1920, ((sc_h.max(2)) / 2) * 2)
        } else {
            ((orig_w / 2) * 2, (orig_h / 2) * 2)
        };

        let shared = Arc::new((
            Mutex::new(VideoStreamShared {
                first_frame: None,
                frames: HashMap::with_capacity(400),
                target_frame: 0,
                stop: false,
            }),
            Condvar::new(),
        ));

        let shared_clone = Arc::clone(&shared);
        let video_path = path.to_path_buf();

        thread::Builder::new()
            .name(format!(
                "openavi-vdec-{}",
                path.file_name().map(|s| s.to_string_lossy()).unwrap_or_default()
            ))
            .spawn(move || {
                video_stream_worker(
                    ffmpeg,
                    video_path,
                    orig_w,
                    orig_h,
                    decode_w,
                    decode_h,
                    shared_clone,
                );
            })
            .ok();

        // 初回のみ先頭フレーム (frame 0) が届くまで最大 250ms 待機してサムネイルを確実に用意する
        let (lock, cvar) = &*shared;
        if let Ok(mut st) = lock.lock() {
            let deadline = Instant::now() + Duration::from_millis(250);
            while st.first_frame.is_none() && !st.stop {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                let (new_st, _) = cvar.wait_timeout(st, deadline - now).unwrap();
                st = new_st;
            }
        }

        CachedAsset::VideoStream(VideoStreamHandle { shared })
    }
}

/// バックグラウンドで `ffmpeg` の連続パイプを開き、フレームを高速に先読みし続けるワーカースレッド。
fn video_stream_worker(
    ffmpeg: PathBuf,
    video_path: PathBuf,
    orig_w: u32,
    orig_h: u32,
    decode_w: u32,
    decode_h: u32,
    shared: Arc<(Mutex<VideoStreamShared>, Condvar)>,
) {
    let frame_bytes = (decode_w * decode_h * 4) as usize;
    let (lock, cvar) = &*shared;

    let mut child_opt: Option<Child> = None;
    let mut cur_decode_frame: u32 = 0;
    let mut eof_reached = false;

    let kill_child = |c_opt: &mut Option<Child>| {
        if let Some(mut c) = c_opt.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    };

    loop {
        // 1. 現在の目標フレームとキャッシュ状態を確認
        let (stop, target, has_target) = {
            let Ok(st) = lock.lock() else { break };
            (st.stop, st.target_frame, st.frames.contains_key(&st.target_frame))
        };
        if stop {
            kill_child(&mut child_opt);
            break;
        }

        // 2. シーク判定:
        // - パイプがまだ開かれていない
        // - または target が現在のデコード位置より過去でキャッシュに無い
        // - または target が現在のデコード位置より 60 フレーム (2秒) 以上先へ飛んでいる
        let need_reopen = if child_opt.is_none() {
            !eof_reached || !has_target
        } else if !has_target && (target < cur_decode_frame || target > cur_decode_frame + 60) {
            true
        } else {
            false
        };

        if need_reopen {
            kill_child(&mut child_opt);
            eof_reached = false;
            // 初回は必ず 0 から開始してサムネイルを取得
            let start_frame = {
                let Ok(st) = lock.lock() else { break };
                if st.first_frame.is_none() {
                    0
                } else {
                    target
                }
            };
            cur_decode_frame = start_frame;
            let start_sec = start_frame as f64 / 30.0;

            let mut cmd = Command::new(&ffmpeg);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            }
            if start_frame > 0 {
                cmd.args(["-ss", &format!("{start_sec:.3}")]);
            }
            cmd.arg("-i")
                .arg(&video_path)
                .args([
                    "-vf",
                    &format!("fps=30,scale={decode_w}:{decode_h}:flags=fast_bilinear"),
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "rgba",
                    "-loglevel",
                    "error",
                    "-",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null());

            match cmd.spawn() {
                Ok(c) => child_opt = Some(c),
                Err(_) => {
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
            }
        }

        // 3. もし現在のデコード位置が target より 240 フレーム (8秒) 以上先まで進んでいる場合は一時待機（メモリとCPUの節約）
        if child_opt.is_some() && cur_decode_frame > target + 240 {
            if let Ok(st) = lock.lock() {
                let _ = cvar.wait_timeout(st, Duration::from_millis(15));
            }
            continue;
        }

        // 4. パイプから 1 フレーム分を読み出す
        if let Some(child) = child_opt.as_mut() {
            if let Some(stdout) = child.stdout.as_mut() {
                let mut buf = vec![0u8; frame_bytes];
                match read_exact_or_eof(stdout, &mut buf) {
                    Ok(true) => {
                        let frame_arc = Arc::new(RgbaFrame {
                            width: decode_w,
                            height: decode_h,
                            original_width: orig_w,
                            original_height: orig_h,
                            pixels: buf,
                        });
                        if let Ok(mut st) = lock.lock() {
                            if cur_decode_frame == 0 && st.first_frame.is_none() {
                                st.first_frame = Some(Arc::clone(&frame_arc));
                            }
                            // スライディングウィンドウ: 最大 360 フレーム (12秒分) を保持し、古いフレームを破棄
                            if st.frames.len() >= 360 {
                                let tgt = st.target_frame;
                                st.frames.retain(|&k, _| {
                                    k + 30 >= tgt && k <= tgt + 300
                                });
                            }
                            st.frames.insert(cur_decode_frame, frame_arc);
                            cvar.notify_all();
                        }
                        cur_decode_frame += 1;
                    }
                    _ => {
                        // EOF 到達
                        eof_reached = true;
                        kill_child(&mut child_opt);
                        if let Ok(st) = lock.lock() {
                            let _ = cvar.wait_timeout(st, Duration::from_millis(30));
                        }
                    }
                }
            } else {
                kill_child(&mut child_opt);
            }
        } else {
            if let Ok(st) = lock.lock() {
                let _ = cvar.wait_timeout(st, Duration::from_millis(20));
            }
        }
    }
}

fn read_exact_or_eof<R: Read>(reader: &mut R, mut buf: &mut [u8]) -> std::io::Result<bool> {
    while !buf.is_empty() {
        match reader.read(buf) {
            Ok(0) => return Ok(false),
            Ok(n) => buf = &mut buf[n..],
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

fn load_static_image(path: &Path) -> CachedAsset {
    match image::open(path) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let w = rgba.width();
            let h = rgba.height();
            CachedAsset::Static(Arc::new(RgbaFrame {
                width: w,
                height: h,
                original_width: w,
                original_height: h,
                pixels: rgba.into_raw(),
            }))
        }
        Err(e) => {
            log::warn!("画像の読み込み失敗 ({}): {e}", path.display());
            CachedAsset::Failed
        }
    }
}

fn load_gif_or_static(path: &Path) -> CachedAsset {
    if let Ok(file) = std::fs::File::open(path) {
        let reader = std::io::BufReader::new(file);
        if let Ok(decoder) = image::codecs::gif::GifDecoder::new(reader) {
            if let Ok(frames) = decoder.into_frames().collect_frames() {
                let list: Vec<Arc<RgbaFrame>> = frames
                    .into_iter()
                    .map(|f| {
                        let buf = f.into_buffer();
                        let w = buf.width();
                        let h = buf.height();
                        Arc::new(RgbaFrame {
                            width: w,
                            height: h,
                            original_width: w,
                            original_height: h,
                            pixels: buf.into_raw(),
                        })
                    })
                    .collect();
                if list.len() == 1 {
                    return CachedAsset::Static(list.into_iter().next().unwrap());
                } else if !list.is_empty() {
                    return CachedAsset::AnimatedGif(list);
                }
            }
        }
    }
    load_static_image(path)
}

/// アプリ内ダウンロードした FFmpeg の配置先 (`%LOCALAPPDATA%\OpenAvi\bin`)。
/// インストール先が書き込み不可でも導入できるようユーザー領域を使います。
pub fn user_ffmpeg_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("OpenAvi").join("bin"))
}

/// FFmpeg (Windows x64 essentials ビルド) の配布 URL。
pub const FFMPEG_DOWNLOAD_URL: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";

/// FFmpeg / FFprobe をダウンロードして [`user_ffmpeg_dir`] に展開します（ブロッキング。別スレッドから呼び出すこと）。
/// 成功時は配置先ディレクトリを返します。
pub fn download_ffmpeg_blocking() -> Result<PathBuf, String> {
    let dest = user_ffmpeg_dir().ok_or_else(|| "LOCALAPPDATA が取得できません".to_string())?;
    std::fs::create_dir_all(&dest).map_err(|e| format!("配置先を作成できません: {e}"))?;

    let work = std::env::temp_dir().join("openavi-ffmpeg-download");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("作業フォルダを作成できません: {e}"))?;
    let zip = work.join("ffmpeg.zip");
    let extract = work.join("extract");

    let q = |p: &Path| p.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'; \
         [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12; \
         Invoke-WebRequest -Uri '{url}' -OutFile '{zip}' -UseBasicParsing; \
         Expand-Archive -Path '{zip}' -DestinationPath '{extract}' -Force; \
         $f=Get-ChildItem -Path '{extract}' -Recurse -Filter ffmpeg.exe | Select-Object -First 1; \
         $p=Get-ChildItem -Path '{extract}' -Recurse -Filter ffprobe.exe | Select-Object -First 1; \
         if(-not $f){{ throw 'ffmpeg.exe not found in archive' }}; \
         Copy-Item $f.FullName '{dest}\\ffmpeg.exe' -Force; \
         if($p){{ Copy-Item $p.FullName '{dest}\\ffprobe.exe' -Force }}",
        url = FFMPEG_DOWNLOAD_URL,
        zip = q(&zip),
        extract = q(&extract),
        dest = q(&dest),
    );

    let mut cmd = Command::new("powershell");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let output = cmd
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .output()
        .map_err(|e| format!("PowerShell を起動できません: {e}"))?;
    let _ = std::fs::remove_dir_all(&work);

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ダウンロードに失敗しました: {}", err.trim()));
    }
    if !dest.join("ffmpeg.exe").is_file() {
        return Err("ffmpeg.exe を配置できませんでした".to_string());
    }
    Ok(dest)
}

fn locate_ffmpeg_tools() -> (Option<PathBuf>, Option<PathBuf>) {
    // 探索するディレクトリ候補リスト
    let mut search_dirs = Vec::new();

    // 1. 実行ファイル (openavi.exe) の親ディレクトリおよび bin サブディレクトリ (ポータブル同梱を最優先)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            search_dirs.push(exe_dir.to_path_buf());
            search_dirs.push(exe_dir.join("bin"));
            search_dirs.push(exe_dir.join("tools"));
        }
    }

    // 1.5 アプリ内ダウンロードで導入された FFmpeg (%LOCALAPPDATA%\OpenAvi\bin)
    if let Some(dir) = user_ffmpeg_dir() {
        search_dirs.push(dir);
    }

    // 2. カレントワーキングディレクトリおよび bin サブディレクトリ
    if let Ok(cur_dir) = std::env::current_dir() {
        search_dirs.push(cur_dir.clone());
        search_dirs.push(cur_dir.join("bin"));
        search_dirs.push(cur_dir.join("tools"));
    }

    // 3. インストール先ディレクトリ (%LOCALAPPDATA%\Programs\OpenAvi)
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let install_dir = PathBuf::from(&local_app_data).join("Programs").join("OpenAvi");
        search_dirs.push(install_dir.clone());
        search_dirs.push(install_dir.join("bin"));
    }

    // 4. Scoop / Chocolatey
    if let Ok(user_profile) = std::env::var("USERPROFILE") {
        search_dirs.push(PathBuf::from(user_profile).join("scoop").join("shims"));
    }
    search_dirs.push(PathBuf::from(r"C:\ProgramData\chocolatey\bin"));

    // ディレクトリ候補から ffmpeg.exe / ffprobe.exe を探索
    for dir in &search_dirs {
        let ffmpeg_candidate = dir.join("ffmpeg.exe");
        if ffmpeg_candidate.is_file() {
            let ffprobe_candidate = dir.join("ffprobe.exe");
            let probe = if ffprobe_candidate.is_file() {
                Some(ffprobe_candidate)
            } else {
                None
            };
            return (Some(ffmpeg_candidate), probe);
        }
    }

    // 5. システム PATH から検索
    if Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
    {
        return (Some(PathBuf::from("ffmpeg")), Some(PathBuf::from("ffprobe")));
    }

    // 6. Windows WinGet パッケージ領域を検索
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let pkg_dir = PathBuf::from(local_app_data).join("Microsoft\\WinGet\\Packages");
        if let Ok(entries) = std::fs::read_dir(pkg_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if name.contains("ffmpeg") {
                    if let Some(bin_dir) = find_file_recursive(&entry.path(), "ffmpeg.exe", 4) {
                        let probe = bin_dir.parent().unwrap().join("ffprobe.exe");
                        return (Some(bin_dir), probe.exists().then_some(probe));
                    }
                }
            }
        }
    }

    (None, None)
}

fn find_file_recursive(dir: &Path, target_name: &str, depth: u8) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().is_some_and(|n| n.eq_ignore_ascii_case(target_name)) {
            return Some(path);
        } else if path.is_dir() {
            if let Some(found) = find_file_recursive(&path, target_name, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

fn probe_media_metadata(ffprobe: &Path, path: &Path) -> Option<MediaMetadata> {
    let mut cmd = Command::new(ffprobe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let out = cmd
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_type,width,height,r_frame_rate:stream_tags=rotate:stream_side_data=rotation",
            "-of",
            "default=noprint_wrappers=1:nokey=0",
        ])
        .arg(path)
        .output()
        .ok()?;

    let text = String::from_utf8_lossy(&out.stdout);
    let mut width = 640;
    let mut height = 360;
    let mut duration_sec = 0.0;
    let mut fps = 30.0;
    let mut has_audio = false;
    let mut has_video = false;
    // 複数の映像ストリーム（サムネイル等）があっても最初の映像ストリームの値だけを採用する
    let mut got_dims = false;
    let mut got_fps = false;
    let mut rotation: i32 = 0;

    for line in text.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "width" => {
                    if let (false, Ok(w)) = (got_dims, v.trim().parse::<u32>()) {
                        width = w;
                    }
                }
                "height" => {
                    if let (false, Ok(h)) = (got_dims, v.trim().parse::<u32>()) {
                        height = h;
                        got_dims = true;
                    }
                }
                // スマートフォン等で撮影された縦動画の回転メタデータ（ffmpeg は自動回転するため寸法を入れ替える）
                "rotation" | "TAG:rotate" => {
                    if let Ok(r) = v.trim().parse::<f64>() {
                        rotation = (r.round() as i32).rem_euclid(360);
                    }
                }
                "duration" => {
                    if let Ok(d) = v.trim().parse() {
                        duration_sec = d;
                    }
                }
                "codec_type" => {
                    if v.trim() == "video" {
                        has_video = true;
                    } else if v.trim() == "audio" {
                        has_audio = true;
                    }
                }
                "r_frame_rate" => {
                    if let (false, Some((num, den))) = (got_fps, v.trim().split_once('/')) {
                        if let (Ok(n), Ok(d)) = (num.parse::<f64>(), den.parse::<f64>()) {
                            if d > 0.0 && n > 0.0 {
                                fps = (n / d).round();
                                got_fps = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if rotation == 90 || rotation == 270 {
        std::mem::swap(&mut width, &mut height);
    }

    Some(MediaMetadata {
        width: width.max(2),
        height: height.max(2),
        duration_sec,
        fps: if fps > 0.0 { fps } else { 30.0 },
        has_audio,
        has_video,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_kind_detection() {
        assert_eq!(MediaKind::from_path(Path::new("a.png")), Some(MediaKind::Image));
        assert_eq!(MediaKind::from_path(Path::new("b.MP4")), Some(MediaKind::Video));
        assert_eq!(MediaKind::from_path(Path::new("c.wav")), Some(MediaKind::Audio));
    }

    #[test]
    fn decode_sample_image_and_video() {
        let mut cache = MediaCache::new();
        let img_path = Path::new("../../samples/sample_image.png");
        let img_frame = cache.get_frame(img_path, 0, 0.0).expect("decode sample_image.png");
        assert_eq!((img_frame.width, img_frame.height), (240, 160));
        assert_eq!(img_frame.pixels.len(), 240 * 160 * 4);

        if cache.has_ffmpeg() {
            let vid_path = Path::new("../../samples/sample_video.mp4");
            let mut vid_frame = None;
            for _ in 0..100 {
                if let Some(f) = cache.get_frame(vid_path, 0, 0.0) {
                    vid_frame = Some(f);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let vid_frame = vid_frame.expect("decode sample_video.mp4");
            assert_eq!((vid_frame.width, vid_frame.height), (320, 180));
            assert_eq!(vid_frame.pixels.len(), 320 * 180 * 4);

            let audio_vid = Path::new("../../samples/sample_with_audio_10s.mp4");
            if audio_vid.exists() {
                let meta = cache.get_metadata(audio_vid);
                assert!(meta.duration_sec >= 9.9);
                assert!(meta.has_audio);
                let pcm = cache.get_audio_pcm(audio_vid).expect("extract pcm audio");
                assert!(!pcm.is_empty());
            }
        }
    }
}
