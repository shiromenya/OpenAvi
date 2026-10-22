//! # au2-plugin
//!
//! OpenAvi におけるプラグイン（`.auf`, `.aui`, `.auo`, `.auc`）および
//! 拡張編集スクリプト（`.anm`, `.obj`, `.cam`, `.scn`, `.tra`）の検出・解析・管理クレート。
//!
//! ## 対応機能
//! - **拡張編集スクリプト解析**:
//!   - Shift_JIS / UTF-8 自動判別
//!   - 単一スクリプトおよび `@セクション名` によるマルチスクリプト対応
//!   - `--track0`〜`--track3`（名前, 最小値, 最大値, 初期値, ステップ単位）の解析
//!   - `--check0`（名前, 初期値）および `--dialog` の解析
//!   - スクリプト定義から [`Effect`](au2_core::Effect) を直接生成し、タイムラインオブジェクトに追加可能
//! - **ネイティブ DLL プラグイン検査**:
//!   - PE ヘッダ解析による 32-bit (`IMAGE_FILE_MACHINE_I386`) / 64-bit (`IMAGE_FILE_MACHINE_AMD64`) 判定
//!   - `.auf` (フィルタ), `.aui` (入力), `.auo` (出力), `.auc` (色変換) の分類と実行ブリッジ状態管理

use std::fs;
use std::path::{Path, PathBuf};

use au2_core::{Effect, Track};

/// プラグインまたはスクリプトの分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginKind {
    /// `.anm` アニメーション効果スクリプト
    AnmScript,
    /// `.obj` カスタムオブジェクトスクリプト
    ObjScript,
    /// `.cam` カメラ効果スクリプト
    CamScript,
    /// `.scn` シーンチェンジスクリプト
    ScnScript,
    /// `.tra` トラックバー移動方法スクリプト
    TraScript,
    /// `.auf` フィルタプラグイン DLL
    FilterDll,
    /// `.aui` 入力プラグイン DLL
    InputDll,
    /// `.auo` 出力プラグイン DLL
    OutputDll,
    /// `.auc` 色変換プラグイン DLL
    ColorDll,
}

impl PluginKind {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "anm" => Some(Self::AnmScript),
            "obj" => Some(Self::ObjScript),
            "cam" => Some(Self::CamScript),
            "scn" => Some(Self::ScnScript),
            "tra" => Some(Self::TraScript),
            "auf" => Some(Self::FilterDll),
            "aui" => Some(Self::InputDll),
            "auo" => Some(Self::OutputDll),
            "auc" => Some(Self::ColorDll),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::AnmScript => "アニメーション効果 (.anm)",
            Self::ObjScript => "カスタムオブジェクト (.obj)",
            Self::CamScript => "カメラ効果 (.cam)",
            Self::ScnScript => "シーンチェンジ (.scn)",
            Self::TraScript => "移動方法スクリプト (.tra)",
            Self::FilterDll => "フィルタプラグイン (.auf)",
            Self::InputDll => "入力プラグイン (.aui)",
            Self::OutputDll => "出力プラグイン (.auo)",
            Self::ColorDll => "色変換プラグイン (.auc)",
        }
    }

    pub fn is_script(self) -> bool {
        matches!(
            self,
            Self::AnmScript | Self::ObjScript | Self::CamScript | Self::ScnScript | Self::TraScript
        )
    }
}

/// DLL のバイナリアーキテクチャ（PE ヘッダから判定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryArch {
    /// 32-bit x86 (従来の AviUtl プラグイン)
    X86_32,
    /// 64-bit x86_64 (OpenAvi / AviUtl2 ネイティブプラグイン)
    X86_64,
    /// スクリプトファイル（アーキテクチャ非依存）
    Script,
    /// 不明なバイナリ形式
    Unknown,
}

impl BinaryArch {
    pub fn label(self) -> &'static str {
        match self {
            Self::X86_32 => "32-bit (x86 / 要IPCブリッジ)",
            Self::X86_64 => "64-bit (x64 ネイティブ)",
            Self::Script => "Lua スクリプト",
            Self::Unknown => "不明",
        }
    }
}

/// 現在の動作サポート状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportStatus {
    /// パラメータ解析・タイムライン適用・プレビュー反映に対応
    Supported,
    /// メタデータ・パラメータ定義の読み込みに対応（LuaJIT 描画実行は Phase 3 予定）
    PartialScript,
    /// 32bit プラグインとして認識済み（32bit IPC ブリッジは Phase 4 予定）
    RequiresBridge32,
    /// 64bit DLL として認識済み（FFI ホストバインディング準備中）
    PendingNativeHost,
}

impl SupportStatus {
    pub fn summary(self) -> &'static str {
        match self {
            Self::Supported => "動作対応 (プレビュー反映)",
            Self::PartialScript => "パラメータ展開対応 (Lua描画はPhase 3)",
            Self::RequiresBridge32 => "認識済 (32bit IPCブリッジはPhase 4)",
            Self::PendingNativeHost => "認識済 (ネイティブDLLホスト準備中)",
        }
    }
}

/// スクリプトの `--track0`〜`--track3` 定義。
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptTrackDef {
    pub index: u8,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub precision: u8,
}

/// スクリプト内の1つのエフェクトエントリ（単一ファイルまたは `@名前` セクション）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptEntry {
    /// 表示名（例: `揺れ@sample_effects` または `サンプル効果`）
    pub display_name: String,
    /// セクション名（`@` 以降の名前。単一ファイルの場合はファイルステム）
    pub section_name: String,
    pub tracks: Vec<ScriptTrackDef>,
    pub check_name: Option<(String, bool)>,
    pub has_dialog: bool,
}

impl ScriptEntry {
    /// このスクリプト定義から初期状態の [`Effect`] を生成します。
    pub fn to_effect(&self, kind: PluginKind) -> Effect {
        let effect_name = match kind {
            PluginKind::AnmScript => "アニメーション効果",
            PluginKind::ObjScript => "カスタムオブジェクト",
            PluginKind::CamScript => "カメラ効果",
            PluginKind::ScnScript => "シーンチェンジ",
            _ => &self.display_name,
        };
        let mut eff = Effect::new(effect_name);
        for t in &self.tracks {
            eff.push_track(t.name.clone(), Track::constant(t.default, t.precision));
        }
        if let Some((chk_name, def_val)) = &self.check_name {
            eff.push_track(chk_name.clone(), Track::constant(if *def_val { 1.0 } else { 0.0 }, 0));
        }
        eff.push_text("name", self.display_name.clone());
        eff
    }
}

/// 検出されたプラグイン / スクリプト 1 ファイルの情報。
#[derive(Debug, Clone)]
pub struct PluginDescriptor {
    pub file_name: String,
    pub path: PathBuf,
    pub kind: PluginKind,
    pub arch: BinaryArch,
    pub status: SupportStatus,
    /// スクリプトの場合、含まれるエントリ一覧
    pub entries: Vec<ScriptEntry>,
}

/// PE ヘッダを読み取り、32bit / 64bit DLL を判別します。
pub fn detect_pe_arch(bytes: &[u8]) -> BinaryArch {
    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        return BinaryArch::Unknown;
    }
    let pe_offset = u32::from_le_bytes([bytes[0x3C], bytes[0x3D], bytes[0x3E], bytes[0x3F]]) as usize;
    if pe_offset + 6 > bytes.len() || &bytes[pe_offset..pe_offset + 4] != b"PE\0\0" {
        return BinaryArch::Unknown;
    }
    let machine = u16::from_le_bytes([bytes[pe_offset + 4], bytes[pe_offset + 5]]);
    match machine {
        0x014c => BinaryArch::X86_32,
        0x8664 | 0xAA64 => BinaryArch::X86_64,
        _ => BinaryArch::Unknown,
    }
}

fn decode_auto(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    encoding_rs::SHIFT_JIS.decode(bytes).0.into_owned()
}

/// スクリプトテキストから [`ScriptEntry`] 一覧を抽出します。
pub fn parse_script_entries(stem: &str, text: &str) -> Vec<ScriptEntry> {
    let mut entries = Vec::new();
    let mut current: Option<ScriptEntry> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if let Some(sec) = line.strip_prefix('@') {
            if let Some(prev) = current.take() {
                entries.push(prev);
            }
            let sec_name = sec.trim().to_string();
            let display_name = if sec_name.is_empty() {
                stem.to_string()
            } else {
                format!("{sec_name}@{stem}")
            };
            current = Some(ScriptEntry {
                display_name,
                section_name: sec_name,
                tracks: Vec::new(),
                check_name: None,
                has_dialog: false,
            });
            continue;
        }

        if let Some(rest) = line.strip_prefix("--track") {
            let entry = current.get_or_insert_with(|| ScriptEntry {
                display_name: stem.to_string(),
                section_name: stem.to_string(),
                tracks: Vec::new(),
                check_name: None,
                has_dialog: false,
            });
            if let Some(def) = parse_track_directive(rest) {
                entry.tracks.push(def);
            }
        } else if let Some(rest) = line.strip_prefix("--check0:") {
            let entry = current.get_or_insert_with(|| ScriptEntry {
                display_name: stem.to_string(),
                section_name: stem.to_string(),
                tracks: Vec::new(),
                check_name: None,
                has_dialog: false,
            });
            if let Some((name, val)) = rest.split_once(',') {
                entry.check_name = Some((name.trim().to_string(), val.trim() == "1"));
            }
        } else if line.starts_with("--dialog:") {
            let entry = current.get_or_insert_with(|| ScriptEntry {
                display_name: stem.to_string(),
                section_name: stem.to_string(),
                tracks: Vec::new(),
                check_name: None,
                has_dialog: false,
            });
            entry.has_dialog = true;
        }
    }

    if let Some(last) = current.take() {
        entries.push(last);
    }
    if entries.is_empty() {
        entries.push(ScriptEntry {
            display_name: stem.to_string(),
            section_name: stem.to_string(),
            tracks: Vec::new(),
            check_name: None,
            has_dialog: false,
        });
    }
    entries
}

fn parse_track_directive(rest: &str) -> Option<ScriptTrackDef> {
    let (idx_str, body) = rest.split_once(':')?;
    let index: u8 = idx_str.trim().parse().ok()?;
    let parts: Vec<&str> = body.split(',').collect();
    if parts.len() < 4 {
        return None;
    }
    let name = parts[0].trim().to_string();
    let min: f64 = parts[1].trim().parse().ok()?;
    let max: f64 = parts[2].trim().parse().ok()?;
    let default: f64 = parts[3].trim().parse().ok()?;
    let precision = match parts.get(4).map(|s| s.trim()) {
        Some("1") => 0,
        Some("0.1") => 1,
        Some("0.01") => 2,
        _ => {
            if parts[3].contains('.') {
                2
            } else {
                1
            }
        }
    };
    Some(ScriptTrackDef { index, name, min, max, default, precision })
}

/// 単一ファイルを検査して [`PluginDescriptor`] を返します。
pub fn inspect_plugin_file(path: &Path) -> Option<PluginDescriptor> {
    let ext = path.extension()?.to_str()?;
    let kind = PluginKind::from_extension(ext)?;
    let file_name = path.file_name()?.to_string_lossy().into_owned();
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let bytes = fs::read(path).ok()?;

    if kind.is_script() {
        let text = decode_auto(&bytes);
        let entries = parse_script_entries(&stem, &text);
        let has_preview_builtin = entries.iter().any(|e| {
            matches!(
                e.section_name.as_str(),
                "揺れ" | "振動" | "点滅" | "回転" | "拡大縮小"
            )
        });
        let status = if has_preview_builtin {
            SupportStatus::Supported
        } else {
            SupportStatus::PartialScript
        };
        Some(PluginDescriptor {
            file_name,
            path: path.to_path_buf(),
            kind,
            arch: BinaryArch::Script,
            status,
            entries,
        })
    } else {
        let arch = detect_pe_arch(&bytes);
        let status = match arch {
            BinaryArch::X86_32 => SupportStatus::RequiresBridge32,
            BinaryArch::X86_64 => SupportStatus::PendingNativeHost,
            _ => SupportStatus::RequiresBridge32,
        };
        Some(PluginDescriptor {
            file_name,
            path: path.to_path_buf(),
            kind,
            arch,
            status,
            entries: Vec::new(),
        })
    }
}

/// プラグインレジストリ。指定ディレクトリ内のプラグイン・スクリプトを管理します。
#[derive(Debug, Default, Clone)]
pub struct PluginRegistry {
    pub plugin_dir: PathBuf,
    pub plugins: Vec<PluginDescriptor>,
}

impl PluginRegistry {
    pub fn new(plugin_dir: impl Into<PathBuf>) -> Self {
        let mut reg = Self { plugin_dir: plugin_dir.into(), plugins: Vec::new() };
        reg.reload();
        reg
    }

    /// ディレクトリを再スキャンします。
    pub fn reload(&mut self) {
        self.plugins.clear();
        let Ok(read_dir) = fs::read_dir(&self.plugin_dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = read_dir.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            if p.is_file() {
                if let Some(desc) = inspect_plugin_file(&p) {
                    self.plugins.push(desc);
                }
            }
        }
    }

    /// 外部のプラグイン／スクリプトファイルを `plugin_dir` にコピーしてインストールします。
    pub fn install_file(&mut self, src: &Path) -> std::io::Result<String> {
        fs::create_dir_all(&self.plugin_dir)?;
        let file_name = src
            .file_name()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "ファイル名が不正です"))?;
        let dst = self.plugin_dir.join(file_name);
        if src != dst {
            fs::copy(src, &dst)?;
        }
        self.reload();
        Ok(file_name.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_multi_anm_script() {
        let src = "\
@揺れ
--track0:X振幅,0,500,30,0.1
--track1:Y振幅,0,500,20,0.1
--track2:周期,0.01,10,0.5,0.01
--check0:ランダム,0

@点滅
--track0:速度,0.1,60,4.0,0.1
";
        let entries = parse_script_entries("sample_effects", src);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].display_name, "揺れ@sample_effects");
        assert_eq!(entries[0].tracks.len(), 3);
        assert_eq!(entries[0].tracks[0].name, "X振幅");
        assert_eq!(entries[0].tracks[0].default, 30.0);
        assert_eq!(entries[0].check_name, Some(("ランダム".into(), false)));

        let eff = entries[0].to_effect(PluginKind::AnmScript);
        assert_eq!(eff.name, "アニメーション効果");
        assert_eq!(eff.text("name"), Some("揺れ@sample_effects"));
        assert_eq!(eff.eval("X振幅", 0, 0.0), 30.0);
    }

    #[test]
    fn detect_pe_32_and_64() {
        let mut fake32 = vec![0u8; 0x80];
        fake32[0..2].copy_from_slice(b"MZ");
        fake32[0x3C] = 0x40;
        fake32[0x40..0x44].copy_from_slice(b"PE\0\0");
        fake32[0x44..0x46].copy_from_slice(&0x014cu16.to_le_bytes());
        assert_eq!(detect_pe_arch(&fake32), BinaryArch::X86_32);

        let mut fake64 = fake32.clone();
        fake64[0x44..0x46].copy_from_slice(&0x8664u16.to_le_bytes());
        assert_eq!(detect_pe_arch(&fake64), BinaryArch::X86_64);
    }
}
