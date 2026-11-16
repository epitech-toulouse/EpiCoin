import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

// Where the API listens. The browser only ever talks to this server (same origin), which
// relays `/api` — exactly what nginx does in production.
const apiTarget = process.env.EPICOIN_API_URL ?? 'http://127.0.0.1:8090';
const proxy = { '/api': { target: apiTarget, changeOrigin: false } };

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: { port: 5173, proxy },
  preview: { port: Number(process.env.EPICOIN_WEB_PORT ?? 4173), strictPort: true, proxy },
  test: { include: ['src/**/*.test.ts'] },
});
