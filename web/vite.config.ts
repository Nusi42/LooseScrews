import { defineConfig } from "vite";

// Dev server on 5173; the game server (ls-server) serves the built client on 7777.
// In dev, /ws is proxied to a local ls-server so "Play online" works from Vite too.
export default defineConfig({
  server: {
    port: 5173,
    proxy: { "/ws": { target: "ws://127.0.0.1:7777", ws: true } },
  },
  build: { target: "es2022", outDir: "dist" },
});
