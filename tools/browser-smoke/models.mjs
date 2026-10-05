#!/usr/bin/env node
// VQ4 model screenshots: serves the built client (web/dist) on 127.0.0.1,
// opens the model showcase (`?view=ships`, web/src/renderer/models/showcase.ts)
// in a local Chrome/Edge with WebGPU and saves before (`models=legacy`, the
// pre-VQ4 primitives) and after shots of the ship bay, the wrecks, the props
// and an in-flight view. Fails (exit 1) on console errors, WebGPU validation
// errors or a scene that never gets ready; exit 0 without a browser unless
// --require-browser.
//
//   node tools/browser-smoke/models.mjs [--web-root web/dist] [--out-dir target/visual-gallery/vq4]
//        [--browser <path>] [--headful] [--require-browser] [--only after|before]
//
// Needs `npm ci` in tools/browser-smoke and a built client (`cd web && npm run build`).

import { createReadStream, existsSync, mkdirSync, statSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { dirname, extname, join, normalize, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer-core";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "..", "..");

function parseArgs(argv) {
  const o = { webRoot: join(repo, "web", "dist"), outDir: join(repo, "target", "visual-gallery", "vq4"), browser: process.env.LS_BROWSER ?? "", headful: false, require: false, only: "" };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => argv[++i] ?? "";
    if (a === "--web-root") o.webRoot = resolve(val());
    else if (a === "--out-dir") o.outDir = resolve(val());
    else if (a === "--browser") o.browser = val();
    else if (a === "--headful") o.headful = true;
    else if (a === "--require-browser") o.require = true;
    else if (a === "--only") o.only = val();
    else if (a === "--help" || a === "-h") {
      console.log("node tools/browser-smoke/models.mjs [--web-root dir] [--out-dir dir] [--browser path] [--headful] [--require-browser] [--only after|before]");
      process.exit(0);
    } else {
      console.error(`unknown option ${a}`);
      process.exit(2);
    }
  }
  return o;
}

function findBrowser(explicit) {
  if (explicit) return existsSync(explicit) ? explicit : null;
  const env = process.env;
  const out = [];
  if (process.platform === "win32") {
    const roots = [env.ProgramFiles, env["ProgramFiles(x86)"], env.LOCALAPPDATA].filter(Boolean);
    for (const r of roots) out.push(join(r, "Google", "Chrome", "Application", "chrome.exe"));
    for (const r of roots) out.push(join(r, "Microsoft", "Edge", "Application", "msedge.exe"));
  } else if (process.platform === "darwin") {
    out.push("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge");
  } else {
    for (const d of (env.PATH ?? "/usr/bin").split(":")) for (const n of ["google-chrome", "google-chrome-stable", "chromium", "microsoft-edge"]) out.push(join(d, n));
  }
  return out.find((p) => existsSync(p)) ?? null;
}

const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".wasm": "application/wasm", ".json": "application/json", ".png": "image/png", ".svg": "image/svg+xml", ".ico": "image/x-icon" };

/** Static file server for the built client on 127.0.0.1 (port 0). */
function serve(root) {
  const server = createServer((req, res) => {
    const path = decodeURIComponent((req.url ?? "/").split("?")[0] ?? "/");
    let file = normalize(join(root, path === "/" ? "index.html" : path));
    if (!file.startsWith(normalize(root) + sep) && file !== normalize(join(root, "index.html"))) {
      res.writeHead(403).end();
      return;
    }
    if (!existsSync(file) || statSync(file).isDirectory()) file = join(root, "index.html");
    res.writeHead(200, { "content-type": TYPES[extname(file)] ?? "application/octet-stream" });
    createReadStream(file).pipe(res);
  });
  return new Promise((res) => server.listen(0, "127.0.0.1", () => res(server)));
}

/** Shots: name, query (without models=legacy). */
const SHOTS = [
  ["ship-bay", "view=ships"],
  ["ship-bay-boost", "view=ships&boost=1"],
  ["wrecks", "view=ships&show=wrecks"],
  ["props", "view=ships&show=props"],
  ["in-flight", "view=ships&scene=flight"],
  ["closeup-skitter", "view=ships&focus=0"],
  ["closeup-tinker", "view=ships&focus=1"],
  ["closeup-hauler", "view=ships&focus=2&boost=1"],
  ["closeup-tug", "view=ships&focus=3"],
  ["closeup-tinker-wreck", "view=ships&show=wrecks&focus=1"],
];
/** Shots without a "before" (the primitives have no boost look and one shared dart). */
const AFTER_ONLY = new Set(["ship-bay-boost", "closeup-skitter", "closeup-hauler", "closeup-tug", "closeup-tinker-wreck"]);

async function main() {
  const o = parseArgs(process.argv.slice(2));
  const browserPath = findBrowser(o.browser);
  if (!browserPath) {
    console.log("models: no Chrome/Edge found; skipped");
    return o.require ? 1 : 0;
  }
  if (!existsSync(join(o.webRoot, "index.html"))) {
    console.error(`models: ${o.webRoot} has no index.html (cd web && npm run build)`);
    return 1;
  }
  mkdirSync(o.outDir, { recursive: true });
  const server = await serve(o.webRoot);
  const base = `http://127.0.0.1:${server.address().port}`;
  const browser = await puppeteer.launch({
    executablePath: browserPath,
    headless: !o.headful,
    args: ["--enable-unsafe-webgpu", "--ignore-gpu-blocklist", "--no-first-run", "--no-default-browser-check", "--window-size=1280,760"],
    defaultViewport: { width: 1280, height: 720 },
    protocolTimeout: 120_000,
  });
  const errors = [];
  const files = [];
  try {
    for (const variant of o.only ? [o.only] : ["before", "after"]) {
      for (const [name, query] of SHOTS) {
        if (variant === "before" && AFTER_ONLY.has(name)) continue;
        const page = await browser.newPage();
        page.on("console", (m) => {
          if (m.type() === "error" || /WebGPU/.test(m.text())) errors.push(`${variant}/${name}: ${m.text()}`);
        });
        page.on("pageerror", (e) => errors.push(`${variant}/${name}: ${e.message}`));
        const url = `${base}/?${query}${variant === "before" ? "&models=legacy" : ""}`;
        await page.goto(url, { waitUntil: "load" });
        await page.waitForFunction(() => document.body.dataset.showcase === "ready", { timeout: 60_000 }).catch(() => errors.push(`${variant}/${name}: never ready`));
        await new Promise((r) => setTimeout(r, 1500));
        const file = join(o.outDir, `${name}-${variant}.png`);
        await page.screenshot({ path: file, type: "png" });
        files.push(file);
        await page.close();
      }
    }
  } finally {
    await browser.close().catch(() => {});
    server.close();
  }
  // Object cost (8 ships + 200 props + 8 wrecks) at 1920x1080, before and after.
  const benches = {};
  {
    const b2 = await puppeteer.launch({
      executablePath: browserPath,
      headless: !o.headful,
      args: ["--enable-unsafe-webgpu", "--ignore-gpu-blocklist", "--no-first-run", "--no-default-browser-check", "--window-size=1920,1120"],
      defaultViewport: { width: 1920, height: 1080 },
      protocolTimeout: 120_000,
    });
    const server2 = await serve(o.webRoot);
    const base2 = `http://127.0.0.1:${server2.address().port}`;
    try {
      for (const variant of ["before", "after"]) {
        const page = await b2.newPage();
        page.on("pageerror", (e) => errors.push(`bench/${variant}: ${e.message}`));
        await page.goto(`${base2}/?view=ships&bench=1${variant === "before" ? "&models=legacy" : ""}`, { waitUntil: "load" });
        await page.waitForFunction(() => document.body.dataset.showcase === "ready", { timeout: 120_000 }).catch(() => errors.push(`bench/${variant}: never ready`));
        benches[variant] = JSON.parse((await page.evaluate(() => document.body.dataset.bench)) ?? "null");
        await page.close();
      }
    } finally {
      await b2.close().catch(() => {});
      server2.close();
    }
    console.log(`models: bench ${JSON.stringify(benches)}`);
  }
  writeFileSync(join(o.outDir, "summary.json"), JSON.stringify({ files, errors, benches }, null, 2));
  for (const f of files) console.log(`models: ${f}`);
  for (const e of errors) console.error(`models: ERROR ${e}`);
  return errors.length ? 1 : 0;
}

main().then(
  (code) => process.exit(code),
  (err) => {
    console.error(err);
    process.exit(1);
  },
);
