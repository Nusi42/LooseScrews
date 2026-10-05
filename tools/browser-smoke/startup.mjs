#!/usr/bin/env node
// Start-screen timing and screenshots (first-impression pass, FI): serves a
// built client on 127.0.0.1, loads the start page in a headless Chrome/Edge
// with WebGPU (fake pointer lock injected) several times and measures:
//
//   shown    navigation start -> the start card is in the DOM (ms)
//   tti      time to interactive: the first moment after `shown` that is
//            followed by 5 s without a long task (> 50 ms main-thread task),
//            observed for --window ms (Lighthouse's definition, from `shown`)
//   tbt      total blocking time after `shown` (sum of long-task time over 50 ms)
//   longest  the longest task after `shown` (ms)
//   backdrop navigation start -> the first frame of the live backdrop
//            (performance mark `ls:backdrop-frame`; absent before FI)
//   fps      backdrop frames per second over the last 3 s of the window
//   + the page's `ls:*` performance measures (renderer creation, backdrop world, ...)
//
// Prints the median of each over the runs, writes `startup.json` and a
// screenshot of the last run (`start.png`) to <out-dir>/<label>/.
//
//   node tools/browser-smoke/startup.mjs [--web-root web/dist] [--label current]
//        [--out-dir target/startup] [--runs 5] [--window 12000] [--size 1920x1080]
//        [--query k=v&...] [--browser <path>]
//
// Headless only (a visible window would take the desktop mouse). Needs
// `npm ci` in tools/browser-smoke and a built client (`cd web && npm run build`).

import { createReadStream, existsSync, mkdirSync, statSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { dirname, extname, join, normalize, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { fakePointerLock } from "./fake-pointer-lock.mjs";
import { assertWorkFolder } from "../lib/work-folder.mjs";

assertWorkFolder("tools/browser-smoke/startup.mjs");

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "..", "..");

function parseArgs(argv) {
  const o = {
    webRoot: join(repo, "web", "dist"),
    outDir: join(repo, "target", "startup"),
    label: "current",
    runs: 5,
    window: 12000,
    width: 1920,
    height: 1080,
    query: "",
    browser: process.env.LS_BROWSER ?? "",
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => argv[++i] ?? "";
    if (a === "--web-root") o.webRoot = resolve(val());
    else if (a === "--out-dir") o.outDir = resolve(val());
    else if (a === "--label") o.label = val();
    else if (a === "--runs") o.runs = Math.max(1, Number(val()) | 0);
    else if (a === "--window") o.window = Math.max(6000, Number(val()) | 0);
    else if (a === "--query") o.query = val();
    else if (a === "--browser") o.browser = val();
    else if (a === "--size") {
      const m = /^(\d+)x(\d+)$/.exec(val());
      if (!m) die("--size must be WxH");
      o.width = Number(m[1]);
      o.height = Number(m[2]);
    } else if (a === "--help" || a === "-h") {
      console.log("node tools/browser-smoke/startup.mjs [--web-root dir] [--label name] [--out-dir dir] [--runs n] [--window ms] [--size WxH] [--query k=v&...] [--browser path]");
      process.exit(0);
    } else die(`unknown option ${a}`);
  }
  if (!/^[\w.-]+$/.test(o.label)) die("--label may only use letters, digits, '.', '_' and '-'");
  if (o.query && !/^[\w.=&-]+$/.test(o.query)) die("--query may only use letters, digits and . = & _ -");
  return o;
}

function die(msg) {
  console.error(`startup: ${msg}`);
  process.exit(2);
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

const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".wasm": "application/wasm", ".json": "application/json", ".png": "image/png", ".svg": "image/svg+xml", ".woff2": "font/woff2" };

/** Static file server for the built client on 127.0.0.1 (port 0); unknown API paths answer 404. */
function serve(root) {
  const server = createServer((req, res) => {
    const path = decodeURIComponent((req.url ?? "/").split("?")[0] ?? "/");
    let file = normalize(join(root, path === "/" ? "index.html" : path));
    if (!file.startsWith(normalize(root) + sep)) {
      res.writeHead(403).end();
      return;
    }
    if (!existsSync(file) || statSync(file).isDirectory()) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, { "content-type": TYPES[extname(file)] ?? "application/octet-stream", "cache-control": "no-store" });
    createReadStream(file).pipe(res);
  });
  return new Promise((res) => server.listen(0, "127.0.0.1", () => res(server)));
}

/** Page script: records long tasks, the start card's appearance and animation frames. */
function probe() {
  const w = /** @type {any} */ (window);
  w.__startup = { longTasks: [], shown: null, frames: [] };
  try {
    new PerformanceObserver((list) => {
      for (const e of list.getEntries()) w.__startup.longTasks.push([e.startTime, e.duration]);
    }).observe({ type: "longtask", buffered: true });
  } catch {
    /* no long-task timing */
  }
  const look = () => {
    if (w.__startup.shown === null && document.querySelector(".start-card")) w.__startup.shown = performance.now();
  };
  new MutationObserver(look).observe(document, { childList: true, subtree: true });
}

export function median(xs) {
  const s = xs.filter((x) => typeof x === "number" && Number.isFinite(x)).sort((a, b) => a - b);
  if (s.length === 0) return null;
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
}

/** TTI from `shown`: the end of the last long task that starts less than `quiet` ms after the previous candidate. */
export function timeToInteractive(shown, longTasks, quiet = 5000) {
  let tti = shown;
  for (const [start, dur] of [...longTasks].filter(([s, d]) => s + d > shown).sort((a, b) => a[0] - b[0])) {
    if (start - tti >= quiet) break;
    tti = Math.max(tti, start + dur);
  }
  return tti;
}

async function main() {
  const o = parseArgs(process.argv.slice(2));
  if (!existsSync(join(o.webRoot, "index.html"))) die(`${o.webRoot}/index.html missing (cd web && npm run build)`);
  const browserPath = findBrowser(o.browser);
  if (!browserPath) die("no Chrome or Edge found (set LS_BROWSER or pass --browser <path>)");
  const puppeteer = (await import("puppeteer-core")).default;
  const server = await serve(o.webRoot);
  const base = `http://127.0.0.1:${server.address().port}`;
  const dir = join(o.outDir, o.label);
  mkdirSync(dir, { recursive: true });
  const browser = await puppeteer.launch({
    executablePath: browserPath,
    headless: true, // never headful: a visible window takes the desktop mouse
    args: ["--enable-unsafe-webgpu", "--ignore-gpu-blocklist", "--no-first-run", "--no-default-browser-check", `--window-size=${o.width},${o.height}`],
    defaultViewport: null,
  });
  const runs = [];
  let failed = false;
  try {
    for (let i = 0; i < o.runs; i++) {
      const ctx = await browser.createBrowserContext();
      const page = await ctx.newPage();
      await page.evaluateOnNewDocument(fakePointerLock); // never lock the OS cursor
      await page.evaluateOnNewDocument(probe);
      await page.setViewport({ width: o.width, height: o.height, deviceScaleFactor: 1 });
      const errors = [];
      page.on("pageerror", (e) => errors.push(String(e?.message ?? e)));
      page.on("console", (m) => {
        if (m.type() === "error") errors.push(m.text());
      });
      await page.goto(`${base}/${o.query ? `?${o.query}` : ""}`, { waitUntil: "load" });
      await page.waitForSelector(".start-card", { timeout: 30_000 });
      const shownAt = await page.evaluate(() => window.__startup.shown);
      await new Promise((r) => setTimeout(r, Math.max(0, o.window - shownAt)));
      const r = await page.evaluate(() => {
        const s = window.__startup;
        const mark = (n) => performance.getEntriesByName(n)[0]?.startTime ?? null;
        return { shown: s.shown, longTasks: s.longTasks, backdrop: mark("ls:backdrop-frame"), measures: Object.fromEntries(performance.getEntriesByType("measure").filter((m) => m.name.startsWith("ls:")).map((m) => [m.name.slice(3), Math.round(m.duration)])), frames: Number(document.body.dataset.backdropFrames ?? NaN), now: performance.now() };
      });
      // Backdrop frame rate over the last 3 s.
      const f0 = r.frames;
      const t0 = Date.now();
      await new Promise((res) => setTimeout(res, 3000));
      const f1 = await page.evaluate(() => Number(document.body.dataset.backdropFrames ?? NaN));
      const fps = Number.isFinite(f0) && Number.isFinite(f1) ? ((f1 - f0) * 1000) / (Date.now() - t0) : null;
      const after = r.longTasks.filter(([s, d]) => s + d > r.shown && s < r.shown + o.window);
      const run = {
        shown: r.shown,
        tti: timeToInteractive(r.shown, after),
        tbt: after.reduce((sum, [, d]) => sum + Math.max(0, d - 50), 0),
        longest: after.reduce((m, [, d]) => Math.max(m, d), 0),
        longTasks: after.length,
        backdrop: r.backdrop,
        fps,
        measures: r.measures,
        errors,
      };
      runs.push(run);
      if (errors.length) failed = true;
      console.log(
        `run ${i + 1}: shown ${run.shown.toFixed(0)} ms · tti ${run.tti.toFixed(0)} ms · tbt ${run.tbt.toFixed(0)} ms · longest ${run.longest.toFixed(0)} ms (${run.longTasks} long tasks)` +
          `${run.backdrop !== null ? ` · backdrop ${run.backdrop.toFixed(0)} ms` : ""}${run.fps !== null ? ` · ${run.fps.toFixed(1)} fps` : ""}${Object.keys(run.measures).length ? ` · ${Object.entries(run.measures).map(([k, v]) => `${k} ${v} ms`).join(" · ")}` : ""}${errors.length ? ` · ERRORS: ${errors.slice(0, 2).join(" | ")}` : ""}`,
      );
      if (i === o.runs - 1) await page.screenshot({ path: join(dir, "start.png") });
      await ctx.close();
    }
  } finally {
    await browser.close().catch(() => {});
    server.close();
  }
  const keys = ["shown", "tti", "tbt", "longest", "backdrop", "fps"];
  const med = Object.fromEntries(keys.map((k) => [k, median(runs.map((r) => r[k]))]));
  console.log(`median: ${keys.map((k) => `${k} ${med[k] === null ? "-" : med[k].toFixed(k === "fps" ? 1 : 0)}`).join(" · ")}`);
  writeFileSync(join(dir, "startup.json"), `${JSON.stringify({ label: o.label, date: new Date().toISOString(), size: `${o.width}x${o.height}`, median: med, runs }, null, 2)}\n`);
  console.log(`      ${join(dir, "start.png")}`);
  return failed ? 1 : 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().then(
    (code) => process.exit(code),
    (err) => {
      console.error(err);
      process.exit(1);
    },
  );
}
