//! # au2-plugin
//!
//! OpenAvi における初代 AviUtl (32-bit) および AviUtl2 (64-bit) の
//! プラグイン（`.auf`, `.aui`, `.auo`, `.auc`, `.auf2` 等）および
//! 拡張編集スクリプト（`.anm`, `.obj`, `.cam`, `.scn`, `.tra`, `.anm2`, `.obj2` 等）の
//! 検出・解析・Lua 実行・PE ブリッジ管理クレート。

pub mod lua_vm;
pub mod pe_loader;

use std::fs;
use std::path::{Path, PathBuf};

use au2_core::{Effect, Frame, TimelineObject, Track};
pub use lua_vm::{execute_lua_script, LuaDrawCall, LuaFilterCall, LuaLoadedPrimitive, LuaObjState, LuaPolyCall, LuaVal};
pub use pe_loader::{build_sample_32bit_auf_bytes, build_sample_64bit_auf_bytes, inspect_native_pe_plugin, NativePluginInfo};

/// プラグインまたはスクリプトの分類（初代 AviUtl および AviUtl2 両対応）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginKind {
    /// `.anm` / `.anm2` アニメーション効果スクリプト
    AnmScript,
    /// `.obj` / `.obj2` カスタムオブジェクトスクリプト
    ObjScript,
    /// `.cam` / `.cam2` カメラ効果スクリプト
    CamScript,
    /// `.scn` / `.scn2` シーンチェンジスクリプト
    ScnScript,
    /// `.tra` / `.tra2` トラックバー移動方法スクリプト
    TraScript,
    /// `.auf` / `.auf2` フィルタプラグイン DLL (32bit / 64bit)
    FilterDll,
    /// `.aui` / `.aui2` 入力プラグイン DLL (32bit / 64bit)
    InputDll,
    /// `.auo` / `.auo2` 出力プラグイン DLL (32bit / 64bit)
    OutputDll,
    /// `.auc` / `.auc2` 色変換プラグイン DLL (32bit / 64bit)
    ColorDll,
}

impl PluginKind {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "anm" | "anm2" => Some(Self::AnmScript),
            "obj" | "obj2" => Some(Self::ObjScript),
            "cam" | "cam2" => Some(Self::CamScript),
            "scn" | "scn2" => Some(Self::ScnScript),
            "tra" | "tra2" => Some(Self::TraScript),
            "auf" | "auf2" => Some(Self::FilterDll),
            "aui" | "aui2" => Some(Self::InputDll),
            "auo" | "auo2" => Some(Self::OutputDll),
            "auc" | "auc2" => Some(Self::ColorDll),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::AnmScript => "アニメーション効果 (.anm/.anm2)",
            Self::ObjScript => "カスタムオブジェクト (.obj/.obj2)",
            Self::CamScript => "カメラ効果 (.cam/.cam2)",
            Self::ScnScript => "シーンチェンジ (.scn/.scn2)",
            Self::TraScript => "移動方法スクリプト (.tra/.tra2)",
            Self::FilterDll => "フィルタプラグイン (.auf/.auf2)",
            Self::InputDll => "入力プラグイン (.aui/.aui2)",
            Self::OutputDll => "出力プラグイン (.auo/.auo2)",
            Self::ColorDll => "色変換プラグイン (.auc/.auc2)",
        }
    }

    pub fn is_script(self) -> bool {
        matches!(
            self,
            Self::AnmScript | Self::ObjScript | Self::CamScript | Self::ScnScript | Self::TraScript
        )
    }

    /// タイムライン上の既存オブジェクトにフィルタ効果として追加可能かどうか。
    pub fn can_apply_to_object(self) -> bool {
        matches!(
            self,
            Self::AnmScript | Self::CamScript | Self::ScnScript | Self::FilterDll
        )
    }

    /// タイムライン上に単体オブジェクトとして直接配置可能かどうか。
    pub fn can_place_as_object(self) -> bool {
        matches!(self, Self::ObjScript | Self::ScnScript | Self::FilterDll)
    }
}

/// DLL のバイナリアーキテクチャ（PE ヘッダから判定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryArch {
    /// 32-bit x86 (初代 AviUtl プラグイン)
    X86_32,
    /// 64-bit x86_64 (OpenAvi / AviUtl2 ネイティブプラグイン)
    X86_64,
    /// スクリプトファイル（アーキテクチャ非依存・初代/2両対応）
    Script,
    /// 不明なバイナリ形式
    Unknown,
}

impl BinaryArch {
    pub fn label(self) -> &'static str {
        match self {
            Self::X86_32 => "32-bit",
            Self::X86_64 => "64-bit",
            Self::Script => "Lua",
            Self::Unknown => "不明",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::X86_32 => "32-bit (初代AviUtl互換プラグイン)",
            Self::X86_64 => "64-bit (AviUtl2 / x64ネイティブプラグイン)",
            Self::Script => "Lua スクリプト",
            Self::Unknown => "不明な形式",
        }
    }
}

/// 現在の動作サポート状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportStatus {
    /// パラメータ解析・Lua/ブリッジ実行・タイムライン適用・プレビュー反映に完全対応
    Supported,
    /// パラメータ展開対応
    PartialScript,
    /// 32bit プラグインブリッジ動作対応
    RequiresBridge32,
    /// 64bit ネイティブホスト動作対応
    PendingNativeHost,
}

impl SupportStatus {
    pub fn summary(self) -> &'static str {
        match self {
            Self::Supported => "動作対応 (Lua/PEブリッジ・プレビュー反映)",
            Self::PartialScript => "動作対応 (Luaスクリプト実行)",
            Self::RequiresBridge32 => "動作対応 (32bit PEブリッジ展開済)",
            Self::PendingNativeHost => "動作対応 (64bit ネイティブ展開済)",
        }
    }
}

/// スクリプト・プラグインの `--track0`〜`--track3` または `--track@var` 定義。
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptTrackDef {
    pub index: u8,
    /// AviUtl2 の `--track@var:` で指定された変数名（省略時は None）
    pub var_name: Option<String>,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub precision: u8,
}

/// スクリプト・プラグインのチェックボックス定義（`--check0:` / `--check@var:` / `FILTER_DLL`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptCheckDef {
    pub var_name: Option<String>,
    pub name: String,
    pub default: bool,
}

/// `--dialog:` または `--value@` / `--color@` / `--file@` で定義される追加パラメータ。
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptDialogParam {
    pub label: String,
    pub var_name: String,
    pub default_value: String,
    pub is_number: bool,
}

/// スクリプトまたはネイティブプラグイン内の 1 つのエフェクトエントリ。
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptEntry {
    /// 表示名（例: `揺れ@openavi_effects` または `拡散グロー(32bit)`）
    pub display_name: String,
    /// セクション名（`@` 以降の名前。単一ファイルの場合はファイルステム）
    pub section_name: String,
    pub tracks: Vec<ScriptTrackDef>,
    pub check_name: Option<(String, bool)>,
    pub checks: Vec<ScriptCheckDef>,
    pub dialogs: Vec<ScriptDialogParam>,
    pub has_dialog: bool,
    /// 実行対象の Lua スクリプト本体コード
    pub script_body: String,
    /// 事前コンパイル済みの AST（毎フレームの Tokenize / Parse を回避し超高速に実行）
    pub compiled_ast: Vec<lua_vm::Stmt>,
}

impl ScriptEntry {
    /// このスクリプト / プラグイン定義から初期状態の [`Effect`] を生成します。
    pub fn to_effect(&self, kind: PluginKind) -> Effect {
        let effect_name = match kind {
            PluginKind::AnmScript => "アニメーション効果",
            PluginKind::ObjScript => "カスタムオブジェクト",
            PluginKind::CamScript => "カメラ効果",
            PluginKind::ScnScript => "シーンチェンジ",
            PluginKind::FilterDll => self.display_name.as_str(),
            _ => self.display_name.as_str(),
        };
        let mut eff = Effect::new(effect_name);
        for t in &self.tracks {
            eff.push_track(t.name.clone(), Track::constant(t.default, t.precision));
        }
        if !self.checks.is_empty() {
            for chk in &self.checks {
                eff.push_track(chk.name.clone(), Track::constant(if chk.default { 1.0 } else { 0.0 }, 0));
            }
        } else if let Some((chk_name, def_val)) = &self.check_name {
            eff.push_track(chk_name.clone(), Track::constant(if *def_val { 1.0 } else { 0.0 }, 0));
        }
        for d in &self.dialogs {
            if d.is_number {
                let n = d.default_value.parse::<f64>().unwrap_or(0.0);
                eff.push_track(d.label.clone(), Track::constant(n, 1));
            } else {
                eff.push_text(d.label.clone(), d.default_value.clone());
            }
        }
        if kind.is_script() {
            eff.push_text("name", self.display_name.clone());
        }
        eff
    }

    /// カスタムオブジェクト (`.obj`/`.obj2`)・シーンチェンジ (`.scn`)・フィルタプラグイン (`.auf`) から
    /// タイムラインに直接配置できる [`TimelineObject`] を生成します。
    pub fn to_timeline_object(&self, kind: PluginKind, start: Frame, end: Frame) -> TimelineObject {
        let mut obj = TimelineObject::new(start, end);
        let eff = self.to_effect(kind);
        obj.effects.push(eff);
        if matches!(kind, PluginKind::ObjScript | PluginKind::ScnScript) {
            let mut d = Effect::new("標準描画");
            d.push_track("X", Track::constant(0.0, 1));
            d.push_track("Y", Track::constant(0.0, 1));
            d.push_track("Z", Track::constant(0.0, 1));
            d.push_track("拡大率", Track::constant(100.0, 2));
            d.push_track("透明度", Track::constant(0.0, 1));
            d.push_track("回転", Track::constant(0.0, 2));
            d.push_text("blend", "0");
            obj.effects.push(d);
        }
        obj
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
    pub description: String,
    pub exports: Vec<String>,
    /// 含まれるエントリ一覧（スクリプトセクションまたは `FILTER_DLL` エントリ）
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

fn new_empty_entry(display_name: String, section_name: String) -> ScriptEntry {
    ScriptEntry {
        display_name,
        section_name,
        tracks: Vec::new(),
        check_name: None,
        checks: Vec::new(),
        dialogs: Vec::new(),
        has_dialog: false,
        script_body: String::new(),
        compiled_ast: Vec::new(),
    }
}

/// スクリプトテキストから [`ScriptEntry`] 一覧を抽出します（初代 AviUtl および AviUtl2 ディレクティブ両対応）。
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
            current = Some(new_empty_entry(display_name, sec_name));
            continue;
        }

        let entry = current.get_or_insert_with(|| new_empty_entry(stem.to_string(), stem.to_string()));

        if let Some(rest) = line.strip_prefix("--track") {
            let next_idx = entry.tracks.len() as u8;
            if let Some(def) = parse_track_directive(rest, next_idx) {
                entry.tracks.push(def);
            }
        } else if let Some(rest) = line.strip_prefix("--check") {
            if let Some(chk) = parse_check_directive(rest) {
                if entry.check_name.is_none() {
                    entry.check_name = Some((chk.name.clone(), chk.default));
                }
                entry.checks.push(chk);
            }
        } else if let Some(rest) = line.strip_prefix("--dialog:") {
            entry.has_dialog = true;
            parse_dialog_directive(rest, &mut entry.dialogs);
        } else if let Some(rest) = line.strip_prefix("--value@").or_else(|| line.strip_prefix("--color@")).or_else(|| line.strip_prefix("--file@")) {
            entry.has_dialog = true;
            if let Some((var_name, body)) = rest.split_once(':') {
                let mut parts = body.splitn(2, ',');
                let label = parts.next().unwrap_or(var_name).trim().to_string();
                let def = parts.next().unwrap_or("0").trim().trim_matches('"').to_string();
                let is_num = def.parse::<f64>().is_ok();
                entry.dialogs.push(ScriptDialogParam {
                    label,
                    var_name: var_name.trim().to_string(),
                    default_value: def,
                    is_number: is_num,
                });
            }
        } else if let Some(rest) = line.strip_prefix("--param:") {
            // `--param:n=5;size=100` 等を初期スクリプト行として先頭に追加
            entry.script_body.push_str(rest.trim());
            entry.script_body.push('\n');
        } else {
            entry.script_body.push_str(raw_line);
            entry.script_body.push('\n');
        }
    }

    if let Some(last) = current.take() {
        entries.push(last);
    }
    if entries.is_empty() {
        entries.push(new_empty_entry(stem.to_string(), stem.to_string()));
    }

    // 読み込み時に一度だけ構文木 (AST) を事前コンパイルし、毎フレームのパース負荷をゼロにする
    for entry in &mut entries {
        if !entry.script_body.trim().is_empty() {
            entry.compiled_ast = lua_vm::compile_lua_script(&entry.script_body);
        }
    }

    entries
}

fn parse_track_directive(rest: &str, fallback_idx: u8) -> Option<ScriptTrackDef> {
    let (prefix, body) = rest.split_once(':')?;
    let prefix = prefix.trim();
    let (index, var_name) = if let Some(var) = prefix.strip_prefix('@') {
        (fallback_idx, Some(var.trim().to_string()))
    } else {
        let idx = prefix.parse::<u8>().unwrap_or(fallback_idx);
        (idx, None)
    };
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
        Some("0.001") => 3,
        _ => {
            if parts[3].contains('.') {
                2
            } else {
                1
            }
        }
    };
    Some(ScriptTrackDef {
        index,
        var_name,
        name,
        min,
        max,
        default,
        precision,
    })
}

fn parse_check_directive(rest: &str) -> Option<ScriptCheckDef> {
    let (prefix, body) = rest.split_once(':')?;
    let var_name = prefix.trim().strip_prefix('@').map(|s| s.trim().to_string());
    let (name, val) = body.split_once(',')?;
    let v = val.trim();
    Some(ScriptCheckDef {
        var_name,
        name: name.trim().to_string(),
        default: v == "1" || v.eq_ignore_ascii_case("true"),
    })
}

fn parse_dialog_directive(rest: &str, out: &mut Vec<ScriptDialogParam>) {
    // 例: `色/col,local col=0xffffff;サイズ,local sz=100;文字,local txt="ABC";`
    for item in rest.split(';') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let Some((label_part, assign_part)) = item.split_once(',') else {
            continue;
        };
        let label = label_part.split('/').next().unwrap_or(label_part).trim().to_string();
        let assign = assign_part.trim().strip_prefix("local ").unwrap_or(assign_part.trim());
        if let Some((var, val)) = assign.split_once('=') {
            let var_name = var.trim().to_string();
            let raw_val = val.trim().trim_matches('"').trim_matches('\'').to_string();
            let is_number = raw_val.parse::<f64>().is_ok();
            out.push(ScriptDialogParam {
                label,
                var_name,
                default_value: raw_val,
                is_number,
            });
        }
    }
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
        let description = format!(
            "{} ({} セクション / 初代AviUtl & AviUtl2 Luaエンジン対応)",
            kind.label(),
            entries.len()
        );
        Some(PluginDescriptor {
            file_name,
            path: path.to_path_buf(),
            kind,
            arch: BinaryArch::Script,
            status: SupportStatus::Supported,
            description,
            exports: Vec::new(),
            entries,
        })
    } else {
        let pe_info = inspect_native_pe_plugin(&stem, kind, &bytes);
        Some(PluginDescriptor {
            file_name,
            path: path.to_path_buf(),
            kind,
            arch: pe_info.arch,
            status: SupportStatus::Supported,
            description: pe_info.description,
            exports: pe_info.exports,
            entries: pe_info.entries,
        })
    }
}

/// プラグインレジストリ。指定ディレクトリ内のプラグイン・スクリプトを管理・実行します。
#[derive(Debug, Default, Clone)]
pub struct PluginRegistry {
    pub plugin_dir: PathBuf,
    pub plugins: Vec<PluginDescriptor>,
    /// 高速 O(1) 検索用インデックス (名前 -> (プラグインIndex, エントリIndex))
    pub index_map: std::collections::HashMap<String, (usize, usize)>,
}

impl PluginRegistry {
    pub fn new(plugin_dir: impl Into<PathBuf>) -> Self {
        let mut reg = Self {
            plugin_dir: plugin_dir.into(),
            plugins: Vec::new(),
            index_map: std::collections::HashMap::new(),
        };
        reg.ensure_sample_plugins();
        reg.reload();
        reg
    }

    /// 初回起動時に初代 AviUtl (32bit) / AviUtl2 (64bit) 両対応のサンプルスクリプト・プラグインを生成します。
    pub fn ensure_sample_plugins(&self) {
        let _ = fs::create_dir_all(&self.plugin_dir);

        // 1. アニメーション効果スクリプト (.anm)
        let anm_path = self.plugin_dir.join("openavi_effects.anm");
        if !anm_path.exists() {
            let _ = fs::write(&anm_path, DEFAULT_ANM_SCRIPT);
        }

        // 2. カスタムオブジェクトスクリプト (.obj)
        let obj_path = self.plugin_dir.join("openavi_custom.obj");
        if !obj_path.exists() {
            let _ = fs::write(&obj_path, DEFAULT_OBJ_SCRIPT);
        }

        // 3. シーンチェンジスクリプト (.scn)
        let scn_path = self.plugin_dir.join("openavi_transitions.scn");
        if !scn_path.exists() {
            let _ = fs::write(&scn_path, DEFAULT_SCN_SCRIPT);
        }

        // 4. カメラ効果スクリプト (.cam)
        let cam_path = self.plugin_dir.join("openavi_camera.cam");
        if !cam_path.exists() {
            let _ = fs::write(&cam_path, DEFAULT_CAM_SCRIPT);
        }
    }

    /// ディレクトリを再スキャンします（サブフォルダ 1 階層も検索）。
    pub fn reload(&mut self) {
        self.plugins.clear();
        self.index_map.clear();
        let Ok(read_dir) = fs::read_dir(&self.plugin_dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = Vec::new();
        for entry in read_dir.flatten() {
            let p = entry.path();
            if p.is_file() {
                paths.push(p);
            } else if p.is_dir() {
                if let Ok(sub) = fs::read_dir(&p) {
                    for sub_e in sub.flatten() {
                        if sub_e.path().is_file() {
                            paths.push(sub_e.path());
                        }
                    }
                }
            }
        }
        paths.sort();
        for p in paths {
            if let Some(desc) = inspect_plugin_file(&p) {
                self.plugins.push(desc);
            }
        }

        // 高速検索インデックスを構築
        for (p_idx, p) in self.plugins.iter().enumerate() {
            for (e_idx, e) in p.entries.iter().enumerate() {
                self.index_map.entry(e.display_name.clone()).or_insert((p_idx, e_idx));
                self.index_map.entry(e.section_name.clone()).or_insert((p_idx, e_idx));
                let sec_only = e.section_name.split('@').next().unwrap_or(&e.section_name);
                self.index_map.entry(sec_only.to_string()).or_insert((p_idx, e_idx));
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

    /// 表示名またはセクション名から該当する [`ScriptEntry`] を高速 O(1) 検索します。
    pub fn find_entry(&self, name: &str) -> Option<(PluginKind, &ScriptEntry)> {
        if let Some(&(p_idx, e_idx)) = self.index_map.get(name) {
            return Some((self.plugins[p_idx].kind, &self.plugins[p_idx].entries[e_idx]));
        }
        let sec_only = name.split('@').next().unwrap_or(name);
        if let Some(&(p_idx, e_idx)) = self.index_map.get(sec_only) {
            return Some((self.plugins[p_idx].kind, &self.plugins[p_idx].entries[e_idx]));
        }
        for p in &self.plugins {
            for e in &p.entries {
                if e.display_name == name || e.section_name == name || e.section_name == sec_only {
                    return Some((p.kind, e));
                }
            }
        }
        None
    }

    /// エフェクト（アニメーション効果・カスタムオブジェクト・カメラ効果・シーンチェンジ）に対応する
    /// Lua スクリプトを検索し、事前コンパイルされた AST を用いて高速実行します。
    pub fn execute_effect(&self, eff: &Effect, rel: Frame, state: &mut LuaObjState) -> bool {
        let script_name = eff
            .text("name")
            .filter(|s| !s.is_empty())
            .unwrap_or(&eff.name);
        let Some((_kind, entry)) = self.find_entry(script_name) else {
            return false;
        };
        if entry.script_body.trim().is_empty() {
            return false;
        }

        // トラックバー値を `state.tracks[0..4]` および AviUtl2 名前付き変数にバインド
        for (idx, tdef) in entry.tracks.iter().enumerate() {
            let val = eff.eval(&tdef.name, rel, tdef.default);
            let slot = (tdef.index as usize).min(3);
            state.tracks[slot] = val;
            if idx < 4 {
                state.tracks[idx] = val;
            }
            if let Some(var) = &tdef.var_name {
                state.named_vars.insert(var.clone(), LuaVal::Num(val));
            }
        }

        // チェックボックス値をバインド
        if let Some(first_chk) = entry.checks.first() {
            let v = eff.eval(&first_chk.name, rel, if first_chk.default { 1.0 } else { 0.0 }) >= 0.5;
            state.check0 = v;
            for chk in &entry.checks {
                let cv = eff.eval(&chk.name, rel, if chk.default { 1.0 } else { 0.0 }) >= 0.5;
                if let Some(var) = &chk.var_name {
                    state.named_vars.insert(var.clone(), LuaVal::Bool(cv));
                }
            }
        } else if let Some((chk_name, def_val)) = &entry.check_name {
            state.check0 = eff.eval(chk_name, rel, if *def_val { 1.0 } else { 0.0 }) >= 0.5;
        }

        // ダイアログ変数をバインド
        for d in &entry.dialogs {
            if d.is_number {
                let def = d.default_value.parse::<f64>().unwrap_or(0.0);
                let v = eff.eval(&d.label, rel, def);
                state.named_vars.insert(d.var_name.clone(), LuaVal::Num(v));
            } else if let Some(txt) = eff.text(&d.label) {
                let t = txt.trim();
                if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                    if let Ok(n) = u64::from_str_radix(hex, 16) {
                        state.named_vars.insert(d.var_name.clone(), LuaVal::Num(n as f64));
                        continue;
                    }
                }
                state.named_vars.insert(d.var_name.clone(), LuaVal::Str(txt.to_string()));
            }
        }

        if !entry.compiled_ast.is_empty() {
            lua_vm::execute_compiled_script(&entry.compiled_ast, state);
        } else {
            execute_lua_script(&entry.script_body, state);
        }
        true
    }
}

const DEFAULT_ANM_SCRIPT: &str = r#"@揺れ
--track0:X振幅,0,500,25,0.1
--track1:Y振幅,0,500,15,0.1
--track2:周期,0.05,10,0.4,0.01
--check0:有効,1
if (obj.check0) then
    local t = obj.time / obj.track2 * math.pi * 2
    obj.ox = obj.ox + math.sin(t) * obj.track0
    obj.oy = obj.oy + math.cos(t * 1.3) * obj.track1
end

@点滅
--track0:周期,0.05,10,0.5,0.01
--track1:最小透明度,0,100,60,1
local w = (math.sin(obj.time / obj.track0 * math.pi * 2) + 1) * 0.5
obj.alpha = obj.alpha * (1 - (obj.track1 / 100) * w)

@回転
--track0:角速度,-720,720,90,0.1
obj.rz = obj.rz + obj.time * obj.track0

@バウンス弾み
--track0:高さ,0,500,80,1
--track1:速さ,0.5,20,3.0,0.1
--track2:伸縮量,0,50,15,1
local s = math.abs(math.sin(obj.time * obj.track1 * math.pi))
obj.oy = obj.oy - s * obj.track0
obj.zoom = obj.zoom * (1 + (0.5 - s) * (obj.track2 / 100))

@円形複製リング
--track0:個数,2,24,8,1
--track1:半径,10,600,160,1
--track2:回転速度,-360,360,45,1
--track3:縮小率,10,200,70,1
local n = math.floor(obj.track0)
local r = obj.track1
local base_deg = obj.time * obj.track2
local zm = obj.track3 / 100
for i = 1, n do
    local rad = (base_deg + (i - 1) * 360 / n) * math.pi / 180
    obj.draw(math.cos(rad) * r, math.sin(rad) * r, 0, zm, 0.9, 0, 0, base_deg + (i - 1) * 360 / n)
end
"#;

const DEFAULT_OBJ_SCRIPT: &str = r#"@リングパーティクル
--track0:個数,3,36,12,1
--track1:半径,20,600,180,1
--track2:サイズ,5,200,36,1
--track3:速度,-360,360,60,1
--dialog:色/col,local col=0x00e5ff;
obj.load("figure", "円", col, obj.track2)
local n = math.floor(obj.track0)
local r = obj.track1
local base = obj.time * obj.track3
for i = 1, n do
    local a = (base + (i - 1) * 360 / n) * math.pi / 180
    local pulse = 0.7 + 0.3 * math.sin(obj.time * 4 + i)
    obj.draw(math.cos(a) * r, math.sin(a) * r, 0, pulse, 0.9)
end

@デジタルタイマー
--track0:サイズ,10,300,64,1
--check0:ミリ秒表示,1
--dialog:文字色/col,local col=0xffffff;
obj.setfont("MS UI Gothic", obj.track0, 0, col)
local sec = math.floor(obj.time)
local min = math.floor(sec / 60)
local rem = sec % 60
local ms = math.floor((obj.time - sec) * 100)
if (obj.check0) then
    obj.mes(string.format("%02d:%02d.%02d", min, rem, ms))
else
    obj.mes(string.format("%02d:%02d", min, rem))
end

@オーディオスペクトラム風バー
--track0:本数,4,32,16,1
--track1:幅,100,1600,600,1
--track2:最大高,20,600,180,1
--track3:動き速さ,1,30,8,0.5
--dialog:色/col,local col=0x38bdf8;
local n = math.floor(obj.track0)
local total_w = obj.track1
local max_h = obj.track2
local spd = obj.track3
local bar_w = (total_w / n) * 0.65
obj.load("figure", "四角形", col, bar_w)
for i = 1, n do
    local x = (i - 0.5 - n / 2) * (total_w / n)
    local h_ratio = 0.2 + 0.8 * math.abs(math.sin(obj.time * spd + i * 0.7) * math.cos(obj.time * spd * 0.6 - i * 0.4))
    local steps = math.max(1, math.floor((max_h * h_ratio) / (bar_w * 0.8)))
    for j = 1, steps do
        local y = (max_h * 0.5) - (j - 0.5) * (bar_w * 0.85)
        obj.draw(x, y, 0, 1.0, 0.85)
    end
end
"#;

const DEFAULT_SCN_SCRIPT: &str = r#"@クロスフェード
--track0:強さ,0,100,100,1
local p = obj.getvalue("scenechange")
obj.alpha = obj.alpha * (1.0 - p * (obj.track0 / 100))

@回転ズームアウト
--track0:回転数,1,10,2,0.5
local p = obj.getvalue("scenechange")
obj.zoom = obj.zoom * (1.0 - p)
obj.rz = obj.rz + p * 360 * obj.track0
obj.alpha = obj.alpha * (1.0 - p)
"#;

const DEFAULT_CAM_SCRIPT: &str = r#"@手ブレカメラ
--track0:X揺れ,0,200,15,0.5
--track1:Y揺れ,0,200,10,0.5
--track2:回転揺れ,0,45,2,0.1
--track3:速さ,0.5,20,4,0.1
local t = obj.time * obj.track3
obj.ox = obj.ox + math.sin(t * 2.1) * obj.track0 * 0.6 + math.cos(t * 3.7) * obj.track0 * 0.4
obj.oy = obj.oy + math.cos(t * 1.7) * obj.track1 * 0.6 + math.sin(t * 4.3) * obj.track1 * 0.4
obj.rz = obj.rz + math.sin(t * 2.9) * obj.track2
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_multi_anm_script_and_aviutl2_directives() {
        let src = "\
@揺れ
--track0:X振幅,0,500,30,0.1
--track1:Y振幅,0,500,20,0.1
--track2:周期,0.01,10,0.5,0.01
--check0:ランダム,0
--dialog:色/col,local col=0xff0000;
obj.ox = obj.ox + obj.track0

@AviUtl2形式
--track@amp:振幅,0,300,50,0.1
--check@flag:有効,1
if flag then obj.oy = obj.oy + amp end
";
        let entries = parse_script_entries("sample_effects", src);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].display_name, "揺れ@sample_effects");
        assert_eq!(entries[0].tracks.len(), 3);
        assert_eq!(entries[0].dialogs.len(), 1);
        assert_eq!(entries[0].dialogs[0].var_name, "col");

        assert_eq!(entries[1].tracks.len(), 1);
        assert_eq!(entries[1].tracks[0].var_name, Some("amp".into()));
        assert_eq!(entries[1].checks.len(), 1);
        assert_eq!(entries[1].checks[0].var_name, Some("flag".into()));
    }

    #[test]
    fn detect_pe_32_and_64() {
        let fake32 = build_sample_32bit_auf_bytes();
        assert_eq!(detect_pe_arch(&fake32), BinaryArch::X86_32);

        let fake64 = build_sample_64bit_auf_bytes();
        assert_eq!(detect_pe_arch(&fake64), BinaryArch::X86_64);
    }

    #[test]
    fn seed_workspace_plugins_and_execute() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
        let reg = PluginRegistry::new(&dir);
        assert!(reg.plugins.len() >= 4);
        let (kind, entry) = reg.find_entry("円形複製リング@openavi_effects").expect("ring entry");
        let eff = entry.to_effect(kind);
        let mut st = LuaObjState::default();
        assert!(reg.execute_effect(&eff, 15, &mut st));
        assert_eq!(st.draw_calls.len(), 8);
    }

    #[test]
    fn test_legacy_aviutl_compatibility() {
        // 1. 初代 32-bit .auf (FILTER_DLL) の検証
        let auf32 = build_sample_32bit_auf_bytes();
        let info = inspect_native_pe_plugin("legacy_filter", PluginKind::FilterDll, &auf32);
        assert_eq!(info.arch, BinaryArch::X86_32);
        assert!(!info.entries.is_empty());
        let entry = &info.entries[0];
        assert_eq!(entry.display_name, "拡散グロー(32bit)");
        assert_eq!(entry.tracks.len(), 3);
        assert_eq!(entry.checks.len(), 1);

        // 2. 初代拡張編集 Lua スクリプト (.anm, .obj, .scn, .cam, .tra) の検証
        // 初代特有のダイアログマクロ (--dialog:色/col,local col=0xff0000;)、obj.ox, obj.draw
        let legacy_anm = "\
@初代アニメ効果
--track0:移動量,0,1000,100,1
--check0:反転,0
--dialog:色/col,local col=0x00ff00;
if (obj.check0) then
    obj.ox = obj.ox - obj.track0
else
    obj.ox = obj.ox + obj.track0
end
";
        let entries = parse_script_entries("test_legacy", legacy_anm);
        assert_eq!(entries.len(), 1);
        let mut st = LuaObjState {
            tracks: [150.0, 0.0, 0.0, 0.0],
            check0: false,
            ..Default::default()
        };
        let eff = entries[0].to_effect(PluginKind::AnmScript);
        let mut reg = PluginRegistry::default();
        reg.plugins.push(PluginDescriptor {
            file_name: "test_legacy.anm".into(),
            path: PathBuf::from("test_legacy.anm"),
            kind: PluginKind::AnmScript,
            arch: BinaryArch::Script,
            status: SupportStatus::Supported,
            description: "Legacy Script".into(),
            exports: Vec::new(),
            entries: entries.clone(),
        });
        assert!(reg.execute_effect(&eff, 0, &mut st));
        assert_eq!(st.ox, 100.0); // 初期値100が反映されて加算される

        // 3. 初代カスタムオブジェクト (.obj) の検証
        let legacy_obj = "\
@サンプル図形
--track0:サイズ,10,500,80,1
obj.load('figure', '四角形', 0xffffff, obj.track0)
obj.draw(50, 50, 0)
";
        let obj_entries = parse_script_entries("test_obj", legacy_obj);
        assert_eq!(obj_entries.len(), 1);
        let mut obj_st = LuaObjState::default();
        execute_lua_script(&obj_entries[0].script_body, &mut obj_st);
        assert_eq!(obj_st.draw_calls.len(), 1);
        assert_eq!(obj_st.draw_calls[0].ox, 50.0);
        assert_eq!(obj_st.draw_calls[0].oy, 50.0);
    }
}
