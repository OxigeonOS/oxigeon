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
| Watch a line without stopping | A [logpoint](#logpoints) |
| Debug without freezing the other players | `trace freeze off` (needs a `lua55` build) |
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
trace freeze on|off     whether a breakpoint stops the whole game
```

Worked example — tracing the `who` command, verbatim:

```
> trace status
Trace status
  mode:      off
  hook:      removed (no overhead)
  scope:     0 session(s)
  breaks:    freeze the whole game
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
LuaJIT constraint, not an oversight — `tests/driver/debug_ret_spike.rs` records the
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

### Logpoints

A breakpoint on a line execution reaches over and over — a combat round, a
regeneration tick, a movement handler — is a stop over and over. Often what you
actually wanted was to *watch* it. Set a **logMessage** on the breakpoint instead
and it reports and keeps running:

```
line:       mudlib/daemons/combat_d.lua:235
condition:  target.name == 'rat'
logMessage: {attacker.name} hits {target.name} for {raw}
```

```
sheridan hits rat for 7
sheridan hits rat for 4
rat hits sheridan for 2
```

- **It is a template, not an expression.** Every `{...}` is evaluated in the
  frame, exactly as the Debug Console would, and substituted; everything else is
  printed as written. A message of `player.is_alive()` therefore logs that text
  verbatim — what you wanted is `alive={player.is_alive()}`.
- **The condition and hit count still apply**, which is where this earns its
  keep: one player, one mob template, every third pass. The gates are what turn
  "print everything" into a question.
- **Methods need a colon.** `player.is_alive()` passes no `self`, so it fails
  *inside* the callee — `mobile.lua:114: attempt to index a nil value (local
  'self')`, a file and a line with nothing to do with what you typed. Write
  `player:is_alive()`. The evaluator adds "did you mean :is_alive()?" when it
  can see that is what happened.
- An expression that cannot be resolved renders as `expr=<error>` in place
  rather than killing the line. A logpoint that went silent the moment one field
  went nil would be worse than useless on the line it is watching.
- Nothing stops, so this works the same on both runtimes and whatever
  `stop_the_world` says.
- Bounded at 200 lines per dispatch; the last one says so. A logpoint inside a
  loop is otherwise thousands of console events for a single command.

This is standard DAP, so VS Code's own Logpoint (right-click the gutter → *Add
Logpoint*) sets it with no extra configuration, and `oxigeon-tui` sets one with
`⇧F9` — see [the cockpit](../tui.md#logpoints).

### ⚠ What a breakpoint costs

**On a LuaJIT build, hitting a breakpoint stops the entire game.** Every player
is frozen until you continue. That is not a limitation that can be engineered
away there: the whole mudlib runs on one thread in one VM, and mlua's hook API
offers no yield on LuaJIT — returning `VmState::Yield` raises *"attempt to yield
from a hook"*. Blocking that thread is the only way to pause.

**On a `lua55` build you choose.** `[servers.debug] stop_the_world` defaults to
`true`, which behaves exactly as above — the expected debugger, and the only one
LuaJIT can offer. Set it `false`, or run `trace freeze off`, and the hook yields
instead: the engine parks that one command as a suspended coroutine and goes back
to its loop, so other players' commands, timers, regeneration and effect ticks
all carry on. That is what the runtime option is for — see
[LuaJIT against Lua 5.5](./performance.md#luajit-against-lua-55). Debugging a
server with people on it is a reasonable thing to do there, and is not on LuaJIT.

```
> trace freeze off
Breakpoints now suspend only the dispatch that hit them. Everyone else keeps playing.

> trace status
  breaks:    suspend one dispatch (other players keep playing)
  suspended: 2 dispatch(es) waiting at a breakpoint
```

Two things to expect once you do:

- **Several dispatches can be stopped at once**, each reported as its own DAP
  thread and named for what it is — `sheridan: hit`, `timer:combat.round`. Break
  on a line a combat round reaches and you get a stop *per round*, because the
  rounds keep coming. They are separately inspectable and separately resumable.
  If what you wanted was to watch rather than to stop, see
  [Logpoints](#logpoints).
- **The world moves while you read.** Whatever you are stepping through is being
  changed underneath you by everything that is still running. That is the trade;
  `stop_the_world = true` is the other side of it.
- **The garbage collector is held off for as long as anything is parked.** Not a
  tuning choice: a thread suspended at a hook yield sits with its stack `top`
  below its own live registers, and a collection nils them as a dead slice — the
  frame comes back with its parameters gone. Nothing outside the VM can raise a
  suspended thread's `top`, so the collector waits instead. The Lua heap
  therefore only grows while a stop is open; `auto_continue_secs` bounds how long
  that can last, and freezing (`stop_the_world = true`) never reaches it, because
  nothing allocates while the world is blocked.

Two things are the same on both:

- **A stop that cannot yield blocks either way.** Player commands *and ticks*
  run on coroutines, so breakpoints in combat, regeneration and effect ticks all
  suspend just the one dispatch. What is left blocking is connect and disconnect
  handlers, GMCP messages, hot reloads — and anything called *by C*, which cannot
  yield past that frame even on a coroutine: a `table.sort` comparator, a `gsub`
  replacement function, an `__index` metamethod. The hook asks `lua_isyieldable`
  and blocks whenever the answer is no, so both are correct rather than merely
  tolerable.
- **Game time does not pass while anything is stopped.**

What freezing means in practice, on LuaJIT or in one of the cases above:

- Connections stay alive and player input queues; nothing is dropped.
- Output already queued still flushes.
- **Repeating timers accumulate** during the pause and fire as a burst on resume.
- **`os_time()` excludes the frozen interval.** `os_time()` excludes every interval the world
  Regeneration, cooldowns and effect durations are therefore where you left
  them. Without it they ran on the wall clock while the VM sat blocked in the
  hook: a rat beaten down to 5 hit points came back to 20 over a few steps —
  1 hit point per 3 seconds of *reading a stack trace* — and combat looked
  endless for no visible reason. The counter is only ever non-zero on a server
  with the adapter enabled, so nothing about production timekeeping changes.
  One consequence worth knowing: an expiry written *during* a debugging session
  is stamped on the shifted clock, so after a restart it sits slightly in the
  future. On a development server that is not worth correcting.
- `auto_continue_secs` (default 300) resumes if the editor stops responding. On
  LuaJIT that is a safety valve against a crashed VS Code wedging the whole
  server; on `lua55` nothing is wedged, and what it rescues is the one player
  left at a dead prompt.

On LuaJIT, use this on a development server.

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
