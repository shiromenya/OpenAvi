//! AviUtl (初代拡張編集) および AviUtl2 (ExEdit2) 互換の組み込み Lua スクリプト実行エンジン。
//!
//! `.anm`, `.obj`, `.cam`, `.scn`, `.tra`（および `.anm2`, `.obj2`, `.cam2`, `.scn2`, `.tra2`）内の
//! Lua スクリプトをサンドボックス内で高速に評価し、`obj.*` プロパティ変更・`obj.draw()`・
//! `obj.drawpoly()`・`obj.load()`・`obj.mes()`・`obj.effect()` 等の描画コマンドを生成します。

use std::collections::HashMap;
use std::f64::consts::PI;

/// 1 回のスクリプト実行における最大命令ステップ数（無限ループ防止用）。
const MAX_STEPS: usize = 50_000;

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
    pub alpha: f64,
    pub aspect: f64,
    pub time: f64,
    pub totaltime: f64,
    pub frame: i64,
    pub totalframe: i64,
    pub layer: i64,
    pub index: i64,
    pub num: i64,
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
    pub font_color: u32,
    pub font_color2: u32,
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
    /// `.tra` スクリプト等が `return` した数値結果
    pub return_value: Option<f64>,
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
            alpha: 1.0,
            aspect: 0.0,
            time: 0.0,
            totaltime: 5.0,
            frame: 0,
            totalframe: 150,
            layer: 1,
            index: 0,
            num: 1,
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
            font_color: 0xFFFFFF,
            font_color2: 0x000000,
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
            return_value: None,
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

// ============================================================================
// Lexer
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    // Keywords
    Local,
    If,
    Then,
    ElseIf,
    Else,
    End,
    While,
    Do,
    For,
    In,
    Repeat,
    Until,
    Function,
    Return,
    Break,
    And,
    Or,
    Not,
    True,
    False,
    Nil,
    // Symbols
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Hash,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Assign,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semi,
    Colon,
    Comma,
    Dot,
    DotDot,
    Eof,
}

fn tokenize(src: &str) -> Vec<Tok> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // Comments (`--`)
        if c == '-' && i + 1 < n && chars[i + 1] == '-' {
            i += 2;
            // Multi-line comment `--[[ ... ]]`
            if i + 1 < n && chars[i] == '[' && chars[i + 1] == '[' {
                i += 2;
                while i + 1 < n && !(chars[i] == ']' && chars[i + 1] == ']') {
                    i += 1;
                }
                i = (i + 2).min(n);
            } else {
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
            }
            continue;
        }
        // Preprocessor / directive lines starting with `@` at line start
        if c == '@' {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // Multi-line strings `[[ ... ]]`
        if c == '[' && i + 1 < n && chars[i + 1] == '[' {
            i += 2;
            let start = i;
            while i + 1 < n && !(chars[i] == ']' && chars[i + 1] == ']') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            tokens.push(Tok::Str(s));
            i = (i + 2).min(n);
            continue;
        }
        // Quoted strings
        if c == '"' || c == '\'' {
            let quote = c;
            i += 1;
            let mut s = String::new();
            while i < n && chars[i] != quote {
                if chars[i] == '\\' && i + 1 < n {
                    i += 1;
                    match chars[i] {
                        'n' => s.push('\n'),
                        'r' => s.push('\r'),
                        't' => s.push('\t'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        '\'' => s.push('\''),
                        other => s.push(other),
                    }
                } else {
                    s.push(chars[i]);
                }
                i += 1;
            }
            if i < n {
                i += 1;
            }
            tokens.push(Tok::Str(s));
            continue;
        }
        // Numbers (decimal & hex `0x...`)
        if c.is_ascii_digit() || (c == '.' && i + 1 < n && chars[i + 1].is_ascii_digit()) {
            if c == '0' && i + 1 < n && (chars[i + 1] == 'x' || chars[i + 1] == 'X') {
                i += 2;
                let start = i;
                while i < n && chars[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let hex_str: String = chars[start..i].iter().collect();
                let val = u64::from_str_radix(&hex_str, 16).unwrap_or(0) as f64;
                tokens.push(Tok::Num(val));
            } else {
                let start = i;
                while i < n && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                if i < n && (chars[i] == 'e' || chars[i] == 'E') {
                    i += 1;
                    if i < n && (chars[i] == '+' || chars[i] == '-') {
                        i += 1;
                    }
                    while i < n && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let num_str: String = chars[start..i].iter().collect();
                let val = num_str.parse::<f64>().unwrap_or(0.0);
                tokens.push(Tok::Num(val));
            }
            continue;
        }
        // Identifiers & keywords (including Japanese identifiers used in AviUtl dialogs)
        if c.is_alphabetic() || c == '_' || (c as u32) >= 0x80 {
            let start = i;
            while i < n && (chars[i].is_alphanumeric() || chars[i] == '_' || (chars[i] as u32) >= 0x80) {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();
            let tok = match ident.as_str() {
                "local" => Tok::Local,
                "if" => Tok::If,
                "then" => Tok::Then,
                "elseif" => Tok::ElseIf,
                "else" => Tok::Else,
                "end" => Tok::End,
                "while" => Tok::While,
                "do" => Tok::Do,
                "for" => Tok::For,
                "in" => Tok::In,
                "repeat" => Tok::Repeat,
                "until" => Tok::Until,
                "function" => Tok::Function,
                "return" => Tok::Return,
                "break" => Tok::Break,
                "and" => Tok::And,
                "or" => Tok::Or,
                "not" => Tok::Not,
                "true" => Tok::True,
                "false" => Tok::False,
                "nil" => Tok::Nil,
                _ => Tok::Ident(ident),
            };
            tokens.push(tok);
            continue;
        }
        // Two-character operators
        if i + 1 < n {
            let two = (c, chars[i + 1]);
            let matched = match two {
                ('=', '=') => Some(Tok::EqEq),
                ('~', '=') | ('!', '=') => Some(Tok::NotEq),
                ('<', '=') => Some(Tok::Le),
                ('>', '=') => Some(Tok::Ge),
                ('.', '.') => Some(Tok::DotDot),
                _ => None,
            };
            if let Some(t) = matched {
                tokens.push(t);
                i += 2;
                continue;
            }
        }
        // Single-character symbols
        let single = match c {
            '+' => Some(Tok::Plus),
            '-' => Some(Tok::Minus),
            '*' => Some(Tok::Star),
            '/' => Some(Tok::Slash),
            '%' => Some(Tok::Percent),
            '^' => Some(Tok::Caret),
            '#' => Some(Tok::Hash),
            '=' => Some(Tok::Assign),
            '<' => Some(Tok::Lt),
            '>' => Some(Tok::Gt),
            '(' => Some(Tok::LParen),
            ')' => Some(Tok::RParen),
            '{' => Some(Tok::LBrace),
            '}' => Some(Tok::RBrace),
            '[' => Some(Tok::LBracket),
            ']' => Some(Tok::RBracket),
            ';' => Some(Tok::Semi),
            ':' => Some(Tok::Colon),
            ',' => Some(Tok::Comma),
            '.' => Some(Tok::Dot),
            _ => None,
        };
        if let Some(t) = single {
            tokens.push(t);
        }
        i += 1;
    }

    tokens.push(Tok::Eof);
    tokens
}

// ============================================================================
// AST & Parser
// ============================================================================

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

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Tok>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Tok {
        self.tokens.get(self.pos).unwrap_or(&Tok::Eof)
    }

    fn next(&mut self) -> Tok {
        let t = self.peek().clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, expected: &Tok) -> bool {
        if self.peek() == expected {
            self.next();
            true
        } else {
            false
        }
    }

    fn is_block_end(&self) -> bool {
        matches!(
            self.peek(),
            Tok::End | Tok::Else | Tok::ElseIf | Tok::Until | Tok::Eof
        )
    }

    fn parse_block(&mut self) -> Vec<Stmt> {
        let mut stmts = Vec::new();
        while !self.is_block_end() {
            if self.eat(&Tok::Semi) {
                continue;
            }
            if let Some(s) = self.parse_stmt() {
                stmts.push(s);
            } else {
                // Error recovery: advance 1 token to prevent infinite loop
                self.next();
            }
        }
        stmts
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        match self.peek().clone() {
            Tok::Local => {
                self.next();
                if self.eat(&Tok::Function) {
                    if let Tok::Ident(name) = self.next() {
                        let (params, body) = self.parse_func_body();
                        return Some(Stmt::FuncDef {
                            is_local: true,
                            name,
                            field: None,
                            params,
                            body,
                        });
                    }
                    return None;
                }
                let mut names = Vec::new();
                while let Tok::Ident(name) = self.peek().clone() {
                    self.next();
                    names.push(name);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                let mut exprs = Vec::new();
                if self.eat(&Tok::Assign) {
                    exprs = self.parse_expr_list();
                }
                Some(Stmt::Local(names, exprs))
            }
            Tok::Function => {
                self.next();
                if let Tok::Ident(name) = self.next() {
                    let field = if self.eat(&Tok::Dot) {
                        if let Tok::Ident(f) = self.next() {
                            Some(f)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let (params, body) = self.parse_func_body();
                    Some(Stmt::FuncDef {
                        is_local: false,
                        name,
                        field,
                        params,
                        body,
                    })
                } else {
                    None
                }
            }
            Tok::If => {
                self.next();
                let cond = self.parse_expr()?;
                self.eat(&Tok::Then);
                let then_body = self.parse_block();
                let mut branches = vec![(cond, then_body)];
                while self.eat(&Tok::ElseIf) {
                    let c = self.parse_expr()?;
                    self.eat(&Tok::Then);
                    let b = self.parse_block();
                    branches.push((c, b));
                }
                let else_body = if self.eat(&Tok::Else) {
                    self.parse_block()
                } else {
                    Vec::new()
                };
                self.eat(&Tok::End);
                Some(Stmt::If { branches, else_body })
            }
            Tok::While => {
                self.next();
                let cond = self.parse_expr()?;
                self.eat(&Tok::Do);
                let body = self.parse_block();
                self.eat(&Tok::End);
                Some(Stmt::While(cond, body))
            }
            Tok::Repeat => {
                self.next();
                let body = self.parse_block();
                self.eat(&Tok::Until);
                let cond = self.parse_expr()?;
                Some(Stmt::Repeat(body, cond))
            }
            Tok::Do => {
                self.next();
                let body = self.parse_block();
                self.eat(&Tok::End);
                Some(Stmt::Do(body))
            }
            Tok::For => {
                self.next();
                let Tok::Ident(first_var) = self.next() else {
                    return None;
                };
                if self.eat(&Tok::Assign) {
                    let start = self.parse_expr()?;
                    self.eat(&Tok::Comma);
                    let limit = self.parse_expr()?;
                    let step = if self.eat(&Tok::Comma) {
                        Some(self.parse_expr()?)
                    } else {
                        None
                    };
                    self.eat(&Tok::Do);
                    let body = self.parse_block();
                    self.eat(&Tok::End);
                    Some(Stmt::ForNum {
                        var: first_var,
                        start,
                        limit,
                        step,
                        body,
                    })
                } else {
                    let mut vars = vec![first_var];
                    while self.eat(&Tok::Comma) {
                        if let Tok::Ident(v) = self.next() {
                            vars.push(v);
                        }
                    }
                    self.eat(&Tok::In);
                    let iter = self.parse_expr()?;
                    self.eat(&Tok::Do);
                    let body = self.parse_block();
                    self.eat(&Tok::End);
                    Some(Stmt::ForIn { vars, iter, body })
                }
            }
            Tok::Return => {
                self.next();
                let exprs = if self.is_block_end() || matches!(self.peek(), Tok::Semi) {
                    Vec::new()
                } else {
                    self.parse_expr_list()
                };
                Some(Stmt::Return(exprs))
            }
            Tok::Break => {
                self.next();
                Some(Stmt::Break)
            }
            _ => {
                let expr = self.parse_expr()?;
                // Check if assignment `lval1, lval2 = ...`
                if matches!(self.peek(), Tok::Assign | Tok::Comma) {
                    let mut lvals = vec![expr_to_lvalue(expr)?];
                    while self.eat(&Tok::Comma) {
                        let e = self.parse_expr()?;
                        lvals.push(expr_to_lvalue(e)?);
                    }
                    self.eat(&Tok::Assign);
                    let rvals = self.parse_expr_list();
                    Some(Stmt::Assign(lvals, rvals))
                } else {
                    Some(Stmt::ExprStmt(expr))
                }
            }
        }
    }

    fn parse_func_body(&mut self) -> (Vec<String>, Vec<Stmt>) {
        self.eat(&Tok::LParen);
        let mut params = Vec::new();
        while let Tok::Ident(p) = self.peek().clone() {
            self.next();
            params.push(p);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.eat(&Tok::RParen);
        let body = self.parse_block();
        self.eat(&Tok::End);
        (params, body)
    }

    fn parse_expr_list(&mut self) -> Vec<Expr> {
        let mut list = Vec::new();
        if let Some(first) = self.parse_expr() {
            list.push(first);
            while self.eat(&Tok::Comma) {
                if let Some(e) = self.parse_expr() {
                    list.push(e);
                } else {
                    break;
                }
            }
        }
        list
    }

    fn parse_expr(&mut self) -> Option<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Option<Expr> {
        let mut left = self.parse_and()?;
        while self.eat(&Tok::Or) {
            let right = self.parse_and()?;
            left = Expr::BinOp(BinOp::Or, Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn parse_and(&mut self) -> Option<Expr> {
        let mut left = self.parse_cmp()?;
        while self.eat(&Tok::And) {
            let right = self.parse_cmp()?;
            left = Expr::BinOp(BinOp::And, Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn parse_cmp(&mut self) -> Option<Expr> {
        let mut left = self.parse_concat()?;
        loop {
            let op = match self.peek() {
                Tok::EqEq => BinOp::Eq,
                Tok::NotEq => BinOp::NotEq,
                Tok::Lt => BinOp::Lt,
                Tok::Le => BinOp::Le,
                Tok::Gt => BinOp::Gt,
                Tok::Ge => BinOp::Ge,
                _ => break,
            };
            self.next();
            let right = self.parse_concat()?;
            left = Expr::BinOp(op, Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn parse_concat(&mut self) -> Option<Expr> {
        let left = self.parse_add()?;
        if self.eat(&Tok::DotDot) {
            let right = self.parse_concat()?;
            Some(Expr::BinOp(BinOp::Concat, Box::new(left), Box::new(right)))
        } else {
            Some(left)
        }
    }

    fn parse_add(&mut self) -> Option<Expr> {
        let mut left = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            self.next();
            let right = self.parse_mul()?;
            left = Expr::BinOp(op, Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn parse_mul(&mut self) -> Option<Expr> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Mod,
                _ => break,
            };
            self.next();
            let right = self.parse_unary()?;
            left = Expr::BinOp(op, Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn parse_unary(&mut self) -> Option<Expr> {
        match self.peek() {
            Tok::Minus => {
                self.next();
                Some(Expr::UnaryOp(UnaryOp::Neg, Box::new(self.parse_unary()?)))
            }
            Tok::Not => {
                self.next();
                Some(Expr::UnaryOp(UnaryOp::Not, Box::new(self.parse_unary()?)))
            }
            Tok::Hash => {
                self.next();
                Some(Expr::UnaryOp(UnaryOp::Len, Box::new(self.parse_unary()?)))
            }
            _ => self.parse_pow(),
        }
    }

    fn parse_pow(&mut self) -> Option<Expr> {
        let left = self.parse_postfix()?;
        if self.eat(&Tok::Caret) {
            let right = self.parse_unary()?;
            Some(Expr::BinOp(BinOp::Pow, Box::new(left), Box::new(right)))
        } else {
            Some(left)
        }
    }

    fn parse_postfix(&mut self) -> Option<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek().clone() {
                Tok::Dot => {
                    self.next();
                    if let Tok::Ident(field) = self.next() {
                        expr = Expr::Field(Box::new(expr), field);
                    } else {
                        break;
                    }
                }
                Tok::LBracket => {
                    self.next();
                    let idx = self.parse_expr()?;
                    self.eat(&Tok::RBracket);
                    expr = Expr::Index(Box::new(expr), Box::new(idx));
                }
                Tok::Colon => {
                    self.next();
                    if let Tok::Ident(method) = self.next() {
                        let args = self.parse_call_args()?;
                        expr = Expr::MethodCall(Box::new(expr), method, args);
                    } else {
                        break;
                    }
                }
                Tok::LParen | Tok::Str(_) | Tok::LBrace => {
                    let args = self.parse_call_args()?;
                    expr = Expr::Call(Box::new(expr), args);
                }
                _ => break,
            }
        }
        Some(expr)
    }

    fn parse_call_args(&mut self) -> Option<Vec<Expr>> {
        match self.peek().clone() {
            Tok::LParen => {
                self.next();
                let args = if self.eat(&Tok::RParen) {
                    Vec::new()
                } else {
                    let list = self.parse_expr_list();
                    self.eat(&Tok::RParen);
                    list
                };
                Some(args)
            }
            Tok::Str(s) => {
                self.next();
                Some(vec![Expr::Str(s)])
            }
            Tok::LBrace => {
                let tbl = self.parse_primary()?;
                Some(vec![tbl])
            }
            _ => None,
        }
    }

    fn parse_primary(&mut self) -> Option<Expr> {
        match self.peek().clone() {
            Tok::Nil => {
                self.next();
                Some(Expr::Nil)
            }
            Tok::True => {
                self.next();
                Some(Expr::Bool(true))
            }
            Tok::False => {
                self.next();
                Some(Expr::Bool(false))
            }
            Tok::Num(n) => {
                self.next();
                Some(Expr::Num(n))
            }
            Tok::Str(s) => {
                self.next();
                Some(Expr::Str(s))
            }
            Tok::Ident(name) => {
                self.next();
                Some(Expr::Var(name))
            }
            Tok::Function => {
                self.next();
                let (params, body) = self.parse_func_body();
                Some(Expr::AnonFunc(params, body))
            }
            Tok::LParen => {
                self.next();
                let e = self.parse_expr()?;
                self.eat(&Tok::RParen);
                Some(e)
            }
            Tok::LBrace => {
                self.next();
                let mut entries = Vec::new();
                while !matches!(self.peek(), Tok::RBrace | Tok::Eof) {
                    if self.eat(&Tok::LBracket) {
                        let k = self.parse_expr()?;
                        self.eat(&Tok::RBracket);
                        self.eat(&Tok::Assign);
                        let v = self.parse_expr()?;
                        entries.push((Some(k), v));
                    } else if let Tok::Ident(k) = self.peek().clone() {
                        if self.tokens.get(self.pos + 1) == Some(&Tok::Assign) {
                            self.next();
                            self.next();
                            let v = self.parse_expr()?;
                            entries.push((Some(Expr::Str(k)), v));
                        } else {
                            let v = self.parse_expr()?;
                            entries.push((None, v));
                        }
                    } else {
                        let v = self.parse_expr()?;
                        entries.push((None, v));
                    }
                    if !self.eat(&Tok::Comma) && !self.eat(&Tok::Semi) {
                        break;
                    }
                }
                self.eat(&Tok::RBrace);
                Some(Expr::Table(entries))
            }
            _ => None,
        }
    }
}

fn expr_to_lvalue(expr: Expr) -> Option<LValue> {
    match expr {
        Expr::Var(name) => Some(LValue::Var(name)),
        Expr::Field(base, field) => Some(LValue::Field(*base, field)),
        Expr::Index(base, idx) => Some(LValue::Index(*base, *idx)),
        _ => None,
    }
}

// ============================================================================
// Virtual Machine / Evaluator
// ============================================================================

enum Flow {
    None,
    Break,
    Return(Vec<LuaVal>),
}

struct Vm<'a> {
    obj: &'a mut LuaObjState,
    globals: HashMap<String, LuaVal>,
    scopes: Vec<HashMap<String, LuaVal>>,
    steps: usize,
    rng_state: u64,
    multi_return: Vec<LuaVal>,
}

impl<'a> Vm<'a> {
    fn new(obj: &'a mut LuaObjState) -> Self {
        let mut globals = HashMap::new();
        for (k, v) in &obj.named_vars {
            globals.insert(k.clone(), v.clone());
        }
        let seed = ((obj.frame as u64).wrapping_mul(6364136223846793005)) ^ 0x9E3779B97F4A7C15;
        Self {
            obj,
            globals,
            scopes: vec![HashMap::new()],
            steps: 0,
            rng_state: seed,
            multi_return: Vec::new(),
        }
    }

    fn tick(&mut self) -> bool {
        self.steps += 1;
        self.steps <= MAX_STEPS
    }

    fn next_rand(&mut self) -> f64 {
        self.rng_state = self
            .rng_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.rng_state >> 11) as f64) / ((1u64 << 53) as f64)
    }

    fn get_var(&self, name: &str) -> LuaVal {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return v.clone();
            }
        }
        if let Some(v) = self.globals.get(name) {
            return v.clone();
        }
        LuaVal::Nil
    }

    fn set_var(&mut self, name: &str, val: LuaVal) {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), val);
                return;
            }
        }
        self.globals.insert(name.to_string(), val);
    }

    fn set_local(&mut self, name: String, val: LuaVal) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, val);
        }
    }

    fn get_obj_prop(&self, field: &str) -> LuaVal {
        match field {
            "ox" => LuaVal::Num(self.obj.ox),
            "oy" => LuaVal::Num(self.obj.oy),
            "oz" => LuaVal::Num(self.obj.oz),
            "x" => LuaVal::Num(self.obj.x),
            "y" => LuaVal::Num(self.obj.y),
            "z" => LuaVal::Num(self.obj.z),
            "rx" => LuaVal::Num(self.obj.rx),
            "ry" => LuaVal::Num(self.obj.ry),
            "rz" => LuaVal::Num(self.obj.rz),
            "cx" => LuaVal::Num(self.obj.cx),
            "cy" => LuaVal::Num(self.obj.cy),
            "cz" => LuaVal::Num(self.obj.cz),
            "zoom" => LuaVal::Num(self.obj.zoom),
            "alpha" => LuaVal::Num(self.obj.alpha),
            "aspect" => LuaVal::Num(self.obj.aspect),
            "time" => LuaVal::Num(self.obj.time),
            "totaltime" => LuaVal::Num(self.obj.totaltime),
            "frame" => LuaVal::Num(self.obj.frame as f64),
            "totalframe" => LuaVal::Num(self.obj.totalframe as f64),
            "layer" => LuaVal::Num(self.obj.layer as f64),
            "index" => LuaVal::Num(self.obj.index as f64),
            "num" => LuaVal::Num(self.obj.num as f64),
            "w" | "getpixel_w" => LuaVal::Num(self.obj.w),
            "h" | "getpixel_h" => LuaVal::Num(self.obj.h),
            "screen_w" => LuaVal::Num(self.obj.screen_w),
            "screen_h" => LuaVal::Num(self.obj.screen_h),
            "framerate" => LuaVal::Num(self.obj.framerate),
            "track0" => LuaVal::Num(self.obj.tracks[0]),
            "track1" => LuaVal::Num(self.obj.tracks[1]),
            "track2" => LuaVal::Num(self.obj.tracks[2]),
            "track3" => LuaVal::Num(self.obj.tracks[3]),
            "check0" => LuaVal::Bool(self.obj.check0),
            other => self.obj.named_vars.get(other).cloned().unwrap_or(LuaVal::Nil),
        }
    }

    fn set_obj_prop(&mut self, field: &str, val: LuaVal) {
        match field {
            "ox" => self.obj.ox = val.as_num(),
            "oy" => self.obj.oy = val.as_num(),
            "oz" => self.obj.oz = val.as_num(),
            "rx" => self.obj.rx = val.as_num(),
            "ry" => self.obj.ry = val.as_num(),
            "rz" => self.obj.rz = val.as_num(),
            "cx" => self.obj.cx = val.as_num(),
            "cy" => self.obj.cy = val.as_num(),
            "cz" => self.obj.cz = val.as_num(),
            "zoom" => self.obj.zoom = val.as_num(),
            "alpha" => self.obj.alpha = val.as_num().clamp(0.0, 1.0),
            "aspect" => self.obj.aspect = val.as_num(),
            "track0" => self.obj.tracks[0] = val.as_num(),
            "track1" => self.obj.tracks[1] = val.as_num(),
            "track2" => self.obj.tracks[2] = val.as_num(),
            "track3" => self.obj.tracks[3] = val.as_num(),
            "check0" => self.obj.check0 = val.truthy(),
            other => {
                self.obj.named_vars.insert(other.to_string(), val);
            }
        }
    }

    fn exec_block(&mut self, stmts: &[Stmt]) -> Flow {
        for s in stmts {
            if !self.tick() {
                return Flow::Break;
            }
            let flow = self.exec_stmt(s);
            if !matches!(flow, Flow::None) {
                return flow;
            }
        }
        Flow::None
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> Flow {
        if !self.tick() {
            return Flow::Break;
        }
        match stmt {
            Stmt::Local(names, exprs) => {
                self.multi_return.clear();
                let mut vals: Vec<LuaVal> = exprs.iter().map(|e| self.eval_expr(e)).collect();
                if vals.len() < names.len() && !self.multi_return.is_empty() {
                    vals.append(&mut self.multi_return);
                }
                for (i, name) in names.iter().enumerate() {
                    let v = vals.get(i).cloned().unwrap_or(LuaVal::Nil);
                    self.set_local(name.clone(), v);
                }
                Flow::None
            }
            Stmt::Assign(lvals, exprs) => {
                self.multi_return.clear();
                let mut vals: Vec<LuaVal> = exprs.iter().map(|e| self.eval_expr(e)).collect();
                if vals.len() < lvals.len() && !self.multi_return.is_empty() {
                    vals.append(&mut self.multi_return);
                }
                for (i, lv) in lvals.iter().enumerate() {
                    let v = vals.get(i).cloned().unwrap_or(LuaVal::Nil);
                    self.assign_lvalue(lv, v);
                }
                Flow::None
            }
            Stmt::If { branches, else_body } => {
                for (cond, body) in branches {
                    if self.eval_expr(cond).truthy() {
                        self.scopes.push(HashMap::new());
                        let f = self.exec_block(body);
                        self.scopes.pop();
                        return f;
                    }
                }
                if !else_body.is_empty() {
                    self.scopes.push(HashMap::new());
                    let f = self.exec_block(else_body);
                    self.scopes.pop();
                    return f;
                }
                Flow::None
            }
            Stmt::While(cond, body) => {
                while self.tick() && self.eval_expr(cond).truthy() {
                    self.scopes.push(HashMap::new());
                    let f = self.exec_block(body);
                    self.scopes.pop();
                    match f {
                        Flow::None => {}
                        Flow::Break => break,
                        Flow::Return(r) => return Flow::Return(r),
                    }
                }
                Flow::None
            }
            Stmt::Repeat(body, cond) => {
                loop {
                    if !self.tick() {
                        break;
                    }
                    self.scopes.push(HashMap::new());
                    let f = self.exec_block(body);
                    let done = self.eval_expr(cond).truthy();
                    self.scopes.pop();
                    match f {
                        Flow::None => {
                            if done {
                                break;
                            }
                        }
                        Flow::Break => break,
                        Flow::Return(r) => return Flow::Return(r),
                    }
                }
                Flow::None
            }
            Stmt::ForNum { var, start, limit, step, body } => {
                let mut cur = self.eval_expr(start).as_num();
                let lim = self.eval_expr(limit).as_num();
                let stp = step.as_ref().map(|e| self.eval_expr(e).as_num()).unwrap_or(1.0);
                if stp == 0.0 {
                    return Flow::None;
                }
                while self.tick() && ((stp > 0.0 && cur <= lim + 1e-12) || (stp < 0.0 && cur >= lim - 1e-12)) {
                    let mut scope = HashMap::new();
                    scope.insert(var.clone(), LuaVal::Num(cur));
                    self.scopes.push(scope);
                    let f = self.exec_block(body);
                    self.scopes.pop();
                    match f {
                        Flow::None => {}
                        Flow::Break => break,
                        Flow::Return(r) => return Flow::Return(r),
                    }
                    cur += stp;
                }
                Flow::None
            }
            Stmt::ForIn { vars, iter, body } => {
                let tbl = self.eval_expr(iter);
                if let LuaVal::Table(map, arr) = tbl {
                    for (idx, val) in arr.into_iter().enumerate() {
                        if !self.tick() {
                            break;
                        }
                        let mut scope = HashMap::new();
                        if let Some(k_var) = vars.first() {
                            scope.insert(k_var.clone(), LuaVal::Num((idx + 1) as f64));
                        }
                        if let Some(v_var) = vars.get(1) {
                            scope.insert(v_var.clone(), val);
                        }
                        self.scopes.push(scope);
                        let f = self.exec_block(body);
                        self.scopes.pop();
                        match f {
                            Flow::None => {}
                            Flow::Break => break,
                            Flow::Return(r) => return Flow::Return(r),
                        }
                    }
                    for (k, val) in map {
                        if !self.tick() {
                            break;
                        }
                        let mut scope = HashMap::new();
                        if let Some(k_var) = vars.first() {
                            scope.insert(k_var.clone(), LuaVal::Str(k));
                        }
                        if let Some(v_var) = vars.get(1) {
                            scope.insert(v_var.clone(), val);
                        }
                        self.scopes.push(scope);
                        let f = self.exec_block(body);
                        self.scopes.pop();
                        match f {
                            Flow::None => {}
                            Flow::Break => break,
                            Flow::Return(r) => return Flow::Return(r),
                        }
                    }
                }
                Flow::None
            }
            Stmt::FuncDef { is_local, name, field, params, body } => {
                let fn_val = LuaVal::Func(params.clone(), body.clone());
                if let Some(f) = field {
                    let mut base = self.get_var(name);
                    if let LuaVal::Table(ref mut map, _) = base {
                        map.insert(f.clone(), fn_val);
                        self.set_var(name, base);
                    }
                } else if *is_local {
                    self.set_local(name.clone(), fn_val);
                } else {
                    self.set_var(name, fn_val);
                }
                Flow::None
            }
            Stmt::Do(body) => {
                self.scopes.push(HashMap::new());
                let f = self.exec_block(body);
                self.scopes.pop();
                f
            }
            Stmt::Return(exprs) => {
                let vals = exprs.iter().map(|e| self.eval_expr(e)).collect();
                Flow::Return(vals)
            }
            Stmt::Break => Flow::Break,
            Stmt::ExprStmt(e) => {
                self.eval_expr(e);
                Flow::None
            }
        }
    }

    fn assign_lvalue(&mut self, lv: &LValue, val: LuaVal) {
        match lv {
            LValue::Var(name) => self.set_var(name, val),
            LValue::Field(Expr::Var(base), field) if base == "obj" => {
                self.set_obj_prop(field, val);
            }
            LValue::Field(Expr::Var(base), field) => {
                let mut tbl = self.get_var(base);
                if let LuaVal::Table(ref mut map, _) = tbl {
                    map.insert(field.clone(), val);
                    self.set_var(base, tbl);
                }
            }
            LValue::Index(Expr::Var(base), idx_expr) => {
                let idx_val = self.eval_expr(idx_expr);
                let mut tbl = self.get_var(base);
                if let LuaVal::Table(ref mut map, ref mut arr) = tbl {
                    if let LuaVal::Num(n) = idx_val {
                        let i = n as isize;
                        if i >= 1 {
                            let u = (i - 1) as usize;
                            if u >= arr.len() && u < 4096 {
                                arr.resize(u + 1, LuaVal::Nil);
                            }
                            if u < arr.len() {
                                arr[u] = val;
                            }
                        } else {
                            map.insert(idx_val.as_str(), val);
                        }
                    } else {
                        map.insert(idx_val.as_str(), val);
                    }
                    self.set_var(base, tbl);
                }
            }
            _ => {}
        }
    }

    fn eval_expr(&mut self, expr: &Expr) -> LuaVal {
        if !self.tick() {
            return LuaVal::Nil;
        }
        match expr {
            Expr::Nil => LuaVal::Nil,
            Expr::Bool(b) => LuaVal::Bool(*b),
            Expr::Num(n) => LuaVal::Num(*n),
            Expr::Str(s) => LuaVal::Str(s.clone()),
            Expr::Var(name) => match name.as_str() {
                "PI" => LuaVal::Num(PI),
                _ => self.get_var(name),
            },
            Expr::Table(entries) => {
                let mut map = HashMap::new();
                let mut arr = Vec::new();
                for (k_opt, v_expr) in entries {
                    let v = self.eval_expr(v_expr);
                    if let Some(k_expr) = k_opt {
                        let k = self.eval_expr(k_expr);
                        map.insert(k.as_str(), v);
                    } else {
                        arr.push(v);
                    }
                }
                LuaVal::Table(map, arr)
            }
            Expr::AnonFunc(params, body) => LuaVal::Func(params.clone(), body.clone()),
            Expr::UnaryOp(op, inner) => {
                let v = self.eval_expr(inner);
                match op {
                    UnaryOp::Neg => LuaVal::Num(-v.as_num()),
                    UnaryOp::Not => LuaVal::Bool(!v.truthy()),
                    UnaryOp::Len => match v {
                        LuaVal::Str(s) => LuaVal::Num(s.chars().count() as f64),
                        LuaVal::Table(_, arr) => LuaVal::Num(arr.len() as f64),
                        _ => LuaVal::Num(0.0),
                    },
                }
            }
            Expr::BinOp(op, l, r) => match op {
                BinOp::And => {
                    let lv = self.eval_expr(l);
                    if !lv.truthy() {
                        lv
                    } else {
                        self.eval_expr(r)
                    }
                }
                BinOp::Or => {
                    let lv = self.eval_expr(l);
                    if lv.truthy() {
                        lv
                    } else {
                        self.eval_expr(r)
                    }
                }
                _ => {
                    let lv = self.eval_expr(l);
                    let rv = self.eval_expr(r);
                    match op {
                        BinOp::Add => LuaVal::Num(lv.as_num() + rv.as_num()),
                        BinOp::Sub => LuaVal::Num(lv.as_num() - rv.as_num()),
                        BinOp::Mul => LuaVal::Num(lv.as_num() * rv.as_num()),
                        BinOp::Div => {
                            let d = rv.as_num();
                            LuaVal::Num(if d == 0.0 { 0.0 } else { lv.as_num() / d })
                        }
                        BinOp::Mod => {
                            let d = rv.as_num();
                            LuaVal::Num(if d == 0.0 { 0.0 } else { lv.as_num().rem_euclid(d) })
                        }
                        BinOp::Pow => LuaVal::Num(lv.as_num().powf(rv.as_num())),
                        BinOp::Concat => LuaVal::Str(format!("{}{}", lv.as_str(), rv.as_str())),
                        BinOp::Eq => LuaVal::Bool(vals_equal(&lv, &rv)),
                        BinOp::NotEq => LuaVal::Bool(!vals_equal(&lv, &rv)),
                        BinOp::Lt => LuaVal::Bool(lv.as_num() < rv.as_num()),
                        BinOp::Le => LuaVal::Bool(lv.as_num() <= rv.as_num()),
                        BinOp::Gt => LuaVal::Bool(lv.as_num() > rv.as_num()),
                        BinOp::Ge => LuaVal::Bool(lv.as_num() >= rv.as_num()),
                        BinOp::And | BinOp::Or => unreachable!(),
                    }
                }
            },
            Expr::Field(base, field) => {
                if let Expr::Var(bname) = base.as_ref() {
                    match (bname.as_str(), field.as_str()) {
                        ("obj", f) => return self.get_obj_prop(f),
                        ("math", "pi") => return LuaVal::Num(PI),
                        ("math", "huge") => return LuaVal::Num(f64::INFINITY),
                        _ => {}
                    }
                }
                let bval = self.eval_expr(base);
                if let LuaVal::Table(map, _) = bval {
                    map.get(field).cloned().unwrap_or(LuaVal::Nil)
                } else {
                    LuaVal::Nil
                }
            }
            Expr::Index(base, idx_expr) => {
                let bval = self.eval_expr(base);
                let idx = self.eval_expr(idx_expr);
                if let LuaVal::Table(map, arr) = bval {
                    if let LuaVal::Num(n) = idx {
                        let i = n as isize;
                        if i >= 1 && (i as usize) <= arr.len() {
                            return arr[(i - 1) as usize].clone();
                        }
                    }
                    map.get(&idx.as_str()).cloned().unwrap_or(LuaVal::Nil)
                } else {
                    LuaVal::Nil
                }
            }
            Expr::MethodCall(base, method, args) => {
                let eval_args: Vec<LuaVal> = args.iter().map(|a| self.eval_expr(a)).collect();
                if let Expr::Var(bname) = base.as_ref() {
                    if bname == "obj" {
                        return self.call_obj_func(method, &eval_args);
                    }
                }
                LuaVal::Nil
            }
            Expr::Call(callee, args) => {
                let eval_args: Vec<LuaVal> = args.iter().map(|a| self.eval_expr(a)).collect();
                match callee.as_ref() {
                    Expr::Field(base, func_name) => {
                        if let Expr::Var(ns) = base.as_ref() {
                            match ns.as_str() {
                                "obj" => return self.call_obj_func(func_name, &eval_args),
                                "math" => return self.call_math_func(func_name, &eval_args),
                                "string" => return self.call_string_func(func_name, &eval_args),
                                "table" => {
                                    if let Some(Expr::Var(tvar)) = args.first() {
                                        return self.call_table_func(func_name, tvar, &eval_args);
                                    }
                                    return LuaVal::Nil;
                                }
                                _ => {}
                            }
                        }
                        let fn_val = self.eval_expr(callee);
                        self.invoke_user_func(fn_val, eval_args)
                    }
                    Expr::Var(fname) => {
                        if let Some(res) = self.call_global_builtin(fname, &eval_args) {
                            return res;
                        }
                        let fn_val = self.get_var(fname);
                        self.invoke_user_func(fn_val, eval_args)
                    }
                    _ => {
                        let fn_val = self.eval_expr(callee);
                        self.invoke_user_func(fn_val, eval_args)
                    }
                }
            }
        }
    }

    fn invoke_user_func(&mut self, fn_val: LuaVal, args: Vec<LuaVal>) -> LuaVal {
        if let LuaVal::Func(params, body) = fn_val {
            let mut scope = HashMap::new();
            for (i, p) in params.iter().enumerate() {
                scope.insert(p.clone(), args.get(i).cloned().unwrap_or(LuaVal::Nil));
            }
            self.scopes.push(scope);
            let f = self.exec_block(&body);
            self.scopes.pop();
            if let Flow::Return(mut r) = f {
                if !r.is_empty() {
                    let first = r.remove(0);
                    self.multi_return = r;
                    return first;
                }
            }
        }
        LuaVal::Nil
    }

    fn call_global_builtin(&mut self, name: &str, args: &[LuaVal]) -> Option<LuaVal> {
        let a0 = || args.first().map(|v| v.as_num()).unwrap_or(0.0);
        let a1 = || args.get(1).map(|v| v.as_num()).unwrap_or(0.0);
        let a2 = || args.get(2).map(|v| v.as_num()).unwrap_or(0.0);

        match name {
            "tonumber" => Some(LuaVal::Num(a0())),
            "tostring" => Some(LuaVal::Str(args.first().map(|v| v.as_str()).unwrap_or_default())),
            "type" => {
                let t = match args.first().unwrap_or(&LuaVal::Nil) {
                    LuaVal::Nil => "nil",
                    LuaVal::Bool(_) => "boolean",
                    LuaVal::Num(_) => "number",
                    LuaVal::Str(_) => "string",
                    LuaVal::Table(_, _) => "table",
                    LuaVal::Func(_, _) => "function",
                };
                Some(LuaVal::Str(t.into()))
            }
            "ipairs" | "pairs" => Some(args.first().cloned().unwrap_or(LuaVal::Nil)),
            "RGB" => {
                let r = (a0() as u32).min(255);
                let g = (a1() as u32).min(255);
                let b = (a2() as u32).min(255);
                Some(LuaVal::Num(((r << 16) | (g << 8) | b) as f64))
            }
            "HSV" => {
                let h = a0().rem_euclid(360.0);
                let s = (a1() / 100.0).clamp(0.0, 1.0);
                let v = (a2() / 100.0).clamp(0.0, 1.0);
                let c = v * s;
                let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
                let m = v - c;
                let (r1, g1, b1) = match h as u32 {
                    0..=59 => (c, x, 0.0),
                    60..=119 => (x, c, 0.0),
                    120..=179 => (0.0, c, x),
                    180..=239 => (0.0, x, c),
                    240..=299 => (x, 0.0, c),
                    _ => (c, 0.0, x),
                };
                let r = (((r1 + m) * 255.0).round() as u32).min(255);
                let g = (((g1 + m) * 255.0).round() as u32).min(255);
                let b = (((b1 + m) * 255.0).round() as u32).min(255);
                Some(LuaVal::Num(((r << 16) | (g << 8) | b) as f64))
            }
            "AND" => Some(LuaVal::Num(((a0() as i64) & (a1() as i64)) as f64)),
            "OR" => Some(LuaVal::Num(((a0() as i64) | (a1() as i64)) as f64)),
            "XOR" => Some(LuaVal::Num(((a0() as i64) ^ (a1() as i64)) as f64)),
            "SHIFT" => {
                let v = a0() as i64;
                let sh = a1() as i32;
                let res = if sh >= 0 { v.wrapping_shl(sh as u32) } else { v.wrapping_shr((-sh) as u32) };
                Some(LuaVal::Num(res as f64))
            }
            "rand" => Some(self.call_obj_func("rand", args)),
            "print" | "debug_print" => Some(LuaVal::Nil),
            _ => None,
        }
    }

    fn call_math_func(&mut self, func: &str, args: &[LuaVal]) -> LuaVal {
        let a = args.first().map(|v| v.as_num()).unwrap_or(0.0);
        let b = args.get(1).map(|v| v.as_num()).unwrap_or(0.0);
        let n = match func {
            "sin" => a.sin(),
            "cos" => a.cos(),
            "tan" => a.tan(),
            "asin" => a.clamp(-1.0, 1.0).asin(),
            "acos" => a.clamp(-1.0, 1.0).acos(),
            "atan" => {
                if args.len() >= 2 {
                    a.atan2(b)
                } else {
                    a.atan()
                }
            }
            "atan2" => a.atan2(b),
            "sinh" => a.sinh(),
            "cosh" => a.cosh(),
            "tanh" => a.tanh(),
            "sqrt" => a.max(0.0).sqrt(),
            "abs" => a.abs(),
            "floor" => a.floor(),
            "ceil" => a.ceil(),
            "min" => args.iter().map(|v| v.as_num()).fold(f64::INFINITY, f64::min),
            "max" => args.iter().map(|v| v.as_num()).fold(f64::NEG_INFINITY, f64::max),
            "fmod" => if b == 0.0 { 0.0 } else { a % b },
            "modf" => a.trunc(),
            "pow" => a.powf(b),
            "exp" => a.exp(),
            "log" => if args.len() >= 2 { a.max(1e-12).log(b) } else { a.max(1e-12).ln() },
            "log10" => a.max(1e-12).log10(),
            "rad" => a.to_radians(),
            "deg" => a.to_degrees(),
            "randomseed" => {
                self.rng_state = (a as u64) ^ 0x9E3779B97F4A7C15;
                return LuaVal::Nil;
            }
            "random" => {
                let r = self.next_rand();
                match args.len() {
                    0 => r,
                    1 => (1.0 + (r * a.max(1.0)).floor()).min(a.max(1.0)),
                    _ => {
                        let lo = a.min(b);
                        let hi = a.max(b);
                        lo + (r * (hi - lo + 1.0)).floor().min(hi - lo)
                    }
                }
            }
            _ => 0.0,
        };
        LuaVal::Num(n)
    }

    fn call_string_func(&self, func: &str, args: &[LuaVal]) -> LuaVal {
        let s = args.first().map(|v| v.as_str()).unwrap_or_default();
        match func {
            "len" => LuaVal::Num(s.chars().count() as f64),
            "upper" => LuaVal::Str(s.to_uppercase()),
            "lower" => LuaVal::Str(s.to_lowercase()),
            "reverse" => LuaVal::Str(s.chars().rev().collect()),
            "rep" => {
                let count = args.get(1).map(|v| v.as_num() as usize).unwrap_or(0).min(1024);
                LuaVal::Str(s.repeat(count))
            }
            "sub" => {
                let chars: Vec<char> = s.chars().collect();
                let len = chars.len() as isize;
                let mut i = args.get(1).map(|v| v.as_num() as isize).unwrap_or(1);
                let mut j = args.get(2).map(|v| v.as_num() as isize).unwrap_or(len);
                if i < 0 {
                    i = (len + i + 1).max(1);
                }
                if j < 0 {
                    j = len + j + 1;
                }
                i = i.max(1);
                j = j.min(len);
                if i > j {
                    LuaVal::Str(String::new())
                } else {
                    let sub: String = chars[(i - 1) as usize..j as usize].iter().collect();
                    LuaVal::Str(sub)
                }
            }
            "format" => {
                // Simple sprintf-style substitution for %d, %f, %.2f, %s, %x
                let mut out = String::new();
                let mut arg_idx = 1;
                let chars: Vec<char> = s.chars().collect();
                let mut i = 0;
                while i < chars.len() {
                    if chars[i] == '%' && i + 1 < chars.len() {
                        if chars[i + 1] == '%' {
                            out.push('%');
                            i += 2;
                            continue;
                        }
                        i += 1;
                        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '-' || chars[i] == '+') {
                            i += 1;
                        }
                        let spec = chars.get(i).copied().unwrap_or('s');
                        let val = args.get(arg_idx).cloned().unwrap_or(LuaVal::Nil);
                        arg_idx += 1;
                        match spec {
                            'd' | 'i' => out.push_str(&format!("{}", val.as_num() as i64)),
                            'x' => out.push_str(&format!("{:x}", val.as_num() as u64)),
                            'X' => out.push_str(&format!("{:X}", val.as_num() as u64)),
                            'f' => out.push_str(&format!("{:.2}", val.as_num())),
                            _ => out.push_str(&val.as_str()),
                        }
                        i += 1;
                    } else {
                        out.push(chars[i]);
                        i += 1;
                    }
                }
                LuaVal::Str(out)
            }
            _ => LuaVal::Str(s),
        }
    }

    fn call_table_func(&mut self, func: &str, tvar: &str, args: &[LuaVal]) -> LuaVal {
        let mut tbl = self.get_var(tvar);
        if let LuaVal::Table(_, ref mut arr) = tbl {
            match func {
                "insert" => {
                    if args.len() == 2 {
                        arr.push(args[1].clone());
                    } else if args.len() >= 3 {
                        let idx = (args[1].as_num() as usize).saturating_sub(1).min(arr.len());
                        arr.insert(idx, args[2].clone());
                    }
                    self.set_var(tvar, tbl);
                }
                "remove" => {
                    let removed = if !arr.is_empty() {
                        let idx = args
                            .get(1)
                            .map(|v| (v.as_num() as usize).saturating_sub(1).min(arr.len() - 1))
                            .unwrap_or(arr.len() - 1);
                        arr.remove(idx)
                    } else {
                        LuaVal::Nil
                    };
                    self.set_var(tvar, tbl);
                    return removed;
                }
                "concat" => {
                    let sep = args.get(1).map(|v| v.as_str()).unwrap_or_default();
                    let parts: Vec<String> = arr.iter().map(|v| v.as_str()).collect();
                    return LuaVal::Str(parts.join(&sep));
                }
                _ => {}
            }
        }
        LuaVal::Nil
    }

    fn call_obj_func(&mut self, func: &str, args: &[LuaVal]) -> LuaVal {
        match func {
            "track" => {
                let idx = args.first().map(|v| v.as_num() as usize).unwrap_or(0);
                LuaVal::Num(*self.obj.tracks.get(idx).unwrap_or(&0.0))
            }
            "rand" => {
                let st = args.first().map(|v| v.as_num()).unwrap_or(0.0);
                let ed = args.get(1).map(|v| v.as_num()).unwrap_or(100.0);
                let seed = args.get(2).map(|v| v.as_num() as i64).unwrap_or(0);
                let frm = args.get(3).map(|v| v.as_num() as i64).unwrap_or(self.obj.frame);
                let lo = st.min(ed);
                let hi = st.max(ed);
                let mut h = (frm as u64)
                    .wrapping_mul(0x9E3779B97F4A7C15)
                    .wrapping_add((seed as u64).wrapping_mul(0xBF58476D1CE4E5B9))
                    .wrapping_add((self.obj.index as u64).wrapping_mul(0x94D049BB133111EB));
                h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                h = (h ^ (h >> 27)).wrapping_mul(0x94D049BB133111EB);
                h ^= h >> 31;
                let span = (hi - lo + 1.0).max(1.0) as u64;
                LuaVal::Num(lo + (h % span) as f64)
            }
            "getvalue" => {
                let key = args.first().map(|v| v.as_str()).unwrap_or_default();
                match key.as_str() {
                    "x" => LuaVal::Num(self.obj.x + self.obj.ox),
                    "y" => LuaVal::Num(self.obj.y + self.obj.oy),
                    "z" => LuaVal::Num(self.obj.z + self.obj.oz),
                    "rx" => LuaVal::Num(self.obj.rx),
                    "ry" => LuaVal::Num(self.obj.ry),
                    "rz" => LuaVal::Num(self.obj.rz),
                    "zoom" => LuaVal::Num(self.obj.zoom * 100.0),
                    "alpha" => LuaVal::Num(self.obj.alpha),
                    "aspect" => LuaVal::Num(self.obj.aspect),
                    "time" => LuaVal::Num(self.obj.time),
                    "totaltime" => LuaVal::Num(self.obj.totaltime),
                    "scenechange" => {
                        let ratio = if self.obj.totaltime > 0.0 {
                            (self.obj.time / self.obj.totaltime).clamp(0.0, 1.0)
                        } else {
                            0.5
                        };
                        LuaVal::Num(ratio)
                    }
                    other => {
                        if let Some(idx_str) = other.strip_prefix("track") {
                            if let Ok(idx) = idx_str.parse::<usize>() {
                                return LuaVal::Num(*self.obj.tracks.get(idx).unwrap_or(&0.0));
                            }
                        }
                        LuaVal::Num(0.0)
                    }
                }
            }
            "setfont" => {
                if let Some(name) = args.first() {
                    self.obj.font_name = name.as_str();
                }
                if let Some(sz) = args.get(1) {
                    self.obj.font_size = sz.as_num().max(1.0);
                }
                if let Some(c1) = args.get(3) {
                    self.obj.font_color = c1.as_num() as u32;
                }
                if let Some(c2) = args.get(4) {
                    self.obj.font_color2 = c2.as_num() as u32;
                }
                LuaVal::Nil
            }
            "mes" => {
                let text = args.first().map(|v| v.as_str()).unwrap_or_default();
                self.obj.loaded_primitive = Some(LuaLoadedPrimitive::Text {
                    text,
                    font: self.obj.font_name.clone(),
                    size: self.obj.font_size,
                    color: self.obj.font_color,
                    color2: self.obj.font_color2,
                });
                LuaVal::Nil
            }
            "load" => {
                let kind = args.first().map(|v| v.as_str().to_lowercase()).unwrap_or_else(|| "figure".into());
                match kind.as_str() {
                    "figure" => {
                        let fig_type = args.get(1).map(|v| v.as_str()).unwrap_or_else(|| "円".into());
                        let color = args.get(2).map(|v| v.as_num() as u32).unwrap_or(0xFFFFFF);
                        let size = args.get(3).map(|v| v.as_num()).unwrap_or(100.0);
                        let line_width = args.get(4).map(|v| v.as_num()).unwrap_or(4000.0);
                        self.obj.w = size;
                        self.obj.h = size;
                        self.obj.loaded_primitive = Some(LuaLoadedPrimitive::Figure {
                            fig_type,
                            color,
                            size,
                            line_width,
                        });
                    }
                    "text" => {
                        let text = args.get(1).map(|v| v.as_str()).unwrap_or_default();
                        self.obj.loaded_primitive = Some(LuaLoadedPrimitive::Text {
                            text,
                            font: self.obj.font_name.clone(),
                            size: self.obj.font_size,
                            color: self.obj.font_color,
                            color2: self.obj.font_color2,
                        });
                    }
                    "image" | "movie" => {
                        let path = args.get(1).map(|v| v.as_str()).unwrap_or_default();
                        self.obj.loaded_primitive = Some(LuaLoadedPrimitive::Image { path });
                    }
                    _ => {}
                }
                LuaVal::Nil
            }
            "effect" => {
                if let Some(first) = args.first() {
                    let name = first.as_str();
                    let mut params = HashMap::new();
                    let mut i = 1;
                    while i + 1 < args.len() {
                        params.insert(args[i].as_str(), args[i + 1].clone());
                        i += 2;
                    }
                    // Apply immediate coordinate/alpha filters or record visual filter call
                    match name.as_str() {
                        "振動" => {
                            let ax = params.get("X").map(|v| v.as_num()).unwrap_or(10.0);
                            let ay = params.get("Y").map(|v| v.as_num()).unwrap_or(10.0);
                            let period = params.get("周期").map(|v| v.as_num()).unwrap_or(1.0).max(0.1);
                            let phase = (self.obj.frame as f64 / period) * PI * 2.0;
                            self.obj.ox += phase.sin() * ax;
                            self.obj.oy += (phase * 1.37).cos() * ay;
                        }
                        "クリッピング" => {
                            let top = params.get("上").map(|v| v.as_num()).unwrap_or(0.0);
                            let bottom = params.get("下").map(|v| v.as_num()).unwrap_or(0.0);
                            let left = params.get("左").map(|v| v.as_num()).unwrap_or(0.0);
                            let right = params.get("右").map(|v| v.as_num()).unwrap_or(0.0);
                            self.obj.current_clip = Some([top, bottom, left, right]);
                        }
                        "リサイズ" => {
                            let z = params.get("拡大率").map(|v| v.as_num() / 100.0).unwrap_or(1.0);
                            let sx = params.get("X").map(|v| v.as_num() / 100.0).unwrap_or(1.0);
                            let sy = params.get("Y").map(|v| v.as_num() / 100.0).unwrap_or(1.0);
                            let avg_s = ((sx + sy) * 0.5).max(0.0);
                            self.obj.zoom *= z * avg_s;
                            if sx > 0.001 && sy > 0.001 && (sx - sy).abs() > 0.001 {
                                if sx < sy {
                                    self.obj.aspect = ((1.0 - sx / sy) * 100.0).clamp(-100.0, 100.0);
                                } else {
                                    self.obj.aspect = (-(1.0 - sy / sx) * 100.0).clamp(-100.0, 100.0);
                                }
                            }
                        }
                        "透明度" => {
                            if let Some(tr) = params.get("透明度") {
                                self.obj.alpha *= (1.0 - tr.as_num() / 100.0).clamp(0.0, 1.0);
                            }
                        }
                        _ => {}
                    }
                    self.obj.filter_calls.push(LuaFilterCall { name, params });
                }
                LuaVal::Nil
            }
            "draw" => {
                let ox = args.first().map(|v| v.as_num()).unwrap_or(0.0) + self.obj.ox;
                let oy = args.get(1).map(|v| v.as_num()).unwrap_or(0.0) + self.obj.oy;
                let oz = args.get(2).map(|v| v.as_num()).unwrap_or(0.0) + self.obj.oz;
                let zoom = args.get(3).map(|v| v.as_num()).unwrap_or(1.0) * self.obj.zoom;
                let alpha = (args.get(4).map(|v| v.as_num()).unwrap_or(1.0) * self.obj.alpha).clamp(0.0, 1.0);
                let rx = args.get(5).map(|v| v.as_num()).unwrap_or(0.0) + self.obj.rx;
                let ry = args.get(6).map(|v| v.as_num()).unwrap_or(0.0) + self.obj.ry;
                let rz = args.get(7).map(|v| v.as_num()).unwrap_or(0.0) + self.obj.rz;
                let color = self
                    .obj
                    .filter_calls
                    .iter()
                    .rev()
                    .find(|f| f.name == "単色化" || f.name == "グラデーション")
                    .and_then(|f| f.params.get("color").or_else(|| f.params.get("色")).map(|v| v.as_num() as u32));
                if self.obj.draw_calls.len() < 512 {
                    self.obj.draw_calls.push(LuaDrawCall {
                        ox,
                        oy,
                        oz,
                        zoom,
                        alpha,
                        rx,
                        ry,
                        rz,
                        color,
                        primitive: self.obj.loaded_primitive.clone(),
                        clip: self.obj.current_clip,
                    });
                }
                LuaVal::Nil
            }
            "drawpoly" => {
                if args.len() >= 12 && self.obj.poly_calls.len() < 512 {
                    let pts = [
                        (args[0].as_num(), args[1].as_num(), args[2].as_num()),
                        (args[3].as_num(), args[4].as_num(), args[5].as_num()),
                        (args[6].as_num(), args[7].as_num(), args[8].as_num()),
                        (args[9].as_num(), args[10].as_num(), args[11].as_num()),
                    ];
                    let alpha = args
                        .get(20)
                        .map(|v| v.as_num())
                        .unwrap_or(1.0)
                        * self.obj.alpha;
                    let color = match &self.obj.loaded_primitive {
                        Some(LuaLoadedPrimitive::Figure { color, .. }) => Some(*color),
                        Some(LuaLoadedPrimitive::Text { color, .. }) => Some(*color),
                        _ => None,
                    };
                    self.obj.poly_calls.push(LuaPolyCall {
                        pts,
                        alpha: alpha.clamp(0.0, 1.0),
                        color,
                    });
                }
                LuaVal::Nil
            }
            "interpolation" => {
                let catmull = |t: f64, p0: f64, p1: f64, p2: f64, p3: f64| -> f64 {
                    let t2 = t * t;
                    let t3 = t2 * t;
                    0.5 * ((2.0 * p1)
                        + (-p0 + p2) * t
                        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
                };
                let t = args.first().map(|v| v.as_num()).unwrap_or(0.0);
                if args.len() >= 13 {
                    let vx = catmull(t, args[1].as_num(), args[4].as_num(), args[7].as_num(), args[10].as_num());
                    let vy = catmull(t, args[2].as_num(), args[5].as_num(), args[8].as_num(), args[11].as_num());
                    let vz = catmull(t, args[3].as_num(), args[6].as_num(), args[9].as_num(), args[12].as_num());
                    self.multi_return = vec![LuaVal::Num(vy), LuaVal::Num(vz)];
                    LuaVal::Num(vx)
                } else if args.len() >= 9 {
                    let vx = catmull(t, args[1].as_num(), args[3].as_num(), args[5].as_num(), args[7].as_num());
                    let vy = catmull(t, args[2].as_num(), args[4].as_num(), args[6].as_num(), args[8].as_num());
                    self.multi_return = vec![LuaVal::Num(vy)];
                    LuaVal::Num(vx)
                } else if args.len() >= 5 {
                    let v = catmull(t, args[1].as_num(), args[2].as_num(), args[3].as_num(), args[4].as_num());
                    LuaVal::Num(v)
                } else {
                    LuaVal::Num(0.0)
                }
            }
            "getpixel" => {
                if args.is_empty() {
                    self.multi_return = vec![LuaVal::Num(self.obj.h)];
                    LuaVal::Num(self.obj.w)
                } else {
                    self.multi_return = vec![LuaVal::Num(1.0)];
                    LuaVal::Num(0xFFFFFF as f64)
                }
            }
            "getoption" => {
                let opt = args.first().map(|v| v.as_str()).unwrap_or_default();
                match opt.as_str() {
                    "camera_param" => {
                        let mut map = HashMap::new();
                        map.insert("x".into(), LuaVal::Num(self.obj.x + self.obj.ox));
                        map.insert("y".into(), LuaVal::Num(self.obj.y + self.obj.oy));
                        map.insert("z".into(), LuaVal::Num(self.obj.z + self.obj.oz));
                        map.insert("tx".into(), LuaVal::Num(self.obj.cam_tx));
                        map.insert("ty".into(), LuaVal::Num(self.obj.cam_ty));
                        map.insert("tz".into(), LuaVal::Num(self.obj.cam_tz));
                        map.insert("rz".into(), LuaVal::Num(self.obj.rz));
                        map.insert("ux".into(), LuaVal::Num(self.obj.cam_ux));
                        map.insert("uy".into(), LuaVal::Num(self.obj.cam_uy));
                        map.insert("uz".into(), LuaVal::Num(self.obj.cam_uz));
                        map.insert("d".into(), LuaVal::Num(self.obj.cam_d));
                        LuaVal::Table(map, Vec::new())
                    }
                    "section_num" => LuaVal::Num(1.0),
                    "track_mode" => LuaVal::Num(0.0),
                    "camera_mode" => LuaVal::Num(1.0),
                    "multi_object" => LuaVal::Bool(self.obj.num > 1),
                    "gui" => LuaVal::Bool(true),
                    _ => LuaVal::Num(0.0),
                }
            }
            "setoption" => {
                if let Some(opt) = args.first() {
                    match opt.as_str().as_str() {
                        "blend" => {
                            if let Some(mode) = args.get(1) {
                                self.obj.blend_mode = mode.as_num() as i64;
                            }
                        }
                        "culling" => {
                            if let Some(v) = args.get(1) {
                                self.obj.culling = v.as_num() as i64 != 0;
                            }
                        }
                        "billboard" => {
                            if let Some(v) = args.get(1) {
                                self.obj.billboard = v.as_num() as i64;
                            }
                        }
                        "camera_param" => {
                            if let Some(LuaVal::Table(map, _)) = args.get(1) {
                                if let Some(v) = map.get("x") {
                                    self.obj.ox = v.as_num() - self.obj.x;
                                }
                                if let Some(v) = map.get("y") {
                                    self.obj.oy = v.as_num() - self.obj.y;
                                }
                                if let Some(v) = map.get("z") {
                                    self.obj.oz = v.as_num() - self.obj.z;
                                }
                                if let Some(v) = map.get("tx") {
                                    self.obj.cam_tx = v.as_num();
                                }
                                if let Some(v) = map.get("ty") {
                                    self.obj.cam_ty = v.as_num();
                                }
                                if let Some(v) = map.get("tz") {
                                    self.obj.cam_tz = v.as_num();
                                }
                                if let Some(v) = map.get("rz") {
                                    self.obj.rz = v.as_num();
                                }
                                if let Some(v) = map.get("ux") {
                                    self.obj.cam_ux = v.as_num();
                                }
                                if let Some(v) = map.get("uy") {
                                    self.obj.cam_uy = v.as_num();
                                }
                                if let Some(v) = map.get("uz") {
                                    self.obj.cam_uz = v.as_num();
                                }
                                if let Some(v) = map.get("d") {
                                    self.obj.cam_d = v.as_num();
                                }
                            }
                        }
                        "drawtarget" => {
                            if args.get(1).map(|v| v.as_str()).as_deref() == Some("tempbuffer") {
                                if let (Some(w), Some(h)) = (args.get(2), args.get(3)) {
                                    self.obj.w = w.as_num().max(1.0);
                                    self.obj.h = h.as_num().max(1.0);
                                    self.obj.draw_calls.clear();
                                    self.obj.current_clip = None;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                LuaVal::Nil
            }
            "setanchor" => LuaVal::Num(1.0),
            "copybuffer" => LuaVal::Bool(true),
            _ => LuaVal::Nil,
        }
    }
}

fn vals_equal(a: &LuaVal, b: &LuaVal) -> bool {
    match (a, b) {
        (LuaVal::Nil, LuaVal::Nil) => true,
        (LuaVal::Bool(x), LuaVal::Bool(y)) => x == y,
        (LuaVal::Num(x), LuaVal::Num(y)) => (x - y).abs() < 1e-9,
        (LuaVal::Str(x), LuaVal::Str(y)) => x == y,
        _ => false,
    }
}

/// Lua スクリプトを事前コンパイル（Tokenize + Parse）して構文木 (AST) を返します。
pub fn compile_lua_script(source: &str) -> Vec<Stmt> {
    let tokens = tokenize(source);
    let mut parser = Parser::new(tokens);
    parser.parse_block()
}

/// コンパイル済み構文木 (AST) を `LuaObjState` に対して直接実行します。
/// 毎フレームの字句解析・パース・ヒープ確保を完全にスキップするため極めて高速です。
pub fn execute_compiled_script(stmts: &[Stmt], state: &mut LuaObjState) {
    let mut vm = Vm::new(state);
    if let Flow::Return(vals) = vm.exec_block(stmts) {
        if let Some(first) = vals.first() {
            state.return_value = Some(first.as_num());
        }
    }
}

/// `.aup` / `.exo` の `param=` 文字列（例: `num=8;dir=0;rot=0;col=0xffffff;`）を評価し、
/// 代入された変数を `state.named_vars` に格納します。
pub fn execute_param_assignments(param_str: &str, state: &mut LuaObjState) {
    if param_str.trim().is_empty() {
        return;
    }
    let stmts = compile_lua_script(param_str);
    let (globals, scopes) = {
        let mut vm = Vm::new(state);
        let _ = vm.exec_block(&stmts);
        (vm.globals, vm.scopes)
    };
    for (k, v) in globals {
        state.named_vars.insert(k, v);
    }
    if let Some(scope) = scopes.into_iter().next() {
        for (k, v) in scope {
            state.named_vars.insert(k, v);
        }
    }
}

/// Lua スクリプトのソースコードを `LuaObjState` に対して実行します。
pub fn execute_lua_script(source: &str, state: &mut LuaObjState) {
    let stmts = compile_lua_script(source);
    execute_compiled_script(&stmts, state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_shake_and_rotation_lua() {
        let script = r#"
            if (obj.check0) then
                local t = obj.time / obj.track2 * math.pi * 2
                obj.ox = obj.ox + math.sin(t) * obj.track0
                obj.oy = obj.oy + math.cos(t * 1.3) * obj.track1
                obj.rz = obj.rz + 45
            end
        "#;
        let mut st = LuaObjState {
            time: 0.1,
            tracks: [30.0, 20.0, 0.4, 0.0],
            check0: true,
            ..Default::default()
        };
        execute_lua_script(script, &mut st);
        assert!((st.ox - 30.0).abs() < 1e-4);
        assert!((st.rz - 45.0).abs() < 1e-4);
    }

    #[test]
    fn eval_custom_object_draw_loop() {
        let script = r#"
            obj.load("figure", "円", RGB(255, 128, 0), 60)
            local n = 6
            for i = 1, n do
                local a = (i - 1) / n * math.pi * 2
                obj.draw(math.cos(a) * 100, math.sin(a) * 100, 0, 0.8, 0.9)
            end
        "#;
        let mut st = LuaObjState::default();
        execute_lua_script(script, &mut st);
        assert_eq!(st.draw_calls.len(), 6);
        assert!((st.draw_calls[0].ox - 100.0).abs() < 1e-5);
    }
}
