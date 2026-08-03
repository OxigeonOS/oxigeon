# Rust API Reference

This section documents the Rust-level APIs for driver developers extending Oxigeon.

## Components

- **[Extending Oxigeon](./extending.md)** — Start here: what needs Rust, what does not, and the full recipe for adding a persisted model or an efun
- **[Traits — Swappable Components](./traits.md)** — `AccountStore`, `CharacterStore`, `RoleStore`, and why new stores should not add one
- **[Models — Account & Character](./models.md)** — Diesel ORM model details

> [!NOTE]
> Most MUD creators will never need to touch the Rust API — everything is configurable from Lua. This section is for those who want to extend the driver itself.
>
> In particular, **persisting a new kind of thing does not need Rust**: see the
> [Document Store](../lua-api/document-store.md). Nor does moving expensive work
> off the game thread: see the [Compute Bridge](../lua-api/compute.md).
