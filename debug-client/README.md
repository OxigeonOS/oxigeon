# `debug-client` — the development cockpit, in a browser

A port of [`oxigeon-tui`](../docs/src/tui.md) to JavaScript. Same four tabs, same
journal strip, same argument: hitting a breakpoint stops the entire Lua VM and
*every player on the server freezes*, and from an editor that is invisible.

```
 F1 Play   F2 Debug   F3 Inspect   F4 Trace                        ^J journal
┌ game ─────────────────────────────────────────┐┌ room ─────────────────────┐
│ line 28 of the game                           ││ Thornhollow Square        │
│ line 29 ┌───────────────────────────────────┐ ││ thornhollow.square        │
│ line 30 │           ⏸  VM PAUSED            │ │└───────────────────────────┘
│ line 31 │     breakpoint at who.lua:19      │ │┌ vitals ───────────────────┐
│ line 32 │  every player on this server is   │ ││ hp ███████████░░░░  42/50 │
│ line 33 │              frozen               │ ││ mp ████████░░░░░░░  12/30 │
│ line 34 │       auto-continue in 4:52       │ │└───────────────────────────┘
│ line 35 └───────────────────────────────────┘ │┌ effects ──────────────────┐
└───────────────────────────────────────────────┘│ Blessed ×2            42s │
 game up   dap frozen   JIT off while attached  └───────────────────────────┘
```

## Two sockets, and why

**The game does not come through the bridge.** The driver has its own WebSocket
listener — a JSON envelope onto the same sessions telnet serves — and the
browser opens it directly.

| It needs | Reached by |
|---|---|
| the game | `ws://…:4001` — **the driver's own listener** |
| the debugger | the bridge: DAP on `:4711` is raw TCP, `Content-Length` framed |
| every `.lua` file | the bridge: **the adapter has no `source` request**, so a debug client reads files itself |
| `logs/journal.log` | the bridge: it is a file |

So `bridge/` is only what is left over — a small Node process that speaks TCP
and POSIX downwards and one WebSocket upwards, holding no UI state. Every frame
it sends is something a socket or a file said. It changes nothing about the
driver.

The three it does carry stay on one socket deliberately: a `stopped` event and
the file it lands in arrive together, so there is no ordering between two links
to get wrong.

**The bridge holds one adapter connection for its own lifetime**, not one per
browser session, and serves one cockpit at a time — opening a second evicts the
first. Both rules exist because the adapter takes one client: a connection per
session meant that reloading the page left the old one holding the adapter (a
browser opens the new socket before closing the old, and an abandoned socket is
not closed promptly), so every page after the first was refused, permanently,
with nothing saying why. Now a reload changes nothing about the adapter — the
same attached session is handed to whoever is looking at it.

There is no telnet client here and no ANSI decoder, and there were both.
`?ansi=spans` makes the driver send structured runs instead of escape codes, and
the protocol doc is emphatic about why: the interesting cases in that state
machine — a style accumulating across two sequences, an `ESC[m` that means
reset, a sequence truncated at a buffer boundary — get found once there, with
tests, rather than separately in every client. What is left in `src/lib/spans.js`
is the half a browser genuinely owns: a palette index and a few booleans into
CSS.

The envelope client itself is imported from `client/src/lib/connection.js`
rather than copied. It is the repo's reference implementation, and two clients
in one repo with their own idea of the wire format is how they drift.

## Running it

```toml
# config/driver.toml — both listeners, the adapter is off by default
[servers.websocket]
enabled = true

[servers.debug]
enabled = true
```

```bash
cargo run                # the server, from the oxigeon checkout

cd debug-client
npm install
npm run dev              # vite, which starts the bridge itself
```

Vite owns the bridge as a child process, so there is nothing to start in a
second terminal and nothing to forget. If one is already listening on 4712 it is
left alone, which is what makes `npm run bridge` — with your own `--root` or
`--dap` — still work: start it first, then `npm run dev`. `BRIDGE=off` disables
the spawn outright.

Or in one process, with no vite: `npm run build && npm run bridge -- --serve`,
then <http://127.0.0.1:4712/>.

> [!NOTE]
> **4712 is this project's own port, not the debug adapter's.** The bridge
> listens on 4712 and connects *out* to the adapter on 4711. A proxy
> `ECONNREFUSED 127.0.0.1:4712` means the bridge is not running — it is never a
> sign that a port is wrong.

Ports come from `config/driver.toml`, so a server on non-standard ports needs no
flags here either.

| Flag | |
|---|---|
| `--root <PATH>` | the checkout to read `mudlib/`, `game/` and `logs/` from (default `..`) |
| `--config <PATH>` | driver config to read ports from (default `<root>/config/driver.toml`) |
| `--host <HOST>` | default `127.0.0.1` |
| `--ws <PORT>` / `--dap <PORT>` | override either port |
| `--journal <PATH>` | journal to tail (default `<root>/logs/journal.log`) |
| `--port <PORT>` | port for the bridge itself (default `4712`) |
| `--serve` | also serve the built client from `dist/` |

`--root` matters when the game is not in this checkout. It does **not** matter
when `mudlib/` and `game/` are junctions into another one, which is the usual
arrangement: paths are canonicalized through the junction, landing on the same
text `require` produced, which is what makes a breakpoint match.

The page finds the bridge at same-origin `/bridge` (vite proxies it). Override
with `?bridge=ws://host:port/` or `?port=4712`.

> [!IMPORTANT]
> The adapter takes **one client at a time**. If VS Code is attached, this
> cannot be — it says `refused — another debug client is attached` rather than
> sitting there looking connected. Detach one before starting the other.

## The tabs

### F1 Play

A real MUD client: colour, command history, scrollback, over the driver's
envelope. `text` and `prompt` are separate frame types, so there is no "an
unterminated line is the prompt" heuristic here to get wrong.

The side panels are GMCP, not screen-scraping — `Char.Vitals`, `Char.Status`,
`Char.Effects` and `Room.Info`. GMCP defaults **on** for this transport, so
there is no `Core.Supports.Set` to send. `data` arrives as a nested JSON value,
not a string. The room panel shows the **dotted room id**, because that is what
`goto` takes and what the room's file is named after. Exits are buttons;
clicking one types it.

Password prompts mask automatically, off the `echo` frame — `masked: true` means
the *server* has taken over echoing, the inverse of the `start_echo` efun that
produces it. A masked line never enters the recallable history.

Resizing the pane sends a `hello`, which is this transport's NAWS.

### F2 Debug

Files, source with a breakpoint gutter, the call stack, a variables tree and a
REPL over `evaluate`.

| | |
|---|---|
| `Tab` / `⇧Tab` | cycle panes |
| `j` `k` / `↑` `↓` | move within a pane |
| `F9` | toggle a breakpoint on the cursor line — or click the gutter |
| `⇧F9` / `^L` | set or edit a **logpoint** — or right-click the gutter |
| `F5` / `^G` | continue |
| `F10` / `^→` | step over |
| `F11` / `^↓` | step into |
| `⇧F11` / `^↑` | step out |
| `^P` | pause — lands on the next line event, i.e. the next command a player types |
| `Enter` | open a file, expand a variable, or submit the REPL |

**Use the `^` aliases, or the buttons.** In a terminal the function keys are
merely unreliable; in a browser some are not available at all — **F11 is
full-screen and F12 is developer tools, and no page can intercept either**. F5
reloads unless the page prevents it, which this one does. Every step is also a
button in the toolbar, which is the one thing a browser gives the cockpit for
free.

Tab switching is `F1`–`F4`, with `Alt+1`–`Alt+4` as the alias for a browser
that keeps a function key for itself.

### The file tree

A collapsed tree rather than a list of paths. Every `.lua` file under `mudlib/`
and `game/` is several hundred rows, all of them beginning `mudlib/` — a list
you read rather than navigate. Only the two roots start open.

| | |
|---|---|
| `j` `k` / `↑` `↓` | move |
| `Enter` | open a file, or toggle a folder |
| `l` / `→` | open a folder, or descend into an open one |
| `h` / `←` | close a folder, or jump to the parent |

A red `●` beside a folder means something inside it has a breakpoint, so
collapsing the tree never hides one. Opening a file — including the one a stop
lands in — expands everything above it and selects it.

### Moving around a file

The source pane takes the vi motions:

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

`:` and `/` open a line editor in the footer row. `Enter` commits, `Esc`
abandons. While either is open every key is text — typing `n` in a search term
does not jump to the next match.

### Logpoints

`^L` (or right-click the gutter) opens a one-line editor for the line under the
cursor. Whatever you type becomes a breakpoint that **reports instead of
stopping**. `Enter` sets it, `Esc` abandons, and an **empty message removes it**
— the only way to un-set one without clearing the breakpoint and starting over.
Re-opening pre-fills with what is already there, so `^L` twice is an edit.

> [!IMPORTANT]
> The message is a **template**, not an expression. Plain text is printed as
> written; only `{...}` is evaluated. `player.is_alive()` on its own logs the
> literal string `player.is_alive()` — you wanted `alive={player.is_alive()}`.

The gutter marks a logpoint `◆` in cyan rather than `●` in red, because it will
never stop and a gutter that promised otherwise would be lying.

### Reading values

Scopes appear as collapsed headers. `Globals` is flagged expensive by the
adapter and is left alone until you expand it, because expanding it reaches the
entire daemon graph.

**Tab onto the variables pane and it takes the middle column**, swapping places
with the source. A narrow strip is enough to see *that* a local exists and not
much else, and reading values is most of what a debugger is for.

### Console output

Logpoint lines and breakpoint conditions that raised both arrive in the repl
pane, drawn differently on purpose: a logpoint reporting is `·` in green, and a
`⚠` in yellow means something needs looking at.

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

Drives the in-game `trace` command on your session and shows the block it
prints. This is text, not data, and is labelled as such: the trace rings live in
a thread-local on the Lua thread and are exposed only as pre-rendered strings
through the `trace_*` efuns. Needs a character holding `admin` / `efun.trace`.

## The journal strip

`^J` toggles a tail of `logs/journal.log` along the bottom, coloured by level,
with a filter over level, source and message. This needs no cooperation from the
server — it is a file — and because the driver writes **every** Lua crash there
with its traceback, a mudlib error appears whether or not you were attached when
it happened.

## What it will not do

- **Inspect while the VM is running.** `evaluate`, `stackTrace`, `scopes`,
  `variables` and every step request are rejected outright by the adapter unless
  it is stopped — they do not queue. Every control that needs a paused frame is
  disabled and says so.
- **Share the adapter.** One client at a time, as above.
- **Edit source.** It reads files; write them in your editor. The debugger has
  no `source` request, so what you see is what is on disk — reload after a
  change like you always would.
- **Hide the cost of attaching.** While a client is attached the hook stays
  installed, which on a LuaJIT build forces it onto the interpreter. The status
  bar says `JIT off while attached`.

## Tests

```bash
npm test
```

`node --test`, no framework. The suites over `lua.js` and the journal parser are
the Rust `#[cfg(test)]` modules ported case for case — what they assert is that
the port answers what the original answered, and a test rewritten to suit the
port would agree with whatever it happens to do.

`test/envelope.test.js` covers `client/src/lib/connection.js`, which had no
suite of its own. A shared file with two consumers and no tests is the thing
that drifts, and the rules it checks are the ones a client gets wrong
*silently*: that the colour mode goes in the URL because `on_connect` writes the
banner before a `hello` can arrive, that `masked: true` means hide, and that an
unknown frame type does not tear down a session someone is playing.

`test/debugview.test.js` is the one with no Rust counterpart, and it is the
important one: it asserts the protocol rules that this client gets wrong
*silently* if it gets them wrong at all — handshake ordering, that a breakpoint
travels as the absolute path `require` produced, that a resume invalidates every
`variablesReference`, that `allThreadsStopped` and not `stopped` is what draws
the freeze banner.

What no test here reaches is whether a breakpoint **fires** against a real VM —
`dap_live_tests.rs` does that on the Rust side, and doing it from here means
freezing a live server for `auto_continue_secs`.
