//! Permission configuration — loaded from config/permissions.toml

use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct PermissionConfig {
    /// Maps efun name → required permission string
    #[serde(default)]
    pub efuns: HashMap<String, String>,
    /// Maps a jailed path prefix → per-operation permissions.
    ///
    /// Keys are **virtual** paths: the first segment is the root (`/mudlib` or
    /// `/game`), and the rest is relative to it. See [`Self::invalid_directory_keys`].
    #[serde(default)]
    pub directories: HashMap<String, DirPerms>,
    /// Directory keys that named no root, and were therefore dropped.
    ///
    /// Not silently ignored and not silently guessed at: a rule keyed `/areas`
    /// under a two-root jail could mean the mudlib's, the game's, or both, and
    /// those are different trust levels. Applying it to the wrong one is the
    /// same class of failure as the months this file spent with the `/areas`
    /// rule commented out — a boundary everybody believed in that was not there.
    ///
    /// `load_from_file` logs an error per entry, the driver reports them at
    /// startup, and `tests/permission_config.rs` asserts the shipped config has
    /// none, so the repository's own config cannot regress into this state.
    #[serde(skip)]
    pub invalid_directory_keys: Vec<String>,
}

#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct DirPerms {
    pub read:  Option<String>,
    pub write: Option<String>,
}

/// The roots a `[directories]` key may name, as its first path segment.
const ROOTS: [&str; 2] = ["/mudlib", "/game"];

/// Whether `key` is a legal virtual prefix: `/mudlib`, `/game`, or something
/// beneath one of them.
fn names_a_root(key: &str) -> bool {
    ROOTS
        .iter()
        .any(|r| key == *r || key.strip_prefix(r).is_some_and(|rest| rest.starts_with('/')))
}

impl PermissionConfig {
    /// Load from a TOML file. Missing file is not an error — returns empty config.
    pub fn load_from_file(path: &Path) -> Self {
        let mut config: Self = match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).unwrap_or_else(|e| {
                tracing::warn!("Failed to parse permissions config: {}", e);
                PermissionConfig::default()
            }),
            Err(_) => {
                tracing::debug!("No permissions.toml found at {:?}, using defaults (all open)", path);
                PermissionConfig::default()
            }
        };
        config.take_invalid_directory_keys();
        config
    }

    /// Move every rootless `[directories]` key out of the live table and into
    /// [`Self::invalid_directory_keys`], loudly.
    ///
    /// Called by `load_from_file`; also usable by a caller that built the config
    /// some other way.
    pub fn take_invalid_directory_keys(&mut self) -> usize {
        let bad: Vec<String> = self
            .directories
            .keys()
            .filter(|k| !names_a_root(k))
            .cloned()
            .collect();
        for key in &bad {
            self.directories.remove(key);
            tracing::error!(
                "permissions.toml: directory rule '{key}' names no root and has been \
                 DROPPED — it protects nothing. The file efuns are jailed to two \
                 trees, so a rule has to say which: write '/mudlib{key}' or \
                 '/game{key}'."
            );
        }
        self.invalid_directory_keys = bad;
        self.invalid_directory_keys.len()
    }

    /// Find the required permission for a virtual path and operation
    /// ("read" or "write"). Longest matching prefix wins.
    ///
    /// Matching is on **path segments**, not on the raw string: a plain
    /// `starts_with` made `/game/areas_backup` match a rule written for
    /// `/game/areas`, so a directory could inherit a neighbour's protection by
    /// being named after it — or, worse, a rule could be dodged by creating
    /// `/game/areasx` and having it inherit nothing.
    pub fn dir_permission(&self, rel_path: &str, op: &str) -> Option<&String> {
        let mut best: Option<(&str, &DirPerms)> = None;
        for (prefix, perms) in &self.directories {
            if !path_has_prefix(rel_path, prefix) {
                continue;
            }
            let is_longer = best.map(|(b, _)| prefix.len() > b.len()).unwrap_or(true);
            if is_longer {
                best = Some((prefix.as_str(), perms));
            }
        }
        best.and_then(|(_, perms)| match op {
            "read"  => perms.read.as_ref(),
            "write" => perms.write.as_ref(),
            _ => None,
        })
    }
}

/// Is `path` at or below `prefix`, counting whole segments?
fn path_has_prefix(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return true;
    }
    match path.strip_prefix(prefix) {
        Some("") => true,
        Some(rest) => rest.starts_with('/'),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(toml_src: &str) -> PermissionConfig {
        let mut c: PermissionConfig = toml::from_str(toml_src).unwrap();
        c.take_invalid_directory_keys();
        c
    }

    #[test]
    fn a_rule_applies_to_the_directory_it_names_and_everything_under_it() {
        let c = config("[directories]\n\"/game/areas\" = { write = \"w\" }\n");
        assert_eq!(c.dir_permission("/game/areas", "write").map(String::as_str), Some("w"));
        assert_eq!(
            c.dir_permission("/game/areas/crypt/rooms.lua", "write").map(String::as_str),
            Some("w")
        );
    }

    /// The bug a raw `starts_with` had: a neighbour whose name merely begins
    /// with the rule's inherited its protection, and one that did not was
    /// unprotected while looking like it should be.
    #[test]
    fn a_sibling_with_a_prefix_name_is_not_covered() {
        let c = config("[directories]\n\"/game/areas\" = { write = \"w\" }\n");
        assert_eq!(c.dir_permission("/game/areas_backup/x.lua", "write"), None);
        assert_eq!(c.dir_permission("/game/areasx", "write"), None);
    }

    #[test]
    fn the_longest_matching_rule_wins() {
        let c = config(
            "[directories]\n\
             \"/mudlib\" = { write = \"broad\" }\n\
             \"/mudlib/cmds\" = { write = \"narrow\" }\n",
        );
        assert_eq!(c.dir_permission("/mudlib/lib/x.lua", "write").map(String::as_str), Some("broad"));
        assert_eq!(c.dir_permission("/mudlib/cmds/who.lua", "write").map(String::as_str), Some("narrow"));
    }

    /// A rule that names no root is dropped and reported, never guessed at.
    #[test]
    fn a_rootless_key_is_dropped_and_recorded() {
        let c = config("[directories]\n\"/areas\" = { write = \"w\" }\n");
        assert_eq!(c.invalid_directory_keys, vec!["/areas".to_string()]);
        assert_eq!(c.dir_permission("/game/areas/x.lua", "write"), None);
        assert_eq!(c.dir_permission("/mudlib/areas/x.lua", "write"), None);
        assert_eq!(c.dir_permission("/areas/x.lua", "write"), None);
    }

    #[test]
    fn a_root_may_be_named_on_its_own() {
        let c = config("[directories]\n\"/game\" = { read = \"r\" }\n");
        assert!(c.invalid_directory_keys.is_empty());
        assert_eq!(c.dir_permission("/game/areas/x.lua", "read").map(String::as_str), Some("r"));
    }
}
