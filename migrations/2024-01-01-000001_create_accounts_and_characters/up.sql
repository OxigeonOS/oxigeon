-- SQLite-compatible migration
CREATE TABLE accounts (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    username     TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    last_login   TEXT,
    is_admin     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE characters (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id   INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name         TEXT NOT NULL UNIQUE,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    last_played  TEXT
);

CREATE INDEX idx_characters_account_id ON characters(account_id);
