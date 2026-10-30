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

        if let Ok(read_dir) = fs::read_dir(&self.plugin_dir) {
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
        }

        // 初代 AviUtl 拡張編集標準互換スクリプト群をメモリ内ビルトインとして常時登録
        let builtins = [
            ("exedit.anm", PluginKind::AnmScript, BUILTIN_EXEDIT_ANM),
            ("exedit.obj", PluginKind::ObjScript, BUILTIN_EXEDIT_OBJ),
            ("exedit.cam", PluginKind::CamScript, BUILTIN_EXEDIT_CAM),
            ("exedit.scn", PluginKind::ScnScript, BUILTIN_EXEDIT_SCN),
        ];
        for (fname, kind, src) in builtins {
            let stem = fname.split('.').next().unwrap_or(fname);
            let entries = parse_script_entries(stem, src);
            self.plugins.push(PluginDescriptor {
                file_name: fname.to_string(),
                path: PathBuf::from(fname),
                kind,
                arch: BinaryArch::Script,
                status: SupportStatus::Supported,
                description: format!("標準組み込みスクリプト ({} 個)", entries.len()),
                exports: entries.iter().map(|e| e.display_name.clone()).collect(),
                entries,
            });
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

        // 最初に汎用 track0..3 がエフェクトに存在する場合はベース値としてセット
        for i in 0..4 {
            let key = format!("track{}", i);
            if eff.param(&key).is_some() {
                state.tracks[i] = eff.eval(&key, rel, state.tracks[i]);
            }
        }

        // トラックバー値を `state.tracks[0..4]` および AviUtl2 名前付き変数にバインド
        // .exo / .aup では `track0`..`track3` キーで格納される場合と、トラック名（例: `時間`, `振幅`）で格納される場合の両方に対応
        for (idx, tdef) in entry.tracks.iter().enumerate() {
            let slot = (tdef.index as usize).min(3);
            let slot_key = format!("track{}", slot);
            let idx_key = format!("track{}", idx);
            let val = if eff.param(&tdef.name).is_some() {
                eff.eval(&tdef.name, rel, tdef.default)
            } else if eff.param(&slot_key).is_some() {
                eff.eval(&slot_key, rel, tdef.default)
            } else if eff.param(&idx_key).is_some() {
                eff.eval(&idx_key, rel, tdef.default)
            } else {
                tdef.default
            };
            state.tracks[slot] = val;
            if idx < 4 {
                state.tracks[idx] = val;
            }
            if let Some(var) = &tdef.var_name {
                state.named_vars.insert(var.clone(), LuaVal::Num(val));
            }
        }

        // チェックボックス値をバインド (`check0` 等へのフォールバック付き)
        if let Some(first_chk) = entry.checks.first() {
            let def_num = if first_chk.default { 1.0 } else { 0.0 };
            let v = if eff.param(&first_chk.name).is_some() {
                eff.eval(&first_chk.name, rel, def_num) >= 0.5
            } else {
                eff.eval("check0", rel, def_num) >= 0.5
            };
            state.check0 = v;
            for (c_idx, chk) in entry.checks.iter().enumerate() {
                let c_def = if chk.default { 1.0 } else { 0.0 };
                let c_key = format!("check{}", c_idx);
                let cv = if eff.param(&chk.name).is_some() {
                    eff.eval(&chk.name, rel, c_def) >= 0.5
                } else {
                    eff.eval(&c_key, rel, c_def) >= 0.5
                };
                if let Some(var) = &chk.var_name {
                    state.named_vars.insert(var.clone(), LuaVal::Bool(cv));
                }
            }
        } else if let Some((chk_name, def_val)) = &entry.check_name {
            let def_num = if *def_val { 1.0 } else { 0.0 };
            state.check0 = if eff.param(chk_name).is_some() {
                eff.eval(chk_name, rel, def_num) >= 0.5
            } else {
                eff.eval("check0", rel, def_num) >= 0.5
            };
        } else if eff.param("check0").is_some() {
            state.check0 = eff.eval("check0", rel, 0.0) >= 0.5;
        }

        // ダイアログ変数の既定値およびラベル指定値をバインド
        for d in &entry.dialogs {
            let def_trimmed = d.default_value.trim().trim_matches('"').trim_matches('\'');
            if d.is_number {
                let def = if let Some(hex) = def_trimmed.strip_prefix("0x").or_else(|| def_trimmed.strip_prefix("0X")) {
                    u64::from_str_radix(hex, 16).map(|n| n as f64).unwrap_or(0.0)
                } else {
                    def_trimmed.parse::<f64>().unwrap_or(0.0)
                };
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
            } else {
                state.named_vars.insert(d.var_name.clone(), LuaVal::Str(def_trimmed.to_string()));
            }
        }

        // .aup / .exo の `param=...`（例: `num=8;dir=0;rot=0;col=0xffffff;`）が存在する場合は評価して変数を上書き
        if let Some(param_str) = eff.text("param") {
            if !param_str.trim().is_empty() {
                lua_vm::execute_param_assignments(param_str, state);
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

/// 拡張編集互換の標準アニメーション効果スクリプト（クリーンルーム実装）
const BUILTIN_EXEDIT_ANM: &str = r#"@震える
--track0:振幅,0,500,5
--track1:角度,0,180,10
--track2:間隔,0,100,0
local interval = obj.track2
if (interval <= 0) then
    obj.ox = obj.ox + obj.rand(-obj.track0, obj.track0, 0)
    obj.oy = obj.oy + obj.rand(-obj.track0, obj.track0, 1)
    obj.rz = obj.rz + obj.rand(-obj.track1, obj.track1, 2)
else
    local step = math.max(1, math.floor(interval))
    local seed_frame = math.floor(obj.frame / step)
    obj.ox = obj.ox + obj.rand(-1000, 1000, 0, seed_frame) * 0.001 * obj.track0
    obj.oy = obj.oy + obj.rand(-1000, 1000, 1, seed_frame) * 0.001 * obj.track0
    obj.rz = obj.rz + obj.rand(-1000, 1000, 2, seed_frame) * 0.001 * obj.track1
end

@振り子
--track0:速さ,0,10,2,0.01
--track1:角度,0,180,30
--track2:ずらし,0,100,0
obj.rz = obj.rz + math.sin((obj.time * obj.track0 + obj.index * obj.track2 * 0.02) * math.pi) * obj.track1

@弾む
--track0:速さ,0.1,100,20
--track1:高さ,0,1000,150
--track2:角度,0,360,0
--track3:ずらし,0,100,0
local spd = math.max(0.1, obj.track0)
local period = math.max(0.01, math.floor((10 / spd) * obj.framerate) / obj.framerate)
local phase = ((obj.time / period + obj.index * obj.track3 * 0.01) % 1 - 0.5) * 2
local bounce = (1 - phase * phase) * obj.track1
local rad = obj.track2 * math.pi / 180
obj.ox = obj.ox + math.sin(rad) * bounce
obj.oy = obj.oy - math.cos(rad) * bounce

@座標の拡大縮小(個別オブジェクト)
--track0:拡大率,0,1000,100
--track1:中心X,-1000,1000,0
--track2:中心Y,-1000,1000,0
obj.ox = obj.track1 + (obj.ox - obj.track1) * obj.track0 * 0.01
obj.oy = obj.track2 + (obj.oy - obj.track2) * obj.track0 * 0.01

@画面外から登場
--track0:時間,-5,5,0.5,0.01
--track1:角度,-360,360,90
--track2:数,1,16,1,1
--check0:ランダム方向,0
if (obj.track0 ~= 0) then
    local ratio = 0
    if (obj.track0 < 0) then
        ratio = (obj.track0 - obj.time + obj.totaltime) / obj.track0
    else
        ratio = (obj.track0 - obj.time) / obj.track0
    end
    if (ratio > 0) then
        local dir = obj.track1
        if (obj.check0) then
            dir = (dir + obj.rand(0, 360, obj.index, 0)) % 360
        end
        local rad = dir * math.pi / 180
        local dist = math.sqrt(obj.screen_w * obj.screen_w + obj.screen_h * obj.screen_h) * ratio
        obj.ox = obj.ox + math.sin(rad) * dist
        obj.oy = obj.oy - math.cos(rad) * dist
    end
end

@ランダム方向から登場
--track0:時間,-5,5,1.5,0.01
--track1:回転,0,10,3
--track2:ライト,0,100,30
--check0:加減速,1
if (obj.track0 ~= 0) then
    local ratio = 0
    if (obj.track0 < 0) then
        ratio = (obj.track0 - obj.time + obj.totaltime) / obj.track0
    else
        ratio = (obj.track0 - obj.time) / obj.track0
    end
    if (ratio > 0) then
        local ease = ratio
        if (obj.check0) then
            ease = ratio * ratio
        end
        local ang = obj.rand(0, 360, obj.index, 0)
        local dist = math.sqrt(obj.screen_w * obj.screen_w + obj.screen_h * obj.screen_h) * ease
        obj.ox = obj.ox + math.sin(ang) * dist
        obj.oy = obj.oy - math.cos(ang) * dist
        obj.rx = obj.rx + obj.rand(90, 360, obj.index, 1) * ease * obj.track1
        obj.ry = obj.ry + obj.rand(90, 360, obj.index, 2) * ease * obj.track1
        obj.rz = obj.rz + obj.rand(90, 360, obj.index, 3) * ease * obj.track1 * 2
        obj.zoom = obj.zoom * (1 + ease)
    end
end

@拡大縮小して登場
--track0:時間,-5,5,0.3,0.01
--track1:拡大率,0,800,300
--check0:加減速,1
if (obj.track0 ~= 0) then
    local ratio = 0
    if (obj.track0 < 0) then
        ratio = (obj.track0 - obj.time + obj.totaltime) / obj.track0
    else
        ratio = (obj.track0 - obj.time) / obj.track0
    end
    if (ratio > 0) then
        local clamped = math.min(1, ratio)
        obj.alpha = obj.alpha * math.max(0, math.cos(clamped * math.pi * 0.5))
        local ease = clamped
        if (obj.check0) then
            ease = clamped * clamped
        end
        local s = 1 + (obj.track1 - 100) * ease * 0.01
        obj.zoom = obj.zoom * math.max(0, s)
    end
end

@ランダム間隔で落ちながら登場
--track0:時間,-5,5,0.5,0.01
--track1:間隔,0,5,1,0.01
--track2:距離,0,1000,400
--check0:加減速,1
if (obj.track0 ~= 0) then
    local delay = obj.rand(0, 100, obj.index, 0) * obj.track1 * 0.01
    local ratio = 0
    local vy = 0
    if (obj.track0 < 0) then
        ratio = (obj.track0 - obj.time + obj.totaltime - delay) / obj.track0
        vy = obj.track2
    else
        ratio = (obj.track0 - obj.time + delay) / obj.track0
        vy = -obj.track2
    end
    if (ratio >= 1) then
        obj.alpha = 0
    elseif (ratio > 0) then
        local ease = ratio
        if (obj.check0) then
            ease = ratio * ratio
        end
        obj.alpha = obj.alpha * math.max(0, math.cos(ease * math.pi * 0.5))
        obj.oy = obj.oy + vy * ease
    end
end

@弾んで登場
--track0:時間,-5,5,2,0.01
--track1:高さ,-1000,1000,200
--track2:回数,1,20,4,1
local cnt = math.max(1, obj.track2)
local dur = math.floor(obj.track0 * obj.framerate / cnt) * cnt / obj.framerate
if (dur ~= 0) then
    local ratio = 0
    local dir_sign = 1
    if (dur < 0) then
        ratio = (obj.track0 - obj.time + obj.totaltime) / dur
        dir_sign = -1
    else
        ratio = (obj.track0 - obj.time) / dur
    end
    if (ratio >= 0) then
        local span = math.sqrt(obj.screen_w * obj.screen_w + obj.screen_h * obj.screen_h)
        obj.ox = obj.ox + span * ratio * dir_sign
        local u = ((ratio * cnt) % 1 - 0.5) * 2
        obj.oy = obj.oy - (1 - u * u) * obj.track1
    end
end

@広がって登場
--track0:時間,-5,5,0.3,0.01
--check0:縦方向,0
if (obj.track0 ~= 0) then
    local ratio = 0
    if (obj.track0 < 0) then
        ratio = (obj.track0 - obj.time + obj.totaltime) / obj.track0
    else
        ratio = (obj.track0 - obj.time) / obj.track0
    end
    ratio = math.min(1, math.max(0, ratio))
    if (ratio > 0) then
        local ease = 1 - ratio * ratio
        if (obj.check0) then
            obj.effect("リサイズ", "X", 100, "Y", ease * 100)
        else
            obj.effect("リサイズ", "X", ease * 100, "Y", 100)
        end
    end
end

@起き上がって登場
--track0:時間,-5,5,0.6,0.01
--track1:勢い,0,8,2.0
if (obj.track0 ~= 0) then
    local ratio = 0
    if (obj.track0 < 0) then
        ratio = (obj.track0 - obj.time + obj.totaltime) / obj.track0
    else
        ratio = (obj.track0 - obj.time) / obj.track0
    end
    if (ratio >= 0) then
        local ease = math.min(1, ratio * ratio)
        obj.rx = obj.rx - 90 * ease
        obj.oy = obj.oy + obj.h * (1 - math.cos(ease * math.pi * 0.5)) * 0.5
        obj.oz = obj.oz + obj.h * math.sin(ease * math.pi * 0.5) * 0.5
    end
end

@何処からともなく登場
--track0:時間,-5,5,0.3,0.01
--track1:ぼかし,0,5,1.0
--track2:位置,0,5,1.5
if (obj.track0 ~= 0) then
    local ratio = 0
    if (obj.track0 < 0) then
        ratio = (obj.track0 - obj.time + obj.totaltime) / obj.track0
    else
        ratio = (obj.track0 - obj.time) / obj.track0
    end
    if (ratio > 0) then
        local rx = obj.w * obj.track2 * 0.5
        local ry = obj.h * obj.track2 * 0.5
        obj.ox = obj.ox + obj.rand(-rx, rx, obj.index + 100)
        obj.oy = obj.oy + obj.rand(-ry, ry, obj.index + 200)
        obj.alpha = obj.alpha * (obj.rand(0, 100, obj.index + 300) * 0.01)
        obj.effect("ぼかし", "範囲", obj.track1 * obj.w)
    end
end

@反復移動
--track0:速さ,0.1,100,20
--track1:距離,0,1000,100
--track2:角度,0,360,0
--track3:ずらし,0,100,0
local d = math.sin((obj.time * obj.track0 + obj.index * obj.track3 * 0.2) * math.pi * 0.1) * obj.track1
local rad = obj.track2 * math.pi / 180
obj.ox = obj.ox + math.sin(rad) * d
obj.oy = obj.oy - math.cos(rad) * d

@座標の回転(個別オブジェクト)
--track0:角度,-360,360,0
--track1:中心X,-1000,1000,0
--track2:中心Y,-1000,1000,0
local dx = obj.ox - obj.track1
local dy = obj.oy - obj.track2
local rad = obj.track0 * math.pi / 180
obj.ox = obj.track1 + dx * math.cos(rad) - dy * math.sin(rad)
obj.oy = obj.track2 + dx * math.sin(rad) + dy * math.cos(rad)

@立方体(カメラ制御)
--track0:サイズ,1,2000,100
--track1:分解,0,1000,0
--check0:裏面を表示しない,1
if (obj.check0) then
    obj.setoption("culling", 1)
else
    obj.setoption("culling", 0)
end
local sz = obj.track0
local d = sz * 0.5 + obj.track1
local zm = sz / math.max(1, obj.w)
obj.aspect = (1 - obj.w / math.max(1, obj.h)) * 100
obj.draw(0, 0, -d, zm, 1, 0, 0, 0)
obj.draw(0, 0, d, zm, 1, 0, 180, 0)
obj.draw(-d, 0, 0, zm, 1, 0, 90, 0)
obj.draw(d, 0, 0, zm, 1, 0, -90, 0)
obj.draw(0, -d, 0, zm, 1, -90, 0, 0)
obj.draw(0, d, 0, zm, 1, 90, 0, 0)

@球体(カメラ制御)
--track0:サイズ,1,2000,100
--track1:分解,0,1000,0
--check0:裏面を表示しない,1
if (obj.check0) then
    obj.setoption("culling", 1)
end
local r = obj.track0 * 0.5 + obj.track1
local n = 8
for i = 0, n - 1 do
    local a = i * 360 / n
    local rad = a * math.pi / 180
    obj.draw(math.sin(rad) * r, 0, -math.cos(rad) * r, obj.track0 / math.max(1, obj.w) * 0.4, 1, 0, a, 0)
end

@砕け散る
--track0:時間,-5,5,1,0.01
--track1:サイズ,5,500,50
--track2:速度,0,1000,100
--track3:重力,-1000,1000,100
local dur = math.abs(obj.track0)
if (dur > 0) then
    local t = 0
    if (obj.track0 > 0) then
        t = math.max(0, obj.time - (obj.totaltime - dur)) / dur
    else
        t = math.max(0, dur - obj.time) / dur
    end
    t = math.min(1, t)
    if (t > 0) then
        obj.oy = obj.oy + t * t * obj.track3 * 2
        obj.alpha = obj.alpha * (1 - t)
    end
end

@点滅
--track0:割合,0,100,50
--track1:間隔,0,50,1,1
--track2:光強さ,0,200,0
local step = math.max(1, math.floor(obj.track1))
local f = math.floor(obj.frame / step)
local phase = (f * 37) % 100
if (phase >= obj.track0) then
    obj.alpha = 0
end

@点滅して登場
--track0:時間,-5,5,0.5,0.01
--track1:点滅間隔,0,20,1,1
--track2:順序間隔,-5,5,0,0.01
if (obj.track0 ~= 0) then
    local dur = math.abs(obj.track0)
    local ratio = 0
    if (obj.track0 > 0) then
        ratio = (dur - obj.time + obj.track2 * obj.index) / dur
    else
        ratio = (dur - obj.totaltime + obj.time) / dur
    end
    ratio = math.min(1, math.max(0, ratio))
    if (ratio >= 1) then
        obj.alpha = 0
    elseif (ratio > 0) then
        local step = math.max(1, math.floor(obj.track1))
        if (obj.rand(0, 100, obj.index, math.floor(obj.frame / step)) < ratio * 100) then
            obj.alpha = 0
        end
    end
end

@簡易変形
--track0:角度,-3600,3600,0
--track1:幅,0,1000,100
--track2:高さ,0,1000,100
--track3:歪み,-500,500,0
obj.rz = obj.rz + obj.track0
obj.effect("リサイズ", "X", obj.track1, "Y", obj.track2)

@簡易変形(カメラ制御)
--track0:角度,-3600,3600,0
--track1:幅,0,1000,100
--track2:高さ,0,1000,100
obj.rz = obj.rz + obj.track0
obj.effect("リサイズ", "X", obj.track1, "Y", obj.track2)

@リール回転
--track0:速度,-100,100,10
--track1:ブラー,0,100,20
--track2:間隔,0,500,0
--check0:横方向,0
local spd = obj.track0 * obj.time * 100
if (obj.check0) then
    obj.ox = obj.ox + (spd % math.max(1, obj.w + obj.track2))
else
    obj.oy = obj.oy + (spd % math.max(1, obj.h + obj.track2))
end

@万華鏡
--track0:サイズ,10,2000,200
--track1:回転,-3600,3600,0
--track2:分割数,2,64,6,2
local n = math.max(2, math.floor(obj.track2))
for i = 0, n - 1 do
    obj.draw(0, 0, 0, 1, 1, 0, 0, obj.track1 + i * 360 / n)
end

@円形配置
--track0:半径,0,2000,100
--track1:角度,-3600,3600,0
--track2:回転,-3600,3600,0
--check0:カメラの方を向く,0
--dialog:個数,local num=8;方向(0:時計/1:反時計),local dir=0;回転同期(0:OFF/1:ON),local rot=0;
local r = obj.track0
local n = obj.num
local is_multi = (n > 1)
if (not is_multi) then
    n = math.max(1, math.floor(num or 8))
end
if (obj.check0) then
    obj.setoption("billboard", 3)
end
for i = 0, n - 1 do
    local idx = i
    if (is_multi) then idx = obj.index end
    local deg = 360 * idx / n + obj.track1
    if ((dir or 0) == 1) then deg = -deg end
    local rad = deg * math.pi / 180
    local ox = math.sin(rad) * r
    local oy = -math.cos(rad) * r
    local rz = obj.track2
    if ((rot or 0) == 1) then rz = rz + deg end
    if (is_multi) then
        obj.ox = obj.ox + ox
        obj.oy = obj.oy + oy
        obj.rz = obj.rz + rz
        break
    else
        obj.draw(ox, oy, 0, 1, 1, 0, 0, rz)
    end
end

@ランダム配置
--track0:個数,0,1000,10,1
--track1:範囲,0,5000,200
--track2:移動速度,0,1000,0
--track3:回転速度,0,1000,0
--check0:カメラの方を向く,0
--dialog:X範囲[%],local rx=100;Y範囲[%],local ry=100;Z範囲[%],local rz=0;X回転,local dx=0;Y回転,local dy=0;Z回転,local dz=0;最小拡大率[%],local sz=100;
local n = obj.num
local is_multi = (n > 1)
if (not is_multi) then
    n = math.max(1, math.floor(obj.track0))
end
if (obj.check0) then
    obj.setoption("billboard", 3)
end
local rng = obj.track1 * 0.5
local spd = obj.track2 * 0.01
local rspd = obj.track3 * 0.01
local rx_scale = (rx or 100) * 0.01
local ry_scale = (ry or 100) * 0.01
local rz_scale = (rz or 0) * 0.01
local min_sz = (sz or 100)
for i = 1, n do
    local seed = i
    if (is_multi) then seed = -obj.index - 1 end
    local x = (obj.rand(-1000, 1000, seed, 1) * 0.001 + math.sin(obj.time * spd + seed) * 0.2) * rng * rx_scale
    local y = (obj.rand(-1000, 1000, seed, 2) * 0.001 + math.cos(obj.time * spd + seed) * 0.2) * rng * ry_scale
    local z = (obj.rand(-1000, 1000, seed, 3) * 0.001) * rng * rz_scale
    local zm = obj.rand(min_sz, 100, seed, 4) * 0.01
    local rot_x = 0
    local rot_y = 0
    local rot_z = 0
    if ((dx or 0) ~= 0) then rot_x = obj.rand(0, 360, seed, 5) + obj.time * rspd * 60 end
    if ((dy or 0) ~= 0) then rot_y = obj.rand(0, 360, seed, 6) + obj.time * rspd * 60 end
    if ((dz or 0) ~= 0) then rot_z = obj.rand(0, 360, seed, 7) + obj.time * rspd * 60 end
    if (is_multi) then
        obj.ox = obj.ox + x
        obj.oy = obj.oy + y
        obj.oz = obj.oz + z
        obj.rx = obj.rx + rot_x
        obj.ry = obj.ry + rot_y
        obj.rz = obj.rz + rot_z
        obj.zoom = obj.zoom * zm
        break
    else
        obj.draw(x, y, z, zm, 1, rot_x, rot_y, rot_z)
    end
end

@分割登場-スライス@追加1
--track0:分割数,1,100,5,1
--track1:時間,-10,10,0.5,0.01
--track2:ずれ,-10,10,0.05,0.01
--track3:移動量,-2000,2000,100
--dialog:移動方向(0:横/1:縦),local tated=0;分割方向(0:横/1:縦),local tateb=0;交互(0:OFF/1:ON),local kougo=0;べき乗,local beki=2;移動量,local l=100;
local slices = math.max(1, math.floor(obj.track0))
local dur = obj.track1
if (dur == 0) then dur = 0.5 end
local abs_dur = math.abs(dur)
local step_delay = obj.track2
local move_dist = l or obj.track3
local pow_exp = math.max(1, beki or 2)
local is_vert_move = ((tated or 0) ~= 0)
local is_vert_split = ((tateb or 0) ~= 0)
local is_alt = ((kougo or 0) ~= 0)

for i = 0, slices - 1 do
    local ratio = 0
    if (dur > 0) then
        ratio = (abs_dur - obj.time + step_delay * i) / abs_dur
    else
        ratio = (abs_dur - (obj.totaltime - obj.time) + step_delay * i) / abs_dur
    end
    ratio = math.min(1, math.max(0, ratio))
    local ease = math.pow(ratio, pow_exp)
    local sign = 1
    if (is_alt and (i % 2 == 1)) then sign = -1 end
    local delta = move_dist * ease * sign
    local dx = 0
    local dy = 0
    if (is_vert_move) then
        dy = delta
    else
        dx = delta
    end
    if (is_vert_split) then
        local sw = obj.w / slices
        local clip_l = i * sw
        local clip_r = (slices - 1 - i) * sw
        obj.effect("クリッピング", "上", 0, "下", 0, "左", clip_l, "右", clip_r)
    else
        local sh = obj.h / slices
        local clip_t = i * sh
        local clip_b = (slices - 1 - i) * sh
        obj.effect("クリッピング", "上", clip_t, "下", clip_b, "左", 0, "右", 0)
    end
    obj.draw(dx, dy, 0, 1, 1 - ease * 0.5)
end

@ランダム配置@ANM1
--track0:個数,0,1000,10,1
--track1:範囲,0,5000,200
--track2:移動速度,0,1000,0
--track3:回転速度,0,1000,0
--dialog:X範囲[%],local wx=100;Y範囲[%],local wy=100;Z範囲[%],local wz=100;X回転,local kx=0;Y回転,local ky=0;Z回転,local kz=0;最小拡大率[%],local s_min=100;シード,local ran=1;
local n = obj.num
local is_multi = (n > 1)
if (not is_multi) then
    n = math.min(64, math.max(1, math.floor(obj.track0)))
end
local rng = obj.track1 * 0.5
local spd = obj.track2 * 0.01
local rspd = obj.track3 * 0.01
local sx = (wx or rx or 100) * 0.01
local sy = (wy or ry or 100) * 0.01
local sz_r = (wz or rz or 0) * 0.01
local min_s = (s_min or sz or 100)
local base_seed = math.floor(ran or 1) * 100
for i = 1, n do
    local seed = base_seed + i
    if (is_multi) then seed = base_seed - obj.index - 1 end
    local x = (obj.rand(-1000, 1000, seed, 1) * 0.001 + math.sin(obj.time * spd + seed) * 0.2) * rng * sx
    local y = (obj.rand(-1000, 1000, seed, 2) * 0.001 + math.cos(obj.time * spd + seed) * 0.2) * rng * sy
    local z = (obj.rand(-1000, 1000, seed, 3) * 0.001) * rng * sz_r
    local zm = obj.rand(min_s, 100, seed, 4) * 0.01
    local rot_x = 0
    local rot_y = 0
    local rot_z = 0
    if ((kx or dx or 0) ~= 0) then rot_x = obj.rand(0, 360, seed, 5) + obj.time * rspd * 60 end
    if ((ky or dy or 0) ~= 0) then rot_y = obj.rand(0, 360, seed, 6) + obj.time * rspd * 60 end
    if ((kz or dz or 0) ~= 0) then rot_z = obj.rand(0, 360, seed, 7) + obj.time * rspd * 60 end
    if (is_multi) then
        obj.ox = obj.ox + x
        obj.oy = obj.oy + y
        obj.oz = obj.oz + z
        obj.rx = obj.rx + rot_x
        obj.ry = obj.ry + rot_y
        obj.rz = obj.rz + rot_z
        obj.zoom = obj.zoom * zm
        break
    else
        obj.draw(x, y, z, zm, 1, rot_x, rot_y, rot_z)
    end
end

@一方向に並べる@ANM1
--track0:個数,-100,100,5,1
--track1:間隔,-2000,2000,100
--track2:角度,-3600,3600,0
--track3:Z間隔,-2000,2000,0
local raw_n = obj.track0
local n = math.min(64, math.max(1, math.floor(math.abs(raw_n))))
local sign = 1
if (raw_n < 0) then sign = -1 end
local step = obj.track1 * sign
local rad = obj.track2 * math.pi / 180
local dz = obj.track3
local dx = math.cos(rad) * step
local dy = math.sin(rad) * step
for i = 0, n - 1 do
    obj.draw(dx * i, dy * i, dz * i, 1, 1)
end

@扇クリッピングR
--track0:終了角,-3600,3600,360
--track1:開始角,-3600,3600,0
--track2:中心X,-2000,2000,0
--track3:中心Y,-2000,2000,0
obj.effect("扇クリッピングR", "終了角", obj.track0, "開始角", obj.track1, "中心X", obj.track2, "中心Y", obj.track3)

@斜めブラインド@ANM1
--track0:進捗,0,100,50
--track1:幅,2,500,50
--track2:角度,-360,360,45
--track3:ぼかし,0,100,0
local p = math.min(100, math.max(0, obj.track0)) * 0.01
obj.alpha = obj.alpha * (1 - p * 0.85)
local rad = obj.track2 * math.pi / 180
obj.ox = obj.ox + math.cos(rad) * p * obj.track1 * 0.25
obj.oy = obj.oy + math.sin(rad) * p * obj.track1 * 0.25

@画像ループ2@ANM1
--track0:横拡大率,1,2000,100
--track1:縦拡大率,1,2000,100
--track2:X速度,-2000,2000,0
--track3:Y速度,-2000,2000,0
local nx = math.min(12, math.max(1, math.ceil(obj.track0 / 100)))
local ny = math.min(12, math.max(1, math.ceil(obj.track1 / 100)))
local step_x = math.max(32, obj.w)
local step_y = math.max(32, obj.h)
local sx = (obj.time * obj.track2) % step_x
local sy = (obj.time * obj.track3) % step_y
for iy = 0, ny - 1 do
    for ix = 0, nx - 1 do
        local ox = (ix - (nx - 1) * 0.5) * step_x + sx
        local oy = (iy - (ny - 1) * 0.5) * step_y + sy
        obj.draw(ox, oy, 0, 1, 1)
    end
end
"#;

/// 拡張編集互換の標準カスタムオブジェクトスクリプト（クリーンルーム実装）
const BUILTIN_EXEDIT_OBJ: &str = r#"@集中線
--track0:本数,1,1000,100,1
--track1:内半径,0,2000,150
--track2:外半径,10,5000,800
--track3:太さ,1,200,8
--dialog:色/col,local col=0xffffff;
local n = math.max(1, math.floor(obj.track0))
local r_in = obj.track1
local r_out = obj.track2
local w = obj.track3
obj.load("figure", "四角形", col or 0xffffff, math.max(r_out - r_in, 10))
local step = math.floor(obj.frame / 2)
for i = 1, n do
    local deg = (i / n) * 360 + obj.rand(-10, 10, i, step)
    local rad = deg * math.pi / 180
    local mid = (r_in + r_out) * 0.5 + obj.rand(-30, 30, i + 100, step)
    obj.draw(math.cos(rad) * mid, math.sin(rad) * mid, 0, 0.2, 0.85, 0, 0, deg)
end

@走査線
--track0:幅,1,200,4,1
--track1:間隔,1,200,8,1
--track2:速度,-500,500,20
--track3:透明度,0,100,40
--dialog:色/col,local col=0x000000;
local line_h = math.max(1, obj.track0)
local pitch = math.max(2, line_h + obj.track1)
local alpha = 1 - obj.track3 * 0.01
obj.load("figure", "四角形", col or 0x000000, obj.screen_w)
local count = math.min(64, math.ceil(obj.screen_h / pitch))
local offset = (obj.time * obj.track2) % pitch
for i = 0, count do
    local y = -obj.screen_h * 0.5 + i * pitch + offset
    obj.draw(0, y, 0, 1, alpha)
end

@カウンター
--track0:初期値,-100000,100000,0
--track1:速度,-10000,10000,100
--track2:サイズ,1,500,48,1
--dialog:文字色/col,local col=0xffffff;
local val = obj.track0 + obj.time * obj.track1
obj.setfont("MS UI Gothic", obj.track2, 0, col or 0xffffff)
obj.mes(string.format("%d", math.floor(val)))

@レンズフレア
--track0:サイズ,10,2000,200
--track1:強さ,0,100,80
--track2:X,-2000,2000,0
--track3:Y,-2000,2000,0
--dialog:色/col,local col=0x88ccff;
obj.setoption("blend", 1)
obj.load("figure", "円", col or 0x88ccff, obj.track0)
local a = obj.track1 * 0.01
for i = 1, 6 do
    local t = (i - 2.5) * 0.45
    local zm = (0.3 + (i % 3) * 0.25)
    obj.draw(obj.track2 * t, obj.track3 * t, 0, zm, a * 0.5)
end

@雲
--track0:幅,10,2000,400
--track1:高さ,10,2000,250
--track2:密度,0,200,100
--track3:移動速度,-500,500,20
--dialog:色/col,local col=0xffffff;遠近感,local pers=100;細かさ,local freq=4;透過度,local trans=1;解像度,local res=60;境界補正,local adjust=25;
local w = obj.track0
local h = obj.track1
local density = obj.track2 * 0.01
local spd = obj.track3
obj.load("figure", "円", col or 0xffffff, math.min(w, h) * 0.55)
local puffs = 14
for i = 1, puffs do
    local rx = (obj.rand(-1000, 1000, i, 10) * 0.0005 * w + obj.time * spd) % w - w * 0.5
    local ry = obj.rand(-1000, 1000, i, 20) * 0.00035 * h
    local zm = (0.5 + obj.rand(0, 1000, i, 30) * 0.0008) * math.min(1.5, math.max(0.3, density))
    local alp = math.min(0.65, 0.25 * density)
    obj.draw(rx, ry, 0, zm, alp)
end

@星
--track0:幅,0,5000,1000
--track1:周期,0,200,1.5
--track2:数,0,500,20
--track3:速度,-500,500,0
--dialog:サイズ,local size=20;色/col,local col=0xddddff;図形名,local name="円";フェードイン,local f_in=0.15;フェードアウト,local f_out=0.15;
local raw_cnt = obj.track2
if (raw_cnt <= 1) then raw_cnt = math.max(12, obj.track0 * 0.05) end
local count = math.min(120, math.max(1, math.floor(raw_cnt * 2.5)))
local blink_spd = math.max(0.5, obj.track1 * 2.5)
local move_spd = obj.track3 * 2
local sz = size or 20
local fig = name or "円"
obj.setoption("blend", 1)
obj.load("figure", fig, col or 0xddddff, sz)
local w_span = math.max(obj.screen_w, obj.track0)
for i = 1, count do
    local x = (obj.rand(-1000, 1000, i, 1) * 0.0005 * w_span + obj.time * move_spd) % w_span
    if (x > w_span * 0.5) then x = x - w_span end
    if (x < -w_span * 0.5) then x = x + w_span end
    local y = obj.rand(-1000, 1000, i, 2) * 0.0005 * obj.screen_h
    local zm = obj.rand(250, 1000, i, 3) * 0.001
    local phase = obj.rand(0, 628, i, 4) * 0.01 + obj.time * blink_spd
    local twinkle = (math.sin(phase) * 0.45 + 0.55)
    obj.draw(x, y, 0, zm, math.min(1, math.max(0, twinkle)))
end

@雪
--track0:量,0,500,100
--track1:速度,1,500,100
--track2:風,-500,500,0
--track3:揺れ,0,500,50
--dialog:サイズ,local size=15;色/col,local col=0xffffff;図形名,local name="円";揺れ速度,local f_speed=2;奥行き,local depth=100;
local count = math.min(140, math.max(1, math.floor(obj.track0 * 0.5)))
local fall_spd = obj.track1 * 2
local wind = obj.track2
local sway = obj.track3 * 0.3
local sz = size or 15
local fig = name or "円"
obj.load("figure", fig, col or 0xffffff, sz)
local h_span = obj.screen_h + sz * 2
local w_span = obj.screen_w + sz * 2
for i = 1, count do
    local z_ratio = obj.rand(300, 1000, i, 10) * 0.001
    local x0 = obj.rand(-1000, 1000, i, 1) * 0.0005 * w_span
    local y0 = obj.rand(-1000, 1000, i, 2) * 0.0005 * h_span
    local sw = math.sin(obj.time * (f_speed or 2) + i) * sway * z_ratio
    local x = ((x0 + obj.time * wind * z_ratio + sw + w_span * 0.5) % w_span) - w_span * 0.5
    local y = ((y0 + obj.time * fall_spd * z_ratio + h_span * 0.5) % h_span) - h_span * 0.5
    obj.draw(x, y, 0, z_ratio, z_ratio * 0.85)
end

@雨
--track0:幅,10,5000,400
--track1:速度,0.1,1000,4
--track2:数,-500,500,40
--track3:角度,-60,60,0
--dialog:太さ,local size=4;色/col,local col=0xffffff;奥行き,local depth=10;
local w_span = math.max(200, obj.track0)
if (w_span < 100) then w_span = obj.screen_w end
local spd = math.max(50, obj.track1 * 220)
local raw_cnt = math.abs(obj.track2)
if (raw_cnt < 8) then raw_cnt = math.max(15, obj.track0 * 0.4) end
local count = math.min(120, math.max(1, math.floor(raw_cnt)))
local ang = obj.track3
local len = math.max(40, (size or 4) * 28)
obj.load("figure", "四角形", col or 0xffffff, len)
obj.aspect = -93
local h_span = obj.screen_h + len * 2
local rad = ang * math.pi / 180
for i = 1, count do
    local z_ratio = obj.rand(400, 1000, i, 10) * 0.001
    local x0 = obj.rand(-1000, 1000, i, 1) * 0.0005 * w_span
    local y0 = obj.rand(-1000, 1000, i, 2) * 0.0005 * h_span
    local dist = obj.time * spd * z_ratio
    local x = ((x0 - math.sin(rad) * dist + w_span * 0.5) % w_span) - w_span * 0.5
    local y = ((y0 + math.cos(rad) * dist + h_span * 0.5) % h_span) - h_span * 0.5
    obj.draw(x, y, 0, z_ratio, z_ratio * 0.55, 0, 0, ang)
end

@ランダム小物配置(カメラ制御)
--track0:個数,1,500,30,1
--track1:範囲,10,5000,500
--track2:サイズ,1,500,40
--dialog:色/col,local col=0xffffff;図形名,local name="円";
local n = math.min(120, math.max(1, math.floor(obj.track0)))
local rng = obj.track1 * 0.5
obj.load("figure", name or "円", col or 0xffffff, obj.track2)
for i = 1, n do
    local x = obj.rand(-1000, 1000, i, 1) * 0.001 * rng
    local y = obj.rand(-1000, 1000, i, 2) * 0.001 * rng
    local z = obj.rand(-1000, 1000, i, 3) * 0.001 * rng
    obj.draw(x, y, z)
end

@ライン(移動軌跡)
--track0:数,1,500,40,1
--track1:速度,1,1000,100
--track2:サイズ,1,200,15
--dialog:色/col,local col=0xffffff;
local n = math.min(80, math.max(1, math.floor(obj.track0)))
obj.load("figure", "円", col or 0xffffff, obj.track2)
for i = 1, n do
    local t = obj.time - i * 0.02
    local x = math.cos(t * 2) * 200
    local y = math.sin(t * 3) * 120
    obj.draw(x, y, 0, 1 - i / (n + 1), 1 - i / (n + 1))
end

@扇型
--track0:基準角,-3600,3600,0
--track1:角度,0,360,90
--track2:サイズ,1,2000,200
--track3:ライン幅,0,2000,4000
--dialog:色/col,local col=0xffffff;
obj.load("figure", "円", col or 0xffffff, obj.track2, obj.track3)
obj.rz = obj.rz + obj.track0

@多角形
--track0:角数,3,32,5,1
--track1:サイズ,1,2000,200
--track2:ライン幅,0,2000,4000
--dialog:色/col,local col=0xffffff;
obj.load("figure", "五角形", col or 0xffffff, obj.track1, obj.track2)

@周辺ボケ光量
--track0:光量,0,200,80
--track1:範囲,0,200,100
--dialog:色/col,local col=0x000000;
obj.load("figure", "円", col or 0x000000, math.max(obj.screen_w, obj.screen_h), obj.track1 * 2)
obj.alpha = obj.alpha * (1 - obj.track0 * 0.005)

@フレア
--track0:サイズ,1,2000,250
--track1:強さ,0,100,85
--track2:光芒数,0,32,6,1
--track3:光芒長,0,500,120
--dialog:中心透過,local c_alp=0.5;速度,local speed=0.2;点滅,local blink=0.1;輪長さ,local h_len=0.1;輪数,local h_num=1;光色/col,local f_col=0x9999ff;
local sz = math.max(10, obj.track0)
local intensity = math.min(1, math.max(0.1, obj.track1 * 0.01))
local fcol = f_col or 0x9999ff
local pulse = 1 - (blink or 0.1) * 0.5 * (1 - math.sin(obj.time * (speed or 0.2) * math.pi * 6))
obj.setoption("blend", 1)
-- 外周の光輪（ハロー）
obj.load("figure", "円", fcol, sz * 1.4)
obj.draw(0, 0, 0, 1.0, intensity * 0.35 * pulse)
obj.draw(0, 0, 0, 0.65, intensity * 0.55 * pulse)
-- 光芒（クロスストリーク）
local rays = math.min(12, math.max(2, math.floor(obj.track2)))
obj.load("figure", "四角形", fcol, sz * 1.8)
obj.aspect = -94
for i = 1, rays do
    local deg = (i - 1) * (180 / rays) + obj.time * (speed or 0.2) * 30
    obj.draw(0, 0, 0, 1.0, intensity * 0.45 * pulse, 0, 0, deg)
end
-- 中心コア（ホワイト）
obj.load("figure", "円", 0xffffff, sz * 0.4)
obj.draw(0, 0, 0, 1.0, math.min(1, (1 - (c_alp or 0.5) * 0.5) * intensity * pulse))

@水面
--track0:高さ,-1000,1000,150
--track1:振幅,0,200,15
--track2:速度,-100,100,20
--dialog:色/col,local col=0x3388ff;
obj.load("figure", "四角形", col or 0x3388ff, obj.screen_w)
obj.oy = obj.oy + obj.track0 + math.sin(obj.time * obj.track2 * 0.2) * obj.track1
obj.alpha = 0.55

@角丸四角形@hksy
--track0:横幅,0,4000,200
--track1:縦幅,0,4000,200
--track2:ライン幅,0,4000,4000
--track3:角丸半径,0,2000,30
--dialog:色/col,local color=0xffffff;
local w = math.max(2, obj.track0)
local h = math.max(2, obj.track1)
local lw = obj.track2
local c = color or col or 0xffffff
local sz = math.max(w, h)
obj.load("figure", "四角形", c, sz, lw)
if (w >= h) then
    obj.aspect = (h / w) - 1
else
    obj.aspect = 1 - (w / h)
end
obj.w = w
obj.h = h

@日の出
--track0:本数,1,64,8,1
--track1:角度幅,1,100,50
--track2:回転,-3600,3600,0
--track3:内半径,0,2000,0
--dialog:色/col,local col=0xffffff;横幅,local w=1000;縦幅,local h=1000;
local n = math.min(64, math.max(1, math.floor(obj.track0)))
local ratio = math.min(95, math.max(5, obj.track1)) * 0.01
local rot_deg = obj.track2
local r_in = math.max(0, obj.track3)
local r_out = math.max(r_in + 50, math.max(w or 1000, h or 1000) * 0.85)
local c = col or color or 0xffffff
local step_deg = 360 / n
local span_deg = step_deg * ratio
for i = 0, n - 1 do
    local a0 = (rot_deg + i * step_deg - span_deg * 0.5) * math.pi / 180
    local a1 = (rot_deg + i * step_deg + span_deg * 0.5) * math.pi / 180
    local x0 = math.cos(a0) * r_in
    local y0 = math.sin(a0) * r_in
    local x1 = math.cos(a1) * r_in
    local y1 = math.sin(a1) * r_in
    local x2 = math.cos(a1) * r_out
    local y2 = math.sin(a1) * r_out
    local x3 = math.cos(a0) * r_out
    local y3 = math.sin(a0) * r_out
    obj.drawpoly(x0, y0, 0, x1, y1, 0, x2, y2, 0, x3, y3, 0)
end
"#;

/// 拡張編集互換の標準カメラ効果スクリプト（クリーンルーム実装）
const BUILTIN_EXEDIT_CAM: &str = r#"@手ぶれ
--track0:振幅,0,100,10
--track1:角度,0,180,0.1,0.01
--track2:間隔,0,100,50
local cam = obj.getoption("camera_param")
if (obj.track2 <= 0) then
    cam.tx = cam.tx + obj.rand(-obj.track0, obj.track0, 0)
    cam.ty = cam.ty + obj.rand(-obj.track0, obj.track0, 1)
    cam.tz = cam.tz + obj.rand(-obj.track0, obj.track0, 2)
    cam.rz = cam.rz + obj.rand(-obj.track1 * 100, obj.track1 * 100, 3) * 0.01
else
    local t = obj.time * 100 / obj.track2
    local p = math.floor(t)
    t = t - p
    local amp = obj.track0 * 0.001
    local ang = obj.track1 * 0.001
    local x0 = obj.rand(-1000, 1000, 0, p + 0) * amp
    local x1 = obj.rand(-1000, 1000, 0, p + 1) * amp
    local x2 = obj.rand(-1000, 1000, 0, p + 2) * amp
    local x3 = obj.rand(-1000, 1000, 0, p + 3) * amp
    local y0 = obj.rand(-1000, 1000, 1, p + 0) * amp
    local y1 = obj.rand(-1000, 1000, 1, p + 1) * amp
    local y2 = obj.rand(-1000, 1000, 1, p + 2) * amp
    local y3 = obj.rand(-1000, 1000, 1, p + 3) * amp
    local z0 = obj.rand(-1000, 1000, 2, p + 0) * amp
    local z1 = obj.rand(-1000, 1000, 2, p + 1) * amp
    local z2 = obj.rand(-1000, 1000, 2, p + 2) * amp
    local z3 = obj.rand(-1000, 1000, 2, p + 3) * amp
    local r0 = obj.rand(-1000, 1000, 3, p + 0) * ang
    local r1 = obj.rand(-1000, 1000, 3, p + 1) * ang
    local r2 = obj.rand(-1000, 1000, 3, p + 2) * ang
    local r3 = obj.rand(-1000, 1000, 3, p + 3) * ang
    local dx, dy, dz = obj.interpolation(t, x0, y0, z0, x1, y1, z1, x2, y2, z2, x3, y3, z3)
    local dr = obj.interpolation(t, r0, r1, r2, r3)
    cam.tx = cam.tx + dx
    cam.ty = cam.ty + dy
    cam.tz = cam.tz + dz
    cam.rz = cam.rz + dr
end
obj.setoption("camera_param", cam)

@目標中心回転
--track0:水平回転,-3600,3600,0,0.01
--track1:垂直回転,-3600,3600,0,0.01
local cam = obj.getoption("camera_param")
local dx = cam.x - cam.tx
local dy = cam.y - cam.ty
local dz = cam.z - cam.tz
local yaw = obj.track0 * math.pi / 180
local pitch = obj.track1 * math.pi / 180
local cy = math.cos(yaw)
local sy = math.sin(yaw)
local x1 = dx * cy + dz * sy
local z1 = -dx * sy + dz * cy
local cp = math.cos(pitch)
local sp = math.sin(pitch)
local y2 = dy * cp - z1 * sp
local z2 = dy * sp + z1 * cp
cam.x = cam.tx + x1
cam.y = cam.ty + y2
cam.z = cam.tz + z2
obj.setoption("camera_param", cam)

@目標サイズ固定視野角
--track0:増減値,-120,120,0,0.01
local cam = obj.getoption("camera_param")
local dx = cam.tx - cam.x
local dy = cam.ty - cam.y
local dz = cam.tz - cam.z
local dist = math.sqrt(dx * dx + dy * dy + dz * dz)
if (dist > 0 and cam.d > 0) then
    local ratio = dist / cam.d
    local fov = math.atan(obj.screen_h / (cam.d * 2)) * 360 / math.pi
    fov = math.min(120, math.max(1, fov + obj.track0))
    local new_d = obj.screen_h / (math.tan(fov * math.pi / 360) * 2)
    local new_dist = ratio * new_d
    cam.x = cam.tx - (dx / dist) * new_dist
    cam.y = cam.ty - (dy / dist) * new_dist
    cam.z = cam.tz - (dz / dist) * new_dist
    cam.d = new_d
    obj.setoption("camera_param", cam)
end

@ParallelCamera@ParallelCamera
--track0:基準距離,1,10000,1024
local cam = obj.getoption("camera_param")
local dx = cam.tx - cam.x
local dy = cam.ty - cam.y
local dz = cam.tz - cam.z
local dist = math.sqrt(dx * dx + dy * dy + dz * dz)
if (dist > 1) then
    cam.d = dist
    obj.setoption("camera_param", cam)
end
"#;

/// 拡張編集互換の標準シーンチェンジスクリプト（クリーンルーム実装）
const BUILTIN_EXEDIT_SCN: &str = r#"@発光
--track0:強さ,0,100,100
local p = obj.getvalue("scenechange")
local w = math.sin(p * math.pi)
obj.effect("グロー", "強さ", w * obj.track0 * 2, "しきい値", 20)
obj.alpha = obj.alpha * (1 - p)

@レンズブラー
--track0:強さ,0,100,50
local p = obj.getvalue("scenechange")
obj.effect("ぼかし", "範囲", p * obj.track0)
obj.alpha = obj.alpha * (1 - p)

@ドア
--track0:角度,-180,180,90
local p = obj.getvalue("scenechange")
obj.ry = obj.ry + p * obj.track0
obj.alpha = obj.alpha * (1 - p)

@起き上がる
--track0:角度,-180,180,90
local p = obj.getvalue("scenechange")
obj.rx = obj.rx + p * obj.track0
obj.alpha = obj.alpha * (1 - p)

@リール回転
--track0:回数,1,20,3,1
local p = obj.getvalue("scenechange")
obj.oy = obj.oy + ((p * obj.track0 * obj.screen_h) % obj.screen_h)
obj.alpha = obj.alpha * (1 - p)

@図形ワイプ
--track0:サイズ,1,500,100
local p = obj.getvalue("scenechange")
obj.alpha = obj.alpha * (1 - p)

@図形で隠す
--track0:サイズ,1,500,100
local p = obj.getvalue("scenechange")
obj.alpha = obj.alpha * (1 - p)

@図形で隠す(放射)
--track0:分割数,2,32,8,1
local p = obj.getvalue("scenechange")
obj.alpha = obj.alpha * (1 - p)

@砕け散る
--track0:サイズ,10,500,80
local p = obj.getvalue("scenechange")
obj.zoom = obj.zoom * (1 + p * 0.5)
obj.alpha = obj.alpha * (1 - p)

@ページめくり
--track0:角度,-180,180,45
local p = obj.getvalue("scenechange")
obj.ox = obj.ox - p * obj.screen_w * 0.5
obj.alpha = obj.alpha * (1 - p)
"#;

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
