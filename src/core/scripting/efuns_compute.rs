//! The `compute` and `compute_cancel` efuns.
//!
//! A sibling of `efuns_io.rs` rather than four hundred more lines in
//! `efuns.rs`, which is already the largest file in the project.
//!
//! Both are non-blocking and neither returns a result: the answer arrives
//! later at the mudlib hook `on_compute_result`. See [`crate::core::compute`]
//! for why every operational failure goes through that hook too.

use std::sync::Arc;

use mlua::prelude::*;

use crate::core::compute::{marshal, ComputeBridge, LuaData, SubmitError};

use super::efuns::EfunContext;

/// Register `compute` and `compute_cancel`, if a pool is running.
///
/// With compute disabled nothing is registered at all — the same choice
/// `register_auth_efuns` makes. A missing global fails loudly at the call site,
/// which is far easier to diagnose than an efun that silently does nothing.
pub fn register_compute_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let Some(bridge) = ctx.compute.clone() else {
        tracing::debug!("compute is disabled — `compute` efun not registered");
        return Ok(());
    };
    let bridge = Arc::new(bridge);
    let globals = lua.globals();

    // compute(module, fn_name, args, opts) -> id | nil, err
    {
        let bridge = bridge.clone();
        let f = lua.create_function(
            move |lua, (module, func, args, opts): (String, String, LuaValue, Option<LuaTable>)| {
                let limits = bridge.arg_limits();

                // Copied here, on the game thread, because nothing else can:
                // the value belongs to this VM and `Lua` is `!Send`. This is
                // also the only part of a compute job the game thread pays
                // for, which is why the docs advise small arguments.
                let args = match marshal::from_lua(&args, &limits) {
                    Ok(data) => data,
                    Err(e) => return refused(lua, SubmitError::Args(e)),
                };

                let (tag, deadline_ms) = match &opts {
                    None => (LuaData::Nil, None),
                    Some(opts) => {
                        let tag = match opts.get::<LuaValue>("tag") {
                            Ok(v) => match marshal::from_lua(&v, &limits) {
                                Ok(data) => data,
                                Err(e) => return refused(lua, SubmitError::Args(e)),
                            },
                            Err(_) => LuaData::Nil,
                        };
                        (tag, opts.get::<Option<u64>>("deadline_ms").unwrap_or(None))
                    }
                };

                match bridge.submit(module, func, args, tag, deadline_ms) {
                    // A decimal string, not a number: LuaJIT numbers are
                    // doubles, and an id that renders as `1e+06` in a log or a
                    // table key would be its own small nightmare.
                    Ok(id) => Ok((
                        LuaValue::String(lua.create_string(id.to_string())?),
                        LuaValue::Nil,
                    )),
                    Err(e) => refused(lua, e),
                }
            },
        )?;
        globals.set("compute", f)?;
    }

    // compute_cancel(id) -> boolean
    {
        let bridge = bridge.clone();
        let f = lua.create_function(move |_, id: String| {
            let Ok(id) = id.parse::<u64>() else {
                return Ok(false);
            };
            Ok(bridge.cancel(id))
        })?;
        globals.set("compute_cancel", f)?;
    }

    Ok(())
}

/// The `nil, err` pair a call-site mistake comes back as.
///
/// Deliberately not a raise: the mudlib's handling of "I could not start this
/// job" is the same either way, and returning lets a caller write one
/// `if not id then` rather than wrapping every call in `pcall`.
fn refused(lua: &Lua, e: SubmitError) -> LuaResult<(LuaValue, LuaValue)> {
    Ok((
        LuaValue::Nil,
        LuaValue::String(lua.create_string(e.to_string())?),
    ))
}

/// Snapshot of the pool, shaped for `server_info().compute`.
pub fn snapshot_table(lua: &Lua, bridge: &ComputeBridge) -> LuaResult<LuaTable> {
    let s = bridge.snapshot();
    let t = lua.create_table()?;
    t.set("workers", s.workers as f64)?;
    t.set("queue_depth", s.queue_depth as f64)?;
    t.set("instruction_limit", s.instruction_limit as f64)?;
    t.set("in_flight", s.in_flight as f64)?;
    t.set("running", s.running as f64)?;
    t.set("submitted", s.stats.submitted as f64)?;
    t.set("completed", s.stats.completed as f64)?;
    t.set("failed", s.stats.failed as f64)?;
    t.set("timed_out", s.stats.timed_out as f64)?;
    t.set("refused", s.stats.refused as f64)?;
    t.set("cancelled", s.stats.cancelled as f64)?;
    // Non-zero means a worker is gone for good. Worth alerting on.
    t.set("wedged", s.stats.wedged as f64)?;
    Ok(t)
}
