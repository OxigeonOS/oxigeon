# Swappable Components (Traits)

Oxigeon's domain-layer subsystems are defined as Rust traits so the backing implementation
can be replaced without changing any driver code. The default implementations use Diesel + SQLite.

> [!WARNING]
> **These traits are currently aspirational.** All three are implemented, and
> none is consumed: nothing in the tree uses `dyn AccountStore`, or takes one
> as a generic bound. `EfunContext` holds `Arc<DieselAccountStore>`, not
> `Arc<dyn AccountStore>`.
>
> The practical consequence is that adding a method means writing its signature
> three times — inherent impl, trait declaration, forwarding impl — for a
> swappability the code does not actually offer. **New stores should not add a
> trait**; `DieselDocumentStore` deliberately has none. Either make the traits
> load-bearing or delete them, but do not grow the tax in the meantime.

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
    fn set_admin(&self, id: i64, is_admin: bool) -> Result<()>;
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
    /// The character's JSON `data` column — see Character Data & Persistence.
    fn save_data(&self, id: i64, data: &str) -> Result<()>;
    fn load_data(&self, id: i64) -> Result<Option<String>>;
    fn delete(&self, id: i64) -> Result<()>;
}
```

The default `DieselCharacterStore` enforces the `max_characters_per_account` limit
set in `server.toml` and enforces globally unique character names.

---

## `RoleStore`

Defined in `src/domain/traits.rs`. Implemented by `DieselRoleStore`. Backs the RBAC
system described in [Permissions & Roles](../lua-api/permissions.md) — roles, the
permissions attached to them, and the characters they are granted to.

```rust
pub trait RoleStore: Send + Sync {
    fn create_role(&self, name: &str) -> Result<Role>;
    fn find_role_by_name(&self, name: &str) -> Result<Option<Role>>;
    fn list_roles(&self) -> Result<Vec<Role>>;
    fn delete_role(&self, id: i64) -> Result<()>;

    fn grant_permission(&self, role_id: i64, permission: &str) -> Result<()>;
    fn revoke_permission(&self, role_id: i64, permission: &str) -> Result<()>;
    fn get_permissions_for_role(&self, role_id: i64) -> Result<Vec<String>>;

    fn assign_role(&self, character_id: i64, role_id: i64) -> Result<()>;
    fn unassign_role(&self, character_id: i64, role_id: i64) -> Result<()>;
    fn get_roles_for_character(&self, character_id: i64) -> Result<Vec<Role>>;
    fn get_permissions_for_character(&self, character_id: i64) -> Result<Vec<String>>;
}
```

`get_permissions_for_character` is the one the driver actually calls on a hot path:
`enter_game_session` reads it once and caches the result on the session, so a permission
check during play is an in-memory set lookup rather than a query.

> [!NOTE]
> The `**superuser**` sentinel is not a row in this table. It is inserted into a session's
> cached permission set when `accounts.is_admin` is true, and it short-circuits every check.

---

## The document store has no trait

`DieselDocumentStore` (see [Document Store](../lua-api/document-store.md)) deliberately has
no trait mirror. It is the most backend-specific store of the set — its query builder is
SQLite JSON1 throughout — and a fourth copy of the signature tax would buy nothing while
there is no second backend. This is a decision, not an oversight.

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

`EfunContext` holds the stores as `Arc<DieselAccountStore>` and friends, so swapping one
means changing that field's type too — the trait alone does not buy it. See the warning at
the top of this page.

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
