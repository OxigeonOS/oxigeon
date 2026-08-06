//! What unprivileged Lua may reach — the production security boundary.
//!
//! Compiled into both the server and the compute worker, so there is exactly one
//! list of what gets removed. A worker VM with a laxer sandbox than the game's
//! would be the more dangerous of the two: it runs arbitrary game code with no
//! efuns to constrain it and nobody watching its output.
//!
//! `docs/src/lua-api/sandboxing.md` documents the same list. The server calls
//! this from `ScriptEngine::start` after `register_all`, and the worker from
//! `vm::build`; anything registered afterwards is not subject to it.

use mlua::prelude::*;

/// The `os` functions the mudlib is allowed to keep. They read a clock and
/// nothing else; `efuns_io.rs` wraps the same three as `os_time`/`os_clock`/
/// `os_date` for code that would rather use an efun.
const OS_KEPT: &[&str] = &["time", "date", "clock", "difftime"];

/// Strip everything that lets Lua reach outside the game from the VM's globals.
///
/// This is the *production* security boundary. `engine::ScriptEngine::start`
/// calls it on the real VM after `register_all`, so what unprivileged mudlib
/// code can reach is exactly what survives this function — there is no second,
/// parallel sandbox to drift out of sync with.
/// `docs/src/lua-api/sandboxing.md` documents the same list.
///
/// Everything removed here has a jailed efun equivalent (`read_file`,
/// `write_file`, `list_dir`, ...) that enforces the mudlib jail and the
/// directory permissions in `config/permissions.toml`. Raw `io` walked straight
/// around both.
pub fn apply_sandbox(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    // Whole modules with no safe subset. `debug` is already hidden by
    // `debugger::introspect::hide_debug_library` when the debug adapter is on;
    // this covers the case where it was never loaded at all.
    globals.set("io", LuaValue::Nil)?;
    globals.set("debug", LuaValue::Nil)?;

    // Uncontrolled file loading. `require` is the supported route and is
    // confined to package.path, which the engine sets to mudlib + game only.
    globals.set("loadfile", LuaValue::Nil)?;
    globals.set("dofile", LuaValue::Nil)?;

    // LuaJIT's compiler controls. `jit.on()` would re-enable trace compilation,
    // and a compiled trace dispatches no hooks — so leaving this reachable
    // would let one line in a room file disarm the instruction budget. Nothing
    // in the mudlib uses it.
    // Absent on PUC Lua; setting nil there is harmless but says something
    // untrue about what is being defended against.
    #[cfg(feature = "luajit")]
    globals.set("jit", LuaValue::Nil)?;

    // Keep the clock functions, drop everything that touches the host process
    // or the filesystem. Removing the whole `os` table instead would break
    // date formatting for no extra safety.
    if let Ok(os_table) = globals.get::<LuaTable>("os") {
        let mut doomed = Vec::new();
        for pair in os_table.clone().pairs::<LuaValue, LuaValue>() {
            let (k, _) = pair?;
            if let Some(name) = k.as_string().and_then(|s| s.to_str().ok().map(|v| v.to_owned())) {
                if !OS_KEPT.contains(&name.as_str()) {
                    doomed.push(k.clone());
                }
            } else {
                doomed.push(k.clone());
            }
        }
        for k in doomed {
            os_table.set(k, LuaValue::Nil)?;
        }
    }

    // Native code loading. `loadlib` opens a shared library directly; searchers
    // 3 and 4 are how `require` would find one on `cpath`.
    //
    // The table is `package.loaders` in 5.1 and was renamed `package.searchers`
    // in 5.2. This used to read only `loaders` behind an `if let Ok`, so on any
    // 5.2+ runtime it found nothing, did nothing, and **left the C searcher
    // installed** — a sandbox that failed open with no error and no failing
    // test. Handle both names, and truncate rather than poke holes: 5.4's
    // `require` iterates to the array length instead of stopping at the first
    // nil, so clearing index 3 while 4 remained would leave 4 reachable.
    if let Ok(package) = globals.get::<LuaTable>("package") {
        package.set("loadlib", LuaValue::Nil)?;
        package.set("cpath", "")?;

        let mut found_any = false;
        for name in ["searchers", "loaders"] {
            if let Ok(searchers) = package.get::<LuaTable>(name) {
                found_any = true;
                // 1 = package.preload, 2 = the Lua-source searcher. Everything
                // from 3 up is a native path.
                let len = searchers.raw_len().max(4);
                for i in 3..=len {
                    searchers.set(i, LuaValue::Nil)?;
                }
            }
        }
        if !found_any {
            // Neither name present is not a Lua version we know how to sandbox.
            return Err(mlua::Error::RuntimeError(
                "sandbox: package has neither `searchers` nor `loaders`; refusing \
                 to run with an unaudited module searcher"
                    .to_string(),
            ));
        }
    }

    install_text_only_loaders(lua, &globals)?;

    Ok(())
}

/// Give this VM its own random sequence.
///
/// LuaJIT starts every VM from a constant seed, and `math.randomseed` appeared
/// nowhere in `mudlib/`, `game/`, `src/` or `tests/` — so two fresh VMs both
/// returned `794206293` for the first `math.random(1, 1e9)`. That means
/// identical combat to-hit and damage rolls, identical loot outcomes, identical
/// weighted echo choices and identical virtual-room description variation on
/// every restart. It is not a subtle bias; it is the same game twice.
///
/// Seeded in Rust at VM construction rather than in `mudlib/init.lua`, so it
/// covers **every** VM the engine builds: compute workers have their own, and
/// they are the ones meant to run simulations. `salt` distinguishes VMs built
/// in the same nanosecond, which is what a worker pool does.
///
/// `DAEMON.combat._roll` stays overridable, so a test that wants pinned numbers
/// is deterministic by choice rather than by accident.
pub fn seed_prng(lua: &Lua, salt: u64) -> LuaResult<()> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    // Mixed rather than added: consecutive workers salted 0, 1, 2 in the same
    // nanosecond would otherwise get seeds one apart, and LuaJIT's generator
    // does not scramble adjacent seeds well enough for that to be independent.
    let mut seed = nanos ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    seed ^= seed >> 33;
    seed = seed.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    seed ^= seed >> 33;

    // A double carries 53 bits exactly, and `math.randomseed` takes a number.
    let seed = (seed & 0x1F_FFFF_FFFF_FFFF) as f64;

    let f: mlua::Function = lua.globals().get::<LuaTable>("math")?.get("randomseed")?;
    f.call::<()>(seed)?;
    Ok(())
}

/// Replace `load` and `loadstring` with wrappers that refuse binary chunks.
///
/// Pre-compiled LuaJIT bytecode is not validated on load and is a known route
/// to memory corruption, so the only thing the VM will compile is text.
fn install_text_only_loaders(lua: &Lua, globals: &LuaTable) -> LuaResult<()> {
    for name in ["load", "loadstring"] {
        if globals.get::<LuaValue>(name)?.is_nil() {
            continue;
        }
        let default_chunk_name = format!("=({})", name);
        let f = lua.create_function(
            move |lua,
                  (chunk, chunk_name, _mode, env): (
                LuaValue,
                Option<mlua::String>,
                Option<mlua::String>,
                Option<LuaTable>,
            )| {
                let Some(code) = chunk.as_string() else {
                    // The reader-function form of `load` would hand us chunks
                    // one piece at a time, which we cannot screen. Nothing in
                    // the mudlib uses it.
                    return Ok((
                        LuaValue::Nil,
                        Some("load: only string chunks are permitted".to_string()),
                    ));
                };
                let bytes = code.as_bytes();
                if bytes.first() == Some(&0x1B) {
                    return Ok((
                        LuaValue::Nil,
                        Some("load: binary bytecode is not permitted".to_string()),
                    ));
                }
                let name = chunk_name
                    .as_ref()
                    .map(|s| s.to_string_lossy())
                    .unwrap_or_else(|| default_chunk_name.clone());

                let mut chunk = lua.load(bytes.as_ref()).set_name(name);
                // `mode` is deliberately ignored — this wrapper is *stricter*
                // than any mode: it refuses binary above, whatever was asked
                // for. `env` is not optional in the same way. Dropping it made
                // `load(src, name, "t", env)` silently compile against the
                // globals instead, and on 5.2+ that is the *only* way to set a
                // chunk's environment: 5.1's `setfenv`, which the debugger uses
                // on LuaJIT, does not exist there.
                //
                // The visible cost was the whole debug evaluator. Watch
                // expressions, breakpoint conditions and the REPL all compile a
                // snapshot of the paused frame as the chunk's environment, so
                // every local silently read as a global — `player` came back nil
                // on a line where `player` is plainly in scope.
                if let Some(env) = env {
                    chunk = chunk.set_environment(env);
                }
                // Returns `nil, err` on a syntax error, as `load` always has.
                match chunk.into_function() {
                    Ok(func) => Ok((LuaValue::Function(func), None)),
                    Err(e) => Ok((LuaValue::Nil, Some(e.to_string()))),
                }
            },
        )?;
        globals.set(name, f)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn sandbox_leaves_only_the_clock_functions_on_os() {
        let lua = Lua::new();
        apply_sandbox(&lua).unwrap();
        let os: LuaTable = lua.globals().get("os").unwrap();
        let mut kept: Vec<String> = os
            .pairs::<String, LuaValue>()
            .filter_map(|pair: LuaResult<(String, LuaValue)>| pair.ok().map(|(k, _)| k))
            .collect();
        kept.sort();
        let mut expected: Vec<String> = OS_KEPT.iter().map(|s| s.to_string()).collect();
        expected.sort();
        assert_eq!(kept, expected);
    }

    #[test]
    fn sandbox_blocks_binary_bytecode_via_load() {
        let lua = Lua::new();
        apply_sandbox(&lua).unwrap();
        // A real chunk, compiled, then handed back to `load` as bytecode.
        let dumped: mlua::String = lua
            .load(r#"return string.dump(function() return 42 end)"#)
            .eval()
            .unwrap();
        let (val, err): (LuaValue, Option<String>) =
            lua.globals().get::<LuaFunction>("load").unwrap().call(dumped).unwrap();
        assert!(val.is_nil(), "binary bytecode should not compile");
        assert!(err.unwrap().contains("binary bytecode"));
    }

    /// `load`'s fourth argument sets the chunk's environment, and the wrapper
    /// must pass it through.
    ///
    /// Dropping it is invisible: the chunk still compiles and still runs, it
    /// just resolves every name against the globals instead. On 5.2+ this is the
    /// *only* way to set an environment — 5.1's `setfenv` is gone — so the whole
    /// debug evaluator quietly stopped seeing locals. A watch on `player` came
    /// back nil on a line where `player` is plainly in scope.
    #[test]
    fn load_honours_the_environment_it_is_given() {
        let lua = Lua::new();
        apply_sandbox(&lua).unwrap();
        let got: String = lua
            .load(
                r#"
                x = "global"
                local env = { x = "captured" }
                local chunk = load("return x", "=probe", "t", env)
                return chunk()
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(
            got, "captured",
            "the chunk compiled against the globals — `load`'s env was dropped"
        );
    }

    /// And with no environment given, a chunk still sees the globals.
    #[test]
    fn load_without_an_environment_still_sees_globals() {
        let lua = Lua::new();
        apply_sandbox(&lua).unwrap();
        let got: String = lua
            .load(r#" x = "global" return load("return x")() "#)
            .eval()
            .unwrap();
        assert_eq!(got, "global");
    }

    /// A `mode` of `"b"` does not re-open the door the wrapper exists to shut.
    #[test]
    fn asking_for_binary_mode_does_not_permit_binary() {
        let lua = Lua::new();
        apply_sandbox(&lua).unwrap();
        let dumped: mlua::String = lua
            .load(r#"return string.dump(function() return 42 end)"#)
            .eval()
            .unwrap();
        let (val, err): (LuaValue, Option<String>) = lua
            .globals()
            .get::<LuaFunction>("load")
            .unwrap()
            .call((dumped, "=b", "b"))
            .unwrap();
        assert!(val.is_nil(), "`mode = \"b\"` must not permit bytecode");
        assert!(err.unwrap().contains("binary bytecode"));
    }

    #[test]
    fn sandbox_still_allows_text_load() {
        let lua = Lua::new();
        apply_sandbox(&lua).unwrap();
        let n: i64 = lua.load(r#"return load("return 42")()"#).eval().unwrap();
        assert_eq!(n, 42);
    }

    #[test]
    fn sandbox_reports_syntax_errors_the_way_load_always_has() {
        let lua = Lua::new();
        apply_sandbox(&lua).unwrap();
        let (val, err): (LuaValue, Option<String>) = lua
            .globals()
            .get::<LuaFunction>("load")
            .unwrap()
            .call("this is not lua")
            .unwrap();
        assert!(val.is_nil());
        assert!(err.is_some(), "a syntax error must come back as nil, message");
    }

    /// `require` has to keep working — the whole mudlib is built on it.
    #[test]
    fn sandbox_leaves_the_lua_source_searcher_in_place() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("probe.lua"), "return 7").unwrap();
        let lua = Lua::new();
        apply_sandbox(&lua).unwrap();
        lua.load(format!(
            "package.path = [[{}/?.lua]] .. ';' .. package.path",
            dir.path().to_string_lossy().replace('\\', "/")
        ))
        .exec()
        .unwrap();
        let n: i64 = lua.load("return require('probe')").eval().unwrap();
        assert_eq!(n, 7);
    }

    #[test]
    fn sandbox_removes_the_native_module_loaders() {
        let lua = Lua::new();
        apply_sandbox(&lua).unwrap();
        assert!(lua.globals().get::<LuaTable>("package").unwrap()
            .get::<LuaValue>("loadlib").unwrap().is_nil());
        let cpath: String = lua.load("return package.cpath").eval().unwrap();
        assert_eq!(cpath, "");
    }
}
