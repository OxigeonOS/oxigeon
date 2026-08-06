//! Rust side of variable inspection: loads the helper chunk and marshals its
//! tables into DAP shapes.
//!
//! Everything here runs on the Lua thread, from inside the paused hook.

use mlua::prelude::*;

use super::state::{DapScope, DapVariable};

const DEBUG_REGISTRY_KEY: &str = "oxigeon.debug";
const HELPER_REGISTRY_KEY: &str = "oxigeon.dbg_helper";

/// Stash the `debug` table in the registry and remove it from `_G`.
///
/// Must run before any mudlib code, so nothing in the game can ever reach it.
/// Also re-closes `package.loadlib`, which `unsafe_new_with` leaves open.
pub fn hide_debug_library(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();
    let dbg: LuaTable = globals.get("debug")?;
    lua.set_named_registry_value(DEBUG_REGISTRY_KEY, dbg)?;
    globals.set("debug", LuaValue::Nil)?;
    if let Ok(pkg) = globals.get::<LuaTable>("package") {
        let _ = pkg.set("loadlib", LuaValue::Nil);
    }
    Ok(())
}

/// Load the introspection helper, closing it over the hidden `debug` table.
pub fn load_helper(lua: &Lua) -> LuaResult<()> {
    let dbg: LuaTable = lua.named_registry_value(DEBUG_REGISTRY_KEY)?;
    let helper: LuaTable = lua
        .load(include_str!("introspect.lua"))
        .set_name("@<oxigeon>/debugger/introspect.lua")
        .call(dbg)?;
    lua.set_named_registry_value(HELPER_REGISTRY_KEY, helper)?;
    Ok(())
}

fn helper(lua: &Lua) -> Option<LuaTable> {
    lua.named_registry_value::<LuaTable>(HELPER_REGISTRY_KEY).ok()
}

fn call<A: IntoLuaMulti, R: FromLuaMulti>(lua: &Lua, name: &str, args: A) -> Option<R> {
    let f: LuaFunction = helper(lua)?.get(name).ok()?;
    match f.call::<R>(args) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("debugger: introspect.{} failed: {}", name, e);
            None
        }
    }
}

fn to_variables(rows: LuaTable) -> Vec<DapVariable> {
    rows.sequence_values::<LuaTable>()
        .filter_map(Result::ok)
        .map(|r| DapVariable {
            name: r.get("name").unwrap_or_default(),
            value: r.get("value").unwrap_or_default(),
            ty: r.get("type").unwrap_or_default(),
            var_ref: r.get("ref").unwrap_or(0),
        })
        .collect()
}

pub fn scopes(lua: &Lua, frame: i64) -> Vec<DapScope> {
    let Some(rows) = call::<_, LuaTable>(lua, "scopes", frame) else {
        return Vec::new();
    };
    rows.sequence_values::<LuaTable>()
        .filter_map(Result::ok)
        .map(|r| DapScope {
            name: r.get("name").unwrap_or_default(),
            var_ref: r.get("ref").unwrap_or(0),
            expensive: r.get("expensive").unwrap_or(false),
        })
        .collect()
}

pub fn variables(lua: &Lua, var_ref: i64) -> Vec<DapVariable> {
    call::<_, LuaTable>(lua, "expand", var_ref)
        .map(to_variables)
        .unwrap_or_default()
}

/// Freeze the paused frames, so they can be answered for after the thread that
/// owns them has been parked.
///
/// Only meaningful from inside the hook, before yielding: it walks the *current*
/// stack. Returns a capture id, or 0 when there was nothing to capture.
pub fn capture(lua: &Lua, levels: i64) -> i64 {
    call::<_, i64>(lua, "capture", levels).unwrap_or(0)
}

/// Drop a capture and everything it froze. Called on resume.
pub fn release(lua: &Lua, cap: i64) {
    if cap != 0 {
        let _ = call::<_, ()>(lua, "release", cap);
    }
}

/// Scopes for a frame of a captured stop.
pub fn capture_scopes(lua: &Lua, cap: i64, frame: i64) -> Vec<DapScope> {
    let Some(rows) = call::<_, LuaTable>(lua, "cap_scopes", (cap, frame)) else {
        return Vec::new();
    };
    rows.sequence_values::<LuaTable>()
        .filter_map(Result::ok)
        .map(|r| DapScope {
            name: r.get("name").unwrap_or_default(),
            var_ref: r.get("ref").unwrap_or(0),
            expensive: r.get("expensive").unwrap_or(false),
        })
        .collect()
}

/// Evaluate against a captured frame's environment.
pub fn capture_evaluate(lua: &Lua, cap: i64, frame: i64, expr: &str) -> Result<DapVariable, String> {
    let Some((ok, text, ty, var_ref)) = call::<_, (bool, String, String, i64)>(
        lua,
        "cap_eval",
        (cap, frame, expr.to_string()),
    ) else {
        return Err("evaluate is unavailable — the debug library is not loaded".into());
    };
    if ok {
        Ok(DapVariable { name: String::new(), value: text, ty, var_ref })
    } else {
        Err(text)
    }
}

/// Returns the rendered result, or the error text the client should show.
pub fn evaluate(lua: &Lua, frame: i64, expr: &str) -> Result<DapVariable, String> {
    let Some((ok, text, ty, var_ref)) =
        call::<_, (bool, String, String, i64)>(lua, "eval", (frame, expr.to_string()))
    else {
        return Err("evaluate is unavailable — the debug library is not loaded".into());
    };
    if ok {
        Ok(DapVariable { name: String::new(), value: text, ty, var_ref })
    } else {
        Err(text)
    }
}

/// Outcome of evaluating a breakpoint condition.
pub enum Condition {
    /// The expression was truthy — stop.
    Met,
    /// The expression was falsy — keep running.
    NotMet,
    /// The expression could not be evaluated. The caller stops anyway and
    /// surfaces this, because silently never stopping looks like a broken
    /// breakpoint and silently always stopping is just as confusing.
    Failed(String),
}

pub fn eval_condition(lua: &Lua, frame: i64, expr: &str) -> Condition {
    match call::<_, (bool, bool, Option<String>)>(lua, "cond", (frame, expr.to_string())) {
        Some((true, true, _)) => Condition::Met,
        Some((true, false, _)) => Condition::NotMet,
        Some((false, _, err)) => Condition::Failed(err.unwrap_or_else(|| "unknown error".into())),
        None => Condition::Failed(
            "conditions need the debug library, which is not loaded".to_string(),
        ),
    }
}

/// Drop every handle allocated during a stop. Called on resume so the variables
/// pane can never show values belonging to a frame that no longer exists.
pub fn reset(lua: &Lua) {
    let _: Option<()> = call(lua, "reset", ());
}
