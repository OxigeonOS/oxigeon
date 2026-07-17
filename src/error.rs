use thiserror::Error;

/// Main error type for Oxigeon. Must be Send + Sync for use across async tasks.
/// mlua::Error is NOT Send+Sync, so Lua errors are stored as strings.
#[derive(Debug, Error)]
pub enum OxigeonError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Database(#[from] diesel::result::Error),

    #[error("Connection pool error: {0}")]
    Pool(#[from] diesel::r2d2::PoolError),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Authentication failed")]
    AuthenticationFailed,

    #[error("Character limit reached")]
    CharacterLimitReached,

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Path traversal detected: {0}")]
    PathTraversal(String),

    /// Lua errors are stored as strings to remain Send+Sync
    #[error("Lua error: {0}")]
    Lua(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Max connections reached")]
    MaxConnectionsReached,

    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    Toml(String),
}

impl From<mlua::Error> for OxigeonError {
    fn from(e: mlua::Error) -> Self {
        OxigeonError::Lua(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, OxigeonError>;
