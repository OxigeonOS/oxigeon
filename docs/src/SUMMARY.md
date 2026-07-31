# Summary

[Introduction](./introduction.md)

---

# For Mudlib Creators (Lua)

- [Getting Started](./getting-started.md)
- [Architecture Overview](./architecture.md)
- [Lua API Reference](./lua-api/README.md)
  - [Daemons — Service Layer](./lua-api/daemons.md)
  - [Object Hierarchy](./lua-api/object-hierarchy.md)
  - [World Building — Rooms & Areas](./lua-api/world-building.md)
  - [Signals & Events (EVENT_D)](./lua-api/signals.md)
  - [Character Data & Persistence](./lua-api/character-data.md)
  - [Efuns — Driver Functions](./lua-api/efuns.md)
  - [Event Hooks](./lua-api/events.md)
  - [File & System Access](./lua-api/file-access.md)
  - [Observability & Logging](./lua-api/observability.md)
  - [Permissions & Roles](./lua-api/permissions.md)
  - [Sandboxing & Security](./lua-api/sandboxing.md)
- [Configuration Reference](./configuration.md)
  - [driver.toml](./configuration/driver-toml.md)
  - [server.toml](./configuration/server-toml.md)
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
