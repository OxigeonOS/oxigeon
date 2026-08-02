//! Efuns backing the in-game `trace` command.
//!
//! All of these run on the Lua thread, which is also where the ring buffers
//! live, so reads need no synchronization.

use mlua::prelude::*;

use super::state::TraceMode;
use super::trace;
use crate::core::scripting::efuns::{check_efun_permission, get_current_session, EfunContext};

/// Bundle of the handles `check_efun_permission` needs, so each closure can
/// gate itself without capturing the whole `EfunContext`.
#[derive(Clone)]
struct Guard {
    perms: std::sync::Arc<crate::config::permissions_config::PermissionConfig>,
    sessions: std::sync::Arc<std::sync::RwLock<crate::core::session::SessionHandler>>,
    logger: std::sync::Arc<crate::core::logging::GameLogger>,
}

impl Guard {
    fn check(&self, efun: &str) -> LuaResult<()> {
        check_efun_permission(efun, &self.perms, &self.sessions, &self.logger)
    }
}

/// Register `trace_*` into the Lua globals.
///
/// These are gated independently of `cmds/trace.lua`'s `M.permission`, so
/// arbitrary mudlib code cannot turn tracing on by calling the efun directly.
pub fn register_debug_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();
    let st = ctx.debug_state.clone();
    let guard = Guard {
        perms: ctx.permission_config.clone(),
        sessions: ctx.session_handler.clone(),
        logger: ctx.game_logger.clone(),
    };

    trace::set_capacities(st.trace_capacity, st.timing_capacity);

    // trace_set(mode, scope) -> ok, err
    //   mode:  "off" | "time" | "calls" | "lines"
    //   scope: nil (the calling session) | "all" | a session id
    let s = st.clone();
    let g = guard.clone();
    globals.set(
        "trace_set",
        lua.create_function(move |_, (mode, scope): (String, Option<String>)| {
            g.check("trace_set")?;
            let Some(mode) = TraceMode::parse(&mode) else {
                return Ok((false, Some(format!("unknown trace mode '{mode}'"))));
            };

            if mode == TraceMode::Off {
                // Scope-less "off" clears everything; scoped "off" drops one session.
                match scope.as_deref() {
                    Some("all") | None => s.set_trace_config(Default::default()),
                    Some(sid) => {
                        let sid = sid.to_string();
                        s.update_trace_config(|c| {
                            c.sessions.remove(&sid);
                        });
                    }
                }
                return Ok((true, None));
            }

            let target = match scope.as_deref() {
                Some("all") => None,
                Some(sid) => Some(sid.to_string()),
                None => match get_current_session() {
                    Some(sid) => Some(sid),
                    None => {
                        return Ok((
                            false,
                            Some("no current session — use `trace all` or name a session".into()),
                        ))
                    }
                },
            };

            s.update_trace_config(|c| {
                c.mode = mode;
                match target {
                    None => c.all_sessions = true,
                    Some(sid) => {
                        c.sessions.insert(sid);
                    }
                }
            });
            Ok((true, None))
        })?,
    )?;

    // trace_status() -> table
    let s = st.clone();
    let g = guard.clone();
    globals.set(
        "trace_status",
        lua.create_function(move |lua, ()| {
            g.check("trace_status")?;
            let cfg = s.trace_config();
            let t = lua.create_table()?;
            t.set("mode", cfg.mode.as_str())?;
            t.set("all_sessions", cfg.all_sessions)?;
            t.set("armed", s.armed.load(std::sync::atomic::Ordering::Relaxed))?;

            let sessions = lua.create_table()?;
            for (i, sid) in cfg.sessions.iter().enumerate() {
                sessions.set(i + 1, sid.as_str())?;
            }
            t.set("sessions", sessions)?;

            trace::with_rings(|r| -> LuaResult<()> {
                t.set("records", r.trace.len())?;
                t.set("capacity", r.trace_cap)?;
                t.set("timings", r.timing.len())?;
                t.set("dropped", r.dropped)?;
                Ok(())
            })?;
            Ok(t)
        })?,
    )?;

    // trace_show(limit) -> { string, ... }
    let g = guard.clone();
    globals.set(
        "trace_show",
        lua.create_function(move |lua, limit: Option<usize>| {
            g.check("trace_show")?;
            lua.create_sequence_from(trace::format_records(limit.unwrap_or(40)))
        })?,
    )?;

    // trace_timings(limit) -> { string, ... }
    let g = guard.clone();
    globals.set(
        "trace_timings",
        lua.create_function(move |lua, limit: Option<usize>| {
            g.check("trace_timings")?;
            lua.create_sequence_from(trace::format_timings(limit.unwrap_or(20)))
        })?,
    )?;

    // trace_clear()
    let g = guard.clone();
    globals.set(
        "trace_clear",
        lua.create_function(move |_, ()| {
            g.check("trace_clear")?;
            trace::clear();
            Ok(())
        })?,
    )?;

    Ok(())
}
