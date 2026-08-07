# `oxigeon-tui` — The Development Cockpit

```
  F1 Play    F2 Debug    F3 Inspect    F4 Trace
┌ game ──────────────────────────────────────────────┐┌ room ──────────────┐
│line 28 of the g┌ F5 continue · F2 debug ─────────┐  ││Thornhollow Square  │
│line 29 of the g│            ⏸  VM PAUSED         │  ││thornhollow.square  │
│line 30 of the g│                                 │  ││exits north east    │
│line 31 of the g│    breakpoint at who.lua:19     │  │└────────────────────┘
│line 32 of the g│ every player on this server is  │  │┌ vitals ────────────┐
│line 33 of the g│            frozen               │  ││hp ███████████░░ 42 │
│line 34 of the g│      auto-continue in 5:00      │  ││mp ████████░░░░░ 12 │
│line 35 of the g└─────────────────────────────────┘  │└────────────────────┘
└────────────────────────────────────────────────────┘└────────────────────┘
 telnet up  dap stopped  JIT off while attached
```

VS Code can debug your mudlib. It cannot **be** the MUD.

So the development loop is two windows: Mudlet in one, the editor in the other,
and no way to see them at the same moment. That matters more here than in most
projects, because of what hitting a breakpoint costs. By default it stops the
entire Lua VM, and *every player on the server freezes*. From an editor that is
invisible: you see a stopped stack, you do not see the game standing still.

(A Lua 5.5 build can suspend just the one dispatch instead — `trace freeze off`,
or `[servers.debug] stop_the_world = false`. Then the cost is the opposite kind
of invisible: the world keeps moving under whatever you are stepping through.
Either way it is a thing worth *seeing*, which is the argument for this window.)

`oxigeon-tui` opens both connections at once — telnet for play, DAP for debug —
and puts them in one window. You type `who` in the left pane; the right pane
fills with the dispatch chain, and the left greys out under a banner counting
the adapter's own `auto_continue_secs` down.

It changes nothing about the driver. It is a client.

## Running it

```toml
# config/driver.toml — the adapter is off by default
[servers.debug]
enabled = true
```

```bash
cargo run                    # the server
cargo run --bin oxigeon-tui  # the cockpit
```

Ports come from `config/driver.toml`, so a server on non-standard ports needs no
flags here either.

| Flag | |
|---|---|
| `--config <PATH>` | driver config to read ports from (default `config/driver.toml`) |
| `--host <HOST>` | default `127.0.0.1` |
| `--telnet <PORT>` / `--dap <PORT>` | override either port |
| `--journal <PATH>` | journal to tail (default `logs/journal.log`) |

> [!IMPORTANT]
> The adapter takes **one client at a time**. If VS Code is attached, the TUI
> cannot be — it will say `refused — another debug client is attached` rather
> than sitting there looking connected. Detach one before starting the other.

## The tabs

### F1 Play

A real MUD client: telnet, ANSI colour, command history, scrollback. The side
panels are GMCP, not screen-scraping — `Char.Vitals`, `Char.Status`,
`Char.Effects` and `Room.Info`, pushed by `gmcp_d` and gated on the
`Core.Supports.Set` this client sends. The room panel shows the **dotted room
id**, because that is what `goto` takes and what the room's file is named after.

Password prompts mask automatically: the driver sends `IAC WILL ECHO` around
them, and a masked line never enters the recallable history.

### F2 Debug

Files, source with a breakpoint gutter, the call stack, a variables tree and a
REPL over `evaluate`.

| | |
|---|---|
| `Tab` | cycle panes |
| `j` `k` / `↑` `↓` | move within a pane |
| `F9` | toggle a breakpoint on the cursor line |
| `⇧F9` / `^L` | set or edit a **logpoint** on the cursor line |
| `F5` / `^G` | continue |
| `F10` / `^→` | step over |
| `F11` / `^↓` | step into |
| `⇧F11` / `^↑` | step out |
| `^P` | pause — lands on the next line event, i.e. the next command a player types |
| `Enter` | open a file, expand a variable, or submit the REPL |

**Use the `^` aliases if the function keys do nothing.** They are not ours to
take: `F11` toggles full-screen in most terminals and never reaches the
application, and `F10` opens the menu bar in some. The arrows read as what they
do — down *into* a call, up *out* of it, right *along* the line. Both families
are always live; the pane footer shows them.

### The file tree

The files pane is a collapsed tree rather than a list of paths. Every `.lua`
file under `mudlib/` and `game/` is several hundred rows, all of them beginning
`mudlib/` — a list you read rather than navigate. Only the two roots start open.

| | |
|---|---|
| `j` `k` / `↑` `↓` | move |
| `Enter` | open a file, or toggle a folder |
| `l` / `→` | open a folder, or descend into an open one |
| `h` / `←` | close a folder, or jump to the parent |

A red `●` beside a folder means something inside it has a breakpoint, so
collapsing the tree never hides one. Opening a file — including the one a stop
lands in — expands everything above it and selects it, so you can always see
where you are.

### Moving around a file

The source pane takes the vi motions, because that is what your hands already do:

| | |
|---|---|
| `:` | go to a line number |
| `/` | search — case-insensitive, wraps at the ends |
| `//` | repeat the last search |
| `n` / `N` | next / previous match |
| `:noh` | stop highlighting, keeping the pattern for `n` |
| `g` / `G` | top / bottom |
| `j` `k` / `↑` `↓`, `PgUp` `PgDn` | move |

Lua is syntax-highlighted, and search hits are painted on top of it — including
inside a string or a comment, which is usually where you were looking.

`:` and `/` open a line editor in the footer row, next to the keys that apply.
`Enter` commits, `Esc` abandons. While either is open every key is text — typing
`n` in a search term does not jump to the next match — and matches are
highlighted in the source, so you can see why the cursor landed where it did
rather than having to trust it.

### Logpoints

`⇧F9` (or `^L`) opens a one-line editor in the REPL row for the line under the
cursor. Whatever you type becomes a breakpoint that **reports instead of
stopping**:

```
logpoint 235 › {attacker.name} hits {target.name} for {raw}
```

`Enter` sets it, `Esc` abandons the edit, and an **empty message removes it** —
the only way to un-set one without clearing the breakpoint and starting over.
Re-opening pre-fills with what is already there, so `^L` twice is an edit.

> [!IMPORTANT]
> The message is a **template**, not an expression. Plain text is printed as
> written; only `{...}` is evaluated. `player.is_alive()` on its own logs the
> literal string `player.is_alive()` — you wanted `alive={player.is_alive()}`.

The gutter marks a logpoint `◆` in cyan rather than `●` in red, because it will
never stop and a gutter that promised otherwise would be lying.

This is what you want on a line execution reaches over and over — a combat
round, a regeneration tick. A breakpoint there is a stop per round; a logpoint is
a running commentary. Conditions still apply, which is what makes "this player
only, every third pass" work. See
[Logpoints](./lua-api/debugging.md#logpoints).

### Reading values

Scopes appear as collapsed headers. `Globals` is flagged expensive by the
adapter and is left alone until you expand it, because expanding it reaches the
entire daemon graph.

**Tab onto the variables pane and it takes the middle column**, swapping places
with the source. A 38-column strip is enough to see *that* a local exists and
not much else, and reading values is most of what a debugger is for. Tab again
and the source comes back — one keystroke each way, and no mode to get stuck in.

### Console output

Logpoint lines and breakpoint conditions that raised both arrive in the repl
pane. They are drawn differently on purpose: a logpoint reporting is `·` in
green, and a `⚠` in yellow means something needs looking at — a condition that
failed to evaluate, a request the adapter refused, or a logpoint that hit its
per-dispatch limit.

### F3 Inspect

The pane the variables tree cannot be. Traits are derived values over a
dependency graph, and effects modify them without ever being stored — so
`entity.stats[id]` is the *stored* number and, for anything derived or buffed,
the wrong answer. A debugger can only show you the raw table.

This reads through `DAEMON.trait.all` and `DAEMON.effect.active` instead, and
shows the effective value with the stored base beside it where they differ.
`max_hp` on a fresh character has **no** stored value at all; this pane shows
what the formula produces.

Type any Lua expression as the target — `player`, a mob, an item. A trait is any
numeric datum on any entity, not a character statistic.

### F4 Trace

Drives the in-game `trace` command on your session and lifts the block it
prints. This is text, not data, and is labelled as such: the trace rings live in
a thread-local on the Lua thread and are exposed only as pre-rendered strings
through the `trace_*` efuns, with no path out of the process. Structured
`trace_*_data` efuns would make this a real pane.

Needs a character holding `admin` / `efun.trace`.

## The journal strip

`^J` toggles a tail of `logs/journal.log` along the bottom, coloured by level.
This needs no cooperation from the server — it is a file — and because the
driver writes **every** Lua crash there with its traceback, a mudlib error
appears whether or not you were attached when it happened.

## What it will not do

- **Inspect while the VM is running.** `evaluate`, `stackTrace`, `scopes`,
  `variables` and every step request are rejected outright by the adapter unless
  it is stopped — they do not queue. Every control that needs a paused frame is
  disabled and says so. The live view of your own character is the Play tab's
  GMCP panels.
- **Share the adapter.** One client at a time, as above.
- **Edit source.** It reads files; write them in your editor. The debugger has
  no `source` request, so what you see is what is on disk — reload after a
  change like you always would.
- **Hide the cost of attaching.** While a client is attached the hook stays
  installed, which on a LuaJIT build forces it onto the interpreter (Lua 5.5 is
  always interpreted, so there is nothing to lose there). The status bar says
  `JIT off while attached`, because "everything is slow" is expected and should
  not have to be rediscovered.

## Where it lives

`src/bin/tui/`, a second binary in the same crate. The DAP wire codec, the
telnet parser and the path normalisation are the driver's own code, reused
rather than reimplemented, so framing and path matching cannot drift between the
two ends.

Tests are in the binary rather than in `tests/`, because they need the real
client rather than a re-implementation of it: `dap_live_tests.rs` drives the
actual transport against a real adapter and a real Lua VM and asserts the
breakpoint fires. That last part matters — a path that does not match the
`@`-chunk name `require` produced is still answered `verified: true`, and then
never stops. `tests/demo_world/tui_inspect_payload.rs` runs the Inspect tab's Lua against a
booted mudlib for the same reason.

See also [Debugging & Tracing](./lua-api/debugging.md) for the adapter itself,
the `trace` command, and the VS Code setup.
