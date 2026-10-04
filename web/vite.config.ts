import { createServer } from "node:net";
import { defineConfig } from "vite";

// Dev server: LS_DEV_PORT if set, else the preferred port 41873 (not Vite's
// default 5173: other Vite projects run on this machine at the same time), else
// any free port; strictPort on the chosen one, loopback only (no firewall prompt).
// The game server (ls-server) serves the built client on 7777. In dev, /ws and
// /cert-hash are proxied to a local ls-server so "Play online" works from Vite
// too (WebTransport then connects straight to the server's UDP port).
const PREFERRED_DEV_PORT = 41873;

/** Resolves to `port` when it is free on 127.0.0.1 (0: any free port). */
function tryPort(port: number): Promise<number | undefined> {
  return new Promise((resolve) => {
    const s = createServer();
    s.once("error", () => {
      resolve(undefined);
    });
    s.listen(port, "127.0.0.1", () => {
      const addr = s.address();
      const got = typeof addr === "object" && addr !== null ? addr.port : undefined;
      s.close(() => {
        resolve(got);
      });
    });
  });
}

async function devPort(): Promise<number> {
  const env = process.env["LS_DEV_PORT"];
  if (env !== undefined && env !== "") return Number(env);
  return (await tryPort(PREFERRED_DEV_PORT)) ?? (await tryPort(0)) ?? PREFERRED_DEV_PORT;
}

export default defineConfig(async ({ command }) => ({
  server: {
    host: "127.0.0.1",
    port: command === "serve" ? await devPort() : PREFERRED_DEV_PORT,
    strictPort: true,
    proxy: {
      "/ws": { target: "ws://127.0.0.1:7777", ws: true },
      "/cert-hash": "http://127.0.0.1:7777",
    },
  },
  build: { target: "es2022", outDir: "dist" },
}));
