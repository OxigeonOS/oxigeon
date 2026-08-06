//! Rendering a filesystem path the way Lua's `require` does.
//!
//! Split out of the server's `debugger::paths` — which also maps chunk names to
//! what a debug client sends, and has no business in a compute worker — because
//! `package.path` has to be built the same way on both sides. A worker that
//! spelled its roots differently would `require` different files than the game
//! does, which is the sort of divergence you only find in production.

use std::path::Path;

/// Render `path` as `require` produces it: absolute, forward slashes, no
/// Windows verbatim `\\?\` prefix.
///
/// Falls back to the path as given if it cannot be canonicalized (it may not
/// exist yet), so this never panics on a missing directory.
pub fn abs_lua_path(path: &Path) -> String {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut s = canon.to_string_lossy().replace('\\', "/");
    if s.starts_with("//?/") {
        s.drain(..4);
    }
    s
}
