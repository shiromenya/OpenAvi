use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExoError {
    #[error("[exedit] セクションが見つかりません（.exo ファイルではない可能性があります）")]
    MissingHeader,
    #[error("{line} 行目: オブジェクト [{section}] の必須キー `{key}` がありません")]
    MissingKey { line: usize, section: String, key: &'static str },
    #[error("{line} 行目: `{key}` の値 `{value}` を解釈できません")]
    InvalidValue { line: usize, key: String, value: String },
    #[error("I/O エラー: {0}")]
    Io(#[from] std::io::Error),
}
