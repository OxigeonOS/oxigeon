// The envelope client, which this app imports from `client/src/lib/` rather
// than copying.
//
// It is the repo's reference implementation and it has no suite of its own, so
// these are here — a shared file with two consumers and no tests is the thing
// that drifts. What they assert is the handful of rules in
// docs/src/protocols/websocket.md that a client gets wrong *silently*.

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { Connection } from '../../client/src/lib/connection.js'

/// A fake socket, so nothing here opens a port.
class FakeSocket {
  static OPEN = 1
  constructor(url) {
    this.url = url
    this.readyState = 1
    this.sent = []
  }
  send(data) {
    this.sent.push(JSON.parse(data))
  }
  close() {
    this.readyState = 3
  }
}

/// Stand a connection up against the fake, and hand back what it did.
function connect(handlers = {}, url = 'ws://127.0.0.1:4001/') {
  const real = globalThis.WebSocket
  globalThis.WebSocket = FakeSocket
  try {
    const conn = new Connection(url, handlers)
    conn.connect()
    const socket = conn.ws
    socket.onopen?.()
    return {
      conn,
      socket,
      deliver: (frame) => socket.onmessage({ data: JSON.stringify(frame) }),
      raw: (data) => socket.onmessage({ data }),
    }
  } finally {
    globalThis.WebSocket = real
  }
}

test('the colour mode goes in the URL, not only in hello', () => {
  // `on_connect` writes the login banner the moment the socket opens, so a
  // `hello` frame cannot arrive before it. Without this the first several lines
  // render in the default mode and the rest in ours, with the boundary moving
  // depending on how long the handshake took.
  const { socket } = connect()
  const url = new URL(socket.url)
  assert.equal(url.searchParams.get('ansi'), 'spans')
  assert.equal(url.searchParams.get('width'), '80')
  assert.ok(url.searchParams.get('terminal'))
})

test('hello goes out on open, before anything can be typed', () => {
  // The server assumes 80 columns until told otherwise.
  const { socket } = connect()
  assert.equal(socket.sent[0].type, 'hello')
  assert.equal(socket.sent[0].ansi, 'spans')
})

test('hello updates only the fields it carries', () => {
  const { conn, socket } = connect()
  conn.hello({ width: 120 })
  const last = socket.sent.at(-1)
  assert.equal(last.width, 120)
  assert.equal(last.height, 24, 'omitting a field does not clear it')
  assert.equal(last.ansi, 'spans')
})

test('input is one frame; the server splits on newlines', () => {
  const { conn, socket } = connect()
  conn.send('look')
  assert.deepEqual(socket.sent.at(-1), { type: 'input', text: 'look' })
})

test('echo is masked-means-hide, the player-visible polarity', () => {
  // Inverted relative to the efuns that produce it: `start_echo` means the
  // *server* echoes, so the client must stop. Getting it backwards puts a
  // password in the DOM.
  const seen = []
  const { deliver } = connect({ onEcho: (masked) => seen.push(masked) })
  deliver({ type: 'echo', masked: true })
  deliver({ type: 'echo', masked: false })
  assert.deepEqual(seen, [true, false])
})

test('gmcp data arrives as a nested value, not a string to parse', () => {
  let got = null
  const { deliver } = connect({ onGmcp: (pkg, data) => (got = { pkg, data }) })
  deliver({ type: 'gmcp', package: 'Char.Vitals', data: { hp: 40, maxhp: 50 } })
  assert.equal(got.pkg, 'Char.Vitals')
  assert.equal(got.data.hp, 40)
})

test('text and prompt are separate types, so there is no heuristic to get wrong', () => {
  const seen = []
  const { deliver } = connect({
    onText: (f) => seen.push(['text', f.text]),
    onPrompt: (f) => seen.push(['prompt', f.text]),
  })
  deliver({ type: 'text', text: 'You are in a clearing.' })
  deliver({ type: 'prompt', text: 'HP:40/40 > ' })
  assert.deepEqual(seen, [
    ['text', 'You are in a clearing.'],
    ['prompt', 'HP:40/40 > '],
  ])
})

test('an unknown type is forwarded, not thrown away or fatal', () => {
  // A running server outlives several versions of a browser client.
  let unknown = null
  const { deliver, socket } = connect({ onUnknown: (f) => (unknown = f) })
  deliver({ type: 'from-a-newer-server', x: 1 })
  assert.equal(unknown.type, 'from-a-newer-server')
  assert.notEqual(socket.readyState, 3, 'the session is not torn down')
})

test('a frame that is not JSON is reported and the session lives', () => {
  let why = null
  const { raw, socket } = connect({ onProtocolError: (m) => (why = m) })
  raw('{not json')
  assert.match(why, /not JSON/)
  assert.notEqual(socket.readyState, 3)
})

test('sending on a closed socket answers false rather than throwing', () => {
  const { conn, socket } = connect()
  socket.readyState = 3
  assert.equal(conn.send('look'), false)
})
