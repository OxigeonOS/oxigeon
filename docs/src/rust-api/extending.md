# Extending Oxigeon

See [Traits](./traits.md) for the main guide to extending subsystems.

Additional extension points:
- **Adding efuns**: register new Lua functions in `src/core/scripting/efuns.rs`
- **New Telnet options**: add constants to `telnet/constants.rs`, handle in connection task
- **New GMCP packages**: handled in Lua via `on_gmcp` — no Rust changes needed
- **New network protocols**: implement a new listener module alongside `core/network/telnet/`
