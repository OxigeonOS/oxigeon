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
    /// Whether hitting a breakpoint holds the whole VM, or only the dispatch
    /// that hit it.
    ///
    /// Default true, which is what every debugger does and what LuaJIT can only
    /// do. On a Lua 5.5 build, false suspends one command — or one tick — and
    /// lets every other player carry on, which is the point of that runtime and
    /// the only way to debug a server with people on it. Changeable while
    /// running with `trace freeze on|off`.
    #[serde(default = "default_stop_the_world")]
    pub stop_the_world: bool,
    #[serde(default = "default_trace_capacity")]
    pub trace_capacity: usize,
    #[serde(default = "default_timing_capacity")]
    pub timing_capacity: usize,
}

fn default_debug_bind() -> String { "127.0.0.1".to_string() }
fn default_debug_port() -> u16 { 4711 }
fn default_auto_continue() -> u64 { 300 }
fn default_stop_the_world() -> bool { true }
fn default_trace_capacity() -> usize { 5_000 }
fn default_timing_capacity() -> usize { 200 }

impl Default for DebugServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_debug_bind(),
            port: default_debug_port(),
            auto_continue_secs: default_auto_continue(),
            stop_the_world: default_stop_the_world(),
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
