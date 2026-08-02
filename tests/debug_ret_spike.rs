//! Spike: what is actually visible at a return hook under LuaJIT?
//!
//! Determines whether a trace record for a `Ret` event can show the value being
//! returned, or only the returning frame's locals.

use mlua::prelude::*;
use mlua::{HookTriggers, StdLib, VmState};
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn what_is_readable_at_a_return_event() {
    let lua = unsafe {
        Lua::unsafe_new_with(StdLib::ALL_SAFE | StdLib::DEBUG, LuaOptions::default())
    };
    let globals = lua.globals();
    let dbg: LuaTable = globals.get("debug").unwrap();
    lua.set_named_registry_value("dbg", dbg).unwrap();

    let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();

    lua.set_hook(HookTriggers::new().on_returns(), move |lua, d| {
        let src = d.source().source.map(|s| s.to_string()).unwrap_or_default();
        if !src.contains("retspike") {
            return Ok(VmState::Continue);
        }
        let dbg: LuaTable = lua.named_registry_value("dbg").unwrap();
        let getlocal: LuaFunction = dbg.get("getlocal").unwrap();

        // Level 1 is the returning frame (level 0 is the getlocal C function).
        let mut locals = Vec::new();
        for n in 1..=6i32 {
            match getlocal.call::<(Option<String>, LuaValue)>((1, n)) {
                Ok((Some(name), v)) => locals.push(format!("{name}={}", render(&v))),
                _ => break,
            }
        }
        // Lua 5.2+ uses negative indices for varargs; check whether LuaJIT
        // exposes anything there that might be the pending return values.
        let mut negatives = Vec::new();
        for n in -1..=-3i32 {
            if let Ok((Some(name), v)) = getlocal.call::<(Option<String>, LuaValue)>((1, n)) {
                negatives.push(format!("{name}={}", render(&v)));
            }
        }

        sink.borrow_mut().push(format!(
            "{:?} name={} locals=[{}] negative=[{}]",
            d.event(),
            d.names().name.map(|s| s.to_string()).unwrap_or_default(),
            locals.join(", "),
            negatives.join(", "),
        ));
        Ok(VmState::Continue)
    });

    lua.load(
        r#"
        local function named_local(n)
            local doubled = n * 2
            return doubled          -- returns a named local
        end
        local function computed(n)
            return n * 3 + 1        -- returns a temporary, no local holds it
        end
        local function multi(n)
            return n, n + 1         -- two values
        end
        named_local(5)
        computed(5)
        multi(5)
        "#,
    )
    .set_name("@retspike.lua")
    .exec()
    .unwrap();

    lua.remove_hook();

    println!("--- what a Ret hook can see ---");
    for line in seen.borrow().iter() {
        println!("  {line}");
    }
    let out = seen.borrow().clone();
    assert!(!out.is_empty(), "no return events observed");

    // The finding this spike exists to record: the returned values ARE on the
    // stack as unnamed temporaries...
    let computed = out.iter().find(|l| l.contains("name=computed")).unwrap();
    assert!(
        computed.contains("(*temporary)=16"),
        "expected the return value of `n * 3 + 1` in a temporary: {computed}"
    );

    // ...but the frame is already being dismantled, so the name-to-slot mapping
    // is wrong. `named_local(5)` sets doubled = 10, yet the slot *named*
    // `doubled` holds 5. Nothing distinguishes a return temporary from any
    // other, so trace records deliberately show no value for a return.
    let named = out.iter().find(|l| l.contains("name=named_local")).unwrap();
    assert!(
        named.contains("doubled=5"),
        "expected the shifted name mapping this spike documents: {named}"
    );

    // LuaJIT exposes nothing through 5.2-style negative (vararg) indices either.
    assert!(
        out.iter().all(|l| l.contains("negative=[]")),
        "negative getlocal indices unexpectedly returned something: {out:#?}"
    );
}

fn render(v: &LuaValue) -> String {
    match v {
        LuaValue::Integer(i) => i.to_string(),
        LuaValue::Number(f) => f.to_string(),
        LuaValue::String(s) => format!("{:?}", s.to_string_lossy()),
        LuaValue::Nil => "nil".into(),
        other => format!("<{}>", other.type_name()),
    }
}
