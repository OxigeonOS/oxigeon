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

/// Connect to the adapter and pump messages, retrying until told to stop.
///
/// The adapter takes **one client at a time**, and drops connection number two
/// on the floor with no protocol error at all — which reads as a silent hang
/// unless it is detected and named. `spoke` is how: a close before the adapter
/// has said anything at all means it never accepted us.
///
/// Retrying is what makes reloading the page survivable. A browser opens the
/// new WebSocket *before* it closes the old one, so for a moment two bridge
/// sessions exist and the newer one's adapter connection is refused — and it
/// stayed refused, permanently, because nothing tried again. Every reload
/// therefore cost you the debugger until you restarted the bridge.
///
/// Reconnecting costs nothing to get right: breakpoints are client-owned truth
/// and re-sent on `initialized`, so a fresh attach lands back where it was.
export function connect({ host, port, onMessage, onUp, onDown }) {
  let seq = 0
  /// The wire protocol does not echo request arguments, and `variables`
  /// responses are indistinguishable without knowing which reference was asked
  /// for — so the originating request is reattached here under `__request`.
  const pending = new Map()

  let socket = null
  let timer = null
  let closed = false
  let attempts = 0

  function retry(why) {
    onDown(why)
    if (closed || timer) return
    attempts += 1
    // Quick at first — a reload resolves in well under a second — then backing
    // off, because a server that is simply not running should not be hammered.
    timer = setTimeout(() => {
      timer = null
      open()
    }, Math.min(400 * attempts, 4000))
  }

  function open() {
    if (closed) return
    const decoder = new DapDecoder()
    let spoke = false
    let settled = false
    /// One `onDown` per connection: `error` and `close` both fire for a refused
    /// connection, and two retries would be scheduled for one failure.
    const fail = (why) => {
      if (settled) return
      settled = true
      pending.clear()
      retry(why)
    }

    socket = net.createConnection({ host, port }, () => {
      socket.setNoDelay(true)
      attempts = 0
      onUp()
    })

    socket.on('data', (chunk) => {
      let messages
      try {
        messages = decoder.feed(chunk)
      } catch (e) {
        socket.destroy()
        fail(e.message)
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

    socket.on('error', (e) => fail(`${e.message} — is [servers.debug] enabled?`))
    socket.on('close', () =>
      fail(spoke ? 'adapter closed' : 'refused — another debug client is attached')
    )
  }

  open()

  return {
    request(command, args) {
      // While disconnected there is nowhere to put it. Queueing would be worse
      // than dropping: the adapter rejects most requests unless it is stopped,
      // so a replay on reconnect is a burst of refusals.
      if (!socket || socket.destroyed || socket.connecting) return false
      seq += 1
      pending.set(seq, { command, arguments: args })
      socket.write(encode({ seq, type: 'request', command, arguments: args }))
      return true
    },
    close() {
      closed = true
      clearTimeout(timer)
      timer = null
      socket?.destroy()
    },
  }
}
