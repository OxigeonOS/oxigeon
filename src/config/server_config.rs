use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerConfig {
    pub game: GameConfig,
    pub sessions: SessionsConfig,
    pub accounts: AccountsConfig,
    pub limits: LimitsConfig,
    /// Off unless a `[compute]` section says otherwise, so every existing
    /// `server.toml` keeps parsing and a feature nobody uses costs nothing.
    #[serde(default)]
    pub compute: ComputeConfig,
    /// Ceilings on the generic document store. See [`crate::domain::models::document`].
    #[serde(default)]
    pub documents: crate::domain::models::document::DocumentLimits,
}

/// Settings that have a default the driver applies when the key is absent.
///
/// `config()` is a *generic* reader — it serialises this struct and walks a
/// dotted path — so without this table an unset optional key would answer `nil`
/// and every caller would have to repeat the default. Repeating a default is
/// how two parts of a system come to disagree about what it is.
///
/// Only keys whose absence has a meaning other than "off" belong here. A key
/// the driver genuinely has no default for stays absent, and `nil` is the
/// honest answer.
pub const CONFIG_DEFAULTS: &[(&str, i64)] = &[
    ("game.area_reset_seconds", 900),
    ("game.autosave_seconds", 300),
    ("game.cache_flush_seconds", 5),
    ("game.cache_flush_budget", 32),
    ("game.cache_evict_seconds", 900),
    ("game.cooldown_durable_seconds", 60),
    ("game.effect_sweep_seconds", 5),
    ("game.effect_heartbeat_seconds", 3),
    ("game.combat_round_seconds", 3),
    ("game.shutdown_timeout_seconds", 30),
];

impl ServerConfig {
    /// The whole configuration as JSON, for the generic `config()` efun.
    ///
    /// Built once at VM construction and walked per lookup. Serialising here
    /// rather than naming each key in a `match` is the difference between
    /// adding a config key and adding a config key *plus a Rust edit* — the
    /// eighteen-key allowlist this replaced meant every new game-layer setting
    /// needed one.
    pub fn as_lookup_json(&self) -> serde_json::Value {
        let mut root = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        for (path, default) in CONFIG_DEFAULTS {
            let (section, key) = match path.split_once('.') {
                Some(pair) => pair,
                None => continue,
            };
            let slot = root
                .get_mut(section)
                .and_then(|s| s.as_object_mut())
                .map(|obj| obj.entry(key.to_string()).or_insert(serde_json::Value::Null));
            if let Some(slot) = slot {
                if slot.is_null() {
                    *slot = serde_json::Value::from(*default);
                }
            }
        }
        root
    }
}

/// The worker pool that runs Lua off the game thread.
/// See [`crate::core::compute`].
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ComputeConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Worker threads, each with its own LuaJIT VM, built lazily on first use.
    #[serde(default = "default_compute_workers")]
    pub workers: usize,
    /// Jobs allowed to wait before new ones are refused. Shallow on purpose: a
    /// compute job is expected to take a while, so a deep queue only produces
    /// answers that arrive long after they stopped mattering.
    #[serde(default = "default_compute_queue_depth")]
    pub queue_depth: usize,
    /// Wall clock a job gets before the *caller* is told it timed out. The
    /// worker may still be running after this — see the module docs.
    #[serde(default = "default_compute_deadline_ms")]
    pub default_deadline_ms: u64,
    #[serde(default = "default_compute_max_deadline_ms")]
    pub max_deadline_ms: u64,
    /// Instructions one job may execute, enforced the same way (and with the
    /// same JIT cost) as `limits.lua_instruction_limit`. 0 keeps the compiler
    /// and accepts that a runaway job burns a worker for the life of the
    /// process.
    #[serde(default)]
    pub instruction_limit: u64,
    /// Memory ceiling for each worker VM, in megabytes. 0 = no ceiling.
    #[serde(default = "default_compute_memory_mb")]
    pub memory_mb: usize,
    /// Module prefixes an entry point may live under, relative to the game and
    /// mudlib roots. A guardrail, not a security boundary — the boundary is
    /// that the compute VM has no efuns at all.
    #[serde(default = "default_compute_roots")]
    pub roots: Vec<String>,
    /// Ceilings on copying a value between VMs. A table deeper or larger than
    /// this is refused at the call site.
    #[serde(default = "default_compute_max_depth")]
    pub max_arg_depth: usize,
    #[serde(default = "default_compute_max_nodes")]
    pub max_arg_nodes: usize,
}

fn default_compute_workers() -> usize { 2 }
fn default_compute_queue_depth() -> usize { 16 }
fn default_compute_deadline_ms() -> u64 { 5_000 }
fn default_compute_max_deadline_ms() -> u64 { 60_000 }
fn default_compute_memory_mb() -> usize { 256 }
fn default_compute_roots() -> Vec<String> { vec!["compute".to_string()] }
fn default_compute_max_depth() -> usize { 64 }
fn default_compute_max_nodes() -> usize { 100_000 }

impl Default for ComputeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            workers: default_compute_workers(),
            queue_depth: default_compute_queue_depth(),
            default_deadline_ms: default_compute_deadline_ms(),
            max_deadline_ms: default_compute_max_deadline_ms(),
            instruction_limit: 0,
            memory_mb: default_compute_memory_mb(),
            roots: default_compute_roots(),
            max_arg_depth: default_compute_max_depth(),
            max_arg_nodes: default_compute_max_nodes(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GameConfig {
    pub name: String,
    pub mudlib_path: String,
    /// Path to the game-specific layer (rooms, game commands, areas).
    /// Defaults to "./game" if not set.
    pub game_path: Option<String>,
    /// Ordered list of subdirectory names to search for commands.
    /// e.g. ["cmds"] searches cmds/ in both game/ and mudlib/ roots.
    /// Defaults to ["cmds"] if not set.
    pub command_paths: Option<Vec<String>>,
    /// Room ID where new characters spawn. e.g. "wizard_workshop.entrance"
    pub start_room: Option<String>,
    /// How often areas reset in seconds. 0 = disabled. Default: 900 (15 minutes).
    pub area_reset_seconds: Option<u64>,
    /// How often player data is auto-saved in seconds. 0 = disabled. Default: 300 (5 minutes).
    pub autosave_seconds: Option<u64>,
    /// How often the write-behind cache considers flushing dirty scopes, in
    /// seconds. This is the scheduler's granularity, not the flush interval —
    /// each namespace declares its own. 0 = no flush ticker (tests). Default: 5.
    pub cache_flush_seconds: Option<u64>,
    /// How many scopes one flush tick may write before deferring the rest to
    /// the next tick. Bounds the hitch when many scopes go dirty at once.
    /// Default: 32 (~3.2 ms of game thread).
    pub cache_flush_budget: Option<u64>,
    /// How long an unowned, clean cache scope may sit untouched before it is
    /// dropped from memory. 0 = never evict on idle. Default: 900.
    pub cache_evict_seconds: Option<u64>,
    /// A cooldown at least this long is stored durably; anything shorter lives
    /// in memory and is forgotten on restart. Default: 60.
    pub cooldown_durable_seconds: Option<u64>,
    /// How often expired effects are swept so `on_expire` fires for a player
    /// who is typing nothing. 0 = no sweep. Default: 5.
    pub effect_sweep_seconds: Option<u64>,
    /// The interval driving effects that tick (regeneration, damage over time).
    /// 0 = no heartbeat. Default: 3.
    pub effect_heartbeat_seconds: Option<u64>,
    /// Seconds per combat round. 0 = no combat ticker. Default: 3.
    pub combat_round_seconds: Option<u64>,
    /// How long a clean shutdown waits for the mudlib's `on_shutdown` to
    /// finish saving, in seconds. Default: 30.
    ///
    /// The wait exists so a restart does not race the flush to disk; the bound
    /// exists so a mudlib that wedges in `on_shutdown` cannot leave the process
    /// hanging. When it expires the server logs an error and exits anyway.
    pub shutdown_timeout_seconds: Option<u64>,

    /// Everything else under `[game]`.
    ///
    /// The game layer needs settings the driver has no opinion about — where
    /// the dead respawn, how long a shop takes to restock, which area the
    /// builders own. Without this each one is a Rust edit before Lua can read
    /// it, and `death_d` hardcoding `wizard_workshop.entrance` in the *mudlib*
    /// layer is exactly what that pressure produces.
    ///
    /// Flattened, so an unrecognised key is captured rather than rejected, and
    /// readable from Lua as `config("game.<key>")` like any other.
    #[serde(flatten, default)]
    pub extra: std::collections::HashMap<String, toml::Value>,
}

impl GameConfig {
    /// How long to wait for `on_shutdown`, with the default applied.
    pub fn shutdown_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.shutdown_timeout_seconds.unwrap_or(30))
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SessionsConfig {
    pub multisession_mode: MultisessionMode,
    pub max_connections: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MultisessionMode {
    Single,
    SharedCharacter,
    MultiCharacter,
    FullMulti,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AccountsConfig {
    pub allow_creation: bool,
    pub min_password_length: usize,
    pub max_characters_per_account: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LimitsConfig {
    pub lua_memory_mb: usize,
    pub lua_instruction_limit: u64,
    pub input_buffer_bytes: usize,
}
