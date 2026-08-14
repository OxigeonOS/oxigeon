import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

// The bridge owns 4712 by default (see bridge/server.js). Proxying it through
// vite means the page has one origin and no CORS or mixed-port confusion, and
// `?bridge=` can still point somewhere else when the server is not local.
export default defineConfig({
  plugins: [svelte()],
  server: {
    port: 5273,
    proxy: {
      '/bridge': { target: 'ws://127.0.0.1:4712', ws: true },
    },
  },
})
