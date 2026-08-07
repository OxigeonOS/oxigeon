# Summary

[Introduction](./introduction.md)

---

# For Mudlib Creators (Lua)

- [Getting Started](./getting-started.md)
- [Architecture Overview](./architecture.md)
- [oxigeon-tui — The Development Cockpit](./tui.md)
- [Lua API Reference](./lua-api/README.md)
  - [Daemons — Service Layer](./lua-api/daemons.md)
  - [Object Hierarchy](./lua-api/object-hierarchy.md)
  - [Components — Roles an Item Can Play](./lua-api/components.md)
  - [World Building — Rooms & Areas](./lua-api/world-building.md)
  - [Signals & Events (EVENT_D)](./lua-api/signals.md)
  - [Character Data & Persistence](./lua-api/character-data.md)
  - [Document Store — Persisting Anything](./lua-api/document-store.md)
  - [State Cache — Tiering by What You'd Mind Losing](./lua-api/state-cache.md)
  - [Items, Equipment & Containers](./lua-api/items.md)
  - [Shops & the Economy](./lua-api/shops.md)
  - [Traits — Any Numeric Data](./lua-api/traits.md)
  - [Effects — Buffs & the Event Pipeline](./lua-api/effects.md)
  - [Abilities — Gameplay as Data](./lua-api/abilities.md)
  - [Action Queues — Roundtime](./lua-api/queues.md)
  - [Messages — One Line, Three Readers](./lua-api/messages.md)
  - [Creatures & Combat](./lua-api/combat.md)
  - [Body Layouts — Where a Blow Lands](./lua-api/bodies.md)
  - [Spawners — Places That Produce Creatures](./lua-api/spawners.md)
  - [Efuns — Driver Functions](./lua-api/efuns.md)
  - [Event Hooks](./lua-api/events.md)
  - [File & System Access](./lua-api/file-access.md)
  - [Interface — Prompt, Colour, Pager, Channels](./lua-api/interface.md)
  - [Observability & Logging](./lua-api/observability.md)
  - [Debugging & Tracing](./lua-api/debugging.md)
  - [OLC — Building In-Game](./lua-api/olc.md)
  - [Prototypes — Authoring by Inheritance](./lua-api/prototypes.md)
  - [Permissions & Roles](./lua-api/permissions.md)
  - [Sandboxing & Security](./lua-api/sandboxing.md)
  - [Performance & the JIT Trade-off](./lua-api/performance.md)
  - [Compute — Off-Thread Lua](./lua-api/compute.md)
- [Configuration Reference](./configuration.md)
  - [permissions.toml](./configuration/permissions-toml.md)

---

# For Driver Developers (Rust)

- [Rust API Reference](./rust-api/README.md)
  - [Traits — Swappable Components](./rust-api/traits.md)
  - [Models — Account & Character](./rust-api/models.md)
  - [Extending Oxigeon](./rust-api/extending.md)
- [Testing](./testing.md)
- [Protocol Details](./protocols/README.md)
  - [Telnet (RFC 854)](./protocols/telnet.md)
  - [GMCP](./protocols/gmcp.md)
  - [MCCP2 Compression](./protocols/mccp.md)
  - [ECHO (Password Masking)](./protocols/echo.md)

---

# Reference

- [Changelog](./changelog.md)
