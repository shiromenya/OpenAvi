//! Lua サブセットの字句解析器 (Lexer) および再帰下降構文解析器 (Parser)。

use super::types::{BinOp, Expr, LValue, Stmt, UnaryOp};

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Tok {
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

pub(super) fn tokenize(src: &str) -> Vec<Tok> {
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

pub(super) struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    pub(super) fn new(tokens: Vec<Tok>) -> Self {
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

    pub(super) fn parse_block(&mut self) -> Vec<Stmt> {
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