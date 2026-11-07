//! # audio
//!
//! クロスプラットフォーム対応のゼロ依存・低遅延オーディオ再生エンジン。
//! - **Windows**: 標準の `winmm.dll` (`waveOut`)
//! - **macOS**: 標準の `AudioToolbox.framework` (`AudioQueue`)
//! - **Linux / Unix**: `PipeWire` (`pw-cat`) / `PulseAudio` (`pacat`) / `ALSA` (`aplay`) / `ffplay` パイプ出力
//!
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

/// WAV (RIFF WAVE) ファイルを標準ライブラリのみで直接パースし、
/// 44.1kHz 16-bit ステレオ PCM に高速デコードします。
/// FFmpeg 不要で 8/16/24/32bit PCM および 32bit IEEE float、モノラル/ステレオ/任意サンプリングレートに対応。
pub fn decode_wav_pcm(bytes: &[u8]) -> Option<DecodedAudio> {
    if bytes.len() < 44 {
        return None;
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }

    let mut pos = 12;
    let mut fmt_opt: Option<(u16, u16, u32, u16)> = None; // (format, channels, sample_rate, bits_per_sample)
    let mut data_slice: Option<&[u8]> = None;

    while pos + 8 <= bytes.len() {
        let chunk_id = &bytes[pos..pos + 4];
        let chunk_size = u32::from_le_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]) as usize;
        let chunk_data_start = pos + 8;
        let chunk_data_end = (chunk_data_start + chunk_size).min(bytes.len());

        if chunk_id == b"fmt " && chunk_size >= 16 {
            let data = &bytes[chunk_data_start..chunk_data_end];
            let format_tag = u16::from_le_bytes([data[0], data[1]]);
            let channels = u16::from_le_bytes([data[2], data[3]]);
            let sample_rate = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
            let bits_per_sample = u16::from_le_bytes([data[14], data[15]]);
            fmt_opt = Some((format_tag, channels, sample_rate, bits_per_sample));
        } else if chunk_id == b"data" {
            data_slice = Some(&bytes[chunk_data_start..chunk_data_end]);
            if fmt_opt.is_some() {
                break;
            }
        }

        // 次のチャンクへ（奇数バイトサイズの場合は 1 バイトパディング）
        pos = chunk_data_start + chunk_size + (chunk_size % 2);
    }

    let (format_tag, in_channels, in_rate, in_bits) = fmt_opt?;
    let raw_data = data_slice?;

    if in_channels == 0 || in_rate == 0 {
        return None;
    }

    let bytes_per_sample = (in_bits as usize) / 8;
    if bytes_per_sample == 0 {
        return None;
    }
    let frame_size = bytes_per_sample * in_channels as usize;
    if frame_size == 0 {
        return None;
    }
    let total_frames = raw_data.len() / frame_size;
    if total_frames == 0 {
        return None;
    }

    // 44.1kHz 16-bit PCM の標準 WAV は直接ビットパーフェクトに変換（ゼロ誤差＆超高速）
    if in_rate == SAMPLE_RATE && format_tag == 1 && in_bits == 16 {
        let mut out = Vec::with_capacity(total_frames * 2);
        for frame_idx in 0..total_frames {
            let offset = frame_idx * frame_size;
            let ch0 = i16::from_le_bytes([raw_data[offset], raw_data[offset + 1]]);
            let ch1 = if in_channels > 1 {
                i16::from_le_bytes([raw_data[offset + 2], raw_data[offset + 3]])
            } else {
                ch0
            };
            out.push(ch0);
            out.push(ch1);
        }
        let duration_sec = total_frames as f64 / SAMPLE_RATE as f64;
        return Some(DecodedAudio {
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            samples: Arc::new(out),
            duration_sec,
        });
    }

    let mut in_samples_stereo: Vec<[f32; 2]> = Vec::with_capacity(total_frames);

    match (format_tag, in_bits) {
        // PCM 16bit
        (1, 16) => {
            for frame_idx in 0..total_frames {
                let offset = frame_idx * frame_size;
                let ch0 = i16::from_le_bytes([raw_data[offset], raw_data[offset + 1]]) as f32 / 32768.0;
                let ch1 = if in_channels > 1 {
                    i16::from_le_bytes([raw_data[offset + 2], raw_data[offset + 3]]) as f32 / 32768.0
                } else {
                    ch0
                };
                in_samples_stereo.push([ch0, ch1]);
            }
        }
        // PCM 24bit
        (1, 24) => {
            for frame_idx in 0..total_frames {
                let offset = frame_idx * frame_size;
                let b0 = [0, raw_data[offset], raw_data[offset + 1], raw_data[offset + 2]];
                let ch0 = (i32::from_le_bytes(b0) >> 8) as f32 / 8388608.0;
                let ch1 = if in_channels > 1 {
                    let b1 = [0, raw_data[offset + 3], raw_data[offset + 4], raw_data[offset + 5]];
                    (i32::from_le_bytes(b1) >> 8) as f32 / 8388608.0
                } else {
                    ch0
                };
                in_samples_stereo.push([ch0, ch1]);
            }
        }
        // PCM 8bit (unsigned)
        (1, 8) => {
            for frame_idx in 0..total_frames {
                let offset = frame_idx * frame_size;
                let ch0 = (raw_data[offset] as f32 - 128.0) / 128.0;
                let ch1 = if in_channels > 1 {
                    (raw_data[offset + 1] as f32 - 128.0) / 128.0
                } else {
                    ch0
                };
                in_samples_stereo.push([ch0, ch1]);
            }
        }
        // PCM 32bit int
        (1, 32) => {
            for frame_idx in 0..total_frames {
                let offset = frame_idx * frame_size;
                let ch0 = i32::from_le_bytes(raw_data[offset..offset + 4].try_into().unwrap()) as f32 / 2147483648.0;
                let ch1 = if in_channels > 1 {
                    i32::from_le_bytes(raw_data[offset + 4..offset + 8].try_into().unwrap()) as f32 / 2147483648.0
                } else {
                    ch0
                };
                in_samples_stereo.push([ch0, ch1]);
            }
        }
        // IEEE Float 32bit (format 3)
        (3, 32) => {
            for frame_idx in 0..total_frames {
                let offset = frame_idx * frame_size;
                let ch0 = f32::from_le_bytes(raw_data[offset..offset + 4].try_into().unwrap());
                let ch1 = if in_channels > 1 {
                    f32::from_le_bytes(raw_data[offset + 4..offset + 8].try_into().unwrap())
                } else {
                    ch0
                };
                in_samples_stereo.push([ch0, ch1]);
            }
        }
        _ => return None,
    }

    // リサンプリング (in_rate -> 44100)
    let out_frames = if in_rate == SAMPLE_RATE {
        let mut out = Vec::with_capacity(in_samples_stereo.len() * 2);
        for [l, r] in in_samples_stereo {
            out.push((l.clamp(-1.0, 1.0) * 32767.0).round() as i16);
            out.push((r.clamp(-1.0, 1.0) * 32767.0).round() as i16);
        }
        out
    } else {
        let ratio = in_rate as f64 / SAMPLE_RATE as f64;
        let out_count = ((in_samples_stereo.len() as f64) / ratio).round() as usize;
        let mut out = Vec::with_capacity(out_count * 2);
        for i in 0..out_count {
            let src_pos = i as f64 * ratio;
            let idx0 = src_pos.floor() as usize;
            let frac = (src_pos - idx0 as f64) as f32;
            let s0 = in_samples_stereo.get(idx0).copied().unwrap_or([0.0, 0.0]);
            let s1 = in_samples_stereo.get(idx0 + 1).copied().unwrap_or(s0);
            let l = s0[0] + (s1[0] - s0[0]) * frac;
            let r = s0[1] + (s1[1] - s0[1]) * frac;
            out.push((l.clamp(-1.0, 1.0) * 32767.0).round() as i16);
            out.push((r.clamp(-1.0, 1.0) * 32767.0).round() as i16);
        }
        out
    };

    let duration_sec = out_frames.len() as f64 / (SAMPLE_RATE as f64 * CHANNELS as f64);
    Some(DecodedAudio {
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS,
        samples: Arc::new(out_frames),
        duration_sec,
    })
}

/// WAV または各種メディアファイル（MP3 / M4A / AAC / FLAC / MP4 / AVI）の音声を
/// 44.1kHz 16-bit ステレオ PCM に一括デコードします。
/// WAV ファイルは FFmpeg なしで直接高速ネイティブデコードし、その他は FFmpeg を使用します。
pub fn decode_audio_pcm(ffmpeg_opt: Option<&Path>, media_path: &Path) -> Option<DecodedAudio> {
    // 1. まず WAV ネイティブデコーダーを試行（FFmpeg 不要＆超高速）
    if let Ok(bytes) = std::fs::read(media_path) {
        if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
            if let Some(decoded) = decode_wav_pcm(&bytes) {
                return Some(decoded);
            }
        }
    }

    // 2. FFmpeg が利用可能な場合は FFmpeg でデコード
    let Some(ffmpeg) = ffmpeg_opt else {
        return None;
    };

    let mut cmd = Command::new(ffmpeg);
    au2_platform::hide_console_window(&mut cmd);
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
        .stderr(Stdio::piped())
        .output()
        .ok()?;

    if !output.status.success() || output.stdout.is_empty() {
        if !output.stderr.is_empty() {
            log::warn!(
                "FFmpeg audio decode failed for {:?}: {}",
                media_path,
                String::from_utf8_lossy(&output.stderr)
            );
        }
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

/// `cursor_sample_frame` から `chunk_frames` サンプルフレーム分の音声を全トラックから合成し、
/// リトルエンディアン 16-bit ステレオ PCM バイト列として `target_buf` に書き込みます。
pub fn mix_audio_chunk(
    tracks: &[AudioTrackItem],
    cursor_sample_frame: usize,
    chunk_frames: usize,
    fps: f64,
    target_buf: &mut [u8],
) {
    let mut mix_buf = vec![0.0f32; chunk_frames * (CHANNELS as usize)];
    let chunk_start_sec = cursor_sample_frame as f64 / SAMPLE_RATE as f64;
    let chunk_end_sec = (cursor_sample_frame + chunk_frames) as f64 / SAMPLE_RATE as f64;
    let fps_safe = fps.max(1.0);

    for trk in tracks {
        let track_start_sec = trk.start_frame as f64 / fps_safe;
        let track_end_sec = (trk.end_frame + 1) as f64 / fps_safe;
        if chunk_end_sec <= track_start_sec || chunk_start_sec >= track_end_sec {
            continue;
        }

        let track_start_sample_frame = (track_start_sec * SAMPLE_RATE as f64).round() as isize;
        let offset_sample_frame = (trk.offset_sec * SAMPLE_RATE as f64).round() as isize;
        let vol = trk.volume.clamp(0.0, 2.0);
        let total_interleaved = trk.audio.len();

        for f in 0..chunk_frames {
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

    for (i, &sample_f32) in mix_buf.iter().enumerate() {
        let clamped = sample_f32.clamp(-32768.0, 32767.0) as i16;
        let b = clamped.to_le_bytes();
        if i * 2 + 1 < target_buf.len() {
            target_buf[i * 2] = b[0];
            target_buf[i * 2 + 1] = b[1];
        }
    }
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_snake_case)]
struct AudioStreamBasicDescription {
    mSampleRate: f64,
    mFormatID: u32,
    mFormatFlags: u32,
    mBytesPerPacket: u32,
    mFramesPerPacket: u32,
    mBytesPerFrame: u32,
    mChannelsPerFrame: u32,
    mBitsPerChannel: u32,
    mReserved: u32,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[allow(non_snake_case)]
struct AudioQueueBuffer {
    mAudioDataBytesCapacity: u32,
    mAudioData: *mut u8,
    mAudioDataByteSize: u32,
    mUserData: *mut std::ffi::c_void,
    mPacketDescriptionCapacity: u32,
    mPacketDescriptions: *const std::ffi::c_void,
    mPacketDescriptionCount: u32,
}

#[cfg(target_os = "macos")]
type AudioQueueRef = *mut std::ffi::c_void;

#[cfg(target_os = "macos")]
#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    fn AudioQueueNewOutput(
        inFormat: *const AudioStreamBasicDescription,
        inCallbackProc: extern "C" fn(*mut std::ffi::c_void, AudioQueueRef, *mut AudioQueueBuffer),
        inUserData: *mut std::ffi::c_void,
        inCallbackRunLoop: *mut std::ffi::c_void,
        inCallbackRunLoopMode: *mut std::ffi::c_void,
        inFlags: u32,
        outAQ: *mut AudioQueueRef,
    ) -> i32;
    fn AudioQueueAllocateBuffer(
        inAQ: AudioQueueRef,
        inBufferByteSize: u32,
        outBuffer: *mut *mut AudioQueueBuffer,
    ) -> i32;
    fn AudioQueueEnqueueBuffer(
        inAQ: AudioQueueRef,
        inBuffer: *mut AudioQueueBuffer,
        inNumPacketDescs: u32,
        inPacketDescs: *const std::ffi::c_void,
    ) -> i32;
    fn AudioQueueStart(inAQ: AudioQueueRef, inStartTime: *const std::ffi::c_void) -> i32;
    fn AudioQueueReset(inAQ: AudioQueueRef) -> i32;
    fn AudioQueueStop(inAQ: AudioQueueRef, inImmediate: u8) -> i32;
    fn AudioQueueDispose(inAQ: AudioQueueRef, inImmediate: u8) -> i32;
}

#[cfg(target_os = "macos")]
extern "C" fn macos_aq_callback(
    _user_data: *mut std::ffi::c_void,
    _aq: AudioQueueRef,
    buf: *mut AudioQueueBuffer,
) {
    unsafe {
        if !buf.is_null() && !(*buf).mUserData.is_null() {
            let flag = &*((*buf).mUserData as *const AtomicBool);
            flag.store(false, Ordering::Release);
        }
    }
}

/// クロスプラットフォーム対応のリアルタイム・オーディオ出力エンジン。
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
                // 再生中に大きくシークまたはループバックした場合のみ再シーク（許容差を1.0秒に設定し、描画負荷によるリセット誤爆を防止）
                if let Some(inst) = st.seek_instant {
                    let expected_sec = st.seek_start_sec + inst.elapsed().as_secs_f64();
                    if (cur_sec - expected_sec).abs() > 1.0 {
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

        // 1チャンク = 2048 サンプルフレーム (約 46.4ms) × 8 バッファ = 約 370ms の極めて安定したリングキュー
        const CHUNK_FRAMES: usize = 2048;
        const BUFFER_COUNT: usize = 8;
        let mut raw_buffers: Vec<Vec<u8>> = vec![vec![0u8; CHUNK_FRAMES * (CHANNELS as usize) * 2]; BUFFER_COUNT];
        let mut hdrs: Vec<WAVEHDR> = Vec::with_capacity(BUFFER_COUNT);
        let mut is_prepared: Vec<bool> = vec![false; BUFFER_COUNT];

        for buf in &mut raw_buffers {
            let hdr = WAVEHDR {
                lpData: buf.as_mut_ptr(),
                dwBufferLength: buf.len() as u32,
                dwBytesRecorded: 0,
                dwUser: 0,
                dwFlags: 0,
                dwLoops: 0,
                lpNext: 0,
                reserved: 0,
            };
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
                    for i in 0..BUFFER_COUNT {
                        if is_prepared[i] {
                            unsafe {
                                let _ = waveOutUnprepareHeader(hwave, &mut hdrs[i], std::mem::size_of::<WAVEHDR>() as u32);
                            }
                            is_prepared[i] = false;
                        }
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
                for i in 0..BUFFER_COUNT {
                    if is_prepared[i] {
                        unsafe {
                            let _ = waveOutUnprepareHeader(hwave, &mut hdrs[i], std::mem::size_of::<WAVEHDR>() as u32);
                        }
                        is_prepared[i] = false;
                    }
                }
                local_generation = gen;
                last_was_playing = true;
                cursor_sample_frame = (seek_sec.max(0.0) * SAMPLE_RATE as f64).round() as usize;
                buf_idx = 0;
            }

            // 現在対象としているバッファがまだ再生キューにある場合は少し待つ
            if is_prepared[buf_idx] && (hdrs[buf_idx].dwFlags & WHDR_INQUEUE) != 0 {
                thread::sleep(Duration::from_millis(4));
                continue;
            }

            // すでに再生が完了したバッファは一度 unprepare して再初期化（Windows waveOut 仕様完全準拠）
            if is_prepared[buf_idx] {
                unsafe {
                    let _ = waveOutUnprepareHeader(hwave, &mut hdrs[buf_idx], std::mem::size_of::<WAVEHDR>() as u32);
                }
                is_prepared[buf_idx] = false;
            }

            // `cursor_sample_frame` から正確に `CHUNK_FRAMES` 分をミックスして書き込む
            let target_buf = &mut raw_buffers[buf_idx];
            mix_audio_chunk(&tracks, cursor_sample_frame, CHUNK_FRAMES, fps, target_buf);

            let hdr = &mut hdrs[buf_idx];
            hdr.lpData = target_buf.as_mut_ptr();
            hdr.dwBufferLength = target_buf.len() as u32;
            hdr.dwBytesRecorded = 0;
            hdr.dwFlags = 0;

            unsafe {
                let prep_res = waveOutPrepareHeader(hwave, hdr, std::mem::size_of::<WAVEHDR>() as u32);
                if prep_res == 0 {
                    is_prepared[buf_idx] = true;
                    let write_res = waveOutWrite(hwave, hdr, std::mem::size_of::<WAVEHDR>() as u32);
                    if write_res != 0 {
                        log::warn!("waveOutWrite error: {write_res}");
                    }
                } else {
                    log::warn!("waveOutPrepareHeader error: {prep_res}");
                }
            }

            cursor_sample_frame += CHUNK_FRAMES;
            buf_idx = (buf_idx + 1) % BUFFER_COUNT;
        }

        unsafe {
            let _ = waveOutReset(hwave);
            for (i, hdr) in hdrs.iter_mut().enumerate() {
                if is_prepared[i] {
                    let _ = waveOutUnprepareHeader(hwave, hdr, std::mem::size_of::<WAVEHDR>() as u32);
                }
            }
            let _ = waveOutClose(hwave);
        }
    }

    #[cfg(target_os = "macos")]
    fn worker_loop(shared: Arc<Mutex<EngineSharedState>>, running: Arc<AtomicBool>) {
        let bytes_per_frame = (CHANNELS * BITS_PER_SAMPLE / 8) as u32;
        let desc = AudioStreamBasicDescription {
            mSampleRate: SAMPLE_RATE as f64,
            mFormatID: 0x6c70636d, // 'lpcm' (kAudioFormatLinearPCM)
            mFormatFlags: 0x0C,    // kLinearPCMFormatFlagIsSignedInteger | kLinearPCMFormatFlagIsPacked
            mBytesPerPacket: bytes_per_frame,
            mFramesPerPacket: 1,
            mBytesPerFrame: bytes_per_frame,
            mChannelsPerFrame: CHANNELS as u32,
            mBitsPerChannel: BITS_PER_SAMPLE as u32,
            mReserved: 0,
        };

        let mut aq: AudioQueueRef = std::ptr::null_mut();
        let status = unsafe {
            AudioQueueNewOutput(
                &desc,
                macos_aq_callback,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                &mut aq,
            )
        };
        if status != 0 || aq.is_null() {
            log::warn!("AudioQueueNewOutput failed ({status}), falling back to external audio sink");
            Self::stream_via_audio_sink_loop(shared, running);
            return;
        }

        const CHUNK_FRAMES: usize = 2048;
        const BUFFER_COUNT: usize = 8;
        let buf_byte_size = (CHUNK_FRAMES * (CHANNELS as usize) * 2) as u32;
        let in_queue: Vec<AtomicBool> = (0..BUFFER_COUNT).map(|_| AtomicBool::new(false)).collect();
        let mut aq_buffers: Vec<*mut AudioQueueBuffer> = vec![std::ptr::null_mut(); BUFFER_COUNT];

        for i in 0..BUFFER_COUNT {
            unsafe {
                if AudioQueueAllocateBuffer(aq, buf_byte_size, &mut aq_buffers[i]) == 0 && !aq_buffers[i].is_null() {
                    (*aq_buffers[i]).mUserData = &in_queue[i] as *const AtomicBool as *mut std::ffi::c_void;
                }
            }
        }

        let mut buf_idx = 0;
        let mut last_was_playing = false;
        let mut local_generation: u64 = 0;
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
                        let _ = AudioQueueReset(aq);
                        let _ = AudioQueueStop(aq, 1);
                    }
                    for flag in &in_queue {
                        flag.store(false, Ordering::Release);
                    }
                    last_was_playing = false;
                }
                thread::sleep(Duration::from_millis(10));
                continue;
            }

            if !last_was_playing || gen != local_generation {
                unsafe {
                    let _ = AudioQueueReset(aq);
                }
                for flag in &in_queue {
                    flag.store(false, Ordering::Release);
                }
                local_generation = gen;
                last_was_playing = true;
                cursor_sample_frame = (seek_sec.max(0.0) * SAMPLE_RATE as f64).round() as usize;
                buf_idx = 0;
            }

            if in_queue[buf_idx].load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(4));
                continue;
            }

            let buf_ptr = aq_buffers[buf_idx];
            if !buf_ptr.is_null() {
                unsafe {
                    let slice = std::slice::from_raw_parts_mut((*buf_ptr).mAudioData, buf_byte_size as usize);
                    mix_audio_chunk(&tracks, cursor_sample_frame, CHUNK_FRAMES, fps, slice);
                    (*buf_ptr).mAudioDataByteSize = buf_byte_size;
                    in_queue[buf_idx].store(true, Ordering::Release);
                    if AudioQueueEnqueueBuffer(aq, buf_ptr, 0, std::ptr::null()) == 0 {
                        let _ = AudioQueueStart(aq, std::ptr::null());
                    } else {
                        in_queue[buf_idx].store(false, Ordering::Release);
                    }
                }
            }

            cursor_sample_frame += CHUNK_FRAMES;
            buf_idx = (buf_idx + 1) % BUFFER_COUNT;
        }

        unsafe {
            let _ = AudioQueueStop(aq, 1);
            let _ = AudioQueueDispose(aq, 1);
        }
    }

    #[cfg(all(not(windows), not(target_os = "macos")))]
    fn worker_loop(shared: Arc<Mutex<EngineSharedState>>, running: Arc<AtomicBool>) {
        Self::stream_via_audio_sink_loop(shared, running);
    }

    /// Linux / Unix 環境向けの汎用 PCM ストリーミング出力ループ。
    /// システム上の `pw-cat` (PipeWire) / `pacat` (PulseAudio) / `aplay` (ALSA) / `ffplay` を自動検出して低遅延再生します。
    #[cfg(not(windows))]
    fn stream_via_audio_sink_loop(shared: Arc<Mutex<EngineSharedState>>, running: Arc<AtomicBool>) {
        use std::io::Write;
        use std::process::Child;

        fn spawn_sink() -> Option<Child> {
            let candidates: &[(&str, &[&str])] = &[
                ("pw-cat", &["--playback", "--rate", "44100", "--channels", "2", "--format", "s16", "--latency", "30ms", "-"]),
                ("pacat", &["--playback", "--rate=44100", "--channels=2", "--format=s16le", "--latency-msec=30"]),
                ("aplay", &["-q", "-f", "S16_LE", "-t", "raw", "-r", "44100", "-c", "2", "--buffer-size=4096", "-"]),
                ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet", "-fflags", "nobuffer", "-flags", "low_delay", "-f", "s16le", "-ar", "44100", "-ac", "2", "-i", "-"]),
            ];
            for (program, args) in candidates {
                let mut cmd = Command::new(program);
                au2_platform::hide_console_window(&mut cmd);
                if let Ok(child) = cmd
                    .args(*args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    return Some(child);
                }
            }
            None
        }

        fn close_sink(sink: &mut Option<Child>) {
            if let Some(mut child) = sink.take() {
                drop(child.stdin.take());
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        const CHUNK_FRAMES: usize = 1024;
        let mut pcm_buf = vec![0u8; CHUNK_FRAMES * (CHANNELS as usize) * 2];
        let mut sink: Option<Child> = None;
        let mut last_was_playing = false;
        let mut local_generation: u64 = 0;
        let mut cursor_sample_frame: usize = 0;
        let mut origin_sample_frame: usize = 0;
        let mut origin_instant = Instant::now();

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
                    close_sink(&mut sink);
                    last_was_playing = false;
                }
                thread::sleep(Duration::from_millis(10));
                continue;
            }

            if !last_was_playing || gen != local_generation {
                // シーク時は既存のオーディオシンクプロセスを殺さず常時接続のまま維持し、
                // プロセス再起動による遅延やポップノイズをゼロにする
                if sink.is_none() {
                    sink = spawn_sink();
                }
                local_generation = gen;
                last_was_playing = true;
                cursor_sample_frame = (seek_sec.max(0.0) * SAMPLE_RATE as f64).round() as usize;
                origin_sample_frame = cursor_sample_frame;
                origin_instant = Instant::now();
            }

            // 実時間に対して最大約 70ms (約3チャンク分) 先行して書き込むようペーシング
            let written_sec = cursor_sample_frame.saturating_sub(origin_sample_frame) as f64 / SAMPLE_RATE as f64;
            let elapsed_sec = origin_instant.elapsed().as_secs_f64();
            if written_sec > elapsed_sec + 0.07 {
                thread::sleep(Duration::from_millis(4));
                continue;
            }

            mix_audio_chunk(&tracks, cursor_sample_frame, CHUNK_FRAMES, fps, &mut pcm_buf);
            if let Some(child) = &mut sink {
                if let Some(stdin) = &mut child.stdin {
                    if stdin.write_all(&pcm_buf).is_err() {
                        close_sink(&mut sink);
                    } else {
                        let _ = stdin.flush();
                    }
                }
            }

            cursor_sample_frame += CHUNK_FRAMES;
        }

        close_sink(&mut sink);
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_wav_pcm_synthetic() {
        // 44.1kHz 16-bit ステレオ 100サンプルの合成 WAV バイナリを作成
        let num_frames = 100u32;
        let num_channels = 2u16;
        let bits_per_sample = 16u16;
        let sample_rate = 44100u32;
        let byte_rate = sample_rate * num_channels as u32 * (bits_per_sample as u32 / 8);
        let block_align = num_channels * (bits_per_sample / 8);
        let data_size = num_frames * block_align as u32;

        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVE");

        // fmt chunk
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&num_channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits_per_sample.to_le_bytes());

        // data chunk
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        for i in 0..num_frames {
            let sample_val = (i as i16) * 100;
            wav.extend_from_slice(&sample_val.to_le_bytes()); // L
            wav.extend_from_slice(&(-sample_val).to_le_bytes()); // R
        }

        let decoded = decode_wav_pcm(&wav).expect("should decode synthetic wav");
        assert_eq!(decoded.sample_rate, 44100);
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.samples.len(), 200);
        assert_eq!(decoded.samples[0], 0);
        assert_eq!(decoded.samples[1], 0);
        assert_eq!(decoded.samples[2], 100);
        assert_eq!(decoded.samples[3], -100);
    }

    #[test]
    fn test_decode_wav_pcm_48k_mono() {
        // 48kHz 16-bit モノラル 480 サンプル (10ms)
        let num_frames = 480u32;
        let num_channels = 1u16;
        let bits_per_sample = 16u16;
        let sample_rate = 48000u32;
        let block_align = 2u16;
        let byte_rate = sample_rate * block_align as u32;
        let data_size = num_frames * block_align as u32;

        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&num_channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits_per_sample.to_le_bytes());

        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        for i in 0..num_frames {
            let sample_val = (i as i16) * 10;
            wav.extend_from_slice(&sample_val.to_le_bytes());
        }

        let decoded = decode_wav_pcm(&wav).expect("should decode 48k mono wav");
        assert_eq!(decoded.sample_rate, 44100);
        assert_eq!(decoded.channels, 2);
        // 480 サンプル @ 48kHz -> 約 441 サンプル @ 44.1kHz * 2ch = 882
        assert!((decoded.samples.len() as i32 - 882).abs() <= 4);
    }

    #[test]
    fn test_mix_audio_chunk_multi_track() {
        let trk1 = AudioTrackItem {
            audio: Arc::new(vec![1000i16, -1000, 2000, -2000]),
            start_frame: 0,
            end_frame: 30,
            offset_sec: 0.0,
            volume: 1.0,
        };
        let trk2 = AudioTrackItem {
            audio: Arc::new(vec![500i16, 500, 1000, 1000]),
            start_frame: 0,
            end_frame: 30,
            offset_sec: 0.0,
            volume: 0.5,
        };
        let mut buf = vec![0u8; 2 * 4]; // 2 frames * 2ch * 2bytes
        mix_audio_chunk(&[trk1, trk2], 0, 2, 30.0, &mut buf);
        let s0_l = i16::from_le_bytes([buf[0], buf[1]]);
        let s0_r = i16::from_le_bytes([buf[2], buf[3]]);
        let s1_l = i16::from_le_bytes([buf[4], buf[5]]);
        let s1_r = i16::from_le_bytes([buf[6], buf[7]]);
        assert_eq!(s0_l, 1250);
        assert_eq!(s0_r, -750);
        assert_eq!(s1_l, 2500);
        assert_eq!(s1_r, -1500);
    }
}
