# Sandboxing & Security

Oxigeon runs Lua in a controlled sandbox designed to prevent untrusted mudlib code from escaping the server.

The boundary is one function — `apply_sandbox` in `src/core/scripting/sandbox.rs` — called by `ScriptEngine::start` after the efuns are registered and before any mudlib code loads. What game code can reach is exactly what survives that call, and `tests/driver/sandbox_reality_check.rs` asserts the table below against the VM the engine actually builds, not against a helper.

## What is Removed

| Module / Function | Status | Reason |
|-------------------|--------|--------|
| `io.*` | ❌ Removed | Arbitrary file system access |
| `os.execute` | ❌ Removed | Arbitrary command execution |
| `os.exit` | ❌ Removed | Would kill the server process |
| `os.getenv` | ❌ Removed | Environment variable leakage |
| `os.remove`, `os.rename`, `os.tmpname` | ❌ Removed | Uncontrolled file system writes |
| `debug.*` | ❌ Removed | Can escape sandbox, inspect/modify any closure |
| `jit.*` | ❌ Removed | `jit.on()` would disarm the instruction limit |
| `loadfile(path)` | ❌ Removed | Uncontrolled file loading; use `require` |
| `dofile(path)` | ❌ Removed | Same |
| `package.loadlib` | ❌ Removed | Loading native C extensions |
| `package.cpath`, C loaders | ❌ Removed | `require` cannot reach a native module |
| Binary bytecode (`\x1B...`) | ❌ Blocked | Only text Lua is allowed |

## What is Available

| Module / Function | Status | Notes |
|-------------------|--------|-------|
| `string.*` | ✅ Available | All functions |
| `table.*` | ✅ Available | All functions |
| `math.*` | ✅ Available | All functions |
| `coroutine.*` | ✅ Available | All functions |
| `pcall`, `xpcall` | ✅ Available | Error handling |
| `require(module)` | ✅ Jailed | Lua sources on `package.path` — game and mudlib roots only |
| `load(code)` | ✅ Text only | Binary bytecode rejected; returns `nil, err` |
| `read_file`, `write_file`, `append_file` | ✅ Jailed | Only within mudlib |
| `list_dir`, `file_exists`, `delete_file` | ✅ Jailed | Only within mudlib (and, for `list_dir`, the game root — jailed against each separately) |
| `uuid` | ✅ Available | v4; carries no host information |
| `math.random` | ✅ Available | **Seeded per VM at construction**, in Rust — see below |
| `os.time`, `os.date`, `os.clock`, `os.difftime` | ✅ Available | The clock functions have no side effects |
| `os_time`, `os_clock`, `os_date` | ✅ Available | Efun equivalents of the above |

## Why These Choices?

**`io` removed**: Unrestricted file access would allow mudlib code to read `/etc/passwd`, server private keys, or the database file directly. The `read_file`/`write_file` efuns provide controlled alternatives jailed to the mudlib directory *and* checked against the directory permissions in `config/permissions.toml`. Raw `io` walked around both.

**`os.execute` removed**: This would allow arbitrary shell command execution on the server host. This is a complete security boundary violation.

**`os.exit` removed**: Calling `os.exit()` from Lua would immediately kill the server process, allowing players to crash the game.

**`os` kept for clocks**: `os.time`, `os.date`, `os.clock` and `os.difftime` read a clock and do nothing else. Removing the whole table would break date formatting for no gain.

**`debug` removed**: The `debug` library allows inspecting and modifying closures, upvalues, and metatables — it can be used to break out of any sandbox by patching internal state. When the debug adapter is enabled the library is loaded but stashed in the registry and removed from `_G` before any mudlib code runs, so the game still cannot see it.

**Binary bytecode blocked**: Pre-compiled Lua bytecode is not validated by the
VM and can trigger memory corruption. `load` — and `loadstring`, on the runtimes
that have it — is replaced with a wrapper that rejects any chunk starting with
`\x1B`, and reports it the way `load` always has: `nil` plus a message.

The wrapper is otherwise transparent, and both halves of that matter:

- **`mode` is ignored.** Asking for `"b"` does not re-open the door the wrapper
  exists to shut; text is the only thing that compiles, whatever was requested.
- **`env` is honoured.** `load(src, name, "t", env)` sets the chunk's
  environment, which on Lua 5.2+ is the *only* way to do it — `setfenv` is gone.
  The wrapper used to drop that argument, and the failure was silent: the chunk
  still compiled and still ran, it just resolved every name against the globals.
  That quietly broke the whole debug evaluator — watch expressions, breakpoint
  conditions, the REPL and logpoints all compile a snapshot of the paused frame
  as the chunk's environment, so a local read as a global and came back `nil`.
  `tests/driver/sandbox.rs` pins both properties.

## The `require` Jail

`require` is available but restricted to Lua sources found on `package.path`, which the engine sets to the game and mudlib roots only. The native-module loaders are removed, so `require` cannot load a `.dll`/`.so` at all.

> The searcher list is called `package.loaders` in Lua 5.1 and `package.searchers`
> from 5.2. The sandbox clears **both** names, and refuses to start if it finds
> neither — an unrecognised searcher list is a module loader nobody has audited.
> Reading only the 5.1 name meant that on any 5.2+ runtime it found nothing, did
> nothing, and left the C loader installed: a sandbox that failed open, with no
> error and no failing test.

```lua
-- ✅ Allowed — loads from mudlib/lib/strings.lua
local strings = require("lib.strings")

-- ❌ Blocked — path traversal
local evil = require("../../evil")

-- ❌ Blocked — absolute path
local evil = require("/etc/passwd")
```

Dots in module names are converted to directory separators: `require("lib.strings")` → `mudlib/lib/strings.lua`.

## The PRNG is seeded per VM

LuaJIT starts every VM from a **constant** seed, and `math.randomseed` appeared
nowhere in `mudlib/`, `game/`, `src/` or `tests/`. Two fresh VMs both returned
`794206293` for the first `math.random(1, 1e9)`: identical combat to-hit and
damage rolls, identical loot outcomes, identical weighted echo choices and
identical virtual-room description variation on every restart. Not a subtle
bias — the same game twice.

Seeded in **Rust at VM construction**, immediately after `apply_sandbox` and
before any mudlib code can roll anything, rather than in `mudlib/init.lua`. That
way it covers every VM the engine builds: compute workers have their own, and
they are the ones meant to run simulations. Each worker is salted with its index
so two built in the same nanosecond still diverge.

`DAEMON.combat._roll` stays overridable, so a test that wants pinned numbers is
deterministic **by choice** rather than by accident. That distinction now
matters: before the seed, a combat test that forgot to pin its dice passed
anyway.

## Memory & CPU Limits

Configured via `config/server.toml`:

```toml
[limits]
lua_memory_mb = 64                # enforced
lua_instruction_limit = 1000000   # enforced; 0 = off
```

### `lua_memory_mb` — enforced

Applied to the VM at startup. An allocation past the ceiling raises a normal, catchable Lua error and the VM keeps serving, so one greedy command cannot take the game down. Set to `0` for no ceiling.

### `lua_instruction_limit` — enforced, but it costs the JIT

A limit greater than zero installs a `every_nth_instruction` debug hook that charges each dispatch against a budget and raises a Lua error past it. The budget is per dispatch: a command that blows it does not affect the next one.

**On a LuaJIT build, enforcing this disables the compiler.** (Lua 5.5 has none to
disable, so the budget costs only the hook there.) LuaJIT dispatches no debug hooks from inside a compiled trace, so with the JIT on, a one-line `while true do s = s + 1 end` delivers *no* hook events at all — not count, not line, not call. There is no hook mask that catches it. The engine therefore calls `jit.off()` at startup whenever a limit is configured, and `apply_sandbox` removes the `jit` table so game code cannot turn the compiler back on.

Measured through the real mudlib with `scripts/bench.ps1`:

| Workload | Cost of enforcing |
|---|---|
| `look` | 1.03× |
| `who` | 1.02× |
| `mudstatus` | 1.07× |
| Tight numeric loop *(control)* | 2.59× |

The compiler is worth 2.10× on that numeric loop and ~1.00× on real command dispatch, so **the limit is on by default**. See [Performance & the JIT Trade-off](./performance.md) for the full results and how to re-run them.

> [!WARNING]
> **Known gap.** The limit stops accidents, not sabotage. `pcall` catches the error the budget raises and Lua 5.1 has no uncatchable error, so
>
> ```lua
> while true do pcall(function() while true do end end) end
> ```
>
> still wedges the game thread. Every raise lands inside the inner loop at a fixed offset, so the outer loop is never reached. Treat the ability to write Lua on this server as a trusted privilege regardless of this setting.

While a debug adapter client is attached the budget is suspended — stepping through a breakpoint is meant to take as long as it takes.

## Permissions System

Oxigeon has a full RBAC (Role-Based Access Control) system that controls which efuns, commands, and file paths each character can access. Permissions are checked in-memory via `has_permission(session_id, perm)`.

See [Permissions & Roles](./permissions.md) for the full documentation.
