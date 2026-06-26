import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import basicSsl from '@vitejs/plugin-basic-ssl';

// Same-origin WebSocket proxy kept for LAN/mobile same-origin access. The Rust
// Voice Gateway is normally reached directly via VITE_FOXLINE_GATEWAY_URL.
const wsProxy = {};

// Mobile browsers require a secure context for microphone access; run with
// `HTTPS=1 bun dev` to serve over self-signed TLS for phones on the LAN.
const useHttps = !!process.env.HTTPS;

export default defineConfig({
  plugins: [react(), ...(useHttps ? [basicSsl()] : [])],
  server: { host: true, port: Number(process.env.PORT) || 5173, proxy: wsProxy },
  preview: { host: true, port: Number(process.env.PORT) || 4173, proxy: wsProxy },
});
