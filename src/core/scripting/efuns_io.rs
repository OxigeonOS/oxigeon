use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use mlua::prelude::*;
use crate::core::lock::RwLockExt;

use crate::config::PermissionConfig;
use crate::core::session::{SessionHandler, SessionId};

/// Drop `.` and resolve `..` without touching the filesystem, so a path that
/// does not exist yet can still be checked.
fn lexically_normalize(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            c => components.push(c),
        }
    }
    components.iter().collect()
}

/// Resolve a Lua-supplied path relative to the mudlib root, preventing `../` escapes.
///
/// Both sides are normalized before the comparison. Normalizing only the
/// requested path was a real bug: the configured root is `./mudlib`, the
/// requested path normalized to `mudlib/...`, and `starts_with` then said no —
/// so every legitimate read was refused on a default install. `audit_d` could
/// not load its watch list, and said nothing about it.
fn resolve_jailed_path(mudlib_root: &Path, lua_path: &str) -> Result<PathBuf, String> {
    let root = lexically_normalize(mudlib_root);
    let resolved = lexically_normalize(&root.join(lua_path));
    if !resolved.starts_with(&root) {
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
    // Against the *normalized* root, for the same reason `resolve_jailed_path`
    // normalizes: `resolved_path` has had its `./` removed and the configured
    // root has not, so a raw `strip_prefix` fails on every relative root and
    // this function denies everything.
    let rel = match resolved_path.strip_prefix(lexically_normalize(mudlib_root)) {
        Ok(r) => format!("/{}", r.to_string_lossy().replace('\\', "/")),
        Err(_) => return false, // not under mudlib, already jailed
    };
    match perm_config.dir_permission(&rel, op) {
        None => true, // no restriction
        // Engine-internal dispatch — a daemon writing on a tick, the mudlib
        // load — has no session and acts with the driver's own authority. See
        // `efuns::enter_system_dispatch`.
        Some(_) if crate::core::scripting::efuns::is_system_dispatch() => true,
        Some(required_perm) => {
            crate::core::scripting::efuns::get_current_session()
                .and_then(|sid_str| sid_str.parse::<SessionId>().ok())
                .map(|sid| sh.read_recover().has_permission(&sid, required_perm))
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
    game_path: Option<&std::path::Path>,
    perm_config: Arc<PermissionConfig>,
    sh: Arc<std::sync::RwLock<SessionHandler>>,
    debug_state: crate::core::scripting::debugger::SharedDebugState,
) -> mlua::Result<()> {
    let globals = lua.globals();
    let root = mudlib_path.to_path_buf();
    let game_root = game_path.map(|p| p.to_path_buf());

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
    //
    // This is the *only* `list_dir`. There used to be a second one registered
    // later, from `register_utility_efuns`, which overwrote this one — and it
    // joined the caller's path straight onto the two roots with no jail check
    // and no permission check, so `list_dir("../../..")` escaped. The jailed
    // implementation existed the whole time and production never reached it,
    // which is the same failure shape as the sandbox and instruction-limit bugs
    // `CLAUDE.md`'s testing section was written about. `tests/list_dir_jail.rs`
    // asks the question through the engine's own VM, so a helper-level test
    // cannot pass while the reachable version is broken.
    //
    // Both roots are searched because command and area discovery spans the
    // mudlib and the game layer, and each is jailed against its own root: a
    // path that escapes one is refused for that root rather than for the call,
    // so listing `cmds` still works when only one layer has it. Entries are
    // deduplicated by name with the game layer winning, matching `package.path`
    // order — the layer that would be required is the layer that is reported.
    {
        let root = root.clone();
        let game_root = game_root.clone();
        let perm_config = perm_config.clone();
        let sh = sh.clone();
        let f = lua.create_function(move |lua, path: String| {
            let arr = lua.create_table()?;
            let mut idx = 1usize;
            let mut seen = std::collections::HashSet::new();
            let mut any_root_readable = false;

            let roots: Vec<&std::path::PathBuf> =
                game_root.iter().chain(std::iter::once(&root)).collect();

            for base in roots {
                let real = match resolve_jailed_path(base, &path) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!("list_dir jail violation: {}", e);
                        continue;
                    }
                };
                // Reading a directory is a read. The mudlib root is the one
                // `permissions.toml` describes, so a game-root listing is not
                // gated by it — there is no rule that could name it.
                if base == &root && !check_dir_permission(&real, &root, "read", &perm_config, &sh) {
                    tracing::warn!("list_dir permission denied for '{}'", path);
                    continue;
                }
                let rd = match std::fs::read_dir(&real) {
                    Ok(rd) => rd,
                    Err(e) => {
                        tracing::debug!("list_dir '{}': {}", path, e);
                        continue;
                    }
                };
                any_root_readable = true;
                for entry in rd.flatten() {
                    let meta = match entry.metadata() {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !seen.insert(name.clone()) {
                        continue;
                    }
                    let is_dir = meta.is_dir();
                    let size: i64 = if is_dir { 0 } else { meta.len() as i64 };
                    let t = lua.create_table()?;
                    t.set("name", name)?;
                    t.set("is_dir", is_dir)?;
                    t.set("size", size)?;
                    arr.set(idx, t)?;
                    idx += 1;
                }
            }

            // `nil` for "no such directory, or refused" and an empty table for
            // "a directory with nothing in it" — the caller can tell a missing
            // command path from an empty one, which is the difference between a
            // misconfiguration and a fact.
            if any_root_readable {
                Ok(LuaValue::Table(arr))
            } else {
                Ok(LuaValue::Nil)
            }
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

    // uuid() -> string
    //
    // A globally unique handle for something that needs addressing but has no
    // natural key. Item instances are the first user: a monotonic counter is
    // enough for mobs, which are never saved, but a container in a player's
    // inventory *is* saved — and a counter that restarts at zero on every boot
    // would hand out an id that already means something else in somebody's save
    // file. That is a data-corruption bug that only shows up after a restart.
    //
    // v4, so it carries no timestamp and no MAC address: an id that leaks when
    // the server was started is an id that leaks more than an id needs to.
    {
        let f = lua.create_function(|_, ()| Ok(uuid::Uuid::new_v4().to_string()))?;
        globals.set("uuid", f)?;
    }

    // os_time() -> number  (Unix timestamp as float seconds)
    //
    // This is *game* time, not wall time: it excludes any period the debugger
    // had the world frozen. The mudlib's entire sense of time runs through
    // here — regeneration settles against it, cooldowns and effects expire
    // against it — and freezing the VM at a breakpoint does not freeze the
    // clock. Without the subtraction, a minute spent reading a stack trace
    // healed the monster you were fighting by twenty hit points.
    //
    // With `[servers.debug]` absent or disabled — the default — nothing ever
    // pauses, the counter stays zero, and this is the wall clock exactly as it
    // was. See `DebugState::paused_ms`.
    {
        let st = debug_state.clone();
        let f = lua.create_function(move |_, ()| {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            Ok(secs - st.paused_secs())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The configured `mudlib_path` is `./mudlib`, so this is the real shape.
    #[test]
    fn a_relative_root_still_admits_paths_inside_it() {
        let root = Path::new("./mudlib");
        assert_eq!(
            resolve_jailed_path(root, "logs/audit_watch.json").unwrap(),
            Path::new("mudlib/logs/audit_watch.json")
        );
    }

    #[test]
    fn an_absolute_root_works_the_same_way() {
        let root = Path::new("/srv/oxigeon/mudlib");
        assert_eq!(
            resolve_jailed_path(root, "cmds/who.lua").unwrap(),
            Path::new("/srv/oxigeon/mudlib/cmds/who.lua")
        );
    }

    #[test]
    fn traversal_out_of_a_relative_root_is_still_refused() {
        let root = Path::new("./mudlib");
        assert!(resolve_jailed_path(root, "../Cargo.toml").is_err());
        assert!(resolve_jailed_path(root, "../../etc/passwd").is_err());
        assert!(resolve_jailed_path(root, "cmds/../../Cargo.toml").is_err());
    }

    /// `..` that stays inside the root is fine — refusing it would break
    /// nothing dangerous and surprise anyone building a path by hand.
    #[test]
    fn traversal_that_stays_inside_the_root_is_allowed() {
        let root = Path::new("./mudlib");
        assert_eq!(
            resolve_jailed_path(root, "cmds/../lib/strings.lua").unwrap(),
            Path::new("mudlib/lib/strings.lua")
        );
    }

    /// A sibling directory whose name merely starts with the root's must not
    /// pass — `starts_with` on components, not on the string, is what stops it.
    #[test]
    fn a_sibling_with_a_prefix_name_does_not_slip_through() {
        let root = Path::new("./mudlib");
        assert!(resolve_jailed_path(root, "../mudlib_secrets/keys.txt").is_err());
    }
}
