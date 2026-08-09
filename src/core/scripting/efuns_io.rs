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

/// Resolve a Lua-supplied path relative to one root, preventing `../` escapes.
///
/// Both sides are normalized before the comparison. Normalizing only the
/// requested path was a real bug: the configured root is `./mudlib`, the
/// requested path normalized to `mudlib/...`, and `starts_with` then said no —
/// so every legitimate read was refused on a default install. `audit_d` could
/// not load its watch list, and said nothing about it.
fn resolve_jailed_path(root: &Path, lua_path: &str) -> Result<PathBuf, String> {
    let root = lexically_normalize(root);
    let resolved = lexically_normalize(&root.join(lua_path));
    if !resolved.starts_with(&root) {
        return Err(format!("Path '{}' escapes the jail root", lua_path));
    }
    Ok(resolved)
}

/// Which of the two trees a path resolved into.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Layer {
    Game,
    Mudlib,
}

impl Layer {
    fn name(self) -> &'static str {
        match self {
            Layer::Game => "game",
            Layer::Mudlib => "mudlib",
        }
    }
}

/// What the caller is about to do. Decides where an *unprefixed* path lands.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Op {
    Read,
    Write,
}

/// The two trees Lua may touch, and the rules for choosing between them.
///
/// # Why an unprefixed write still means the mudlib
///
/// Reads search the game layer first and fall back to the mudlib, mirroring
/// `package.path` and what `list_dir` has always done: the layer that would be
/// *required* is the layer that is read.
///
/// Writes deliberately do **not** follow that rule. A write names a file that
/// may not exist yet, so there is nothing to search — the root has to be chosen,
/// and every rule for choosing it automatically is a guess. `audit_d` writes
/// `logs/audit_watch.json` (`audit_d.lua:141`) and creates it on first use; a
/// "new files go to the game root" rule would relocate it, a later read would
/// still find it through the fallback, and the two copies would drift with
/// nothing reporting it.
///
/// So: an unprefixed write goes where it has always gone, and anything that
/// means the game tree says so. `write_file("game:areas/crypt/rooms.lua", …)`.
struct Roots {
    mudlib: PathBuf,
    game: Option<PathBuf>,
}

impl Roots {
    fn of(&self, layer: Layer) -> Option<&PathBuf> {
        match layer {
            Layer::Mudlib => Some(&self.mudlib),
            Layer::Game => self.game.as_ref(),
        }
    }

    /// Split a `game:` / `mudlib:` scheme off the front of a Lua path.
    ///
    /// An unknown scheme is an **error** rather than a filename. `gmae:rooms.lua`
    /// is a typo, and treating it as a relative path would create a file with a
    /// colon in its name on the platforms that allow one and fail obscurely on
    /// the ones that do not.
    fn split_scheme(lua_path: &str) -> Result<(Option<Layer>, &str), String> {
        let Some((head, rest)) = lua_path.split_once(':') else {
            return Ok((None, lua_path));
        };
        // Only a bare lowercase word before the colon is a scheme attempt.
        // Anything else — a Windows drive letter, a stray colon mid-path — is
        // not, and falls through to the jail, which refuses it anyway.
        if head.is_empty() || !head.bytes().all(|b| b.is_ascii_lowercase()) {
            return Ok((None, lua_path));
        }
        match head {
            "game" => Ok((Some(Layer::Game), rest)),
            "mudlib" => Ok((Some(Layer::Mudlib), rest)),
            other => Err(format!(
                "unknown root '{other}:' — the roots are 'game:' and 'mudlib:'"
            )),
        }
    }

    /// Jail `lua_path` and say which tree it landed in.
    fn resolve(&self, lua_path: &str, op: Op) -> Result<(PathBuf, Layer), String> {
        let (explicit, rest) = Self::split_scheme(lua_path)?;

        if let Some(layer) = explicit {
            let Some(root) = self.of(layer) else {
                return Err(format!("no {} root is configured", layer.name()));
            };
            return resolve_jailed_path(root, rest).map(|p| (p, layer));
        }

        // Unprefixed. A read prefers whichever tree actually has the file; a
        // write stays in the mudlib. See the type's doc comment.
        if op == Op::Read {
            if let Some(game) = &self.game {
                if let Ok(p) = resolve_jailed_path(game, rest) {
                    if p.exists() {
                        return Ok((p, Layer::Game));
                    }
                }
            }
        }
        resolve_jailed_path(&self.mudlib, rest).map(|p| (p, Layer::Mudlib))
    }

    /// The virtual path a resolved file is known by: `/game/areas/crypt/rooms.lua`.
    ///
    /// This is the string `permissions.toml` keys on, the string `ls` prints and
    /// the string a builder types. One namespace for the config, the shell and
    /// the error messages, rather than three that have to be kept in step.
    fn virtual_path(&self, resolved: &Path, layer: Layer) -> Option<String> {
        let root = lexically_normalize(self.of(layer)?);
        let rest = resolved.strip_prefix(&root).ok()?;
        let rest = rest.to_string_lossy().replace('\\', "/");
        Some(if rest.is_empty() {
            format!("/{}", layer.name())
        } else {
            format!("/{}/{}", layer.name(), rest)
        })
    }
}

/// Resolve a Lua path for reading, against both roots, for callers outside this
/// module.
///
/// Exists so there is one jail rather than two. `verify_file` used
/// `sandbox::resolve_jailed_path` instead, which is stricter in a way nothing
/// wanted: it refuses any path *containing* `..`, so `verify cmds/../who.lua`
/// was refused while `read_file("cmds/../who.lua")` succeeded. Two jails that
/// disagree about the same path is a question with two answers, and the
/// interesting half — whether the resolved path is inside a root — they agreed
/// on all along.
///
/// Returns the real path and the virtual name (`/game/areas/crypt/rooms.lua`),
/// which is what an error message should quote.
pub fn resolve_read_path(
    mudlib_root: &Path,
    game_root: Option<&Path>,
    lua_path: &str,
) -> Result<(PathBuf, String), String> {
    let roots = Roots {
        mudlib: mudlib_root.to_path_buf(),
        game: game_root.map(|p| p.to_path_buf()),
    };
    let (real, layer) = roots.resolve(lua_path, Op::Read)?;
    let virt = roots
        .virtual_path(&real, layer)
        .unwrap_or_else(|| lua_path.to_string());
    Ok((real, virt))
}

/// The permission a directory rule demands here, or `None` if it is unrestricted.
fn required_dir_permission<'a>(
    roots: &Roots,
    resolved: &Path,
    layer: Layer,
    op: &str,
    perm_config: &'a PermissionConfig,
) -> Option<&'a String> {
    let virt = roots.virtual_path(resolved, layer)?;
    perm_config.dir_permission(&virt, op)
}

/// Check if the current session has permission for a directory operation.
/// Returns true if no restriction is configured OR the session has the required perm.
///
/// The same `[directories]` table governs both roots. Exempting the game root —
/// which `list_dir` used to do, on the reasoning that `permissions.toml`
/// described only the mudlib — would make `dir.write.game.areas` decorative for
/// *exactly* the files OLC writes, which is the "rule that was a no-op" this
/// config file already documents once.
fn check_dir_permission(
    roots: &Roots,
    resolved: &Path,
    layer: Layer,
    op: &str,
    perm_config: &PermissionConfig,
    sh: &Arc<std::sync::RwLock<SessionHandler>>,
) -> bool {
    match required_dir_permission(roots, resolved, layer, op, perm_config) {
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

/// Why a write was refused, in a form worth showing a builder.
///
/// "permission denied" tells you nothing you can act on. The permission's *name*
/// is the one thing that turns the refusal into a request somebody can grant, so
/// it goes in the returned error rather than only into the server log where the
/// person who hit it cannot see it.
fn denial_reason(
    roots: &Roots,
    resolved: &Path,
    layer: Layer,
    op: &str,
    perm_config: &PermissionConfig,
) -> String {
    let virt = roots
        .virtual_path(resolved, layer)
        .unwrap_or_else(|| resolved.to_string_lossy().into_owned());
    match required_dir_permission(roots, resolved, layer, op, perm_config) {
        Some(perm) => format!("permission denied: {virt} needs '{perm}' to {op}"),
        None => format!("permission denied: {virt}"),
    }
}

/// Register all file I/O efuns into the Lua global table.
///
/// Path arguments are relative to one of two jail roots — the mudlib and the
/// game layer — and are jail-checked so Lua cannot escape either. A path may
/// name its root explicitly with a `game:` or `mudlib:` prefix; see [`Roots`]
/// for what an unprefixed one means.
pub fn register_io_file_efuns(
    lua: &Lua,
    mudlib_path: &std::path::Path,
    game_path: Option<&std::path::Path>,
    perm_config: Arc<PermissionConfig>,
    sh: Arc<std::sync::RwLock<SessionHandler>>,
    debug_state: crate::core::scripting::debugger::SharedDebugState,
) -> mlua::Result<()> {
    let globals = lua.globals();
    let roots = Arc::new(Roots {
        mudlib: mudlib_path.to_path_buf(),
        game: game_path.map(|p| p.to_path_buf()),
    });

    // read_file(path) -> string|nil
    {
        let roots = roots.clone();
        let perm_config = perm_config.clone();
        let sh = sh.clone();
        let f = lua.create_function(move |lua, path: String| {
            let (real, layer) = match roots.resolve(&path, Op::Read) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("read_file jail violation: {}", e);
                    return Ok(LuaValue::Nil);
                }
            };
            if !check_dir_permission(&roots, &real, layer, "read", &perm_config, &sh) {
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

    // write_file(path, contents) -> (bool ok, string? err)
    //
    // The second return value is why this is not `pcall`-able: these efuns
    // *return* failure rather than raising it, so `local ok, err =
    // pcall(write_file, ...)` yields `ok = true, err = false` and the guard
    // never fires. `codegen_d` was written that way and reported success for
    // every refused write for as long as it existed. Raising instead would be
    // the other fix, but `types/oxigeon.lua` documents a boolean return and the
    // change ripples; a second value is additive and the first stays truthy.
    {
        let roots = roots.clone();
        let perm_config = perm_config.clone();
        let sh = sh.clone();
        let f = lua.create_function(move |_, (path, contents): (String, String)| {
            let (real, layer) = match roots.resolve(&path, Op::Write) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("write_file jail violation: {}", e);
                    return Ok((false, Some(e)));
                }
            };
            if !check_dir_permission(&roots, &real, layer, "write", &perm_config, &sh) {
                let why = denial_reason(&roots, &real, layer, "write", &perm_config);
                tracing::warn!("write_file permission denied for '{}'", path);
                return Ok((false, Some(why)));
            }
            if let Some(parent) = real.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    tracing::warn!("write_file mkdir '{}': {}", path, e);
                    return Ok((false, Some(e.to_string())));
                }
            }
            match std::fs::write(&real, contents) {
                Ok(_) => Ok((true, None)),
                Err(e) => {
                    tracing::warn!("write_file '{}': {}", path, e);
                    Ok((false, Some(e.to_string())))
                }
            }
        })?;
        globals.set("write_file", f)?;
    }

    // append_file(path, contents) -> (bool ok, string? err)
    {
        let roots = roots.clone();
        let perm_config = perm_config.clone();
        let sh = sh.clone();
        let f = lua.create_function(move |_, (path, contents): (String, String)| {
            let (real, layer) = match roots.resolve(&path, Op::Write) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("append_file jail violation: {}", e);
                    return Ok((false, Some(e)));
                }
            };
            if !check_dir_permission(&roots, &real, layer, "write", &perm_config, &sh) {
                let why = denial_reason(&roots, &real, layer, "write", &perm_config);
                tracing::warn!("append_file permission denied for '{}'", path);
                return Ok((false, Some(why)));
            }
            if let Some(parent) = real.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    tracing::warn!("append_file mkdir '{}': {}", path, e);
                    return Ok((false, Some(e.to_string())));
                }
            }
            use std::io::Write;
            match std::fs::OpenOptions::new().create(true).append(true).open(&real) {
                Ok(mut file) => match file.write_all(contents.as_bytes()) {
                    Ok(_) => Ok((true, None)),
                    Err(e) => {
                        tracing::warn!("append_file write '{}': {}", path, e);
                        Ok((false, Some(e.to_string())))
                    }
                },
                Err(e) => {
                    tracing::warn!("append_file open '{}': {}", path, e);
                    Ok((false, Some(e.to_string())))
                }
            }
        })?;
        globals.set("append_file", f)?;
    }

    // file_exists(path) -> bool
    {
        let roots = roots.clone();
        let f = lua.create_function(move |_, path: String| {
            match roots.resolve(&path, Op::Read) {
                Ok((real, _)) => Ok(real.exists()),
                Err(_) => Ok(false),
            }
        })?;
        globals.set("file_exists", f)?;
    }

    // file_root(path) -> "game"|"mudlib"|nil
    //
    // Where a read of this path would land, or nil if it exists in neither
    // tree. Without it there is no way to ask *which* file you got, which makes
    // shadowing between the layers untestable and makes `ls` unable to say
    // where a name came from.
    {
        let roots = roots.clone();
        let f = lua.create_function(move |_, path: String| {
            match roots.resolve(&path, Op::Read) {
                Ok((real, layer)) if real.exists() => Ok(Some(layer.name())),
                _ => Ok(None),
            }
        })?;
        globals.set("file_root", f)?;
    }

    // dir_permission(virtual_path, "read"|"write") -> string|nil
    //
    // The permission a directory rule demands at this path, or nil when it is
    // unrestricted. Read-only and answers about the *rule*, not about you, so it
    // is ungated: `permissions.toml` is not a secret, and a shell that cannot
    // ask this has to probe by attempting the operation — which conflates
    // "denied" with "does not exist" and costs a syscall per entry listed.
    //
    // Takes a **virtual** path (`/game/areas`), not a jail path, because that is
    // what `permissions.toml` keys on and what `ls` prints. It answers about the
    // rule, so the path need not exist — asking whether you could write
    // `/game/areas/crypt` before creating it is the normal case.
    {
        let perm_config = perm_config.clone();
        let f = lua.create_function(move |_, (path, op): (String, String)| {
            if op != "read" && op != "write" {
                return Ok(None);
            }
            // Normalize the spelling only: leading slash, no trailing one, `\`
            // folded to `/`. Whether the first segment names a real root is the
            // config's business — an unmatched path simply has no rule.
            let cleaned = path.replace('\\', "/");
            let cleaned = cleaned.trim_end_matches('/');
            let virt = if cleaned.starts_with('/') {
                cleaned.to_string()
            } else {
                format!("/{cleaned}")
            };
            Ok(perm_config.dir_permission(&virt, &op).cloned())
        })?;
        globals.set("dir_permission", f)?;
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
    // Both roots are searched *when the path names neither*, because command
    // and area discovery spans the mudlib and the game layer, and each is
    // jailed against its own root: a path that escapes one is refused for that
    // root rather than for the call, so listing `cmds` still works when only one
    // layer has it. Entries are deduplicated by name with the game layer
    // winning, matching `package.path` order — the layer that would be required
    // is the layer that is reported, and each entry carries a `root` field so a
    // caller that needs to know which can ask.
    //
    // A path that *does* name a root — `list_dir("game:areas")` — lists only
    // that one. The merge is what discovery wants and the opposite of what a
    // builder deciding where a file goes wants, so it is chosen at the call
    // site rather than assumed.
    {
        let roots = roots.clone();
        let perm_config = perm_config.clone();
        let sh = sh.clone();
        let f = lua.create_function(move |lua, path: String| {
            let arr = lua.create_table()?;
            let mut idx = 1usize;
            let mut seen = std::collections::HashSet::new();
            let mut any_root_readable = false;

            let (explicit, rest) = match Roots::split_scheme(&path) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("list_dir: {}", e);
                    return Ok(LuaValue::Nil);
                }
            };
            let layers: Vec<Layer> = match explicit {
                Some(l) => vec![l],
                None => vec![Layer::Game, Layer::Mudlib],
            };

            for layer in layers {
                let Some(base) = roots.of(layer) else { continue };
                let real = match resolve_jailed_path(base, rest) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!("list_dir jail violation: {}", e);
                        continue;
                    }
                };
                // Reading a directory is a read, and the same `[directories]`
                // table governs both roots. The game layer used to be exempt on
                // the reasoning that the config described only the mudlib; under
                // a two-root jail that would make a rule written for `/game/...`
                // a no-op, which is the failure this whole file keeps having.
                if !check_dir_permission(&roots, &real, layer, "read", &perm_config, &sh) {
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
                    t.set("root", layer.name())?;
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

    // delete_file(path) -> (bool ok, string? err)
    //
    // Resolved as a *read*, deliberately: deleting is the one operation with no
    // sensible answer for a file that does not exist, so it goes to whichever
    // tree actually holds it rather than to the write default. Getting that
    // backwards would mean `delete_file("areas/x.lua")` silently missing the
    // game-layer file it was plainly aimed at and reporting "no such file".
    {
        let roots = roots.clone();
        let perm_config = perm_config.clone();
        let sh = sh.clone();
        let f = lua.create_function(move |_, path: String| {
            let (real, layer) = match roots.resolve(&path, Op::Read) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("delete_file jail violation: {}", e);
                    return Ok((false, Some(e)));
                }
            };
            if !check_dir_permission(&roots, &real, layer, "write", &perm_config, &sh) {
                let why = denial_reason(&roots, &real, layer, "write", &perm_config);
                tracing::warn!("delete_file permission denied for '{}'", path);
                return Ok((false, Some(why)));
            }
            match std::fs::remove_file(&real) {
                Ok(_) => Ok((true, None)),
                Err(e) => {
                    tracing::debug!("delete_file '{}': {}", path, e);
                    Ok((false, Some(e.to_string())))
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
            // Whole seconds, as an integer. Nothing in the mudlib wants
            // sub-second game time — regeneration, cooldowns and effect expiry
            // all work in seconds — and on 5.3+ a float here would render every
            // stored timestamp as `1712345678.0`.
            Ok((secs - st.paused_secs()).floor() as i64)
        })?;
        globals.set("os_time", f)?;
    }

    // os_time_precise() -> number  (game time, fractional seconds)
    //
    // `os_time` with the floor taken off. Same clock and the same subtraction of
    // frozen time, so a breakpoint still does not expire anything — the only
    // difference is that it keeps the fraction.
    //
    // It exists because "nothing in the mudlib wants sub-second game time",
    // asserted immediately above, stopped being true. A movement track charges a
    // roundtime per room and a city street costs half a second; against an
    // integer clock, `os_time() + 0.5` and `os_time() + 1.0` expire on the same
    // tick, so every terrain under a second was the same terrain and a
    // 7%-per-rank discount was invisible unless it happened to cross a whole
    // second.
    //
    // Deliberately **not** a change to `os_time`. Every stored timestamp in the
    // game goes through that one — save files, journal entries, cooldown
    // documents — and on 5.3+ making it a float renders all of them as
    // `1712345678.0`. A caller that needs the fraction asks for it; everything
    // else keeps the integer it was written against.
    //
    // Not `os_clock` either: that is the raw wall clock with no subtraction, so
    // anything measured against it keeps ticking down while the world is stopped
    // at a breakpoint.
    {
        let st = debug_state.clone();
        let f = lua.create_function(move |_, ()| {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            Ok(secs - st.paused_secs())
        })?;
        globals.set("os_time_precise", f)?;
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

    // ─── the two-root jail ───────────────────────────────────────────────────

    fn roots() -> Roots {
        Roots {
            mudlib: PathBuf::from("./mudlib"),
            game: Some(PathBuf::from("./game")),
        }
    }

    #[test]
    fn an_explicit_root_goes_where_it_says() {
        let r = roots();
        assert_eq!(
            r.resolve("game:areas/crypt/rooms.lua", Op::Write).unwrap(),
            (PathBuf::from("game/areas/crypt/rooms.lua"), Layer::Game)
        );
        assert_eq!(
            r.resolve("mudlib:lib/strings.lua", Op::Read).unwrap(),
            (PathBuf::from("mudlib/lib/strings.lua"), Layer::Mudlib)
        );
    }

    /// The back-compatibility that lets every existing caller stay put.
    /// `audit_d` writes `logs/audit_watch.json` and creates it on first use; a
    /// rule that sent new files to the game root would relocate it, a later read
    /// would still find it through the fallback, and the two would drift.
    #[test]
    fn an_unprefixed_write_stays_in_the_mudlib() {
        let r = roots();
        assert_eq!(
            r.resolve("logs/audit_watch.json", Op::Write).unwrap(),
            (PathBuf::from("mudlib/logs/audit_watch.json"), Layer::Mudlib)
        );
        // Even for a path that plainly belongs to the game layer: saying so is
        // one prefix, and guessing is a class of bug.
        assert_eq!(
            r.resolve("areas/crypt/rooms.lua", Op::Write).unwrap().1,
            Layer::Mudlib
        );
    }

    /// An unprefixed read falls back to the mudlib when the game layer has no
    /// such file — which, for a path that exists in neither, is every read.
    #[test]
    fn an_unprefixed_read_of_a_missing_file_names_the_mudlib() {
        let r = roots();
        assert_eq!(
            r.resolve("lib/nothing_here.lua", Op::Read).unwrap(),
            (PathBuf::from("mudlib/lib/nothing_here.lua"), Layer::Mudlib)
        );
    }

    /// A prefix must not become a way around the jail.
    #[test]
    fn a_prefixed_path_is_still_jailed() {
        let r = roots();
        assert!(r.resolve("game:../../etc/passwd", Op::Read).is_err());
        assert!(r.resolve("mudlib:../Cargo.toml", Op::Write).is_err());
        assert!(r.resolve("game:areas/../../secrets", Op::Write).is_err());
    }

    /// A typo is an error, not a filename. `gmae:rooms.lua` creating a file
    /// called `gmae:rooms.lua` would be obscure on the platforms that allow it
    /// and an unexplained failure on the ones that do not.
    #[test]
    fn an_unknown_root_is_refused_rather_than_treated_as_a_name() {
        let r = roots();
        let err = r.resolve("gmae:areas/x.lua", Op::Write).unwrap_err();
        assert!(err.contains("unknown root"), "{err}");
        assert!(err.contains("game:"), "the message should say what is valid: {err}");
    }

    /// Not every colon is a scheme. A Windows drive letter is uppercase and a
    /// stray colon mid-path has no bare-word head, so both fall through to the
    /// jail — which refuses them on their own merits.
    #[test]
    fn a_colon_that_is_not_a_scheme_falls_through_to_the_jail() {
        let r = roots();
        assert!(r.resolve("C:/Windows/System32/config", Op::Read).is_err());
        assert!(Roots::split_scheme("weird:name.lua").is_err()); // a bare word: still a scheme attempt
        assert_eq!(Roots::split_scheme("a/b:c.lua").unwrap().0, None);
    }

    #[test]
    fn a_missing_game_root_is_reported_rather_than_falling_back() {
        let r = Roots { mudlib: PathBuf::from("./mudlib"), game: None };
        assert!(r.resolve("game:areas/x.lua", Op::Write).is_err());
        // …and an unprefixed path still works, so a mudlib-only install is fine.
        assert_eq!(r.resolve("lib/x.lua", Op::Read).unwrap().1, Layer::Mudlib);
    }

    /// The string `permissions.toml` keys on, `ls` prints, and a builder types.
    #[test]
    fn a_resolved_path_knows_its_virtual_name() {
        let r = roots();
        let (p, l) = r.resolve("game:areas/crypt/rooms.lua", Op::Write).unwrap();
        assert_eq!(r.virtual_path(&p, l).unwrap(), "/game/areas/crypt/rooms.lua");

        let (p, l) = r.resolve("mudlib:lib/strings.lua", Op::Read).unwrap();
        assert_eq!(r.virtual_path(&p, l).unwrap(), "/mudlib/lib/strings.lua");

        let (p, l) = r.resolve("game:", Op::Read).unwrap();
        assert_eq!(r.virtual_path(&p, l).unwrap(), "/game");
    }
}
