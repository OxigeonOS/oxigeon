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
