//! Path normalization between Lua chunk names and debug-client file paths.
//!
//! Three textual forms have to line up before a breakpoint can ever resolve:
//!
//! | Source | Example |
//! |---|---|
//! | `package.path` / `require` chunk name | `@C:/Code/oxigeon/mudlib/cmds/who.lua` |
//! | What VS Code sends in `setBreakpoints` | `c:\Code\oxigeon\mudlib\cmds\who.lua` |
//! | Windows canonicalized path | `\\?\C:\Code\oxigeon\mudlib\cmds\who.lua` |
//!
//! [`normalize`] folds any of them into a single comparable key.

use std::path::Path;

/// A comparable path key: no `@` prefix, no `\\?\` verbatim prefix, forward
/// slashes, and lowercased on Windows (whose filesystem is case-insensitive,
/// and where debug clients are inconsistent about the drive-letter case).
pub type NormPath = String;

/// Strip Windows' verbatim `\\?\` prefix, which `Path::canonicalize` adds but
/// neither Lua nor any debug client understands.
fn strip_verbatim(s: &mut String) {
    if s.starts_with("//?/") {
        s.drain(..4);
    }
}

/// Render `path` in the same textual form Lua's `require` produces: absolute,
/// forward slashes, no verbatim prefix.
///
/// Falls back to the path as given if it cannot be canonicalized (e.g. it does
/// not exist yet), so this never panics on a missing file.
pub fn abs_lua_path(path: &Path) -> String {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut s = canon.to_string_lossy().replace('\\', "/");
    strip_verbatim(&mut s);
    s
}

/// The `@`-prefixed chunk name for a file.
///
/// The `@` is what makes LuaJIT treat the name as a real file path, so errors
/// and hook events report `path:line` instead of `[string "..."]:line`. Without
/// it, breakpoints in that chunk can never be matched to a client path.
pub fn chunk_name(path: &Path) -> String {
    format!("@{}", abs_lua_path(path))
}

/// Fold a client-supplied path into a comparable key.
pub fn normalize(raw: &str) -> NormPath {
    let mut s = raw.replace('\\', "/");
    strip_verbatim(&mut s);
    if cfg!(windows) {
        s = s.to_lowercase();
    }
    s
}

/// A chunk name rendered as a path a debug client can actually open.
///
/// Separators are unified but case is preserved, unlike [`normalize`], whose
/// lowercasing exists only to make comparison keys. This matters on Windows:
/// LuaJIT's `require` substitutes `?` into `package.path` using the platform
/// separator, so a chunk loaded from a forward-slashed template comes back as
/// `C:/Code/oxigeon/mudlib/cmds\who.lua` — mixed, and rejected by some clients.
pub fn display_path(chunk: &str) -> Option<String> {
    let s = chunk.strip_prefix('@')?;
    let mut s = s.replace('\\', "/");
    strip_verbatim(&mut s);
    Some(s)
}

/// Shorten a chunk name or path to its last two components, for display.
///
/// An absolute Windows path would otherwise swamp a trace line or a journal
/// `source` field.
pub fn short(raw: &str) -> String {
    let s = raw.strip_prefix('@').unwrap_or(raw);
    let mut tail: Vec<&str> = s.rsplit(['/', '\\']).take(2).collect();
    tail.reverse();
    tail.join("/")
}

/// Fold a Lua chunk name into a comparable key.
///
/// Returns `None` for chunks that are not backed by a file — LuaJIT marks those
/// with `=` (or no prefix at all, for `load`ed strings), and a breakpoint can
/// never apply to them.
pub fn chunk_key(chunk: &str) -> Option<NormPath> {
    chunk.strip_prefix('@').map(normalize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_key_requires_a_file_chunk() {
        assert!(chunk_key("init.lua").is_none(), "bare string chunk is not a file");
        assert!(chunk_key("=(load)").is_none(), "`=` chunk is not a file");
        assert!(chunk_key("@/tmp/a.lua").is_some());
    }

    #[test]
    fn separators_and_verbatim_prefix_are_folded() {
        assert_eq!(normalize("//?/C:/x/y.lua"), normalize("C:\\x\\y.lua"));
    }
}
