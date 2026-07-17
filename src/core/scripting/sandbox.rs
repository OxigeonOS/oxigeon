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

/// Create the sandboxed Lua environment.
/// Removes io, os, debug and replaces with controlled efuns.
pub fn create_sandboxed_env(lua: &Lua, mudlib_path: String) -> LuaResult<LuaTable> {
    let env = lua.create_table()?;
    let globals = lua.globals();

    // Whitelisted global functions — safe builtins with no I/O side effects
    let safe_globals = [
        "print", "tostring", "tonumber", "type",
        "pairs", "ipairs", "next", "select", "unpack",
        "error", "pcall", "xpcall", "assert",
        "setmetatable", "getmetatable",
        "rawget", "rawset", "rawequal",
        "setfenv", "getfenv",
        "collectgarbage",
    ];
    for name in &safe_globals {
        if let Ok(val) = globals.get::<LuaValue>(*name) {
            env.set(*name, val)?;
        }
    }

    // Whitelisted standard library modules (no I/O side effects)
    for table_name in &["string", "table", "math", "coroutine"] {
        if let Ok(val) = globals.get::<LuaValue>(*table_name) {
            env.set(*table_name, val)?;
        }
    }

    // Safe `load` — text chunks only, no binary bytecode
    let safe_load = lua.create_function(|lua, (code, name): (String, Option<String>)| {
        // Reject binary bytecode (first byte = 0x1B/ESC)
        if code.as_bytes().first() == Some(&27) {
            return Err(LuaError::RuntimeError(
                "Loading binary bytecode is not permitted".into()
            ));
        }
        lua.load(code.as_str())
            .set_name(name.as_deref().unwrap_or("=(load)"))
            .into_function()
    })?;
    env.set("load", safe_load)?;

    // Controlled require — jailed to mudlib path, no C modules
    let mudlib_for_require = mudlib_path.clone();
    let safe_require = lua.create_function(move |lua, module_name: String| {
        // Sanitize module name (convert dots to path separators)
        let safe_name = module_name.replace('.', "/");
        if safe_name.contains("..") {
            return Err(LuaError::RuntimeError(
                format!("Invalid module name: {}", module_name)
            ));
        }

        // Check package.loaded first
        let loaded: LuaTable = lua.globals()
            .get::<LuaTable>("package")?
            .get("loaded")?;
        if let Ok(cached) = loaded.get::<LuaValue>(module_name.clone()) {
            if !matches!(cached, LuaValue::Nil) {
                return Ok(cached);
            }
        }

        // Load from mudlib path only
        let lua_path = format!("{}/{}.lua", mudlib_for_require, safe_name);
        let code = std::fs::read_to_string(&lua_path)
            .map_err(|_| LuaError::RuntimeError(
                format!("module '{}' not found at {}", module_name, lua_path)
            ))?;

        if code.as_bytes().first() == Some(&27) {
            return Err(LuaError::RuntimeError(
                "Binary Lua modules are not permitted".into()
            ));
        }

        let module: LuaValue = lua.load(code.as_str())
            .set_name(&lua_path)
            .call(())?;

        // Cache in package.loaded
        loaded.set(module_name, module.clone())?;
        Ok(module)
    })?;
    env.set("require", safe_require)?;

    // ── BLOCKED — NOT included in sandbox ──
    // io.*        → use read_file(), write_file() efuns
    // os.execute  → BLOCKED (arbitrary command execution)
    // os.exit     → BLOCKED (would kill the server!)
    // os.getenv   → BLOCKED (env variable leakage)
    // debug.*     → BLOCKED (can escape sandbox)
    // loadfile    → BLOCKED (use require)
    // dofile      → BLOCKED (use require)

    Ok(env)
}

/// Register controlled I/O efuns as a table in the Lua environment.
/// These replace the native io/os modules.
pub fn register_io_efuns(lua: &Lua, env: &LuaTable, mudlib_path: String) -> LuaResult<()> {
    // read_file(path) -> string|nil
    let mudlib_for_read = mudlib_path.clone();
    let read_file = lua.create_function(move |lua, path: String| {
        match resolve_jailed_path(Path::new(&mudlib_for_read), &path) {
            Err(_) => Ok(LuaValue::Nil),
            Ok(full_path) => {
                match std::fs::read_to_string(&full_path) {
                    Ok(content) => {
                        let s = lua.create_string(&content)?;
                        Ok(LuaValue::String(s))
                    }
                    Err(_) => Ok(LuaValue::Nil),
                }
            }
        }
    })?;
    env.set("read_file", read_file)?;

    // write_file(path, content) -> bool
    let mudlib_for_write = mudlib_path.clone();
    let write_file = lua.create_function(move |_, (path, content): (String, String)| {
        match resolve_jailed_path(Path::new(&mudlib_for_write), &path) {
            Err(_) => Ok(false),
            Ok(full_path) => {
                if let Some(parent) = full_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                Ok(std::fs::write(full_path, content).is_ok())
            }
        }
    })?;
    env.set("write_file", write_file)?;

    // append_file(path, content) -> bool
    let mudlib_for_append = mudlib_path.clone();
    let append_file = lua.create_function(move |_, (path, content): (String, String)| {
        match resolve_jailed_path(Path::new(&mudlib_for_append), &path) {
            Err(_) => Ok(false),
            Ok(full_path) => {
                use std::io::Write;
                match std::fs::OpenOptions::new().append(true).create(true).open(full_path) {
                    Ok(mut f) => Ok(f.write_all(content.as_bytes()).is_ok()),
                    Err(_) => Ok(false),
                }
            }
        }
    })?;
    env.set("append_file", append_file)?;

    // file_exists(path) -> bool
    let mudlib_for_exists = mudlib_path.clone();
    let file_exists = lua.create_function(move |_, path: String| {
        match resolve_jailed_path(Path::new(&mudlib_for_exists), &path) {
            Err(_) => Ok(false),
            Ok(full_path) => Ok(full_path.exists()),
        }
    })?;
    env.set("file_exists", file_exists)?;

    // delete_file(path) -> bool
    let mudlib_for_delete = mudlib_path.clone();
    let delete_file = lua.create_function(move |_, path: String| {
        match resolve_jailed_path(Path::new(&mudlib_for_delete), &path) {
            Err(_) => Ok(false),
            Ok(full_path) => Ok(std::fs::remove_file(full_path).is_ok()),
        }
    })?;
    env.set("delete_file", delete_file)?;

    // os_time() -> number
    let os_time = lua.create_function(|_, ()| {
        Ok(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as f64)
    })?;
    env.set("os_time", os_time)?;

    // os_clock() -> number
    let os_clock = lua.create_function(|_, ()| {
        Ok(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64())
    })?;
    env.set("os_clock", os_clock)?;

    Ok(())
}

/// Remove dangerous modules from Lua globals in-place.
/// This is the "hard" sandbox — after calling this, io/os/debug cannot be accessed
/// from any code unless they are explicitly re-added.
/// Used for testing and for the default VM setup.
pub fn apply_sandbox(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();
    // Remove dangerous modules
    globals.set("io", LuaValue::Nil)?;
    globals.set("debug", LuaValue::Nil)?;
    // Remove dangerous os functions (keep os table but strip dangerous keys)
    if let Ok(os_table) = globals.get::<LuaTable>("os") {
        os_table.set("execute", LuaValue::Nil)?;
        os_table.set("exit", LuaValue::Nil)?;
        os_table.set("getenv", LuaValue::Nil)?;
        os_table.set("tmpname", LuaValue::Nil)?;
        os_table.set("rename", LuaValue::Nil)?;
        os_table.set("remove", LuaValue::Nil)?;
    }
    // Remove file-loading functions
    globals.set("loadfile", LuaValue::Nil)?;
    globals.set("dofile", LuaValue::Nil)?;
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

    #[test]
    fn test_sandbox_blocks_io() {
        let lua = Lua::new();
        // Ensure io module is NOT accessible by default after sandbox
        let env = create_sandboxed_env(&lua, "/tmp/mudlib".to_string()).unwrap();
        let io_val: LuaValue = env.get("io").unwrap();
        assert!(matches!(io_val, LuaValue::Nil), "io should be nil in sandbox");
    }

    #[test]
    fn test_sandbox_blocks_os() {
        let lua = Lua::new();
        let env = create_sandboxed_env(&lua, "/tmp/mudlib".to_string()).unwrap();
        let os_val: LuaValue = env.get("os").unwrap();
        assert!(matches!(os_val, LuaValue::Nil), "os should be nil in sandbox");
    }

    #[test]
    fn test_sandbox_blocks_debug() {
        let lua = Lua::new();
        let env = create_sandboxed_env(&lua, "/tmp/mudlib".to_string()).unwrap();
        let debug_val: LuaValue = env.get("debug").unwrap();
        assert!(matches!(debug_val, LuaValue::Nil), "debug should be nil in sandbox");
    }

    #[test]
    fn test_sandbox_allows_string() {
        let lua = Lua::new();
        let env = create_sandboxed_env(&lua, "/tmp/mudlib".to_string()).unwrap();
        let string_val: LuaValue = env.get("string").unwrap();
        assert!(!matches!(string_val, LuaValue::Nil), "string should be available");
    }

    #[test]
    fn test_sandbox_allows_math() {
        let lua = Lua::new();
        let env = create_sandboxed_env(&lua, "/tmp/mudlib".to_string()).unwrap();
        let math_val: LuaValue = env.get("math").unwrap();
        assert!(!matches!(math_val, LuaValue::Nil), "math should be available");
    }

    #[test]
    fn test_sandbox_blocks_binary_bytecode_via_load() {
        let lua = Lua::new();
        let env = create_sandboxed_env(&lua, "/tmp/mudlib".to_string()).unwrap();
        let load_fn: LuaFunction = env.get("load").unwrap();
        // Binary chunk starts with 0x1B (ESC)
        let binary = "\x1BLua";
        let result: LuaResult<LuaValue> = load_fn.call(binary.to_string());
        assert!(result.is_err(), "Binary bytecode should be rejected");
    }

    #[test]
    fn test_sandbox_allows_text_load() {
        let lua = Lua::new();
        let env = create_sandboxed_env(&lua, "/tmp/mudlib".to_string()).unwrap();
        let load_fn: LuaFunction = env.get("load").unwrap();
        let result: LuaResult<LuaValue> = load_fn.call("return 42".to_string());
        assert!(result.is_ok(), "Text chunks should be allowed");
    }
}
