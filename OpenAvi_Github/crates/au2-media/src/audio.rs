//! # audio
//!
//! Windows 標準の `winmm.dll` (waveOut) を用いたゼロ依存・低遅延オーディオ再生エンジン。
//! 動画および音声ファイルから抽出された 44.1kHz 16-bit ステレオ PCM を
//! サンプル単位で欠落・重複なく連続ストリーミング再生します。

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const SAMPLE_RATE: u32 = 44100;
pub const CHANNELS: u16 = 2;
pub const BITS_PER_SAMPLE: u16 = 16;

/// 動画または音声ファイルから抽出した 44.1kHz 16-bit ステレオ PCM 音声データ。
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    pub sample_rate: u32,
    pub channels: u16,
    /// インターリーブされた 16-bit PCM サンプル (L, R, L, R...)
    pub samples: Arc<Vec<i16>>,
    pub duration_sec: f64,
}

impl DecodedAudio {
    pub fn empty() -> Self {
        Self {
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            samples: Arc::new(Vec::new()),
            duration_sec: 0.0,
        }
    }

    pub fn total_samples_per_channel(&self) -> usize {
        self.samples.len() / (self.channels as usize).max(1)
    }
}

/// FFmpeg を使用してメディアファイルの音声ストリームを 44.1kHz 16-bit ステレオ PCM に一括デコードします。
pub fn decode_audio_pcm(ffmpeg: &Path, media_path: &Path) -> Option<DecodedAudio> {
    let mut cmd = Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let output = cmd
        .args(["-i"])
        .arg(media_path)
        .args([
            "-vn",
            "-ac",
            "2",
            "-ar",
            "44100",
            "-f",
            "s16le",
            "-loglevel",
            "error",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }

    let bytes = output.stdout;
    let sample_count = bytes.len() / 2;
    let mut samples = Vec::with_capacity(sample_count);
    for chunk in bytes.chunks_exact(2) {
        samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }

    let duration_sec = samples.len() as f64 / (SAMPLE_RATE as f64 * CHANNELS as f64);
    Some(DecodedAudio {
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS,
        samples: Arc::new(samples),
        duration_sec,
    })
}

#[derive(Clone)]
pub struct AudioTrackItem {
    pub audio: Arc<Vec<i16>>,
    pub start_frame: u32,
    pub end_frame: u32,
    pub offset_sec: f64,
    pub volume: f32,
}

struct EngineSharedState {
    playing: bool,
    fps: f64,
    tracks: Vec<AudioTrackItem>,
    seek_generation: u64,
    seek_start_sec: f64,
    seek_instant: Option<Instant>,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_snake_case)]
struct WAVEFORMATEX {
    wFormatTag: u16,
    nChannels: u16,
    nSamplesPerSec: u32,
    nAvgBytesPerSec: u32,
    nBlockAlign: u16,
    wBitsPerSample: u16,
    cbSize: u16,
}

#[cfg(windows)]
#[repr(C)]
#[allow(non_snake_case)]
struct WAVEHDR {
    lpData: *mut u8,
    dwBufferLength: u32,
    dwBytesRecorded: u32,
    dwUser: usize,
    dwFlags: u32,
    dwLoops: u32,
    lpNext: usize,
    reserved: usize,
}

#[cfg(windows)]
const WHDR_INQUEUE: u32 = 0x00000010;

#[cfg(windows)]
#[link(name = "winmm")]
extern "system" {
    fn waveOutOpen(
        phwo: *mut usize,
        uDeviceID: u32,
        pwfx: *const WAVEFORMATEX,
        dwCallback: usize,
        dwInstance: usize,
        fdwOpen: u32,
    ) -> u32;
    fn waveOutPrepareHeader(hwo: usize, pwh: *mut WAVEHDR, cbwh: u32) -> u32;
    fn waveOutUnprepareHeader(hwo: usize, pwh: *mut WAVEHDR, cbwh: u32) -> u32;
    fn waveOutWrite(hwo: usize, pwh: *mut WAVEHDR, cbwh: u32) -> u32;
    fn waveOutReset(hwo: usize) -> u32;
    fn waveOutClose(hwo: usize) -> u32;
}

/// Windows ネイティブのリアルタイム・オーディオ出力エンジン。
pub struct AudioEngine {
    shared: Arc<Mutex<EngineSharedState>>,
    running: Arc<AtomicBool>,
    _worker: Option<JoinHandle<()>>,
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEngine {
    pub fn new() -> Self {
        let shared = Arc::new(Mutex::new(EngineSharedState {
            playing: false,
            fps: 30.0,
            tracks: Vec::new(),
            seek_generation: 0,
            seek_start_sec: 0.0,
            seek_instant: None,
        }));
        let running = Arc::new(AtomicBool::new(true));

        let shared_clone = Arc::clone(&shared);
        let running_clone = Arc::clone(&running);

        let worker = thread::Builder::new()
            .name("openavi-audio-worker".into())
            .spawn(move || {
                Self::worker_loop(shared_clone, running_clone);
            })
            .ok();

        Self {
            shared,
            running,
            _worker: worker,
        }
    }

    /// 再生状態および現在のタイムラインフレームを同期します。
    pub fn sync_playback(&self, playing: bool, current_frame: u32, fps: f64) {
        if let Ok(mut st) = self.shared.lock() {
            let fps_safe = fps.max(1.0);
            let cur_sec = current_frame as f64 / fps_safe;

            if playing && !st.playing {
                st.seek_generation = st.seek_generation.wrapping_add(1);
                st.seek_start_sec = cur_sec;
                st.seek_instant = Some(Instant::now());
            } else if !playing && st.playing {
                st.seek_instant = None;
            } else if playing {
                // 再生中に大きくシークまたはループバックした場合のみ再シーク
                if let Some(inst) = st.seek_instant {
                    let expected_sec = st.seek_start_sec + inst.elapsed().as_secs_f64();
                    if (cur_sec - expected_sec).abs() > 0.25 {
                        st.seek_generation = st.seek_generation.wrapping_add(1);
                        st.seek_start_sec = cur_sec;
                        st.seek_instant = Some(Instant::now());
                    }
                } else {
                    st.seek_generation = st.seek_generation.wrapping_add(1);
                    st.seek_start_sec = cur_sec;
                    st.seek_instant = Some(Instant::now());
                }
            }
            st.playing = playing;
            st.fps = fps_safe;
        }
    }

    /// 現在のシーンにある再生対象音声トラック一覧を設定します。
    pub fn set_active_tracks(&self, tracks: Vec<AudioTrackItem>) {
        if let Ok(mut st) = self.shared.lock() {
            st.tracks = tracks;
        }
    }

    /// 停止時にオーディオを即座にリセットします。
    pub fn stop(&self) {
        if let Ok(mut st) = self.shared.lock() {
            if st.playing {
                st.playing = false;
                st.seek_instant = None;
            }
        }
    }

    #[cfg(windows)]
    fn worker_loop(shared: Arc<Mutex<EngineSharedState>>, running: Arc<AtomicBool>) {
        let block_align = (CHANNELS * BITS_PER_SAMPLE / 8) as u16;
        let avg_bytes_per_sec = SAMPLE_RATE * block_align as u32;
        let wfx = WAVEFORMATEX {
            wFormatTag: 1, // WAVE_FORMAT_PCM
            nChannels: CHANNELS,
            nSamplesPerSec: SAMPLE_RATE,
            nAvgBytesPerSec: avg_bytes_per_sec,
            nBlockAlign: block_align,
            wBitsPerSample: BITS_PER_SAMPLE,
            cbSize: 0,
        };

        let mut hwave: usize = 0;
        let res = unsafe { waveOutOpen(&mut hwave, 0xFFFFFFFF, &wfx, 0, 0, 0) };
        if res != 0 || hwave == 0 {
            log::warn!("waveOutOpen failed: {res}");
            return;
        }

        // 1チャンク = 1024 サンプルフレーム (約 23.2ms) × 4 バッファ = 約 93ms の連続リングキュー
        const CHUNK_FRAMES: usize = 1024;
        const BUFFER_COUNT: usize = 4;
        let mut raw_buffers: Vec<Vec<u8>> = vec![vec![0u8; CHUNK_FRAMES * (CHANNELS as usize) * 2]; BUFFER_COUNT];
        let mut hdrs: Vec<WAVEHDR> = Vec::with_capacity(BUFFER_COUNT);

        for buf in &mut raw_buffers {
            let mut hdr = WAVEHDR {
                lpData: buf.as_mut_ptr(),
                dwBufferLength: buf.len() as u32,
                dwBytesRecorded: 0,
                dwUser: 0,
                dwFlags: 0,
                dwLoops: 0,
                lpNext: 0,
                reserved: 0,
            };
            unsafe {
                let _ = waveOutPrepareHeader(hwave, &mut hdr, std::mem::size_of::<WAVEHDR>() as u32);
            }
            hdrs.push(hdr);
        }

        let mut buf_idx = 0;
        let mut last_was_playing = false;
        let mut local_generation: u64 = 0;
        // 連続書き込み位置（タイムライン先頭からのサンプルフレーム数, 1秒 = 44100）
        let mut cursor_sample_frame: usize = 0;

        while running.load(Ordering::Relaxed) {
            let (is_playing, gen, seek_sec, fps, tracks) = {
                if let Ok(st) = shared.lock() {
                    (
                        st.playing,
                        st.seek_generation,
                        st.seek_start_sec,
                        st.fps,
                        st.tracks.clone(),
                    )
                } else {
                    (false, 0, 0.0, 30.0, Vec::new())
                }
            };

            if !is_playing {
                if last_was_playing {
                    unsafe {
                        let _ = waveOutReset(hwave);
                    }
                    last_was_playing = false;
                }
                thread::sleep(Duration::from_millis(10));
                continue;
            }

            if !last_was_playing || gen != local_generation {
                unsafe {
                    let _ = waveOutReset(hwave);
                }
                local_generation = gen;
                last_was_playing = true;
                cursor_sample_frame = (seek_sec.max(0.0) * SAMPLE_RATE as f64).round() as usize;
                buf_idx = 0;
            }

            // 現在対象としているバッファがまだ再生キューにある場合は少し待つ
            if (hdrs[buf_idx].dwFlags & WHDR_INQUEUE) != 0 {
                thread::sleep(Duration::from_millis(4));
                continue;
            }

            // `cursor_sample_frame` から正確に `CHUNK_FRAMES` 分をミックスして書き込む
            let mut mix_buf = [0.0f32; CHUNK_FRAMES * (CHANNELS as usize)];
            let chunk_start_sec = cursor_sample_frame as f64 / SAMPLE_RATE as f64;
            let chunk_end_sec = (cursor_sample_frame + CHUNK_FRAMES) as f64 / SAMPLE_RATE as f64;

            for trk in &tracks {
                let track_start_sec = trk.start_frame as f64 / fps;
                let track_end_sec = (trk.end_frame + 1) as f64 / fps;
                if chunk_end_sec <= track_start_sec || chunk_start_sec >= track_end_sec {
                    continue;
                }

                let track_start_sample_frame = (track_start_sec * SAMPLE_RATE as f64).round() as isize;
                let offset_sample_frame = (trk.offset_sec * SAMPLE_RATE as f64).round() as isize;
                let vol = trk.volume.clamp(0.0, 2.0);
                let total_interleaved = trk.audio.len();

                for f in 0..CHUNK_FRAMES {
                    let timeline_sf = (cursor_sample_frame + f) as isize;
                    let rel_sf = timeline_sf - track_start_sample_frame + offset_sample_frame;
                    if rel_sf < 0 {
                        continue;
                    }
                    let base = (rel_sf as usize) * (CHANNELS as usize);
                    if base + 1 < total_interleaved {
                        mix_buf[f * 2] += trk.audio[base] as f32 * vol;
                        mix_buf[f * 2 + 1] += trk.audio[base + 1] as f32 * vol;
                    }
                }
            }

            let target_buf = &mut raw_buffers[buf_idx];
            for (i, &sample_f32) in mix_buf.iter().enumerate() {
                let clamped = sample_f32.clamp(-32768.0, 32767.0) as i16;
                let b = clamped.to_le_bytes();
                target_buf[i * 2] = b[0];
                target_buf[i * 2 + 1] = b[1];
            }

            let hdr = &mut hdrs[buf_idx];
            unsafe {
                let _ = waveOutWrite(hwave, hdr, std::mem::size_of::<WAVEHDR>() as u32);
            }

            cursor_sample_frame += CHUNK_FRAMES;
            buf_idx = (buf_idx + 1) % BUFFER_COUNT;
        }

        unsafe {
            let _ = waveOutReset(hwave);
            for mut hdr in hdrs {
                let _ = waveOutUnprepareHeader(hwave, &mut hdr, std::mem::size_of::<WAVEHDR>() as u32);
            }
            let _ = waveOutClose(hwave);
        }
    }

    #[cfg(not(windows))]
    fn worker_loop(_shared: Arc<Mutex<EngineSharedState>>, running: Arc<AtomicBool>) {
        while running.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
