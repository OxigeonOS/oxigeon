# Oxigeon web client

A reference browser client for the WebSocket envelope — Svelte 5 and Vite, with
no dependency on anything else in this repository.

It lives in its own directory on purpose: the driver is a Rust project and
nothing about a JavaScript toolchain belongs at its root. Nothing here is built,
served or tested by `cargo`.

```bash
cd client
npm install
npm run dev        # http://localhost:5173
npm run build      # → client/dist, a static bundle
```

The driver serves no HTTP. `npm run dev` is a separate origin from the MUD, and
a deployed `dist/` is served by whatever you already use for static files. That
is also why the page has a URL box rather than a hardcoded endpoint.

## Pointing it at a server

Defaults to `ws://<the page's host>:4001/`, and to `wss://…:4444/` when the page
itself is on `https://` — a browser refuses a `ws://` socket from a secure page,
so the scheme has to follow. Override with the box in the header, or:

```
http://localhost:5173/?ws=ws://192.168.1.10:4001/
http://localhost:5173/?port=4501
```

The server side needs a `[servers.websocket]` block; see
`docs/src/protocols/websocket.md` and `docs/src/configuration.md`.

### A self-signed certificate

A browser will not open `wss://` to an untrusted certificate and gives you no
prompt when the refusal comes from a script. Visit `https://host:4444/` once in
a tab, accept the warning, and the socket will connect afterwards.

## What it does

- **Colour.** Asks for `ansi=spans` and renders each span as a styled
  `<span>`, so there is no SGR state machine here — `src/lib/palette.js` is a
  lookup table and nothing more.
- **Password masking.** An `echo` frame with `masked: true` switches the input
  to `type="password"` and keeps it out of the history. The polarity is
  inverted relative to the efun names that produce it; `connection.js` says so
  where it matters.
- **Width.** Measures the pane against a 100-character ruler and reports it, in
  the URL at connect time and again in a `hello` on every resize. That is this
  transport's NAWS.
- **GMCP.** An inspector rather than a HUD: it shows what the game is actually
  sending, package by package. A fixed set of vitals bars would look better and
  prove less, and would need editing every time the mudlib adds a package.
- Command history on ↑/↓, and a scrollback that follows new output only when you
  have not scrolled up to read something.

## Why the mode is in the URL

`on_connect` writes the login banner the moment the socket opens, so a `hello`
frame cannot arrive before it. Declaring `ansi` in the upgrade URL settles it
before the first frame is rendered — otherwise the banner arrives in one mode
and the rest of the session in another, with the boundary moving depending on
how long the handshake took. It was visibly different between `ws://` and
`wss://` against the same server before this was fixed.

## Layout

| File | |
|---|---|
| `src/lib/connection.js` | The protocol. No Svelte — read this one first if you are writing a client in something else. |
| `src/lib/palette.js` | xterm-256 → CSS, and one span → one inline style. |
| `src/lib/Output.svelte` | Scrollback and the prompt line. |
| `src/lib/Gmcp.svelte` | The GMCP inspector. |
| `src/App.svelte` | Connection state, input, width measurement, history. |

## What it is not

A finished game client. There is no triggers/aliases system, no macro bar, no
map, no mobile layout, no reconnect-on-drop, and no localisation. It exists to
prove the protocol end to end and to be a readable starting point.
