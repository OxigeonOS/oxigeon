# MXP — MUD eXtension Protocol

Telnet option **91**. MXP is a markup language for the game text stream: it
makes a word clickable, tells an automapper which line is a room name, and sets
client-side variables. The specification is Zugg Software's
[MXP 1.0](https://wiki.mudlet.org/images/c/ca/MUD_eXtension_Protocol.pdf)
(12-Mar-2003), public domain.

Oxigeon implements the server half. Clients that speak it include Mudlet,
MUSHclient, zMUD/CMUD and KildClient. The repository's own `oxigeon-tui`
deliberately refuses it — it answers `IAC DONT` to any option it cannot render,
which is the correct behaviour for a terminal UI with no link affordance.

## The one thing to understand first

MXP marks each **line** as open, secure or locked. On a secure line every tag
parses, including `<SEND>`, which runs a command. The specification is blunt
about what that means for a server:

> Be very careful when sending Secure lines from the MUD. Be absolutely sure
> that MUD players cannot control the output of a secure line. If a MUD player
> is able to send a secure MXP command, he will be able to cause great damage to
> other MUD players using MXP.

A MUD is a machine for showing one player's text to another. Item names, mob
names, descriptions, channel messages — all of them are reachable by somebody.
So "be careful" is not a strategy, and Oxigeon does not use one. Two mechanisms
replace it, and between them there is no call site that can be got wrong.

### 1. The default line mode is LOCKED

Immediately after negotiation the driver sends `ESC[7z`, which makes LOCKED the
mode every line reverts to. A locked line is not parsed at all, so:

```lua
send(sid, "You see a <sign> here. Tom & Sons.")
```

reaches an MXP client exactly as it reaches a plain telnet client — byte for
byte, no entity escaping, no transformation. A mob named
`<send href="quit">bow</send>` is a silly name and nothing more.

This is why enabling MXP is safe with a mudlib that knows nothing about it.

### 2. Line-mode sequences are stripped from game text

A mode tag is honoured in *every* mode, locked included — that is how a client
gets back out of locked mode. So a player who types

```
say <ESC>[1z<send href="quit">free gold here</send>
```

would otherwise have their own line promoted to secure on everyone else's
screen. `mxp::strip_line_modes` removes CSI sequences whose final byte is `z`
from all mudlib output once MXP is live, and from every input line. SGR ends in
`m` and is untouched, so colour is unaffected.

That single strip is the load-bearing part of the implementation.

### 3. Markup has exactly one door

`send_rich` takes a **tree**, not a string. `crate::core::render::to_mxp` is the
only function in the driver that emits a `<`, and every caller-supplied string
reaching it is escaped on the way. There is no `mxp_escape` helper, because
there is no call site that would have to remember to use one.

## Negotiation

```
S→C  FF FB 5B                 IAC WILL MXP     (last in the opening burst)
C→S  FF FD 5B                 IAC DO MXP
S→C  FF FA 5B FF F0           IAC SB MXP IAC SE
S→C  1B 5B 37 7A              ESC[7z           default mode := LOCKED
S→C  1B 5B 31 7A <VERSION>    ESC[1z<VERSION>
S→C  1B 5B 31 7A <SUPPORT>    ESC[1z<SUPPORT>
```

A client sending `IAC WILL MXP` — the wrong direction, but several do it meaning
agreement — is taken at its word. A repeated `DO` is ignored rather than
re-locking the stream and re-asking the handshake mid-page. `IAC DONT MXP` at
any point stops everything and clears the capability along with everything
derived from it.

The client answers on the **ordinary input stream**, on a secure line of its
own:

```
C→S  1B 5B 31 7A <VERSION MXP=0.4 CLIENT=mushclient VERSION=5.06>
C→S  1B 5B 31 7A <SUPPORTS +b +send.href -image>
```

The relay intercepts these before they reach `on_input`. Without that, every MXP
client's login would end with the mudlib complaining about something the driver
itself asked the client to send.

## Line modes

| Code | Meaning |
|---|---|
| `ESC[0z` | Open — presentational tags only (`<B> <I> <U> <S> <C> <H> <FONT>`) |
| `ESC[1z` | Secure — every tag parses |
| `ESC[2z` | Locked — nothing parses |
| `ESC[3z` | Reset — close open tags, reset colour, back to open |
| `ESC[4z` | Temp secure — secure for the next tag only |
| `ESC[5z` / `ESC[6z` / `ESC[7z` | Lock open / secure / locked as the new default |
| `ESC[10z` `ESC[11z` `ESC[12z` | Room name, description, exits (automapper) |
| `ESC[19z` | Welcome text |
| `ESC[20z`–`ESC[99z` | Game-defined categories |

Modes 0-2 and 10-99 revert to the default at the newline the client reads.
Oxigeon sends `ESC[6z` — lock **secure** — never: it is the mirror image of the
safety property above, and would make every unreviewed line of game text a
markup document.

## Lua API

### `send_rich(session_id, parts, opts?) → boolean`

```lua
send_rich(sid, {
    "The baker offers ",
    { send = "buy bread", hint = "A fresh loaf — 3 copper", text = item.short },
    " for three copper.",
})
```

Note the missing `\r\n`: unlike `send`, a rich line **terminates itself**, because
the line boundary is where an MXP mode reverts and the driver has to know where
it is. Pass `opts.newline = false` for a prompt.

An MXP client sees `a fresh loaf` underlined and clicking it sends `buy bread`.
Every other client sees the sentence with the word plain. That degradation —
one lost affordance, no prose lost — is why a rich line is safe to send to
anybody without checking `mxp_supported` first.

`parts` is an array whose elements are strings (literal text) or tables:

| Key | Meaning |
|---|---|
| `text` | Literal content. Mutually exclusive with `parts`. |
| `parts` | Nested array, for a group inside a group. |
| `send` | Command to run when clicked. An array is a popup menu. |
| `hint` | Mouse-over text. With a menu, `[1]` is the caption and the rest label the items. |
| `href` | A URL. `http://`, `https://` or `mailto:` only. |
| `prompt` | Put the command on the input line instead of running it. |
| `expire` | Name this link, for a later `mxp_expire`. |
| `var` | Also store the rendered text in a client variable of this name. |
| `br` | A hard line break. Ignores every other key. |
| `expire_now` | Emit `<EXPIRE>`, using `expire` as the name. |

`opts` takes `mode` (`"secure"` by default, or `"open"`/`"locked"`), `line`
(`"room_name"`, `"room_desc"`, `"room_exits"`, `"welcome"`, or a number 20-99),
and `newline` (`false` for a prompt).

```lua
-- A popup menu.
send_rich(sid, {
    { send = { "buy bread", "buy cake" },
      hint = { "Shop", "Bread — 3cp", "Cake — 8cp" },
      text = "the counter" },
})

-- Client-side room tagging, for a client with an automapper.
send_rich(sid, { room.name },        { line = "room_name" })
send_rich(sid, { room.description }, { line = "room_desc" })
```

**Author errors raise**, with the field named, the way `lua_to_json` and the
`db_*` efuns do — a command containing `|` (which separates menu items and has
no escape), a `javascript:` URL, a mixed table, a tree deeper than 32. **A full
output channel returns `false`**, because that is not the caller's fault.

Nothing here is permission-gated, and that is the design working rather than an
omission: `send_rich` cannot emit a `<` that did not come from the driver, so
gating it would be gating "send text".

### `mxp_var(session_id, name, value) → boolean`

`<VAR hp>40</VAR>` — set a client-side variable *and* display the value, which
is the difference between `<VAR>` and `<!ENTITY>`. Session-scoped, so there is
no registry to replay on reconnect. No terminator is appended.

### `mxp_expire(session_id, name?) → boolean`

Retire the links tagged with `name`, or every named link if omitted. Links that
never carried an `expire` name never expire. A room's exits stop being clickable
once the player has left the room.

### Capabilities

```lua
local s = get_session(sid)
if s.mxp_supported then … end
```

| Field | |
|---|---|
| `mxp_supported` | The client negotiated MXP. **Branch on this** — most clients never answer `<VERSION>` but parse markup perfectly well. |
| `mxp_version` | Spec level from its `<VERSION>` reply, e.g. `"0.4"`. |
| `mxp_client` | Client name and version, e.g. `"mushclient 5.06"`. Not `terminal_type`, which answers a different question. |
| `mxp_supports` | `+tag` / `-tag` tokens from its `<SUPPORTS>` reply, signs kept. |

### `on_mxp_ready(session_id)`

An optional global, called once per session on the client's first handshake
reply. MXP completes *after* `on_connect`, so a capability field answers "is it
there" but not "is it there *now*" — which is the question anything that wants
to greet the player with a clickable line has to ask. Same shape as
`on_auth_result`.

## Configuration

```toml
[servers.telnet]
mxp = true   # the default
```

Per listener, so `[servers.telnet]` and `[servers.telnet_tls]` can differ. Set
`false` to stop offering the option at all.

## What Oxigeon deliberately does not do

- **`<!ELEMENT>`, `<!ENTITY>`, `<!ATTLIST>`, `<!TAG>`.** Client-side macro
  definitions are a second structured data channel, and this driver already has
  GMCP, which carries values better and has a dispatch table behind it. Line
  tags cover the styling case at a fraction of the cost. Shipping both would
  mean a game with two vitals channels that disagree the first time one of them
  is guarded on a capability the other is not.
- **`<FRAME>`, `<DEST>`, `<IMAGE>`, `<SOUND>`, `<MUSIC>`, `<GAUGE>`, `<STAT>`.**
  Which file, when, at what volume, in which pane — all content decisions
  belonging to a game.
- **`<RELOCATE>`.** It tells a client to connect somewhere else. There is no
  legitimate use for that from a driver.
- **Auto-linking.** No turning exits or capitalised words into links. "You go
  north", "the north wind" and an exit named `north` are the same six letters,
  and the false-positive rate makes guessing worse than not.
- **Wrapping.** `strings.wrap` decides where lines break, in Lua, using the
  width `get_session` reports. A rich line the mudlib did not pre-wrap will not
  wrap around a `<SEND>`.
- **Styling in `send_rich`.** No `fg`, no `bold`. The mudlib already owns colour
  — `{colour}` tags become ANSI long before the driver sees a byte — and MXP is
  explicit that ANSI keeps working alongside it. A `text` part carries whatever
  escape sequences the mudlib put there and the escaper steps over them whole.

## WebSocket

A rich line reaches a WebSocket client as an ordinary `text` or `prompt` frame,
with the action dropped — the same degradation a plain telnet client gets. There
is no new frame type, deliberately: one would mean every client needed a new
branch before it could render game text at all.

Carrying the action as extra optional `Span` fields, so a browser could render a
button, is the obvious next step and is wire-compatible with what ships today,
because no existing field would change.

MXP markup is never composed on the WebSocket path. `AnsiMode::Raw` passes a
non-SGR CSI through untouched, so an `ESC[1z` built anywhere but the telnet
transport would land in a browser's DOM. Choosing the rendering at the transport
is what prevents that.

## Implementation

| | |
|---|---|
| `src/core/network/telnet/mxp.rs` | Modes, `MxpState`, the strip and the escaper, the handshake parser, negotiation policy |
| `src/core/render/mod.rs` | `Node` / `Group` / `RichLine`, `to_mxp`, `to_text` |
| `src/core/scripting/efuns_render.rs` | `send_rich`, `mxp_var`, `mxp_expire` and the marshaller |
| `src/core/network/telnet/relay.rs` | Handshake interception, output arms |
| `tests/driver/telnet_mxp.rs` | Negotiation, the injection, byte-identical game text, over a real socket |
