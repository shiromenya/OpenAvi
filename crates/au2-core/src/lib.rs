//! # au2-core
//!
//! OpenAvi のコアデータモデルクレート。
//!
//! タイムライン・シーン・レイヤー・オブジェクト・エフェクト・キーフレーム補間など、
//! 動画編集プロジェクトの全データ構造を純粋な Rust データモデルとして定義します。
//!
//! ## データ階層構造
//!
//! 階層構造は AviUtl / AviUtl2 拡張編集のデータモデルと完全な互換性を保持しています：
//!
//! ```text
//! Project
//!  └─ Scene (Root / Scene1..N)
//!      ├─ SceneSettings (解像度・フレームレート・音声サンプリング周波数等)
//!      └─ Layer[] (レイヤー1..100)
//!          └─ TimelineObject[] (タイムライン上の各クリップオブジェクト)
//!              ├─ midpoints (中間点 = クリップ共通のキーフレーム時間位置)
//!              └─ Effect[] (先頭=メディア本体, 以降=フィルタ効果, 末尾=標準描画等)
//!                  └─ Param[] (トラックバー / チェックボックス / テキスト設定)
//!                      └─ Track (開始値・終了値・移動方法・加減速ベジェ)
//!                          └─ Keyframe[]
//! ```
//!
//! ## クレートの設計方針
//!
//! このクレートは UI（egui / eframe）、GPU（wgpu）、ファイル形式（.aup / .exo / .aup2）に
//! 一切依存しない純粋なデータ層です。
//! 外部ファイルの入出力は `au2-exo` や `au2-aup`、レンダリングは `au2-media` や `au2-app` が担います。

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
