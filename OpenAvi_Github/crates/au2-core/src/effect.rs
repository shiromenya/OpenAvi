//! エフェクト（メディアオブジェクト本体・フィルタ効果・描画設定）とパラメータ。

use crate::time::Frame;
use crate::track::Track;

/// パラメータ値。
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    /// トラックバー・チェックボックスなど数値で表現されるパラメータ。
    Track(Track),
    /// ファイルパス・色・フォント名・テキスト本文などの文字列設定。
    Text(String),
}

/// 名前付きパラメータ。順序は `.exo` 上の並びを保持します。
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub key: String,
    pub value: ParamValue,
}

/// エフェクトの種類（`_name` から判定）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EffectKind {
    // --- メディアオブジェクト ---
    VideoFile,
    ImageFile,
    AudioFile,
    Text,
    Shape,
    FrameBuffer,
    AudioWaveform,
    Scene,
    SceneAudio,
    PreviousObject,
    ParticleOutput,
    CustomObject,
    // --- 制御オブジェクト ---
    Camera,
    TimeControl,
    GroupControl,
    // --- 描画/再生設定（オブジェクトの末尾に付く） ---
    StandardDraw,
    ExtendedDraw,
    StandardPlayback,
    // --- その他（フィルタ効果・プラグインフィルタ・未知のもの） ---
    Filter,
}

impl EffectKind {
    pub fn from_name(name: &str) -> Self {
        match name {
            "動画ファイル" => Self::VideoFile,
            "画像ファイル" => Self::ImageFile,
            "音声ファイル" => Self::AudioFile,
            "テキスト" => Self::Text,
            "図形" => Self::Shape,
            "フレームバッファ" => Self::FrameBuffer,
            "音声波形表示" => Self::AudioWaveform,
            "シーン" => Self::Scene,
            "シーン(音声)" => Self::SceneAudio,
            "直前オブジェクト" => Self::PreviousObject,
            "パーティクル出力" => Self::ParticleOutput,
            "カスタムオブジェクト" => Self::CustomObject,
            "カメラ制御" => Self::Camera,
            "時間制御" => Self::TimeControl,
            "グループ制御" => Self::GroupControl,
            "標準描画" => Self::StandardDraw,
            "拡張描画" => Self::ExtendedDraw,
            "標準再生" => Self::StandardPlayback,
            _ => Self::Filter,
        }
    }

    /// オブジェクトの先頭に置かれる「本体」エフェクトかどうか。
    pub fn is_source(&self) -> bool {
        !matches!(self, Self::Filter | Self::StandardDraw | Self::ExtendedDraw | Self::StandardPlayback)
    }
}

/// エフェクト 1 つ分（`.exo` の `[N.M]` セクションに対応）。
#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    /// 表示名（`_name`）。プラグインフィルタやスクリプト名もここに入ります。
    pub name: String,
    pub kind: EffectKind,
    /// `_disable=1` のとき false。
    pub enabled: bool,
    pub params: Vec<Param>,
}

impl Effect {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self { kind: EffectKind::from_name(&name), name, enabled: true, params: Vec::new() }
    }

    pub fn param(&self, key: &str) -> Option<&ParamValue> {
        self.params.iter().find(|p| p.key == key).map(|p| &p.value)
    }

    pub fn param_mut(&mut self, key: &str) -> Option<&mut ParamValue> {
        self.params.iter_mut().find(|p| p.key == key).map(|p| &mut p.value)
    }

    pub fn track(&self, key: &str) -> Option<&Track> {
        match self.param(key)? {
            ParamValue::Track(t) => Some(t),
            ParamValue::Text(_) => None,
        }
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        match self.param(key)? {
            ParamValue::Text(s) => Some(s),
            ParamValue::Track(_) => None,
        }
    }

    /// トラックを相対フレームで評価します。存在しなければ `default`。
    pub fn eval(&self, key: &str, frame: Frame, default: f64) -> f64 {
        self.track(key).map(|t| t.evaluate(frame)).unwrap_or(default)
    }

    pub fn push_track(&mut self, key: impl Into<String>, track: Track) {
        self.params.push(Param { key: key.into(), value: ParamValue::Track(track) });
    }

    pub fn push_text(&mut self, key: impl Into<String>, text: impl Into<String>) {
        self.params.push(Param { key: key.into(), value: ParamValue::Text(text.into()) });
    }
}
