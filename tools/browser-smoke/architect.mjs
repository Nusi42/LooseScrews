// Browser smoke phase "architect" (M4 C4; milestone-4-plan.md wave C
// acceptance "C4"): a server with The Architect's rule set, two headless
// pages (fake pointer lock on both). The first page joins, the second joins
// within the 10 s grace, so the first becomes the round's architect and the
// second a runner. On the architect page: the architect HUD shows, a
// Gravity+ stroke (key 4: no rock check, so the server cannot refuse it as
// "nothing to change") is placed with a click; the runner page sees it
// telegraphed, then acting; the architect's budget went down by the cost.
// Screenshots: architect-architect.png (the map view with the dock, where a
// new architect starts), architect-runner-telegraph.png,
// architect-runner-acting.png, architect-map.png (after the stroke),
// architect-free-flight.png (V).

/** Gravity+ (StrokeKind 3): key 4, cost 30 in data/rulesets/the_architect.ron. */
const STROKE_KEY = "Digit4";
const STROKE_COST = 30;

/** Reads the architect HUD's smoke hooks (`.arch-hud` data attributes, ui/architect.ts). */
function hud(page) {
  return page.evaluate(() => {
    const e = document.querySelector(".arch-hud");
    if (!e) return null;
    const d = e.dataset;
    return { role: d.role ?? "", budget: Number(d.budget ?? NaN), telegraphs: Number(d.telegraphs ?? 0), acting: Number(d.acting ?? 0), hidden: e.classList.contains("hidden") };
  });
}

async function waitHud(page, pred, ms) {
  const t0 = Date.now();
  let last = null;
  while (Date.now() - t0 < ms) {
    last = await hud(page).catch(() => null);
    if (last && pred(last)) return { ok: true, hud: last, ms: Date.now() - t0 };
    await new Promise((r) => setTimeout(r, 100));
  }
  return { ok: false, hud: last, ms: Date.now() - t0 };
}

/**
 * h: { opts, here, query, newPage, startGame, check, note, sleep, shot, gpuReport, waitLoaded, lockPointer, startServer, stopServer }
 * `startServer(args, logName)` starts ls-server with extra args on a free port and resolves { base, proc } (or null).
 */
export async function architectPhase(h) {
  const { check, sleep } = h;
  const srv = await h.startServer(["--warmup", "0", "--countdown", "0", "--ruleset", "data/rulesets/the_architect.ron", "--stats-interval", "5"], "server-architect.log");
  if (!check("architect: server with The Architect's rule set starts", srv !== null)) return;
  const pages = [];
  try {
    const a = await h.newPage();
    pages.push(a);
    const errA = await h.startGame(a, "online", "SmokeArchitect", srv.base);
    if (!check("architect: the first pilot joins", errA === null, errA ?? "")) return;
    const b = await h.newPage();
    pages.push(b);
    const errB = await h.startGame(b, "online", "SmokeRunner", srv.base);
    if (!check("architect: the second pilot joins within the grace", errB === null, errB ?? "")) return;
    // Only the page in front gets animation frames (the others idle on a timer): bring each forward.
    await a.bringToFront();
    const roleA = await waitHud(a, (x) => x.role === "architect" && !x.hidden, 20_000);
    await b.bringToFront();
    const roleB = await waitHud(b, (x) => x.role === "runner" && !x.hidden, 20_000);
    check("architect: the first pilot is the architect", roleA.ok, JSON.stringify(roleA.hud));
    check("architect: the second pilot is a runner", roleB.ok, JSON.stringify(roleB.hud));
    if (!roleA.ok || !roleB.ok) return;
    check("architect: runner page loads its terrain", await h.waitLoaded(b));
    await a.bringToFront();
    check("architect: architect page loads its terrain", await h.waitLoaded(a));
    check("architect: the architect's dock shows eight strokes", (await a.$$eval(".arch-dock:not(.hidden) .arch-card", (c) => c.length)) === 8);
    check("architect: pointer lock (fake) on the architect page", await h.lockPointer(a));

    check("architect: the architect starts in the map view", await a.$eval(".arch-hud", (e) => e.classList.contains("overview")).catch(() => false));
    // Pan the map a little, pick Gravity+ and wait until it is affordable.
    await a.keyboard.down("KeyW");
    await sleep(400);
    await a.keyboard.up("KeyW");
    await a.keyboard.press(STROKE_KEY);
    const armed = await a.waitForFunction(() => document.querySelector(".arch-card.armed .arch-card-name")?.textContent?.startsWith("Gravity"), { timeout: 5000 }).then(() => true).catch(() => false);
    check("architect: key 4 arms Gravity+", armed);
    const ready = await waitHud(a, (x) => x.budget >= STROKE_COST, 20_000);
    check("architect: budget covers the stroke", ready.ok, JSON.stringify(ready.hud));
    await h.shot(a, "architect-architect");
    const before = (await hud(a))?.budget ?? NaN;
    const t0 = Date.now();
    // A click as a hand makes it (down for a few frames, then up).
    await a.mouse.down();
    await sleep(150);
    await a.mouse.up();
    // The architect's page sees its stroke telegraphed (the server accepted it).
    const own = await waitHud(a, (x) => x.telegraphs + x.acting >= 1, 3000);
    const said = await a.evaluate(() => [document.querySelector(".arch-reticle-status")?.textContent ?? "", document.querySelector(".arch-toast:not(.hidden)")?.textContent ?? ""].join(" | "));
    check("architect: the server accepts the stroke (telegraphed on the architect's page)", own.ok, `${own.ms} ms; ${JSON.stringify(own.hud)}; reticle and toast: ${said}`);
    if (!own.ok) await h.shot(a, "architect-refused");
    await b.bringToFront();

    // Telegraphed on the runner's page, then acting.
    const tele = await waitHud(b, (x) => x.telegraphs >= 1, 6000);
    check("architect: the runner's page shows the stroke telegraphed", tele.ok, `${tele.ms} ms after the click; ${JSON.stringify(tele.hud)}`);
    if (tele.ok) await h.shot(b, "architect-runner-telegraph");
    const act = await waitHud(b, (x) => x.acting >= 1, 6000);
    check("architect: the stroke acts on the runner's page after its telegraph", act.ok, `${act.ms} ms later; ${JSON.stringify(act.hud)}`);
    if (act.ok) await h.shot(b, "architect-runner-acting");
    await a.bringToFront();
    await sleep(300);
    const after = await hud(a);
    const secs = (Date.now() - t0) / 1000;
    // +4 points a second accrue meanwhile; paying 30 leaves it clearly below that line.
    const expectMax = before + 4 * secs + 1 - (STROKE_COST - 5);
    check("architect: the stroke's cost left the architect's budget", after !== null && after.budget <= expectMax, `before ${before}, after ${after?.budget} (${secs.toFixed(1)} s, at most ${expectMax.toFixed(0)} expected)`);

    await h.shot(a, "architect-map");
    // V: free flight through the level (and back).
    await a.keyboard.press("KeyV");
    await sleep(600);
    check("architect: V switches to free flight", !(await a.$eval(".arch-hud", (e) => e.classList.contains("overview")).catch(() => true)));
    await h.shot(a, "architect-free-flight");
    await h.gpuReport(a, "architect (architect page)");
    await h.gpuReport(b, "architect (runner page)");
    const stats = srv.proc.text.match(/architect_strokes[=: ]+(\d+)/g);
    if (stats) h.note(`server: ${stats[stats.length - 1]}`);
    const errors = srv.proc.text.split(/\r?\n/).filter((l) => /\bERROR\b|panicked/.test(l));
    check("architect: server logs no errors or panics", errors.length === 0, errors.slice(0, 3).join(" | "));
  } finally {
    for (const p of pages) await p.close().catch(() => {});
    await h.stopServer(srv?.proc ?? null);
  }
}
