//! `.auf` / `.auf2` ネイティブフィルタプラグインの静的パラメータ抽出および
//! 手動移植用ボイラープレート（雛形）生成モジュール。
//!
//! ※ 本モジュールは DLL 内の機械語（`func_proc`）を実行・逆コンパイルするものではなく、
//! プラグイン名から推測したエフェクトを自動注入するものでもありません。
//! PE バイナリ内の `FILTER_DLL` / `FILTER_PLUGIN_TABLE` 構造体から静的に抽出したパラメータ定義（トラックバー・
//! チェックボックス・ダイアログ項目）をそのまま確認・出力するためのインスペクタ＆ボイラープレート生成機能を提供します。

use crate::{BinaryArch, PluginDescriptor, ScriptEntry};

/// `.auf` / `.auf2` プラグインのパラメータ構造および外部 API 依存性の分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortingDifficulty {
    /// 標準の `FILTER_DLL` / `FILTER_PLUGIN_TABLE` パラメータ定義を持つフィルタ（パラメータ定義の静的抽出に対応）
    AutoCompatible,
    /// `USER32.dll` / `GDI32.dll` 等の独自 Win32 GUI/GDI API を参照するプラグイン（基本パラメータ定義のみ抽出可能、独自 HWND UI・DLL 処理は非対応）
    GuiOrGdiAssisted,
    /// AviUtl 本体の内部メモリをフックするシステム拡張プラグイン（`exedit.auf` 本体など・DLL コードの実行には非対応）
    SystemHook,
}

impl PortingDifficulty {
    pub fn badge_label(self) -> &'static str {
        match self {
            Self::AutoCompatible => "パラメータ定義抽出済 (DLLネイティブ処理は未実行)",
            Self::GuiOrGdiAssisted => "Win32 GUI/GDI 依存あり (基本定義のみ抽出 / 独自UI・DLL未実行)",
            Self::SystemHook => "システムフック型 DLL (ネイティブ実行非対応)",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::AutoCompatible => {
                "FILTER_DLL / FILTER_PLUGIN_TABLE 構造体からトラックバー・チェックボックス・ダイアログ定義を静的に抽出しました。DLL 内の機械語 (func_proc) は実行されません。手動でスクリプトやシェーダーへ移植する際のパラメータ定義ボイラープレートを出力できます。"
            }
            Self::GuiOrGdiAssisted => {
                "Win32 のウィンドウ/GDI API (USER32.dll / GDI32.dll) を参照しています。独自ウィンドウ (HWND) の表示や DLL 機械語の実行には非対応ですが、テーブル内の基本パラメータ定義は抽出済みです。"
            }
            Self::SystemHook => {
                "AviUtl 本体の実行メモリを直接フックするシステム拡張プラグインです。DLL 機械語は実行されません。"
            }
        }
    }
}

/// `.auf` / `.auf2` バイナリの静的構造解析レポート。
#[derive(Debug, Clone)]
pub struct AufPortingReport {
    pub file_name: String,
    pub arch: BinaryArch,
    pub sdk_generation: String,
    pub difficulty: PortingDifficulty,
    pub exports: Vec<String>,
    pub imported_dlls: Vec<String>,
    pub filter_flags: u32,
    pub total_tracks: usize,
    pub total_checks: usize,
    pub total_dialogs: usize,
}

impl AufPortingReport {
    pub fn analyze(desc: &PluginDescriptor) -> Self {
        let sdk_generation = if desc.exports.iter().any(|e| e.contains("FilterPluginTable")) {
            "AviUtl2 SDK (FILTER_PLUGIN_TABLE / 64-bit UTF-16LE)".to_string()
        } else if desc.exports.iter().any(|e| e.contains("ScriptModuleTable")) {
            "AviUtl2 SDK (SCRIPT_MODULE_TABLE / .mod2)".to_string()
        } else if desc.arch == BinaryArch::X86_64 {
            "64-bit FILTER_DLL (PE32+ x86_64)".to_string()
        } else if desc.arch == BinaryArch::X86_32 {
            "初代 AviUtl SDK (FILTER_DLL / 32-bit x86)".to_string()
        } else {
            "スクリプト / 汎用プラグイン".to_string()
        };

        let lower_fname = desc.file_name.to_ascii_lowercase();
        let has_gui_dll = desc.imports.iter().any(|d| {
            let l = d.to_ascii_lowercase();
            l.contains("user32") || l.contains("gdi32") || l.contains("comdlg32") || l.contains("shell32")
        });

        let difficulty = if lower_fname == "exedit.auf" || lower_fname.contains("patch.aul") {
            PortingDifficulty::SystemHook
        } else if has_gui_dll {
            PortingDifficulty::GuiOrGdiAssisted
        } else {
            PortingDifficulty::AutoCompatible
        };

        let mut total_tracks = 0;
        let mut total_checks = 0;
        let mut total_dialogs = 0;

        for e in &desc.entries {
            total_tracks += e.tracks.len();
            total_checks += e.checks.len();
            total_dialogs += e.dialogs.len();
        }

        Self {
            file_name: desc.file_name.clone(),
            arch: desc.arch,
            sdk_generation,
            difficulty,
            exports: desc.exports.clone(),
            imported_dlls: desc.imports.clone(),
            filter_flags: desc.filter_flags,
            total_tracks,
            total_checks,
            total_dialogs,
        }
    }
}

fn track_var_expr(entry: &ScriptEntry, idx: usize) -> String {
    if let Some(t) = entry.tracks.get(idx) {
        if let Some(var) = &t.var_name {
            return var.clone();
        }
    }
    if idx < 4 {
        format!("obj.track{idx}")
    } else {
        format!("t{idx}")
    }
}

fn sanitize_lua_ident(raw: &str, fallback: &str) -> String {
    let mut s = String::new();
    for (i, ch) in raw.chars().enumerate() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            if i == 0 && ch.is_ascii_digit() {
                s.push('_');
            }
            s.push(ch);
        }
    }
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

fn step_str_from_precision(precision: u8) -> &'static str {
    match precision {
        0 => "1",
        1 => "0.1",
        2 => "0.01",
        _ => "0.001",
    }
}

fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-6 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v}")
    }
}

/// 単一の [`ScriptEntry`]（`.auf` / `.auf2` から抽出されたパラメータ定義）に対し、
/// 抽出されたトラックバー・チェックボックス変数の受け取りボイラープレートを生成します。
pub fn synthesize_entry_lua_body(entry: &ScriptEntry) -> String {
    let mut lines = Vec::new();
    lines.push("-- 抽出されたパラメータ変数の取得ボイラープレート".to_string());
    for (idx, t) in entry.tracks.iter().enumerate() {
        let expr = track_var_expr(entry, idx);
        lines.push(format!(
            "local track_{idx} = {expr} -- {} (min: {}, max: {}, default: {})",
            t.name,
            fmt_num(t.min),
            fmt_num(t.max),
            fmt_num(t.default)
        ));
    }
    for (idx, c) in entry.checks.iter().enumerate() {
        let expr = if let Some(var) = &c.var_name {
            var.clone()
        } else if idx == 0 {
            "obj.check0".to_string()
        } else {
            format!("chk{idx}")
        };
        lines.push(format!("local check_{idx} = {expr} -- {}", c.name));
    }
    lines.push("-- TODO: 元 DLL の機械語 (func_proc) は自動変換されません。手動でスクリプト処理を実装してください。".to_string());
    lines.join("\n")
}

/// `.auf` / `.auf2` プラグインから静的に抽出したパラメータ定義（トラックバー・チェックボックス・ダイアログ項目）を
/// `.anm` スクリプトヘッダ雛形として出力します。
pub fn generate_anm_script(desc: &PluginDescriptor) -> String {
    let report = AufPortingReport::analyze(desc);
    let mut out = String::new();

    out.push_str(&format!(
        "-- ============================================================================\n\
         -- OpenAvi パラメータ定義ボイラープレート (.anm 雛形)\n\
         -- 元プラグイン  : {} ({})\n\
         -- 検出シンボル  : {}\n\
         -- 解析ステータス: {}\n\
         -- ※ 注意: PE 構造体から静的に抽出したパラメータ定義ヘッダのみを出力しています。\n\
         --    DLL 内のネイティブ機械語 (func_proc) は含まれないため、処理本体は手動で記述してください。\n\
         -- ============================================================================\n\n",
        desc.file_name,
        report.sdk_generation,
        if desc.exports.is_empty() {
            "なし (セクション構造体スキャン)".to_string()
        } else {
            desc.exports.join(", ")
        },
        report.difficulty.badge_label()
    ));

    let multi_section = desc.entries.len() > 1;
    for entry in &desc.entries {
        let sec_name = crate::clean_script_name(&entry.display_name);
        if multi_section || !sec_name.is_empty() {
            out.push_str(&format!("@{sec_name}\n"));
        }
        if !desc.description.is_empty() {
            out.push_str(&format!("--information:{}\n", desc.description.replace('\n', " ")));
        }

        // トラックバー出力 (--track0..3 および 5本目以降は AviUtl2 互換 --track@tN)
        for (idx, t) in entry.tracks.iter().enumerate() {
            let step = step_str_from_precision(t.precision);
            if let Some(var) = &t.var_name {
                out.push_str(&format!(
                    "--track@{var}:{},{},{},{},{step}\n",
                    t.name,
                    fmt_num(t.min),
                    fmt_num(t.max),
                    fmt_num(t.default)
                ));
            } else if idx < 4 {
                out.push_str(&format!(
                    "--track{idx}:{},{},{},{},{step}\n",
                    t.name,
                    fmt_num(t.min),
                    fmt_num(t.max),
                    fmt_num(t.default)
                ));
            } else {
                out.push_str(&format!(
                    "--track@t{idx}:{},{},{},{},{step}\n",
                    t.name,
                    fmt_num(t.min),
                    fmt_num(t.max),
                    fmt_num(t.default)
                ));
            }
        }

        // チェックボックス出力
        for (idx, c) in entry.checks.iter().enumerate() {
            let def_int = if c.default { 1 } else { 0 };
            if let Some(var) = &c.var_name {
                out.push_str(&format!("--check@{var}:{}, {def_int}\n", c.name));
            } else if idx == 0 {
                out.push_str(&format!("--check0:{},{def_int}\n", c.name));
            } else {
                out.push_str(&format!("--check@chk{idx}:{},{def_int}\n", c.name));
            }
        }

        // ダイアログパラメータ出力 (--dialog:)
        if !entry.dialogs.is_empty() {
            let mut dlg_items = Vec::new();
            for (idx, d) in entry.dialogs.iter().enumerate() {
                let vname = sanitize_lua_ident(&d.var_name, &format!("col{idx}"));
                let is_color = d.label.contains("色")
                    || d.label.to_lowercase().contains("color")
                    || d.default_value.starts_with("0x");
                if is_color {
                    dlg_items.push(format!("{}/col,local {vname}={}", d.label, d.default_value));
                } else if d.is_number {
                    dlg_items.push(format!("{},local {vname}={}", d.label, d.default_value));
                } else {
                    dlg_items.push(format!("{},local {vname}=\"{}\"", d.label, d.default_value));
                }
            }
            out.push_str(&format!("--dialog:{};\n", dlg_items.join(";")));
        }

        out.push('\n');
        let body = if !entry.script_body.trim().is_empty() {
            entry.script_body.trim().to_string()
        } else {
            synthesize_entry_lua_body(entry)
        };
        out.push_str(&body);
        out.push_str("\n\n");
    }

    out
}

/// `.auf` / `.auf2` のパラメータ定義から、WebGPU / wgpu 用の Uniform 構造体ボイラープレート (WGSL) を生成します。
pub fn generate_wgsl_shader(desc: &PluginDescriptor) -> String {
    let entry = desc.entries.first();
    let plugin_title = entry
        .map(|e| e.display_name.as_str())
        .unwrap_or(desc.file_name.as_str());

    let mut struct_fields = Vec::new();
    if let Some(e) = entry {
        for (i, t) in e.tracks.iter().enumerate() {
            struct_fields.push(format!(
                "    // Track {i}: {} (min: {}, max: {}, default: {})\n    track_{i}: f32,",
                t.name,
                fmt_num(t.min),
                fmt_num(t.max),
                fmt_num(t.default)
            ));
        }
        for (i, c) in e.checks.iter().enumerate() {
            struct_fields.push(format!(
                "    // Check {i}: {} (default: {})\n    check_{i}: u32,",
                c.name, c.default
            ));
        }
        for (i, d) in e.dialogs.iter().enumerate() {
            struct_fields.push(format!(
                "    // Dialog {i}: {} (default: {})\n    param_{i}: vec4<f32>,",
                d.label, d.default_value
            ));
        }
    }
    if struct_fields.is_empty() {
        struct_fields.push("    track_0: f32,".to_string());
    }
    struct_fields.push("    resolution: vec2<f32>,".to_string());
    struct_fields.push("    time: f32,".to_string());

    format!(
        "// ============================================================================\n\
         // OpenAvi パラメータ定義 Uniform ボイラープレート (WGSL 雛形)\n\
         // 対象プラグイン: {plugin_title} ({})\n\
         // ※ パラメータ構造体のみ抽出しています。フラグメントシェーダー処理は手動で実装してください。\n\
         // ============================================================================\n\n\
         struct AufParams {{\n\
         {}\n\
         }};\n\n\
         @group(0) @binding(0) var src_tex: texture_2d<f32>;\n\
         @group(0) @binding(1) var src_smp: sampler;\n\
         @group(0) @binding(2) var<uniform> params: AufParams;\n\n\
         struct VertexOutput {{\n\
         \x20   @builtin(position) clip_pos: vec4<f32>,\n\
         \x20   @location(0) uv: vec2<f32>,\n\
         }};\n\n\
         @fragment\n\
         fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {{\n\
         \x20   let base = textureSample(src_tex, src_smp, in.uv);\n\
         \x20   // TODO: ここにシェーダー処理を実装してください\n\
         \x20   return base;\n\
         }}\n",
        desc.file_name,
        struct_fields.join("\n")
    )
}

/// `.auf` / `.auf2` のパラメータ定義から、Rust のパラメータ評価ボイラープレートコードを生成します。
pub fn generate_rust_effect_code(desc: &PluginDescriptor) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "// ============================================================================\n\
         // OpenAvi パラメータ取得ボイラープレート (Rust 雛形)\n\
         // 元プラグイン: {} ({})\n\
         // ============================================================================\n\n\
         use au2_core::{{Effect, Frame}};\n\n",
        desc.file_name,
        desc.arch.description()
    ));

    for (idx, entry) in desc.entries.iter().enumerate() {
        let fn_suffix = if desc.entries.len() == 1 {
            String::new()
        } else {
            format!("_{idx}")
        };
        out.push_str(&format!(
            "/// `{}` のパラメータ評価ボイラープレート\n\
             pub fn evaluate_auf_filter{fn_suffix}(eff: &Effect, rel: Frame) {{\n",
            entry.display_name
        ));
        for (ti, t) in entry.tracks.iter().enumerate() {
            out.push_str(&format!(
                "    // Track {ti}: {} (範囲: {}..={}, 初期値: {})\n\
                 \x20   let _track_{ti} = eff.eval({:?}, rel, {:.1}).clamp({:.1}, {:.1});\n",
                t.name, t.min, t.max, t.default, t.name, t.default, t.min, t.max
            ));
        }
        for (ci, c) in entry.checks.iter().enumerate() {
            let def_num = if c.default { 1.0 } else { 0.0 };
            out.push_str(&format!(
                "    // Check {ci}: {} (初期値: {})\n\
                 \x20   let _check_{ci} = eff.eval({:?}, rel, {:.1}) >= 0.5;\n",
                c.name, c.default, c.name, def_num
            ));
        }
        for (di, d) in entry.dialogs.iter().enumerate() {
            out.push_str(&format!(
                "    // Dialog {di}: {}\n\
                 \x20   let _dialog_{di} = eff.text({:?}).unwrap_or({:?});\n",
                d.label, d.label, d.default_value
            ));
        }
        out.push_str("    // TODO: 描画・ピクセル処理を実装してください\n}\n\n");
    }

    out
}

/// `.auf` / `.auf2` から抽出した全構造体メタデータ・エクスポート・インポート・パラメータ定義を JSON 文字列として返します。
pub fn generate_json_spec(desc: &PluginDescriptor) -> String {
    let report = AufPortingReport::analyze(desc);
    let mut entries_json = Vec::new();
    for e in &desc.entries {
        let tracks_json: Vec<String> = e
            .tracks
            .iter()
            .map(|t| {
                format!(
                    "        {{ \"index\": {}, \"name\": {:?}, \"min\": {}, \"max\": {}, \"default\": {}, \"precision\": {} }}",
                    t.index, t.name, t.min, t.max, t.default, t.precision
                )
            })
            .collect();
        let checks_json: Vec<String> = e
            .checks
            .iter()
            .map(|c| format!("        {{ \"name\": {:?}, \"default\": {} }}", c.name, c.default))
            .collect();
        let dialogs_json: Vec<String> = e
            .dialogs
            .iter()
            .map(|d| {
                format!(
                    "        {{ \"label\": {:?}, \"var_name\": {:?}, \"default_value\": {:?}, \"is_number\": {} }}",
                    d.label, d.var_name, d.default_value, d.is_number
                )
            })
            .collect();

        entries_json.push(format!(
            "    {{\n\
             \x20     \"display_name\": {:?},\n\
             \x20     \"tracks\": [\n{}\n      ],\n\
             \x20     \"checks\": [\n{}\n      ],\n\
             \x20     \"dialogs\": [\n{}\n      ]\n\
             \x20   }}",
            e.display_name,
            tracks_json.join(",\n"),
            checks_json.join(",\n"),
            dialogs_json.join(",\n")
        ));
    }

    format!(
        "{{\n\
         \x20 \"file_name\": {:?},\n\
         \x20 \"arch\": {:?},\n\
         \x20 \"sdk_generation\": {:?},\n\
         \x20 \"status\": {:?},\n\
         \x20 \"filter_flags\": \"0x{:08X}\",\n\
         \x20 \"exports\": {:?},\n\
         \x20 \"imported_dlls\": {:?},\n\
         \x20 \"entries\": [\n{}\n  ]\n\
         }}\n",
        desc.file_name,
        desc.arch.label(),
        report.sdk_generation,
        report.difficulty.badge_label(),
        desc.filter_flags,
        desc.exports,
        desc.imports,
        entries_json.join(",\n")
    )
}
