use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub game: GameConfig,
    pub sessions: SessionsConfig,
    pub accounts: AccountsConfig,
    pub limits: LimitsConfig,
}

#[derive(Debug, Deserialize, Clone)]
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
}

#[derive(Debug, Deserialize, Clone)]
pub struct SessionsConfig {
    pub multisession_mode: MultisessionMode,
    pub max_connections: usize,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MultisessionMode {
    Single,
    SharedCharacter,
    MultiCharacter,
    FullMulti,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AccountsConfig {
    pub allow_creation: bool,
    pub min_password_length: usize,
    pub max_characters_per_account: usize,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LimitsConfig {
    pub lua_memory_mb: usize,
    pub lua_instruction_limit: u64,
    pub input_buffer_bytes: usize,
}
