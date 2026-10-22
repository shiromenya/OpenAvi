//! # au2-core
//!
//! AviUtl2-OSS のコアデータモデル。
//!
//! 階層構造は AviUtl 拡張編集の概念をそのまま踏襲しています。
//!
//! ```text
//! Project
//!  └─ Scene (Root / Scene1..N)
//!      ├─ SceneSettings (解像度・フレームレート・音声設定)
//!      └─ Layer[] (レイヤー1..100)
//!          └─ TimelineObject[] (タイムライン上のオブジェクト)
//!              ├─ midpoints (中間点 = オブジェクト共通のキーフレーム位置)
//!              └─ Effect[] (先頭=メディアオブジェクト本体, 以降=フィルタ効果, 末尾=標準描画等)
//!                  └─ Param[] (トラックバー / チェックボックス / 文字列設定)
//!                      └─ Track
//!                          └─ Keyframe[]
//! ```
//!
//! このクレートは UI・GPU・ファイル形式に一切依存しない純粋なデータ層です。
//! `.exo` / `.aup` などの入出力は別クレート (`au2-exo` など) が担当します。

pub mod effect;
pub mod id;
pub mod object;
pub mod project;
pub mod time;
pub mod track;

pub use effect::{Effect, EffectKind, Param, ParamValue};
pub use id::ObjectId;
pub use object::{ObjectCategory, TimelineObject};
pub use project::{Layer, Project, Scene, SceneSettings, MAX_LAYERS};
pub use time::{Frame, Rational};
pub use track::{Easing, Interpolation, Keyframe, MoveKind, Track};
