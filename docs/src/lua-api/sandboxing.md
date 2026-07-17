# Sandboxing & Security

Oxigeon runs Lua in a controlled sandbox designed to prevent untrusted mudlib code from escaping the server.

## What is Removed

| Module / Function | Status | Reason |
|-------------------|--------|--------|
| `io.*` | ❌ Removed | Arbitrary file system access |
| `os.execute` | ❌ Removed | Arbitrary command execution |
| `os.exit` | ❌ Removed | Would kill the server process |
| `os.getenv` | ❌ Removed | Environment variable leakage |
| `debug.*` | ❌ Removed | Can escape sandbox, inspect/modify any closure |
| `loadfile(path)` | ❌ Removed | Uncontrolled file loading; use `require` |
| `dofile(path)` | ❌ Removed | Same |
| `package.loadlib` | ❌ Removed | Loading native C extensions |
| `require` (outside mudlib) | ❌ Blocked | Jailed to mudlib directory |
| Binary bytecode (`\x1B...`) | ❌ Blocked | Only text Lua is allowed |

## What is Available

| Module / Function | Status | Notes |
|-------------------|--------|-------|
| `string.*` | ✅ Available | All functions |
| `table.*` | ✅ Available | All functions |
| `math.*` | ✅ Available | All functions |
| `coroutine.*` | ✅ Available | All functions |
| `pcall`, `xpcall` | ✅ Available | Error handling |
| `require(module)` | ✅ Jailed | Only from mudlib directory |
| `load(code)` | ✅ Text only | Binary bytecode rejected |
| `read_file`, `write_file`, `append_file` | ✅ Jailed | Only within mudlib |
| `list_dir`, `file_exists`, `delete_file` | ✅ Jailed | Only within mudlib |
| `os_time`, `os_clock`, `os_date` | ✅ Available | Safe subset of `os` |

## Why These Choices?

**`io` removed**: Unrestricted file access would allow mudlib code to read `/etc/passwd`, server private keys, or the database file directly. The `read_file`/`write_file` efuns provide controlled alternatives jailed to the mudlib directory.

**`os.execute` removed**: This would allow arbitrary shell command execution on the server host. This is a complete security boundary violation.

**`os.exit` removed**: Calling `os.exit()` from Lua would immediately kill the server process, allowing players to crash the game.

**`debug` removed**: The `debug` library allows inspecting and modifying closures, upvalues, and metatables — it can be used to break out of any sandbox by patching internal state.

**Binary bytecode blocked**: Pre-compiled Lua bytecode can trigger memory corruption bugs in LuaJIT. Only text source code is loaded.

## The `require` Jail

`require` is available but restricted:

```lua
-- ✅ Allowed — loads from mudlib/lib/utils.lua
local utils = require("lib.utils")

-- ❌ Blocked — path traversal
local evil = require("../../evil")

-- ❌ Blocked — absolute path
local evil = require("/etc/passwd")
```

Dots in module names are converted to directory separators: `require("lib.utils")` → `mudlib/lib/utils.lua`.

## Memory & CPU Limits

Configured via `config/server.toml`:

```toml
[limits]
lua_memory_mb = 64          # Max Lua VM memory
lua_instruction_limit = 1000000  # Max instructions per call
```

> [!NOTE]
> The instruction limit prevents infinite loops from hanging the server. A Lua script that exceeds the limit will receive a runtime error.

## Permissions System (Future)

A future version will allow creators to define per-character permission levels controlling which efuns they can call. The `this_session()` efun will be used to identify the caller.

```lua
-- Planned future API:
local session = get_session(this_session())
if session.permissions.can_reload then
    reload("some.module")
end
```
