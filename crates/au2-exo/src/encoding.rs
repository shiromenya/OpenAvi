//! 文字コード処理。
//!
//! - 旧 AviUtl (1.x / 拡張編集 0.92) の `.exo` は Shift_JIS (CP932)。
//! - 新しいツールが出力するファイルは UTF-8 の場合があるため自動判別します。
//! - テキストオブジェクトの本文 `text=` は UTF-16LE のバイト列を 16 進文字列化したもの。

/// `.exo` ファイル全体の文字コード。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEncoding {
    #[default]
    ShiftJis,
    Utf8,
    Utf8Bom,
}

/// バイト列を文字列にデコードし、判別した文字コードを返します。
pub fn decode(bytes: &[u8]) -> (String, TextEncoding) {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (String::from_utf8_lossy(rest).into_owned(), TextEncoding::Utf8Bom);
    }
    // Shift_JIS の日本語列が厳密な UTF-8 として妥当になることは稀なので、UTF-8 を先に試す。
    if let Ok(s) = std::str::from_utf8(bytes) {
        return (s.to_owned(), TextEncoding::Utf8);
    }
    let (s, _, _) = encoding_rs::SHIFT_JIS.decode(bytes);
    (s.into_owned(), TextEncoding::ShiftJis)
}

/// 文字列を指定の文字コードでエンコードします。
pub fn encode(text: &str, enc: TextEncoding) -> Vec<u8> {
    match enc {
        TextEncoding::ShiftJis => encoding_rs::SHIFT_JIS.encode(text).0.into_owned(),
        TextEncoding::Utf8 => text.as_bytes().to_vec(),
        TextEncoding::Utf8Bom => {
            let mut v = vec![0xEF, 0xBB, 0xBF];
            v.extend_from_slice(text.as_bytes());
            v
        }
    }
}

/// `text=` の 16 進文字列（UTF-16LE, NUL 終端）をデコードします。
pub fn decode_text_hex(hex: &str) -> Option<String> {
    let hex = hex.trim().as_bytes();
    let mut units = Vec::with_capacity(hex.len() / 4);
    for chunk in hex.chunks_exact(4) {
        let s = std::str::from_utf8(chunk).ok()?;
        let lo = u8::from_str_radix(&s[0..2], 16).ok()?;
        let hi = u8::from_str_radix(&s[2..4], 16).ok()?;
        let unit = u16::from_le_bytes([lo, hi]);
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    Some(String::from_utf16_lossy(&units))
}

/// 拡張編集のテキスト本文バッファ長（16 進文字数）。
pub const TEXT_HEX_LEN: usize = 4096;

/// 文字列を `text=` 形式の 16 進文字列にエンコードします（4096 文字まで 0 埋め）。
pub fn encode_text_hex(text: &str) -> String {
    let mut out = String::with_capacity(TEXT_HEX_LEN);
    for unit in text.encode_utf16() {
        let [lo, hi] = unit.to_le_bytes();
        out.push_str(&format!("{lo:02x}{hi:02x}"));
    }
    while out.len() < TEXT_HEX_LEN {
        out.push('0');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_hex_roundtrip() {
        let s = "こんにちは\r\nAviUtl";
        let hex = encode_text_hex(s);
        assert_eq!(hex.len(), TEXT_HEX_LEN);
        assert!(hex.starts_with("53309330"));
        assert_eq!(decode_text_hex(&hex).unwrap(), s);
    }

    #[test]
    fn detect_sjis() {
        let bytes = encode("_name=標準描画", TextEncoding::ShiftJis);
        let (s, enc) = decode(&bytes);
        assert_eq!(enc, TextEncoding::ShiftJis);
        assert_eq!(s, "_name=標準描画");
    }
}
