// Browser smoke phase "moods" (M4 C6; milestone-4-plan.md wave C acceptance): every comedy event
// forced once in a practice match in the real client (headless, fake pointer lock), with a
// three-frame strip per event: the tell (1.5 s into the 3 s warning), the peak (a moment of the
// active phase chosen per event, reached with `window.__lsEvents.warp`, then 0.9 s of live
// frames while the scrap laser fires) and the aftermath (after the event ended). The first event
// comes from the `?event=` page parameter, the rest from `window.__lsEvents.schedule`. Checks:
// each event announces, runs and ends on the client (the mode state's mood), frames render,
// zero WebGPU validation or uncaptured errors, no console errors.
// Screenshots: event-<id>-1-tell.png, event-<id>-2-peak.png, event-<id>-3-after.png, and
// moods-strip.json (event, phase and timing of every frame).

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { aimAt } from "./steer.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/**
 * The practice world: a default cave level with gravity wells (Gravity coffee break has
 * something to switch off) and Absurdity 0, so the director rolls nothing of its own between
 * the forced events. Share-code slider order.
 */
export const MOODS_WORLD = { seed: "4040", sliders: [30, 30, 50, 60, 0, 0, 0], archetype: 0 };

/**
 * Seconds into the active phase that show each event best (its "peak"), and whether the pilot
 * fires through it (shots show the chickens, the big heads, the cake craters).
 */
export const EVENT_PEAKS = {
  bored_level: { at: 5.5 },
  tax_drone: { at: 8 },
  gravity_coffee_break: { at: 5 },
  rubber_chickens: { at: 1.5, fire: true },
  cat_paw: { at: 1.2 },
  tired_announcer: { at: 2, fire: true },
  wrong_genre: { at: 2, fire: true },
  gossip_wall: { at: 4 },
  pizza_delivery: { at: 5 },
  everything_is_cake: { at: 3, fire: true },
  duck_parade: { at: 15 },
  hat_rain: { at: 8 },
  customer_survey: { at: 2 },
  disco_inferno: { at: 5 },
  big_head_day: { at: 3, fire: true },
  moving_day: { at: 1 },
  mimic_pickups: { at: 5 },
  level_hiccups: { at: 3.2 },
};

/** Entity kinds of event props (ls-wasm entityKindNames): the camera turns to the nearest. */
const PROP_KINDS = ["drone", "duck", "hat", "catPaw", "gossipFace", "discoBall", "mimic"];

/** Longest turn toward an event's prop before a screenshot (s). */
const AIM_SECS = 0.6;
/** Announce phase length (data/events/director.ron `announce_secs`). */
const ANNOUNCE_SECS = 3;
/** Live seconds before a screenshot after a warp (remeshes, particles and lights catch up). */
const SETTLE_SECS = 0.9;
/**
 * Simulated seconds one practice page may use: practice plays Paint the Void (5 min) and the
 * director starts nothing once the match is over, so the events are split over pages.
 */
const PAGE_SIM_SECS = 250;

/** ls-wasm under Node (web/src/wasm/pkg), or null when it is not built. */
async function loadCore() {
  const dir = join(repoRoot, "web", "src", "wasm", "pkg");
  const js = join(dir, "ls_wasm.js");
  const wasm = join(dir, "ls_wasm_bg.wasm");
  if (!existsSync(js) || !existsSync(wasm)) return null;
  const pkg = await import(pathToFileURL(js).href);
  pkg.initSync({ module: readFileSync(wasm) });
  return pkg;
}

/** `data/events/<id>.ron` active and ending seconds. */
function eventTimes(id) {
  const text = readFileSync(join(repoRoot, "data", "events", `${id}.ron`), "utf8");
  const f = (k) => Number(new RegExp(`\\b${k}:\\s*([0-9.]+)`).exec(text)?.[1] ?? 0);
  return { active: f("active_secs"), ending: f("ending_secs") };
}

/** Splits the events into groups whose simulated time fits one practice page (exported for tests). */
export function eventBatches(ids, times = eventTimes, budget = PAGE_SIM_SECS) {
  const out = [];
  let cur = [];
  let used = 8;
  for (const id of ids) {
    const t = times(id);
    const need = ANNOUNCE_SECS + t.active + t.ending + 2 * SETTLE_SECS + 1.5;
    if (cur.length > 0 && used + need > budget) {
      out.push(cur);
      cur = [];
      used = 8;
    }
    cur.push(id);
    used += need;
  }
  if (cur.length) out.push(cur);
  return out;
}

/**
 * h: { opts, base, query, newPage, check, note, sleep, shot, gpuReport, sceneCheck, waitLoaded, lockPointer }
 */
export async function moodsPhase(h) {
  const { check, note } = h;
  const core = await loadCore();
  const ids = core?.eventIds?.() ?? Object.keys(EVENT_PEAKS);
  const code = core?.makeShareCode
    ? core.makeShareCode(MOODS_WORLD.seed, Uint8Array.from(MOODS_WORLD.sliders), MOODS_WORLD.archetype)
    : h.opts.code;
  const batches = eventBatches(ids);
  note(`moods: practice on ${code}, ${ids.length} events on ${batches.length} pages`);
  const strip = [];
  let ok = 0;
  try {
    for (const [i, batch] of batches.entries()) ok += await moodsPage(h, code, batch, strip, i === batches.length - 1);
  } finally {
    writeFileSync(join(h.opts.outDir, "moods-strip.json"), JSON.stringify(strip, null, 2));
  }
  check(`moods: all ${ids.length} events forced, ran and ended in practice`, ok === ids.length, `${ok} of ${ids.length}`);
}

/** One practice page through `ids`; returns how many events announced, ran and ended. */
async function moodsPage(h, code, ids, strip, last) {
  const { check, note, sleep } = h;
  const page = await h.newPage();
  let ok = 0;
  try {
    const first = ids[0];
    const sep = h.query ? "&" : "?";
    // The first event through the page parameter, 4 s after the start (the level loads first).
    await page.goto(`${h.base}/${h.query}${sep}event=${first}@4`, { waitUntil: "load" });
    await page.waitForSelector(".start-card", { timeout: 30_000 });
    await page.$eval('input[name="name"]', (el) => (el.value = "SmokeMoods"));
    await page.$eval('input[name="code"]', (el, v) => (el.value = v), code);
    await page.click("button.play.secondary");
    await page.waitForFunction(() => document.querySelector(".error-screen") || document.querySelector(".hud:not(.hidden)"), { timeout: 90_000 });
    const err = await page.$eval("#ui", (ui) => ui.querySelector(".error-screen")?.textContent ?? null);
    if (!check(`moods: practice starts (${first} and ${ids.length - 1} more)`, err === null, err ?? "")) return 0;
    check("moods: initial terrain load finishes", await h.waitLoaded(page));
    const hasHook = await page.evaluate(() => Boolean(window.__lsEvents));
    if (!check("moods: the client exposes window.__lsEvents (practice)", hasHook)) return 0;
    const locked = await h.lockPointer(page);
    note(`moods: pointer lock ${locked ? "granted" : "not granted"}`);

    // Frame the view: the most open direction around the pilot (64 rays, mostly level).
    const frameOpen = async () => {
      const best = await page.evaluate(() => {
        const g = window.__lsGame;
        const pose = g?.pose();
        if (!pose) return null;
        let pick = null;
        for (let i = 0; i < 64; i++) {
          const a = (i / 64) * Math.PI * 2;
          const y = [-0.25, -0.1, 0.05, 0.2][i % 4];
          const h = Math.sqrt(1 - y * y);
          const d = { x: Math.cos(a) * h, y, z: Math.sin(a) * h };
          const t = g.raycast(pose.position, d, 160);
          const free = t < 0 ? 160 : t;
          if (!pick || free > pick.free) pick = { free, d };
        }
        const p = pose.position;
        return { free: pick.free, at: { x: p.x + pick.d.x * 40, y: p.y + pick.d.y * 40, z: p.z + pick.d.z * 40 } };
      });
      const left = best ? await aimAt(page, best.at) : Infinity;
      return { free: best?.free ?? 0, left };
    };
    // The nearest event prop within `range` u, if any.
    const nearestProp = (range) =>
      page.evaluate(
        (kinds, r) => {
          const g = window.__lsGame;
          const pose = g?.pose();
          if (!pose) return null;
          let best = null;
          for (const e of g.entities()) {
            if (!kinds.includes(e.kind)) continue;
            const d = Math.hypot(e.position.x - pose.position.x, e.position.y - pose.position.y, e.position.z - pose.position.z);
            if (d <= r && (!best || d < best.d)) best = { d, kind: e.kind, at: e.position };
          }
          return best;
        },
        PROP_KINDS,
        range,
      );
    const lookAtProp = async () => {
      const prop = await nearestProp(120);
      if (prop) await aimAt(page, prop.at, { ms: AIM_SECS * 1000 });
      return prop;
    };
    const view = await frameOpen();
    note(`moods: most open view ${view.free.toFixed(0)} u (aimed within ${view.left.toFixed(3)} rad)`);
    const mood = () => page.evaluate(() => window.__lsEvents.mood());
    const waitFor = async (pred, ms) => {
      const end = Date.now() + ms;
      for (;;) {
        const m = await mood();
        if (pred(m)) return m;
        if (Date.now() > end) return m;
        await sleep(100);
      }
    };
    const warp = (secs) => (secs > 0 ? page.evaluate((s) => window.__lsEvents.warp(s), secs) : Promise.resolve(0));
    const fire = async (secs) => {
      if (!locked) return sleep(secs * 1000);
      await page.mouse.down({ button: "left" });
      await sleep(secs * 1000);
      await page.mouse.up({ button: "left" });
    };

    for (const id of ids) {
      const peak = EVENT_PEAKS[id] ?? { at: 2 };
      const t = eventTimes(id);
      if (id !== first) {
        const scheduled = await page.evaluate((e) => window.__lsEvents.schedule(e, 0.1), id);
        if (!check(`moods: ${id}: scheduled`, scheduled)) continue;
      }
      // Tell: 1.5 s into the warning.
      const announced = await waitFor((m) => m.eventId === id, id === first ? 15_000 : 3000);
      if (!check(`moods: ${id}: announced`, announced.eventId === id && announced.phase === "announce", `${announced.eventId || "no event"} ${announced.phase}`)) {
        continue;
      }
      const announcedAt = Date.now();
      await lookAtProp();
      await sleep(600);
      const tell = await mood();
      await h.shot(page, `event-${id}-1-tell`);
      strip.push({ id, frame: "tell", phase: tell.phase, mood: tell.mood });
      // Peak: warp to `peak.at - AIM - SETTLE` into the active phase, turn to its prop, then live frames.
      // Practice runs in real time: the simulated time since the announcement is the wall time.
      const elapsed = (Date.now() - announcedAt) / 1000;
      const toPeak = ANNOUNCE_SECS - elapsed + Math.min(peak.at, Math.max(0, t.active - 0.2)) - SETTLE_SECS - AIM_SECS;
      await warp(Math.max(ANNOUNCE_SECS - elapsed + 0.1, toPeak));
      const atPeak = await mood();
      const ran = atPeak.eventId === id && (atPeak.phase === "active" || atPeak.phase === "ending");
      check(`moods: ${id}: runs (active at its peak)`, ran, `${atPeak.eventId || "no event"} ${atPeak.phase}`);
      const prop = await lookAtProp();
      if (prop) note(`moods: ${id}: looking at its ${prop.kind} ${prop.d.toFixed(0)} u away`);
      if (peak.fire) await fire(SETTLE_SECS);
      else await sleep(SETTLE_SECS * 1000);
      await h.shot(page, `event-${id}-2-peak`);
      strip.push({ id, frame: "peak", phase: atPeak.phase, at: peak.at });
      // Aftermath: past the end (and the wind-down), live for a moment.
      const left = Math.max(0, t.active + t.ending - peak.at - SETTLE_SECS - AIM_SECS) + 1;
      await warp(left);
      const over = await waitFor((m) => m.eventId !== id, 3000);
      await sleep(SETTLE_SECS * 1000);
      await h.shot(page, `event-${id}-3-after`);
      await frameOpen();
      strip.push({ id, frame: "after", phase: over.phase, event: over.eventId });
      if (check(`moods: ${id}: ends`, over.eventId !== id, `${over.eventId || "no event"} ${over.phase}`) && ran) ok++;
    }
    if (last) await h.sceneCheck(page, "moods", "moods");
    await h.gpuReport(page, `moods (${ids.join(", ")})`);
    return ok;
  } finally {
    await page.close();
  }
}
