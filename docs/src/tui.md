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
projects, because of the adapter's defining property — hitting a breakpoint
stops the entire Lua VM, and *every player on the server freezes*. From an
editor that is invisible. You see a stopped stack; you do not see the game
standing still.

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
| `F9` | toggle a breakpoint on the cursor line |
| `F5` / `F10` / `F11` / `⇧F11` | continue / over / into / out |
| `^P` | pause — lands on the next line event, i.e. the next command a player types |
| `Enter` | open a file, expand a variable, or submit the REPL |

Scopes appear as collapsed headers. `Globals` is flagged expensive by the
adapter and is left alone until you expand it, because expanding it reaches the
entire daemon graph.

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
  installed, which forces LuaJIT onto the interpreter. The status bar says
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
never stops. `tests/tui_inspect_payload.rs` runs the Inspect tab's Lua against a
booted mudlib for the same reason.

See also [Debugging & Tracing](./lua-api/debugging.md) for the adapter itself,
the `trace` command, and the VS Code setup.
