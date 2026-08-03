-- Generic JSON document store.
--
-- One table serves every collection a game author invents, so shipping a new
-- persisted type needs no Rust and no migration. That matters because
-- `embed_migrations!` bakes this directory into the binary at compile time:
-- the hot-reloadable game/ layer can otherwise never ship schema of its own.

CREATE TABLE documents (
    collection  TEXT NOT NULL,
    id          TEXT NOT NULL,
    data        TEXT NOT NULL DEFAULT '{}',
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (collection, id),

    -- The storage layer's own guarantee. Anything that writes this table, now
    -- or later, gets a hard error rather than a row that json_extract silently
    -- returns NULL for on every subsequent read.
    CHECK (json_valid(data)),
    CHECK (length(collection) BETWEEN 1 AND 64),
    CHECK (length(id) BETWEEN 1 AND 128)
);

-- The primary key already confines every scan to one collection. This index is
-- for the ordered case — "the oldest 20 documents in this collection" — which
-- is what a report queue, a mail spool and a moderation list all ask for, and
-- which is db_find()'s default sort.
CREATE INDEX idx_documents_collection_created
    ON documents(collection, created_at);
