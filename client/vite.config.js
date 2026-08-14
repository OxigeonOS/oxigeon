import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

export default defineConfig({
  plugins: [svelte()],
  server: {
    port: 5173,
    // The driver serves no HTTP, so this dev server and the MUD are always two
    // different origins. Nothing is proxied: the page talks to `ws://` directly,
    // which is exactly what a deployed client would do.
    strictPort: false,
  },
  build: {
    outDir: 'dist',
  },
})
