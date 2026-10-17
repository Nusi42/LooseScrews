// Shared pieces of the browser scripts (smoke.mjs, gallery.mjs, startup.mjs, models.mjs, the
// smoke phases and tools/readme-media/record.mjs): finding Chrome or Edge, the WebGPU launch
// flags, free ports on 127.0.0.1, spawning and stopping the game's binaries with a log, game
// page waits (terrain loaded, pointer lock), --help from a script's header, and ls-wasm under
// Node. No puppeteer import here: each script brings its own puppeteer-core.

import { spawn } from "node:child_process";
import { createWriteStream, existsSync, readFileSync } from "node:fs";
import { createServer } from "node:net";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
/** Executable suffix of the cargo binaries on this platform. */
export const exe = process.platform === "win32" ? ".exe" : "";

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** The header comment of the script at `metaUrl` (its `import.meta.url`) without the "//": its --help. */
export function headerHelp(metaUrl) {
  const src = readFileSync(fileURLToPath(metaUrl), "utf8").split(/\r?\n/).slice(1);
  const end = src.findIndex((l) => !l.startsWith("//"));
  return src.slice(0, end).map((l) => l.replace(/^\/\/ ?/, "")).join("\n");
}

// ----- browser discovery -----------------------------------------------------------

/** Installed Chrome/Edge candidates, most preferred first. */
export function browserCandidates(env = process.env, platform = process.platform) {
  if (platform === "win32") {
    const roots = [env.ProgramFiles, env["ProgramFiles(x86)"], env.LOCALAPPDATA].filter(Boolean);
    return [
      ...roots.map((r) => join(r, "Google", "Chrome", "Application", "chrome.exe")),
      ...roots.map((r) => join(r, "Microsoft", "Edge", "Application", "msedge.exe")),
    ];
  }
  if (platform === "darwin") {
    return ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"];
  }
  const dirs = (env.PATH ?? "/usr/bin").split(":");
  const names = ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "microsoft-edge", "microsoft-edge-stable"];
  return names.flatMap((n) => dirs.map((d) => join(d, n)));
}

/** `explicit` if it exists, else the first installed candidate; null if none. */
export function findBrowser(explicit) {
  if (explicit) return existsSync(explicit) ? explicit : null;
  return browserCandidates().find((p) => existsSync(p)) ?? null;
}

/**
 * Chrome flags for real WebGPU in a (headless) automated browser, a window of `width` x
 * `height` and no throttling of background pages; `extra` is appended.
 */
export function webgpuArgs({ width = 1280, height = 760, extra = [] } = {}) {
  return [
    "--enable-unsafe-webgpu",
    "--ignore-gpu-blocklist",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-timer-throttling",
    "--disable-renderer-backgrounding",
    "--disable-backgrounding-occluded-windows",
    `--window-size=${width},${height}`,
    ...extra,
    // Linux: WebGPU runs on Vulkan (Windows uses D3D12, macOS Metal by default).
    ...(process.platform === "linux" ? ["--enable-features=Vulkan", "--use-angle=vulkan"] : []),
  ];
}

// ----- processes -------------------------------------------------------------------

/** A free TCP port on 127.0.0.1. */
export function freePort() {
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

/** Spawns `bin` in the repo root, tees its output to `logFile` and collects the text. */
export function run(bin, args, logFile) {
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

/** Resolves true once the output of `proc` matches `re`, false after `ms` or at its exit. */
export function waitForLine(proc, re, ms) {
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

/** Stops a process started by `run` (waits up to 5 s for its exit). */
export async function stop(proc) {
  if (!proc || proc.child.exitCode !== null || proc.child.signalCode !== null) return;
  proc.child.kill();
  await Promise.race([proc.exited, sleep(5000)]);
}

// ----- game pages ------------------------------------------------------------------

/** Waits for a game page's initial terrain load ("Loading terrain" status) to finish. */
export function waitLoaded(page) {
  return page
    .waitForFunction(() => !/Loading terrain/.test(document.querySelector(".hud-status")?.textContent ?? ""), { timeout: 60_000 })
    .then(() => true)
    .catch(() => false);
}

/**
 * Clicks the game canvas at (`x`, `y`) to take the (fake, fake-pointer-lock.mjs) pointer lock;
 * true once the game holds it.
 */
export async function lockPointer(page, x = 640, y = 360) {
  await page.mouse.click(x, y);
  await page.waitForFunction(() => document.pointerLockElement?.id === "game", { timeout: 5000 }).catch(() => {});
  return page.evaluate(() => document.pointerLockElement?.id === "game");
}

// ----- the core under Node ---------------------------------------------------------

/** ls-wasm under Node (web/src/wasm/pkg: share codes, event ids), or null when it is not built. */
export async function loadCore() {
  const dir = join(repoRoot, "web", "src", "wasm", "pkg");
  const js = join(dir, "ls_wasm.js");
  const wasm = join(dir, "ls_wasm_bg.wasm");
  if (!existsSync(js) || !existsSync(wasm)) return null;
  const pkg = await import(pathToFileURL(js).href);
  pkg.initSync({ module: readFileSync(wasm) });
  return pkg;
}
