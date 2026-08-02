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
    /// Debug Adapter Protocol server for VS Code. Absent (the default) leaves
    /// the VM byte-for-byte as it was before the debugger existed.
    #[serde(default)]
    pub debug: Option<DebugServerConfig>,
}

/// Every field has a default so an existing `[servers.debug]` block never has to
/// be rewritten when a new key is added.
#[derive(Debug, Deserialize, Clone)]
pub struct DebugServerConfig {
    pub enabled: bool,
    /// Never expose this: `evaluate` runs arbitrary Lua in the game VM.
    #[serde(default = "default_debug_bind")]
    pub bind: String,
    #[serde(default = "default_debug_port")]
    pub port: u16,
    /// Resume the VM if the debug client stops responding while stopped at a
    /// breakpoint, so a crashed editor cannot wedge the server. 0 disables.
    #[serde(default = "default_auto_continue")]
    pub auto_continue_secs: u64,
    #[serde(default = "default_trace_capacity")]
    pub trace_capacity: usize,
    #[serde(default = "default_timing_capacity")]
    pub timing_capacity: usize,
}

fn default_debug_bind() -> String { "127.0.0.1".to_string() }
fn default_debug_port() -> u16 { 4711 }
fn default_auto_continue() -> u64 { 300 }
fn default_trace_capacity() -> usize { 5_000 }
fn default_timing_capacity() -> usize { 200 }

impl Default for DebugServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_debug_bind(),
            port: default_debug_port(),
            auto_continue_secs: default_auto_continue(),
            trace_capacity: default_trace_capacity(),
            timing_capacity: default_timing_capacity(),
        }
    }
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
