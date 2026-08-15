import { spawn } from 'node:child_process'
import net from 'node:net'
import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

const BRIDGE_PORT = Number(process.env.BRIDGE_PORT ?? 4712)

/// Is something already listening there?
function inUse(port) {
  return new Promise((resolve) => {
    const socket = net
      .connect({ host: '127.0.0.1', port })
      .on('connect', () => (socket.destroy(), resolve(true)))
      .on('error', () => resolve(false))
    socket.setTimeout(400, () => (socket.destroy(), resolve(false)))
  })
}

/// Start the bridge alongside the dev server.
///
/// It used to be `node bridge/server.js & vite` in an npm script, which is
/// broken on Windows: npm runs scripts through `cmd.exe`, where `&` separates
/// commands **sequentially** rather than backgrounding one. So the bridge ran,
/// blocked forever, and vite never started — and starting vite on its own gave
/// a proxy `ECONNREFUSED 127.0.0.1:4712` that names the bridge's own port and
/// looks for all the world like a wrong port number.
///
/// Owning the child here means there is nothing to forget and no shell operator
/// to be portable about. If a bridge is already up — someone running it by hand
/// with their own `--root` — this leaves it alone.
function bridge() {
  let child = null
  return {
    name: 'oxigeon-bridge',
    apply: 'serve',
    async configureServer(server) {
      if (process.env.BRIDGE === 'off') return
      if (await inUse(BRIDGE_PORT)) {
        server.config.logger.info(`  bridge already running on ${BRIDGE_PORT}, leaving it alone`)
        return
      }

      child = spawn(process.execPath, ['bridge/server.js', '--port', String(BRIDGE_PORT)], {
        cwd: import.meta.dirname,
        stdio: ['ignore', 'pipe', 'pipe'],
      })
      const relay = (chunk) => {
        for (const line of String(chunk).trimEnd().split('\n')) {
          server.config.logger.info(`  [bridge] ${line}`)
        }
      }
      child.stdout.on('data', relay)
      child.stderr.on('data', relay)
      child.on('exit', (code) => {
        if (code) server.config.logger.error(`  [bridge] exited with ${code}`)
        child = null
      })

      const stop = () => {
        child?.kill()
        child = null
      }
      server.httpServer?.on('close', stop)
      process.once('exit', stop)
      process.once('SIGINT', () => (stop(), process.exit(0)))
    },
  }
}

// The game does not come through vite at all: the browser opens the driver's
// own listener on 4001 directly. What is proxied is the bridge — the debugger,
// the file tree and the journal — so the page has one origin and no CORS or
// mixed-port confusion. `?bridge=` still overrides when it is not local.
//
// `fs.allow` reaches one level up because the envelope client is imported from
// `client/src/lib/connection.js` rather than copied. Two clients in one repo
// with their own idea of the wire format is how they drift.
export default defineConfig({
  plugins: [svelte(), bridge()],
  server: {
    port: 5273,
    fs: { allow: ['..'] },
    proxy: {
      '/bridge': {
        target: `ws://127.0.0.1:${BRIDGE_PORT}`,
        ws: true,
        // Without this the failure is a stack trace naming a port nobody chose,
        // repeated once per reconnect attempt.
        configure: (proxy) => {
          proxy.on('error', (err) => {
            if (err.code === 'ECONNREFUSED') {
              console.error(
                `\n  the bridge is not answering on ${BRIDGE_PORT}.` +
                  `\n  that is this project's own process, not the debug adapter (4711).` +
                  `\n  start it with \`npm run bridge\`, or let vite do it (unset BRIDGE=off).\n`
              )
            } else {
              console.error(`  bridge proxy: ${err.message}`)
            }
          })
        },
      },
    },
  },
})
