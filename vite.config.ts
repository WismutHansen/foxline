import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import basicSsl from '@vitejs/plugin-basic-ssl';

// Same-origin WebSocket proxy so the app works from other devices (phones) without
// hardcoding host IPs, and so wss:// works when serving over HTTPS.
const wsProxy = {
  '/ws/bridge': { target: `ws://127.0.0.1:${process.env.CODEC_BRIDGE_PORT || 8770}`, ws: true, rewrite: () => '/' },
  '/ws/stt': { target: `ws://127.0.0.1:${process.env.PARAKEET_STT_PORT || 8796}`, ws: true, rewrite: () => '/ws' },
};

// Mobile browsers require a secure context for microphone access; run with
// `HTTPS=1 bun dev` to serve over self-signed TLS for phones on the LAN.
const useHttps = !!process.env.HTTPS;

export default defineConfig({
  plugins: [react(), ...(useHttps ? [basicSsl()] : [])],
  server: { host: true, port: Number(process.env.PORT) || 5173, proxy: wsProxy },
  preview: { host: true, port: Number(process.env.PORT) || 4173, proxy: wsProxy },
});
