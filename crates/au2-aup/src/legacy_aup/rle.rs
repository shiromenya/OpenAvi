//! 初代 AviUtl (.aup) バイナリ向けの RLE 圧縮・伸張および Shift_JIS・プリミティブ読み書きユーティリティ。

/// 固定長バイナリバッファ内の先頭 null 終端（0x00）までのバイト列を Shift-JIS デコードします。
pub fn decode_sjis_cstring(slice: &[u8]) -> String {
    let null_idx = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(&slice[..null_idx]);
    cow.into_owned()
}

/// AviUtl 独自の RLE 伸張（AupUtil.Decomp 互換）
pub fn decompress_aup(input: &[u8], decomp_size: usize) -> Vec<u8> {
    let mut out = vec![0u8; decomp_size];
    let mut in_idx = 0;
    let mut out_idx = 0;

    while out_idx < decomp_size && in_idx < input.len() {
        let size1 = input[in_idx];
        in_idx += 1;
        if (size1 & 0x80) != 0 {
            let rep = if (size1 & 0x7F) != 0 {
                (size1 & 0x7F) as usize
            } else {
                if in_idx + 3 > input.len() {
                    break;
                }
                let s = input[in_idx] as usize
                    | ((input[in_idx + 1] as usize) << 8)
                    | ((input[in_idx + 2] as usize) << 16);
                in_idx += 3;
                s
            };
            if in_idx >= input.len() {
                break;
            }
            let b = input[in_idx];
            in_idx += 1;
            let end = (out_idx + rep).min(decomp_size);
            for i in out_idx..end {
                out[i] = b;
            }
            out_idx += rep;
        } else {
            let count = if size1 != 0 {
                size1 as usize
            } else {
                if in_idx + 3 > input.len() {
                    break;
                }
                let s = input[in_idx] as usize
                    | ((input[in_idx + 1] as usize) << 8)
                    | ((input[in_idx + 2] as usize) << 16);
                in_idx += 3;
                s
            };
            let end = (out_idx + count).min(decomp_size);
            for i in out_idx..end {
                if in_idx < input.len() {
                    out[i] = input[in_idx];
                    in_idx += 1;
                }
            }
            out_idx += count;
        }
    }
    out
}

pub(super) fn get_i32(slice: &[u8], offset: usize) -> Option<i32> {
    slice.get(offset..offset + 4).and_then(|s| s.try_into().ok()).map(i32::from_le_bytes)
}

#[inline]
pub(super) fn get_i16(slice: &[u8], offset: usize) -> Option<i16> {
    slice.get(offset..offset + 2).and_then(|s| s.try_into().ok()).map(i16::from_le_bytes)
}


pub(super) fn clean_file_path(raw: &[u8]) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let mut slice = raw;
    let mut is_null_drive = false;
    // 先頭バイトが 0x00 で直後に ":\" または ":/" が続く場合（AviUtl拡張編集特有の先頭nullドライブ文字）
    if slice.len() >= 3 && slice[0] == 0 && slice[1] == b':' && (slice[2] == b'\\' || slice[2] == b'/') {
        is_null_drive = true;
        slice = &slice[1..];
    } else if slice.len() >= 2 && slice[0] == 0 && (slice[1] == b'\\' || slice[1] == b'/') {
        slice = &slice[1..];
    }
    let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    let valid_slice = &slice[..end];
    if valid_slice.is_empty() {
        return None;
    }
    let (decoded, _, _) = encoding_rs::SHIFT_JIS.decode(valid_slice);
    let mut path = decoded.trim().to_string();
    if is_null_drive || path.starts_with(":\\") || path.starts_with(":/") {
        if path.starts_with(":\\") || path.starts_with(":/") {
            path = format!("C{}", path);
        } else {
            path = format!("C:{}", path);
        }
    }
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

/// 4バイト先頭がすべて同一バイトか判定します (`AupUtil.IsSame4Bytes` 互換)。
#[inline]
fn is_same_4_bytes(data: &[u8]) -> bool {
    data.len() >= 4 && data[0] == data[1] && data[0] == data[2] && data[0] == data[3]
}

/// AviUtl プロジェクトファイル (.aup) 固有の RLE 圧縮を行います (`AupUtil.Comp` 完全互換)。
pub fn compress_aup(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 4 + 64);
    let mut pos = 0usize;
    while pos < data.len() {
        if is_same_4_bytes(&data[pos..]) {
            let b = data[pos];
            let max_len = (data.len() - pos).min(0x7F_FFFF);
            let mut rep = 4usize;
            while rep < max_len && data[pos + rep] == b {
                rep += 1;
            }
            if rep < 0x80 {
                out.push((rep as u8) | 0x80);
            } else {
                let size_data = ((rep as u32) << 8) | 0x80;
                out.extend_from_slice(&size_data.to_le_bytes());
            }
            out.push(b);
            pos += rep;
        } else {
            let max_len = (data.len() - pos).min(0x7F_FFFF);
            let mut rep = 0usize;
            while rep < max_len && !is_same_4_bytes(&data[pos + rep..]) {
                rep += 1;
            }
            if rep < 0x80 {
                out.push(rep as u8);
            } else {
                let size_data = (rep as u32) << 8;
                out.extend_from_slice(&size_data.to_le_bytes());
            }
            out.extend_from_slice(&data[pos..pos + rep]);
            pos += rep;
        }
    }
    out
}

pub(super) fn encode_sjis_fixed(s: &str, max_len: usize) -> Vec<u8> {
    let mut out = vec![0u8; max_len];
    if max_len == 0 {
        return out;
    }
    let (cow, _, _) = encoding_rs::SHIFT_JIS.encode(s);
    let copy_len = cow.len().min(max_len.saturating_sub(1));
    out[..copy_len].copy_from_slice(&cow[..copy_len]);
    out
}