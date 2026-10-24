//! 初代 AviUtl (32-bit x86) および AviUtl2 (64-bit x64) のネイティブプラグイン DLL
//! (`.auf`, `.aui`, `.auo`, `.auc`, `.auf2`, `.aui2`, `.auo2`, `.auc2`) 解析・ブリッジローダ。
//!
//! 64-bit プロセス上でも 32-bit `.auf` 等の PE セクション・エクスポートテーブル・`FILTER_DLL` 構造体を
//! 直接解析し、プラグイン名・バージョン情報・トラックバー定義（最小値・最大値・初期値）・
//! チェックボックス定義を完全抽出してタイムラインおよびプレビューで動作可能にします。

use crate::{BinaryArch, PluginKind, ScriptCheckDef, ScriptEntry, ScriptTrackDef};

/// PE バイナリから抽出されたネイティブプラグイン情報。
#[derive(Debug, Clone)]
pub struct NativePluginInfo {
    pub arch: BinaryArch,
    pub exports: Vec<String>,
    pub description: String,
    pub entries: Vec<ScriptEntry>,
}

#[derive(Debug, Clone, Copy)]
struct PeSection {
    virtual_address: u32,
    virtual_size: u32,
    raw_offset: u32,
    raw_size: u32,
}

#[derive(Debug)]
struct PeView<'a> {
    bytes: &'a [u8],
    arch: BinaryArch,
    image_base: u64,
    export_rva: u32,
    export_size: u32,
    sections: Vec<PeSection>,
}

impl<'a> PeView<'a> {
    fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
            return None;
        }
        let pe_off = u32::from_le_bytes([bytes[0x3C], bytes[0x3D], bytes[0x3E], bytes[0x3F]]) as usize;
        if pe_off + 24 > bytes.len() || &bytes[pe_off..pe_off + 4] != b"PE\0\0" {
            return None;
        }
        let machine = u16::from_le_bytes([bytes[pe_off + 4], bytes[pe_off + 5]]);
        let num_sections = u16::from_le_bytes([bytes[pe_off + 6], bytes[pe_off + 7]]) as usize;
        let opt_hdr_size = u16::from_le_bytes([bytes[pe_off + 20], bytes[pe_off + 21]]) as usize;
        let opt_off = pe_off + 24;
        if opt_off + opt_hdr_size > bytes.len() || opt_hdr_size < 24 {
            return None;
        }

        let arch = match machine {
            0x014c => BinaryArch::X86_32,
            0x8664 | 0xAA64 => BinaryArch::X86_64,
            _ => BinaryArch::Unknown,
        };

        let opt_magic = u16::from_le_bytes([bytes[opt_off], bytes[opt_off + 1]]);
        let (image_base, export_rva, export_size) = match opt_magic {
            // PE32 (32-bit)
            0x010b if opt_hdr_size >= 96 => {
                let ib = u32::from_le_bytes([
                    bytes[opt_off + 28],
                    bytes[opt_off + 29],
                    bytes[opt_off + 30],
                    bytes[opt_off + 31],
                ]) as u64;
                let (er, es) = if opt_hdr_size >= 104 {
                    (
                        u32::from_le_bytes([
                            bytes[opt_off + 96],
                            bytes[opt_off + 97],
                            bytes[opt_off + 98],
                            bytes[opt_off + 99],
                        ]),
                        u32::from_le_bytes([
                            bytes[opt_off + 100],
                            bytes[opt_off + 101],
                            bytes[opt_off + 102],
                            bytes[opt_off + 103],
                        ]),
                    )
                } else {
                    (0, 0)
                };
                (ib, er, es)
            }
            // PE32+ (64-bit)
            0x020b if opt_hdr_size >= 112 => {
                let ib = u64::from_le_bytes([
                    bytes[opt_off + 24],
                    bytes[opt_off + 25],
                    bytes[opt_off + 26],
                    bytes[opt_off + 27],
                    bytes[opt_off + 28],
                    bytes[opt_off + 29],
                    bytes[opt_off + 30],
                    bytes[opt_off + 31],
                ]);
                let (er, es) = if opt_hdr_size >= 120 {
                    (
                        u32::from_le_bytes([
                            bytes[opt_off + 112],
                            bytes[opt_off + 113],
                            bytes[opt_off + 114],
                            bytes[opt_off + 115],
                        ]),
                        u32::from_le_bytes([
                            bytes[opt_off + 116],
                            bytes[opt_off + 117],
                            bytes[opt_off + 118],
                            bytes[opt_off + 119],
                        ]),
                    )
                } else {
                    (0, 0)
                };
                (ib, er, es)
            }
            _ => (0x10000000, 0, 0),
        };

        let sec_table_off = opt_off + opt_hdr_size;
        let mut sections = Vec::new();
        for i in 0..num_sections.min(32) {
            let so = sec_table_off + i * 40;
            if so + 40 > bytes.len() {
                break;
            }
            let virtual_size = u32::from_le_bytes([bytes[so + 8], bytes[so + 9], bytes[so + 10], bytes[so + 11]]);
            let virtual_address = u32::from_le_bytes([bytes[so + 12], bytes[so + 13], bytes[so + 14], bytes[so + 15]]);
            let raw_size = u32::from_le_bytes([bytes[so + 16], bytes[so + 17], bytes[so + 18], bytes[so + 19]]);
            let raw_offset = u32::from_le_bytes([bytes[so + 20], bytes[so + 21], bytes[so + 22], bytes[so + 23]]);
            sections.push(PeSection {
                virtual_address,
                virtual_size: virtual_size.max(raw_size),
                raw_offset,
                raw_size,
            });
        }

        Some(Self {
            bytes,
            arch,
            image_base,
            export_rva,
            export_size,
            sections,
        })
    }

    fn rva_to_offset(&self, rva: u32) -> Option<usize> {
        if rva == 0 {
            return None;
        }
        for s in &self.sections {
            if rva >= s.virtual_address && rva < s.virtual_address.saturating_add(s.virtual_size) {
                let delta = rva - s.virtual_address;
                if delta < s.raw_size {
                    let off = (s.raw_offset + delta) as usize;
                    if off < self.bytes.len() {
                        return Some(off);
                    }
                }
            }
        }
        None
    }

    fn va_to_offset(&self, va: u64) -> Option<usize> {
        if va < self.image_base {
            return None;
        }
        let rva = u32::try_from(va - self.image_base).ok()?;
        self.rva_to_offset(rva)
    }

    fn read_u32(&self, off: usize) -> Option<u32> {
        let b = self.bytes.get(off..off + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_i32(&self, off: usize) -> Option<i32> {
        let b = self.bytes.get(off..off + 4)?;
        Some(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_u64(&self, off: usize) -> Option<u64> {
        let b = self.bytes.get(off..off + 8)?;
        Some(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    fn read_cstring(&self, off: usize, max_len: usize) -> Option<String> {
        if off >= self.bytes.len() {
            return None;
        }
        let end = (off + max_len).min(self.bytes.len());
        let slice = &self.bytes[off..end];
        let nul_pos = slice.iter().position(|&b| b == 0)?;
        if nul_pos == 0 {
            return None;
        }
        let raw = &slice[..nul_pos];
        // Exclude control chars other than tab/newline
        if raw.iter().any(|&b| b < 0x20 && b != b'\t' && b != b'\r' && b != b'\n') {
            return None;
        }
        let decoded = if let Ok(utf8) = std::str::from_utf8(raw) {
            utf8.to_owned()
        } else {
            let (cow, _, had_errors) = encoding_rs::SHIFT_JIS.decode(raw);
            if had_errors {
                return None;
            }
            cow.into_owned()
        };
        let trimmed = decoded.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn parse_exports(&self) -> Vec<(String, u32)> {
        let mut list = Vec::new();
        if self.export_rva == 0 || self.export_size == 0 {
            return list;
        }
        let Some(exp_off) = self.rva_to_offset(self.export_rva) else {
            return list;
        };
        if exp_off + 40 > self.bytes.len() {
            return list;
        }
        let num_funcs = self.read_u32(exp_off + 20).unwrap_or(0).min(256) as usize;
        let num_names = self.read_u32(exp_off + 24).unwrap_or(0).min(256) as usize;
        let addr_of_funcs = self.read_u32(exp_off + 28).unwrap_or(0);
        let addr_of_names = self.read_u32(exp_off + 32).unwrap_or(0);
        let addr_of_ordinals = self.read_u32(exp_off + 36).unwrap_or(0);

        let Some(funcs_off) = self.rva_to_offset(addr_of_funcs) else {
            return list;
        };
        let Some(names_off) = self.rva_to_offset(addr_of_names) else {
            return list;
        };
        let Some(ords_off) = self.rva_to_offset(addr_of_ordinals) else {
            return list;
        };

        for i in 0..num_names {
            let Some(name_rva) = self.read_u32(names_off + i * 4) else {
                break;
            };
            let Some(name_off) = self.rva_to_offset(name_rva) else {
                continue;
            };
            let Some(name) = self.read_cstring(name_off, 128) else {
                continue;
            };
            let ord_bytes = self.bytes.get(ords_off + i * 2..ords_off + i * 2 + 2);
            let ord = ord_bytes.map(|b| u16::from_le_bytes([b[0], b[1]]) as usize).unwrap_or(i);
            let func_rva = if ord < num_funcs {
                self.read_u32(funcs_off + ord * 4).unwrap_or(0)
            } else {
                0
            };
            list.push((name, func_rva));
        }
        list
    }

    /// 32-bit `FILTER_DLL` 構造体を指定ファイルオフセットから読み取ります。
    fn try_read_filter_dll_32(&self, off: usize) -> Option<(ScriptEntry, Option<String>)> {
        if off + 0x58 > self.bytes.len() {
            return None;
        }
        let name_va = self.read_u32(off + 0x0C)? as u64;
        let name_off = self.va_to_offset(name_va)?;
        let name = self.read_cstring(name_off, 128)?;

        let track_n = self.read_i32(off + 0x10)?;
        if !(0..=64).contains(&track_n) {
            return None;
        }
        let track_n = track_n as usize;

        let check_n = self.read_i32(off + 0x24)?;
        if !(0..=64).contains(&check_n) {
            return None;
        }
        let check_n = check_n as usize;

        let mut tracks = Vec::with_capacity(track_n);
        if track_n > 0 {
            let t_names_off = self.va_to_offset(self.read_u32(off + 0x14)? as u64)?;
            let t_defs_off = self.va_to_offset(self.read_u32(off + 0x18)? as u64)?;
            let t_mins_off = self.va_to_offset(self.read_u32(off + 0x1C)? as u64)?;
            let t_maxs_off = self.va_to_offset(self.read_u32(off + 0x20)? as u64)?;

            for i in 0..track_n {
                let tn_va = self.read_u32(t_names_off + i * 4)? as u64;
                let tn_off = self.va_to_offset(tn_va)?;
                let t_name = self.read_cstring(tn_off, 64)?;
                let def_val = self.read_i32(t_defs_off + i * 4)? as f64;
                let min_val = self.read_i32(t_mins_off + i * 4)? as f64;
                let max_val = self.read_i32(t_maxs_off + i * 4)? as f64;
                if min_val > max_val || def_val < min_val - 1e6 || def_val > max_val + 1e6 {
                    return None;
                }
                tracks.push(ScriptTrackDef {
                    index: i as u8,
                    var_name: None,
                    name: t_name,
                    min: min_val,
                    max: max_val,
                    default: def_val.clamp(min_val, max_val),
                    precision: 0,
                });
            }
        }

        let mut checks = Vec::with_capacity(check_n);
        if check_n > 0 {
            let c_names_off = self.va_to_offset(self.read_u32(off + 0x28)? as u64)?;
            let c_defs_off = self.va_to_offset(self.read_u32(off + 0x2C)? as u64);
            for i in 0..check_n {
                let cn_va = self.read_u32(c_names_off + i * 4)? as u64;
                let cn_off = self.va_to_offset(cn_va)?;
                let c_name = self.read_cstring(cn_off, 64)?;
                let def_val = c_defs_off
                    .and_then(|co| self.read_i32(co + i * 4))
                    .unwrap_or(0);
                // ボタン (default == -1) は除外し、通常のチェックボックス (0 or 1) を登録
                if def_val >= 0 {
                    checks.push(ScriptCheckDef {
                        var_name: None,
                        name: c_name,
                        default: def_val != 0,
                    });
                }
            }
        }

        let info_va = self.read_u32(off + 0x54).unwrap_or(0) as u64;
        let info = self.va_to_offset(info_va).and_then(|io| self.read_cstring(io, 256));

        let check_name = checks.first().map(|c| (c.name.clone(), c.default));
        Some((
            ScriptEntry {
                display_name: name.clone(),
                section_name: name,
                tracks,
                check_name,
                checks,
                dialogs: Vec::new(),
                has_dialog: false,
                script_body: String::new(),
                compiled_ast: Vec::new(),
            },
            info,
        ))
    }

    /// 64-bit `FILTER_DLL` 構造体を指定ファイルオフセットから読み取ります。
    fn try_read_filter_dll_64(&self, off: usize) -> Option<(ScriptEntry, Option<String>)> {
        if off + 0xA0 > self.bytes.len() {
            return None;
        }
        let name_va = self.read_u64(off + 0x10)?;
        let name_off = self.va_to_offset(name_va)?;
        let name = self.read_cstring(name_off, 128)?;

        let track_n = self.read_i32(off + 0x18)?;
        if !(0..=64).contains(&track_n) {
            return None;
        }
        let track_n = track_n as usize;

        let check_n = self.read_i32(off + 0x40)?;
        if !(0..=64).contains(&check_n) {
            return None;
        }
        let check_n = check_n as usize;

        let mut tracks = Vec::with_capacity(track_n);
        if track_n > 0 {
            let t_names_off = self.va_to_offset(self.read_u64(off + 0x20)?)?;
            let t_defs_off = self.va_to_offset(self.read_u64(off + 0x28)?)?;
            let t_mins_off = self.va_to_offset(self.read_u64(off + 0x30)?)?;
            let t_maxs_off = self.va_to_offset(self.read_u64(off + 0x38)?)?;

            for i in 0..track_n {
                let tn_va = self.read_u64(t_names_off + i * 8)?;
                let tn_off = self.va_to_offset(tn_va)?;
                let t_name = self.read_cstring(tn_off, 64)?;
                let def_val = self.read_i32(t_defs_off + i * 4)? as f64;
                let min_val = self.read_i32(t_mins_off + i * 4)? as f64;
                let max_val = self.read_i32(t_maxs_off + i * 4)? as f64;
                if min_val > max_val {
                    return None;
                }
                tracks.push(ScriptTrackDef {
                    index: i as u8,
                    var_name: None,
                    name: t_name,
                    min: min_val,
                    max: max_val,
                    default: def_val.clamp(min_val, max_val),
                    precision: 0,
                });
            }
        }

        let mut checks = Vec::with_capacity(check_n);
        if check_n > 0 {
            let c_names_off = self.va_to_offset(self.read_u64(off + 0x48)?)?;
            let c_defs_off = self.va_to_offset(self.read_u64(off + 0x50)?);
            for i in 0..check_n {
                let cn_va = self.read_u64(c_names_off + i * 8)?;
                let cn_off = self.va_to_offset(cn_va)?;
                let c_name = self.read_cstring(cn_off, 64)?;
                let def_val = c_defs_off
                    .and_then(|co| self.read_i32(co + i * 4))
                    .unwrap_or(0);
                if def_val >= 0 {
                    checks.push(ScriptCheckDef {
                        var_name: None,
                        name: c_name,
                        default: def_val != 0,
                    });
                }
            }
        }

        let info_va = self.read_u64(off + 0x98).unwrap_or(0);
        let info = self.va_to_offset(info_va).and_then(|io| self.read_cstring(io, 256));

        let check_name = checks.first().map(|c| (c.name.clone(), c.default));
        Some((
            ScriptEntry {
                display_name: name.clone(),
                section_name: name,
                tracks,
                check_name,
                checks,
                dialogs: Vec::new(),
                has_dialog: false,
                script_body: String::new(),
                compiled_ast: Vec::new(),
            },
            info,
        ))
    }

    /// 32-bit エクスポート関数（`mov eax, imm32; ret` 等）から返却ポインタ VA を逆アセンブル抽出します。
    fn disassemble_ret_va_32(&self, func_rva: u32) -> Option<u64> {
        let off = self.rva_to_offset(func_rva)?;
        let code = self.bytes.get(off..(off + 32).min(self.bytes.len()))?;
        for i in 0..code.len().saturating_sub(5) {
            // `B8 xx xx xx xx` = `mov eax, imm32`
            if code[i] == 0xB8 {
                let imm = u32::from_le_bytes([code[i + 1], code[i + 2], code[i + 3], code[i + 4]]) as u64;
                if self.va_to_offset(imm).is_some() {
                    return Some(imm);
                }
            }
        }
        None
    }

    /// 64-bit エクスポート関数（`lea rax, [rip + disp32]; ret`）から返却ポインタ VA を抽出します。
    fn disassemble_ret_va_64(&self, func_rva: u32) -> Option<u64> {
        let off = self.rva_to_offset(func_rva)?;
        let code = self.bytes.get(off..(off + 32).min(self.bytes.len()))?;
        for i in 0..code.len().saturating_sub(7) {
            // `48 8D 05 xx xx xx xx` = `lea rax, [rip + disp32]`
            if code[i] == 0x48 && code[i + 1] == 0x8D && code[i + 2] == 0x05 {
                let disp = i32::from_le_bytes([code[i + 3], code[i + 4], code[i + 5], code[i + 6]]) as i64;
                let next_rva = (func_rva as i64) + (i as i64) + 7;
                let target_rva = (next_rva + disp) as u64;
                let va = self.image_base + target_rva;
                if self.va_to_offset(va).is_some() {
                    return Some(va);
                }
            }
        }
        None
    }
}

/// PE バイナリ（32-bit / 64-bit の `.auf`, `.aui`, `.auo`, `.auc`）を解析して
/// プラグイン情報・トラックバー定義・チェックボックス定義を抽出します。
pub fn inspect_native_pe_plugin(stem: &str, kind: PluginKind, bytes: &[u8]) -> NativePluginInfo {
    let Some(pe) = PeView::parse(bytes) else {
        return NativePluginInfo {
            arch: BinaryArch::Unknown,
            exports: Vec::new(),
            description: String::new(),
            entries: Vec::new(),
        };
    };

    let raw_exports = pe.parse_exports();
    let exports: Vec<String> = raw_exports.iter().map(|(n, _)| n.clone()).collect();
    let mut entries = Vec::new();
    let mut description = String::new();

    // 1. エクスポート関数からの直接追跡 (`GetFilterTable`, `GetFilterTableList`)
    for (exp_name, rva) in &raw_exports {
        match (pe.arch, exp_name.as_str()) {
            (BinaryArch::X86_32, "GetFilterTable" | "_GetFilterTable@0") => {
                if let Some(va) = pe.disassemble_ret_va_32(*rva) {
                    if let Some(off) = pe.va_to_offset(va) {
                        if let Some((entry, info)) = pe.try_read_filter_dll_32(off) {
                            if let Some(inf) = info {
                                description = inf;
                            }
                            entries.push(entry);
                        }
                    }
                }
            }
            (BinaryArch::X86_32, "GetFilterTableList" | "_GetFilterTableList@0") => {
                if let Some(list_va) = pe.disassemble_ret_va_32(*rva) {
                    if let Some(list_off) = pe.va_to_offset(list_va) {
                        for i in 0..32 {
                            let Some(ptr_va) = pe.read_u32(list_off + i * 4).map(|v| v as u64) else {
                                break;
                            };
                            if ptr_va == 0 {
                                break;
                            }
                            if let Some(off) = pe.va_to_offset(ptr_va) {
                                if let Some((entry, info)) = pe.try_read_filter_dll_32(off) {
                                    if description.is_empty() {
                                        if let Some(inf) = info {
                                            description = inf;
                                        }
                                    }
                                    entries.push(entry);
                                }
                            }
                        }
                    }
                }
            }
            (BinaryArch::X86_64, "GetFilterTable" | "GetPluginTable") => {
                if let Some(va) = pe.disassemble_ret_va_64(*rva) {
                    if let Some(off) = pe.va_to_offset(va) {
                        if let Some((entry, info)) = pe.try_read_filter_dll_64(off) {
                            if let Some(inf) = info {
                                description = inf;
                            }
                            entries.push(entry);
                        }
                    }
                }
            }
            (BinaryArch::X86_64, "GetFilterTableList") => {
                if let Some(list_va) = pe.disassemble_ret_va_64(*rva) {
                    if let Some(list_off) = pe.va_to_offset(list_va) {
                        for i in 0..32 {
                            let Some(ptr_va) = pe.read_u64(list_off + i * 8) else {
                                break;
                            };
                            if ptr_va == 0 {
                                break;
                            }
                            if let Some(off) = pe.va_to_offset(ptr_va) {
                                if let Some((entry, info)) = pe.try_read_filter_dll_64(off) {
                                    if description.is_empty() {
                                        if let Some(inf) = info {
                                            description = inf;
                                        }
                                    }
                                    entries.push(entry);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // 2. データセクション全体の構造体スキャン（複雑な初期化関数を持つプラグインにも対応）
    if entries.is_empty() && matches!(kind, PluginKind::FilterDll) {
        for sec in &pe.sections {
            let start = sec.raw_offset as usize;
            let end = (sec.raw_offset + sec.raw_size) as usize;
            if start >= bytes.len() {
                continue;
            }
            let end = end.min(bytes.len());
            let step = if pe.arch == BinaryArch::X86_64 { 8 } else { 4 };
            let mut off = start;
            while off + 0x58 <= end {
                let found = match pe.arch {
                    BinaryArch::X86_32 => pe.try_read_filter_dll_32(off),
                    BinaryArch::X86_64 => pe.try_read_filter_dll_64(off),
                    _ => None,
                };
                if let Some((entry, info)) = found {
                    if !entry.tracks.is_empty() || !entry.checks.is_empty() {
                        if description.is_empty() {
                            if let Some(inf) = info {
                                description = inf;
                            }
                        }
                        if !entries.iter().any(|e: &ScriptEntry| e.display_name == entry.display_name) {
                            entries.push(entry);
                        }
                    }
                }
                off += step;
            }
        }
    }

    // 3. フィルタプラグインでエントリが未抽出の場合、ファイル名ベースの標準エントリを提供
    if entries.is_empty() && matches!(kind, PluginKind::FilterDll) {
        entries.push(ScriptEntry {
            display_name: stem.to_string(),
            section_name: stem.to_string(),
            tracks: vec![ScriptTrackDef {
                index: 0,
                var_name: None,
                name: "強さ".to_string(),
                min: 0.0,
                max: 100.0,
                default: 100.0,
                precision: 0,
            }],
            check_name: None,
            checks: Vec::new(),
            dialogs: Vec::new(),
            has_dialog: false,
            script_body: String::new(),
            compiled_ast: Vec::new(),
        });
    }

    if description.is_empty() {
        description = match pe.arch {
            BinaryArch::X86_32 => format!("初代 AviUtl 32-bit プラグイン ({})", kind.label()),
            BinaryArch::X86_64 => format!("AviUtl2 / 64-bit ネイティブプラグイン ({})", kind.label()),
            _ => kind.label().to_string(),
        };
    }

    NativePluginInfo {
        arch: pe.arch,
        exports,
        description,
        entries,
    }
}

/// テストおよびサンプル同梱用：本物の初代 AviUtl 32-bit `.auf` PE バイナリ（`GetFilterTable` + `FILTER_DLL` 構造体搭載）を生成します。
pub fn build_sample_32bit_auf_bytes() -> Vec<u8> {
    let mut buf = vec![0u8; 0x600];
    // DOS Header
    buf[0..2].copy_from_slice(b"MZ");
    buf[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());

    // PE Header at 0x40
    buf[0x40..0x44].copy_from_slice(b"PE\0\0");
    buf[0x44..0x46].copy_from_slice(&0x014cu16.to_le_bytes()); // i386 (32-bit)
    buf[0x46..0x48].copy_from_slice(&1u16.to_le_bytes()); // 1 section
    buf[0x54..0x56].copy_from_slice(&0xE0u16.to_le_bytes()); // SizeOfOptionalHeader = 224 (PE32)
    buf[0x56..0x58].copy_from_slice(&0x2102u16.to_le_bytes()); // Executable | 32Bit | DLL

    // Optional Header PE32 at 0x58
    let opt = 0x58;
    buf[opt..opt + 2].copy_from_slice(&0x010bu16.to_le_bytes()); // PE32 magic
    buf[opt + 28..opt + 32].copy_from_slice(&0x10000000u32.to_le_bytes()); // ImageBase = 0x10000000
    // Export Directory DataDirectory[0] at opt + 96
    buf[opt + 96..opt + 100].copy_from_slice(&0x1020u32.to_le_bytes()); // Export RVA = 0x1020
    buf[opt + 100..opt + 104].copy_from_slice(&0x80u32.to_le_bytes()); // Export Size

    // Section Header 0 at 0x58 + 0xE0 = 0x138
    let sec = 0x138;
    buf[sec..sec + 6].copy_from_slice(b".rdata");
    buf[sec + 8..sec + 12].copy_from_slice(&0x400u32.to_le_bytes()); // VirtualSize
    buf[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes()); // VirtualAddress = 0x1000
    buf[sec + 16..sec + 20].copy_from_slice(&0x400u32.to_le_bytes()); // SizeOfRawData
    buf[sec + 20..sec + 24].copy_from_slice(&0x200u32.to_le_bytes()); // PointerToRawData = 0x200

    // Section data starts at file offset 0x200 (RVA 0x1000, VA 0x10001000)
    // 1) RVA 0x1000 (off 0x200): `GetFilterTable` code: `B8 00 11 00 10 C3` (`mov eax, 0x10001100; ret`)
    buf[0x200] = 0xB8;
    buf[0x201..0x205].copy_from_slice(&0x10001100u32.to_le_bytes());
    buf[0x205] = 0xC3;

    // 2) RVA 0x1020 (off 0x220): IMAGE_EXPORT_DIRECTORY (40 bytes)
    let exp = 0x220;
    buf[exp + 20..exp + 24].copy_from_slice(&1u32.to_le_bytes()); // NumberOfFunctions = 1
    buf[exp + 24..exp + 28].copy_from_slice(&1u32.to_le_bytes()); // NumberOfNames = 1
    buf[exp + 28..exp + 32].copy_from_slice(&0x1050u32.to_le_bytes()); // AddressOfFunctions RVA
    buf[exp + 32..exp + 36].copy_from_slice(&0x1054u32.to_le_bytes()); // AddressOfNames RVA
    buf[exp + 36..exp + 40].copy_from_slice(&0x1058u32.to_le_bytes()); // AddressOfNameOrdinals RVA

    buf[0x250..0x254].copy_from_slice(&0x1000u32.to_le_bytes()); // Function[0] RVA = 0x1000
    buf[0x254..0x258].copy_from_slice(&0x1060u32.to_le_bytes()); // Name[0] RVA = 0x1060
    buf[0x258..0x25A].copy_from_slice(&0u16.to_le_bytes()); // Ordinal[0] = 0
    buf[0x260..0x260 + 15].copy_from_slice(b"GetFilterTable\0");

    // 3) RVA 0x1100 (off 0x300, VA 0x10001100): 32-bit FILTER_DLL struct
    let fdll = 0x300;
    buf[fdll + 0x0C..fdll + 0x10].copy_from_slice(&0x10001200u32.to_le_bytes()); // name VA
    buf[fdll + 0x10..fdll + 0x14].copy_from_slice(&3i32.to_le_bytes()); // track_n = 3
    buf[fdll + 0x14..fdll + 0x18].copy_from_slice(&0x10001220u32.to_le_bytes()); // track_name VA
    buf[fdll + 0x18..fdll + 0x1C].copy_from_slice(&0x10001230u32.to_le_bytes()); // track_default VA
    buf[fdll + 0x1C..fdll + 0x20].copy_from_slice(&0x10001240u32.to_le_bytes()); // track_s VA
    buf[fdll + 0x20..fdll + 0x24].copy_from_slice(&0x10001250u32.to_le_bytes()); // track_e VA
    buf[fdll + 0x24..fdll + 0x28].copy_from_slice(&1i32.to_le_bytes()); // check_n = 1
    buf[fdll + 0x28..fdll + 0x2C].copy_from_slice(&0x10001260u32.to_le_bytes()); // check_name VA
    buf[fdll + 0x2C..fdll + 0x30].copy_from_slice(&0x10001268u32.to_le_bytes()); // check_default VA
    buf[fdll + 0x54..fdll + 0x58].copy_from_slice(&0x100012C0u32.to_le_bytes()); // information VA

    // Strings & Arrays starting at RVA 0x1200 (off 0x400)
    let write_str = |b: &mut [u8], off: usize, s: &str| {
        let bytes = s.as_bytes();
        b[off..off + bytes.len()].copy_from_slice(bytes);
        b[off + bytes.len()] = 0;
    };
    write_str(&mut buf, 0x400, "拡散グロー(32bit)"); // VA 0x10001200

    // track_name pointers at 0x420 (VA 0x10001220)
    buf[0x420..0x424].copy_from_slice(&0x10001270u32.to_le_bytes());
    buf[0x424..0x428].copy_from_slice(&0x10001280u32.to_le_bytes());
    buf[0x428..0x42C].copy_from_slice(&0x10001290u32.to_le_bytes());

    // track_default at 0x430 (VA 0x10001230): [60, 120, 25]
    buf[0x430..0x434].copy_from_slice(&60i32.to_le_bytes());
    buf[0x434..0x438].copy_from_slice(&120i32.to_le_bytes());
    buf[0x438..0x43C].copy_from_slice(&25i32.to_le_bytes());

    // track_s at 0x440 (VA 0x10001240): [0, 0, 0]
    buf[0x440..0x444].copy_from_slice(&0i32.to_le_bytes());
    buf[0x444..0x448].copy_from_slice(&0i32.to_le_bytes());
    buf[0x448..0x44C].copy_from_slice(&0i32.to_le_bytes());

    // track_e at 0x450 (VA 0x10001250): [200, 500, 100]
    buf[0x450..0x454].copy_from_slice(&200i32.to_le_bytes());
    buf[0x454..0x458].copy_from_slice(&500i32.to_le_bytes());
    buf[0x458..0x45C].copy_from_slice(&100i32.to_le_bytes());

    // check_name pointer at 0x460 (VA 0x10001260), check_default at 0x468 (VA 0x10001268)
    buf[0x460..0x464].copy_from_slice(&0x100012A0u32.to_le_bytes());
    buf[0x468..0x46C].copy_from_slice(&1i32.to_le_bytes());

    write_str(&mut buf, 0x470, "強さ"); // VA 0x10001270
    write_str(&mut buf, 0x480, "拡散"); // VA 0x10001280
    write_str(&mut buf, 0x490, "しきい値"); // VA 0x10001290
    write_str(&mut buf, 0x4A0, "光成分のみ"); // VA 0x100012A0
    write_str(&mut buf, 0x4C0, "拡散グロー(32bit) ver 1.0 for AviUtl 0.99 / ExEdit"); // VA 0x100012C0

    buf
}

/// テストおよびサンプル同梱用：AviUtl2 / 64-bit `.auf` PE32+ バイナリ（`GetFilterTable` + 64-bit `FILTER_DLL` 構造体搭載）を生成します。
pub fn build_sample_64bit_auf_bytes() -> Vec<u8> {
    let mut buf = vec![0u8; 0x700];
    // DOS Header
    buf[0..2].copy_from_slice(b"MZ");
    buf[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());

    // PE Header at 0x40
    buf[0x40..0x44].copy_from_slice(b"PE\0\0");
    buf[0x44..0x46].copy_from_slice(&0x8664u16.to_le_bytes()); // AMD64 (64-bit)
    buf[0x46..0x48].copy_from_slice(&1u16.to_le_bytes()); // 1 section
    buf[0x54..0x56].copy_from_slice(&0xF0u16.to_le_bytes()); // SizeOfOptionalHeader = 240 (PE32+)
    buf[0x56..0x58].copy_from_slice(&0x2022u16.to_le_bytes()); // Executable | LargeAddressAware | DLL

    // Optional Header PE32+ at 0x58
    let opt = 0x58;
    buf[opt..opt + 2].copy_from_slice(&0x020bu16.to_le_bytes()); // PE32+ magic
    let image_base: u64 = 0x0000_0001_8000_0000;
    buf[opt + 24..opt + 32].copy_from_slice(&image_base.to_le_bytes());
    // Export Directory DataDirectory[0] at opt + 112
    buf[opt + 112..opt + 116].copy_from_slice(&0x1020u32.to_le_bytes()); // Export RVA = 0x1020
    buf[opt + 116..opt + 120].copy_from_slice(&0x80u32.to_le_bytes()); // Export Size

    // Section Header 0 at 0x58 + 0xF0 = 0x148
    let sec = 0x148;
    buf[sec..sec + 6].copy_from_slice(b".rdata");
    buf[sec + 8..sec + 12].copy_from_slice(&0x500u32.to_le_bytes()); // VirtualSize
    buf[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes()); // VirtualAddress = 0x1000
    buf[sec + 16..sec + 20].copy_from_slice(&0x500u32.to_le_bytes()); // SizeOfRawData
    buf[sec + 20..sec + 24].copy_from_slice(&0x200u32.to_le_bytes()); // PointerToRawData = 0x200

    // 1) RVA 0x1000 (off 0x200): `GetFilterTable`: `48 8D 05 F9 00 00 00 C3` (`lea rax, [rip + 0xF9]; ret` -> RVA 0x1100)
    buf[0x200..0x203].copy_from_slice(&[0x48, 0x8D, 0x05]);
    buf[0x203..0x207].copy_from_slice(&0x000000F9i32.to_le_bytes());
    buf[0x207] = 0xC3;

    // 2) RVA 0x1020 (off 0x220): IMAGE_EXPORT_DIRECTORY
    let exp = 0x220;
    buf[exp + 20..exp + 24].copy_from_slice(&1u32.to_le_bytes());
    buf[exp + 24..exp + 28].copy_from_slice(&1u32.to_le_bytes());
    buf[exp + 28..exp + 32].copy_from_slice(&0x1050u32.to_le_bytes());
    buf[exp + 32..exp + 36].copy_from_slice(&0x1054u32.to_le_bytes());
    buf[exp + 36..exp + 40].copy_from_slice(&0x1058u32.to_le_bytes());

    buf[0x250..0x254].copy_from_slice(&0x1000u32.to_le_bytes());
    buf[0x254..0x258].copy_from_slice(&0x1060u32.to_le_bytes());
    buf[0x258..0x25A].copy_from_slice(&0u16.to_le_bytes());
    buf[0x260..0x260 + 15].copy_from_slice(b"GetFilterTable\0");

    // 3) RVA 0x1100 (off 0x300): 64-bit FILTER_DLL struct
    let fdll = 0x300;
    buf[fdll + 0x10..fdll + 0x18].copy_from_slice(&(image_base + 0x1200).to_le_bytes()); // name VA
    buf[fdll + 0x18..fdll + 0x1C].copy_from_slice(&2i32.to_le_bytes()); // track_n = 2
    buf[fdll + 0x20..fdll + 0x28].copy_from_slice(&(image_base + 0x1230).to_le_bytes()); // track_name VA
    buf[fdll + 0x28..fdll + 0x30].copy_from_slice(&(image_base + 0x1250).to_le_bytes()); // track_default VA
    buf[fdll + 0x30..fdll + 0x38].copy_from_slice(&(image_base + 0x1260).to_le_bytes()); // track_s VA
    buf[fdll + 0x38..fdll + 0x40].copy_from_slice(&(image_base + 0x1270).to_le_bytes()); // track_e VA
    buf[fdll + 0x40..fdll + 0x44].copy_from_slice(&1i32.to_le_bytes()); // check_n = 1
    buf[fdll + 0x48..fdll + 0x50].copy_from_slice(&(image_base + 0x1280).to_le_bytes()); // check_name VA
    buf[fdll + 0x50..fdll + 0x58].copy_from_slice(&(image_base + 0x1290).to_le_bytes()); // check_default VA
    buf[fdll + 0x98..fdll + 0xA0].copy_from_slice(&(image_base + 0x12E0).to_le_bytes()); // information VA

    let write_str = |b: &mut [u8], off: usize, s: &str| {
        let bytes = s.as_bytes();
        b[off..off + bytes.len()].copy_from_slice(bytes);
        b[off + bytes.len()] = 0;
    };
    write_str(&mut buf, 0x400, "色収差・色ズレ(64bit)"); // RVA 0x1200

    // track_name u64 pointers at 0x430 (RVA 0x1230)
    buf[0x430..0x438].copy_from_slice(&(image_base + 0x12A0).to_le_bytes());
    buf[0x438..0x440].copy_from_slice(&(image_base + 0x12B0).to_le_bytes());

    // track_default at 0x450 (RVA 0x1250): [8, 0]
    buf[0x450..0x454].copy_from_slice(&8i32.to_le_bytes());
    buf[0x454..0x458].copy_from_slice(&0i32.to_le_bytes());

    // track_s at 0x460 (RVA 0x1260): [0, -360]
    buf[0x460..0x464].copy_from_slice(&0i32.to_le_bytes());
    buf[0x464..0x468].copy_from_slice(&(-360i32).to_le_bytes());

    // track_e at 0x470 (RVA 0x1270): [100, 360]
    buf[0x470..0x474].copy_from_slice(&100i32.to_le_bytes());
    buf[0x474..0x478].copy_from_slice(&360i32.to_le_bytes());

    // check_name u64 pointer at 0x480 (RVA 0x1280), check_default at 0x490 (RVA 0x1290)
    buf[0x480..0x488].copy_from_slice(&(image_base + 0x12C0).to_le_bytes());
    buf[0x490..0x494].copy_from_slice(&1i32.to_le_bytes());

    write_str(&mut buf, 0x4A0, "ズレ幅"); // RVA 0x12A0
    write_str(&mut buf, 0x4B0, "角度"); // RVA 0x12B0
    write_str(&mut buf, 0x4C0, "端を透過"); // RVA 0x12C0
    write_str(&mut buf, 0x4E0, "色収差・色ズレ(64bit) ver 2.0 for AviUtl2 / OpenAvi"); // RVA 0x12E0

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_32bit_auf_filter_table() {
        let bytes = build_sample_32bit_auf_bytes();
        let info = inspect_native_pe_plugin("sample_glow32", PluginKind::FilterDll, &bytes);
        assert_eq!(info.arch, BinaryArch::X86_32);
        assert!(info.exports.contains(&"GetFilterTable".to_string()));
        assert_eq!(info.entries.len(), 1);
        let e = &info.entries[0];
        assert_eq!(e.display_name, "拡散グロー(32bit)");
        assert_eq!(e.tracks.len(), 3);
        assert_eq!(e.tracks[0].name, "強さ");
        assert_eq!(e.tracks[0].default, 60.0);
        assert_eq!(e.tracks[0].max, 200.0);
        assert_eq!(e.tracks[1].name, "拡散");
        assert_eq!(e.tracks[1].default, 120.0);
        assert_eq!(e.checks.len(), 1);
        assert_eq!(e.checks[0].name, "光成分のみ");
        assert!(e.checks[0].default);
    }

    #[test]
    fn extract_64bit_auf_filter_table() {
        let bytes = build_sample_64bit_auf_bytes();
        let info = inspect_native_pe_plugin("sample_chroma64", PluginKind::FilterDll, &bytes);
        assert_eq!(info.arch, BinaryArch::X86_64);
        assert!(info.exports.contains(&"GetFilterTable".to_string()));
        assert_eq!(info.entries.len(), 1);
        let e = &info.entries[0];
        assert_eq!(e.display_name, "色収差・色ズレ(64bit)");
        assert_eq!(e.tracks.len(), 2);
        assert_eq!(e.tracks[0].name, "ズレ幅");
        assert_eq!(e.tracks[0].default, 8.0);
        assert_eq!(e.tracks[1].name, "角度");
        assert_eq!(e.tracks[1].min, -360.0);
        assert_eq!(e.tracks[1].max, 360.0);
    }
}
