# Protocol Overview

Oxigeon supports the following MUD protocols:

- [Telnet (RFC 854)](./telnet.md) — The base TCP/Telnet protocol
- [WebSocket](./websocket.md) — A JSON envelope onto the same sessions, for browser clients
- [TLS](./tls.md) — `telnets://` and `wss://`, one acceptor for both
- [GMCP](./gmcp.md) — Generic MUD Communication Protocol for rich data exchange
- [MXP](./mxp.md) — MUD eXtension Protocol: clickable commands and client-side line tagging
- [MCCP2 Compression](./mccp.md) — Server→client zlib compression
- [ECHO (Password Masking)](./echo.md) — Input masking for passwords
