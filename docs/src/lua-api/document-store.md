# Document Store — Persisting Anything

Want to save a thing? `db_put` it.

```lua
db_put("reports", "R0001", {
    status   = "open",
    reporter = player.name,
    summary  = "the cauldron is stuck",
    filed_at = os_time(),
})

for _, rec in ipairs(db_find("reports", { status = "open" }, { limit = 20 })) do
    player:send(rec.id .. "  " .. rec.data.summary)
end
```

No migration, no `schema.rs` edit, no model file, no store, no `EfunContext`
field, no rebuild. That is the point.

> [!IMPORTANT]
> **Do not write here on every change.** A `db_put` costs about 84 microseconds
> against 0.8 for an in-memory write, synchronously on the game thread.
> Persisting every stat change, every combat hit and every mob swing is the
> mistake this store makes easy and the [state cache](./state-cache.md) exists
> to prevent.
>
> Choose the tier by how much you would mind losing it, not by convenience.
> Combat state and short buffs should never be written at all; character
> progression belongs behind a write-behind namespace; a once-a-day gate can
> come straight here.

## Why this exists

Adding one persisted type the Rust way takes roughly fourteen steps across nine
files — a migration, a hand-edited `schema.rs` table block, a model with three
derives, a store, module re-exports, an `EfunContext` field, an efun
registration function and driver wiring.

Worse, `embed_migrations!` bakes the migration directory into the binary at
**compile time**. The `game/` layer is hot-reloadable Lua; the schema is
compile-time-frozen Rust. Without this store, a game author simply cannot ship
a table.

> [!NOTE]
> Reach for a real Diesel model when you need indexed columns, joins, or
> foreign keys — see [Extending Oxigeon](../rust-api/extending.md). Use the
> document store for everything else.

## Records

Reads return an **envelope**, not the bare document:

| Field | Type | Description |
|-------|------|-------------|
| `collection` | string | the collection it lives in |
| `id` | string | its id |
| `data` | table | what you stored |
| `created_at` | string | RFC 3339, set on first write |
| `updated_at` | string | RFC 3339, moves on every write |

So it is `rec.data.status`, not `rec.status`. The extra `.data` buys a stable
id round-trip, useful timestamps, and — importantly — no reserved key names
inside your own document.

Collection names are lowercase, digits and underscores. Ids allow letters,
digits and `. _ - :`.

## Reference

### `db_put(collection, id, doc) → id`
Insert or replace under an id you choose. `created_at` is preserved on
overwrite.

### `db_insert(collection, doc) → id`
The same, with a generated id.

### `db_get(collection, id) → record|nil`

### `db_exists(collection, id) → boolean`
Avoids deserializing a large document to answer a yes/no.

### `db_delete(collection, id) → boolean`
Whether a document was removed.

### `db_find(collection, filter?, opts?) → array of records`
See the filter language below. Always returns an array, never `nil`.

### `db_count(collection, filter?) → integer`
Avoids materializing 500 records to print a number.

### `db_update(collection, id, patch) → boolean`
Recursive merge (RFC 7396): objects merge key by key, **arrays are replaced
wholesale**. One atomic statement, so the common read-modify-write needs no
transaction.

```lua
db_update("reports", "R0001", { status = "closed", resolved_by = player.name })
-- every other field is untouched
```

### `db_unset(collection, id, field) → boolean`
Remove one field, including a nested one (`"target.area"`). Exists because Lua
tables cannot hold `nil`, so RFC 7396's delete-by-null is unreachable from Lua.

### `db_incr(collection, id, field, delta?) → number`
Atomic increment; `delta` defaults to 1. Creates the document if it is missing,
so a counter needs no bootstrap:

```lua
local n  = db_incr("counters", "reports", "next")
local id = string.format("R%04d", n)
```

### `db_collections() → array of {name, count}`

### `db_clear(collection) → integer`
Delete a whole collection. Gated by `efun.db.clear` in `permissions.toml`.

## The filter language

Keys are dotted JSON paths. A bare value means equality; a table means one
operator.

```lua
db_find("reports", {
    status           = "open",              -- equality
    priority         = { [">="] = 3 },      -- comparison
    ["target.area"]  = "workshop",          -- nested path
    tags             = { contains = "urgent" },
    reporter         = { ["in"] = { "amy", "bo" } },
    resolved_by      = { exists = false },
}, { sort = "priority", order = "desc", limit = 10 })
```

| Operator | Meaning |
|---|---|
| *(bare value)* or `==` | equal |
| `~=` | not equal |
| `>` `>=` `<` `<=` | comparison |
| `in` / `nin` | in / not in a list (max 64 values) |
| `like` | SQL `LIKE`; `%` is the wildcard, and matching is ASCII case-**insensitive** |
| `exists` | `true` or `false` — whether the field is present at all |
| `contains` | the field is an array containing this value |

Multiple keys are combined with **AND**. There is no `OR`: it needs a real
expression tree and nothing in a MUD needs it that two `db_find` calls and a
merge will not do.

`opts` takes `limit`, `offset`, `sort` (`"id"`, `"created_at"`, `"updated_at"`,
or any dotted path) and `order` (`"asc"` or `"desc"`).

> [!IMPORTANT]
> **`~=` and `nin` also match documents where the field is missing entirely.**
>
> A Lua author writing `doc.status ~= "closed"` against a document with no
> `status` expects that to be true. Plain SQL `<>` against NULL matches
> nothing, which would be a silent wrong answer — so this deliberately
> diverges from SQL.

> [!WARNING]
> **SQLite compares across storage classes, so a number is not a string.**
>
> ```lua
> db_put("mobs", "m1", { level = 5 })
> db_find("mobs", { level = "5" })   --> {} — TEXT never equals INTEGER
> ```
>
> The driver cannot detect this without knowing every document's stored type.
> Filter with the same type you stored.

## Limits and failure

Every ceiling in `[documents]` is a **hard error**. Nothing is ever silently
truncated.

| Setting | Default | Applies to |
|---|---|---|
| `max_bytes` | 65536 | one document, serialized |
| `max_per_collection` | 100000 | documents in one collection |
| `max_collections` | 256 | distinct collections |
| `max_results` | 500 | rows one `db_find` may return |

The subtle one: **a `db_find` with no explicit `limit` that matches more than
`max_results` errors** rather than returning the first 500. A report list
quietly missing its oldest entries, but looking complete, is precisely the
failure this project forbids. Pass `{ limit = n }` and paginate with
`{ offset = n }`.

Author errors — a bad field name, an unknown operator, an oversize document,
incrementing a field that holds a string — **raise**, naming the offender.
Commands are `pcall`-wrapped, so a raise lands in the log rather than killing
anything. Expected absence (`db_get` on an unknown id) returns `nil`.

## What it will not do

**No transactions.** A Lua callback running inside a Rust-held transaction
would pin a pooled connection across arbitrary game code, and any nested `db_*`
call would deadlock — instantly and always with a small pool. `db_update` and
`db_incr` are single atomic statements precisely so the common cases do not
need one.

**Not a replacement for `CHARACTER_D`.** Character data still goes through
`save_character_data`; it is per-character, already indexed by the characters
table, and already handled.

**Not a replacement for `set_object_state`.** That is deliberately ephemeral —
`world_d` clears it on every area reset — and is the right home for state that
should reset. Use the document store when you want state to *survive*.

> [!NOTE]
> Gating a `db_*` efun in `permissions.toml` does not stop a ticker callback.
> Engine-internal dispatch runs with the driver's own authority, so a daemon
> can call `db_clear` on a tick whatever the config says. That is already true
> of `write_file`.
