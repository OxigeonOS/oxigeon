use crate::error::{OxigeonError, Result};

pub mod driver_config;
pub mod server_config;
pub mod permissions_config;

pub use driver_config::{DriverConfig, DatabaseConfig, DatabaseBackend, TelnetServerConfig, DebugServerConfig};
pub use server_config::{ServerConfig, GameConfig, SessionsConfig, AccountsConfig, LimitsConfig, MultisessionMode};
pub use permissions_config::PermissionConfig;

pub fn load_driver_config(path: &str) -> Result<DriverConfig> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| OxigeonError::Config(format!("Cannot read {}: {}", path, e)))?;
    toml::from_str(&content)
        .map_err(|e| OxigeonError::Config(format!("Invalid driver config: {}", e)))
}

pub fn load_server_config(path: &str) -> Result<ServerConfig> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| OxigeonError::Config(format!("Cannot read {}: {}", path, e)))?;
    toml::from_str(&content)
        .map_err(|e| OxigeonError::Config(format!("Invalid server config: {}", e)))
}
