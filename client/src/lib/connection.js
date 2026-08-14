/**
 * The Oxigeon WebSocket envelope, as a small event emitter.
 *
 * Deliberately free of Svelte: this file is the reference implementation of the
 * protocol and should be readable by someone writing a client in something
 * else. Everything UI lives in the components.
 *
 * See docs/src/protocols/websocket.md for the wire format.
 */

/** @typedef {'raw'|'spans'|'none'} AnsiMode */

export class Connection {
  /**
   * @param {string} url  `ws://host:port/` or `wss://…`
   * @param {object} handlers
   */
  constructor(url, handlers = {}) {
    this.url = url
    this.h = handlers
    this.ws = null
    this.open = false
    /** Everything announced in `hello`, resent on every window resize. */
    this.caps = { width: 80, height: 24, gmcp: true, terminal: 'oxigeon-web', ansi: 'spans' }
  }

  connect() {
    // The mode goes in the URL, not only in the `hello` below. The server
    // writes the login banner the moment the socket opens, and a `hello` frame
    // cannot arrive before it — so without this the first several lines render
    // in the default mode and the rest in ours, with the boundary moving
    // depending on how long the handshake took.
    const u = new URL(this.url)
    u.searchParams.set('ansi', this.caps.ansi)
    u.searchParams.set('width', String(this.caps.width))
    u.searchParams.set('terminal', this.caps.terminal)
    this.ws = new WebSocket(u.toString())

    this.ws.onopen = () => {
      this.open = true
      // The server assumes 80 columns until told otherwise, so this is the
      // first thing that goes out — before the player can type anything that
      // would be wrapped to the wrong width.
      this.hello()
      this.h.onOpen?.()
    }

    this.ws.onmessage = (ev) => {
      let frame
      try {
        frame = JSON.parse(ev.data)
      } catch {
        // A frame we cannot read is the server's problem to fix, not a reason
        // to tear down a session someone is playing.
        this.h.onProtocolError?.('received a frame that was not JSON')
        return
      }
      this.#dispatch(frame)
    }

    this.ws.onclose = (ev) => {
      this.open = false
      this.h.onClose?.(ev.code, ev.reason)
    }

    // `onerror` carries nothing useful by design — the browser withholds the
    // detail. `onclose` always follows, and that is where the report belongs.
    this.ws.onerror = () => {}
  }

  #dispatch(frame) {
    switch (frame.type) {
      case 'text':
        this.h.onText?.(frame)
        break
      case 'prompt':
        this.h.onPrompt?.(frame)
        break
      case 'gmcp':
        this.h.onGmcp?.(frame.package, frame.data)
        break
      case 'echo':
        // `masked: true` means the *server* has taken over echoing, so this
        // client must stop showing what is typed — a password is being asked
        // for. The polarity is inverted relative to the efun names that produce
        // it; getting it backwards puts a password in the DOM.
        this.h.onEcho?.(frame.masked)
        break
      case 'bye':
        this.h.onBye?.(frame.reason ?? null)
        break
      case 'error':
        this.h.onServerError?.(frame.message)
        break
      case 'pong':
        this.h.onPong?.()
        break
      default:
        // Forward-compatible: a newer server may send a type this client has
        // never heard of, and that is not a reason to break.
        this.h.onUnknown?.(frame)
    }
  }

  #send(obj) {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(obj))
      return true
    }
    return false
  }

  /** One line of input. The server splits on newlines, so a paste is fine. */
  send(text) {
    return this.#send({ type: 'input', text })
  }

  /** (Re)announce capabilities. This transport's NAWS — send it on resize. */
  hello(patch = {}) {
    Object.assign(this.caps, patch)
    return this.#send({ type: 'hello', ...this.caps })
  }

  gmcp(pkg, data) {
    return this.#send({ type: 'gmcp', package: pkg, data })
  }

  ping() {
    return this.#send({ type: 'ping' })
  }

  close() {
    this.ws?.close()
  }
}

/**
 * A default URL from the page's own location.
 *
 * A page served over `https://` may not open a `ws://` socket — the browser
 * blocks it as mixed content — so the scheme has to follow the page's.
 */
export function defaultUrl() {
  const params = new URLSearchParams(location.search)
  if (params.get('ws')) return params.get('ws')
  const secure = location.protocol === 'https:'
  const port = params.get('port') ?? (secure ? '4444' : '4001')
  return `${secure ? 'wss' : 'ws'}://${location.hostname || 'localhost'}:${port}/`
}
