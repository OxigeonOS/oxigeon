# WebSocket

A second way in, onto the same sessions. A `send()` from Lua reaches a browser
and a telnet client by the same path, because everything above the connection
task was already transport-neutral: `Session` carries a `protocol` string,
`SessionHandler` is keyed by a UUID, and every output efun resolves a session id
to a channel without knowing what is on the far end.

Nothing in the mudlib needs to change to serve a WebSocket client, and nothing in
it did.

```toml
[servers.websocket]          # ws://
enabled = true
bind = "127.0.0.1"
port = 4001
max_frame_bytes = 65536      # largest client message accepted
ping_interval_secs = 30      # server keepalive; 0 disables
missed_pongs = 3             # unanswered pings tolerated (~90s at 30s)

[servers.websocket_tls]      # wss://
enabled = true
bind = "0.0.0.0"
port = 4444
cert_path = "certs/server.crt"
key_path = "certs/server.key"
```

Two listeners rather than a flag, so a plaintext port can stay open on loopback
for local development while the public one is encrypted. A browser on an
`https://` page **may not** open a `ws://` socket — it is blocked as mixed
content — so a hosted client needs the TLS listener or a proxy in front.

A renewed certificate is picked up without a restart. See [TLS](./tls.md).

## The wire format is a JSON envelope, not a text stream

Telnet carries an undifferentiated byte stream and signals everything else out of
band, in IAC sequences the text has to be escaped against. A frame protocol has
no such channel and needs none: every message says what it is.

Every frame is a JSON object with a `type`. Binary frames are refused.

### Server → client

| `type` | Fields | Meaning |
|---|---|---|
| `text` | `text` | A block of game text. One frame per `send()` from Lua. |
| `prompt` | `text` | Text belonging on the input line, not the scrollback. |
| `gmcp` | `package`, `data` | GMCP. `data` is a **nested JSON value**, not a string. |
| `echo` | `masked` | Whether to hide what the player types. See below. |
| `bye` | `reason?` | The server is ending the session; a close frame follows. |
| `error` | `message` | A frame could not be read. Advisory — the session lives. |
| `pong` | | Reply to a client `ping`. |

```json
{"type":"text","text":"You are in a forest clearing."}
{"type":"prompt","text":"HP:40/40 > "}
{"type":"gmcp","package":"Char.Vitals","data":{"hp":40,"maxhp":50}}
{"type":"echo","masked":true}
```

### Client → server

| `type` | Fields | Meaning |
|---|---|---|
| `input` | `text` | One or more lines of input. Split on `\n`; empty lines are kept. |
| `hello` | `width?`, `height?`, `gmcp?`, `terminal?`, `ansi?` | Capabilities. Repeatable. |
| `gmcp` | `package`, `data?` | Inbound GMCP. |
| `ping` | | Answered with `pong`. |

```json
{"type":"input","text":"look"}
{"type":"hello","width":100,"height":40,"gmcp":true,"terminal":"web","ansi":"spans"}
```

### Or in the upgrade URL

The same capabilities can be declared as query parameters, and `ansi`
usually should be:

```
ws://host:4001/?ansi=spans&width=120&height=40&terminal=web
```

**`on_connect` writes the login banner the moment the socket opens**, so a
`hello` frame cannot arrive before it. Declaring the mode in the URL settles it
before the first frame exists; leaving it to `hello` means the banner renders one
way and the rest of the session another, with the boundary moving depending on
how long the handshake took — visibly different between `ws://` and `wss://`
against the same server.

An unrecognised or unparseable parameter is ignored rather than refused. A query
string is part of a URL a human may have typed, and losing the session over a
typo in an optional hint is the worse outcome.

An unrecognised `type` gets an `error` frame and **the connection stays open**. A
running server outlives several versions of a browser client; closing on an
unknown frame would make every client deploy a hostile act.

## `echo` is inverted relative to the efuns that produce it

This is the single most likely thing to get backwards, and getting it wrong puts
a password into a browser's DOM.

The mudlib calls `start_echo(sid)` before asking for a password. On telnet that
sends `IAC WILL ECHO` — *the server* will do the echoing — so the client stops
echoing locally and the typing is hidden. `stop_echo` reverses it.

So the field is named for the player-visible effect, not for the efun:

| efun | frame | the player |
|---|---|---|
| `start_echo` | `{"type":"echo","masked":true}` | cannot see what they type |
| `stop_echo` | `{"type":"echo","masked":false}` | can see it again |

Order is load-bearing. Masking that arrives after the prompt it protects has
already let the player type in the clear, so the transport never reorders a
control frame around the text between them.

See [ECHO (Password Masking)](./echo.md) for the telnet half.

## Capabilities

A telnet client's capabilities emerge over several negotiation round trips. A
WebSocket client announces them in one `hello`, and may send another whenever
its window changes — that is this transport's NAWS.

Before any `hello`, a session starts at:

- `gmcp_supported = true`
- `window_width = 80`, `window_height = 24`
- `terminal_type = "websocket"`

GMCP defaults **on**, unlike telnet. A client that did not want GMCP would not
have connected to a JSON envelope, and the failure modes are not symmetric:
guessing "off" makes every `gmcp_d` sender return at its first guard while the
link still looks healthy — which is the exact bug `publish_capabilities` exists
to record. 80 columns matches the mudlib's own `Player.DEFAULT_WRAP_WIDTH`, so a
client that never says hello gets the wrap the mudlib would have chosen anyway
rather than a second, different guess.

A `hello` updates only the fields it carries; omitting one does not clear it.

The transport also pushes a `Core.Hello` GMCP frame at connect, the counterpart
of the one the telnet path sends directly.

## Line endings are normalized

`Player:_process_output` ends every message with `\r\n` and `strings.wrap` joins
with it. The driver is already in the business of per-transport line endings —
`TelnetCodec::encode_text` rewrites every bare `\n` into CRLF on the way out — so
this transport does the opposite:

- exactly one trailing terminator is stripped (a deliberate blank final line is
  content, and survives)
- interior `\r\n` becomes `\n`
- no `\r` reaches the client

A client can therefore split on `\n` and trust that there are no carriage
returns. Doing it once here is what keeps it out of every client: the first one
to forget would ship a UI with a phantom blank line after every message.

`prompt` frames are **not** normalized. `send_prompt` exists precisely to leave
the cursor on the line.

## Colour: three modes

`ansi` picks one, in the URL or in a `hello`. It defaults to `raw`, so a client
written before this existed keeps getting exactly what it got.

| mode | `text` | `spans` |
|---|---|---|
| `raw` | escape codes intact | — |
| `none` | escape codes stripped | — |
| `spans` | — | structured runs |

Exactly one of the two fields is present. `spans` omits `text` rather than
sending both: `text` is the busiest frame in the protocol and duplicating its
content would double it. A client that wants the plain string concatenates the
span texts.

```json
{"type":"text","spans":[
  {"text":"You are bleeding","fg":1,"bold":true},
  {"text":" badly."}
]}
```

A span carries `text` plus whichever of these apply: `fg`, `bg`, `bold`, `dim`,
`italic`, `underline`, `blink`, `inverse`, `strike`. Absent means off, so an
unstyled run is just `{"text":"…"}`.

**Colours are xterm-256 palette indices, always.** The sixteen basic ANSI
colours are 0-15 in that palette, so one integer covers everything
`lib/color.lua` can emit — `{red}` is `1`, `{bright_blue}` is `12`, `{orange}`
is `208` — and a client needs one lookup table rather than three. A 24-bit
`ESC[38;2;r;g;bm` sequence, which the mudlib does not produce but a hand-written
`send()` could, is folded onto the nearest palette entry rather than widening
the type.

Only SGR is interpreted. A cursor move or an erase is recognised and **dropped**
— a browser cannot act on one, and letting its parameter bytes through as text
is how `[2J` ends up printed in the scrollback.

### Why the driver parses this and not the client

Not because a browser could not: it is a few hundred lines. Because the
alternative is that *every* client reimplements the same state machine, and the
interesting cases in it — a style that must accumulate across two sequences, an
`ESC[m` that means reset, a truncated sequence at the end of a buffer — get
found once here, with tests, instead of separately in each one.

This is a **client capability**, not a player preference. The mudlib already
strips colour for a player who has turned it off, in `Player:_process_output`;
that is a game decision and it still applies. `ansi` answers a different
question — whether the thing on the far end can render an escape code at all. A
terminal can, a `<div>` cannot.

## Origin

A WebSocket is not subject to the same-origin policy: any page a visitor loads
can open a socket to your server from their browser. `allowed_origins` limits
which pages may:

```toml
[servers.websocket_tls]
allowed_origins = ["https://play.example.com", "http://localhost:5173"]
```

Empty — the default — accepts any. A refused upgrade gets `403 Forbidden` and
never becomes a session.

Origins are compared **exactly**. An `Origin` is scheme + host + port and
nothing else, so there is no path to normalise and no wildcard to get subtly
wrong; `*.example.com` matching is where this kind of check usually springs a
leak, and a list of exact origins is shorter to write and harder to be wrong
about. Note that `http://` and `https://` on the same host are different
origins, as are two ports.

**A request with no `Origin` header is allowed.** Browsers always send one;
anything else — a bot, `wscat`, a native client — sends none, and could put any
value there if it wanted to. Refusing the absent case would break every
non-browser client while stopping nothing, because the header is only
trustworthy in exactly the case where the browser controls it.

So this is a defence against *other people's pages*, not against attackers. It
matters less here than for a cookie-backed API — there is no ambient credential
and login is in band, so nobody's account is at risk — but it is how an
unrelated site turns its visitors into connections to your MUD, and
`max_connections` is a shared resource.

## A rich line arrives as ordinary text

`send_rich` produces a line with an action attached — "clicking this word sends
`buy bread`". Over telnet with [MXP](./mxp.md) negotiated that becomes a
`<SEND>` tag. Here it becomes an ordinary `text` or `prompt` frame with the
action **dropped**, and the prose intact.

There is no `rich` frame type, deliberately. One would mean every client needed
a new branch before it could render game text at all, and any client not updated
in the same breath — `debug-client/`, anything third-party — would silently drop
whatever the mudlib sent through it. Degrading to text costs one affordance,
which is exactly what a plain telnet client already gets.

Carrying the action as extra optional fields on a `Span` — `send`, `hint`,
`href` — so a browser could render a `<button>` is the obvious next step, and it
is wire-compatible with what ships today because no existing field would change.
It is not implemented yet.

MXP markup is **never** composed on this path. `ansi: "raw"` passes a non-SGR
CSI sequence through untouched, so an `ESC[1z` built anywhere but the telnet
transport would land in the browser's DOM as three visible characters. Choosing
the rendering at the transport, rather than in the efun, is what prevents that —
the same reason `AnsiMode` is read here and not where `send()` is called.

## What is deliberately absent

- **Compression** — see below.

Two things a mudlib written for telnet will do that look wrong in a browser, and
neither blocks play: the pager blanks a line by writing `\r`, spaces and `\r`
again, which is meaningless without a cursor; and box-drawn banners assume a
monospace grid.

## Why there is no permessage-deflate

Browsers offer `Sec-WebSocket-Extensions: permessage-deflate` on every upgrade.
This server declines it, which is legal — an extension only applies if the
server accepts it — and it is not going to stop declining soon. This section
exists so nobody spends an afternoon rediscovering why.

**No Rust WebSocket crate implements RFC 7692.** Checked, rather than assumed:

| | |
|---|---|
| `tungstenite` 0.29 | `protocol/mod.rs:643` rejects any frame with a reserved bit set (`NonZeroReservedBits`). No extension hook, no feature. Compressed frames set RSV1, so they cannot be read or written through it at all. |
| `fastwebsockets` 0.10 | No compression feature. |
| `tokio-websockets` 0.13 | No compression feature. |
| helper crates | None. |

Supporting it therefore means vendoring a patched WebSocket implementation and
writing the extension: parameter negotiation (`client_max_window_bits`,
`server_no_context_takeover` and the rest), per-message deflate with the
`00 00 FF FF` tail handling, and a compression context per direction per
connection. That is a permanent fork of security-critical framing code.

Worth knowing before anyone argues for it: the trade is worse here than it looks.
MUD messages are small, and permessage-deflate *expands* short ones — the win is
on help pages and long room prose, not on the `You hit the rat.` that dominates
the frame count. Context takeover also holds a window per direction per
connection, which is real memory at a few hundred players.

The place it would actually pay is a mobile client on a slow link reading a lot
of long-form text. If that becomes the case, the honest options are a vendored
fork or terminating at a proxy that does support it — nginx does.
