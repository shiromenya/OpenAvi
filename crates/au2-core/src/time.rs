//! 時間・フレーム関連の基本型。

/// 0 始まりのフレーム番号。
///
/// `.exo` は 1 始まりで記録されるため、入出力層で変換します。
pub type Frame = u32;

/// 有理数（フレームレートなど）。AviUtl の `rate / scale` 表現に対応します。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rational {
    pub num: u32,
    pub den: u32,
}

impl Rational {
    pub const fn new(num: u32, den: u32) -> Self {
        Self { num, den: if den == 0 { 1 } else { den } }
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// フレーム番号を秒に変換します。
    pub fn frame_to_seconds(self, frame: Frame) -> f64 {
        frame as f64 * self.den as f64 / self.num.max(1) as f64
    }

    /// 秒をフレーム番号に変換します（切り捨て）。
    pub fn seconds_to_frame(self, seconds: f64) -> Frame {
        (seconds * self.as_f64()).max(0.0).floor() as Frame
    }

    /// `HH:MM:SS.mmm` 形式のタイムコード文字列。
    pub fn timecode(self, frame: Frame) -> String {
        let s = self.frame_to_seconds(frame);
        let total_ms = (s * 1000.0).round() as u64;
        let ms = total_ms % 1000;
        let sec = (total_ms / 1000) % 60;
        let min = (total_ms / 60_000) % 60;
        let hour = total_ms / 3_600_000;
        format!("{hour:02}:{min:02}:{sec:02}.{ms:03}")
    }
}

impl Default for Rational {
    fn default() -> Self {
        Self::new(30, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntsc_rate() {
        let r = Rational::new(30000, 1001);
        assert!((r.as_f64() - 29.97).abs() < 0.01);
        assert_eq!(r.seconds_to_frame(r.frame_to_seconds(1234) + 1e-9), 1234);
    }

    #[test]
    fn timecode_format() {
        let r = Rational::new(30, 1);
        assert_eq!(r.timecode(30 * 61 + 15), "00:01:01.500");
    }
}
