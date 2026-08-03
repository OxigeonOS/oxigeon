# Extending Oxigeon

Most extensions do not need Rust at all. Start here:

| You want to… | Reach for |
|---|---|
| Persist a new kind of thing | [Document store](../lua-api/document-store.md) — no Rust |
| Run something expensive without freezing the game | [Compute bridge](../lua-api/compute.md) — no Rust |
| Add a command, room, daemon or area | Lua, in `game/` |
| Query hard: indexed columns, joins, foreign keys | **A Diesel model — read on** |
| Expose a new driver capability to Lua | **A new efun — read on** |

## Adding a persisted model

Do this only when the document store genuinely is not enough — when you need a
column the database can index, a foreign key, or a join. Everything else costs
you a rebuild for nothing.

### 1. The migration

```
migrations/2024-01-05-000001_create_guilds/
├── up.sql
└── down.sql
```

```sql
-- up.sql — SQLite dialect
CREATE TABLE guilds (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL UNIQUE,
    leader_id  INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_guilds_leader ON guilds(leader_id);
```

> [!IMPORTANT]
> `embed_migrations!` reads this directory at **compile time** and bakes the
> SQL into the binary. Editing `migrations/` at runtime does nothing, and the
> `game/` layer can never ship one — which is exactly why the document store
> exists.

### 2. The schema

`src/domain/db/schema.rs` is hand-maintained despite its `@generated` header
(`diesel.toml` points `print_schema` at a path that no longer exists). Add the
block yourself, and **keep the column order identical to the struct** — queries
decode positionally, so a mismatch mis-decodes silently rather than failing to
compile.

```rust
diesel::table! {
    guilds (id) {
        id -> BigInt,
        name -> Text,
        leader_id -> BigInt,
        created_at -> Text,
    }
}

diesel::joinable!(guilds -> characters (leader_id));
```

Add the name to `allow_tables_to_appear_in_same_query!` if it will be joined.

### 3. The model and store

`src/domain/models/guild.rs`, following `character.rs`:

```rust
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = guilds)]
pub struct Guild { /* fields, in schema order */ }

#[derive(Insertable)]
#[diesel(table_name = guilds)]
struct NewGuild<'a> { /* everything except `id` */ }

pub struct DieselGuildStore { pool: AnyPool }

impl DieselGuildStore {
    pub fn new(pool: AnyPool) -> Self { Self { pool } }
    pub fn create(&self, name: &str, leader: i64) -> Result<Guild> { /* … */ }
    pub fn find_by_id(&self, id: i64) -> Result<Option<Guild>> { /* … */ }
}
```

Add `pub mod guild;` and a re-export to `src/domain/models/mod.rs`.

> [!NOTE]
> **Do not add a matching trait to `src/domain/traits.rs`.** The three that
> live there are implemented and never consumed — nothing in the tree uses
> `dyn AccountStore` or takes one as a bound — so they only make every method
> signature get written three times. `DieselDocumentStore` deliberately has no
> trait for the same reason. See [Traits](./traits.md).

### 4. Wiring

Add the store to `EfunContext` (`src/core/scripting/efuns.rs`) and construct it
in `Driver::new` (`src/driver.rs`). There are exactly two places that name
every `EfunContext` field — the driver and `tests/common/mod.rs` — so this
costs two lines.

### 5. The efuns

A sibling file, as `efuns_io.rs`, `efuns_compute.rs` and `efuns_document.rs`
are, then one line in `register_all`. The conventions:

- **Never fail silently.** Expected absence returns `nil`; an author error or
  an infrastructure failure raises. Commands are `pcall`-wrapped, so a raise
  lands in the log rather than killing anything.
- **Gate what needs gating** with `check_efun_permission("name", …)`. It is a
  no-op when the name is absent from `permissions.toml`, so calling it
  unconditionally lets an operator gate the efun later without a code change.
- **`register_all` runs before `apply_sandbox`**, so a new efun needs no
  sandbox change. The sandbox only strips; it never whitelists.

### 6. What else to update

| File | Why |
|---|---|
| `types/oxigeon.lua` | Or the mudlib gets no completion or type checking — and a *wrong* stub is worse than none, which is how `mudstatus` came to print "0s" uptime |
| `docs/src/lua-api/efuns.md` | The efun index |
| `config/permissions.toml` | If it should be gated |
| A test using `RealVm` | See below |

## Testing an extension

Anything reachable from game code gets a test that drives **the VM the engine
actually builds**, via `tests/common/mod.rs`:

```rust
mod common;
use common::RealVm;

#[test]
fn the_new_efun_works_from_lua() {
    let mut vm = RealVm::boot();
    assert_eq!(vm.eval("return guild_create('Smiths', 1).name").unwrap(), "Smiths");
}
```

This is not a style preference. Two security controls once shipped broken
because their tests exercised a helper in isolation while production took a
different path — the sandbox was never applied to the real VM, and the
instruction limit was parsed and never read. Both suites were green throughout.

## Other extension points

- **New efuns** — above, and [Traits](./traits.md).
- **New telnet options** — `src/core/network/telnet/`: add the option constant
  to `constants.rs`, teach the negotiator, handle the subnegotiation in
  `driver.rs`.
- **New GMCP packages** — no driver change needed; handle them in `on_gmcp`
  and send with `send_gmcp`.
- **New network protocols** — a listener module alongside
  `core/network/telnet/`.
- **A new database backend** — `AnyPool` in `src/domain/db/connection.rs` is an
  enum with one working variant today. All existing SQL is SQLite dialect and
  every store calls `get_sqlite()`, so this is a larger job than it looks.
