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

## Status in Oxigeon

MCCP2 negotiation offers are sent during initial handshake. If the client accepts (`IAC DO MCCP2`), Oxigeon records this capability.

> [!NOTE]
> Full MCCP2 compression (wrapping the write stream in zlib) is not yet implemented in the driver. The negotiation infrastructure is in place. This is a good first PR opportunity.

## MCCP3

MCCP3 (option 87) enables client→server compression. This is not yet implemented.
