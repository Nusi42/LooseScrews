import { defineConfig } from "vite";

// Dev server on 5173; the game server (ls-server) serves the built client on 7777.
export default defineConfig({
  server: { port: 5173 },
  build: { target: "es2022", outDir: "dist" },
});
