// Debug Adapter Protocol transport.
//
// A port of `src/core/scripting/debugger/dap/codec.rs`:
//
//   Content-Length: <byte-count>\r\n
//   [other headers, ignored]\r\n
//   \r\n
//   <byte-count bytes of UTF-8 JSON>
//
// `Content-Length` counts **bytes, not characters** — a body with any non-ASCII
// in it (a room description, a player name) will desynchronize the stream if
// this is measured in string length.

import net from 'node:net'

/// Refuse absurd headers rather than pre-allocating from them.
const MAX_BODY = 8 * 1024 * 1024

/// Incremental decoder. Feed it chunks, take whole messages out.
export class DapDecoder {
  constructor() {
    this.buf = Buffer.alloc(0)
    this.need = null
  }

  feed(chunk) {
    this.buf = this.buf.length ? Buffer.concat([this.buf, chunk]) : chunk
    const out = []

    for (;;) {
      if (this.need === null) {
        const end = this.buf.indexOf('\r\n\r\n')
        if (end === -1) return out // header block still incomplete
        const header = this.buf.subarray(0, end).toString('utf8')
        this.buf = this.buf.subarray(end + 4)

        const line = header
          .split(/\r?\n/)
          .find((l) => l.split(':')[0]?.trim().toLowerCase() === 'content-length')
        const len = line ? Number.parseInt(line.slice(line.indexOf(':') + 1).trim(), 10) : NaN
        if (!Number.isFinite(len)) throw new Error('DAP header has no usable Content-Length')
        if (len > MAX_BODY) throw new Error(`DAP body of ${len} bytes exceeds the limit`)
        this.need = len
      } else {
        if (this.buf.length < this.need) return out
        const body = this.buf.subarray(0, this.need).toString('utf8')
        this.buf = this.buf.subarray(this.need)
        this.need = null
        try {
          out.push(JSON.parse(body))
        } catch {
          throw new Error('DAP body is not valid JSON')
        }
      }
    }
  }
}

export function encode(message) {
  const body = Buffer.from(JSON.stringify(message), 'utf8')
  return Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, 'ascii'), body])
}

/// Connect to the adapter and pump messages.
///
/// The adapter takes **one client at a time**, and drops connection number two
/// on the floor with no protocol error at all — which reads as a silent hang
/// unless it is detected and named. `spoke` is how: a close before the adapter
/// has said anything at all means it never accepted us.
export function connect({ host, port, onMessage, onUp, onDown }) {
  const decoder = new DapDecoder()
  let spoke = false
  let seq = 0
  /// The wire protocol does not echo request arguments, and `variables`
  /// responses are indistinguishable without knowing which reference was asked
  /// for — so the originating request is reattached here under `__request`.
  const pending = new Map()

  const socket = net.createConnection({ host, port }, () => {
    socket.setNoDelay(true)
    onUp()
  })

  socket.on('data', (chunk) => {
    let messages
    try {
      messages = decoder.feed(chunk)
    } catch (e) {
      onDown(e.message)
      socket.destroy()
      return
    }
    for (const msg of messages) {
      spoke = true
      if (msg.type === 'response' && typeof msg.request_seq === 'number') {
        const req = pending.get(msg.request_seq)
        if (req) {
          pending.delete(msg.request_seq)
          msg.__request = req
        }
      }
      onMessage(msg)
    }
  })

  socket.on('error', (e) => onDown(`${e.message} — is [servers.debug] enabled?`))
  socket.on('close', () =>
    onDown(spoke ? 'adapter closed' : 'refused — another debug client is attached')
  )

  return {
    request(command, args) {
      if (socket.destroyed) return
      seq += 1
      pending.set(seq, { command, arguments: args })
      socket.write(encode({ seq, type: 'request', command, arguments: args }))
    },
    close() {
      socket.destroy()
    },
  }
}
