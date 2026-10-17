#!/usr/bin/env node
// Visual gallery (docs/visual-quality-plan.md, VQ8): renders the fixed scenes
// of web/src/gallery/scenes.ts (cave close-up, cave tunnel, open sky, mine
// shaft, material border, paint edges, surface states, the four hulls, a
// fight frame with an explosion) in a locally
// installed Chrome or Edge with WebGPU, saves one screenshot per scene under
// <out-dir>/<label>/, records the per-pass GPU times (timestamp-query) and
// checks them against the frame budget, then writes <out-dir>/index.html
// with every label side by side per scene (before/after).
//
//   node tools/browser-smoke/gallery.mjs [options]
//
// Options (defaults in brackets):
//   --label <name>        folder for this run's shots [current]
//   --compare <a,b,...>   labels (in order) the index shows [every label, oldest first]
//   --scenes <id,...>     scenes to render [all]
//   --web-root <dir>      built web client [web/dist]
//   --out-dir <dir>       [target/visual-gallery]
//   --size <WxH>          viewport in CSS pixels at scale 1 [1920x1080]
//   --browser <path>      Chrome/Edge executable [$LS_BROWSER, else searched]
//   --mode auto|headless|headful
//                         auto = headless (no automatic headful fallback:
//                         a visible window takes the desktop mouse) [auto]
//   --power high|low      GPU on two-GPU machines (low = integrated, ?gpu=low-power) [high]
//   --adapter-luid <h,l>  pin Chrome's GPU (--use-adapter-luid; chrome://gpu lists the
//                         LUIDs). With --power low and no LUID, an integrated GPU that
//                         Chrome does not hand out by power preference (it keeps WebGPU
//                         on the GPU process's GPU) is looked up in chrome://gpu [none]
//   --quality <level>     render preset high|medium|low (?quality=; no automatic pick) [high]
//   --grade <preset>      colour grade from post.ts GRADE_PRESETS (?grade=) [none]
//   --lights on|off       dynamic lights (VQ6; off = ?lights=0, the same frame unlit, for cost comparisons) [on]
//   --vfx on|off          particle effects (VQ5); off draws the pre-VQ5 mesh effects (?vfx=off) [on]
//   --query <k=v&...>     extra page parameters, e.g. view=legacy (the pre-VQ1
//                         camera: 75° vertical FOV, no cockpit frame or fit ring) [none]
//   --budget-scale <x>    multiplies every budget (1 = reference mid GPU at
//                         "high"; 1.75 = integrated GPU at "medium") [1]
//   --frames <n>          timed frames averaged per scene [120]
//   --no-budget           record the GPU times but do not fail on them
//   --no-atmosphere       no per-world sky, fog and grade (?atmosphere=off;
//                         the pre-VQ7 look, for before shots of new scenes)
//   --index-only          only rewrite index.html from the existing labels
//
// Fails (exit 1) on a scene that does not render (error, console error,
// WebGPU error), or, unless --no-budget, on a pass over its budget share by
// more than 25 % (PASS_BUDGETS_MS / BUDGET_TOLERANCE in scenes.ts). Without
// timestamp-query, or on a software adapter, the budget is not checked (a
// note says so). The page is served by a static file server on 127.0.0.1;
// no game server is needed (the scenes run offline).

import { createReadStream, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { fakePointerLock } from "./fake-pointer-lock.mjs";
import { findBrowser, headerHelp, repoRoot, sleep, webgpuArgs } from "./harness.mjs";
import { execFileSync } from "node:child_process";

import { assertWorkFolder } from "../lib/work-folder.mjs";
import { checkBudget, FRAME_BUDGET_MS, PASS_BUDGETS_MS, SCENES } from "../../web/src/gallery/scenes.ts";

assertWorkFolder("tools/browser-smoke/gallery.mjs");


const defaults = {
  label: "current",
  compare: "",
  scenes: "",
  "web-root": join(repoRoot, "web", "dist"),
  "out-dir": join(repoRoot, "target", "visual-gallery"),
  size: "1920x1080",
  browser: process.env.LS_BROWSER ?? "",
  mode: "auto",
  power: "high",
  "adapter-luid": "",
  quality: "high",
  grade: "",
  lights: "on",
  vfx: "on",
  query: "",
  "budget-scale": "1",
  frames: "120",
};

function die(msg) {
  console.error(`gallery: ${msg}`);
  process.exit(2);
}

function parseArgs(argv) {
  const o = { ...defaults, "no-budget": false, "index-only": false, "no-atmosphere": false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--help" || a === "-h") {
      console.log(headerHelp(import.meta.url));
      process.exit(0);
    }
    const key = a.replace(/^--/, "");
    if (key === "no-budget" || key === "index-only" || key === "no-atmosphere") {
      o[key] = true;
      continue;
    }
    if (!a.startsWith("--") || !(key in defaults)) die(`unknown argument ${a}`);
    const v = argv[++i];
    if (v === undefined) die(`missing value for ${a}`);
    o[key] = v;
  }
  const size = /^(\d+)x(\d+)$/.exec(o.size);
  if (!size) die("--size must be WxH");
  if (!["auto", "headless", "headful"].includes(o.mode)) die("--mode must be auto, headless or headful");
  if (!["high", "low"].includes(o.power)) die("--power must be high or low");
  if (o["adapter-luid"] && !/^-?\d+,\d+$/.test(o["adapter-luid"])) die("--adapter-luid must be <high>,<low> (decimal, as chrome://gpu shows it)");
  if (!["high", "medium", "low"].includes(o.quality)) die("--quality must be high, medium or low");
  if (!["on", "off"].includes(o.lights)) die("--lights must be on or off");
  if (!["on", "off"].includes(o.vfx)) die("--vfx must be on or off");
  if (o.query && !/^[\w.=&-]+$/.test(o.query)) die("--query may only use letters, digits and . = & _ -");
  if (!/^[\w.-]+$/.test(o.label)) die("--label may only use letters, digits, '.', '_' and '-'");
  const ids = o.scenes ? o.scenes.split(",") : SCENES.map((s) => s.id);
  for (const id of ids) if (!SCENES.some((s) => s.id === id)) die(`unknown scene ${id} (${SCENES.map((s) => s.id).join(", ")})`);
  const scale = Number(o["budget-scale"]);
  if (!(scale > 0)) die("--budget-scale must be a positive number");
  const frames = Number(o.frames);
  if (!(frames >= 1 && frames <= 600)) die("--frames must be 1..600");
  return {
    label: o.label,
    compare: o.compare ? o.compare.split(",") : null,
    scenes: ids,
    webRoot: resolve(o["web-root"]),
    outDir: resolve(o["out-dir"]),
    width: Number(size[1]),
    height: Number(size[2]),
    browser: o.browser ? resolve(o.browser) : "",
    mode: o.mode,
    power: o.power,
    adapterLuid: o["adapter-luid"],
    quality: o.quality,
    grade: o.grade,
    lights: o.lights,
    vfx: o.vfx,
    query: o.query,
    budgetScale: scale,
    frames,
    noBudget: o["no-budget"],
    noAtmosphere: o["no-atmosphere"],
    indexOnly: o["index-only"],
  };
}

/**
 * The LUID (`high,low`) of the integrated GPU in chrome://gpu: the name after
 * Dawn's "<Integrated GPU> D3D12 backend - " header, matched to the "GPUn"
 * line that carries its LUID. Null if there is none.
 */
export function integratedLuidFromGpuPage(text) {
  const t = text.replace(/\s+/g, " ");
  const name = /<Integrated GPU> D3D12 backend - (.+?) -{3,}/.exec(t)?.[1]?.trim();
  if (!name) return null;
  for (const m of t.matchAll(/GPU\d+ : VENDOR= ?0x[0-9a-f]+, DEVICE=0x[0-9a-f]+ \[([^\]]+)\][^{]*LUID=\{(-?\d+),(\d+)\}/gi)) {
    if (m[1].trim() === name) return `${m[2]},${m[3]}`;
  }
  return null;
}

async function integratedLuid(browser) {
  const page = await browser.newPage();
  await page.evaluateOnNewDocument(fakePointerLock); // never lock the OS cursor
  try {
    await page.goto("chrome://gpu", { waitUntil: "load" });
    // The page fills in its sections after load.
    for (let i = 0; i < 60; i++) {
      const text = await page.evaluate(() => {
        const walk = (n) => (n.shadowRoot ? walk(n.shadowRoot) : "") + [...n.childNodes].map((c) => (c.nodeType === 3 ? c.textContent : walk(c))).join(" ");
        return walk(document.body);
      });
      const luid = integratedLuidFromGpuPage(text);
      if (luid || /D3D12 backend/.test(text)) return luid;
      await new Promise((r) => setTimeout(r, 250));
    }
    return null;
  } catch {
    return null;
  } finally {
    await page.close();
  }
}

// ----- static file server ----------------------------------------------------------------

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
};

function serve(root) {
  const server = createServer((req, res) => {
    const path = decodeURIComponent(new URL(req.url ?? "/", "http://x").pathname);
    if (path === "/__probe") {
      // A blank page (secure context on 127.0.0.1) for the WebGPU adapter probe.
      res.writeHead(200, { "content-type": "text/html; charset=utf-8" }).end("<!doctype html><title>probe</title>");
      return;
    }
    let file = resolve(root, `.${path}`);
    if (file !== root && !file.startsWith(root + sep)) {
      res.writeHead(403).end();
      return;
    }
    if (existsSync(file) && statSync(file).isDirectory()) file = join(file, "index.html");
    if (!existsSync(file)) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, { "content-type": MIME[extname(file)] ?? "application/octet-stream", "cache-control": "no-store" });
    createReadStream(file).pipe(res);
  });
  return new Promise((res, rej) => {
    server.once("error", rej);
    server.listen(0, "127.0.0.1", () => res({ server, base: `http://127.0.0.1:${server.address().port}` }));
  });
}

// ----- index ---------------------------------------------------------------------------

const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

function readManifests(outDir) {
  if (!existsSync(outDir)) return [];
  return readdirSync(outDir)
    .map((d) => join(outDir, d, "manifest.json"))
    .filter((f) => existsSync(f))
    .map((f) => JSON.parse(readFileSync(f, "utf8")))
    .sort((a, b) => (a.date < b.date ? -1 : a.date > b.date ? 1 : 0));
}

function timingText(t) {
  if (!t?.gpu) return t?.canTime === false ? "no timestamp-query" : "no GPU timing";
  const passes = t.gpu.passes.map(([l, ms]) => `${l} ${ms.toFixed(2)}`).join(" · ");
  return `GPU ${t.gpu.total.toFixed(2)} ms (worst ${t.gpu.worst.toFixed(2)}) · ${passes}`;
}

export function writeIndex(outDir, compare) {
  let runs = readManifests(outDir);
  if (compare) runs = compare.map((l) => runs.find((r) => r.label === l)).filter(Boolean);
  const sceneIds = SCENES.map((s) => s.id).filter((id) => runs.some((r) => r.scenes.some((s) => s.id === id)));
  const head = runs
    .map(
      (r) =>
        `<th>${esc(r.label)}<br><small>${esc(r.date.slice(0, 16).replace("T", " "))} · ${esc(r.git)}<br>${esc(r.adapter)}</small></th>`,
    )
    .join("");
  const rows = sceneIds
    .map((id) => {
      const def = SCENES.find((s) => s.id === id);
      const cells = runs
        .map((r) => {
          const s = r.scenes.find((x) => x.id === id);
          if (!s) return "<td></td>";
          const img = `${r.label}/${id}.png`;
          const budget = s.budget ? (s.budget.ok ? "within budget" : `<b class="bad">over budget: ${esc(s.budget.failures.join("; "))}</b>`) : "";
          return `<td><a href="${esc(img)}"><img src="${esc(img)}" loading="lazy"></a><div class="t">${esc(timingText(s))}<br>${budget}</div></td>`;
        })
        .join("");
      return `<tr><th class="scene">${esc(def?.title ?? id)}<br><small>${esc(id)} · ${esc(def?.covers ?? "")}</small></th>${cells}</tr>`;
    })
    .join("\n");
  const budgets = Object.entries(PASS_BUDGETS_MS)
    .map(([k, v]) => `${k} ${v}`)
    .join(" · ");
  const html = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Visual gallery</title>
<style>
:root { color-scheme: dark; }
body { background: #111318; color: #d8dbe2; font: 14px/1.4 system-ui, sans-serif; margin: 16px; }
table { border-collapse: collapse; }
th, td { border: 1px solid #2a2e38; padding: 6px; vertical-align: top; text-align: left; }
img { width: 640px; max-width: 45vw; display: block; }
.t { font-size: 12px; color: #9aa1b0; max-width: 640px; }
.bad { color: #ff7a6b; }
th.scene { width: 160px; }
small { color: #9aa1b0; font-weight: normal; }
</style></head><body>
<h1>Visual gallery</h1>
<p>Fixed poses per scene (web/src/gallery/scenes.ts); columns are runs (labels), oldest first. GPU budget per pass at 1080p "high" on the reference mid GPU (ms): ${esc(budgets)} · frame ${FRAME_BUDGET_MS}; a pass fails above 125 %.</p>
<table><tr><th></th>${head}</tr>
${rows}
</table></body></html>
`;
  writeFileSync(join(outDir, "index.html"), html);
  return join(outDir, "index.html");
}

// ----- main ----------------------------------------------------------------------------

function gitRev() {
  try {
    const rev = execFileSync("git", ["rev-parse", "--short", "HEAD"], { cwd: repoRoot }).toString().trim();
    const dirty = execFileSync("git", ["status", "--porcelain", "--untracked-files=no"], { cwd: repoRoot }).toString().trim();
    return dirty ? `${rev}+dirty` : rev;
  } catch {
    return "unknown";
  }
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  mkdirSync(opts.outDir, { recursive: true });
  if (opts.indexOnly) {
    console.log(`index: ${writeIndex(opts.outDir, opts.compare)}`);
    return 0;
  }
  const browserPath = findBrowser(opts.browser);
  if (!browserPath) {
    console.error("gallery: no Chrome or Edge found (set LS_BROWSER or pass --browser <path>)");
    return 1;
  }
  if (!existsSync(join(opts.webRoot, "index.html"))) die(`${opts.webRoot}/index.html missing (cd web && npm run build)`);
  let puppeteer;
  try {
    puppeteer = (await import("puppeteer-core")).default;
  } catch {
    die("puppeteer-core is not installed (cd tools/browser-smoke && npm ci)");
  }

  const { server, base } = await serve(opts.webRoot);
  const labelDir = join(opts.outDir, opts.label);
  mkdirSync(labelDir, { recursive: true });
  const power = `&quality=${opts.quality}${opts.grade ? `&grade=${encodeURIComponent(opts.grade)}` : ""}${opts.power === "low" ? "&gpu=low-power" : ""}${opts.lights === "off" ? "&lights=0" : ""}${opts.vfx === "off" ? "&vfx=off" : ""}${opts.noAtmosphere ? "&atmosphere=off" : ""}${opts.query ? `&${opts.query}` : ""}`;
  const launch = (headless, luid = opts.adapterLuid) =>
    puppeteer.launch({
      executablePath: browserPath,
      headless,
      args: webgpuArgs({
        width: opts.width,
        height: opts.height + 120,
        extra: [
          // Unquantised timestamps (Chrome rounds them to 100 µs otherwise).
          "--enable-webgpu-developer-features",
          // Chrome on Windows ignores requestAdapter's powerPreference by default.
          ...(opts.power === "low" ? ["--use-webgpu-power-preference=force-low-power"] : []),
          // Chrome hands WebGPU only the adapter of its GPU process; this picks that GPU.
          ...(luid ? [`--use-adapter-luid=${luid}`] : []),
        ],
      }),
      defaultViewport: null,
      protocolTimeout: 180_000,
    });

  let browser = null;
  let failed = false;
  const manifest = {
    label: opts.label,
    date: new Date().toISOString(),
    git: gitRev(),
    browser: browserPath,
    mode: null,
    adapter: "",
    size: `${opts.width}x${opts.height}`,
    power: opts.power,
    quality: opts.quality,
    budgetScale: opts.budgetScale,
    scenes: [],
  };
  try {
    let adapter = null;
    for (const headless of [opts.mode !== "headful"] /* no automatic headful fallback: a visible window takes the desktop mouse */) {
      browser = await launch(headless);
      const page = await browser.newPage();
      await page.evaluateOnNewDocument(fakePointerLock); // never lock the OS cursor
      await page.goto(`${base}/__probe`, { waitUntil: "load" });
      adapter = await page.evaluate(async (pref) => {
        if (!navigator.gpu) return null;
        const a = await navigator.gpu.requestAdapter({ powerPreference: pref });
        if (!a) return null;
        const i = a.info ?? {};
        return {
          text: `${i.vendor ?? ""} ${i.architecture ?? ""} ${i.description ?? ""}`.trim(),
          fallback: Boolean(i.isFallbackAdapter ?? a.isFallbackAdapter),
          timestamps: a.features.has("timestamp-query"),
        };
      }, opts.power === "low" ? "low-power" : "high-performance");
      await page.close();
      manifest.mode = headless ? "headless" : "headful";
      if (adapter) break;
      await browser.close();
      browser = null;
    }
    if (!adapter && opts.power === "low" && !opts.adapterLuid && opts.mode !== "headful") {
      // M4 D5: the integrated GPU is listed (Dawn info) but not handed out by
      // power preference; pin it by its LUID from chrome://gpu.
      const probe = await launch(true);
      const luid = await integratedLuid(probe);
      await probe.close();
      if (luid) {
        console.log(`gallery: integrated GPU by LUID ${luid} (--use-adapter-luid)`);
        browser = await launch(true, luid);
        const page = await browser.newPage();
        await page.evaluateOnNewDocument(fakePointerLock); // never lock the OS cursor
        await page.goto(`${base}/__probe`, { waitUntil: "load" });
        adapter = await page.evaluate(async () => {
          const a = await navigator.gpu?.requestAdapter({ powerPreference: "low-power" });
          if (!a) return null;
          const i = a.info ?? {};
          return { text: `${i.vendor ?? ""} ${i.architecture ?? ""} ${i.description ?? ""}`.trim(), fallback: Boolean(i.isFallbackAdapter ?? a.isFallbackAdapter), timestamps: a.features.has("timestamp-query") };
        });
        await page.close();
        manifest.mode = "headless";
        manifest.adapterLuid = luid;
        if (!adapter) {
          await browser.close();
          browser = null;
        }
      }
    }
    if (!adapter) {
      console.error("gallery: FAIL - no WebGPU adapter (headless or headful)");
      return 1;
    }
    manifest.adapter = `${adapter.text}${adapter.fallback ? " (fallback)" : ""}`;
    console.log(`gallery: ${manifest.mode} ${await browser.version()} · adapter ${manifest.adapter} · timestamp-query ${adapter.timestamps ? "yes" : "no"}`);
    const budgetChecked = !opts.noBudget && adapter.timestamps && !adapter.fallback;
    if (!budgetChecked && !opts.noBudget) console.log("      note: GPU budget not checked (no timestamp-query or a software adapter)");

    for (const id of opts.scenes) {
      const page = await browser.newPage();
      await page.evaluateOnNewDocument(fakePointerLock); // never lock the OS cursor
      await page.setViewport({ width: opts.width, height: opts.height, deviceScaleFactor: 1 });
      const problems = [];
      page.on("console", (m) => {
        if (m.type() === "error" || (m.type() === "warning" && /webgpu|wgsl|shader|pipeline/i.test(m.text()))) problems.push(`[${m.type()}] ${m.text()}`);
      });
      page.on("pageerror", (e) => problems.push(`[pageerror] ${e?.message ?? e}`));
      const t0 = Date.now();
      await page.goto(`${base}/?gallery=${id}${power}`, { waitUntil: "load" });
      const ready = await page
        .waitForFunction(() => window.__gallery && window.__gallery.phase !== "loading", { timeout: 180_000, polling: 100 })
        .then(() => true)
        .catch(() => false);
      let status = await page.evaluate(() => window.__gallery ?? null);
      if (ready && status?.phase === "ready" && status.canTime) {
        // Fill the timing window with this frame (the page keeps redrawing it).
        await page
          .waitForFunction((n) => (window.__gallery?.gpu?.frames ?? 0) >= n, { timeout: 60_000, polling: 100 }, Math.min(opts.frames, 60))
          .catch(() => {});
        await sleep(Math.max(0, (opts.frames - 60) * 17));
        status = await page.evaluate(() => window.__gallery ?? null);
      }
      const file = join(labelDir, `${id}.png`);
      if (status?.phase === "ready") await page.screenshot({ path: file, type: "png" });
      const entry = {
        id,
        shareCode: status?.shareCode ?? "",
        camera: status?.camera ?? null,
        chunksDrawn: status?.chunksDrawn ?? 0,
        triangles: status?.triangles ?? 0,
        instances: status?.instances ?? 0,
        particles: status?.particles ?? null,
        canTime: status?.canTime ?? false,
        gpu: status?.gpu ?? null,
        budget: null,
        seconds: (Date.now() - t0) / 1000,
        // VQ1 scenes with a ship: the fit ring and its probes' CPU time per frame.
        ...(status?.fit ? { fit: status.fit, probeMs: status.probeMs } : {}),
      };
      const ok = ready && status?.phase === "ready" && !status.gpuError && problems.length === 0;
      if (!ok) {
        failed = true;
        const why = !ready ? "timed out" : status?.error ?? status?.gpuError ?? problems.slice(0, 3).join(" | ");
        console.log(`FAIL  ${id}: ${why}`);
      } else {
        if (budgetChecked && entry.gpu) {
          entry.budget = checkBudget(entry.gpu.passes, opts.budgetScale);
          if (!entry.budget.ok) failed = true;
        }
        const verdict = entry.budget ? (entry.budget.ok ? "within budget" : `OVER BUDGET: ${entry.budget.failures.join("; ")}`) : "";
        console.log(`${entry.budget && !entry.budget.ok ? "FAIL" : "PASS"}  ${id}: ${timingText(entry)} · ${Math.round(entry.triangles / 1000)}k tris${entry.particles ? ` · ${entry.particles} particles` : ""} ${verdict}`);
        if (entry.fit) {
          const f = entry.fit;
          console.log(`      fit ring ${f.visible ? `${f.state} ${f.radiusPx.toFixed(1)} px at ${f.depth.toFixed(2)} u` : "hidden"} · probes ${entry.probeMs?.toFixed(3)} ms CPU`);
        }
        console.log(`      ${relative(repoRoot, file)}`);
      }
      manifest.scenes.push(entry);
      await page.close();
    }
  } finally {
    await browser?.close().catch(() => {});
    server.close();
  }
  // A partial run (--scenes) keeps the label's other scenes.
  const manifestFile = join(labelDir, "manifest.json");
  if (existsSync(manifestFile)) {
    const old = JSON.parse(readFileSync(manifestFile, "utf8"));
    const fresh = new Set(manifest.scenes.map((s) => s.id));
    manifest.scenes = [...(old.scenes ?? []).filter((s) => !fresh.has(s.id)), ...manifest.scenes];
    manifest.scenes.sort((x, y) => SCENES.findIndex((s) => s.id === x.id) - SCENES.findIndex((s) => s.id === y.id));
  }
  writeFileSync(manifestFile, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`index: ${writeIndex(opts.outDir, opts.compare)}`);
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
