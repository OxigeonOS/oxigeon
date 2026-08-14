// Telnet, the client half.
//
// A port of `src/core/network/telnet/parser.rs` (a pure byte state machine, so
// it does not care which end of the connection it is on) plus the negotiation
// *answers* from `src/bin/tui/telnet.rs`.
//
// The server offers `WILL SGA`, `DO SGA`, `WILL GMCP`, `WILL MCCP2`, `DO TTYPE`
// and `DO NAWS` on connect (`connection.rs::send_initial_negotiations`), then
// `WILL ECHO` / `WONT ECHO` around the password prompt.

export const IAC = 255
export const SE = 240
export const SB = 250
export const WILL = 251
export const WONT = 252
export const DO = 253
export const DONT = 254

export const OPT_ECHO = 1
export const OPT_SGA = 3
export const OPT_TTYPE = 24
export const OPT_NAWS = 31
export const OPT_MCCP2 = 86
export const OPT_MCCP3 = 87
export const OPT_GMCP = 201

const TTYPE_IS = 0
const TTYPE_SEND = 1

/// What we tell the server we understand. `gmcp_d.wants()` gates every outbound
/// package on this list — a module covers its packages, so `Char` buys
/// `Char.Vitals`, `Char.Status` and `Char.Effects`.
const SUPPORTS = '["Char 1","Room 1","Core 1"]'
const TERMINAL_TYPE = 'OXIGEON-WEB'
const CLIENT_VERSION = '0.1.0'

// ─── codec ───────────────────────────────────────────────────────────────────

export function encodeNegotiate(verb, option) {
  return Buffer.from([IAC, verb, option])
}

export function encodeSubnegotiation(option, payload) {
  return Buffer.concat([
    Buffer.from([IAC, SB, option]),
    Buffer.from(payload),
    Buffer.from([IAC, SE]),
  ])
}

export function encodeGmcp(pkg, json) {
  const payload =
    json === undefined || json === null
      ? Buffer.from(pkg, 'utf8')
      : Buffer.concat([Buffer.from(pkg + ' ', 'utf8'), Buffer.from(json, 'utf8')])
  return encodeSubnegotiation(OPT_GMCP, payload)
}

/// Split a GMCP payload on the first space, as `TelnetCodec::parse_gmcp` does.
export function parseGmcp(data) {
  const at = data.indexOf(0x20)
  if (at === -1) return { package: data.toString('utf8'), json: null }
  return {
    package: data.subarray(0, at).toString('utf8'),
    json: at + 1 < data.length ? data.subarray(at + 1).toString('utf8') : null,
  }
}

/// LF → CRLF, and 0xFF escaped as IAC IAC so a player typing a high byte
/// cannot inject a command.
export function encodeText(text) {
  const out = []
  for (const byte of Buffer.from(text, 'utf8')) {
    if (byte === IAC) out.push(IAC, IAC)
    else if (byte === 0x0a) out.push(0x0d, 0x0a)
    else out.push(byte)
  }
  return Buffer.from(out)
}

/// NAWS payload is two big-endian u16s: width then height.
export function naws(w, h) {
  return encodeSubnegotiation(OPT_NAWS, [(w >> 8) & 0xff, w & 0xff, (h >> 8) & 0xff, h & 0xff])
}

// ─── parser ──────────────────────────────────────────────────────────────────

const NORMAL = 0
const IN_IAC = 1
const IN_NEGOTIATION = 2
const IN_SUB = 3
const IN_SUB_IAC = 4

/// Byte-level telnet parser (RFC 854). Feed it chunks; it yields events.
/// `data` events carry a Buffer, because a UTF-8 character can straddle a TCP
/// read and decoding here would corrupt it.
export class TelnetParser {
  constructor() {
    this.state = NORMAL
    this.verb = 0
    this.option = 0
    this.data = []
    this.sub = []
    this.subOptionPending = false
  }

  #flushData(out) {
    if (this.data.length) {
      out.push({ kind: 'data', bytes: Buffer.from(this.data) })
      this.data = []
    }
  }

  feed(chunk) {
    const out = []
    for (const byte of chunk) {
      switch (this.state) {
        case NORMAL:
          if (byte === IAC) {
            this.#flushData(out)
            this.state = IN_IAC
          } else {
            this.data.push(byte)
          }
          break

        case IN_IAC:
          if (byte === IAC) {
            // Escaped IAC — an ordinary 0xFF data byte.
            this.data.push(255)
            this.state = NORMAL
          } else if (byte === WILL || byte === WONT || byte === DO || byte === DONT) {
            this.verb = byte
            this.state = IN_NEGOTIATION
          } else if (byte === SB) {
            this.sub = []
            this.subOptionPending = true
            this.state = IN_SUB
          } else if (byte === SE) {
            this.state = NORMAL // SE without SB — ignore
          } else {
            out.push({ kind: 'command', command: byte })
            this.state = NORMAL
          }
          break

        case IN_NEGOTIATION:
          out.push({ kind: 'negotiate', verb: this.verb, option: byte })
          this.state = NORMAL
          break

        case IN_SUB:
          if (this.subOptionPending) {
            this.option = byte
            this.subOptionPending = false
          } else if (byte === IAC) {
            this.state = IN_SUB_IAC
          } else {
            this.sub.push(byte)
          }
          break

        case IN_SUB_IAC:
          if (byte === SE) {
            out.push({ kind: 'subnegotiation', option: this.option, data: Buffer.from(this.sub) })
            this.sub = []
            this.state = NORMAL
          } else if (byte === IAC) {
            this.sub.push(255)
            this.state = IN_SUB
          } else {
            // A stray command inside a subnegotiation. Abandon it rather than
            // swallowing the rest of the stream looking for an SE.
            this.state = IN_SUB
          }
          break
      }
    }
    this.#flushData(out)
    return out
  }
}

// ─── negotiation ─────────────────────────────────────────────────────────────

/// Answer one telnet event.
///
/// Returns `{ reply, emit }` — bytes to write back, and what the UI should be
/// told. `answered` is a Set the caller keeps across the connection: everything
/// except ECHO is answered exactly once.
export function respond(event, answered, size) {
  const reply = []
  const emit = []

  switch (event.kind) {
    case 'data':
      emit.push({ t: 'game', bytes: event.bytes })
      break

    case 'negotiate': {
      const { verb, option } = event

      // ECHO is not an ordinary option: the server toggles it repeatedly around
      // every password prompt, so it must never be de-duplicated.
      if (option === OPT_ECHO) {
        if (verb === WILL) {
          reply.push(encodeNegotiate(DO, OPT_ECHO))
          emit.push({ t: 'echo', on: true })
        } else if (verb === WONT) {
          reply.push(encodeNegotiate(DONT, OPT_ECHO))
          emit.push({ t: 'echo', on: false })
        }
        break
      }

      // Everything else is answered once. Replying to a repeat is how a naive
      // client and a Q-method server talk each other into a loop.
      const key = verb * 256 + option
      if (answered.has(key)) break
      answered.add(key)

      if (verb === WILL && option === OPT_GMCP) {
        reply.push(encodeNegotiate(DO, OPT_GMCP))
        reply.push(
          encodeGmcp('Core.Hello', JSON.stringify({ client: 'oxigeon-web', version: CLIENT_VERSION }))
        )
        reply.push(encodeGmcp('Core.Supports.Set', SUPPORTS))
      } else if (verb === WILL && (option === OPT_MCCP2 || option === OPT_MCCP3)) {
        // MCCP2 is negotiated by the driver but never performed. `flate2` is a
        // declared dependency and is used nowhere in `src/`; `mccp2_active` is
        // declared on the connection and never set. Accepting would agree to a
        // compression that never starts, and every byte after would be garbage.
        reply.push(encodeNegotiate(DONT, option))
      } else if (verb === DO && option === OPT_NAWS) {
        reply.push(encodeNegotiate(WILL, OPT_NAWS))
        reply.push(naws(size.w, size.h))
      } else if (verb === DO && option === OPT_TTYPE) {
        reply.push(encodeNegotiate(WILL, OPT_TTYPE))
      } else if (verb === WILL && option === OPT_SGA) {
        reply.push(encodeNegotiate(DO, OPT_SGA))
      } else if (verb === DO && option === OPT_SGA) {
        reply.push(encodeNegotiate(WILL, OPT_SGA))
      } else if (verb === WILL) {
        // Refuse anything we have not implemented, rather than leaving it
        // unanswered — an unanswered DO stalls some servers.
        reply.push(encodeNegotiate(DONT, option))
      } else if (verb === DO) {
        reply.push(encodeNegotiate(WONT, option))
      }
      break
    }

    case 'subnegotiation':
      if (event.option === OPT_GMCP) {
        const { package: pkg, json } = parseGmcp(event.data)
        emit.push({ t: 'gmcp', package: pkg, json: json ?? 'null' })
      } else if (event.option === OPT_TTYPE && event.data[0] === TTYPE_SEND) {
        reply.push(
          encodeSubnegotiation(OPT_TTYPE, Buffer.concat([Buffer.from([TTYPE_IS]), Buffer.from(TERMINAL_TYPE)]))
        )
      }
      break

    // NOP, AYT and friends. Nothing here needs a response the driver acts on.
    case 'command':
      break
  }

  return { reply: reply.length ? Buffer.concat(reply) : null, emit }
}
