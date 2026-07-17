CREATE TABLE roles (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE permissions (
    id   INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE role_permissions (
    role_id       INTEGER NOT NULL REFERENCES roles(id)       ON DELETE CASCADE,
    permission_id INTEGER NOT NULL REFERENCES permissions(id) ON DELETE CASCADE,
    PRIMARY KEY (role_id, permission_id)
);

CREATE TABLE character_roles (
    character_id INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    role_id      INTEGER NOT NULL REFERENCES roles(id)      ON DELETE CASCADE,
    PRIMARY KEY (character_id, role_id)
);
