//! 識別子。

use std::sync::atomic::{AtomicU64, Ordering};

/// タイムラインオブジェクトの一意な ID（セッション内で一意）。
///
/// `.exo` のセクション番号はファイル内の並び順に過ぎず編集で変動するため、
/// UI の選択状態や Undo 履歴などは安定した `ObjectId` で参照します。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(pub u64);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl ObjectId {
    /// 新しい一意な ID を払い出します（スレッドセーフ）。
    pub fn new() -> Self {
        Self(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for ObjectId {
    fn default() -> Self {
        Self::new()
    }
}
