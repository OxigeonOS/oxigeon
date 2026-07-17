# Swappable Components (Traits)

Oxigeon's domain-layer subsystems are defined as Rust traits so the backing implementation
can be replaced without changing any driver code. The default implementations use Diesel + SQLite.

---

## `AccountStore`

Defined in `src/domain/traits.rs`. Implemented by `DieselAccountStore`.

```rust
pub trait AccountStore: Send + Sync {
    fn create(&self, username: &str, password: &str) -> Result<Account>;
    fn authenticate(&self, username: &str, password: &str) -> Result<Account>;
    fn find_by_id(&self, id: i64) -> Result<Option<Account>>;
    fn find_by_name(&self, name: &str) -> Result<Option<Account>>;
    fn update_password(&self, id: i64, new_password: &str) -> Result<()>;
    fn delete(&self, id: i64) -> Result<()>;
}
```

The default `DieselAccountStore` hashes passwords with **Argon2id** and enforces a minimum
password length set in `server.toml`.

---

## `CharacterStore`

Defined in `src/domain/traits.rs`. Implemented by `DieselCharacterStore`.

```rust
pub trait CharacterStore: Send + Sync {
    fn create(&self, account_id: i64, name: &str) -> Result<Character>;
    fn find_by_id(&self, id: i64) -> Result<Option<Character>>;
    fn find_by_account(&self, account_id: i64) -> Result<Vec<Character>>;
    fn find_by_name(&self, name: &str) -> Result<Option<Character>>;
    fn delete(&self, id: i64) -> Result<()>;
}
```

The default `DieselCharacterStore` enforces the `max_characters_per_account` limit
set in `server.toml` and enforces globally unique character names.

---

## Swapping Implementations

To replace the default SQLite-backed stores with a custom implementation:

1. Create your struct in `src/domain/` (or a new crate)
2. `impl AccountStore for MyStore { ... }`
3. In `src/driver.rs`, replace the `Arc::new(DieselAccountStore::new(...))` lines
   with `Arc::new(MyStore::new(...))`

```rust
// src/driver.rs — replace:
let account_store = Arc::new(DieselAccountStore::new(db_pool.clone(), min_pw_len));
// with:
let account_store = Arc::new(MyRedisAccountStore::new(redis_client.clone()));
```

The `EfunContext` holds the stores as `Arc<DieselAccountStore>` currently — a future refactor
will change these to `Arc<dyn AccountStore>` to make trait-object dispatch automatic.

---

## Design Principles

- **`Arc<T>` sharing** — Stores are shared between the async driver and the Lua efun context via `Arc`.
- **`Send + Sync`** — Required: the Lua thread accesses them via efun closures.
- **Liskov substitution** — Any implementation should be a transparent drop-in.

---

## Adding a New Database Backend

Currently `AnyPool` in `src/domain/db/connection.rs` supports SQLite (and stubs PostgreSQL).
To add a new backend:

1. Add the Diesel feature to `Cargo.toml` (e.g. `features = ["postgres"]`)
2. Add a new variant to the `AnyPool` enum
3. Implement `get_<backend>()` and route it in all match arms
4. Add migration support for the new backend in `src/driver.rs`

## Adding a New Network Protocol

To add WebSocket support:
1. Create `src/core/network/websocket/mod.rs` with a `WebSocketListener` struct
2. Implement the same accept/connection pattern as `TelnetListener`
3. Sessions created from WebSocket connections use the same `Session` type — protocol is noted in `session.protocol`
4. Add a `[servers.websocket]` section to `driver.toml`

## Adding New Efuns

To expose a new Rust function to Lua:

```rust
// In src/core/scripting/efuns.rs, add to an appropriate register_*_efuns() function:
let my_efun = lua.create_function(|_, (arg1, arg2): (String, i64)| {
    // Your implementation here
    Ok(result)
})?;
globals.set("my_efun", my_efun)?;
```

Then document it in `docs/src/lua-api/efuns.md`.
