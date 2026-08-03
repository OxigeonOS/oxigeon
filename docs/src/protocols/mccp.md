# MCCP2 — MUD Client Compression Protocol

MCCP2 (Telnet option 86) compresses server→client traffic using zlib, reducing bandwidth by 60-80%.

## Negotiation

```
Server → Client: IAC WILL MCCP2
Client → Server: IAC DO MCCP2

# To start compression:
Server → Client: IAC SB MCCP2 IAC SE
# All following bytes are zlib-compressed
```

## Status in Oxigeon — **negotiated, not performed**

> [!WARNING]
> **Oxigeon negotiates MCCP2 and does not compress.** The offer is sent, a
> client's `IAC DO MCCP2` is recorded, and the write stream is never wrapped in
> zlib. `flate2` is a declared dependency that appears nowhere in `src/`, and
> `mccp2_active` is never set true.
>
> This is a deliberate decision to record rather than a bug to fix quietly: the
> negotiation is harmless — a client that agrees to compression it never
> receives is not broken — and the alternative was to leave a page claiming a
> feature the server does not have.

### Why it has not been implemented

Compression is a bandwidth optimisation, and bandwidth is the resource a MUD is
least short of. A busy server sends a few kilobytes per player per minute. The
60–80% saving is real and it is 60–80% of a number that is already small.

What it would cost is not small: every write goes through a `flate2` encoder
with its own buffer and flush discipline, the flush has to happen at exactly the
right moment or output arrives late, and a bug in it corrupts the stream in a
way that looks like a client bug. That is a reasonable trade for a server
pushing megabytes; it is not one for a server pushing kilobytes.

### If you want it

The shape is: wrap the writer after the `IAC SB MCCP2 IAC SE` subnegotiation is
sent, flush on every complete message rather than on a byte count, and be
careful that the prompt — which is a `Raw` write with no newline — still
arrives. `mccp2_active` is the flag the rest of the code already expects.

## MCCP3

MCCP3 (option 87) enables client→server compression. The option constant is
defined and there is no handler. Same reasoning, more so: the client→server
direction carries a line of typing at a time.
