# TLS — `telnets://` and `wss://`

TLS is a wrapper around the socket, not a property of the protocol above it. By
the time a stream reaches the telnet parser or the WebSocket relay the handshake
is over and what is left is bytes, so **one acceptor serves both transports** and
neither of them knows which it got.

That is the whole design, and it is what made `telnets` cost a type change rather
than a second implementation of telnet.

## Two listeners, not a flag

```toml
[servers.telnet]              # telnet://
enabled = true
bind = "0.0.0.0"
port = 4000

[servers.telnet_tls]          # telnets://
enabled = true
bind = "0.0.0.0"
port = 4443
cert_path = "certs/server.crt"
key_path = "certs/server.key"

[servers.websocket]           # ws://
enabled = true
bind = "127.0.0.1"
port = 4001

[servers.websocket_tls]       # wss://
enabled = true
bind = "0.0.0.0"
port = 4444
cert_path = "certs/server.crt"
key_path = "certs/server.key"
```

Separate blocks because you almost always want both: existing players keep the
plaintext port while clients that can negotiate TLS use the secure one. A
listener is encrypted exactly when its block names a certificate.

**A server needs at least one listener.** Enabling none is a startup error, not
a process that exits 0 having accepted nothing.

## Certificates

```bash
mkdir -p certs
openssl req -x509 -newkey rsa:2048 -nodes -days 365 \
  -keyout certs/server.key -out certs/server.crt -subj "/CN=localhost"
```

PKCS#8, PKCS#1 and SEC1 keys are all accepted, so it does not matter which
`openssl` incantation produced yours. The certificate file is a PEM chain.

## Renewal, without a restart

Both files are watched and re-read when they change, so `certbot` renewal is
picked up on its own. `cert_reload_seconds` sets how often they are checked;
default 300, and 0 reads them once at startup and never again.

```toml
[servers.websocket_tls]
cert_reload_seconds = 300   # 0 = read once at startup
```

Polling rather than a filesystem watcher: a certificate changes every few
months, so minutes is the right granularity, two `stat` calls cost nothing, and
it behaves identically on Windows and Linux with no extra dependency.

Three details that matter:

- **Only new handshakes see a reloaded certificate.** A TLS session already
  established keeps the one it negotiated with. That is correct — that
  certificate authenticated that connection when it opened — and it means
  nobody is disconnected by a renewal.
- **A failed reload keeps the previous certificate**, and says so at `warn`.
  Renewal is not atomic: there is a moment where the certificate on disk is the
  new one and the key is still the old. A poll landing in that window must not
  take the listener down.
- **A failed reload is retried.** The file stamps are only recorded on success,
  so a half-written pair is picked up on the next tick without intervention.

The restart-free path is why a small deployment can serve `wss://` directly.
A reverse proxy is still the better answer once you want OCSP stapling, SNI
across several names, or HTTP on the same port.

## Failures are fatal, on purpose

A certificate that will not load stops the listener that asked for it, and a
listener that was asked for and did not come up stops the server. Neither
degrades to plaintext.

The reason is that the alternative is undetectable from outside: a port called
`telnet_tls` that quietly serves cleartext looks exactly like one that works,
to the operator and to every player on it. `cert_path` without `key_path` is
refused for the same reason.

## What clients need

| Client | |
|---|---|
| Mudlet | Connection settings → **Secure** |
| TinTin++ | `#session mud host 4443 {ssl}` |
| MUSHclient | Enable SSL in the world configuration |
| A browser | `wss://` — and note a page on `https://` **may not** open `ws://` at all; it is blocked as mixed content |

A self-signed certificate will be refused by a browser with no prompt when the
refusal comes from a script. Visit `https://host:4444/` once in a tab and accept
the warning first. MUD clients generally offer a "verify certificate" toggle.

## What is not here

- **Client certificates.** No mutual TLS; authentication is the mudlib's, in
  band.
- **ACME.** Nothing here obtains or renews a certificate; it only notices when
  one has been replaced on disk. Pair it with `certbot` or equivalent.
- **SNI.** One certificate per listener.
- **SSH.** A different protocol entirely, not a variant of this one — see the
  note in [WebSocket](./websocket.md) about why `telnets` covers what it would
  have been used for.
