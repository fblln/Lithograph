/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    proxy: {
      // Lets `npm run dev` talk to a `cargo run -- serve` instance without
      // both needing to share an origin -- production instead serves this
      // build's `dist/` and /rpc from the same Lithograph process.
      '/rpc': 'http://127.0.0.1:4317',
    },
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test-setup.ts'],
  },
})
