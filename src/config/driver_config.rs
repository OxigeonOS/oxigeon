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
    /// WebSocket listener. Absent (the default) is a telnet-only server,
    /// byte-for-byte as before.
    #[serde(default)]
    pub websocket: Option<WebSocketServerConfig>,
    /// `telnets://` — the same telnet protocol inside TLS, on its own port.
    ///
    /// A *second* listener rather than a flag on the first, because you almost
    /// always want both: existing players keep their plaintext port while
    /// clients that can negotiate TLS use the secure one.
    #[serde(default)]
    pub telnet_tls: Option<TelnetServerConfig>,
    /// `wss://`. A second listener for the same reason as `telnet_tls`.
    #[serde(default)]
    pub websocket_tls: Option<WebSocketServerConfig>,
}

/// A certificate and its key, as named by a listener that wants TLS.
///
/// Both or neither. One alone is a misconfiguration that would otherwise
/// present as a port that quietly serves plaintext under a secure-sounding
/// name, so it is refused at startup.
pub fn tls_files(
    cert_path: &Option<String>,
    key_path: &Option<String>,
    what: &str,
) -> std::result::Result<Option<(String, String)>, String> {
    match (cert_path, key_path) {
        (None, None) => Ok(None),
        (Some(c), Some(k)) => Ok(Some((c.clone(), k.clone()))),
        (Some(_), None) => Err(format!("[servers.{what}] has cert_path but no key_path")),
        (None, Some(_)) => Err(format!("[servers.{what}] has key_path but no cert_path")),
    }
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
    /// PEM certificate chain. Set on a `[servers.telnet_tls]` block to serve
    /// `telnets://`; absent on the plaintext listener.
    #[serde(default)]
    pub cert_path: Option<String>,
    #[serde(default)]
    pub key_path: Option<String>,
    /// How often to check whether the certificate has been renewed. 0 reads it
    /// once at startup. See `network::tls::ReloadingCert`.
    #[serde(default = "default_cert_reload")]
    pub cert_reload_seconds: u64,
}

/// Five minutes. A certificate changes every few months, so this only has to
/// be short enough that a renewal is picked up the same day — and two `stat`
/// calls at that interval cost nothing worth measuring.
pub(crate) fn default_cert_reload() -> u64 { 300 }

/// Every field but `enabled` has a default, so an existing
/// `[servers.websocket]` block never has to be rewritten when a new key is
/// added — the same rule as `[servers.debug]`.
#[derive(Debug, Deserialize, Clone)]
pub struct WebSocketServerConfig {
    pub enabled: bool,
    /// Loopback by default. On the plaintext `[servers.websocket]` block that
    /// is a safety default — anything reachable off-host should be `wss://` or
    /// behind a proxy. A `[servers.websocket_tls]` block will normally set
    /// `0.0.0.0`.
    #[serde(default = "default_ws_bind")]
    pub bind: String,
    #[serde(default = "default_ws_port")]
    pub port: u16,
    /// Largest client message accepted. A WebSocket message arrives whole, so
    /// without a cap one client decides how much the server allocates.
    #[serde(default = "default_ws_max_frame")]
    pub max_frame_bytes: usize,
    /// Seconds between server keepalive pings. 0 disables. A suspended browser
    /// tab and a NAT that has silently dropped the flow look identical to an
    /// idle player until something asks.
    #[serde(default = "default_ws_ping_secs")]
    pub ping_interval_secs: u64,
    /// Consecutive unanswered pings tolerated before the connection is closed.
    /// Separate from the interval because a backgrounded tab can stall pongs
    /// for far longer than one period, and evicting someone who alt-tabbed is
    /// worse than holding a dead socket for another minute.
    #[serde(default = "default_ws_missed_pongs")]
    pub missed_pongs: u32,
    /// PEM certificate chain. Set on a `[servers.websocket_tls]` block to serve
    /// `wss://`; absent on the plaintext listener.
    #[serde(default)]
    pub cert_path: Option<String>,
    #[serde(default)]
    pub key_path: Option<String>,
    /// How often to check whether the certificate has been renewed. 0 reads it
    /// once at startup.
    #[serde(default = "default_cert_reload")]
    pub cert_reload_seconds: u64,
    /// Browser origins permitted to open a socket. Empty (the default) accepts
    /// any. See `websocket::connection` for what an absent `Origin` means.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

fn default_ws_bind() -> String { "127.0.0.1".to_string() }
fn default_ws_port() -> u16 { 4001 }
fn default_ws_max_frame() -> usize { 64 * 1024 }
fn default_ws_ping_secs() -> u64 { 30 }
fn default_ws_missed_pongs() -> u32 { 3 }

impl Default for WebSocketServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_ws_bind(),
            port: default_ws_port(),
            max_frame_bytes: default_ws_max_frame(),
            ping_interval_secs: default_ws_ping_secs(),
            missed_pongs: default_ws_missed_pongs(),
            cert_path: None,
            key_path: None,
            cert_reload_seconds: default_cert_reload(),
            allowed_origins: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct LoggingConfig {
    pub level: String,
    pub file: Option<String>,
}
