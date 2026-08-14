#!/usr/bin/env node
// The bridge.
//
// A browser cannot open a raw TCP socket, and the cockpit needs three things
// that are not WebSockets: telnet on 4000, the debug adapter on 4711, and the
// filesystem — `logs/journal.log` and every `.lua` file under `mudlib/` and
// `game/`, because the adapter has no `source` request and a debug client reads
// files itself.
//
// So this process sits between them. It speaks TCP and POSIX downwards and one
// WebSocket upwards, and it holds no UI state of its own: every frame it sends
// is something a socket or a file said. The client is where the debugger lives.
//
// It changes nothing about the driver.

import fs from 'node:fs'
import http from 'node:http'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { StringDecoder } from 'node:string_decoder'
import { WebSocketServer } from 'ws'

import { loadDriverConfig } from './config.js'
import * as dap from './dap.js'
import { discoverLuaFiles, readLuaFile } from './files.js'
import { tail } from './journal.js'
import * as telnet from './telnet.js'

const HERE = path.dirname(fileURLToPath(import.meta.url))

const USAGE = `\
oxigeon debug-client bridge — TCP and the filesystem, as one WebSocket

USAGE:
    node bridge/server.js [OPTIONS]

OPTIONS:
    --root <PATH>      the oxigeon checkout to read files from [default: ..]
    --config <PATH>    driver config to read ports from [default: <root>/config/driver.toml]
    --host <HOST>      server host [default: 127.0.0.1]
    --telnet <PORT>    override the telnet port
    --dap <PORT>       override the debug adapter port
    --journal <PATH>   journal to tail [default: <root>/logs/journal.log]
    --port <PORT>      port for this bridge to listen on [default: 4712]
    --serve            also serve the built web client from dist/
    -h, --help         print this

The debug adapter must be enabled in the driver config:

    [servers.debug]
    enabled = true
`

function parseArgs(argv) {
  const args = {
    root: path.resolve(HERE, '..', '..'),
    config: null,
    host: '127.0.0.1',
    telnetPort: null,
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
      case '--telnet':
        args.telnetPort = Number.parseInt(value(), 10)
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
const telnetPort = args.telnetPort ?? cfg.telnetPort
const dapPort = args.dapPort ?? cfg.dapPort

// ─── one browser, one set of connections ─────────────────────────────────────

/// Everything a single WebSocket client owns. Telnet is per-client because each
/// tab is its own player session; DAP is per-client because the adapter takes
/// one at a time and the second one has to be *told* so, rather than silently
/// sharing a session with the first.
function serve(ws) {
  const send = (frame) => {
    if (ws.readyState === ws.OPEN) ws.send(JSON.stringify(frame))
  }

  let size = { w: 100, h: 30 }
  let socket = null
  let adapter = null
  let stopJournal = null

  send({
    t: 'hello',
    root: args.root,
    telnet: `${args.host}:${telnetPort}`,
    dap: `${args.host}:${dapPort}`,
    journal: args.journal,
    autoContinueSecs: cfg.autoContinueSecs,
    debugEnabled: cfg.debugEnabled,
    files: discoverLuaFiles(args.root),
  })

  // ─── telnet ────────────────────────────────────────────────────────────
  {
    const parser = new telnet.TelnetParser()
    const answered = new Set()
    // A UTF-8 character can straddle a TCP read; the escape sequences it may be
    // sitting inside are the *client's* problem, and its ANSI decoder is a state
    // machine for exactly that reason.
    const decoder = new StringDecoder('utf8')

    socket = net.createConnection({ host: args.host, port: telnetPort }, () => {
      socket.setNoDelay(true)
      send({ t: 'telnet.up' })
    })

    socket.on('data', (chunk) => {
      const out = []
      for (const event of parser.feed(chunk)) {
        const { reply, emit } = telnet.respond(event, answered, size)
        if (reply) out.push(reply)
        for (const frame of emit) {
          if (frame.t === 'game') send({ t: 'game', text: decoder.write(frame.bytes) })
          else send(frame)
        }
      }
      if (out.length) socket.write(Buffer.concat(out))
    })

    socket.on('error', (e) => send({ t: 'telnet.down', why: `${args.host}:${telnetPort}: ${e.message}` }))
    socket.on('close', () => send({ t: 'telnet.down', why: 'server closed' }))
  }

  // ─── debug adapter ─────────────────────────────────────────────────────
  adapter = dap.connect({
    host: args.host,
    port: dapPort,
    onUp: () => send({ t: 'dap.up' }),
    onDown: (why) => send({ t: 'dap.down', why }),
    onMessage: (msg) => send({ t: 'dap.msg', msg }),
  })

  // ─── journal ───────────────────────────────────────────────────────────
  stopJournal = tail(args.journal, (entry) => send({ t: 'journal', entry }))

  ws.on('message', (raw) => {
    let frame
    try {
      frame = JSON.parse(raw)
    } catch {
      return send({ t: 'error', why: 'received a frame that was not JSON' })
    }
    switch (frame.t) {
      case 'input':
        // `encodeText` handles LF → CRLF and escapes 0xFF as IAC IAC, so a
        // player typing a high byte cannot inject a command.
        socket?.write(telnet.encodeText(`${frame.text ?? ''}\n`))
        break
      case 'naws':
        size = { w: frame.w | 0, h: frame.h | 0 }
        socket?.write(telnet.naws(size.w, size.h))
        break
      case 'dap':
        adapter?.request(frame.command, frame.arguments ?? {})
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
        send({ t: 'error', why: `unknown frame '${frame.t}'` })
    }
  })

  ws.on('close', () => {
    socket?.destroy()
    adapter?.close()
    stopJournal?.()
  })
}

// ─── listener ────────────────────────────────────────────────────────────────

const dist = path.join(HERE, '..', 'dist')
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' }

const server = http.createServer((req, res) => {
  if (!args.serve) {
    res.writeHead(404).end('bridge: WebSocket only (run `npm run web`, or start with --serve)')
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

server.listen(args.port, args.host, () => {
  process.stdout.write(
    `bridge listening on ws://${args.host}:${args.port}/\n` +
      `  telnet  ${args.host}:${telnetPort}\n` +
      `  dap     ${args.host}:${dapPort}${cfg.found && !cfg.debugEnabled ? '  (disabled in the driver config)' : ''}\n` +
      `  journal ${args.journal}\n` +
      `  root    ${args.root}\n` +
      (args.serve ? `  serving dist/ over http://${args.host}:${args.port}/\n` : '')
  )
})
