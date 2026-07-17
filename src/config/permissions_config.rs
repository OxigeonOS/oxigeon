//! Permission configuration — loaded from config/permissions.toml

use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct PermissionConfig {
    /// Maps efun name → required permission string
    #[serde(default)]
    pub efuns: HashMap<String, String>,
    /// Maps mudlib-relative path prefix → per-operation permissions
    #[serde(default)]
    pub directories: HashMap<String, DirPerms>,
}

#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct DirPerms {
    pub read:  Option<String>,
    pub write: Option<String>,
}

impl PermissionConfig {
    /// Load from a TOML file. Missing file is not an error — returns empty config.
    pub fn load_from_file(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).unwrap_or_else(|e| {
                tracing::warn!("Failed to parse permissions config: {}", e);
                PermissionConfig::default()
            }),
            Err(_) => {
                tracing::debug!("No permissions.toml found at {:?}, using defaults (all open)", path);
                PermissionConfig::default()
            }
        }
    }

    /// Find the required permission for a given directory path and operation ("read" or "write").
    /// Uses longest-prefix matching.
    pub fn dir_permission(&self, rel_path: &str, op: &str) -> Option<&String> {
        // Find the longest matching prefix
        let mut best: Option<(&str, &DirPerms)> = None;
        for (prefix, perms) in &self.directories {
            if rel_path.starts_with(prefix.as_str()) {
                let is_longer = best.map(|(b, _)| prefix.len() > b.len()).unwrap_or(true);
                if is_longer {
                    best = Some((prefix.as_str(), perms));
                }
            }
        }
        best.and_then(|(_, perms)| match op {
            "read"  => perms.read.as_ref(),
            "write" => perms.write.as_ref(),
            _ => None,
        })
    }
}
