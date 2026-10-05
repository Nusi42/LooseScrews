#!/usr/bin/env node
// Browser smoke test: drives a locally installed Chrome or Edge (puppeteer-core)
// with WebGPU against a local ls-server serving the built client (web/dist).
// Called by tools/browser-smoke.ps1 / .sh after they built the client and the
// release server; can also be run directly (after `npm ci` in this directory).
//
//   node tools/browser-smoke/smoke.mjs --server-bin <ls-server> [options]
//
// Options (defaults in brackets):
//   --server-bin <path>     ls-server binary [$CARGO_TARGET_DIR/release/ls-server]
//   --web-root <dir>        served web client [web/dist]
//   --browser <path>        Chrome/Edge executable [$LS_BROWSER, else searched]
//   --out-dir <dir>         screenshots, logs, summary.json [target/browser-smoke]
//   --code <share code>     server world [LS0C-0000-0000-0000-8006-8001-8F9M]
//   --transport ws|wt|auto  browser transport (?transport=) [ws]
//   --mode auto|headless|headful
//                           auto = headless (no automatic headful fallback:
//                           a visible window takes the desktop mouse) [auto]
//   --practice-secs <s>     practice render time [5]
//   --fly-secs <s>          online flight time [10]
//   --mech-secs <s>         extra settle time in the mechanics phase [1]
//   --require-browser       fail (exit 1) instead of skipping (exit 0) when no
//                           Chrome/Edge is found
//   --allow-fallback        accept a software (fallback) WebGPU adapter
//   --find-browser          only print the browser that would be used (exit 0),
//                           or exit 3 when none is found
//
// Steps: probe WebGPU in the browser; start ls-server on a free port; open the
// page and start Practice (every WGSL module and render pipeline is created
// under an error scope and its compilation info checked); render for
// --practice-secs; reload, Play online, fly with synthetic keys and mouse for
// --fly-secs while firing the scrap laser, the Paint Thrower (cycling colours
// with V), the Bolt Rocket and the shield gadget; mechanics: a second
// ls-server with tools/browser-smoke/mechanics.ron (Fusion Charger, Leech Arc,
// Paint Thrower, Eyeball, Decoy; no Hauler): open the ship bay (B) and pick a
// ship, hold the Fusion Charger (charge meter), fire the Leech Arc (no
// predicted bolt), cycle the paint colour with V (HUD follows the server),
// launch the Eyeball (guided view), use the Decoy (cooldown shown); lobby: a third
// ls-server with the lobby on (tools/browser-smoke/lobby.ron, a 12 s match): the lobby card
// in warm-up, ready with Enter, the countdown, the match, the results screen and the next
// match's warm-up and match on the same page and connection (no reload, one join); the
// fit ring before a 1.2 u tunnel (?gallery=fit-tunnel, fit-tunnel.png); open the rule set editor
// (?editor=rules), change the rules, read the generated code and check that
// ls-server accepts it (--ruleset <code>). Fails on console errors, page
// errors, WebGPU validation or uncaptured errors, error screens, blank frames,
// no confirmed shots online, or server errors. Screenshots go to --out-dir.
// Exit codes: 0 passed (or skipped: no browser), 1 failed, 2 usage error,
// 3 no browser (--find-browser only).

import { spawn } from "node:child_process";
import { createWriteStream, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { fakePointerLock } from "./fake-pointer-lock.mjs";

import { assertWorkFolder } from "../lib/work-folder.mjs";

assertWorkFolder("tools/browser-smoke/smoke.mjs");

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..", "..");
const exe = process.platform === "win32" ? ".exe" : "";

// ----- arguments -------------------------------------------------------------------

function defaultTarget() {
  const t = process.env.CARGO_TARGET_DIR || "target";
  return resolve(repoRoot, t);
}

const defaults = {
  "server-bin": join(defaultTarget(), "release", `ls-server${exe}`),
  "web-root": join(repoRoot, "web", "dist"),
  browser: process.env.LS_BROWSER ?? "",
  "out-dir": join(repoRoot, "target", "browser-smoke"),
  code: "LS0C-0000-0000-0000-8006-8001-8F9M",
  transport: "ws",
  mode: "auto",
  "practice-secs": "5",
  "fly-secs": "10",
  "mech-secs": "1",
};

function die(msg) {
  console.error(`browser-smoke: ${msg}`);
  process.exit(2);
}

function parseArgs(argv) {
  const o = { ...defaults, "require-browser": false, "find-browser": false, "allow-fallback": false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--help" || a === "-h") {
      console.log(HELP_TEXT());
      process.exit(0);
    }
    if (a === "--find-browser") {
      o["find-browser"] = true;
      continue;
    }
    if (a === "--require-browser") {
      o["require-browser"] = true;
      continue;
    }
    if (a === "--allow-fallback") {
      o["allow-fallback"] = true;
      continue;
    }
    const key = a.replace(/^--/, "");
    if (!a.startsWith("--") || !(key in defaults)) die(`unknown argument ${a}`);
    const v = argv[++i];
    if (v === undefined) die(`missing value for ${a}`);
    o[key] = v;
  }
  if (!["ws", "wt", "auto"].includes(o.transport)) die("--transport must be ws, wt or auto");
  if (!["auto", "headless", "headful"].includes(o.mode)) die("--mode must be auto, headless or headful");
  const secs = (k) => {
    const n = Number(o[k]);
    if (!(n > 0)) die(`--${k} must be a positive number`);
    return n;
  };
  return {
    serverBin: resolve(o["server-bin"]),
    webRoot: resolve(o["web-root"]),
    browser: o.browser ? resolve(o.browser) : "",
    outDir: resolve(o["out-dir"]),
    code: o.code,
    transport: o.transport,
    mode: o.mode,
    practiceSecs: secs("practice-secs"),
    flySecs: secs("fly-secs"),
    mechSecs: secs("mech-secs"),
    requireBrowser: o["require-browser"],
    allowFallback: o["allow-fallback"],
    findOnly: o["find-browser"],
  };
}

function HELP_TEXT() {
  // The header comment of this file.
  const src = readFileSync(fileURLToPath(import.meta.url), "utf8").split(/\r?\n/).slice(1);
  const end = src.findIndex((l) => !l.startsWith("//"));
  return src.slice(0, end).map((l) => l.replace(/^\/\/ ?/, "")).join("\n");
}

// ----- browser discovery -----------------------------------------------------------

/** Installed Chrome/Edge candidates, most preferred first. */
export function browserCandidates(env = process.env, platform = process.platform) {
  if (platform === "win32") {
    const roots = [env.ProgramFiles, env["ProgramFiles(x86)"], env.LOCALAPPDATA].filter(Boolean);
    const out = [];
    for (const r of roots) out.push(join(r, "Google", "Chrome", "Application", "chrome.exe"));
    for (const r of roots) out.push(join(r, "Microsoft", "Edge", "Application", "msedge.exe"));
    return out;
  }
  if (platform === "darwin") {
    return [
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
      "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ];
  }
  const dirs = (env.PATH ?? "/usr/bin").split(":");
  const names = ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "microsoft-edge", "microsoft-edge-stable"];
  return names.flatMap((n) => dirs.map((d) => join(d, n)));
}

function findBrowser(explicit) {
  if (explicit) return existsSync(explicit) ? explicit : null;
  return browserCandidates().find((p) => existsSync(p)) ?? null;
}

// ----- processes -------------------------------------------------------------------

function freePort() {
  return new Promise((res, rej) => {
    const s = createServer();
    s.once("error", rej);
    s.listen(0, "127.0.0.1", () => {
      const { port } = s.address();
      s.close(() => res(port));
    });
  });
}

const ANSI = /\x1b\[[0-9;]*m/g;

/** Spawns `bin`, tees its output to `logFile` and collects the text. */
function run(bin, args, logFile) {
  const log = createWriteStream(logFile);
  const child = spawn(bin, args, {
    cwd: repoRoot,
    env: { ...process.env, NO_COLOR: "1", RUST_LOG: process.env.RUST_LOG ?? "info" },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  const proc = { child, text: "", listeners: [], exited: null };
  const onData = (buf) => {
    proc.text += buf.toString().replace(ANSI, "");
    log.write(buf);
    for (const f of proc.listeners) f();
  };
  child.stdout.on("data", onData);
  child.stderr.on("data", onData);
  proc.exited = new Promise((res) => {
    child.on("error", (e) => {
      proc.text += `\nspawn error: ${e.message}\n`;
      log.end();
      res({ code: -1 });
    });
    child.on("exit", (code, signal) =>
      setTimeout(() => {
        log.end();
        res({ code, signal });
      }, 100),
    );
  });
  return proc;
}

function waitForLine(proc, re, ms) {
  return new Promise((res) => {
    const check = () => {
      if (re.test(proc.text)) done(true);
    };
    const done = (v) => {
      clearTimeout(timer);
      proc.listeners = proc.listeners.filter((f) => f !== check);
      res(v);
    };
    const timer = setTimeout(() => done(false), ms);
    proc.listeners.push(check);
    proc.exited.then(() => done(re.test(proc.text)));
    check();
  });
}

async function stop(proc) {
  if (!proc || proc.child.exitCode !== null || proc.child.signalCode !== null) return;
  proc.child.kill();
  await Promise.race([proc.exited, sleep(5000)]);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ----- checks ------------------------------------------------------------------------

const results = [];
function check(name, ok, detail = "") {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? ` - ${detail}` : ""}`);
  return ok;
}
function note(text) {
  console.log(`      ${text}`);
}

// ----- page instrumentation ------------------------------------------------------------

/**
 * Runs in every page before its scripts: wraps GPUAdapter.requestDevice so each
 * device reports uncaptured errors, the compilation info of every shader
 * module, and validation errors of every render/compute pipeline (created
 * inside an error scope, so the failing pipeline is named).
 */
function gpuInstrumentation() {
  const smoke = {
    adapter: null,
    devices: 0,
    uncaptured: [],
    shaders: [],
    pipelines: [],
    lost: [],
  };
  window.__smoke = smoke;
  if (typeof GPUAdapter === "undefined") return;
  const requestDevice = GPUAdapter.prototype.requestDevice;
  GPUAdapter.prototype.requestDevice = async function (...args) {
    const info = this.info ?? {};
    smoke.adapter = {
      vendor: info.vendor ?? "",
      architecture: info.architecture ?? "",
      description: info.description ?? "",
      fallback: Boolean(info.isFallbackAdapter ?? this.isFallbackAdapter),
    };
    const device = await requestDevice.apply(this, args);
    smoke.devices++;
    device.addEventListener("uncapturederror", (e) => {
      smoke.uncaptured.push(`${e.error?.constructor?.name ?? "GPUError"}: ${e.error?.message ?? e}`);
    });
    device.lost.then((i) => smoke.lost.push(`${i.reason}: ${i.message}`)).catch(() => {});
    const createShaderModule = device.createShaderModule.bind(device);
    device.createShaderModule = (desc) => {
      const rec = { label: desc?.label ?? "", bytes: desc?.code?.length ?? 0, messages: null };
      smoke.shaders.push(rec);
      const module = createShaderModule(desc);
      module
        .getCompilationInfo()
        .then((ci) => {
          rec.messages = ci.messages.map((m) => ({ type: m.type, line: m.lineNum, text: m.message }));
        })
        .catch((err) => {
          rec.messages = [{ type: "error", line: 0, text: `getCompilationInfo failed: ${err}` }];
        });
      return module;
    };
    const scoped = (name) => {
      const orig = device[name].bind(device);
      device[name] = (desc) => {
        const rec = { kind: name, label: desc?.label ?? "", error: null, done: false };
        smoke.pipelines.push(rec);
        device.pushErrorScope("validation");
        device.pushErrorScope("internal");
        const p = orig(desc);
        device
          .popErrorScope()
          .then((internal) => {
            if (internal) rec.error = `internal: ${internal.message}`;
            return device.popErrorScope();
          })
          .then((validation) => {
            if (validation) rec.error = `${rec.error ? `${rec.error}; ` : ""}validation: ${validation.message}`;
            rec.done = true;
          })
          .catch((err) => {
            rec.error = `error scope failed: ${err}`;
            rec.done = true;
          });
        return p;
      };
    };
    scoped("createRenderPipeline");
    scoped("createComputePipeline");
    return device;
  };
}

/** Collects console messages and page errors of `page` into `sink` (tagged by phase). */
function watchPage(page, sink, phase) {
  page.on("console", (m) => {
    const loc = m.location();
    sink.push({
      phase: phase(),
      type: m.type(),
      text: m.text(),
      where: loc?.url ? `${loc.url}:${loc.lineNumber ?? 0}` : "",
    });
  });
  page.on("pageerror", (err) => sink.push({ phase: phase(), type: "pageerror", text: String(err?.stack ?? err) }));
  page.on("requestfailed", (r) => {
    const f = r.failure()?.errorText ?? "";
    // Aborted requests (navigation away, closed sockets) are expected.
    if (!/ERR_ABORTED/.test(f)) sink.push({ phase: phase(), type: "requestfailed", text: `${r.url()} ${f}` });
  });
  page.on("response", (r) => {
    if (r.status() >= 400) sink.push({ phase: phase(), type: "http", text: `${r.status()} ${r.url()}` });
  });
}

/** Luminance statistics of a PNG (decoded in the page with a 2D canvas). */
async function imageStats(page, png) {
  const b64 = Buffer.from(png).toString("base64");
  return page.evaluate(async (data) => {
    const img = new Image();
    img.src = `data:image/png;base64,${data}`;
    await img.decode();
    const c = new OffscreenCanvas(img.width, img.height);
    const g = c.getContext("2d");
    g.drawImage(img, 0, 0);
    const px = g.getImageData(0, 0, img.width, img.height).data;
    let n = 0;
    let sum = 0;
    let sq = 0;
    const colors = new Set();
    for (let i = 0; i < px.length; i += 4 * 7) {
      const l = 0.2126 * px[i] + 0.7152 * px[i + 1] + 0.0722 * px[i + 2];
      sum += l;
      sq += l * l;
      n++;
      colors.add(((px[i] >> 4) << 8) | ((px[i + 1] >> 4) << 4) | (px[i + 2] >> 4));
    }
    const mean = sum / n;
    return { width: img.width, height: img.height, mean, std: Math.sqrt(Math.max(0, sq / n - mean * mean)), colors: colors.size };
  }, b64);
}

// ----- main ----------------------------------------------------------------------------

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const browserPath = findBrowser(opts.browser);
  if (opts.findOnly) {
    // For scripts: print the browser and exit 0, or exit 3 when there is none.
    if (browserPath) console.log(browserPath);
    return browserPath ? 0 : 3;
  }
  mkdirSync(opts.outDir, { recursive: true });

  if (!browserPath) {
    const msg = opts.browser
      ? `browser not found: ${opts.browser}`
      : "no Chrome or Edge found (set LS_BROWSER or pass --browser <path>)";
    if (opts.requireBrowser) {
      console.error(`browser-smoke: FAIL - ${msg}`);
      return 1;
    }
    console.log(`browser-smoke: skipped - ${msg}`);
    return 0;
  }
  if (!existsSync(opts.serverBin)) die(`server binary not found: ${opts.serverBin} (cargo build --release -p ls-server)`);
  if (!existsSync(join(opts.webRoot, "index.html"))) die(`${opts.webRoot}/index.html missing (cd web && npm run build)`);

  let puppeteer;
  try {
    puppeteer = (await import("puppeteer-core")).default;
  } catch {
    die("puppeteer-core is not installed (cd tools/browser-smoke && npm ci)");
  }

  const summary = { browser: browserPath, mode: null, adapter: null, results, console: [], screenshots: [] };
  const consoleLog = [];
  let phase = "startup";
  let server = null;
  let browser = null;

  const shot = async (page, name, { hideUi = false } = {}) => {
    const file = join(opts.outDir, `${name}.png`);
    if (hideUi) await page.evaluate(() => document.getElementById("ui")?.style.setProperty("visibility", "hidden"));
    const png = await page.screenshot({ path: file, type: "png" });
    if (hideUi) await page.evaluate(() => document.getElementById("ui")?.style.removeProperty("visibility"));
    summary.screenshots.push(file);
    return png;
  };

  const launch = async (headless) => {
    const args = [
      "--enable-unsafe-webgpu",
      "--ignore-gpu-blocklist",
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-background-timer-throttling",
      "--disable-renderer-backgrounding",
      "--disable-backgrounding-occluded-windows",
      "--window-size=1280,760",
    ];
    // Linux: WebGPU runs on Vulkan (Windows uses D3D12, macOS Metal by default).
    if (process.platform === "linux") args.push("--enable-features=Vulkan", "--use-angle=vulkan");
    return puppeteer.launch({
      executablePath: browserPath,
      headless,
      args,
      defaultViewport: headless ? { width: 1280, height: 720 } : null,
      protocolTimeout: 120_000,
    });
  };

  /** WebGPU adapter probe on a page from the running server (secure context: localhost). */
  const probe = async (b, base) => {
    const page = await b.newPage();
    try {
      await page.goto(`${base}/cert-hash`, { waitUntil: "load" }).catch(() => {});
      return await page.evaluate(async () => {
        if (!navigator.gpu) return { ok: false, detail: "navigator.gpu missing" };
        const a = await navigator.gpu.requestAdapter({ powerPreference: "high-performance" });
        if (!a) return { ok: false, detail: "no adapter" };
        const i = a.info ?? {};
        return { ok: true, detail: `${i.vendor} ${i.architecture} ${i.description}`.trim() };
      });
    } finally {
      await page.close();
    }
  };

  const cleanup = async () => {
    await browser?.close().catch(() => {});
    browser = null;
    await stop(server);
  };
  const onSignal = () => {
    cleanup().finally(() => process.exit(130));
  };
  process.on("SIGINT", onSignal);
  process.on("SIGTERM", onSignal);

  try {
    // --- server ---
    const port = await freePort();
    // 127.0.0.1, not localhost: the server binds IPv4 only here, and WebTransport
    // (UDP) does not fall back from ::1 the way TCP connections do.
    const base = `http://127.0.0.1:${port}`;
    const serverArgs = ["--bind", `127.0.0.1:${port}`, "--warmup", "0", "--countdown", "0", "--web-root", opts.webRoot, "--code", opts.code, "--stats-interval", "5"];
    if (opts.transport === "ws") serverArgs.push("--no-webtransport");
    server = run(opts.serverBin, serverArgs, join(opts.outDir, "server.log"));
    if (!check("server started", await waitForLine(server, /listening \(http \+ websocket/, 60_000), `port ${port}`)) {
      return 1;
    }

    // --- browser + WebGPU probe ---
    // Never fall back to a visible window on our own: it pops up on the developer's desktop
    // and takes their mouse. Headful only when asked for explicitly (--mode headful).
    const tries = [opts.mode !== "headful"];
    let probeResult = null;
    for (const headless of tries) {
      browser = await launch(headless);
      probeResult = await probe(browser, base);
      summary.mode = headless ? "headless" : "headful";
      note(`${summary.mode} ${await browser.version()}: WebGPU ${probeResult.ok ? `adapter ${probeResult.detail}` : probeResult.detail}`);
      if (probeResult.ok) break;
      await browser.close();
      browser = null;
    }
    if (!check("WebGPU adapter available", Boolean(probeResult?.ok), `${summary.mode}: ${probeResult?.detail ?? "no browser"}`)) {
      return 1;
    }
    await browser.defaultBrowserContext().overridePermissions(base, ["clipboard-read", "clipboard-write"]).catch(() => {});

    const query = opts.transport === "auto" ? "" : `?transport=${opts.transport}`;
    const newGamePage = async () => {
      const page = await browser.newPage();
      if (summary.mode === "headful") await page.setViewport({ width: 1280, height: 720 });
      await page.evaluateOnNewDocument(gpuInstrumentation);
      // Never lock the real OS cursor (hidden headless windows still grab it on Windows).
      await page.evaluateOnNewDocument(fakePointerLock);
      watchPage(page, consoleLog, () => phase);
      return page;
    };

    const startGame = async (page, which, name, at = base) => {
      await page.goto(`${at}/${query}`, { waitUntil: "load" });
      await page.waitForSelector(".start-card", { timeout: 30_000 });
      await page.$eval('input[name="name"]', (el, v) => (el.value = v), name);
      await page.$eval('input[name="code"]', (el) => (el.value = ""));
      await page.click(which === "practice" ? "button.play.secondary" : "button.play:not(.secondary)");
      await page.waitForFunction(
        () => document.querySelector(".error-screen") || document.querySelector(".hud:not(.hidden)"),
        { timeout: 90_000 },
      );
      const err = await page.$eval("#ui", (ui) => ui.querySelector(".error-screen")?.textContent ?? null);
      return err;
    };

    /** Waits for the initial terrain load ("Loading terrain" status) to finish. */
    const waitLoaded = (page) =>
      page
        .waitForFunction(() => !/Loading terrain/.test(document.querySelector(".hud-status")?.textContent ?? ""), {
          timeout: 60_000,
        })
        .then(() => true)
        .catch(() => false);

    const lockPointer = async (page) => {
      await page.mouse.click(640, 360);
      await page
        .waitForFunction(() => document.pointerLockElement?.id === "game", { timeout: 5000 })
        .catch(() => {});
      return page.evaluate(() => document.pointerLockElement?.id === "game");
    };

    const overlayText = (page) => page.$eval(".debug-overlay", (e) => e.textContent ?? "").catch(() => "");

    const gpuReport = async (page, label) => {
      // Let pending compilation-info and error-scope promises settle.
      await page.waitForFunction(
        () => window.__smoke.shaders.every((s) => s.messages) && window.__smoke.pipelines.every((p) => p.done),
        { timeout: 10_000 },
      ).catch(() => {});
      const s = await page.evaluate(() => window.__smoke);
      summary.adapter ??= s.adapter;
      const compileErrors = s.shaders.flatMap((m) =>
        (m.messages ?? [{ type: "error", text: "compilation info never arrived" }])
          .filter((x) => x.type === "error")
          .map((x) => `${m.label}:${x.line ?? 0}: ${x.text}`),
      );
      const compileWarnings = s.shaders.flatMap((m) =>
        (m.messages ?? []).filter((x) => x.type === "warning").map((x) => `${m.label}:${x.line}: ${x.text}`),
      );
      const pipelineErrors = s.pipelines.filter((p) => p.error).map((p) => `${p.label}: ${p.error}`);
      check(
        `${label}: every WGSL module compiles`,
        s.shaders.length > 0 && compileErrors.length === 0,
        `${s.shaders.length} modules (${s.shaders.map((m) => m.label).join(", ")})${compileErrors.length ? `; ${compileErrors.join(" | ")}` : ""}`,
      );
      for (const w of compileWarnings) note(`WGSL warning: ${w}`);
      // The renderer builds the scene pipelines (terrain, instanced,
      // translucent), the cockpit hull frame (VQ1) and the post chain
      // (bloom down, bloom up, composite);
      // later packages add more, so check that these are among them.
      const required = ["terrain", "instanced", "translucent", "bloom down", "bloom up", "composite", "hull frame"];
      const missing = required.filter((l) => !s.pipelines.some((p) => p.label === l));
      check(
        `${label}: every render pipeline validates`,
        missing.length === 0 && pipelineErrors.length === 0,
        `${s.pipelines.length} pipelines (${s.pipelines.map((p) => p.label).join(", ")})${missing.length ? `; missing ${missing.join(", ")}` : ""}${pipelineErrors.length ? `; ${pipelineErrors.join(" | ")}` : ""}`,
      );
      check(
        `${label}: hardware WebGPU adapter`,
        !s.adapter?.fallback || opts.allowFallback,
        s.adapter ? `${s.adapter.vendor} ${s.adapter.architecture}${s.adapter.fallback ? " (fallback adapter; --allow-fallback accepts it)" : ""}` : "no adapter recorded",
      );
      check(`${label}: no uncaptured WebGPU errors`, s.uncaptured.length === 0, s.uncaptured.slice(0, 5).join(" | "));
      check(`${label}: GPU device not lost`, s.lost.length === 0, s.lost.join(" | "));
    };

    const sceneCheck = async (page, name, label) => {
      const png = await shot(page, `${name}-scene`, { hideUi: true });
      const st = await imageStats(page, png);
      check(
        `${label}: 3D scene rendered (not a blank frame)`,
        st.std > 4 && st.colors > 16,
        `${st.width}x${st.height}, mean ${st.mean.toFixed(1)}, std ${st.std.toFixed(1)}, ${st.colors} colours`,
      );
    };

    const consoleProblems = (ph) =>
      consoleLog.filter(
        (m) =>
          m.phase === ph &&
          (m.type === "error" || m.type === "pageerror" || m.type === "requestfailed" || m.type === "http" || m.type === "assert" ||
            (m.type === "warning" && /webgpu|wgsl|shader|pipeline/i.test(m.text))),
      );
    const consoleCheck = (ph, label) => {
      const bad = consoleProblems(ph);
      check(`${label}: no console errors`, bad.length === 0, bad.slice(0, 5).map((m) => `[${m.type}] ${m.text}`).join(" | "));
    };

    // --- 1: practice ---
    phase = "practice";
    {
      const page = await newGamePage();
      const err = await startGame(page, "practice", "SmokePractice");
      if (check("practice: game starts (HUD shown, no error screen)", err === null, err ?? "")) {
        const loaded = await waitLoaded(page);
        check("practice: initial terrain load finishes", loaded);
        await page.keyboard.press("F3");
        const locked = await lockPointer(page);
        note(`pointer lock ${locked ? "granted" : "not granted"}`);
        const t0 = Date.now();
        if (locked) await page.keyboard.down("KeyW");
        await sleep(opts.practiceSecs * 1000);
        if (locked) await page.keyboard.up("KeyW");
        const text = await overlayText(page);
        const fps = Number(/^(\d+) fps/.exec(text)?.[1] ?? 0);
        note(`debug overlay after ${((Date.now() - t0) / 1000).toFixed(1)} s: ${text.replace(/\n/g, " | ")}`);
        check("practice: frames render", fps > 0, `${fps} fps`);
        await shot(page, "practice");
        await sceneCheck(page, "practice", "practice");
        await gpuReport(page, "practice");
      }
      consoleCheck("practice", "practice");
      await page.close();
    }

    // --- 2: online ---
    phase = "online";
    {
      const page = await newGamePage();
      const err = await startGame(page, "online", "SmokeOnline");
      if (check("online: joins the server (HUD shown, no error screen)", err === null, err ?? "")) {
        check("online: server logs the join", await waitForLine(server, /player joined.*SmokeOnline/, 10_000));
        check("online: initial terrain load finishes", await waitLoaded(page));
        await page.keyboard.press("F3");
        const locked = check("online: pointer lock granted (keys reach the game)", await lockPointer(page));
        const flight = locked ? await fly(page, opts.flySecs, (name) => shot(page, name)) : null;
        if (flight) {
          const shield = /Shield\s*(on|off|[\d.]+\s*s)/.exec(flight.gadget)?.[0] ?? flight.gadget;
          check("online: G switches the shield gadget on", /Shield\s*on/.test(flight.gadget), shield);
        }
        await sleep(500);
        const text = await overlayText(page);
        note(`debug overlay: ${text.replace(/\n/g, " | ")}`);
        const transport = /ping [^·]+· (\w+)/.exec(text)?.[1] ?? "";
        const shots = /shots: (\d+) predicted · (\d+) confirmed · (\d+) dropped/.exec(text);
        const fps = Number(/^(\d+) fps/.exec(text)?.[1] ?? 0);
        check("online: frames render", fps > 0, `${fps} fps`);
        check(
          "online: connected over the requested transport",
          opts.transport === "auto" || transport === (opts.transport === "ws" ? "WebSocket" : "WebTransport"),
          transport,
        );
        check("online: synced with the server", /ping \d+ ms/.test(text) && !/syncing/.test(text), /ping[^\n]*/.exec(text)?.[0] ?? "");
        if (locked) {
          check(
            "online: fired shots confirmed by the server",
            Boolean(shots) && Number(shots[2]) > 0,
            shots ? `${shots[1]} predicted, ${shots[2]} confirmed, ${shots[3]} dropped` : "no shot line",
          );
          const speed = Number(/([\d.]+) u\/s/.exec(text)?.[1] ?? 0);
          note(`speed at the end of the flight: ${speed} u/s`);
        }
        await shot(page, "online");
        await sceneCheck(page, "online", "online");
        await gpuReport(page, "online");
        const still = await page.$eval("#ui", (ui) => ui.querySelector(".error-screen")?.textContent ?? null);
        check("online: still playing at the end (no error screen)", still === null, still ?? "");
      }
      consoleCheck("online", "online");
      await page.close();
    }

    // --- 3: wave C mechanics (second server with the mechanics rule set) ---
    phase = "mechanics";
    {
      const mport = await freePort();
      const mbase = `http://127.0.0.1:${mport}`;
      const margs = ["--bind", `127.0.0.1:${mport}`, "--warmup", "0", "--countdown", "0", "--web-root", opts.webRoot, "--code", opts.code, "--ruleset", join(here, "mechanics.ron")];
      if (opts.transport === "ws") margs.push("--no-webtransport");
      const mserver = run(opts.serverBin, margs, join(opts.outDir, "server-mechanics.log"));
      try {
        if (check("mechanics: server with the mechanics rule set starts", await waitForLine(mserver, /listening \(http \+ websocket/, 60_000))) {
          const page = await newGamePage();
          const err = await startGame(page, "online", "SmokeMech", mbase);
          if (check("mechanics: joins (HUD shown, no error screen)", err === null, err ?? "")) {
            check("mechanics: initial terrain load finishes", await waitLoaded(page));
            await page.keyboard.press("F3");
            if (check("mechanics: pointer lock granted", await lockPointer(page))) {
              await mechanics(page, opts, (name) => shot(page, name), overlayText);
            }
            await gpuReport(page, "mechanics");
          }
          consoleCheck("mechanics", "mechanics");
          await page.close();
        }
        const errors = mserver.text.split(/\r?\n/).filter((l) => /\bERROR\b|panicked/.test(l));
        check("mechanics: server logs no errors or panics", errors.length === 0, errors.slice(0, 3).join(" | "));
      } finally {
        await stop(mserver);
      }
    }

    // --- 4: lobby -> match -> results -> next match without a reload (M3, B6) ---
    phase = "lobby";
    {
      const lport = await freePort();
      const lbase = `http://127.0.0.1:${lport}`;
      const largs = [
        "--bind", `127.0.0.1:${lport}`, "--warmup", "30", "--countdown", "2", "--intermission", "4",
        "--web-root", opts.webRoot, "--code", opts.code, "--ruleset", join(here, "lobby.ron"),
      ];
      if (opts.transport === "ws") largs.push("--no-webtransport");
      const lserver = run(opts.serverBin, largs, join(opts.outDir, "server-lobby.log"));
      try {
        if (check("lobby: server with the lobby on starts", await waitForLine(lserver, /listening \(http \+ websocket/, 60_000))) {
          const page = await newGamePage();
          const err = await startGame(page, "online", "SmokeLobby", lbase);
          if (check("lobby: joins (HUD shown, no error screen)", err === null, err ?? "")) {
            await lobbyCycle(page, (name) => shot(page, name));
            const joins = lserver.text.split(/\r?\n/).filter((l) => /player joined/.test(l)).length;
            check("lobby: one join for both matches (no reconnect)", joins === 1, `${joins} joins`);
            await gpuReport(page, "lobby");
          }
          consoleCheck("lobby", "lobby");
          await page.close();
        }
        const errors = lserver.text.split(/\r?\n/).filter((l) => /\bERROR\b|panicked/.test(l));
        check("lobby: server logs no errors or panics", errors.length === 0, errors.slice(0, 3).join(" | "));
      } finally {
        await stop(lserver);
      }
    }

    // --- fit ring (VQ1): the gallery's fixed pose before a 1.2 u tunnel ---
    phase = "fit";
    {
      const page = await newGamePage();
      await page.goto(`${base}/?gallery=fit-tunnel`, { waitUntil: "load" });
      const ready = await page
        .waitForFunction(() => window.__gallery?.phase === "ready" || window.__gallery?.phase === "error", { timeout: 120_000 })
        .then(() => page.evaluate(() => window.__gallery))
        .catch(() => null);
      if (check("fit: the fit-tunnel scene renders", ready?.phase === "ready", ready ? (ready.error ?? `share code ${ready.shareCode}`) : "timed out")) {
        const fit = ready.fit;
        check(
          "fit: the fit ring shows before the tunnel",
          Boolean(fit?.visible) && fit.radiusPx > 4,
          fit ? `${fit.state}, ${fit.radiusPx.toFixed(1)} px at ${fit.depth.toFixed(2)} u, probes ${ready.probeMs?.toFixed(3)} ms` : "no ring",
        );
        await shot(page, "fit-tunnel");
        await gpuReport(page, "fit");
      }
      consoleCheck("fit", "fit");
      await page.close();
    }

    // --- 5: rule set editor ---
    phase = "editor";
    {
      const page = await browser.newPage();
      if (summary.mode === "headful") await page.setViewport({ width: 1280, height: 900 });
      watchPage(page, consoleLog, () => phase);
      await page.goto(`${base}/?editor=rules`, { waitUntil: "load" });
      const shown = await page
        .waitForSelector(".rules-card", { timeout: 15_000 })
        .then(() => true)
        .catch(() => false);
      if (check("editor: rule set editor opens", shown)) {
        const initial = await ruleCode(page);
        note(`default code: ${initial}`);
        // Match length 300 s and ban the first weapon with an availability select.
        const edits = await page.evaluate(() => {
          const done = [];
          for (const label of document.querySelectorAll(".rules-card label.rules-field")) {
            if (label.querySelector("span")?.textContent === "Match length (s)") {
              const input = label.querySelector("input");
              input.value = "300";
              input.dispatchEvent(new Event("input", { bubbles: true }));
              input.dispatchEvent(new Event("change", { bubbles: true }));
              done.push("match length 300");
              break;
            }
          }
          return done;
        });
        const banned = await page.evaluate(() => {
          for (const row of document.querySelectorAll(".rules-card tbody tr")) {
            const sel = row.querySelector("select");
            if (!sel || ![...sel.options].some((o) => o.value === "Banned")) continue;
            sel.value = "Banned";
            sel.dispatchEvent(new Event("change", { bubbles: true }));
            return row.querySelector("td")?.textContent ?? "?";
          }
          return null;
        });
        if (banned) edits.push(`banned ${banned}`);
        note(`edits: ${edits.join(", ") || "none"}`);
        const code = await ruleCode(page);
        const problems = await page.$$eval(".rules-problems li", (l) => l.map((e) => e.textContent));
        check("editor: generates a rule set code", /^LR[0-9A-Z-]+$/.test(code ?? "") && problems.length === 0, `${code ?? "none"}${problems.length ? `; problems: ${problems.join("; ")}` : ""}`);
        check("editor: edits change the code", Boolean(code) && code !== initial, `${initial} -> ${code}`);
        await shot(page, "editor");
        if (code) await serverAccepts(opts, code, banned);
      }
      consoleCheck("editor", "editor");
      await page.close();
    }

    // Server health over the whole run.
    const serverText = server.text;
    const errors = serverText.split(/\r?\n/).filter((l) => /\bERROR\b|panicked/.test(l));
    check("server: no errors or panics logged", errors.length === 0, errors.slice(0, 3).join(" | "));
    const warns = serverText.split(/\r?\n/).filter((l) => /\bWARN\b/.test(l));
    for (const w of warns.slice(0, 10)) note(`server warning: ${w.trim()}`);
    check("server: still running at the end", server.child.exitCode === null);
  } catch (err) {
    check(`unexpected failure during ${phase}`, false, String(err?.stack ?? err));
  } finally {
    process.off("SIGINT", onSignal);
    process.off("SIGTERM", onSignal);
    await cleanup();
    summary.console = consoleLog;
    writeFileSync(join(opts.outDir, "console.log"), consoleLog.map((m) => `[${m.phase}] ${m.type}: ${m.text}${m.where ? ` (${m.where})` : ""}`).join("\n") + "\n");
    writeFileSync(join(opts.outDir, "summary.json"), JSON.stringify(summary, null, 2));
  }

  const failed = results.filter((r) => !r.ok);
  console.log(`\nbrowser-smoke: ${summary.mode ?? "no"} browser (${browserPath})`);
  if (summary.adapter) console.log(`browser-smoke: GPU adapter ${JSON.stringify(summary.adapter)}`);
  console.log(`browser-smoke: screenshots and logs in ${opts.outDir}`);
  console.log(failed.length ? `browser-smoke: FAILED (${failed.length} of ${results.length} checks)` : `browser-smoke: passed (${results.length} checks)`);
  return failed.length ? 1 : 0;
}

/**
 * Lobby phase: warm-up (lobby card) -> Enter (ready) -> countdown -> match ->
 * results -> the next match's warm-up -> Enter -> next match, all on one
 * page (a reload would reset `window.__lsPage`) and one connection. The
 * client exposes the stage and the match count as `#ui[data-stage]` and
 * `#ui[data-matches]` (each match transition, warm-up rooms included,
 * counts one).
 */
async function lobbyCycle(page, screenshot) {
  const stage = () => page.$eval("#ui", (ui) => `${ui.dataset.stage ?? "?"} #${ui.dataset.matches ?? "?"}`).catch(() => "?");
  await page.evaluate(() => (window.__lsPage = 1));
  const inWarmup = await waitIn(page, () => document.getElementById("ui")?.dataset.stage === "warmup" && document.querySelector(".lobby:not(.hidden)"), 20_000);
  if (!check("lobby: warm-up shows the lobby card", inWarmup, await stage())) return;
  const mode = await visibleText(page, ".lobby-mode");
  check("lobby: the card names the next mode", mode === "Paint the Void", mode);
  await screenshot("lobby-warmup");
  const matches0 = Number(await page.$eval("#ui", (ui) => ui.dataset.matches ?? "0"));
  for (let round = 1; round <= 2; round++) {
    await page.keyboard.press("Enter");
    const ready = await waitIn(page, () => document.querySelector(".lobby-player.you.ready"), 5000);
    check(`lobby: Enter readies up (round ${round})`, ready);
    const target = matches0 + (round === 1 ? 1 : 3);
    const started = await waitIn(
      page,
      (n) => document.getElementById("ui")?.dataset.stage === "match" && Number(document.getElementById("ui")?.dataset.matches) === n,
      20_000,
      target,
    );
    if (!check(`lobby: countdown leads into match ${round} on the same connection`, started, await stage())) return;
    check(`lobby: the card hides in match ${round}`, (await visibleText(page, ".lobby")) === "");
    const timer = await visibleText(page, ".mode-timer");
    check(`lobby: the mode HUD shows the match timer (match ${round})`, /^\d+:\d\d$/.test(timer), timer);
    await screenshot(`lobby-match-${round}`);
    if (round === 2) break;
    const over = await waitIn(page, () => document.querySelector(".results:not(.hidden)"), 30_000);
    if (!check("lobby: the results screen shows after the match", over, await stage())) return;
    const title = await visibleText(page, ".results-title");
    const footer = await visibleText(page, ".results-footer");
    check("lobby: results name the outcome and what comes next", /Victory|Defeat|Draw|wins/.test(title) && /Next/.test(footer), `${title} · ${footer}`);
    await sleep(700); // let the results card animate in
    await screenshot("lobby-results");
    const back = await waitIn(
      page,
      (n) => document.getElementById("ui")?.dataset.stage === "warmup" && Number(document.getElementById("ui")?.dataset.matches) === n,
      20_000,
      matches0 + 2,
    );
    if (!check("lobby: back to the warm-up of the next match", back, await stage())) return;
    check("lobby: the results screen hides in the warm-up", (await visibleText(page, ".results")) === "");
  }
  const same = await page.evaluate(() => window.__lsPage === 1);
  check("lobby: no page reload across the matches", same);
  const still = await page.$eval("#ui", (ui) => ui.querySelector(".error-screen")?.textContent ?? null);
  check("lobby: no error screen", still === null, still ?? "");
}

/** The "Rule set code" output of the editor, or null while the rules have problems. */
function ruleCode(page) {
  return page.evaluate(() => {
    for (const label of document.querySelectorAll(".rules-out")) {
      if (label.querySelector("span")?.textContent === "Rule set code") return label.querySelector("input")?.value ?? null;
    }
    return null;
  });
}

/** Starts a second ls-server with `--ruleset <code>` and checks it comes up with that code. */
async function serverAccepts(opts, code, banned) {
  const port = await freePort();
  const proc = run(
    opts.serverBin,
    ["--bind", `127.0.0.1:${port}`, "--warmup", "0", "--countdown", "0", "--no-webtransport", "--web-root", opts.webRoot, "--ruleset", code],
    join(opts.outDir, "server-ruleset.log"),
  );
  try {
    const up = await waitForLine(proc, /listening \(http \+ websocket/, 30_000);
    const line = proc.text.split(/\r?\n/).find((l) => /listening \(http/.test(l)) ?? "";
    const served = /rules_code=(\S+)/.exec(line)?.[1] ?? "";
    check(
      "editor: ls-server accepts the generated code (--ruleset)",
      up && served === code,
      up ? `server rules_code ${served}${banned ? ` (with ${banned} banned)` : ""}` : proc.text.trim().split(/\r?\n/).slice(-3).join(" | "),
    );
  } finally {
    await stop(proc);
  }
}

/**
 * Online flight: synthetic keys and mouse for `secs` seconds. Thrust, boost,
 * look around, roll, and fire: scrap laser (1), Paint Thrower (2) with a
 * colour cycle (V), Bolt Rocket (right button, X), shield (G).
 */
async function fly(page, secs, screenshot) {
  const kb = page.keyboard;
  const mouse = page.mouse;
  const end = Date.now() + secs * 1000;
  const at = (f) => Date.now() >= end - secs * 1000 * (1 - f);
  let x = 640;
  let dir = 1;
  const steps = [
    { f: 0.0, run: () => kb.down("KeyW") },
    { f: 0.05, run: () => kb.press("Digit1") },
    { f: 0.1, run: () => mouse.down({ button: "left" }) },
    { f: 0.3, run: () => mouse.up({ button: "left" }) },
    { f: 0.3, run: () => screenshot("online-laser") },
    { f: 0.32, run: () => kb.press("Digit2") },
    { f: 0.35, run: () => mouse.down({ button: "left" }) },
    { f: 0.45, run: () => kb.press("KeyV") },
    { f: 0.55, run: () => mouse.up({ button: "left" }) },
    { f: 0.55, run: () => screenshot("online-paint") },
    { f: 0.58, run: () => mouse.down({ button: "right" }) },
    { f: 0.61, run: () => mouse.up({ button: "right" }) },
    { f: 0.63, run: () => kb.press("KeyX") },
    { f: 0.66, run: () => mouse.down({ button: "right" }) },
    { f: 0.69, run: () => mouse.up({ button: "right" }) },
    { f: 0.7, run: () => screenshot("online-rocket") },
    // Held for a few frames: the gadget key is sampled as held, not as an edge.
    { f: 0.72, run: () => kb.down("KeyG") },
    { f: 0.74, run: () => kb.up("KeyG") },
    { f: 0.74, run: () => kb.down("ShiftLeft") },
    {
      f: 0.78,
      run: async () => {
        seen.gadget = await page.$eval(".hud-loadout", (e) => e.textContent ?? "").catch(() => "");
        await screenshot("online-shield");
      },
    },
    { f: 0.8, run: () => kb.down("KeyE") },
    { f: 0.85, run: () => kb.up("KeyE") },
    { f: 0.85, run: () => kb.down("Space") },
    { f: 0.9, run: () => kb.up("Space") },
    { f: 0.9, run: () => kb.up("ShiftLeft") },
    { f: 0.95, run: () => kb.press("Digit1") },
  ];
  const seen = { gadget: "" };
  let next = 0;
  while (Date.now() < end) {
    while (next < steps.length && at(steps[next].f)) await steps[next++].run();
    // Sweep the view left and right (pointer lock turns movement into look).
    x += 12 * dir;
    if (x > 900 || x < 380) dir = -dir;
    await mouse.move(x, 360 + 40 * Math.sin(Date.now() / 700));
    await sleep(33);
  }
  while (next < steps.length) await steps[next++].run();
  await kb.up("KeyW");
  return seen;
}

/** Visible text of the first element matching `sel` ("" if none or hidden). */
function visibleText(page, sel) {
  return page
    .$eval(sel, (e) => (e.classList.contains("hidden") ? "" : (e.textContent ?? "")))
    .catch(() => "");
}

/** Waits up to `ms` for `fn` (evaluated in the page) to be truthy. */
function waitIn(page, fn, ms, ...args) {
  return page
    .waitForFunction(fn, { timeout: ms, polling: 50 }, ...args)
    .then(() => true)
    .catch(() => false);
}

/**
 * Wave C mechanics with the mechanics.ron loadout (1 Fusion Charger, 2 Leech
 * Arc, 3 Paint Thrower, Eyeball secondary, Decoy gadget).
 */
async function mechanics(page, opts, screenshot, overlayText) {
  const kb = page.keyboard;
  const mouse = page.mouse;
  const settle = () => sleep(opts.mechSecs * 1000);
  await kb.down("KeyW");
  await sleep(300);
  await kb.up("KeyW");

  // Ship bay: B opens it, 1 picks the Skitter for the next respawn; the
  // Hauler is not allowed by this rule set.
  await kb.press("KeyB");
  const bay = await waitIn(page, () => document.querySelector(".ship-panel:not(.hidden) .ship-card"), 3000);
  if (check("mechanics: B opens the ship bay", bay)) {
    const banned = await page.$$eval(".ship-card.banned", (c) => c.map((e) => e.textContent ?? ""));
    check("mechanics: the ship bay marks the banned Hauler", banned.some((t) => /Hauler/.test(t)), banned.join(" | "));
    await screenshot("mech-shipbay");
    // A successful pick closes the bay; reopened, the Skitter card says "(next)".
    await kb.press("Digit1");
    check("mechanics: picking a ship closes the bay", await waitIn(page, () => document.querySelector(".ship-panel.hidden"), 3000));
    await kb.press("KeyB");
    const next = await waitIn(
      page,
      () =>
        [...document.querySelectorAll(".ship-panel:not(.hidden) .ship-card")].some(
          (c) => /Skitter/.test(c.textContent ?? "") && /\(next\)/.test(c.textContent ?? ""),
        ),
      3000,
    );
    check("mechanics: 1 picked the Skitter for the next respawn", next, (await visibleText(page, ".ship-panel")).slice(0, 200));
    await kb.press("KeyB");
    check("mechanics: B closes the ship bay", await waitIn(page, () => document.querySelector(".ship-panel.hidden"), 3000));
  }

  // Fusion Charger: holding the trigger shows the charge meter.
  await kb.press("Digit1");
  await settle();
  await mouse.down({ button: "left" });
  const charging = await waitIn(
    page,
    () => {
      const e = document.querySelector(".hud-charge");
      return e && !e.classList.contains("hidden") && e.dataset["state"];
    },
    3000,
  );
  await sleep(500);
  const state = await page
    .$eval(".hud-charge", (e) => `${e.dataset["state"] ?? ""} ${e.textContent ?? ""}`)
    .catch(() => "");
  await screenshot("mech-charge");
  await mouse.up({ button: "left" });
  check("mechanics: the Fusion Charger shows the charge meter", charging, state.trim());
  check(
    "mechanics: releasing hides the charge meter",
    await waitIn(page, () => document.querySelector(".hud-charge.hidden"), 3000),
  );

  // Leech Arc: instant, so the predictor draws no bolt for it.
  const predicted = async () => Number(/shots: (\d+) predicted/.exec(await overlayText(page))?.[1] ?? NaN);
  await kb.press("Digit2");
  await settle();
  await sleep(600); // let the Fusion shot settle; the overlay refreshes a few times a second
  const before = await predicted();
  await mouse.down({ button: "left" });
  await sleep(1200);
  await mouse.up({ button: "left" });
  await sleep(600);
  const after = await predicted();
  check(
    "mechanics: the Leech Arc predicts no projectile",
    Number.isFinite(before) && after === before,
    `predicted shots ${before} -> ${after}`,
  );

  // Paint Thrower: V cycles red -> blue in the simulation; the HUD line
  // follows the server's colour.
  await kb.press("Digit3");
  await settle();
  const red = await waitIn(
    page,
    () => /Red/.test(document.querySelector(".hud-paint-color:not(.hidden)")?.textContent ?? ""),
    3000,
  );
  const redText = await visibleText(page, ".hud-paint-color");
  await kb.press("KeyV");
  await sleep(1500);
  const blueText = await visibleText(page, ".hud-paint-color");
  check("mechanics: the paint line shows while the Paint Thrower is selected", red, redText);
  check("mechanics: V switches the paint colour (server-confirmed)", /Blue/.test(blueText), `${redText} -> ${blueText}`);
  await mouse.down({ button: "left" });
  await sleep(700);
  await mouse.up({ button: "left" });
  await screenshot("mech-paint");
  await kb.press("Digit1");
  await sleep(400);
  check("mechanics: the paint line hides for other weapons", (await visibleText(page, ".hud-paint-color")) === "");

  // Eyeball: the guided missile's view frame shows.
  await mouse.down({ button: "right" });
  await sleep(120);
  await mouse.up({ button: "right" });
  const eye = await waitIn(page, () => document.querySelector(".fx-eyeball:not(.hidden)"), 3000);
  if (eye) await screenshot("mech-eyeball");
  check("mechanics: the Eyeball view shows while it is guided", eye);
  await waitIn(page, () => document.querySelector(".fx-eyeball.hidden"), 12_000);

  // Decoy: an instant gadget; the gadget line shows its cooldown.
  await kb.down("KeyG");
  await sleep(150);
  await kb.up("KeyG");
  await sleep(1000);
  const gadget = await page.$eval(".hud-loadout", (e) => e.textContent ?? "").catch(() => "");
  const decoy = /Decoy.{0,20}/.exec(gadget)?.[0] ?? gadget;
  check("mechanics: the Decoy cooldown shows after G", /Decoy\s*[\d.]+\s*s/.test(gadget), decoy);
  await screenshot("mech-decoy");
}

main().then(
  (code) => process.exit(code),
  (err) => {
    console.error(err);
    process.exit(1);
  },
);
