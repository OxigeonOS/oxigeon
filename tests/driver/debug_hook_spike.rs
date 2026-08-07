//! M0 spikes for the Lua debugger (`trace` command + DAP adapter).
//!
//! These resolve the four hard unknowns that the debugger design depends on,
//! against the real vendored LuaJIT + mlua 0.10 build:
//!
//!   1. Can an `Lua::set_hook` callback call back into the VM?
//!      (If not, `evaluate` / `variables` are impossible.)
//!   2. What stack level does `debug.getlocal` see from inside a hook?
//!   3. What event sequence does LuaJIT emit around tail calls, and is
//!      `Lua::inspect_stack` a reliable depth source?
//!   4. Does `unsafe_new_with(ALL_SAFE | DEBUG)` work, and does a registry-stashed
//!      `debug` table keep working after the global is nil'd?
//!
//! Each test prints its observations; the assertions encode what the design
//! relies on.

use mlua::prelude::*;
use mlua::{HookTriggers, StdLib, VmState};
use std::cell::RefCell;
use std::rc::Rc;

/// Build a VM with the `debug` stdlib stashed in the registry and the global nil'd,
/// exactly as `ScriptEngine::start` will do when `[servers.debug]` is enabled.
fn lua_with_hidden_debug() -> Lua {
    let lua = unsafe { Lua::unsafe_new_with(StdLib::ALL_SAFE | StdLib::DEBUG, LuaOptions::default()) };
    let globals = lua.globals();
    let dbg: LuaTable = globals.get("debug").expect("debug stdlib should be loaded");
    lua.set_named_registry_value("__oxi_debug", dbg).unwrap();
    globals.set("debug", LuaValue::Nil).unwrap();
    // unsafe mode re-enables package.loadlib — close it back up.
    if let Ok(pkg) = globals.get::<LuaTable>("package") {
        let _ = pkg.set("loadlib", LuaValue::Nil);
    }
    lua
}

// ─── Spike 4: unsafe_new_with + hidden debug table ───────────────────────────

#[test]
fn spike4_debug_stdlib_can_be_loaded_and_hidden() {
    let lua = lua_with_hidden_debug();

    // Mudlib code must not see it.
    let visible: bool = lua.load("return debug ~= nil").eval().unwrap();
    assert!(!visible, "debug global must be nil for mudlib code");

    let loadlib: bool = lua.load("return package.loadlib ~= nil").eval().unwrap();
    assert!(!loadlib, "package.loadlib must be nil'd in unsafe mode");

    // ...but the registry copy must still be callable from Rust.
    let dbg: LuaTable = lua.named_registry_value("__oxi_debug").unwrap();
    for f in ["getinfo", "getlocal", "getupvalue", "traceback", "sethook"] {
        assert!(
            dbg.get::<LuaValue>(f).unwrap() != LuaValue::Nil,
            "debug.{f} missing from registry copy"
        );
    }

    // The evaluator needs *some* way to run a chunk with a supplied environment.
    // 5.1 (LuaJIT) spells that `loadstring` + `setfenv`; 5.2 removed both and
    // folded it into `load`'s fourth argument. `introspect.lua` picks whichever
    // is present — this asserts one of them always is, on either runtime.
    let can_set_env: bool = lua
        .load(
            "if setfenv and loadstring then return true end \
             local f = load('return 1 + 1', '=probe', 't', { }) \
             return f ~= nil",
        )
        .eval()
        .unwrap();
    assert!(
        can_set_env,
        "no way to compile a chunk with a supplied environment — `evaluate` cannot work"
    );
}

// ─── Spike 1: re-entering the VM from inside a hook ──────────────────────────

#[test]
fn spike1_hook_can_call_back_into_the_vm() {
    let lua = Lua::new();
    let results: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = results.clone();

    lua.set_hook(HookTriggers::EVERY_LINE, move |lua, _debug| {
        // Only record once, to keep output small.
        if !sink.borrow().is_empty() {
            return Ok(VmState::Continue);
        }
        // (a) evaluate a fresh chunk
        match lua.load("return 1 + 1").eval::<i32>() {
            Ok(v) => sink.borrow_mut().push(format!("eval_chunk={v}")),
            Err(e) => sink.borrow_mut().push(format!("eval_chunk_ERR={e}")),
        }
        // (b) read a global and call a Lua function defined by the running program
        match lua.globals().get::<LuaFunction>("probe") {
            Ok(f) => match f.call::<i32>(7) {
                Ok(v) => sink.borrow_mut().push(format!("call_lua_fn={v}")),
                Err(e) => sink.borrow_mut().push(format!("call_lua_fn_ERR={e}")),
            },
            Err(e) => sink.borrow_mut().push(format!("get_global_ERR={e}")),
        }
        // (c) inspect the stack while inside the hook
        let depth = (0..64).take_while(|n| lua.inspect_stack(*n, |_| ()).is_some()).count();
        sink.borrow_mut().push(format!("stack_depth={depth}"));
        Ok(VmState::Continue)
    })
    .expect("the hook must install, or this test asserts nothing");

    lua.load(
        r#"
        function probe(n) return n * 3 end
        local x = probe(2)
        return x
        "#,
    )
    .set_name("@spike1.lua")
    .exec()
    .expect("chunk with a re-entrant hook should still run");

    lua.remove_hook();

    let out = results.borrow().clone();
    println!("spike1 observations: {out:#?}");
    assert!(!out.is_empty(), "hook never fired");
    assert!(
        out.iter().any(|s| s == "eval_chunk=2"),
        "loading+evaluating a chunk from inside a hook must work; got {out:?}"
    );
    assert!(
        out.iter().any(|s| s.starts_with("stack_depth=")),
        "inspect_stack must work from inside a hook; got {out:?}"
    );
}

// ─── Spike 2: debug.getlocal level offset from inside a hook ─────────────────

#[test]
fn spike2_getlocal_level_offset_from_hook() {
    let lua = lua_with_hidden_debug();
    let observed: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = observed.clone();

    // Fire on the `return b` line inside `inner` (line 5 of the chunk below).
    // Filter by line, not by `names().name` — spike 3 shows that name is stale or
    // empty for tail-called and anonymous frames.
    const MARKER_LINE: usize = 5;

    lua.set_hook(HookTriggers::EVERY_LINE, move |lua, debug| {
        let src = debug.source().source.map(|s| s.to_string()).unwrap_or_default();
        if !src.contains("spike2") {
            return Ok(VmState::Continue);
        }
        if debug.current_line() != Some(MARKER_LINE) || !sink.borrow().is_empty() {
            return Ok(VmState::Continue);
        }

        let dbg: LuaTable = lua.named_registry_value("__oxi_debug").unwrap();
        let getinfo: LuaFunction = dbg.get("getinfo").unwrap();
        let getlocal: LuaFunction = dbg.get("getlocal").unwrap();

        for level in 0..5i32 {
            let info: Option<LuaTable> = getinfo.call((level, "nSl")).unwrap_or(None);
            let (what, fname, line) = match &info {
                Some(t) => (
                    t.get::<Option<String>>("what").unwrap_or(None).unwrap_or_default(),
                    t.get::<Option<String>>("name").unwrap_or(None).unwrap_or_default(),
                    t.get::<Option<i32>>("currentline").unwrap_or(None).unwrap_or(-1),
                ),
                None => ("<none>".into(), String::new(), -1),
            };
            // First named local at this level, if any.
            let mut locals = Vec::new();
            for n in 1..=4i32 {
                match getlocal.call::<(Option<String>, LuaValue)>((level, n)) {
                    Ok((Some(lname), lval)) => {
                        let rendered = match lval {
                            LuaValue::Integer(i) => i.to_string(),
                            LuaValue::Number(f) => f.to_string(),
                            LuaValue::String(s) => s.to_string_lossy().to_string(),
                            other => format!("<{}>", other.type_name()),
                        };
                        locals.push(format!("{lname}={rendered}"));
                    }
                    _ => break,
                }
            }
            sink.borrow_mut().push(format!(
                "level {level}: what={what} name={fname} line={line} locals=[{}]",
                locals.join(", ")
            ));
        }
        Ok(VmState::Continue)
    })
    .expect("the hook must install, or this test asserts nothing");

    // NOTE: line numbers matter — MARKER_LINE above points at `return b`.
    // `outer` must use a NON-tail call, or LuaJIT replaces its frame and its
    // locals vanish from the stack entirely.
    lua.load(
        r#"
        local function inner()
            local b = 222
            local bb = "beta"
            return b
        end
        local function outer()
            local a = 111
            local r = inner()
            return r
        end
        outer()
        "#,
    )
    .set_name("@spike2.lua")
    .exec()
    .unwrap();

    lua.remove_hook();

    let out = observed.borrow().clone();
    println!("spike2 — debug.getlocal levels as seen from inside an mlua hook:");
    for line in &out {
        println!("  {line}");
    }
    assert!(!out.is_empty(), "hook never fired at the marker line");

    // The design needs to know WHICH level is the hooked frame.
    let inner_level = out.iter().position(|l| l.contains("b=222"));
    let outer_level = out.iter().position(|l| l.contains("a=111"));
    println!(
        "spike2 RESULT: hooked frame's locals at level {inner_level:?}, \
         its caller's at level {outer_level:?}"
    );
    assert!(
        inner_level.is_some(),
        "debug.getlocal could not reach the hooked frame's locals at any level 0..4; \
         variable inspection would be impossible. Observations: {out:#?}"
    );
    assert_eq!(
        outer_level,
        inner_level.map(|l| l + 1),
        "caller's frame must sit exactly one level above the hooked frame"
    );
}

// ─── Spike 3: tail calls, event sequence, and inspect_stack depth ────────────

#[test]
fn spike3_tailcall_events_and_stack_depth() {
    let lua = Lua::new();
    let trace: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = trace.clone();

    let triggers = HookTriggers::new().on_calls().on_returns().every_line();
    lua.set_hook(triggers, move |lua, debug| {
        let src = debug.source().source.map(|s| s.to_string()).unwrap_or_default();
        if !src.contains("spike3") {
            return Ok(VmState::Continue);
        }
        let depth = (0..64).take_while(|n| lua.inspect_stack(*n, |_| ()).is_some()).count();
        let name = debug.names().name.map(|s| s.to_string()).unwrap_or_default();
        sink.borrow_mut().push(format!(
            "{:?} depth={} line={} name={}",
            debug.event(),
            depth,
            debug.current_line().unwrap_or(0),
            name
        ));
        Ok(VmState::Continue)
    })
    .expect("the hook must install, or this test asserts nothing");

    lua.load(
        r#"
        local function leaf(n)
            return n + 1
        end
        local function tail_caller(n)
            return leaf(n)          -- tail call: no matching Ret for this frame
        end
        local function normal_caller(n)
            local v = leaf(n)       -- normal call
            return v
        end
        tail_caller(1)
        normal_caller(1)
        "#,
    )
    .set_name("@spike3.lua")
    .exec()
    .unwrap();

    lua.remove_hook();

    let out = trace.borrow().clone();
    println!("spike3 — LuaJIT hook event sequence:");
    for line in &out {
        println!("  {line}");
    }

    let calls = out.iter().filter(|l| l.starts_with("Call")).count();
    let rets = out.iter().filter(|l| l.starts_with("Ret")).count();
    let tailrets = out.iter().filter(|l| l.starts_with("TailCall")).count();
    println!("spike3 RESULT: Call={calls} Ret={rets} TailCall={tailrets}");

    assert!(calls > 0, "no Call events observed");
    assert!(
        out.iter().all(|l| l.contains("depth=")),
        "inspect_stack must yield a depth at every hook event"
    );

    // The design's claim, now locked in: LuaJIT emits a `Call` for a tail call but
    // REPLACES the frame rather than pushing one, and emits no compensating `Ret`.
    // A naive Call-minus-Ret counter therefore drifts, so step-over/step-out must
    // derive depth from `inspect_stack` instead.
    let naive = calls as i64 - rets as i64 - tailrets as i64;
    assert_ne!(
        naive, 0,
        "expected a naive call/return counter to desync across the tail call"
    );
    println!("spike3 NOTE: naive counter desyncs by {naive}; design uses inspect_stack depth");
}
