//! Keeping Lua inside the mudlib.
//!
//! The VM-shaped half of the sandbox — what globals survive, and the text-only
//! chunk loaders — lives in the `oxigeon-lua` crate, because the compute worker
//! is a separate process built against a different Lua and must apply exactly
//! the same list. It is re-exported here so callers keep one import path.
//!
//! What stays is the part that needs the server's own error type: the mudlib
//! jail that `read_file` and friends resolve every path through.

use std::path::{Path, PathBuf};

use crate::error::{OxigeonError, Result};

pub use oxigeon_lua::sandbox::{apply_sandbox, seed_prng};

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
}
