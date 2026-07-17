use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use mlua::prelude::*;

use crate::config::PermissionConfig;
use crate::core::session::{SessionHandler, SessionId};

/// Resolve a Lua-supplied path relative to the mudlib root, preventing `../` escapes.
fn resolve_jailed_path(mudlib_root: &Path, lua_path: &str) -> Result<PathBuf, String> {
    let requested = mudlib_root.join(lua_path);
    let mut components = Vec::new();
    for component in requested.components() {
        match component {
            std::path::Component::ParentDir => { components.pop(); }
            std::path::Component::CurDir => {}
            c => { components.push(c); }
        }
    }
    let resolved: PathBuf = components.iter().collect();
    if !resolved.starts_with(mudlib_root) {
        return Err(format!("Path '{}' escapes mudlib root", lua_path));
    }
    Ok(resolved)
}

/// Check if the current session has permission for a directory operation.
/// Returns true if no restriction is configured OR the session has the required perm.
fn check_dir_permission(
    resolved_path: &Path,
    mudlib_root: &Path,
    op: &str,
    perm_config: &PermissionConfig,
    sh: &Arc<std::sync::RwLock<SessionHandler>>,
) -> bool {
    // Get relative path string
    let rel = match resolved_path.strip_prefix(mudlib_root) {
        Ok(r) => format!("/{}", r.to_string_lossy().replace('\\', "/")),
        Err(_) => return false, // not under mudlib, already jailed
    };
    match perm_config.dir_permission(&rel, op) {
        None => true, // no restriction
        Some(required_perm) => {
            crate::core::scripting::efuns::get_current_session()
                .and_then(|sid_str| sid_str.parse::<SessionId>().ok())
                .map(|sid| sh.read().unwrap().has_permission(&sid, required_perm))
                .unwrap_or(false)
        }
    }
}

/// Register all file I/O efuns into the Lua global table.
///
/// All path arguments are relative to `mudlib_path` and are jail-checked so
/// that Lua code cannot escape the mudlib directory tree.
pub fn register_io_file_efuns(
    lua: &Lua,
    mudlib_path: &std::path::Path,
    perm_config: Arc<PermissionConfig>,
    sh: Arc<std::sync::RwLock<SessionHandler>>,
) -> mlua::Result<()> {
    let globals = lua.globals();
    let root = mudlib_path.to_path_buf();

    // read_file(path) -> string|nil
    {
        let root = root.clone();
        let perm_config = perm_config.clone();
        let sh = sh.clone();
        let f = lua.create_function(move |lua, path: String| {
            let real = match resolve_jailed_path(&root, &path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("read_file jail violation: {}", e);
                    return Ok(LuaValue::Nil);
                }
            };
            if !check_dir_permission(&real, &root, "read", &perm_config, &sh) {
                tracing::warn!("read_file permission denied for '{}'", path);
                return Ok(LuaValue::Nil);
            }
            match std::fs::read_to_string(&real) {
                Ok(contents) => {
                    let s = lua.create_string(&contents)?;
                    Ok(LuaValue::String(s))
                }
                Err(e) => {
                    tracing::debug!("read_file '{}': {}", path, e);
                    Ok(LuaValue::Nil)
                }
            }
        })?;
        globals.set("read_file", f)?;
    }

    // write_file(path, contents) -> bool
    {
        let root = root.clone();
        let perm_config = perm_config.clone();
        let sh = sh.clone();
        let f = lua.create_function(move |_, (path, contents): (String, String)| {
            let real = match resolve_jailed_path(&root, &path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("write_file jail violation: {}", e);
                    return Ok(false);
                }
            };
            if !check_dir_permission(&real, &root, "write", &perm_config, &sh) {
                tracing::warn!("write_file permission denied for '{}'", path);
                return Ok(false);
            }
            if let Some(parent) = real.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::write(&real, contents) {
                Ok(_) => Ok(true),
                Err(e) => {
                    tracing::warn!("write_file '{}': {}", path, e);
                    Ok(false)
                }
            }
        })?;
        globals.set("write_file", f)?;
    }

    // append_file(path, contents) -> bool
    {
        let root = root.clone();
        let perm_config = perm_config.clone();
        let sh = sh.clone();
        let f = lua.create_function(move |_, (path, contents): (String, String)| {
            let real = match resolve_jailed_path(&root, &path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("append_file jail violation: {}", e);
                    return Ok(false);
                }
            };
            if !check_dir_permission(&real, &root, "write", &perm_config, &sh) {
                tracing::warn!("append_file permission denied for '{}'", path);
                return Ok(false);
            }
            if let Some(parent) = real.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            use std::io::Write;
            match std::fs::OpenOptions::new().create(true).append(true).open(&real) {
                Ok(mut file) => match file.write_all(contents.as_bytes()) {
                    Ok(_) => Ok(true),
                    Err(e) => {
                        tracing::warn!("append_file write '{}': {}", path, e);
                        Ok(false)
                    }
                },
                Err(e) => {
                    tracing::warn!("append_file open '{}': {}", path, e);
                    Ok(false)
                }
            }
        })?;
        globals.set("append_file", f)?;
    }

    // file_exists(path) -> bool
    {
        let root = root.clone();
        let f = lua.create_function(move |_, path: String| {
            match resolve_jailed_path(&root, &path) {
                Ok(real) => Ok(real.exists()),
                Err(_) => Ok(false),
            }
        })?;
        globals.set("file_exists", f)?;
    }

    // list_dir(path) -> table|nil   (array of {name, is_dir, size})
    {
        let root = root.clone();
        let f = lua.create_function(move |lua, path: String| {
            let real = match resolve_jailed_path(&root, &path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("list_dir jail violation: {}", e);
                    return Ok(LuaValue::Nil);
                }
            };
            let rd = match std::fs::read_dir(&real) {
                Ok(rd) => rd,
                Err(e) => {
                    tracing::debug!("list_dir '{}': {}", path, e);
                    return Ok(LuaValue::Nil);
                }
            };
            let arr = lua.create_table()?;
            let mut idx = 1usize;
            for entry in rd.flatten() {
                let meta = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_dir = meta.is_dir();
                let size: i64 = if is_dir { 0 } else { meta.len() as i64 };
                let t = lua.create_table()?;
                t.set("name", name)?;
                t.set("is_dir", is_dir)?;
                t.set("size", size)?;
                arr.set(idx, t)?;
                idx += 1;
            }
            Ok(LuaValue::Table(arr))
        })?;
        globals.set("list_dir", f)?;
    }

    // delete_file(path) -> bool
    {
        let root = root.clone();
        let perm_config = perm_config.clone();
        let sh = sh.clone();
        let f = lua.create_function(move |_, path: String| {
            let real = match resolve_jailed_path(&root, &path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("delete_file jail violation: {}", e);
                    return Ok(false);
                }
            };
            if !check_dir_permission(&real, &root, "write", &perm_config, &sh) {
                tracing::warn!("delete_file permission denied for '{}'", path);
                return Ok(false);
            }
            match std::fs::remove_file(&real) {
                Ok(_) => Ok(true),
                Err(e) => {
                    tracing::debug!("delete_file '{}': {}", path, e);
                    Ok(false)
                }
            }
        })?;
        globals.set("delete_file", f)?;
    }

    // os_time() -> number  (Unix timestamp as float seconds)
    {
        let f = lua.create_function(|_, ()| {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            Ok(secs)
        })?;
        globals.set("os_time", f)?;
    }

    // os_clock() -> number  (wall-clock seconds; std doesn't expose CPU clock portably)
    {
        let f = lua.create_function(|_, ()| {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            Ok(secs)
        })?;
        globals.set("os_clock", f)?;
    }

    // os_date(format) -> string  (chrono local time, strftime format)
    {
        let f = lua.create_function(|_, format: String| {
            let now = chrono::Local::now();
            Ok(now.format(&format).to_string())
        })?;
        globals.set("os_date", f)?;
    }

    Ok(())
}
