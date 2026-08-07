//! Integration tests for the Lua sandbox.
//! Verifies that dangerous APIs are inaccessible after sandboxing,
//! and that the safe subset works correctly.

use mlua::prelude::*;

fn make_sandboxed_lua() -> Lua {
    let lua = Lua::new();
    oxigeon::core::scripting::sandbox::apply_sandbox(&lua).unwrap();
    lua
}

#[test]
fn test_sandbox_blocks_io_module() {
    let lua = make_sandboxed_lua();
    // io should be nil at globals level
    let result = lua.load("return io").eval::<LuaValue>().unwrap();
    assert!(matches!(result, LuaValue::Nil), "io should be nil in sandbox");
}

#[test]
fn test_sandbox_blocks_io_open_access() {
    let lua = make_sandboxed_lua();
    // Attempting to use io.open should error (io is nil)
    let result = lua.load("return type(io)").eval::<String>().unwrap();
    assert_eq!(result, "nil");
}

#[test]
fn test_sandbox_blocks_debug_module() {
    let lua = make_sandboxed_lua();
    let result = lua.load("return debug").eval::<LuaValue>().unwrap();
    assert!(matches!(result, LuaValue::Nil), "debug should be nil in sandbox");
}

#[test]
fn test_sandbox_blocks_os_execute() {
    let lua = make_sandboxed_lua();
    // os.execute is removed
    let result = lua.load("return os.execute").eval::<LuaValue>().unwrap();
    assert!(matches!(result, LuaValue::Nil), "os.execute should be nil");
}

#[test]
fn test_sandbox_blocks_os_exit() {
    let lua = make_sandboxed_lua();
    let result = lua.load("return os.exit").eval::<LuaValue>().unwrap();
    assert!(matches!(result, LuaValue::Nil), "os.exit should be nil");
}

#[test]
fn test_sandbox_blocks_loadfile() {
    let lua = make_sandboxed_lua();
    let result = lua.load("return loadfile").eval::<LuaValue>().unwrap();
    assert!(matches!(result, LuaValue::Nil), "loadfile should be nil");
}

#[test]
fn test_sandbox_blocks_dofile() {
    let lua = make_sandboxed_lua();
    let result = lua.load("return dofile").eval::<LuaValue>().unwrap();
    assert!(matches!(result, LuaValue::Nil), "dofile should be nil");
}

#[test]
fn test_sandbox_allows_string_operations() {
    let lua = make_sandboxed_lua();
    let result: String = lua.load(r#"return string.upper("hello")"#).eval().unwrap();
    assert_eq!(result, "HELLO");
}

#[test]
fn test_sandbox_allows_math_operations() {
    let lua = make_sandboxed_lua();
    let result: f64 = lua.load("return math.sqrt(16)").eval().unwrap();
    assert!((result - 4.0).abs() < 1e-10);
}

#[test]
fn test_sandbox_allows_table_operations() {
    let lua = make_sandboxed_lua();
    let result: i64 = lua.load(r#"
        local t = {10, 20, 30}
        table.insert(t, 40)
        return #t
    "#).eval().unwrap();
    assert_eq!(result, 4);
}

#[test]
fn test_sandbox_allows_pcall_error_handling() {
    let lua = make_sandboxed_lua();
    let result: bool = lua.load(r#"
        local ok, err = pcall(function()
            error("deliberate test error")
        end)
        return ok
    "#).eval().unwrap();
    assert!(!result, "pcall should return false when error is thrown");
}

#[test]
fn test_sandbox_allows_coroutine() {
    let lua = make_sandboxed_lua();
    let result: String = lua.load(r#"
        local co = coroutine.create(function()
            coroutine.yield("yielded")
        end)
        local ok, val = coroutine.resume(co)
        return val
    "#).eval().unwrap();
    assert_eq!(result, "yielded");
}
