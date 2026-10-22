//! トラック（アニメーション可能なパラメータ）とキーフレーム。
//!
//! AviUtl 拡張編集では「中間点」がオブジェクト単位で共有され、各トラックバーは
//! 中間点ごとの値と、トラック単位の「移動方法」（直線移動・曲線移動など）を持ちます。
//! 本モデルはその構造を素直に表現しつつ、将来の拡張（キーフレーム毎の補間設定など）
//! を行えるよう `Keyframe` を独立した型にしています。

use crate::time::Frame;

/// 移動方法の種類。
///
/// 数値コードは `.exo` における移動方法番号（下位ビット）です。
/// 一部の番号の意味は実ファイルとの照合による検証が必要です（`Other` で生値を保持）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MoveKind {
    /// 直線移動 (1)
    Linear,
    /// 曲線移動 (2)
    Curve,
    /// 瞬間移動 (3)
    Instant,
    /// 中間点無視 (4)
    IgnoreMidpoint,
    /// 移動量指定 (5)
    MoveAmount,
    /// ランダム移動 (6)
    Random,
    /// スクリプトによる移動 (`N@スクリプト名`)。`.tra` スクリプトに相当。
    Script(String),
    /// 未知の移動方法番号（ラウンドトリップのため生値を保持）
    Other(u32),
}

impl MoveKind {
    pub fn label(&self) -> String {
        match self {
            MoveKind::Linear => "直線移動".into(),
            MoveKind::Curve => "曲線移動".into(),
            MoveKind::Instant => "瞬間移動".into(),
            MoveKind::IgnoreMidpoint => "中間点無視".into(),
            MoveKind::MoveAmount => "移動量指定".into(),
            MoveKind::Random => "ランダム移動".into(),
            MoveKind::Script(name) => format!("スクリプト: {name}"),
            MoveKind::Other(code) => format!("不明な移動方法({code})"),
        }
    }
}

/// 加減速設定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Easing {
    #[default]
    None,
    /// 加速
    EaseIn,
    /// 減速
    EaseOut,
    /// 加速＋減速
    EaseInOut,
}

impl Easing {
    pub fn from_flags(accel: bool, decel: bool) -> Self {
        match (accel, decel) {
            (false, false) => Easing::None,
            (true, false) => Easing::EaseIn,
            (false, true) => Easing::EaseOut,
            (true, true) => Easing::EaseInOut,
        }
    }

    pub fn accel(self) -> bool {
        matches!(self, Easing::EaseIn | Easing::EaseInOut)
    }

    pub fn decel(self) -> bool {
        matches!(self, Easing::EaseOut | Easing::EaseInOut)
    }

    /// 0..=1 の正規化時間に加減速カーブを適用します。
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::None => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseInOut => t * t * (3.0 - 2.0 * t),
        }
    }
}

/// 補間設定（トラック単位）。
#[derive(Debug, Clone, PartialEq)]
pub struct Interpolation {
    pub kind: MoveKind,
    pub easing: Easing,
    /// 移動方法の付加パラメータ（「移動フレーム間隔」やスクリプトの設定値など）。
    /// 解釈は移動方法ごとに異なるため、文字列のまま保持します。
    pub param: Option<String>,
    /// 読み込み元ファイルでの移動方法番号の生値（加減速フラグを除いた値）。
    /// スクリプト移動の番号などを書き出し時に復元するために保持します。
    pub legacy_code: Option<u32>,
}

impl Interpolation {
    pub fn new(kind: MoveKind) -> Self {
        Self { kind, easing: Easing::None, param: None, legacy_code: None }
    }

    pub fn linear() -> Self {
        Self::new(MoveKind::Linear)
    }
}

/// キーフレーム。`frame` はオブジェクト先頭からの相対フレームです。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Keyframe {
    pub frame: Frame,
    pub value: f64,
}

/// アニメーション可能な数値パラメータ（トラックバー）。
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// フレーム昇順に並んだキーフレーム。最低 1 つ。
    pub keyframes: Vec<Keyframe>,
    /// `None` のときは移動無し（先頭キーフレームの値で固定）。
    pub interpolation: Option<Interpolation>,
    /// 小数点以下の表示桁数（`.exo` 書き出し時の書式を保つため）。
    pub precision: u8,
}

impl Track {
    /// 固定値のトラック。
    pub fn constant(value: f64, precision: u8) -> Self {
        Self { keyframes: vec![Keyframe { frame: 0, value }], interpolation: None, precision }
    }

    pub fn is_animated(&self) -> bool {
        self.interpolation.is_some() && self.keyframes.len() >= 2
    }

    /// 先頭（開始点）の値。
    pub fn start_value(&self) -> f64 {
        self.keyframes.first().map(|k| k.value).unwrap_or(0.0)
    }

    /// 末尾（終了点）の値。
    pub fn end_value(&self) -> f64 {
        self.keyframes.last().map(|k| k.value).unwrap_or(0.0)
    }

    /// 指定した相対フレームでの値を評価します。
    pub fn evaluate(&self, frame: Frame) -> f64 {
        let Some(interp) = self.interpolation.as_ref().filter(|_| self.keyframes.len() >= 2) else {
            return self.start_value();
        };
        let keys = &self.keyframes;
        let first = keys[0];
        let last = *keys.last().unwrap();
        if frame <= first.frame {
            return first.value;
        }
        if frame >= last.frame {
            return last.value;
        }

        if interp.kind == MoveKind::IgnoreMidpoint {
            let t = norm(frame, first.frame, last.frame);
            return lerp(first.value, last.value, interp.easing.apply(t));
        }

        // frame を含む区間 [i, i+1) を探す
        let i = keys.partition_point(|k| k.frame <= frame).saturating_sub(1).min(keys.len() - 2);
        let (a, b) = (keys[i], keys[i + 1]);
        let t = interp.easing.apply(norm(frame, a.frame, b.frame));

        match &interp.kind {
            MoveKind::Instant => a.value,
            MoveKind::Curve => {
                let p0 = if i > 0 { keys[i - 1].value } else { a.value };
                let p3 = if i + 2 < keys.len() { keys[i + 2].value } else { b.value };
                catmull_rom(p0, a.value, b.value, p3, t)
            }
            // TODO: 移動量指定・ランダム移動・スクリプト移動は Phase 2 以降で正確に実装
            _ => lerp(a.value, b.value, t),
        }
    }

    /// 指定フレームの値を設定します。キーフレームが無ければ挿入します。
    pub fn set_value_at(&mut self, frame: Frame, value: f64) {
        if !self.is_animated() {
            if let Some(k) = self.keyframes.first_mut() {
                k.value = value;
            }
            return;
        }
        match self.keyframes.binary_search_by_key(&frame, |k| k.frame) {
            Ok(i) => self.keyframes[i].value = value,
            Err(i) => self.keyframes.insert(i, Keyframe { frame, value }),
        }
    }
}

fn norm(f: Frame, a: Frame, b: Frame) -> f64 {
    if b <= a {
        0.0
    } else {
        (f - a) as f64 / (b - a) as f64
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn catmull_rom(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1) + (-p0 + p2) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(kind: MoveKind, keys: &[(Frame, f64)]) -> Track {
        Track {
            keyframes: keys.iter().map(|&(frame, value)| Keyframe { frame, value }).collect(),
            interpolation: Some(Interpolation::new(kind)),
            precision: 1,
        }
    }

    #[test]
    fn constant_track() {
        let t = Track::constant(42.0, 1);
        assert_eq!(t.evaluate(0), 42.0);
        assert_eq!(t.evaluate(1000), 42.0);
    }

    #[test]
    fn linear_with_midpoint() {
        let t = track(MoveKind::Linear, &[(0, 0.0), (10, 100.0), (30, 0.0)]);
        assert_eq!(t.evaluate(5), 50.0);
        assert_eq!(t.evaluate(10), 100.0);
        assert_eq!(t.evaluate(20), 50.0);
        assert_eq!(t.evaluate(99), 0.0);
    }

    #[test]
    fn ignore_midpoint() {
        let t = track(MoveKind::IgnoreMidpoint, &[(0, 0.0), (10, 999.0), (20, 100.0)]);
        assert_eq!(t.evaluate(10), 50.0);
    }

    #[test]
    fn curve_passes_through_keys() {
        let t = track(MoveKind::Curve, &[(0, 0.0), (10, 100.0), (20, 0.0)]);
        assert!((t.evaluate(10) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn easing() {
        assert_eq!(Easing::EaseIn.apply(0.5), 0.25);
        assert_eq!(Easing::EaseOut.apply(0.5), 0.75);
    }
}
