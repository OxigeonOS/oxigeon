# Lua API Reference

This section documents all Lua functions (efuns) provided by the Oxigeon driver.

Efuns are Rust functions exposed to Lua — they form the bridge between your mudlib and the driver's subsystems.

## Categories

- **[Efuns — Driver Functions](./efuns.md)** — `send()`, `send_prompt()`, `broadcast()`, `authenticate_session()`, `enter_game_session()`, `reload()`, etc.
- **[Event Hooks](./events.md)** — `on_connect`, `on_input`, `on_disconnect`, `on_gmcp`, `on_load`, `on_unload`
- **[File & System Access](./file-access.md)** — `read_file()`, `write_file()`, `list_dir()`, `os_time()`, `os_date()`, etc.
- **[Sandboxing & Security](./sandboxing.md)** — What is and isn't available, and why.

## Lua Version

Oxigeon uses **LuaJIT (API compatible with Lua 5.1)**. This means:

- Lua 5.1 standard library (string, table, math, coroutine)
- `setfenv`/`getfenv` available (removed in Lua 5.2+)
- **No** Lua 5.2+ features: `goto`, bitwise operators, integer types, UTF-8 library
- **No** Lua 5.3+ features: integer division `//`, bitwise `&|~^`, etc.
- JIT compilation for fast Lua code

## Available Standard Libraries

| Library | Available | Notes |
|---------|-----------|-------|
| `string` | ✅ | All functions |
| `table` | ✅ | All functions |
| `math` | ✅ | All functions |
| `coroutine` | ✅ | All functions |
| `io` | ❌ | Use `read_file()`, `write_file()`, `list_dir()` instead |
| `os` | ❌ | Use `os_time()`, `os_clock()`, `os_date()` instead |
| `debug` | ❌ | Disabled for security |
| `package.loadlib` | ❌ | No C extensions |
| `require` | ✅ (jailed) | Limited to mudlib directory |
