//! Lua サブセット AST 評価器 (Virtual Machine) および obj.* 標準 API 実装。

use std::collections::HashMap;
use std::f64::consts::PI;

use super::types::{
    BinOp, Expr, LValue, LuaDrawCall, LuaFilterCall, LuaLoadedPrimitive, LuaObjState, LuaPolyCall,
    LuaVal, Stmt, UnaryOp,
};

/// 1 回のスクリプト実行における最大命令ステップ数（無限ループ防止用）。
const MAX_STEPS: usize = 50_000;

pub(super) enum Flow {
    None,
    Break,
    Return(Vec<LuaVal>),
}

pub(super) struct Vm<'a> {
    obj: &'a mut LuaObjState,
    pub(super) globals: HashMap<String, LuaVal>,
    pub(super) scopes: Vec<HashMap<String, LuaVal>>,
    steps: usize,
    rng_state: u64,
    multi_return: Vec<LuaVal>,
}

impl<'a> Vm<'a> {
    pub(super) fn new(obj: &'a mut LuaObjState) -> Self {
        let mut globals = HashMap::new();
        for (k, v) in &obj.named_vars {
            globals.insert(k.clone(), v.clone());
        }
        if !globals.contains_key("global") {
            globals.insert("global".to_string(), LuaVal::Table(HashMap::new(), Vec::new()));
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
            "sx" => LuaVal::Num(self.obj.sx),
            "sy" => LuaVal::Num(self.obj.sy),
            "sz" => LuaVal::Num(self.obj.sz),
            "alpha" => LuaVal::Num(self.obj.alpha),
            "aspect" => LuaVal::Num(self.obj.aspect),
            "time" => LuaVal::Num(self.obj.time),
            "totaltime" => LuaVal::Num(self.obj.totaltime),
            "frame" => LuaVal::Num(self.obj.frame as f64),
            "totalframe" => LuaVal::Num(self.obj.totalframe as f64),
            "layer" => LuaVal::Num(self.obj.layer as f64),
            "index" => LuaVal::Num(self.obj.index as f64),
            "num" => LuaVal::Num(self.obj.num as f64),
            "id" => LuaVal::Num(self.obj.id as f64),
            "effect_id" => LuaVal::Num(self.obj.effect_id as f64),
            "frame_s" => LuaVal::Num(self.obj.frame_s as f64),
            "frame_e" => LuaVal::Num(self.obj.frame_e as f64),
            "effect_layer" => LuaVal::Num(self.obj.effect_layer as f64),
            "originframe" => LuaVal::Num(self.obj.originframe as f64),
            "scene_id" => LuaVal::Num(self.obj.scene_id as f64),
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
            "sx" => self.obj.sx = val.as_num(),
            "sy" => self.obj.sy = val.as_num(),
            "sz" => self.obj.sz = val.as_num(),
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

    pub(super) fn exec_block(&mut self, stmts: &[Stmt]) -> Flow {
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
            LValue::Index(Expr::Index(base_box, idx1_expr), idx2_expr) => {
                if let Expr::Var(base) = base_box.as_ref() {
                    let idx1_val = self.eval_expr(idx1_expr);
                    let idx2_val = self.eval_expr(idx2_expr);
                    let mut tbl = self.get_var(base);
                    if let LuaVal::Table(ref mut map1, ref mut arr1) = tbl {
                        let sub_tbl = if let LuaVal::Num(n1) = idx1_val {
                            let i1 = n1 as isize;
                            if i1 >= 1 {
                                let u1 = (i1 - 1) as usize;
                                if u1 >= arr1.len() && u1 < 4096 {
                                    arr1.resize(u1 + 1, LuaVal::Nil);
                                }
                                arr1.get_mut(u1)
                            } else {
                                map1.get_mut(&idx1_val.as_str())
                            }
                        } else {
                            map1.get_mut(&idx1_val.as_str())
                        };
                        if let Some(sub) = sub_tbl {
                            if matches!(sub, LuaVal::Nil) {
                                *sub = LuaVal::Table(HashMap::new(), Vec::new());
                            }
                            if let LuaVal::Table(ref mut map2, ref mut arr2) = sub {
                                if let LuaVal::Num(n2) = idx2_val {
                                    let i2 = n2 as isize;
                                    if i2 >= 1 {
                                        let u2 = (i2 - 1) as usize;
                                        if u2 >= arr2.len() && u2 < 4096 {
                                            arr2.resize(u2 + 1, LuaVal::Nil);
                                        }
                                        if u2 < arr2.len() {
                                            arr2[u2] = val;
                                        }
                                    } else {
                                        map2.insert(idx2_val.as_str(), val);
                                    }
                                } else {
                                    map2.insert(idx2_val.as_str(), val);
                                }
                            }
                        }
                        self.set_var(base, tbl);
                    }
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
                                "obj" => {
                                    let res = self.call_obj_func(func_name, &eval_args);
                                    if func_name == "getaudio" {
                                        if let Some(Expr::Var(buf_var)) = args.first() {
                                            if let Some(LuaVal::Table(m, a)) = self.multi_return.get(1).cloned() {
                                                self.set_var(buf_var, LuaVal::Table(m, a));
                                            }
                                        }
                                    }
                                    return res;
                                }
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
                if args.len() == 1 {
                    let col = a0() as u32;
                    let r = ((col >> 16) & 0xFF) as f64;
                    let g = ((col >> 8) & 0xFF) as f64;
                    let b = (col & 0xFF) as f64;
                    self.multi_return = vec![LuaVal::Num(g), LuaVal::Num(b)];
                    Some(LuaVal::Num(r))
                } else if args.len() >= 6 {
                    let ratio = if self.obj.totaltime > 0.0 {
                        (self.obj.time / self.obj.totaltime).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let r1 = a0().clamp(0.0, 255.0);
                    let g1 = a1().clamp(0.0, 255.0);
                    let b1 = a2().clamp(0.0, 255.0);
                    let r2 = args.get(3).map(|v| v.as_num()).unwrap_or(0.0).clamp(0.0, 255.0);
                    let g2 = args.get(4).map(|v| v.as_num()).unwrap_or(0.0).clamp(0.0, 255.0);
                    let b2 = args.get(5).map(|v| v.as_num()).unwrap_or(0.0).clamp(0.0, 255.0);
                    let r = (r1 + (r2 - r1) * ratio).round() as u32;
                    let g = (g1 + (g2 - g1) * ratio).round() as u32;
                    let b = (b1 + (b2 - b1) * ratio).round() as u32;
                    Some(LuaVal::Num(((r << 16) | (g << 8) | b) as f64))
                } else {
                    let r = (a0() as u32).min(255);
                    let g = (a1() as u32).min(255);
                    let b = (a2() as u32).min(255);
                    Some(LuaVal::Num(((r << 16) | (g << 8) | b) as f64))
                }
            }
            "HSV" => {
                let hsv_to_rgb_u32 = |h_in: f64, s_in: f64, v_in: f64| -> u32 {
                    let h = h_in.rem_euclid(360.0);
                    let s = (s_in / 100.0).clamp(0.0, 1.0);
                    let v = (v_in / 100.0).clamp(0.0, 1.0);
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
                    (r << 16) | (g << 8) | b
                };
                if args.len() == 1 {
                    let col = a0() as u32;
                    let r = ((col >> 16) & 0xFF) as f64 / 255.0;
                    let g = ((col >> 8) & 0xFF) as f64 / 255.0;
                    let b = (col & 0xFF) as f64 / 255.0;
                    let max = r.max(g).max(b);
                    let min = r.min(g).min(b);
                    let d = max - min;
                    let h = if d < 1e-6 {
                        0.0
                    } else if (max - r).abs() < 1e-6 {
                        60.0 * (((g - b) / d).rem_euclid(6.0))
                    } else if (max - g).abs() < 1e-6 {
                        60.0 * (((b - r) / d) + 2.0)
                    } else {
                        60.0 * (((r - g) / d) + 4.0)
                    };
                    let s = if max < 1e-6 { 0.0 } else { (d / max) * 100.0 };
                    let v = max * 100.0;
                    self.multi_return = vec![LuaVal::Num(s.round()), LuaVal::Num(v.round())];
                    Some(LuaVal::Num(h.round()))
                } else if args.len() >= 6 {
                    let ratio = if self.obj.totaltime > 0.0 {
                        (self.obj.time / self.obj.totaltime).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let h = a0() + (args.get(3).map(|v| v.as_num()).unwrap_or(0.0) - a0()) * ratio;
                    let s = a1() + (args.get(4).map(|v| v.as_num()).unwrap_or(0.0) - a1()) * ratio;
                    let v = a2() + (args.get(5).map(|v| v.as_num()).unwrap_or(0.0) - a2()) * ratio;
                    Some(LuaVal::Num(hsv_to_rgb_u32(h, s, v) as f64))
                } else {
                    Some(LuaVal::Num(hsv_to_rgb_u32(a0(), a1(), a2()) as f64))
                }
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
            "rand1" => Some(self.call_obj_func("rand1", args)),
            "mes" => Some(self.call_obj_func("mes", args)),
            "load" => Some(self.call_obj_func("load", args)),
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
            "rand1" => {
                // AviUtl2 新規追加関数: 0.0 以上 1.0 以下の浮動小数点乱数を返す `obj.rand1([seed, frame])`
                let seed = args.first().map(|v| v.as_num() as i64).unwrap_or(0);
                let frm = args.get(1).map(|v| v.as_num() as i64).unwrap_or(self.obj.frame);
                let mut h = (frm as u64)
                    .wrapping_mul(0x9E3779B97F4A7C15)
                    .wrapping_add((seed as u64).wrapping_mul(0xBF58476D1CE4E5B9))
                    .wrapping_add((self.obj.index as u64).wrapping_mul(0x94D049BB133111EB));
                h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                h = (h ^ (h >> 27)).wrapping_mul(0x94D049BB133111EB);
                h ^= h >> 31;
                let ratio = ((h & 0x1F_FFFF_FFFF_FFFF) as f64) / ((0x1F_FFFF_FFFF_FFFFu64) as f64);
                LuaVal::Num(ratio.clamp(0.0, 1.0))
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
                    "cx" => LuaVal::Num(self.obj.cx),
                    "cy" => LuaVal::Num(self.obj.cy),
                    "cz" => LuaVal::Num(self.obj.cz),
                    "sx" => LuaVal::Num(self.obj.sx),
                    "sy" => LuaVal::Num(self.obj.sy),
                    "sz" => LuaVal::Num(self.obj.sz),
                    "zoom" => LuaVal::Num(self.obj.zoom * 100.0),
                    "alpha" => LuaVal::Num(self.obj.alpha),
                    "aspect" => LuaVal::Num(self.obj.aspect),
                    "time" => LuaVal::Num(self.obj.time),
                    "totaltime" => LuaVal::Num(self.obj.totaltime),
                    "frame_s" => LuaVal::Num(self.obj.frame_s as f64),
                    "frame_e" => LuaVal::Num(self.obj.frame_e as f64),
                    "pos" => {
                        self.multi_return = vec![
                            LuaVal::Num(self.obj.y + self.obj.oy),
                            LuaVal::Num(self.obj.z + self.obj.oz),
                        ];
                        LuaVal::Num(self.obj.x + self.obj.ox)
                    }
                    "angle" => {
                        self.multi_return = vec![LuaVal::Num(self.obj.ry), LuaVal::Num(self.obj.rz)];
                        LuaVal::Num(self.obj.rx)
                    }
                    "center" => {
                        self.multi_return = vec![LuaVal::Num(self.obj.cy), LuaVal::Num(self.obj.cz)];
                        LuaVal::Num(self.obj.cx)
                    }
                    "scale" => {
                        self.multi_return = vec![
                            LuaVal::Num(self.obj.zoom * self.obj.sy),
                            LuaVal::Num(self.obj.zoom * self.obj.sz),
                        ];
                        LuaVal::Num(self.obj.zoom * self.obj.sx)
                    }
                    "scenechange" => {
                        let ratio = if self.obj.totaltime > 0.0 {
                            (self.obj.time / self.obj.totaltime).clamp(0.0, 1.0)
                        } else {
                            0.5
                        };
                        LuaVal::Num(ratio)
                    }
                    other => {
                        if let Some(var_name) = other.strip_prefix("track.") {
                            if let Some(v) = self.obj.named_vars.get(var_name) {
                                return LuaVal::Num(v.as_num());
                            }
                        }
                        if let Some(idx_str) = other.strip_prefix("track") {
                            if let Ok(idx) = idx_str.parse::<usize>() {
                                return LuaVal::Num(*self.obj.tracks.get(idx).unwrap_or(&0.0));
                            }
                        }
                        if let Some(v) = self.obj.named_vars.get(other) {
                            return LuaVal::Num(v.as_num());
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
                if let Some(ft) = args.get(2) {
                    self.obj.font_type = ft.as_num() as i64;
                }
                if let Some(c1) = args.get(3) {
                    self.obj.font_color = c1.as_num() as u32;
                }
                if let Some(c2) = args.get(4) {
                    self.obj.font_color2 = c2.as_num() as u32;
                }
                if let Some(bold) = args.get(5) {
                    self.obj.font_bold = bold.truthy();
                }
                if let Some(italic) = args.get(6) {
                    self.obj.font_italic = italic.truthy();
                }
                if let Some(cs) = args.get(7) {
                    self.obj.font_charspacing = cs.as_num();
                }
                if let Some(ls) = args.get(8) {
                    self.obj.font_linespacing = ls.as_num();
                }
                LuaVal::Nil
            }
            "getfont" => {
                self.multi_return = vec![
                    LuaVal::Num(self.obj.font_size),
                    LuaVal::Num(self.obj.font_type as f64),
                    LuaVal::Num(self.obj.font_color as f64),
                    LuaVal::Num(self.obj.font_color2 as f64),
                    LuaVal::Bool(self.obj.font_bold),
                    LuaVal::Bool(self.obj.font_italic),
                    LuaVal::Num(self.obj.font_charspacing),
                    LuaVal::Num(self.obj.font_linespacing),
                ];
                LuaVal::Str(self.obj.font_name.clone())
            }
            "multiobject" => {
                // AviUtl2 新規追加関数: `obj.multiobject(num, func)`
                let count = args.first().map(|v| v.as_num() as i64).unwrap_or(1).clamp(1, 256);
                let func_val = args.get(1).cloned().unwrap_or(LuaVal::Nil);
                if let LuaVal::Func(params, body) = func_val {
                    let saved_ox = self.obj.ox;
                    let saved_oy = self.obj.oy;
                    let saved_oz = self.obj.oz;
                    let saved_rx = self.obj.rx;
                    let saved_ry = self.obj.ry;
                    let saved_rz = self.obj.rz;
                    let saved_zoom = self.obj.zoom;
                    let saved_sx = self.obj.sx;
                    let saved_sy = self.obj.sy;
                    let saved_sz = self.obj.sz;
                    let saved_alpha = self.obj.alpha;
                    let saved_idx = self.obj.index;
                    let saved_num = self.obj.num;

                    self.obj.num = count;
                    for i in 0..count {
                        self.obj.index = i;
                        self.obj.ox = saved_ox;
                        self.obj.oy = saved_oy;
                        self.obj.oz = saved_oz;
                        self.obj.rx = saved_rx;
                        self.obj.ry = saved_ry;
                        self.obj.rz = saved_rz;
                        self.obj.zoom = saved_zoom;
                        self.obj.sx = saved_sx;
                        self.obj.sy = saved_sy;
                        self.obj.sz = saved_sz;
                        self.obj.alpha = saved_alpha;

                        let _ = self.invoke_user_func(LuaVal::Func(params.clone(), body.clone()), Vec::new());

                        if self.obj.draw_calls.len() < 512 {
                            let scale = self.obj.zoom * ((self.obj.sx + self.obj.sy) * 0.5).max(0.0);
                            self.obj.draw_calls.push(LuaDrawCall {
                                ox: self.obj.ox,
                                oy: self.obj.oy,
                                oz: self.obj.oz,
                                zoom: scale,
                                alpha: self.obj.alpha.clamp(0.0, 1.0),
                                rx: self.obj.rx,
                                ry: self.obj.ry,
                                rz: self.obj.rz,
                                color: None,
                                primitive: self.obj.loaded_primitive.clone(),
                                clip: self.obj.current_clip,
                            });
                        }
                    }

                    self.obj.ox = saved_ox;
                    self.obj.oy = saved_oy;
                    self.obj.oz = saved_oz;
                    self.obj.rx = saved_rx;
                    self.obj.ry = saved_ry;
                    self.obj.rz = saved_rz;
                    self.obj.zoom = saved_zoom;
                    self.obj.sx = saved_sx;
                    self.obj.sy = saved_sy;
                    self.obj.sz = saved_sz;
                    self.obj.alpha = saved_alpha;
                    self.obj.index = saved_idx;
                    self.obj.num = saved_num;
                    LuaVal::Bool(true)
                } else {
                    LuaVal::Bool(false)
                }
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
                    "text" | "text.layout" | "textlayout" => {
                        let text = args.get(1).map(|v| v.as_str()).unwrap_or_default();
                        let line_cnt = text.lines().count().max(1) as f64;
                        let max_chars = text.lines().map(|l| l.chars().count()).max().unwrap_or(1) as f64;
                        self.obj.w = (max_chars * self.obj.font_size).max(self.obj.font_size);
                        self.obj.h = (line_cnt * self.obj.font_size * 1.15).max(self.obj.font_size);
                        self.obj.loaded_primitive = Some(LuaLoadedPrimitive::Text {
                            text,
                            font: self.obj.font_name.clone(),
                            size: self.obj.font_size,
                            color: self.obj.font_color,
                            color2: self.obj.font_color2,
                        });
                    }
                    "image" | "movie" | "movie.frame" | "movie.info" => {
                        let path = args.get(1).map(|v| v.as_str()).unwrap_or_default();
                        self.obj.loaded_primitive = Some(LuaLoadedPrimitive::Image { path });
                        if kind == "movie.info" {
                            self.multi_return = vec![
                                LuaVal::Num(300.0),
                                LuaVal::Num(self.obj.w),
                                LuaVal::Num(self.obj.h),
                            ];
                            return LuaVal::Num(10.0);
                        }
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
                    "camera_focus" => {
                        self.multi_return = vec![LuaVal::Num(10000.0), LuaVal::Num(0.0)];
                        LuaVal::Num(self.obj.cam_d)
                    }
                    "script_name" => LuaVal::Str("Script".into()),
                    "group_info" => {
                        let mut map = HashMap::new();
                        map.insert("x".into(), LuaVal::Num(0.0));
                        map.insert("y".into(), LuaVal::Num(0.0));
                        map.insert("z".into(), LuaVal::Num(0.0));
                        map.insert("rx".into(), LuaVal::Num(0.0));
                        map.insert("ry".into(), LuaVal::Num(0.0));
                        map.insert("rz".into(), LuaVal::Num(0.0));
                        map.insert("sx".into(), LuaVal::Num(1.0));
                        map.insert("sy".into(), LuaVal::Num(1.0));
                        map.insert("sz".into(), LuaVal::Num(1.0));
                        LuaVal::Table(map, Vec::new())
                    }
                    "blend" => LuaVal::Num(self.obj.blend_mode as f64),
                    "culling" => LuaVal::Num(if self.obj.culling { 1.0 } else { 0.0 }),
                    "billboard" => LuaVal::Num(self.obj.billboard as f64),
                    "drawtarget" => LuaVal::Str("framebuffer".into()),
                    "draw_state" => LuaVal::Bool(!self.obj.draw_calls.is_empty()),
                    "enable_group" | "enable_camera" => LuaVal::Bool(true),
                    "clipping_object" | "clipping_upper_object" => LuaVal::Bool(false),
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
            // ------------------------------------------------------------------------
            // [アンカー制御 API]
            // ------------------------------------------------------------------------
            "setanchor" => {
                let var_name = args.first().map(|v| v.as_str()).unwrap_or_default();
                let num = args.get(1).map(|v| v.as_num() as usize).unwrap_or(1).clamp(1, 16);
                let is_xyz = args.iter().skip(2).any(|v| v.as_str().to_ascii_lowercase().contains("xyz"));
                let stride = if is_xyz { 3 } else { 2 };
                if !var_name.is_empty() {
                    let hw = (self.obj.w * 0.5).max(50.0);
                    let hh = (self.obj.h * 0.5).max(50.0);
                    let default_corners = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)];
                    let mut tbl = self.get_var(&var_name);
                    if matches!(tbl, LuaVal::Nil) {
                        tbl = LuaVal::Table(HashMap::new(), Vec::new());
                    }
                    if let LuaVal::Table(map, mut arr) = tbl {
                        let needed = num * stride;
                        for i in 0..num {
                            let (dx, dy) = default_corners[i % 4];
                            let base = i * stride;
                            if base >= arr.len() {
                                arr.push(LuaVal::Num(dx));
                            }
                            if base + 1 >= arr.len() {
                                arr.push(LuaVal::Num(dy));
                            }
                            if is_xyz && base + 2 >= arr.len() {
                                arr.push(LuaVal::Num(0.0));
                            }
                        }
                        if arr.len() < needed {
                            arr.resize(needed, LuaVal::Num(0.0));
                        }
                        self.set_var(&var_name, LuaVal::Table(map, arr));
                    }
                }
                LuaVal::Num(num as f64)
            }

            // ------------------------------------------------------------------------
            // [仮想バッファ操作 API (残像・一時退避・多重露光)]
            // クロスプラットフォーム仕様: Win32 GDI を排除し Pure Rust 構造体で管理
            // ------------------------------------------------------------------------
            "copybuffer" => {
                let dst = args.first().map(|v| v.as_str()).unwrap_or_else(|| "cache".into());
                let src = args.get(1).map(|v| v.as_str()).unwrap_or_else(|| "obj".into());

                // コピー元バッファを抽出
                let src_buf = if src == "obj" {
                    Some(self.obj.snapshot_to_buffer())
                } else {
                    self.obj.buffers.get(&src).cloned()
                };

                if let Some(buf) = src_buf {
                    if dst == "obj" {
                        self.obj.restore_from_buffer(&buf);
                    } else {
                        self.obj.buffers.insert(dst, buf);
                    }
                    LuaVal::Bool(true)
                } else {
                    LuaVal::Bool(false)
                }
            }
            "clearbuffer" => {
                let target = args.first().map(|v| v.as_str()).unwrap_or_else(|| "cache".into());
                if target == "obj" {
                    self.obj.draw_calls.clear();
                    self.obj.poly_calls.clear();
                    self.obj.filter_calls.clear();
                } else {
                    self.obj.buffers.remove(&target);
                }
                LuaVal::Bool(true)
            }

            // ------------------------------------------------------------------------
            // [ピクセルデータ・補間法 API (サイズ取得フォールバック / 生ピクセル操作は no-op)]
            // ※ 本エンジンはベクター/メッシュプリミティブ合成方式のため、Cポインタや生RGBA配列の直接書き換えは行いません。
            //    スクリプトが `local data, w, h = obj.getpixeldata()` で画像サイズ (w, h) を参照するケースのみ維持します。
            // ------------------------------------------------------------------------
            "pixeloption" => {
                if let Some(val) = args.get(1).or_else(|| args.first()) {
                    let s = val.as_str();
                    if s != "type" {
                        self.obj.pixel_option = s;
                    }
                }
                LuaVal::Str(self.obj.pixel_option.clone())
            }
            "getpixel" => {
                if args.is_empty() {
                    // 引数なし時は画像サイズ (w, h) をマルチリターン
                    self.multi_return = vec![LuaVal::Num(self.obj.h)];
                    LuaVal::Num(self.obj.w)
                } else {
                    // (x, y) 座標のピクセル色 (RGB) とアルファ (0.0〜1.0)
                    self.multi_return = vec![LuaVal::Num(self.obj.alpha)];
                    LuaVal::Num(0xFFFFFF as f64)
                }
            }
            "putpixel" => {
                // (x, y, col, a)
                LuaVal::Nil
            }
            "copypixel" => {
                // (dst_x, dst_y, src_x, src_y, w, h)
                LuaVal::Nil
            }
            "getpixeldata" => {
                let data_type = args.first().map(|v| v.as_str()).unwrap_or_else(|| "col".into());
                let mut map = HashMap::new();
                map.insert("type".into(), LuaVal::Str(data_type));
                map.insert("w".into(), LuaVal::Num(self.obj.w));
                map.insert("h".into(), LuaVal::Num(self.obj.h));
                self.multi_return = vec![LuaVal::Num(self.obj.w), LuaVal::Num(self.obj.h)];
                LuaVal::Table(map, Vec::new())
            }
            "putpixeldata" => {
                if let Some(LuaVal::Table(map, _)) = args.first() {
                    if let Some(w) = map.get("w") {
                        self.obj.w = w.as_num().max(1.0);
                    }
                    if let Some(h) = map.get("h") {
                        self.obj.h = h.as_num().max(1.0);
                    }
                }
                LuaVal::Bool(true)
            }
            "pixelshader" | "computeshader" => LuaVal::Bool(true),
            "data" | "module" => LuaVal::Table(HashMap::new(), Vec::new()),
            "getaudio" => {
                let n = args.get(3).or_else(|| args.get(2)).map(|v| v.as_num() as usize).unwrap_or(64).clamp(1, 512);
                let arr: Vec<LuaVal> = (0..n)
                    .map(|i| {
                        let v = ((self.obj.time * 8.0 + i as f64 * 0.4).sin().abs() * 500.0).round();
                        LuaVal::Num(v)
                    })
                    .collect();
                self.multi_return = vec![LuaVal::Num(44100.0), LuaVal::Table(HashMap::new(), arr)];
                LuaVal::Num(n as f64)
            }
            "getinfo" => {
                let key = args.first().map(|v| v.as_str()).unwrap_or_default();
                match key.as_str() {
                    "version" => {
                        self.multi_return = vec![LuaVal::Str("2.1.16".into())];
                        LuaVal::Num(2011600.0)
                    }
                    "script_path" => LuaVal::Str(String::new()),
                    "filter" | "saving" => LuaVal::Bool(false),
                    "origin_scene_id" => LuaVal::Num(self.obj.scene_id as f64),
                    "reference_frame" => LuaVal::Num(self.obj.frame as f64),
                    "rendering_id" => LuaVal::Num(1.0),
                    "image_max" => {
                        self.multi_return = vec![LuaVal::Num(4096.0)];
                        LuaVal::Num(4096.0)
                    }
                    "frame_max" => LuaVal::Num(self.obj.totalframe.max(1000) as f64),
                    "layer_max" => LuaVal::Num(100.0),
                    "bpm" => {
                        self.multi_return = vec![
                            LuaVal::Num(4.0),
                            LuaVal::Num(4.0),
                            LuaVal::Num(0.0),
                            LuaVal::Num(1.0),
                            LuaVal::Num(1.0),
                            LuaVal::Num(0.0),
                        ];
                        LuaVal::Num(120.0)
                    }
                    "bpm_list" => LuaVal::Table(HashMap::new(), Vec::new()),
                    "clock" | "script_time" => LuaVal::Num(self.obj.time),
                    _ => LuaVal::Num(0.0),
                }
            }
            "getpoint" => {
                let first = args.first().cloned().unwrap_or(LuaVal::Nil);
                match first {
                    LuaVal::Str(cmd) => match cmd.as_str() {
                        "num" => LuaVal::Num(self.obj.point_values.len().max(2) as f64),
                        "index" => {
                            let idx = self.obj.point_index;
                            let frac = if self.obj.point_totaltime > 0.0 {
                                (self.obj.point_time / self.obj.point_totaltime).clamp(0.0, 1.0)
                            } else {
                                0.0
                            };
                            self.multi_return = vec![LuaVal::Num(frac)];
                            LuaVal::Num((idx as f64) + frac)
                        }
                        "time" => LuaVal::Num(self.obj.point_time),
                        "totaltime" => LuaVal::Num(self.obj.point_totaltime),
                        "framerate" => LuaVal::Num(self.obj.framerate),
                        "accelerate" => LuaVal::Bool(self.obj.point_accelerate),
                        "decelerate" => LuaVal::Bool(self.obj.point_decelerate),
                        "param" => LuaVal::Num(self.obj.point_param),
                        "link" => {
                            let dim = args.get(1).map(|v| v.as_num() as usize).unwrap_or(0);
                            LuaVal::Num(dim as f64)
                        }
                        "default" => {
                            let p0 = self.obj.point_values.get(self.obj.point_index).copied().unwrap_or(0.0);
                            let p1 = self
                                .obj
                                .point_values
                                .get(self.obj.point_index + 1)
                                .copied()
                                .unwrap_or(p0);
                            let ratio = if self.obj.point_totaltime > 0.0 {
                                (self.obj.point_time / self.obj.point_totaltime).clamp(0.0, 1.0)
                            } else {
                                0.0
                            };
                            LuaVal::Num(p0 + (p1 - p0) * ratio)
                        }
                        _ => LuaVal::Num(0.0),
                    },
                    LuaVal::Num(idx_f) => {
                        let base_idx = idx_f.floor().max(0.0) as usize;
                        let val = self
                            .obj
                            .point_values
                            .get(base_idx)
                            .copied()
                            .or_else(|| self.obj.point_values.last().copied())
                            .unwrap_or(0.0);
                        LuaVal::Num(val)
                    }
                    _ => LuaVal::Num(0.0),
                }
            }
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