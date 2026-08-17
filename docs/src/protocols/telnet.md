# Telnet (RFC 854)

Oxigeon implements the Telnet protocol per [RFC 854](https://datatracker.ietf.org/doc/html/rfc854).

## Protocol Basics

Telnet encodes control sequences as **IAC** (byte 255) followed by command bytes. Data bytes between control sequences are the player's text input or server output.

### IAC Byte Sequences

| Sequence | Meaning |
|----------|---------|
| `IAC WILL option` | We want to enable an option |
| `IAC WONT option` | We refuse to enable an option |
| `IAC DO option` | Please enable an option |
| `IAC DONT option` | Please disable an option |
| `IAC SB ... IAC SE` | Subnegotiation (option-specific data) |
| `IAC IAC` | Escaped byte 255 in data |

### Option Codes

| Constant | Value | Name |
|----------|-------|------|
| `OPT_ECHO` | 1 | Echo (password masking) |
| `OPT_SGA` | 3 | Suppress Go Ahead |
| `OPT_TTYPE` | 24 | Terminal Type |
| `OPT_NAWS` | 31 | Window Size |
| `OPT_MCCP2` | 86 | MCCP v2 Compression |
| `OPT_MXP` | 91 | MUD eXtension Protocol |
| `OPT_GMCP` | 201 | GMCP |

## Line Endings

Per the NVT spec, Oxigeon:
- **Sends**: `\n` in Lua strings is converted to `CR LF` (`\r\n`)
- **Receives**: `CR LF` → `\n`, `CR NUL` → `\r`, bare `LF` → `\n`

## Negotiation: RFC 1143 Q Method

Oxigeon implements the Q Method (RFC 1143) to prevent infinite negotiation loops. Each option has independent state for local (`us`) and remote (`him`) sides:

```
States: No, Yes, WantNo{queue}, WantYes{queue}
```

This ensures each WILL/WONT/DO/DONT gets exactly one response.

## Initial Negotiations

When a client connects, Oxigeon offers:
- `IAC WILL SGA` — we will suppress go-ahead
- `IAC DO SGA` — please suppress go-ahead  
- `IAC WILL GMCP` — we support GMCP
- `IAC WILL MCCP2` — we support compression
- `IAC DO TTYPE` — please tell us your terminal type
- `IAC DO NAWS` — please tell us your window size
- `IAC WILL MXP` — we support MXP (unless `[servers.telnet].mxp = false`)

MXP is offered last on purpose: a client that reads one round of negotiation and
then starts talking has already reported its terminal and window size before
markup enters the picture. See [MXP](./mxp.md).
