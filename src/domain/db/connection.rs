use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager, Pool as R2D2Pool};
use crate::config::driver_config::{DatabaseConfig, DatabaseBackend};
use crate::error::{OxigeonError, Result};

pub type SqlitePool = R2D2Pool<ConnectionManager<SqliteConnection>>;

/// Pragmas applied to every new SQLite connection.
///
/// - `foreign_keys` — off by default in SQLite, so without it `ON DELETE
///   CASCADE` silently does nothing.
/// - `journal_mode = WAL` and `synchronous = NORMAL` — the defaults
///   (rollback journal, `synchronous = FULL`) mean **one fsync per write on
///   the Lua thread**, which is the thread every connected player is waiting
///   on. Every character autosave and every `db_put` paid it.
/// - `busy_timeout` — with WAL a reader and a writer can overlap; this waits
///   rather than failing instantly on the rare collision.
///
/// The trade for `NORMAL`: the last few transactions can be lost on an *OS*
/// crash or power loss. A process crash is still safe. For a MUD that is the
/// right side of the trade.
#[derive(Debug)]
struct SqlitePragmaCustomizer;

impl r2d2::CustomizeConnection<SqliteConnection, diesel::r2d2::Error> for SqlitePragmaCustomizer {
    fn on_acquire(&self, conn: &mut SqliteConnection) -> std::result::Result<(), diesel::r2d2::Error> {
        for pragma in [
            "PRAGMA foreign_keys = ON",
            "PRAGMA journal_mode = WAL",
            "PRAGMA synchronous = NORMAL",
            "PRAGMA busy_timeout = 5000",
        ] {
            diesel::sql_query(pragma)
                .execute(conn)
                .map_err(diesel::r2d2::Error::QueryError)?;
        }
        Ok(())
    }
}

/// Get a SQLite connection pool.
pub fn establish_sqlite_pool(config: &DatabaseConfig) -> Result<SqlitePool> {
    let manager = ConnectionManager::<SqliteConnection>::new(&config.url);
    R2D2Pool::builder()
        .max_size(config.pool_size)
        .min_idle(Some(0))  // Don't eagerly create connections
        .connection_timeout(std::time::Duration::from_secs(120))  // Allow slow test ops
        .connection_customizer(Box::new(SqlitePragmaCustomizer))
        .build(manager)
        .map_err(|e| OxigeonError::Internal(format!("Pool build error: {}", e)))
}

/// Wrapped pool type that supports either backend.
#[derive(Clone)]
pub enum AnyPool {
    Sqlite(SqlitePool),
}

impl AnyPool {
    pub fn new(config: &DatabaseConfig) -> Result<Self> {
        match config.backend {
            DatabaseBackend::Sqlite => {
                Ok(AnyPool::Sqlite(establish_sqlite_pool(config)?))
            }
            DatabaseBackend::Postgresql => {
                // PostgreSQL support requires pg client libraries.
                // For now, return an error with a helpful message.
                // Full PG support can be added once libpq is available.
                Err(OxigeonError::Config(
                    "PostgreSQL backend requires libpq. Set backend = \"sqlite\" for now.".into()
                ))
            }
        }
    }

    pub fn sqlite(&self) -> Option<&SqlitePool> {
        match self {
            AnyPool::Sqlite(p) => Some(p),
        }
    }

    pub fn get_sqlite(&self) -> Result<r2d2::PooledConnection<ConnectionManager<SqliteConnection>>> {
        match self {
            AnyPool::Sqlite(pool) => pool.get()
                .map_err(|e| OxigeonError::Pool(e)),
        }
    }
}
