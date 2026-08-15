// Reconnecting to the adapter.
//
// The failure this is about: a browser opens the new WebSocket *before* it
// closes the old one, so on every page reload two bridge sessions briefly
// exist and the newer one's adapter connection is refused — the adapter takes
// one client at a time. Nothing tried again, so the debugger was gone until the
// bridge was restarted. Reloading a page is not an unusual thing to do.

import assert from 'node:assert/strict'
import net from 'node:net'
import { test } from 'node:test'

import { connect, encode } from '../bridge/dap.js'

const wait = (ms) => new Promise((r) => setTimeout(r, ms))

/// A stand-in adapter. `mode` decides how it treats each connection: `refuse`
/// hangs up without a word, which is exactly what the real one does to a second
/// client; `accept` speaks.
function adapter(mode = 'refuse') {
  const state = { mode, connections: 0, sockets: [] }
  const server = net.createServer((socket) => {
    state.connections += 1
    state.sockets.push(socket)
    if (state.mode === 'refuse') {
      socket.destroy()
      return
    }
    socket.on('data', () => socket.write(encode({ type: 'event', event: 'initialized' })))
  })
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () =>
      resolve({ ...state, get port() { return server.address().port },
        set mode(m) { state.mode = m },
        get connections() { return state.connections },
        close: () => new Promise((r) => { for (const s of state.sockets) s.destroy(); server.close(r) }) })
    )
  })
}

test('a refused connection is retried, and names why while it waits', async () => {
  const server = await adapter('refuse')
  const downs = []
  const client = connect({
    host: '127.0.0.1',
    port: server.port,
    onMessage: () => {},
    onUp: () => {},
    onDown: (why) => downs.push(why),
  })

  try {
    await wait(1400)
    assert.ok(server.connections >= 2, `expected a retry, saw ${server.connections} connection(s)`)
  assert.match(downs[0], /another debug client is attached/)
  } finally {
    client.close()
    await server.close()
  }
})

test('once the adapter frees up, the retry attaches — no restart needed', async () => {
  const server = await adapter('refuse')
  const messages = []
  const client = connect({
    host: '127.0.0.1',
    port: server.port,
    onMessage: (m) => messages.push(m),
    onUp: () => {},
    onDown: () => {},
  })

  try {
    // Note there is no assertion here that nothing "attached" while refusing.
    // A refusal is not a rejected TCP connection: the adapter accepts the
    // socket and *then* hangs up without a word, so the connect callback runs
    // either way. Whether a session exists is a protocol question, and the only
    // honest test of it is whether traffic flows.
    await wait(700)
    assert.equal(messages.length, 0, 'a refused connection carries no traffic')

    // The other client lets go.
    server.mode = 'accept'
    await wait(2000)

    client.request('initialize', {})
    await wait(400)
    assert.ok(
      messages.some((m) => m.event === 'initialized'),
      'the reattached session should carry protocol traffic'
    )
  } finally {
    client.close()
    await server.close()
  }
})

test('one failure schedules one retry, not one per error event', async () => {
  // `error` and `close` both fire for a refused connection. Scheduling on each
  // would double the rate every round.
  const server = await adapter('refuse')
  const client = connect({
    host: '127.0.0.1',
    port: server.port,
    onMessage: () => {},
    onUp: () => {},
    onDown: () => {},
  })

  await wait(1500)
  const seen = server.connections
  client.close()
  await server.close()

  // Backoff is 400ms, 800ms, … so 1.5s allows the first connection plus two
  // retries. Anything much above that is a doubling bug.
  assert.ok(seen <= 4, `expected paced retries, saw ${seen}`)
})

test('close stops it retrying for good', async () => {
  const server = await adapter('refuse')
  const client = connect({
    host: '127.0.0.1',
    port: server.port,
    onMessage: () => {},
    onUp: () => {},
    onDown: () => {},
  })
  await wait(600)
  client.close()
  const atClose = server.connections
  await wait(1200)

  assert.equal(server.connections, atClose, 'a closed client must not keep dialing')
  await server.close()
})

test('a request while disconnected is dropped rather than queued', async () => {
  // The adapter rejects most requests unless it is stopped, so replaying a
  // backlog on reconnect is a burst of refusals rather than a recovery.
  const server = await adapter('refuse')
  const client = connect({
    host: '127.0.0.1',
    port: server.port,
    onMessage: () => {},
    onUp: () => {},
    onDown: () => {},
  })
  await wait(500)
  assert.equal(client.request('stackTrace', {}), false)
  client.close()
  await server.close()
})
