//! 初代 AviUtl (拡張編集) および AviUtl2 (ExEdit2) スクリプト向けの組み込み Lua サブセット実行エンジン。
//!
//! ※ 公式の Lua 5.1 VM / LuaJIT ではなく、Rust による軽量な独自 AST インタプリタ実装です。
//! `.anm`, `.obj`, `.cam`, `.scn`, `.tra`（および `.anm2`, `.obj2`, `.cam2`, `.scn2`, `.tra2`）内の
//! 基本構文および `obj.*` プロパティ変更・`obj.draw()`・`obj.drawpoly()`・`obj.load()`・
//! `obj.mes()`・`obj.effect()` 等の描画コマンドを評価します。
//! 外部 C/DLL モジュール（`require` / `package.loadlib`）や生ピクセルバッファの直接メモリ書き換えには非対応です。

mod parser;
mod types;
mod vm;

pub use types::{
    BinOp, Expr, LValue, LuaDrawCall, LuaFilterCall, LuaLoadedPrimitive, LuaObjState, LuaPolyCall,
    LuaVal, LuaVirtualBuffer, Stmt, UnaryOp,
};

use parser::{tokenize, Parser};
use vm::{Flow, Vm};

/// Lua スクリプトを事前コンパイル（Tokenize + Parse）して構文木 (AST) を返します。
pub fn compile_lua_script(source: &str) -> Vec<Stmt> {
    let tokens = tokenize(source);
    let mut parser = Parser::new(tokens);
    parser.parse_block()
}

/// コンパイル済み構文木 (AST) を `LuaObjState` に対して直接実行します。
/// 毎フレームの字句解析・パース・ヒープ確保を完全にスキップするため極めて高速です。
pub fn execute_compiled_script(stmts: &[Stmt], state: &mut LuaObjState) {
    let (ret, globals) = {
        let mut vm = Vm::new(state);
        let flow = vm.exec_block(stmts);
        let ret = if let Flow::Return(vals) = flow {
            vals.first().map(|v| v.as_num())
        } else {
            None
        };
        (ret, vm.globals)
    };
    if let Some(r) = ret {
        state.return_value = Some(r);
    }
    for (k, v) in globals {
        state.named_vars.insert(k, v);
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

    #[test]
    fn test_copybuffer_pixeloption_and_getpixeldata() {
        let script = r#"
            obj.load("figure", "四角形", 0xFF00FF, 120)
            obj.draw(10, 20, 0, 1.5, 0.8)
            -- 現在の obj 状態を cache バッファへ退避
            local ok1 = obj.copybuffer("cache", "obj")
            assert_cache = ok1

            -- obj をクリア
            obj.clearbuffer("obj")

            -- cache バッファから obj へ復元
            local ok2 = obj.copybuffer("obj", "cache")
            assert_restored = ok2

            -- 補間法の設定と取得
            obj.pixeloption("type", "bicubic")

            -- ピクセルデータ情報取得 (data, w, h)
            data, w, h = obj.getpixeldata("col")
        "#;
        let mut st = LuaObjState::default();
        execute_lua_script(script, &mut st);

        assert_eq!(st.named_vars.get("assert_cache"), Some(&LuaVal::Bool(true)));
        assert_eq!(st.named_vars.get("assert_restored"), Some(&LuaVal::Bool(true)));
        assert_eq!(st.draw_calls.len(), 1);
        assert_eq!(st.w, 120.0);
        assert_eq!(st.h, 120.0);
        assert_eq!(st.pixel_option, "bicubic");
        assert_eq!(st.named_vars.get("w"), Some(&LuaVal::Num(120.0)));
        assert_eq!(st.named_vars.get("h"), Some(&LuaVal::Num(120.0)));
    }
}