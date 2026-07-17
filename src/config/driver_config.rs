use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct DriverConfig {
    pub database: DatabaseConfig,
    pub servers: ServersConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub backend: DatabaseBackend,
    pub url: String,
    pub pool_size: u32,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseBackend {
    Sqlite,
    #[serde(alias = "postgres")]
    Postgresql,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServersConfig {
    pub telnet: Option<TelnetServerConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TelnetServerConfig {
    pub enabled: bool,
    pub bind: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LoggingConfig {
    pub level: String,
    pub file: Option<String>,
}
