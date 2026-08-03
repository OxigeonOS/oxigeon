use std::path::{Path, PathBuf};
use mlua::prelude::*;
use crate::error::{OxigeonError, Result};

/// Resolve a Lua-provided path, ensuring it stays within the mudlib directory.
/// Prevents directory traversal attacks (../../etc/passwd).
pub fn resolve_jailed_path(mudlib_root: &Path, lua_path: &str) -> Result<PathBuf> {
    // Basic validation: reject obvious traversal attempts before canonicalize
    if lua_path.contains("..") {
        return Err(OxigeonError::PathTraversal(
            format!("Path '{}' contains '..' and may escape mudlib root", lua_path)
        ));
    }

    let requested = mudlib_root.join(lua_path);

    // Canonicalize the root (handles Windows UNC paths, symlinks, etc.)
    let canonical_root = mudlib_root.canonicalize()
        .unwrap_or_else(|_| mudlib_root.to_path_buf());

    // If the requested path exists, canonicalize it too
    // Otherwise normalize it without filesystem access
    let canonical_requested = if requested.exists() {
        requested.canonicalize()
            .unwrap_or_else(|_| normalize_path(&requested))
    } else {
        normalize_path(&requested)
    };

    if !canonical_requested.starts_with(&canonical_root) {
        return Err(OxigeonError::PathTraversal(
            format!("Path '{}' escapes mudlib root", lua_path)
        ));
    }

    Ok(canonical_requested)
}

/// Normalize a path without requiring it to exist (no canonicalize).
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => { components.pop(); }
            std::path::Component::CurDir => {}
            c => components.push(c),
        }
    }
    components.iter().collect()
}

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
    globals.set("jit", LuaValue::Nil)?;

    // Keep the clock functions, drop everything that touches the host process
    // or the filesystem. Removing the whole `os` table instead would break
    // date formatting for no extra safety.
    if let Ok(os_table) = globals.get::<LuaTable>("os") {
        let mut doomed = Vec::new();
        for pair in os_table.clone().pairs::<LuaValue, LuaValue>() {
            let (k, _) = pair?;
            if let Some(name) = k.as_str() {
                if !OS_KEPT.contains(&name.as_ref()) {
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

    // Native code loading. `loadlib` opens a shared library directly; loaders
    // 3 and 4 are how `require` would find one on `cpath`.
    if let Ok(package) = globals.get::<LuaTable>("package") {
        package.set("loadlib", LuaValue::Nil)?;
        package.set("cpath", "")?;
        if let Ok(loaders) = package.get::<LuaTable>("loaders") {
            // 1 = package.preload, 2 = the Lua-source searcher. Lua 5.1's
            // `require` walks this array and stops at the first nil, so
            // clearing 3 leaves exactly those two in play.
            loaders.set(3, LuaValue::Nil)?;
            loaders.set(4, LuaValue::Nil)?;
        }
    }

    install_text_only_loaders(lua, &globals)?;

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
            move |lua, (chunk, chunk_name): (LuaValue, Option<mlua::String>)| {
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
                // Returns `nil, err` on a syntax error, as `load` always has.
                match lua.load(bytes.as_ref()).set_name(name).into_function() {
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
    fn test_resolve_jailed_path_safe() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // Create a test file
        std::fs::write(root.join("test.lua"), "return {}").unwrap();
        let result = resolve_jailed_path(root, "test.lua");
        assert!(result.is_ok());
    }

    #[test]
    fn test_resolve_jailed_path_traversal_dotdot() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let result = resolve_jailed_path(root, "../../etc/passwd");
        assert!(matches!(result, Err(OxigeonError::PathTraversal(_))));
    }

    #[test]
    fn test_resolve_jailed_path_traversal_embedded() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let result = resolve_jailed_path(root, "subdir/../../../etc/passwd");
        assert!(matches!(result, Err(OxigeonError::PathTraversal(_))));
    }

    #[test]
    fn test_resolve_jailed_path_subdirectory() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::write(root.join("lib/utils.lua"), "return {}").unwrap();
        let result = resolve_jailed_path(root, "lib/utils.lua");
        assert!(result.is_ok());
    }

    /// The `os` keys that survive are exactly `OS_KEPT` — a new LuaJIT release
    /// adding another one must not slip in unnoticed.
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
