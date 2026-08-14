// The WebSocket to the bridge.
//
// One socket carries all four things the cockpit needs — game text, GMCP, DAP
// and the filesystem — because they arrive interleaved and a reconnect that
// restored three of them would be worse than one that restored none.

/// Where the bridge is. `?bridge=` wins; otherwise same-origin `/bridge`, which
/// vite proxies in development and `--serve` answers directly.
export function bridgeUrl() {
  const params = new URLSearchParams(location.search)
  const given = params.get('bridge')
  if (given) return given
  const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
  const port = params.get('port')
  if (port) return `${scheme}://${location.hostname || 'localhost'}:${port}/`
  return `${scheme}://${location.host}/bridge`
}

/// Reconnecting client. The bridge is a local process a developer restarts
/// often, and a cockpit that has to be reloaded by hand every time is one they
/// stop using.
export class Bridge {
  constructor(handlers) {
    this.h = handlers
    this.ws = null
    this.open = false
    this.retry = null
    this.attempts = 0
  }

  connect() {
    const url = bridgeUrl()
    let ws
    try {
      ws = new WebSocket(url)
    } catch (e) {
      this.#scheduleRetry(String(e))
      return
    }
    this.ws = ws

    ws.onopen = () => {
      this.open = true
      this.attempts = 0
      this.h.onOpen?.()
    }
    ws.onmessage = (event) => {
      let frame
      try {
        frame = JSON.parse(event.data)
      } catch {
        this.h.onProtocolError?.('received a frame that was not JSON')
        return
      }
      this.h.onFrame?.(frame)
    }
    ws.onclose = () => {
      this.open = false
      this.h.onClose?.()
      this.#scheduleRetry('bridge closed')
    }
    // `onerror` is always followed by `onclose`, which is where the retry is.
    ws.onerror = () => {}
  }

  #scheduleRetry(why) {
    if (this.retry !== null) return
    this.attempts += 1
    const delay = Math.min(500 * this.attempts, 5000)
    this.h.onRetry?.(why, delay)
    this.retry = setTimeout(() => {
      this.retry = null
      this.connect()
    }, delay)
  }

  send(frame) {
    if (this.ws?.readyState !== WebSocket.OPEN) return false
    this.ws.send(JSON.stringify(frame))
    return true
  }

  input(text) {
    return this.send({ t: 'input', text })
  }
  naws(w, h) {
    return this.send({ t: 'naws', w, h })
  }
  dap(command, args) {
    return this.send({ t: 'dap', command, arguments: args })
  }
  file(path) {
    return this.send({ t: 'file', path })
  }
  files() {
    return this.send({ t: 'files' })
  }
}
