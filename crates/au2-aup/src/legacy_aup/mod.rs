//! 初代 AviUtl (.aup) プロジェクトファイルおよび拡張編集 (exedit.auf)
//! バイナリデータのポータブル・パーサーおよびエンコーダ。
//!
//! Windows のレガシー API や外部ランタイムに依存せず、AviUtl の公式バイナリ仕様および
//! 拡張編集の内部データ構造（RLE 圧縮、EditHandle、FilterProject、ExEditProject）を
//! 純粋な Rust のみで高速かつ安全に読み書きします。

mod reader;
mod rle;
mod tables;
mod writer;

pub use reader::parse_legacy_aup;
pub use rle::{compress_aup, decode_sjis_cstring, decompress_aup};
pub use tables::{
    BUILTIN_ANM_NAMES, BUILTIN_CAM_NAMES, BUILTIN_OBJ_NAMES, BUILTIN_SCN_NAMES, BUILTIN_SCN_TYPES,
    DEFAULT_EFFECT_TYPES,
};
pub use writer::{write_legacy_aup, write_legacy_aup_multi_scene};