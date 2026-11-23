//! シーン音声のオフラインミキサー（44.1kHz 16-bit ステレオ WAV 生成）。

use std::path::PathBuf;

use au2_core::{EffectKind, Frame};
use au2_media::audio::{CHANNELS, SAMPLE_RATE};

use crate::state::audio_sync::{eval_audio_fade_gain, is_audio_mono};
use crate::state::EditorState;

/// シーンの指定フレーム範囲 `[start_frame..=end_frame]` の音声を 44.1kHz 16-bit ステレオ PCM にミックスし、
/// RIFF WAVE バイト列として返します。
/// （音量・左右パン・再生位置・再生速度・音声フェードに対応）
pub fn mix_scene_audio_to_wav_bytes(
    st: &mut EditorState,
    start_frame: Frame,
    end_frame: Frame,
    fps: f64,
) -> Vec<u8> {
    let fps = fps.max(1.0);
    let total_frames = end_frame.saturating_sub(start_frame) + 1;
    let duration_sec = total_frames as f64 / fps;
    let total_stereo_frames = ((duration_sec * SAMPLE_RATE as f64).round() as usize).max(1);
    let mut mix_buf = vec![0.0f32; total_stereo_frames * (CHANNELS as usize)];

    struct AudioClip {
        is_audio_obj: bool,
        file_str: String,
        frame_start: Frame,
        frame_end: Frame,
        obj_clone: au2_core::TimelineObject,
    }

    let mut candidates = Vec::new();
    for layer in &st.scene.layers {
        if !layer.visible {
            continue;
        }
        for obj in &layer.objects {
            if obj.frame_end < start_frame || obj.frame_start > end_frame {
                continue;
            }
            let Some(src) = obj.source().filter(|s| s.enabled) else {
                continue;
            };
            if matches!(src.kind, EffectKind::SceneAudio | EffectKind::Scene) {
                let scene_id = src
                    .text("scene")
                    .and_then(|s| s.trim().parse::<u32>().ok())
                    .unwrap_or(1);
                let start_pos_f = src.eval("再生位置", 0, 0.0).max(0.0).round() as Frame;
                if let Some(sub_scene) = st.scenes.get(&scene_id) {
                    for s_layer in &sub_scene.layers {
                        if !s_layer.visible {
                            continue;
                        }
                        for s_obj in &s_layer.objects {
                            let Some(s_src) = s_obj.source().filter(|s| s.enabled) else {
                                continue;
                            };
                            if !matches!(s_src.kind, EffectKind::AudioFile | EffectKind::VideoFile) {
                                continue;
                            }
                            let Some(file_str) = s_src.text("file").filter(|s| !s.trim().is_empty()) else {
                                continue;
                            };
                            if s_obj.frame_end < start_pos_f {
                                continue;
                            }
                            let rel_s = s_obj.frame_start.saturating_sub(start_pos_f);
                            let rel_e = s_obj.frame_end.saturating_sub(start_pos_f);
                            let mapped_s = obj.frame_start + rel_s;
                            let mapped_e = (obj.frame_start + rel_e).min(obj.frame_end);
                            if mapped_s <= mapped_e && mapped_e >= start_frame && mapped_s <= end_frame {
                                let mut cloned = s_obj.clone();
                                cloned.frame_start = mapped_s;
                                cloned.frame_end = mapped_e;
                                candidates.push(AudioClip {
                                    is_audio_obj: s_src.kind == EffectKind::AudioFile,
                                    file_str: file_str.to_string(),
                                    frame_start: mapped_s,
                                    frame_end: mapped_e,
                                    obj_clone: cloned,
                                });
                            }
                        }
                    }
                }
                continue;
            }
            if !matches!(src.kind, EffectKind::AudioFile | EffectKind::VideoFile) {
                continue;
            }
            let Some(file_str) = src.text("file").filter(|s| !s.trim().is_empty()) else {
                continue;
            };
            candidates.push(AudioClip {
                is_audio_obj: src.kind == EffectKind::AudioFile,
                file_str: file_str.to_string(),
                frame_start: obj.frame_start,
                frame_end: obj.frame_end,
                obj_clone: obj.clone(),
            });
        }
    }

    // AudioFile オブジェクトを優先登録し、ペアの VideoFile との2重再生を防止
    candidates.sort_by_key(|c| !c.is_audio_obj);
    let mut registered_keys: std::collections::HashSet<(PathBuf, Frame)> = std::collections::HashSet::new();

    for clip in candidates {
        let Some(path) = st.resolve_cached_path(&clip.file_str) else {
            continue;
        };
        if !registered_keys.insert((path.clone(), clip.frame_start)) {
            continue;
        };
        let Some(pcm) = st.media.get_audio_pcm(&path) else {
            continue;
        };
        if pcm.is_empty() {
            continue;
        }

        let src_total_frames = pcm.len() / 2;
        let Some(src_eff) = clip.obj_clone.source() else {
            continue;
        };

        let clip_start_f = clip.frame_start.max(start_frame);
        let clip_end_f = clip.frame_end.min(end_frame);
        let out_sample_start = (((clip_start_f - start_frame) as f64 / fps) * SAMPLE_RATE as f64).round() as usize;
        let out_sample_end = (((clip_end_f + 1 - start_frame) as f64 / fps) * SAMPLE_RATE as f64)
            .round()
            .min(total_stereo_frames as f64) as usize;

        let base_start_sec = if clip.is_audio_obj {
            src_eff.eval("再生位置", 0, 0.0).max(0.0)
        } else {
            ((src_eff.eval("再生位置", 0, 1.0) - 1.0) / fps).max(0.0)
        };
        let speed = (src_eff.eval("再生速度", 0, 100.0) / 100.0).max(0.0);
        let loop_play = src_eff.eval("ループ再生", 0, src_eff.eval("check0", 0, 0.0)) >= 0.5;
        let is_mono = is_audio_mono(&clip.obj_clone);

        let obj_dur_sec = (clip.frame_end.saturating_sub(clip.frame_start) + 1) as f64 / fps;

        for out_idx in out_sample_start..out_sample_end {
            let global_sec = out_idx as f64 / SAMPLE_RATE as f64 + (start_frame as f64 / fps);
            let rel_sec = (global_sec - clip.frame_start as f64 / fps).max(0.0);
            let rel_frame = (rel_sec * fps).floor() as u32;

            let src_sec = base_start_sec + rel_sec * speed;
            let mut src_sample_idx = (src_sec * SAMPLE_RATE as f64).floor() as usize;
            if loop_play && src_total_frames > 0 {
                src_sample_idx %= src_total_frames;
            } else if src_sample_idx >= src_total_frames {
                break;
            }

            let vol = (clip
                .obj_clone
                .effect(&EffectKind::StandardPlayback)
                .map(|e| e.eval("音量", rel_frame, 100.0))
                .unwrap_or(100.0)
                / 100.0)
                .clamp(0.0, 5.0) as f32;

            let pan = (clip
                .obj_clone
                .effect(&EffectKind::StandardPlayback)
                .map(|e| e.eval("左右", rel_frame, 0.0))
                .unwrap_or(0.0)
                / 100.0)
                .clamp(-1.0, 1.0) as f32;

            let remain_sec = (obj_dur_sec - rel_sec).max(0.0);
            let fade_gain = eval_audio_fade_gain(&clip.obj_clone, rel_frame, rel_sec, remain_sec);

            let gain_l = vol * fade_gain * (1.0 - pan.max(0.0));
            let gain_r = vol * fade_gain * (1.0 + pan.min(0.0));

            let mut l = pcm[src_sample_idx * 2] as f32 / 32768.0;
            let mut r = pcm[src_sample_idx * 2 + 1] as f32 / 32768.0;
            if is_mono {
                let m = (l + r) * 0.5;
                l = m;
                r = m;
            }

            mix_buf[out_idx * 2] += l * gain_l;
            mix_buf[out_idx * 2 + 1] += r * gain_r;
        }
    }

    // RIFF WAVE (44.1kHz 16-bit Stereo PCM) エンコード
    let data_bytes_len = (mix_buf.len() * 2) as u32;
    let riff_size = 36 + data_bytes_len;
    let byte_rate = SAMPLE_RATE * (CHANNELS as u32) * 2;
    let block_align = CHANNELS * 2;

    let mut wav = Vec::with_capacity(44 + data_bytes_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM = 1
    wav.extend_from_slice(&CHANNELS.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes()); // 16 bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes_len.to_le_bytes());

    for sample in mix_buf {
        let s = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        wav.extend_from_slice(&s.to_le_bytes());
    }

    wav
}