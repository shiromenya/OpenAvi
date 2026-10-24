//! タイムラインオブジェクト。

use crate::effect::{Effect, EffectKind, ParamValue};
use crate::id::ObjectId;
use crate::time::Frame;
use crate::track::Track;

/// タイムライン上での表示分類（色分けなどに使用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectCategory {
    /// 映像系メディアオブジェクト（動画・画像・テキスト・図形など）
    Media,
    /// 音声系メディアオブジェクト
    Audio,
    /// フィルタオブジェクト（本体を持たず、下位レイヤーに効果を掛ける）
    Filter,
    /// 制御オブジェクト（カメラ制御・グループ制御・時間制御）
    Control,
}

/// タイムライン上のオブジェクト 1 つ。
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineObject {
    pub id: ObjectId,
    /// 開始フレーム（0 始まり、含む）
    pub frame_start: Frame,
    /// 終了フレーム（0 始まり、含む）
    pub frame_end: Frame,
    /// 中間点（オブジェクト先頭からの相対フレーム）。昇順、0 と末尾は含まない。
    pub midpoints: Vec<Frame>,
    /// グループ番号（`group=`）
    pub group: Option<u32>,
    /// 上のオブジェクトでクリッピング（`clipping=1`）
    pub clipping: bool,
    /// カメラ制御の対象（`camera=1`）
    pub camera_target: bool,
    /// 音声オブジェクト（`audio=1`）
    pub audio: bool,
    /// 未解釈のヘッダキー（`overlay` 等）。ラウンドトリップ用に順序付きで保持。
    pub extra: Vec<(String, String)>,
    /// エフェクト列。通常は [本体, フィルタ..., 標準描画/標準再生]。
    pub effects: Vec<Effect>,
}

fn standard_draw_effect() -> Effect {
    let mut d = Effect::new("標準描画");
    d.push_track("X", Track::constant(0.0, 1));
    d.push_track("Y", Track::constant(0.0, 1));
    d.push_track("Z", Track::constant(0.0, 1));
    d.push_track("拡大率", Track::constant(100.0, 2));
    d.push_track("透明度", Track::constant(0.0, 1));
    d.push_track("回転", Track::constant(0.0, 2));
    d.push_text("blend", "0");
    d
}

impl TimelineObject {
    pub fn new(frame_start: Frame, frame_end: Frame) -> Self {
        Self {
            id: ObjectId::new(),
            frame_start,
            frame_end: frame_end.max(frame_start),
            midpoints: Vec::new(),
            group: None,
            clipping: false,
            camera_target: false,
            audio: false,
            extra: vec![("overlay".into(), "1".into())],
            effects: Vec::new(),
        }
    }

    /// 動画ファイルオブジェクトを生成します。
    pub fn new_video(start: Frame, end: Frame, file: impl Into<String>) -> Self {
        let mut obj = Self::new(start, end);
        let mut src = Effect::new("動画ファイル");
        src.push_track("再生位置", Track::constant(1.0, 0));
        src.push_track("再生速度", Track::constant(100.0, 1));
        src.push_track("ループ再生", Track::constant(0.0, 0));
        src.push_track("アルファチャンネルを読み込む", Track::constant(0.0, 0));
        src.push_text("file", file);
        obj.effects.push(src);
        obj.effects.push(standard_draw_effect());
        obj
    }

    /// 画像ファイルオブジェクトを生成します。
    pub fn new_image(start: Frame, end: Frame, file: impl Into<String>) -> Self {
        let mut obj = Self::new(start, end);
        let mut src = Effect::new("画像ファイル");
        src.push_text("file", file);
        obj.effects.push(src);
        obj.effects.push(standard_draw_effect());
        obj
    }

    /// 音声ファイルオブジェクトを生成します。
    pub fn new_audio(start: Frame, end: Frame, file: impl Into<String>) -> Self {
        let mut obj = Self::new(start, end);
        obj.audio = true;
        obj.extra.clear();
        let mut src = Effect::new("音声ファイル");
        src.push_track("再生位置", Track::constant(0.0, 2));
        src.push_track("再生速度", Track::constant(100.0, 1));
        src.push_track("ループ再生", Track::constant(0.0, 0));
        src.push_track("動画ファイルと連携", Track::constant(0.0, 0));
        src.push_text("file", file);
        let mut play = Effect::new("標準再生");
        play.push_track("音量", Track::constant(100.0, 1));
        play.push_track("左右", Track::constant(0.0, 1));
        obj.effects.push(src);
        obj.effects.push(play);
        obj
    }

    /// テキストオブジェクトを生成します。
    pub fn new_text(start: Frame, end: Frame, body: impl Into<String>) -> Self {
        let mut obj = Self::new(start, end);
        let mut src = Effect::new("テキスト");
        src.push_track("サイズ", Track::constant(48.0, 0));
        src.push_track("表示速度", Track::constant(0.0, 1));
        src.push_track("文字毎に個別オブジェクト", Track::constant(0.0, 0));
        src.push_track("移動座標上に表示する", Track::constant(0.0, 0));
        src.push_track("自動スクロール", Track::constant(0.0, 0));
        src.push_track("B", Track::constant(0.0, 0));
        src.push_track("I", Track::constant(0.0, 0));
        src.push_text("type", "0");
        src.push_text("autoadjust", "0");
        src.push_text("soft", "1");
        src.push_text("monospace", "0");
        src.push_text("align", "4");
        src.push_text("spacing_x", "0");
        src.push_text("spacing_y", "0");
        src.push_text("precision", "1");
        src.push_text("color", "ffffff");
        src.push_text("color2", "000000");
        src.push_text("font", "MS UI Gothic");
        src.push_text("text", body);
        obj.effects.push(src);
        obj.effects.push(standard_draw_effect());
        obj
    }

    /// 図形オブジェクトを生成します（`shape_type`: 0=背景, 1=円, 2=四角形, 3=三角形, 6=星型）。
    pub fn new_shape(start: Frame, end: Frame, shape_type: u32, color_hex: &str) -> Self {
        let mut obj = Self::new(start, end);
        let mut src = Effect::new("図形");
        src.push_track("サイズ", Track::constant(140.0, 0));
        src.push_track("縦横比", Track::constant(0.0, 1));
        src.push_track("ライン幅", Track::constant(4000.0, 0));
        src.push_text("type", shape_type.to_string());
        src.push_text("color", color_hex);
        src.push_text("name", "");
        obj.effects.push(src);
        obj.effects.push(standard_draw_effect());
        obj
    }

    /// フレームバッファオブジェクトを生成します。
    pub fn new_framebuffer(start: Frame, end: Frame) -> Self {
        let mut obj = Self::new(start, end);
        let mut src = Effect::new("フレームバッファ");
        src.push_track("フレームバッファをクリア", Track::constant(0.0, 0));
        obj.effects.push(src);
        obj.effects.push(standard_draw_effect());
        obj
    }

    /// 色調補正（フィルタオブジェクト）を生成します。
    pub fn new_color_correction(start: Frame, end: Frame) -> Self {
        let mut obj = Self::new(start, end);
        let mut src = Effect::new("色調補正");
        src.push_track("明るさ", Track::constant(100.0, 1));
        src.push_track("ｺﾝﾄﾗｽﾄ", Track::constant(100.0, 1));
        src.push_track("色相", Track::constant(0.0, 1));
        src.push_track("輝度", Track::constant(100.0, 1));
        src.push_track("彩度", Track::constant(100.0, 1));
        src.push_track("飽和する", Track::constant(0.0, 0));
        obj.effects.push(src);
        obj
    }

    /// グループ制御オブジェクトを生成します。
    pub fn new_group_control(start: Frame, end: Frame) -> Self {
        let mut obj = Self::new(start, end);
        let mut src = Effect::new("グループ制御");
        src.push_track("X", Track::constant(0.0, 1));
        src.push_track("Y", Track::constant(0.0, 1));
        src.push_track("Z", Track::constant(0.0, 1));
        src.push_track("拡大率", Track::constant(100.0, 2));
        src.push_track("回転", Track::constant(0.0, 2));
        src.push_text("range", "0");
        obj.effects.push(src);
        obj
    }

    /// カメラ制御オブジェクトを生成します。
    pub fn new_camera_control(start: Frame, end: Frame) -> Self {
        let mut obj = Self::new(start, end);
        let mut src = Effect::new("カメラ制御");
        src.push_track("X", Track::constant(0.0, 1));
        src.push_track("Y", Track::constant(0.0, 1));
        src.push_track("Z", Track::constant(-1024.0, 1));
        src.push_track("目標X", Track::constant(0.0, 1));
        src.push_track("目標Y", Track::constant(0.0, 1));
        src.push_track("目標Z", Track::constant(0.0, 1));
        obj.effects.push(src);
        obj
    }

    /// 指定した絶対フレーム位置に中間点を追加します。
    pub fn add_midpoint(&mut self, abs_frame: Frame) -> bool {
        if abs_frame <= self.frame_start || abs_frame >= self.frame_end {
            return false;
        }
        let rel = abs_frame - self.frame_start;
        if self.midpoints.contains(&rel) {
            return false;
        }
        self.midpoints.push(rel);
        self.midpoints.sort_unstable();

        // アニメーション有効な各トラックにもその相対フレームの補間値をキーフレーム登録
        for eff in &mut self.effects {
            for param in &mut eff.params {
                if let ParamValue::Track(track) = &mut param.value {
                    if track.is_animated() {
                        let val = track.evaluate(rel);
                        track.set_value_at(rel, val);
                    }
                }
            }
        }
        true
    }

    /// 指定した絶対フレーム位置（後半オブジェクトの先頭フレーム）でオブジェクトを2つに分割します。
    pub fn split_at(&mut self, abs_frame: Frame) -> Option<TimelineObject> {
        if abs_frame <= self.frame_start || abs_frame > self.frame_end {
            return None;
        }
        let split_rel = abs_frame - self.frame_start;
        let orig_end = self.frame_end;

        let mut right = self.clone();
        right.id = ObjectId::new();
        right.frame_start = abs_frame;
        right.frame_end = orig_end;
        right.midpoints = self
            .midpoints
            .iter()
            .copied()
            .filter(|&m| m > split_rel)
            .map(|m| m - split_rel)
            .collect();

        self.frame_end = abs_frame - 1;
        self.midpoints.retain(|&m| m < split_rel);

        Some(right)
    }

    /// フレーム数（終了フレームを含む）。
    pub fn length(&self) -> Frame {
        self.frame_end - self.frame_start + 1
    }

    pub fn contains(&self, frame: Frame) -> bool {
        (self.frame_start..=self.frame_end).contains(&frame)
    }

    /// 開始点・中間点・終了点の相対フレーム列。各トラックのキーフレーム位置に対応します。
    pub fn key_frames(&self) -> Vec<Frame> {
        let mut v = Vec::with_capacity(self.midpoints.len() + 2);
        v.push(0);
        v.extend(self.midpoints.iter().copied());
        let last = self.length() - 1;
        if last > 0 {
            v.push(last);
        }
        v
    }

    /// 本体エフェクト（先頭）。
    pub fn source(&self) -> Option<&Effect> {
        self.effects.first()
    }

    pub fn source_mut(&mut self) -> Option<&mut Effect> {
        self.effects.first_mut()
    }

    pub fn effect(&self, kind: &EffectKind) -> Option<&Effect> {
        self.effects.iter().find(|e| &e.kind == kind)
    }

    pub fn effect_mut(&mut self, kind: &EffectKind) -> Option<&mut Effect> {
        self.effects.iter_mut().find(|e| &e.kind == kind)
    }

    pub fn category(&self) -> ObjectCategory {
        match self.source().map(|e| &e.kind) {
            Some(EffectKind::AudioFile | EffectKind::SceneAudio) => ObjectCategory::Audio,
            Some(EffectKind::Camera | EffectKind::TimeControl | EffectKind::GroupControl) => ObjectCategory::Control,
            Some(EffectKind::Filter) | None => ObjectCategory::Filter,
            Some(_) if self.audio => ObjectCategory::Audio,
            Some(_) => ObjectCategory::Media,
        }
    }

    /// タイムライン上に表示する名前。
    pub fn display_name(&self) -> String {
        let Some(src) = self.source() else {
            return "(空のオブジェクト)".into();
        };
        match src.kind {
            EffectKind::Text => {
                let body = src.text("text").unwrap_or_default();
                let first_line = body.lines().next().unwrap_or_default();
                if first_line.is_empty() { src.name.clone() } else { first_line.to_string() }
            }
            EffectKind::VideoFile | EffectKind::ImageFile | EffectKind::AudioFile => src
                .text("file")
                .and_then(|f| f.rsplit(['\\', '/']).next())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| src.name.clone()),
            EffectKind::CustomObject => src
                .text("name")
                .filter(|s| !s.is_empty())
                .map(|n| format!("カスタム ({n})"))
                .unwrap_or_else(|| src.name.clone()),
            _ => src
                .text("name")
                .filter(|s| !s.is_empty() && src.kind != EffectKind::Shape)
                .map(|n| format!("{} ({n})", src.name))
                .unwrap_or_else(|| src.name.clone()),
        }
    }
}
