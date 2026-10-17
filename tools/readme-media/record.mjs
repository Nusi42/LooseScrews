#!/usr/bin/env node
// Records the README's media (docs/media/): animated GIFs of the running game and framed
// stills, from a local ls-server and a locally installed Chrome or Edge with WebGPU. Run it
// again after UI or look changes; the output is reproducible up to GPU timing.
//
//   cd tools/readme-media && npm ci && cd ../..
//   cd web && npm run build && cd ..                     # the client the server serves
//   cargo build --release -p ls-server                   # honours CARGO_TARGET_DIR
//   node tools/readme-media/record.mjs [options]
//
// Options (defaults in brackets):
//   --server-bin <path>   ls-server binary [$CARGO_TARGET_DIR/release/ls-server]
//   --web-root <dir>      built web client [web/dist]
//   --out-dir <dir>       where the GIFs and PNGs go [docs/media]
//   --work-dir <dir>      logs and, with --debug, sample frames [target/readme-media]
//   --browser <path>      Chrome/Edge executable [$LS_BROWSER, else searched]
//   --only <names>        clips and stills to make (comma-separated; see below) [all]
//   --fps <n>             GIF frame rate [10]
//   --width <px>          GIF width; the page renders at 1280x720 [640]
//   --debug               also save every sampled frame (PNG, without the pointer) and its
//                         timing (frames.json) under --work-dir/<clip>, and log the timing
//
// Clips (GIF): menus (start screen and world workshop, with a drawn pointer), flight (the
// start screen's cinematic flythrough without the UI), combat (practice: craters and paint
// on a cave wall), event (practice: the Rubber chickens comedy event). Stills (PNG, each as <name>-dark.png
// and <name>-light.png, framed for GitHub's dark and light theme): the scenes in STILLS
// below, rendered by the visual gallery (`?gallery=<scene>`).
//
// Everything is headless with the fake pointer lock (tools/browser-smoke/fake-pointer-lock.mjs);
// the server binds 127.0.0.1 on a free port and the pilot is called "Sprocket". Frames come
// from the DevTools screencast at the page's frame rate and are sampled at --fps; each GIF
// stores only the pixels that changed (media.mjs). Exit 1 on a failed clip, 2 on bad usage.

import { mkdirSync, statSync, writeFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { fakePointerLock } from "../browser-smoke/fake-pointer-lock.mjs";
import { exe, findBrowser, freePort, headerHelp, loadCore, lockPointer, repoRoot, run, sleep, stop, waitForLine, waitLoaded, webgpuArgs } from "../browser-smoke/harness.mjs";
import { MOODS_WORLD } from "../browser-smoke/moods.mjs";
import { aimAt, look } from "../browser-smoke/steer.mjs";
import { assertWorkFolder } from "../lib/work-folder.mjs";
import { decodePng, DiffGifWriter, drawPointer, drawRing } from "./media.mjs";

assertWorkFolder("tools/readme-media/record.mjs");

/** The page size every clip and still renders at (CSS px). */
const VIEW = { width: 1280, height: 720 };
/** The made-up pilot name the menus clip types. */
const PILOT = "Sprocket";
/** The combat clip's world: caverns (the gallery's cave world), no comedy events. Share-code slider order. */
const COMBAT_WORLD = { seed: "7", sliders: [50, 50, 50, 0, 0, 0, 30], archetype: 0 };
/** The comedy event of the event clip (data/events/); its world is the smoke test's MOODS_WORLD (Absurdity 0). */
const EVENT = "rubber_chickens";

/** Gallery scenes for the stills: file name, scene id, caption. */
const STILLS = [
  { name: "fight", scene: "fight", caption: "A Bolt Rocket goes off in a cave tunnel" },
  { name: "ship-bay", scene: "ship-bay", caption: "The ship bay: Skitter, Tinker, Hauler and Tug" },
  { name: "ink", scene: "palette-100-paint", caption: "Palette 100: a black-and-white ink world, paint keeps its colour" },
  { name: "open-sky", scene: "cockpit-sky", caption: "From the cockpit, under an open sky" },
];

const CLIPS = ["menus", "flight", "combat", "event"];

function die(msg) {
  console.error(`record: ${msg}`);
  process.exit(2);
}

function parseArgs(argv) {
  const target = resolve(repoRoot, process.env.CARGO_TARGET_DIR || "target");
  const o = {
    serverBin: join(target, "release", `ls-server${exe}`),
    webRoot: join(repoRoot, "web", "dist"),
    outDir: join(repoRoot, "docs", "media"),
    workDir: join(repoRoot, "target", "readme-media"),
    browser: process.env.LS_BROWSER ?? "",
    only: [...CLIPS, ...STILLS.map((s) => s.name)],
    fps: 10,
    width: 640,
    debug: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => argv[++i] ?? die(`missing value for ${a}`);
    if (a === "--server-bin") o.serverBin = resolve(val());
    else if (a === "--web-root") o.webRoot = resolve(val());
    else if (a === "--out-dir") o.outDir = resolve(val());
    else if (a === "--work-dir") o.workDir = resolve(val());
    else if (a === "--browser") o.browser = resolve(val());
    else if (a === "--only") o.only = val().split(",");
    else if (a === "--fps") o.fps = Number(val());
    else if (a === "--width") o.width = Number(val());
    else if (a === "--debug") o.debug = true;
    else if (a === "--help" || a === "-h") {
      console.log(headerHelp(import.meta.url));
      process.exit(0);
    } else die(`unknown argument ${a}`);
  }
  const known = [...CLIPS, ...STILLS.map((s) => s.name)];
  for (const n of o.only) if (!known.includes(n)) die(`unknown clip or still ${n} (${known.join(", ")})`);
  if (!(o.fps >= 4 && o.fps <= 30)) die("--fps must be 4..30");
  if (!(o.width >= 320 && o.width <= VIEW.width)) die(`--width must be 320..${VIEW.width}`);
  return o;
}

// ----- pointer -----------------------------------------------------------------------

/** How long a click's ring grows and fades (ms). */
const CLICK_MS = 450;

/**
 * The pointer the clips draw into their frames: where the script last moved the mouse,
 * whether it is shown, and the last click.
 */
class Pointer {
  constructor() {
    this.x = VIEW.width / 2;
    this.y = VIEW.height / 2;
    this.visible = false;
    this.clickAt = -Infinity;
  }

  state(now = Date.now()) {
    return { x: this.x, y: this.y, visible: this.visible, clickAge: now - this.clickAt };
  }

  /** Moves the mouse to (`x`, `y`) along an eased path over `ms`. */
  async glide(page, x, y, ms = 650) {
    const x0 = this.x;
    const y0 = this.y;
    const start = Date.now();
    for (;;) {
      const f = Math.min(1, (Date.now() - start) / ms);
      const e = f < 0.5 ? 2 * f * f : 1 - (-2 * f + 2) ** 2 / 2;
      this.x = x0 + (x - x0) * e;
      this.y = y0 + (y - y0) * e;
      await page.mouse.move(this.x, this.y);
      if (f >= 1) return;
      await sleep(16);
    }
  }

  async click(page, x, y, ms) {
    await this.glide(page, x, y, ms);
    this.clickAt = Date.now();
    await page.mouse.click(x, y);
  }

  /** Drags from (`x0`, `y0`) to (`x1`, `y1`) with the left button held. */
  async drag(page, x0, y0, x1, y1, ms = 1200) {
    await this.glide(page, x0, y0);
    this.clickAt = Date.now();
    await page.mouse.down();
    await this.glide(page, x1, y1, ms);
    await page.mouse.up();
  }
}

/** Draws the pointer state `p` into a frame of `w` x `h` (page coordinates scaled by `s`). */
function overlayPointer(rgba, w, h, s, p) {
  if (!p.visible) return;
  const x = p.x * s;
  const y = p.y * s;
  if (p.clickAge >= 0 && p.clickAge < CLICK_MS) {
    const f = p.clickAge / CLICK_MS;
    drawRing(rgba, w, h, x, y, 5 + 13 * f, { thickness: 2.5, alpha: 0.9 * (1 - f) });
  }
  drawPointer(rgba, w, h, x, y, 1);
}

// ----- recording -----------------------------------------------------------------------

/**
 * Records `page` while `script(cut)` runs: the DevTools screencast keeps the newest frame, a
 * timer samples it with the pointer state at `fps`, and the samples become a GIF of
 * `width` px. `await cut(promise)` leaves the time until `promise` settles out of the clip
 * (a loading wait). `tolerance` and `maxColors` go to DiffGifWriter. Returns { file, frames,
 * bytes, seconds, size }.
 */
async function recordClip(page, opts, name, pointer, script, { tolerance = 10, maxColors = 128 } = {}) {
  const height = Math.round((opts.width * VIEW.height) / VIEW.width);
  const cdp = await page.createCDPSession();
  let latest = null;
  cdp.on("Page.screencastFrame", (f) => {
    latest = f.data;
    cdp.send("Page.screencastFrameAck", { sessionId: f.sessionId }).catch(() => {});
  });
  await cdp.send("Page.startScreencast", { format: "png", maxWidth: opts.width, maxHeight: height, everyNthFrame: 1 });
  const samples = [];
  let paused = false;
  let skipped = 0; // ms cut out so far
  const timer = setInterval(() => {
    if (latest && !paused) samples.push({ t: Date.now() - skipped, data: latest, pointer: pointer.state() });
  }, 1000 / opts.fps);
  const cut = async (promise) => {
    const from = Date.now();
    paused = true;
    try {
      return await promise;
    } finally {
      skipped += Date.now() - from;
      paused = false;
      if (opts.debug) console.log(`      ${name}: cut ${((Date.now() - from) / 1000).toFixed(1)} s at sample ${samples.length}`);
    }
  };
  try {
    await script(cut);
  } finally {
    clearInterval(timer);
    await cdp.send("Page.stopScreencast").catch(() => {});
    await cdp.detach().catch(() => {});
  }
  if (samples.length < 2) throw new Error(`${name}: no frames (${samples.length})`);
  if (opts.debug) console.log(`      ${name}: ${samples.length} samples over ${((samples.at(-1).t - samples[0].t) / 1000).toFixed(1)} s`);
  const first = decodePng(Buffer.from(samples[0].data, "base64"));
  const writer = new DiffGifWriter(first.width, first.height, { tolerance, maxColors });
  const scale = first.width / VIEW.width;
  const debugDir = join(opts.workDir, name);
  if (opts.debug) mkdirSync(debugDir, { recursive: true });
  for (let i = 0; i < samples.length; i++) {
    const s = samples[i];
    const png = decodePng(Buffer.from(s.data, "base64"));
    if (png.width !== first.width || png.height !== first.height) continue;
    overlayPointer(png.data, png.width, png.height, scale, s.pointer);
    // The last frame holds a little longer before the loop restarts.
    const ms = i + 1 < samples.length ? samples[i + 1].t - s.t : 1200;
    writer.add(png.data, ms);
    if (opts.debug) writeFileSync(join(debugDir, `${String(i).padStart(4, "0")}.png`), Buffer.from(s.data, "base64"));
  }
  if (opts.debug) writeFileSync(join(debugDir, "frames.json"), JSON.stringify(samples.map((s) => ({ t: s.t, pointer: s.pointer }))));
  const file = join(opts.outDir, `${name}.gif`);
  const bytes = writer.finish();
  writeFileSync(file, bytes);
  return { file, frames: writer.frames, bytes: bytes.length, seconds: (samples.at(-1).t - samples[0].t) / 1000 + 1.2, size: `${first.width}x${first.height}` };
}

// ----- clips -----------------------------------------------------------------------------

/** A seeded Math.random for the page (the start backdrop picks its world with it). */
function seededRandom() {
  let s = 0x5eed;
  Math.random = () => {
    s = (s + 0x6d2b79f5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

async function startScreen(page, base) {
  await page.goto(`${base}/?transport=ws`, { waitUntil: "load" });
  await page.waitForSelector(".start-card", { timeout: 30_000 });
  const live = await page
    .waitForFunction(() => document.body.classList.contains("backdrop-live"), { timeout: 60_000 })
    .then(() => true)
    .catch(() => false);
  if (!live) throw new Error("the start backdrop never went live");
}

/** The centre, left edge and width of the first element matching `sel` (CSS px). */
const box = (page, sel) =>
  page.$eval(sel, (e) => {
    const b = e.getBoundingClientRect();
    return { x: b.x + b.width / 2, y: b.y + b.height / 2, left: b.x, width: b.width };
  });

/** Start screen: name, ship cards, the world workshop and its Palette slider. */
async function menusClip(ctx) {
  const { page, pointer } = ctx;
  await startScreen(page, ctx.base);
  await sleep(1500);
  pointer.x = 1010;
  pointer.y = 560;
  pointer.visible = true;
  return recordClip(page, ctx.opts, "menus", pointer, async (cut) => {
    await sleep(400);
    const name = await box(page, 'input[name="name"]');
    await pointer.click(page, name.x + 60, name.y, 800);
    await page.keyboard.down("Control");
    await page.keyboard.press("KeyA");
    await page.keyboard.up("Control");
    await page.keyboard.type(PILOT, { delay: 80 });
    await sleep(250);
    const cards = await page.$$eval(".ship-pick-card", (es) => es.map((e) => {
      const b = e.getBoundingClientRect();
      return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
    }));
    await pointer.click(page, cards[2].x, cards[2].y, 600);
    await sleep(700);
    const make = await box(page, ".workshop-open");
    await pointer.click(page, make.x, make.y, 600);
    await page.waitForSelector('.workshop input[name="palette"]', { visible: true, timeout: 10_000 });
    await sleep(900);
    const pal = await box(page, '.workshop input[name="palette"]');
    await pointer.drag(page, pal.left + 8, pal.y, pal.left + 0.86 * pal.width, pal.y, 1100);
    await pointer.glide(page, pal.left + pal.width + 120, pal.y + 90, 600);
    // The preview rebuilds the world in ink: a moment of it, then straight to the result.
    await sleep(1200);
    await cut(
      page
        .waitForFunction(() => document.body.dataset.previewCode === document.querySelector(".workshop")?.dataset.code, { timeout: 30_000, polling: 50 })
        .then(() => sleep(400))
        .catch(() => {}),
    );
    await sleep(2000);
  }, { tolerance: 12, maxColors: 256 }); // UI text stays crisp with a full palette
}

/** The start screen's cinematic flythrough with its dogfight, UI hidden. */
async function flightClip(ctx) {
  const { page, pointer } = ctx;
  await startScreen(page, ctx.base);
  await page.evaluate(() => document.getElementById("ui")?.style.setProperty("visibility", "hidden"));
  await sleep(2500);
  pointer.visible = false;
  return recordClip(page, ctx.opts, "flight", pointer, () => sleep(5500));
}

/** Starts offline practice on `code` (the start card, no pointer drawn); false on an error screen. */
async function practice(page, base, code, query = "") {
  await page.goto(`${base}/?transport=ws${query}`, { waitUntil: "load" });
  await page.waitForSelector(".start-card", { timeout: 30_000 });
  await page.$eval('input[name="name"]', (el, v) => (el.value = v), PILOT);
  await page.$eval('input[name="code"]', (el, v) => (el.value = v), code);
  await page.click("button.play.secondary");
  await page.waitForFunction(() => document.querySelector(".error-screen") || document.querySelector(".hud:not(.hidden)"), { timeout: 90_000 });
  const err = await page.$eval("#ui", (ui) => ui.querySelector(".error-screen")?.textContent ?? null);
  if (err) throw new Error(`practice: ${err}`);
  if (!(await waitLoaded(page))) throw new Error("practice: terrain never loaded");
  if (!(await lockPointer(page))) throw new Error("practice: no pointer lock");
}

/** The nearest rock in front of the pilot within [near, far] u over a fan of mostly level rays. */
function findWall(page, near, far) {
  return page.evaluate(
    (n, f) => {
      const g = window.__lsGame;
      const pose = g?.pose();
      if (!pose) return null;
      let best = null;
      for (let i = 0; i < 96; i++) {
        const a = (i / 96) * Math.PI * 2;
        const y = [-0.15, 0, 0.12][i % 3];
        const h = Math.sqrt(1 - y * y);
        const d = { x: Math.cos(a) * h, y, z: Math.sin(a) * h };
        const t = g.raycast(pose.position, d, f);
        if (t < n) continue;
        // Prefer a wall at about 60 % of the range with open space to both sides.
        const score = t < 0 ? -1 : -Math.abs(t - 0.6 * f);
        if (!best || score > best.score) best = { score, t, d };
      }
      if (!best || best.t < 0) return null;
      const p = pose.position;
      return { t: best.t, at: { x: p.x + best.d.x * best.t, y: p.y + best.d.y * best.t, z: p.z + best.d.z * best.t } };
    },
    near,
    far,
  );
}

/** Holds a mouse button for `ms`. */
async function hold(page, button, ms) {
  await page.mouse.down({ button });
  await sleep(ms);
  await page.mouse.up({ button });
}

/** Practice: the pilot faces a cave wall and works it over with the laser, paint and rockets. */
async function combatClip(ctx) {
  const { page, pointer } = ctx;
  await practice(page, ctx.base, ctx.codes.combat);
  const wall = await findWall(page, 7, 22);
  if (!wall) throw new Error("combat: no wall in range");
  await aimAt(page, wall.at, { tol: 0.03, ms: 4000 });
  await sleep(1200);
  pointer.visible = false;
  return recordClip(page, ctx.opts, "combat", pointer, async () => {
    await sleep(500);
    await page.keyboard.press("Digit1");
    await hold(page, "left", 1300);
    await sleep(200);
    // Paint Thrower: a stroke across the wall, a new colour (V) for each third.
    await page.keyboard.press("Digit2");
    await look(page, -110, 0);
    await page.mouse.down({ button: "left" });
    for (let c = 0; c < 3; c++) {
      if (c > 0) await page.keyboard.press("KeyV");
      for (let k = 0; k < 10; k++) {
        await look(page, 7.3, c === 1 ? -2 : 1);
        await sleep(70);
      }
    }
    await page.mouse.up({ button: "left" });
    await look(page, -110, 0);
    await sleep(300);
    for (let i = 0; i < 3; i++) {
      await hold(page, "right", 120);
      await sleep(650);
    }
    await sleep(1500);
  });
}

/** Practice: Rubber chickens is announced, then every shot the pilot fires is a chicken. */
async function eventClip(ctx) {
  const { page, pointer } = ctx;
  await practice(page, ctx.base, ctx.codes.event, `&event=${EVENT}@6`);
  const wall = await findWall(page, 15, 50);
  if (wall) await aimAt(page, wall.at, { ms: 3000 });
  const announced = await page
    .waitForFunction((id) => window.__lsEvents?.mood().eventId === id, { timeout: 20_000, polling: 50 }, EVENT)
    .then(() => true)
    .catch(() => false);
  if (!announced) throw new Error(`event: ${EVENT} never announced`);
  pointer.visible = false;
  return recordClip(page, ctx.opts, "event", pointer, async () => {
    // The announcement (banner, announcer line), then a spray of chickens and two rocket chickens.
    await sleep(3000);
    await page.keyboard.press("Digit2");
    await look(page, -60, 10);
    await page.mouse.down({ button: "left" });
    for (let k = 0; k < 24; k++) {
      await look(page, 5, k < 12 ? -1 : 1);
      await sleep(90);
    }
    await page.mouse.up({ button: "left" });
    await look(page, -60, 0);
    for (let i = 0; i < 2; i++) {
      await hold(page, "right", 120);
      await sleep(600);
    }
    await sleep(1500);
  });
}

// ----- stills ----------------------------------------------------------------------------

/** Frame colours per GitHub theme. */
const FRAMES = {
  dark: { bezel: "linear-gradient(180deg,#171b24,#0b0d12)", edge: "rgba(255,255,255,0.10)", text: "#e6e8ee", muted: "#9aa1b2", accent: "#ffb347" },
  light: { bezel: "linear-gradient(180deg,#ffffff,#f3f4f7)", edge: "rgba(31,35,40,0.16)", text: "#1f2328", muted: "#59636e", accent: "#d5541b" },
};

/** The framed still: the shot on a rounded bezel with a caption, outside transparent. */
function frameHtml(png, caption, theme) {
  const f = FRAMES[theme];
  return `<!doctype html><html><head><meta charset="utf-8"><style>
html,body{margin:0;background:transparent}
.wrap{padding:14px;display:inline-block}
.frame{width:960px;padding:10px 10px 0;border-radius:16px;background:${f.bezel};border:1px solid ${f.edge};box-shadow:0 8px 24px rgba(0,0,0,${theme === "dark" ? 0.45 : 0.12})}
img{display:block;width:960px;height:540px;border-radius:9px}
.cap{display:flex;align-items:center;gap:10px;padding:10px 4px 11px;font:600 15px/1.2 "Segoe UI",system-ui,-apple-system,"Helvetica Neue",Arial,sans-serif;color:${f.text}}
.cap i{width:18px;height:3px;border-radius:2px;background:${f.accent};flex:none}
.cap span{color:${f.muted};font-weight:500;margin-left:auto;letter-spacing:.06em;font-size:12px;text-transform:uppercase}
</style></head><body><div class="wrap"><div class="frame"><img src="data:image/png;base64,${png.toString("base64")}"><div class="cap"><i></i>${caption}<span>Loose Screws</span></div></div></div></body></html>`;
}

async function still(ctx, s) {
  const { page, base, opts } = ctx;
  await page.goto(`${base}/?gallery=${s.scene}&quality=high`, { waitUntil: "load" });
  const ok = await page
    .waitForFunction(() => window.__gallery && window.__gallery.phase !== "loading", { timeout: 180_000, polling: 100 })
    .then(() => page.evaluate(() => window.__gallery.phase === "ready"))
    .catch(() => false);
  if (!ok) throw new Error(`still ${s.name}: gallery scene ${s.scene} did not render`);
  await sleep(800);
  const shot = Buffer.from(await page.screenshot({ type: "png" }));
  const files = [];
  const framer = await ctx.browser.newPage();
  try {
    await framer.setViewport({ width: 1100, height: 700, deviceScaleFactor: 1 });
    for (const theme of Object.keys(FRAMES)) {
      await framer.setContent(frameHtml(shot, s.caption, theme), { waitUntil: "load" });
      const el = await framer.$(".wrap");
      const file = join(opts.outDir, `${s.name}-${theme}.png`);
      await el.screenshot({ path: file, type: "png", omitBackground: true });
      files.push(file);
    }
  } finally {
    await framer.close();
  }
  return files;
}

// ----- main ------------------------------------------------------------------------------

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const browserPath = findBrowser(opts.browser);
  if (!browserPath) die("no Chrome or Edge found (set LS_BROWSER or pass --browser <path>)");
  if (!statSync(opts.serverBin, { throwIfNoEntry: false })) die(`server binary not found: ${opts.serverBin} (cargo build --release -p ls-server)`);
  if (!statSync(join(opts.webRoot, "index.html"), { throwIfNoEntry: false })) die(`${opts.webRoot}/index.html missing (cd web && npm run build)`);
  let puppeteer;
  try {
    puppeteer = (await import("puppeteer-core")).default;
  } catch {
    die("puppeteer-core is not installed (cd tools/readme-media && npm ci)");
  }
  mkdirSync(opts.outDir, { recursive: true });
  mkdirSync(opts.workDir, { recursive: true });
  const core = await loadCore();
  if (!core) die("web/src/wasm/pkg is not built (cd web && npm run build)");
  const shareCode = (w) => core.makeShareCode(w.seed, Uint8Array.from(w.sliders), w.archetype);
  const codes = { combat: shareCode(COMBAT_WORLD), event: shareCode(MOODS_WORLD) };

  const port = await freePort();
  const base = `http://127.0.0.1:${port}`;
  const server = run(opts.serverBin, ["--bind", `127.0.0.1:${port}`, "--no-webtransport", "--warmup", "0", "--countdown", "0", "--web-root", opts.webRoot, "--code", codes.combat], join(opts.workDir, "server.log"));
  let browser = null;
  let failed = false;
  try {
    if (!(await waitForLine(server, /listening \(http \+ websocket/, 60_000))) throw new Error("ls-server did not start (see server.log)");
    // Headless only: a visible window would take the desktop mouse.
    browser = await puppeteer.launch({ executablePath: browserPath, headless: true, args: webgpuArgs(VIEW), defaultViewport: VIEW, protocolTimeout: 180_000 });
    console.log(`record: ${await browser.version()} · ${base} · practice worlds ${codes.combat} (combat), ${codes.event} (event)`);
    const newPage = async () => {
      const page = await browser.newPage();
      await page.evaluateOnNewDocument(fakePointerLock); // never lock the OS cursor
      await page.evaluateOnNewDocument(seededRandom);
      page.on("pageerror", (e) => console.log(`      page error: ${e?.message ?? e}`));
      return page;
    };
    const clips = { menus: menusClip, flight: flightClip, combat: combatClip, event: eventClip };
    for (const name of CLIPS.filter((c) => opts.only.includes(c))) {
      const page = await newPage();
      try {
        const r = await clips[name]({ page, base, opts, codes, browser, pointer: new Pointer() });
        console.log(`PASS  ${name}: ${relative(repoRoot, r.file)} ${r.size}, ${r.frames} frames, ${r.seconds.toFixed(1)} s, ${(r.bytes / 1024 / 1024).toFixed(2)} MB`);
      } catch (e) {
        failed = true;
        console.log(`FAIL  ${name}: ${e?.message ?? e}`);
      } finally {
        await page.close().catch(() => {});
      }
    }
    for (const s of STILLS.filter((x) => opts.only.includes(x.name))) {
      const page = await newPage();
      try {
        const files = await still({ page, base, opts, browser }, s);
        console.log(`PASS  ${s.name}: ${files.map((f) => `${relative(repoRoot, f)} ${(statSync(f).size / 1024).toFixed(0)} KB`).join(", ")}`);
      } catch (e) {
        failed = true;
        console.log(`FAIL  ${s.name}: ${e?.message ?? e}`);
      } finally {
        await page.close().catch(() => {});
      }
    }
  } finally {
    await browser?.close().catch(() => {});
    await stop(server);
  }
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
