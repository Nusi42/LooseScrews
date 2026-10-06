// Browser smoke phase "architect" (M4 C6; milestone-4-plan.md wave C acceptance for C4): an
// Architect round with two players in separate headless browser contexts against an ls-server
// playing The Architect: the first to join is the architect (no ship), the second a runner.
// The architect places a stroke (a temporary gravity well, falling back to a Palette blackout)
// in front of the runner through `window.__lsGame.architectStroke` (the same ArchitectAction the
// stroke menu sends; the server validates it); the runner's page sees it telegraphed (a STROKE
// objective in the telegraph phase), then acting; the architect's budget drops by its cost.
// Checks: roles, the stroke's telegraph, activation and cost, the server's stroke counter, zero
// WebGPU validation errors, no console errors. Screenshots: architect-architect.png (the
// architect's view at the telegraph), architect-runner-telegraph.png, architect-runner-active.png.

import { join } from "node:path";

/** `ls_sim::mode::objective_kind::STROKE`. */
const STROKE = 6;
/** `ls_sim::mode::player_flags::ARCHITECT`. */
const ARCHITECT_FLAG = 1 << 3;
/** `StrokeKind` indices and their default costs (data/rulesets/the_architect.ron). */
const STROKES = [
  { kind: 3, name: "Gravity Chaos+", cost: 30 },
  { kind: 6, name: "Palette", cost: 15 },
];

/**
 * h: { opts, browser, serverBin, run, stop, waitForLine, freePort, check, note, sleep, shot, gpuReport,
 *      gpuInstrumentation, fakePointerLock, audioProbe, watchPage, consoleLog }
 */
export async function architectPhase(h) {
  const { check, note, sleep, opts } = h;
  const contexts = [];
  const port = await h.freePort();
  const base = `http://127.0.0.1:${port}`;
  const args = ["--bind", `127.0.0.1:${port}`, "--warmup", "0", "--countdown", "0", "--web-root", opts.webRoot, "--code", opts.code, "--ruleset", "the_architect"];
  if (opts.transport === "ws") args.push("--no-webtransport");
  const server = h.run(h.serverBin, args, join(opts.outDir, "server-architect.log"));
  try {
    if (!check("architect: server playing The Architect starts", await h.waitForLine(server, /listening \(http \+ websocket/, 60_000))) return;
    const q = opts.transport === "auto" ? "" : `?transport=${opts.transport}`;
    const join_ = async (name) => {
      const ctx = await h.browser.createBrowserContext();
      contexts.push(ctx);
      const page = await ctx.newPage();
      await page.evaluateOnNewDocument(h.gpuInstrumentation);
      await page.evaluateOnNewDocument(h.fakePointerLock);
      await page.evaluateOnNewDocument(h.audioProbe);
      h.watchPage(page, h.consoleLog, () => "architect");
      await page.goto(`${base}/${q}`, { waitUntil: "load" });
      await page.waitForSelector(".start-card", { timeout: 30_000 });
      await page.$eval('input[name="name"]', (el, v) => (el.value = v), name);
      await page.$eval('input[name="code"]', (el) => (el.value = ""));
      await page.click("button.play:not(.secondary)");
      await page.waitForFunction(() => document.querySelector(".error-screen") || document.querySelector(".hud:not(.hidden)"), { timeout: 90_000 });
      const err = await page.$eval("#ui", (ui) => ui.querySelector(".error-screen")?.textContent ?? null);
      check(`architect: ${name} joins`, err === null, err ?? "");
      return page;
    };
    const architect = await join_("SmokeArchitect");
    // The second player within round 1's first 10 s makes the first one the architect.
    const runner = await join_("SmokeRunner");
    const state = (page) => page.evaluate(() => ({ me: window.__lsGame.playerId(), mode: window.__lsGame.modeState(), pose: window.__lsGame.pose() }));
    const role = (s) => s.mode.players.find((p) => p.id === s.me);
    const waitState = async (page, pred, ms) => {
      const end = Date.now() + ms;
      for (;;) {
        const s = await state(page);
        if (pred(s) || Date.now() > end) return s;
        await sleep(150);
      }
    };
    const a0 = await waitState(architect, (s) => ((role(s)?.flags ?? 0) & ARCHITECT_FLAG) !== 0, 15_000);
    const r0 = await waitState(runner, (s) => Boolean(s.pose?.alive), 15_000);
    if (!check("architect: the first player is the architect", ((role(a0)?.flags ?? 0) & ARCHITECT_FLAG) !== 0, `mode ${a0.mode.mode}, flags ${role(a0)?.flags}`)) return;
    check("architect: the second player runs (a ship in play)", Boolean(r0.pose?.alive) && ((role(r0)?.flags ?? 0) & ARCHITECT_FLAG) === 0, `flags ${role(r0)?.flags}`);
    await sleep(1500); // budget accrues (+4/s), the runner's terrain settles
    const budget0 = role(await state(architect))?.meter ?? 0;
    note(`architect: budget ${budget0} before the stroke`);

    // A stroke 14 u in front of the runner (its nose), radius 14.
    const r1 = await state(runner);
    const p = r1.pose.position;
    const o = r1.pose.orientation;
    // Forward (-Z) rotated by the orientation quaternion.
    const fwd = {
      x: -2 * (o.x * o.z + o.w * o.y),
      y: -2 * (o.y * o.z - o.w * o.x),
      z: -(1 - 2 * (o.x * o.x + o.y * o.y)),
    };
    const at = { x: p.x + 14 * fwd.x, y: p.y + 14 * fwd.y, z: p.z + 14 * fwd.z };
    let placed = null;
    for (const s of STROKES) {
      const before = (await state(architect)).mode.objectives.filter((x) => x.kind === STROKE).length;
      const sent = await architect.evaluate((k, c) => window.__lsGame.architectStroke(k, c.x, c.y, c.z, 14), s.kind, at);
      if (!sent) continue;
      const seen = await waitState(runner, (st) => st.mode.objectives.filter((x) => x.kind === STROKE).length > before, 3000);
      if (seen.mode.objectives.some((x) => x.kind === STROKE)) {
        placed = s;
        break;
      }
      note(`architect: ${s.name} not accepted (trying the next stroke)`);
    }
    if (!check("architect: a stroke is placed and telegraphed on the runner's page", placed !== null, placed ? placed.name : "every stroke refused")) return;
    const tele = (await state(runner)).mode.objectives.find((x) => x.kind === STROKE);
    check("architect: the runner sees the telegraph (phase 0, its kind)", tele && tele.state >> 4 === 0 && (tele.state & 15) === placed.kind, `state ${tele?.state}`);
    await h.shot(runner, "architect-runner-telegraph");
    await h.shot(architect, "architect-architect");
    const active = await waitState(runner, (s) => s.mode.objectives.some((x) => x.kind === STROKE && x.state >> 4 === 1), 5000);
    check("architect: the stroke acts after its telegraph", active.mode.objectives.some((x) => x.kind === STROKE && x.state >> 4 === 1));
    await sleep(800);
    await h.shot(runner, "architect-runner-active");
    const budget1 = role(await state(architect))?.meter ?? 0;
    // The budget grows 4/s meanwhile: the drop is at least the cost minus a few seconds of income.
    check("architect: the stroke's cost left the budget", budget1 <= budget0 - placed.cost + 4 * 6, `${budget0} -> ${budget1} (cost ${placed.cost})`);
    await h.gpuReport(runner, "architect runner");
    await h.gpuReport(architect, "architect architect");
    const errors = server.text.split(/\r?\n/).filter((l) => /\bERROR\b|panicked/.test(l));
    check("architect: server logs no errors or panics", errors.length === 0, errors.slice(0, 3).join(" | "));
  } finally {
    for (const c of contexts) await c.close().catch(() => {});
    await h.stop(server);
  }
}
