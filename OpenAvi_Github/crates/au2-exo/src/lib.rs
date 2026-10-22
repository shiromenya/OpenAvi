//! # au2-exo
//!
//! AviUtl 拡張編集オブジェクトファイル（`.exo`）の読み書き。
//!
//! ## `.exo` の構造
//!
//! ```text
//! [exedit]              ← シーン設定
//! width=1280
//! ...
//! [0]                   ← オブジェクト（タイムライン上の 1 区間）
//! start=1               ← 1 始まり・終端を含む
//! end=60
//! layer=1
//! [0.0]                 ← エフェクト（先頭 = 本体）
//! _name=テキスト
//! text=4100...          ← UTF-16LE を 16 進文字列化したもの
//! [0.1]
//! _name=標準描画
//! X=0.0,100.0,1         ← 開始値,終了値,移動方法[,付加パラメータ]
//! [1]
//! start=61
//! end=120
//! layer=1
//! chain=1               ← 直前オブジェクトの続き（= 中間点で分割された区間）
//! ```
//!
//! 中間点を持つオブジェクトは `chain=1` で連結された複数セクションとして記録されるため、
//! 読み込み時に 1 つの [`TimelineObject`](au2_core::TimelineObject) に統合し、
//! 各トラックのキーフレーム列へ変換します。

mod encoding;
mod error;
mod ini;
mod reader;
mod writer;

pub use encoding::{decode_text_hex, encode_text_hex, TextEncoding};
pub use error::ExoError;
pub use reader::{parse_exo_bytes, parse_exo_str, ExoDocument};
pub use writer::{write_exo_bytes, write_exo_string};
