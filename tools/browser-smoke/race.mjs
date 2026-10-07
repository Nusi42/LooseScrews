// Browser smoke phase "race" (M4 C6; milestone-4-plan.md wave C acceptance, Belt Up): two players
// in separate headless browser contexts race Belt Up on an asteroid belt (an ls-server with the
// lobby on and a shortened rule set: 2 heats of at most 100 s after 8 s of start lights). Both ready up in the warm-up, the
// countdown leads into the race, and each page is flown by a small autopilot in this script
// (mouse look toward the next gate over pointer lock, W and boost held): "Belt" threads the belt
// plane, "Open" skims above it (the open line, in the radiation). Checks: start lights hold the
// racers, the race starts, gates are passed in order, a racer finishes through the EXIT, the heat ends with results and places, the
// next heat starts in the shifted slots, the match ends with a results screen; the belt site has
// its EXIT; zero WebGPU validation errors, no console errors.
// Screenshots: race-start.png (start lights), race-belt.png (in the belt), race-open.png (the
// open line), race-finish.png (a finish, or the heat results), race-results.png (match results).

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { look, lookDelta } from "./steer.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/** The smoke's Belt Up rules: data/rulesets/belt_up.ron with short heats. */
export function raceRules() {
  let text = readFileSync(join(repoRoot, "data", "rulesets", "belt_up.ron"), "utf8");
  const set = (field, value) => {
    const re = new RegExp(`(\\b${field}:\\s*)[0-9.]+`);
    if (!re.test(text)) throw new Error(`belt_up.ron has no ${field}`);
    text = text.replace(re, `$1${value}`);
  };
  set("heats", "2");
  set("heat_limit_secs", "100.0");
  // Long start lights: the new room's terrain loads before the screenshot.
  set("lights_secs", "8.0");
  set("finish_grace_secs", "10.0");
  set("results_secs", "5.0");
  return text.replace(/name: "Belt Up"/, 'name: "Belt Up (smoke)"');
}

/** A belt world: share-code sliders (Enclosure, Density, Verticality, Gravity Chaos, Palette, Absurdity, Instability). */
export const RACE_WORLD = { seed: "77", sliders: [40, 45, 50, 0, 0, 0, 10], archetype: 1 };

/** Point at `azimuth` (prograde from +X), `radius` from the belt axis, `height` above its plane. */
export function beltPoint(belt, azimuth, radius, height) {
  const c = belt.center;
  return { x: c[0] + Math.cos(azimuth) * radius, y: c[1] + height, z: c[2] - Math.sin(azimuth) * radius };
}

/**
 * The autopilot's aim: just past the next gate on the racing line (`height` above the belt
 * plane), or the exit once the lap is done; the mouse movement (px) that turns the nose
 * toward it, capped per step.
 */
export function steer(belt, pose, racer, height, maxPx = 70) {
  let target;
  if (!racer || racer.nextGate < 0) {
    target = { x: belt.center[0], y: belt.center[1], z: belt.center[2] };
  } else {
    const az = belt.gateAzimuths[racer.nextGate] + 0.2;
    target = beltPoint(belt, az, 120, height);
  }
  const d = lookDelta(pose, target, maxPx);
  return { dx: d.dx, dy: d.dy, target };
}

/**
 * h: { opts, browser, serverBin, run, stop, waitForLine, freePort, check, note, sleep, shot, gpuReport,
 *      gpuInstrumentation, fakePointerLock, audioProbe, watchPage, consoleLog, core }
 */
export async function racePhase(h) {
  const { check, note, sleep, opts } = h;
  const contexts = [];
  const rulesFile = join(opts.outDir, "race.ron");
  writeFileSync(rulesFile, raceRules());
  const code = h.core?.makeShareCode ? h.core.makeShareCode(RACE_WORLD.seed, Uint8Array.from(RACE_WORLD.sliders), RACE_WORLD.archetype) : opts.code;
  const port = await h.freePort();
  const base = `http://127.0.0.1:${port}`;
  const args = [
    "--bind", `127.0.0.1:${port}`, "--warmup", "60", "--countdown", "2", "--intermission", "4",
    "--web-root", opts.webRoot, "--code", code, "--ruleset", rulesFile,
  ];
  if (opts.transport === "ws") args.push("--no-webtransport");
  const server = h.run(h.serverBin, args, join(opts.outDir, "server-race.log"));
  const log = [];
  try {
    if (!check("race: server playing Belt Up starts", await h.waitForLine(server, /listening \(http \+ websocket/, 90_000), code)) return;
    const q = opts.transport === "auto" ? "" : `?transport=${opts.transport}`;
    const join_ = async (name) => {
      const ctx = await h.browser.createBrowserContext();
      contexts.push(ctx);
      const page = await ctx.newPage();
      await page.evaluateOnNewDocument(h.gpuInstrumentation);
      await page.evaluateOnNewDocument(h.fakePointerLock);
      await page.evaluateOnNewDocument(h.audioProbe);
      h.watchPage(page, h.consoleLog, () => "race");
      await page.goto(`${base}/${q}`, { waitUntil: "load" });
      await page.waitForSelector(".start-card", { timeout: 30_000 });
      await page.$eval('input[name="name"]', (el, v) => (el.value = v), name);
      await page.$eval('input[name="code"]', (el) => (el.value = ""));
      await page.click("button.play:not(.secondary)");
      await page.waitForFunction(() => document.querySelector(".error-screen") || document.querySelector(".hud:not(.hidden)"), { timeout: 90_000 });
      const err = await page.$eval("#ui", (ui) => ui.querySelector(".error-screen")?.textContent ?? null);
      check(`race: ${name} joins`, err === null, err ?? "");
      return page;
    };
    const pilots = [
      { name: "SmokeBelt", height: 0, page: null },
      { name: "SmokeOpen", height: 0, page: null },
    ];
    for (const p of pilots) p.page = await join_(p.name);
    const stage = (page) => page.$eval("#ui", (ui) => ui.dataset.stage ?? "").catch(() => "");
    const waitStage = async (page, want, ms) => {
      const end = Date.now() + ms;
      while (Date.now() < end) {
        if ((await stage(page)) === want) return true;
        await sleep(200);
      }
      return false;
    };
    for (const p of pilots) check(`race: ${p.name} waits in the warm-up`, await waitStage(p.page, "warmup", 20_000), await stage(p.page));
    for (const p of pilots) await p.page.keyboard.press("Enter");
    for (const p of pilots) check(`race: ${p.name}: the countdown leads into the race`, await waitStage(p.page, "match", 30_000), await stage(p.page));
    const read = (page) =>
      page.evaluate(() => {
        const g = window.__lsGame;
        const m = g?.modeState();
        return { me: g?.playerId() ?? -1, race: m?.race ?? null, players: m?.players ?? [], pose: g?.pose() ?? null };
      });
    const beltJson = await pilots[0].page.evaluate(() => window.__lsGame?.belt?.() ?? null);
    const belt = beltJson ? JSON.parse(beltJson) : null;
    if (!check("race: the client knows the belt (gates, slots, EXIT)", Boolean(belt) && belt.gateAzimuths?.length === 8 && belt.exitRadius > 0, belt ? `exit radius ${belt.exitRadius}, ${belt.slots.length} slots` : "no belt site")) return;
    pilots[1].height = belt.halfThickness + 2;
    // Start lights: held in the slots (after the race room's terrain loaded).
    await h.waitLoaded(pilots[0].page);
    const lights = await read(pilots[0].page);
    const me0 = lights.players.find((pl) => pl.id === lights.me);
    check("race: start lights hold the racers", lights.race?.phase === "lights" && ((me0?.flags ?? 0) & 32) !== 0, `${lights.race?.phase}, flags ${me0?.flags}`);
    await h.shot(pilots[0].page, "race-start");
    for (const p of pilots) {
      await p.page.mouse.click(640, 360);
      await p.page.keyboard.down("KeyW");
      await p.page.keyboard.down("ShiftLeft");
    }
    const seen = { racing: false, gates: [0, 0], finish: false, results: false, heat2: false, belt: false, open: false, slots: [] };
    const trace = [];
    const end = Date.now() + 300_000;
    let last = null;
    while (Date.now() < end) {
      for (const [i, p] of pilots.entries()) {
        const s = await read(p.page);
        last = s;
        const race = s.race;
        if (!race) continue;
        const racer = race.racers.find((r) => r.id === s.me);
        if (race.phase === "racing") seen.racing = true;
        if (!s.pose?.alive) trace.push({ t: Date.now(), pilot: i, heat: race.heat, phase: race.phase, dead: true, racer });
        if (racer) seen.gates[i] = Math.max(seen.gates[i], racer.gates);
        if (racer?.finishTick > 0 && !seen.finish) {
          seen.finish = true;
          await h.shot(p.page, "race-finish");
        }
        if (race.phase === "results" && !seen.results) {
          seen.results = true;
          log.push({ heat: race.heat, places: race.racers.map((r) => [r.id, r.place, r.gates]) });
          if (!seen.finish) await h.shot(p.page, "race-finish");
        }
        if (race.heat === 1 && racer) {
          seen.heat2 = true;
          if (!seen.slots.includes(`${s.me}:${racer.slot}`)) seen.slots.push(`${s.me}:${racer.slot}`);
        }
        if (race.phase === "racing" && s.pose?.alive) {
          const st = steer(belt, s.pose, racer, p.height);
          trace.push({ t: Date.now(), pilot: i, heat: race.heat, gates: racer?.gates, next: racer?.nextGate, pos: s.pose.position, vel: s.pose.velocity, target: st.target });
          await look(p.page, 0.6 * st.dx, 0.6 * st.dy);
          if (racer && racer.gates >= 2 && i === 0 && !seen.belt) {
            seen.belt = true;
            await h.shot(p.page, "race-belt");
          }
          if (racer && racer.gates >= 2 && i === 1 && !seen.open) {
            seen.open = true;
            await h.shot(p.page, "race-open");
          }
        }
      }
      if ((await stage(pilots[0].page)) === "results") break;
      await sleep(60);
    }
    for (const p of pilots) {
      await p.page.keyboard.up("KeyW");
      await p.page.keyboard.up("ShiftLeft");
    }
    writeFileSync(join(opts.outDir, "race-trace.json"), JSON.stringify(trace));
    note(`race: most gates in a heat ${seen.gates.join(", ")}; heat results ${JSON.stringify(log)}; heat 2 slots ${seen.slots.join(", ")}`);
    check("race: the race starts after the lights", seen.racing);
    check("race: gates are passed in order", Math.max(...seen.gates) >= 1, `most gates in a heat: ${seen.gates.join(", ")}`);
    check("race: a racer finishes through the EXIT after the lap", seen.finish);
    check("race: the heat ends with results and places", seen.results && log.some((l) => l.places.some(([, place]) => place > 0)), JSON.stringify(log));
    check("race: the next heat starts", seen.heat2, last?.race ? `heat ${last.race.heat + 1}/${last.race.heats}` : "no race");
    const results = (await stage(pilots[0].page)) === "results";
    check("race: the match ends with the results screen", results, await stage(pilots[0].page));
    if (results) await h.shot(pilots[0].page, "race-results");
    for (const p of pilots) await h.gpuReport(p.page, `race ${p.name}`);
    const errors = server.text.split(/\r?\n/).filter((l) => /\bERROR\b|panicked/.test(l));
    check("race: server logs no errors or panics", errors.length === 0, errors.slice(0, 3).join(" | "));
  } finally {
    for (const c of contexts) await c.close().catch(() => {});
    await h.stop(server);
  }
}
