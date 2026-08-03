# Models — Account & Character

See `src/domain/models/` for the Diesel ORM model definitions.

> [!TIP]
> Most things that need persisting do **not** need a model here. The
> [document store](../lua-api/document-store.md) handles arbitrary JSON from
> Lua with no Rust, no migration and no rebuild. Add a model when you need
> indexed columns, joins or foreign keys.

## Account

The `Account` struct maps to the `accounts` table:

| Column | Type | Description |
|--------|------|-------------|
| `id` | `BigInt` | Primary key |
| `username` | `Text` | Unique username |
| `password_hash` | `Text` | Argon2 hash |
| `created_at` | `Text` | ISO 8601 timestamp |
| `last_login` | `Text?` | Last login timestamp |
| `is_admin` | `Bool` | Admin flag |

## Character

The `Character` struct maps to the `characters` table:

| Column | Type | Description |
|--------|------|-------------|
| `id` | `BigInt` | Primary key |
| `account_id` | `BigInt` | Foreign key to accounts |
| `name` | `Text` | Unique character name |
| `created_at` | `Text` | ISO 8601 timestamp |
| `last_played` | `Text?` | Last played timestamp |

## Adding Columns

1. Create a new migration in `migrations/`
2. Update `src/domain/db/schema.rs`
3. Add the field to the Rust struct in `src/domain/models/`
