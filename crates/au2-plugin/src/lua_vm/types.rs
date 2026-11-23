//! Lua サブセット VM のデータ型（LuaObjState, LuaVal, AST ノード）。

use std::collections::HashMap;

/// `obj.draw(...)` による追加描画インスタンス。
#[derive(Debug, Clone, PartialEq)]
pub struct LuaDrawCall {
    pub ox: f64,
    pub oy: f64,
    pub oz: f64,
    pub zoom: f64,
    pub alpha: f64,
    pub rx: f64,
    pub ry: f64,
    pub rz: f64,
    pub color: Option<u32>,
    pub primitive: Option<LuaLoadedPrimitive>,
    /// `[上, 下, 左, 右]` のクリッピング量（ピクセル単位）
    pub clip: Option<[f64; 4]>,
}

/// `obj.drawpoly(...)` による四角形ポリゴン描画。
#[derive(Debug, Clone, PartialEq)]
pub struct LuaPolyCall {
    pub pts: [(f64, f64, f64); 4],
    pub alpha: f64,
    pub color: Option<u32>,
}

/// `obj.effect(...)` によるスクリプト内フィルタ効果呼び出し。
#[derive(Debug, Clone, PartialEq)]
pub struct LuaFilterCall {
    pub name: String,
    pub params: HashMap<String, LuaVal>,
}

/// `obj.load(...)` または `obj.mes(...)` で生成されたプリミティブ。
#[derive(Debug, Clone, PartialEq)]
pub enum LuaLoadedPrimitive {
    Figure {
        fig_type: String,
        color: u32,
        size: f64,
        line_width: f64,
    },
    Text {
        text: String,
        font: String,
        size: f64,
        color: u32,
        color2: u32,
    },
    Image {
        path: String,
    },
}

/// `obj.copybuffer` などで利用される仮想画像・描画状態バッファ。
///
/// プラットフォーム固有の Win32 GDI / DirectX への依存を排除し、
/// 純粋な Rust データ構造として保持されるため、Mac / Linux / Windows すべてで共通動作します。
#[derive(Debug, Clone, PartialEq)]
pub struct LuaVirtualBuffer {
    pub w: f64,
    pub h: f64,
    pub loaded_primitive: Option<LuaLoadedPrimitive>,
    pub draw_calls: Vec<LuaDrawCall>,
    pub poly_calls: Vec<LuaPolyCall>,
    pub filter_calls: Vec<LuaFilterCall>,
    pub clip: Option<[f64; 4]>,
    pub zoom: f64,
    pub alpha: f64,
}

/// スクリプト実行前後の `obj` テーブル状態。
#[derive(Debug, Clone)]
pub struct LuaObjState {
    pub ox: f64,
    pub oy: f64,
    pub oz: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub rx: f64,
    pub ry: f64,
    pub rz: f64,
    pub cx: f64,
    pub cy: f64,
    pub cz: f64,
    pub zoom: f64,
    /// AviUtl2 の X軸拡大率 (1.0 基準)
    pub sx: f64,
    /// AviUtl2 の Y軸拡大率 (1.0 基準)
    pub sy: f64,
    /// AviUtl2 の Z軸拡大率 (1.0 基準)
    pub sz: f64,
    pub alpha: f64,
    pub aspect: f64,
    pub time: f64,
    pub totaltime: f64,
    pub frame: i64,
    pub totalframe: i64,
    pub layer: i64,
    pub index: i64,
    pub num: i64,
    pub id: i64,
    pub effect_id: i64,
    pub frame_s: i64,
    pub frame_e: i64,
    pub effect_layer: i64,
    pub originframe: i64,
    pub scene_id: i64,
    pub w: f64,
    pub h: f64,
    pub screen_w: f64,
    pub screen_h: f64,
    pub framerate: f64,
    pub tracks: [f64; 4],
    pub check0: bool,
    pub named_vars: HashMap<String, LuaVal>,
    pub font_name: String,
    pub font_size: f64,
    pub font_type: i64,
    pub font_color: u32,
    pub font_color2: u32,
    pub font_bold: bool,
    pub font_italic: bool,
    pub font_charspacing: f64,
    pub font_linespacing: f64,
    pub loaded_primitive: Option<LuaLoadedPrimitive>,
    pub draw_calls: Vec<LuaDrawCall>,
    pub poly_calls: Vec<LuaPolyCall>,
    pub filter_calls: Vec<LuaFilterCall>,
    pub blend_mode: i64,
    pub culling: bool,
    pub billboard: i64,
    pub current_clip: Option<[f64; 4]>,
    pub cam_tx: f64,
    pub cam_ty: f64,
    pub cam_tz: f64,
    pub cam_ux: f64,
    pub cam_uy: f64,
    pub cam_uz: f64,
    pub cam_d: f64,
    /// `.tra` / `.tra2` 移動スクリプト用の制御点リスト (`getpoint` 用)
    pub point_values: Vec<f64>,
    pub point_index: usize,
    pub point_time: f64,
    pub point_totaltime: f64,
    pub point_param: f64,
    pub point_accelerate: bool,
    pub point_decelerate: bool,
    /// `.tra` スクリプト等が `return` した数値結果
    pub return_value: Option<f64>,
    /// `obj.copybuffer` などで利用される仮想画像・描画状態バッファプール ("cache", "tmp", "cache:名前" 等)
    pub buffers: HashMap<String, LuaVirtualBuffer>,
    /// ピクセル補間法設定 ("nearest", "bilinear", "bicubic" 等)
    pub pixel_option: String,
}

impl LuaObjState {
    /// 現在のオブジェクトの描画・フィルタ状態を仮想バッファとして抽出します。
    pub fn snapshot_to_buffer(&self) -> LuaVirtualBuffer {
        LuaVirtualBuffer {
            w: self.w,
            h: self.h,
            loaded_primitive: self.loaded_primitive.clone(),
            draw_calls: self.draw_calls.clone(),
            poly_calls: self.poly_calls.clone(),
            filter_calls: self.filter_calls.clone(),
            clip: self.current_clip,
            zoom: self.zoom,
            alpha: self.alpha,
        }
    }

    /// 仮想バッファの状態を現在のオブジェクトに復元・適用します。
    pub fn restore_from_buffer(&mut self, buf: &LuaVirtualBuffer) {
        self.w = buf.w;
        self.h = buf.h;
        self.loaded_primitive = buf.loaded_primitive.clone();
        self.draw_calls = buf.draw_calls.clone();
        self.poly_calls = buf.poly_calls.clone();
        self.filter_calls = buf.filter_calls.clone();
        self.current_clip = buf.clip;
        self.zoom = buf.zoom;
        self.alpha = buf.alpha;
    }
}

impl Default for LuaObjState {
    fn default() -> Self {
        Self {
            ox: 0.0,
            oy: 0.0,
            oz: 0.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            rx: 0.0,
            ry: 0.0,
            rz: 0.0,
            cx: 0.0,
            cy: 0.0,
            cz: 0.0,
            zoom: 1.0,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            alpha: 1.0,
            aspect: 0.0,
            time: 0.0,
            totaltime: 5.0,
            frame: 0,
            totalframe: 150,
            layer: 1,
            index: 0,
            num: 1,
            id: 1,
            effect_id: 1,
            frame_s: 0,
            frame_e: 150,
            effect_layer: 1,
            originframe: 0,
            scene_id: 0,
            w: 200.0,
            h: 200.0,
            screen_w: 1920.0,
            screen_h: 1080.0,
            framerate: 30.0,
            tracks: [0.0; 4],
            check0: true,
            named_vars: HashMap::new(),
            font_name: "MS UI Gothic".to_string(),
            font_size: 36.0,
            font_type: 0,
            font_color: 0xFFFFFF,
            font_color2: 0x000000,
            font_bold: false,
            font_italic: false,
            font_charspacing: 0.0,
            font_linespacing: 0.0,
            loaded_primitive: None,
            draw_calls: Vec::new(),
            poly_calls: Vec::new(),
            filter_calls: Vec::new(),
            blend_mode: 0,
            culling: false,
            billboard: 0,
            current_clip: None,
            cam_tx: 0.0,
            cam_ty: 0.0,
            cam_tz: 0.0,
            cam_ux: 0.0,
            cam_uy: -1.0,
            cam_uz: 0.0,
            cam_d: 1024.0,
            point_values: vec![0.0, 100.0],
            point_index: 0,
            point_time: 0.5,
            point_totaltime: 1.0,
            point_param: 0.0,
            point_accelerate: false,
            point_decelerate: false,
            return_value: None,
            buffers: HashMap::new(),
            pixel_option: "bilinear".to_string(),
        }
    }
}

/// Lua の動的型値。
#[derive(Debug, Clone, PartialEq)]
pub enum LuaVal {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    Table(HashMap<String, LuaVal>, Vec<LuaVal>),
    Func( Vec<String>, Vec<Stmt>),
}

impl LuaVal {
    pub fn truthy(&self) -> bool {
        !matches!(self, Self::Nil | Self::Bool(false))
    }

    pub fn as_num(&self) -> f64 {
        match self {
            Self::Num(n) => *n,
            Self::Bool(b) => if *b { 1.0 } else { 0.0 },
            Self::Str(s) => {
                let t = s.trim();
                if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                    u64::from_str_radix(hex, 16).map(|v| v as f64).unwrap_or(0.0)
                } else {
                    t.parse::<f64>().unwrap_or(0.0)
                }
            }
            _ => 0.0,
        }
    }

    pub fn as_str(&self) -> String {
        match self {
            Self::Nil => "nil".into(),
            Self::Bool(b) => b.to_string(),
            Self::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            Self::Str(s) => s.clone(),
            Self::Table(_, _) => "table".into(),
            Self::Func(_, _) => "function".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    Var(String),
    Table(Vec<(Option<Expr>, Expr)>),
    UnaryOp(UnaryOp, Box<Expr>),
    BinOp(BinOp, Box<Expr>, Box<Expr>),
    Field(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    MethodCall(Box<Expr>, String, Vec<Expr>),
    AnonFunc(Vec<String>, Vec<Stmt>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    Len,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Concat,
    Eq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LValue {
    Var(String),
    Field(Expr, String),
    Index(Expr, Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Local(Vec<String>, Vec<Expr>),
    Assign(Vec<LValue>, Vec<Expr>),
    If {
        branches: Vec<(Expr, Vec<Stmt>)>,
        else_body: Vec<Stmt>,
    },
    While(Expr, Vec<Stmt>),
    Repeat(Vec<Stmt>, Expr),
    ForNum {
        var: String,
        start: Expr,
        limit: Expr,
        step: Option<Expr>,
        body: Vec<Stmt>,
    },
    ForIn {
        vars: Vec<String>,
        iter: Expr,
        body: Vec<Stmt>,
    },
    FuncDef {
        is_local: bool,
        name: String,
        field: Option<String>,
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    Do(Vec<Stmt>),
    Return(Vec<Expr>),
    Break,
    ExprStmt(Expr),
}