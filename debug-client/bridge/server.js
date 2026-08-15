#!/usr/bin/env node
// The bridge.
//
// The game does not come through here. The driver has its own WebSocket
// listener and the browser opens it directly — a JSON envelope onto the same
// sessions telnet serves, which is why there is no telnet client in this
// process and no ANSI decoder in the one upstairs.
//
// What is left is the three things a browser still cannot reach:
//
//   - the debug adapter, raw TCP on 4711, Content-Length framed
//   - every `.lua` file, because **the adapter has no `source` request** and a
//     debug client reads files itself
//   - `logs/journal.log`, tailed
//
// So this process speaks TCP and POSIX downwards and one WebSocket upwards. It
// holds no UI state: every frame it sends is something a socket or a file said.
//
// It changes nothing about the driver.

import fs from 'node:fs'
import http from 'node:http'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { WebSocketServer } from 'ws'

import { loadDriverConfig } from './config.js'
import * as dap from './dap.js'
import { discoverLuaFiles, readLuaFile } from './files.js'
import { tail } from './journal.js'

const HERE = path.dirname(fileURLToPath(import.meta.url))

const USAGE = `\
oxigeon debug-client bridge — the debugger and the filesystem, as one WebSocket

USAGE:
    node bridge/server.js [OPTIONS]

The game is not served by this process. The browser opens the driver's own
WebSocket listener; this reports where it is so nobody has to type the URL.

OPTIONS:
    --root <PATH>      the checkout to read files from [default: ..]
    --config <PATH>    driver config to read ports from [default: <root>/config/driver.toml]
    --host <HOST>      server host [default: 127.0.0.1]
    --ws <PORT>        override the driver's websocket port
    --dap <PORT>       override the debug adapter port
    --journal <PATH>   journal to tail [default: <root>/logs/journal.log]
    --port <PORT>      port for this bridge to listen on [default: 4712]
    --serve            also serve the built web client from dist/
    -h, --help         print this

Both listeners must be enabled in the driver config:

    [servers.websocket]
    enabled = true

    [servers.debug]
    enabled = true
`

function parseArgs(argv) {
  const args = {
    root: path.resolve(HERE, '..', '..'),
    config: null,
    host: '127.0.0.1',
    wsPort: null,
    dapPort: null,
    journal: null,
    port: 4712,
    serve: false,
  }
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]
    const value = () => {
      const v = argv[++i]
      if (v === undefined) throw new Error(`${arg} needs a value`)
      return v
    }
    switch (arg) {
      case '-h':
      case '--help':
        process.stdout.write(USAGE)
        process.exit(0)
      case '--root':
        args.root = path.resolve(value())
        break
      case '--config':
        args.config = value()
        break
      case '--host':
        args.host = value()
        break
      case '--journal':
        args.journal = value()
        break
      case '--serve':
        args.serve = true
        break
      case '--ws':
        args.wsPort = Number.parseInt(value(), 10)
        break
      case '--dap':
        args.dapPort = Number.parseInt(value(), 10)
        break
      case '--port':
        args.port = Number.parseInt(value(), 10)
        break
      default:
        throw new Error(`unknown argument '${arg}'`)
    }
  }
  args.config ??= path.join(args.root, 'config', 'driver.toml')
  args.journal ??= path.join(args.root, 'logs', 'journal.log')
  return args
}

let args
try {
  args = parseArgs(process.argv.slice(2))
} catch (e) {
  process.stderr.write(`bridge: ${e.message}\n\n${USAGE}`)
  process.exit(2)
}

const cfg = loadDriverConfig(args.config)
const wsPort = args.wsPort ?? cfg.wsPort
const dapPort = args.dapPort ?? cfg.dapPort

// ─── one adapter connection, for the life of the bridge ──────────────────────

/// **The adapter takes one client at a time, so this process is that client.**
///
/// It used to be one connection per browser session, which broke the obvious
/// thing: reloading the page. A browser opens the new socket before it closes
/// the old one, and a socket the browser has abandoned is not closed promptly
/// anyway — so the old session kept the adapter and every page after the first
/// was refused, permanently. The debugger was gone until the bridge restarted,
/// and nothing said why.
///
/// One connection, held here, sidesteps all of it. Reloading the page now
/// changes nothing about the adapter: the same attached session is handed to
/// whoever is looking at it.
let dapState = { t: 'dap.down', why: 'connecting' }
/// The browser session currently being served. One cockpit at a time, for the
/// same reason the adapter allows one client: two of them stepping the same VM
/// is not a thing either end can make sense of.
let session = null

const adapter = dap.connect({
  host: args.host,
  port: dapPort,
  onUp: () => {
    dapState = { t: 'dap.up' }
    session?.send(dapState)
  },
  onDown: (why) => {
    dapState = { t: 'dap.down', why }
    session?.send(dapState)
  },
  onMessage: (msg) => session?.send({ t: 'dap.msg', msg }),
})

function serve(ws) {
  const send = (frame) => {
    if (ws.readyState === ws.OPEN) ws.send(JSON.stringify(frame))
  }

  // Evict whoever was here. Closing it explicitly is what keeps a browser's
  // half-dead socket from being counted as a live cockpit.
  if (session && session.ws !== ws) session.ws.close(1000, 'another cockpit connected')
  session = { ws, send }

  send({
    t: 'hello',
    root: args.root,
    // Where the game is, for the client to open itself.
    game: `ws://${args.host}:${wsPort}/`,
    gameEnabled: cfg.wsEnabled,
    dap: `${args.host}:${dapPort}`,
    journal: args.journal,
    autoContinueSecs: cfg.autoContinueSecs,
    debugEnabled: cfg.debugEnabled,
    files: discoverLuaFiles(args.root),
  })

  // Whatever the adapter is doing already. A page that loads while attached
  // must not have to wait for the next state change to find out.
  send(dapState)

  const stopJournal = tail(args.journal, (entry) => send({ t: 'journal', entry }))

  ws.on('message', (raw) => {
    let frame
    try {
      frame = JSON.parse(raw)
    } catch {
      return send({ t: 'error', why: 'received a frame that was not JSON' })
    }
    switch (frame.t) {
      case 'dap':
        adapter.request(frame.command, frame.arguments ?? {})
        break
      case 'file': {
        const result = readLuaFile(args.root, frame.path)
        send({ t: 'file', path: frame.path, ...result })
        break
      }
      case 'files':
        send({ t: 'files', files: discoverLuaFiles(args.root) })
        break
      default:
        // The driver's own listener keeps a session alive through an unknown
        // frame rather than closing on it, on the grounds that a running server
        // outlives several versions of a browser client. Same reasoning here.
        send({ t: 'error', why: `unknown frame '${frame.t}'` })
    }
  })

  ws.on('close', () => {
    // The adapter connection outlives this session on purpose — see above.
    if (session?.ws === ws) session = null
    stopJournal()
  })
}

// Let go of the adapter when the bridge itself goes, so a restart is clean.
for (const signal of ['SIGINT', 'SIGTERM']) {
  process.once(signal, () => {
    adapter.close()
    process.exit(0)
  })
}

// ─── listener ────────────────────────────────────────────────────────────────

const dist = path.join(HERE, '..', 'dist')
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' }

const server = http.createServer((req, res) => {
  if (!args.serve) {
    res.writeHead(404).end('bridge: WebSocket only (run `npm run dev`, or start with --serve)')
    return
  }
  // `--serve` is for using the cockpit without vite running. Anything that is
  // not a file is index.html, because the client is one page.
  const rel = decodeURIComponent((req.url ?? '/').split('?')[0])
  let file = path.join(dist, rel === '/' ? 'index.html' : rel)
  if (!file.startsWith(dist) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
    file = path.join(dist, 'index.html')
  }
  if (!fs.existsSync(file)) {
    res.writeHead(404).end('bridge: nothing built yet — run `npm run build`')
    return
  }
  res.writeHead(200, { 'content-type': TYPES[path.extname(file)] ?? 'application/octet-stream' })
  fs.createReadStream(file).pipe(res)
})

new WebSocketServer({ server }).on('connection', serve)

const off = (enabled) => (cfg.found && !enabled ? '  (disabled in the driver config)' : '')

server.listen(args.port, args.host, () => {
  process.stdout.write(
    `bridge listening on ws://${args.host}:${args.port}/\n` +
      `  game    ws://${args.host}:${wsPort}/  — opened by the browser, not by this process${off(cfg.wsEnabled)}\n` +
      `  dap     ${args.host}:${dapPort}${off(cfg.debugEnabled)}\n` +
      `  journal ${args.journal}\n` +
      `  root    ${args.root}\n` +
      (args.serve ? `  serving dist/ over http://${args.host}:${args.port}/\n` : '')
  )
})
