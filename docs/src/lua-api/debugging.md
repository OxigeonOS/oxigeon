# Debugging & Tracing

Two tools, one subsystem: an in-game `trace` command for "what did this command
actually execute", and a VS Code debug adapter for breakpoints, stepping, and
variable inspection against the live server.

They share a single Lua hook, because a `lua_State` has exactly one hook slot.

---

## Quick reference

| I want to… | Use |
|---|---|
| See which functions a command ran | `trace calls` then `trace show` |
| Find which command is slow | `trace time` then `trace timings` |
| See every line executed | `trace lines` then `trace show` |
| Stop on a line and inspect variables | VS Code + `[servers.debug]` |
| Stop for one player only | A [conditional breakpoint](#conditional-breakpoints) |
| Play and debug in one window | [`oxigeon-tui`](../tui.md) |
| See what a *derived* trait actually resolves to | [`oxigeon-tui`](../tui.md), Inspect tab |

---

## The `trace` command

Requires the `admin` permission. Tracing is scoped to *your* session unless you
add `all`.

```
trace status            current settings and buffer usage
trace on | calls [all]  record function entry and exit
trace lines [all]       record every executed line (verbose)
trace time [all]        counters only — the cheapest useful mode
trace off [all]         stop
trace show [n]          last n records, oldest first (default 40)
trace timings [n]       per-command elapsed and counts (default 20)
trace clear             empty both ring buffers
```

Worked example — tracing the `who` command, verbatim:

```
> trace status
Trace status
  mode:      off
  hook:      removed (no overhead)
  scope:     0 session(s)
  records:   0 / 5000

> trace calls
Tracing calls for this session.
Every traced line runs interpreted — turn it off when done.

> who

> trace show 12
── Trace ──
   0.351ms >        lib/player.lua:307  get_color
   0.354ms <       lib/player.lua:312  get_color
   0.358ms >        lib/color.lua:69  colorize
   0.362ms >        [C]  colorize
   0.366ms >         lib/color.lua:71
   0.369ms <        lib/color.lua:75
   0.371ms >         lib/color.lua:71
   0.374ms <        lib/color.lua:75
   0.379ms >        [C]  send
   0.406ms >        daemons/snoop_d.lua:61  is_snooped
   0.409ms <       daemons/snoop_d.lua:62  is_snooped
   0.412ms <      lib/player.lua:354  _process_output
   0.414ms <     lib/player.lua:374  send_raw

> trace timings
── Command timings ──
   elapsed  verb              lines    calls  depth
    0.89ms  who                 205       82      8
    0.80ms  trace               162       64      8
```

Reading that: `who` finished in 0.89 ms across 205 executed lines and 82 calls,
nesting 8 frames deep. The tail of the trace is `who`'s output being colorized
and written — `player:send_raw` → `_process_output` → `colorize`, then the `send`
efun, then a check for snoopers.

`>` is a call, `<` a return, indentation is call depth, and the time column is
milliseconds since the start of that command's dispatch. `[C]` marks a C
function (an efun, `string.*`, …), which has no source line.

### Why returns show no value

A `<` line records *that* a function returned, not *what* it returned. This is a
LuaJIT constraint, not an oversight — `tests/debug_ret_spike.rs` records the
evidence:

```
Ret name=named_local locals=[n=<userdata>, doubled=5, (*temporary)=10, ...]
Ret name=computed    locals=[n=5, (*temporary)=16, ...]
Ret name=multi       locals=[n=5, (*temporary)=5, (*temporary)=6, ...]
```

The values are on the stack — `16` for `return n*3+1`, and `5, 6` for a
two-value return. But in `named_local(5)`, where `doubled` should be `10`, the
slot *named* `n` holds userdata and the slot *named* `doubled` holds `5`. The
name-to-slot mapping is shifted because LuaJIT is already dismantling the frame
when the return hook fires, and nothing distinguishes a return temporary from
any other temporary. Reporting them would mean guessing.

To see a value on the way out, put a breakpoint on the `return` line instead:
line events are reliable, and that is where the Variables pane and `evaluate`
both work.

### Notes

- **Nothing is installed when tracing is off.** The hook is removed entirely, so
  LuaJIT keeps JIT-compiling and the cost is zero. `trace status` reports this.
- **While tracing, the traced code runs interpreted.** Any hook forces LuaJIT off
  the JIT path. Turn it off when you are done.
- **Depth is measured, not counted.** Lua 5.1 reuses the caller's frame for tail
  calls (`return f(x)`) and emits no matching return, so a call/return counter
  drifts upward without bound — it reported a depth of 56 for `who` before this
  was fixed. Depth now comes from the real VM stack.
- Per-command timings are recorded for every dispatch, including login input.

---

## VS Code debugging

### One-time setup

1. **Install the extension.** VS Code will not let you set breakpoints against a
   debug type that no extension has registered, so a small shim is required.
   Copy or junction `tools/vscode-oxigeon-debug/` into your extensions folder:

   ```powershell
   # Windows
   New-Item -ItemType SymbolicLink `
     -Path "$env:USERPROFILE\.vscode\extensions\oxigeon-debug-0.1.0" `
     -Target "C:\Code\oxigeon\tools\vscode-oxigeon-debug"
   ```

   Then reload the window. The extension contains no logic — the adapter itself
   lives in the Rust server.

2. **Enable the adapter** in `config/driver.toml`:

   ```toml
   [servers.debug]
   enabled = true
   bind = "127.0.0.1"
   port = 4711
   ```

3. **Start the server**, then press <kbd>F5</kbd> and pick *Attach to Oxigeon*.
   The committed `.vscode/launch.json` already has the configuration.

### What works

- Breakpoints in any `.lua` file under `mudlib/` or `game/`
- **Conditional breakpoints** and hit counts
- Step over, step into, step out, continue, pause
- Call stacks with real file paths
- Locals, upvalues, and globals in the Variables pane
- Watch expressions and the Debug Console (read-only)

---

## Worked example: breaking inside `who`

Every transcript below is real output from `mudlib/cmds/who.lua`.

### Where it stopped

Breakpoint on `cmds/who.lua:19`, then a player types `who`:

```
stopped: reason=breakpoint

stackTrace:
   who.lua:19        cmds/who.lua:11
   [C]:0             pcall
   commands.lua:205  dispatch
   init.lua:132      mudlib/init.lua:125
```

That is the whole dispatch chain: the global `on_input` in `mudlib/init.lua`,
into `commands.dispatch`, through the `pcall` that guards every command, and
into `M.execute`. Frames named after a file and line — `cmds/who.lua:11` — are
functions the VM cannot name, because `pcall` invoked them; the fallback is
where the function was *defined*.

### Variables

```
scopes: Locals(expensive=False), Globals(expensive=True)

Locals in cmds/who.lua at line 19:
   session_id   string    "e898cffd-9a38-4c65-a37e-9edd49c9acb5"
   args_str     string    ""
   args         table     {}
   player       table     {account_id = 1, aggressive = false, channels = {...},
                           char_id = 1, custom = {...}, ...}  (27)
   sessions     table     ["e898cffd-9a38-4c65-..."]  (1)
   playing      table     {}
   total        number    1
```

No *Upvalues* scope appears because `M.execute` has none — it only touches
globals.

Tables are **summarised rather than printed as addresses**. `table: 0x025d651ea7b0`
tells you nothing; the preview shows the first few entries and the total key
count, so you can tell `args` and `playing` are empty and `sessions` holds one
id without expanding anything. Specifically:

- Pure sequences render with brackets — `["e898cffd-…"]  (1)`
- Records render with braces and `key = value` pairs
- Nested tables collapse to `{...}`; their children are one click away
- Keys are sorted, so a preview does not reshuffle between stops
- A table with a `__tostring` metamethod uses it instead — mudlib objects that
  define one know best how to describe themselves

Expanding `player` gives the real thing:

```
player -> 28 rows, first 6:
   account_id       number    1
   aggressive       boolean   false
   channels         table     {}
   char_id          number    1
   custom           table     {}
   description      string    "You see varuser."
```

(The preview says 27 and the expansion lists 28 rows: expansion appends a
synthetic `(metatable)` row, which the key count does not include.)

### Debug Console

```
   session_id                 => "f3911f7b-8695-4a08-a74a-3e79aa26f6cf"
   type(player)               => "table"
   player.name                => "varuser"
   #sessions                  => 1
   player.name .. " waves"    => "varuser waves"
   player = nil               => ERROR: assignment is not supported in evaluate
```

Locals of the paused frame are in scope, and globals resolve through the
environment. Assignment is refused — see [Limitations](#limitations).

### Conditional breakpoints

The loop in `who.lua` walks every connected session, so a plain breakpoint
inside it stops once per player. With two players online — `alice` and `bob` —
put a **condition** on line 24 to stop on only one:

```
condition: char.name == 'bob'
```

`alice` types `who`, and:

```
stopped: reason=breakpoint
  char.name        => "bob"
  s.character_id   => 2
  #playing         => 1     (players collected before this one)
  sid              => "0c368188-2294-4bde-8664-8fed52680ecd"

(continue) -> did not stop again — alice's iteration was filtered out
```

The condition is a Lua expression evaluated **in the paused frame**, so the
loop's own locals — `char`, `s`, `sid`, `playing` — are all in scope. This is
the difference between usable and unusable on a live MUD, where a shared
breakpoint otherwise trips for every player on the server.

Set one in VS Code by right-clicking the gutter → *Add Conditional
Breakpoint…*, or right-clicking an existing breakpoint → *Edit Breakpoint*.

**Hit counts.** A *Hit Count* of `3` ignores the first two times the line is
reached and stops on the third. Counts are per breakpoint and reset whenever you
edit your breakpoints. Only a plain number is honoured; expression forms like
`>5` or `%2` are ignored rather than guessed at.

**A condition that raises stops anyway, and says why:**

```
output event:
  Breakpoint condition failed at cmds/who.lua:24 — eval:1: attempt to index
  field 'nonexistent' (a nil value)
    condition: char.nonexistent.field == 1

stopped anyway: reason=breakpoint
```

Silently never stopping would be indistinguishable from a broken breakpoint, and
silently always stopping would be just as opaque — so it stops and prints the
error to the Debug Console.

> Conditions are only compiled and evaluated once a line has already matched, so
> they cost nothing on lines without a breakpoint. But a condition on a hot line
> runs on every pass: prefer the cheapest test that identifies what you want.

### ⚠ Freeze-the-world

**Hitting a breakpoint stops the entire game.** Every player is frozen until you
continue. This is not a limitation that can be engineered away here: the whole
mudlib runs on one thread in one VM, and mlua's hook API offers no yield on
LuaJIT — returning `VmState::Yield` there raises *"attempt to yield from a
hook"*. Blocking that thread is the only way to pause.

What that means in practice:

- Connections stay alive and player input queues; nothing is dropped.
- Output already queued still flushes.
- **Repeating timers accumulate** during the pause and fire as a burst on resume.
- **Game time does not pass.** `os_time()` excludes every interval the world
  spent frozen, so regeneration, cooldowns and effect durations are where you
  left them. Without that they ran on the wall clock while the VM sat blocked in
  the hook: a rat beaten down to 5 hit points came back to 20 over a few steps —
  1 hit point per 3 seconds of *reading a stack trace* — and combat looked
  endless for no visible reason. The counter is only ever non-zero on a server
  with the adapter enabled, so nothing about production timekeeping changes.
  One consequence worth knowing: an expiry written *during* a debugging session
  is stamped on the shifted clock, so after a restart it sits slightly in the
  future. On a development server that is not worth correcting.
- `auto_continue_secs` (default 300) resumes the VM if the editor stops
  responding, so a crashed VS Code cannot wedge the server permanently.

Use this on a development server.

### Limitations

- **Breakpoints are always reported "verified".** There is no cheap way to tell
  whether a line is executable, so one on a blank line, a comment, or an `end`
  simply never fires. This is the most common "the debugger is broken" report.
- **Startup code is not debuggable.** The hook is installed from the event loop,
  which runs after `init.lua` and all daemon module bodies have executed.
- **You cannot step into an efun.** Lua hooks do not fire inside C functions, so
  `player:send(...)` behaves like step-over.
- **Assignment is refused.** `doubled = 99` in the Debug Console returns an
  error rather than writing. Evaluation runs against an eager snapshot of the
  frame, because a metamethod runs at a different stack depth than the
  evaluator — a write applied from the wrong depth would corrupt live game state.
- **One client at a time.** A second connection is rejected.
- Expanding a table stops at 200 keys, and values are truncated to 256
  characters. Expanding a `Player` otherwise reaches the entire daemon graph.

### Security

The adapter grants **unauthenticated arbitrary Lua execution** in the game VM —
`evaluate` is a REPL with no login. Keep `bind = "127.0.0.1"`. The server logs a
warning if it is bound anywhere else.

Enabling it also loads the Lua `debug` standard library, which requires mlua's
unsafe VM constructor. The table is moved into the Lua registry and removed from
`_G` before any mudlib code runs, so game code still cannot reach it, and
`package.loadlib` is closed back up. With `[servers.debug]` absent or disabled —
the default — the VM is constructed exactly as it was before the debugger
existed.

---

## Troubleshooting

| Symptom | Cause |
|---|---|
| "Configured debug type 'oxigeon-lua' is not supported" | The extension is not installed |
| Breakpoints are hollow grey circles | Same — `contributes.breakpoints` is what binds them |
| Attaches but never stops | Breakpoint is on a non-executable line, or in startup code |
| Conditional breakpoint never fires | Its condition is falsy every time. Remove the condition to confirm the line is reached at all — a *raising* condition would have printed to the Debug Console. |
| Breakpoint fires for every player | Add a condition; see [Conditional breakpoints](#conditional-breakpoints) |
| Session hangs at "starting" | Use the *protocol trace* launch config and read the Debug Console |
| Everything is slow while attached | Expected — the hook disables the JIT. Detach when done. |
