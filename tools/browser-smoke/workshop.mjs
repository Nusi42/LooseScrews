// Browser smoke phase "workshop" (M4 A8; milestone-4-plan.md wave A
// acceptance): open the world workshop from the start screen (headless, fake
// pointer lock), wait for its live preview, change two sliders and the seed,
// see the preview regenerate on the new code, press Save and check that the
// copied code decodes to those sliders (ls-wasm under Node when the package is
// built, and the workshop reopened on `?workshop=<code>`), then "Fly it" and
// check that practice runs on that world. Once more with the archetype
// switched to Asteroid belt when the build generates belts (generator version
// 4, M4 A1); without one the belt pass is noted as not available.
// Screenshots: workshop.png, workshop-building.png, workshop-preview.png, workshop-practice.png,
// workshop-belt.png, workshop-belt-practice.png.

import { loadCore } from "./harness.mjs";

/** The two sliders the smoke moves and the seed it types. */
export const WORKSHOP_EDITS = { seed: "4242", sliders: { enclosure: 72, density: 25 } };

/** Share-code order of the sliders (ls-worldgen `Sliders::to_array`). */
const ORDER = ["enclosure", "density", "verticality", "gravityChaos", "palette", "absurdity", "instability"];

/** Decodes a code with the core: { seed, sliders[7], archetype } or { error }. */
function decode(core, code) {
  const info = core.checkShareCode(code);
  try {
    if (!info.ok) return { error: info.error };
    return { seed: info.seed, sliders: [...info.sliders], archetype: info.archetype ?? 0 };
  } finally {
    info.free();
  }
}

/**
 * h: { opts, base, query, newPage, check, note, sleep, shot, gpuReport, sceneCheck, waitLoaded, overlayText }
 */
export async function workshopPhase(h) {
  const { check, note } = h;
  const core = await loadCore();
  if (!core) note("ls-wasm package not built under web/src/wasm/pkg: decoding through the page only");

  const page = await h.newPage();
  try {
    await page.goto(`${h.base}/${h.query}`, { waitUntil: "load" });
    await page.waitForSelector(".start-card", { timeout: 30_000 });
    const entry = await page.waitForSelector(".workshop-open:not([hidden])", { timeout: 10_000 }).catch(() => null);
    if (!check("workshop: the start screen offers Make a world", entry !== null)) return;
    await entry.click();
    const opened = await page.waitForSelector(".workshop[data-code]", { timeout: 15_000 }).catch(() => null);
    if (!check("workshop: the workshop opens with a share code", opened !== null)) return;
    const first = await page.$eval(".workshop", (w) => w.dataset.code);
    const ready = (code, timeout) =>
      page
        .waitForFunction((c) => document.body.dataset.previewCode === c, { timeout }, code)
        .then(() => true)
        .catch(() => false);
    const t0 = Date.now();
    check("workshop: the preview flies the workshop's world", await ready(first, 90_000), `${first} in ${((Date.now() - t0) / 1000).toFixed(1)} s`);
    await h.sleep(1800); // the start screen's intro shade fades out over the first world
    await h.shot(page, "workshop");

    // Seed and two sliders.
    const edits = WORKSHOP_EDITS;
    await page.evaluate((e) => {
      const set = (el, v) => {
        el.value = String(v);
        el.dispatchEvent(new Event("input", { bubbles: true }));
      };
      set(document.querySelector('.workshop input[name="seed"]'), e.seed);
      for (const [k, v] of Object.entries(e.sliders)) set(document.querySelector(`.workshop input[name="${k}"]`), v);
    }, edits);
    const second = await page.$eval(".workshop", (w) => w.dataset.code);
    check("workshop: the edits change the code", Boolean(second) && second !== first, `${first} -> ${second}`);
    await h.sleep(700); // past the 300 ms debounce: the rebuild shows (blurred old frame, progress chip)
    await h.shot(page, "workshop-building");
    const meaning = await page.$eval('.slider-row[data-slider="enclosure"] .slider-meaning', (e) => e.textContent);
    note(`enclosure ${edits.sliders.enclosure}: "${meaning}"`);
    const t1 = Date.now();
    check("workshop: the preview regenerates on the new code", await ready(second, 90_000), `${((Date.now() - t1) / 1000).toFixed(1)} s`);
    await h.sleep(1200); // the blur of the rebuild fades out
    await h.shot(page, "workshop-preview");
    await h.sceneCheck(page, "workshop-preview", "workshop");

    // Save: the copied code decodes to those sliders.
    // Chrome's async clipboard writes need "clipboard-sanitized-write" besides "clipboard-write".
    await page
      .browserContext()
      .overridePermissions(h.base, ["clipboard-read", "clipboard-write", "clipboard-sanitized-write"])
      .catch((err) => note(`clipboard permissions: ${err}`));
    await page.click(".workshop .save");
    await page.waitForSelector(".workshop .code-status.on", { timeout: 5000 }).catch(() => {});
    const status = await page.$eval(".workshop .code-status", (e) => e.textContent);
    const copied = await page.evaluate(() => navigator.clipboard.readText()).catch((err) => `clipboard: ${err}`);
    check("workshop: Save copies the share code", copied === second, `${copied} (${status})`);
    const sliders = await page.$$eval(".workshop .slider-row input", (l) => l.map((i) => Number(i.value)));
    const expected = ORDER.map((k, i) => edits.sliders[k] ?? sliders[i]);
    if (core) {
      const d = decode(core, copied);
      check(
        "workshop: the copied code decodes to the edited seed and sliders (ls-wasm)",
        !d.error && d.seed === edits.seed && JSON.stringify(d.sliders) === JSON.stringify(expected) && d.archetype === 0,
        d.error ?? `seed ${d.seed}, sliders ${d.sliders.join(" ")}, archetype ${d.archetype}`,
      );
    }
    const belt = await page.$eval('.arch[data-arch="belt"]', (b) => !b.disabled).catch(() => false);

    // Reopened on the code (?workshop=<code>): the same sliders come back.
    {
      const p2 = await h.newPage();
      try {
        await p2.goto(`${h.base}/${h.query ? `${h.query}&` : "?"}workshop=${encodeURIComponent(copied)}&backdrop=off`, { waitUntil: "load" });
        await p2.waitForSelector(".workshop[data-code]", { timeout: 15_000 });
        const back = await p2.evaluate(() => ({
          seed: document.querySelector('.workshop input[name="seed"]').value,
          sliders: [...document.querySelectorAll(".workshop .slider-row input")].map((i) => Number(i.value)),
          code: document.querySelector(".workshop").dataset.code,
        }));
        check(
          "workshop: ?workshop=<code> reopens the same world",
          back.code === copied && back.seed === edits.seed && JSON.stringify(back.sliders) === JSON.stringify(expected),
          `seed ${back.seed}, sliders ${back.sliders.join(" ")}`,
        );
      } finally {
        await p2.close();
      }
    }

    // Fly it: practice on that world.
    await flyIt(h, page, second, "workshop-practice", "workshop");

    if (!belt) {
      note("workshop: this build generates no asteroid belts yet (generator version 4, M4 A1): belt pass not run");
      return;
    }
    // Once more as an asteroid belt.
    await page.goto(`${h.base}/${h.query ? `${h.query}&` : "?"}workshop=${encodeURIComponent(second)}`, { waitUntil: "load" });
    await page.waitForSelector(".workshop[data-code]", { timeout: 15_000 });
    await page.click('.arch[data-arch="belt"]');
    const beltCode = await page.$eval(".workshop", (w) => w.dataset.code);
    const arch = await page.$eval(".workshop", (w) => w.dataset.archetype);
    check("workshop: the archetype switch makes a belt code", arch === "belt" && beltCode !== second, `${second} -> ${beltCode}`);
    if (core) {
      const d = decode(core, beltCode);
      check("workshop: the belt code decodes to the belt archetype and the same sliders", !d.error && d.archetype === 1 && JSON.stringify(d.sliders) === JSON.stringify(expected), d.error ?? `archetype ${d.archetype}`);
    }
    const t2 = Date.now();
    check("workshop: the preview flies the belt", await ready(beltCode, 120_000), `${((Date.now() - t2) / 1000).toFixed(1)} s`);
    await h.sleep(1200);
    const hint = await page.$eval('.slider-row[data-slider="enclosure"] .slider-hint', (e) => e.textContent);
    check("workshop: the sliders show their belt meanings", /asteroid/i.test(hint ?? ""), hint);
    await h.shot(page, "workshop-belt");
    await flyIt(h, page, beltCode, "workshop-belt-practice", "workshop belt");
  } finally {
    await page.close();
  }
}

/** Presses Fly it and checks that practice runs on `code`. */
async function flyIt(h, page, code, shotName, label) {
  const { check, note } = h;
  await page.click(".workshop .fly");
  await page.waitForFunction(() => document.querySelector(".error-screen") || document.querySelector(".hud:not(.hidden)"), { timeout: 90_000 });
  const err = await page.$eval("#ui", (ui) => ui.querySelector(".error-screen")?.textContent ?? null);
  if (!check(`${label}: Fly it starts practice`, err === null, err ?? "")) return;
  check(`${label}: the practice terrain loads`, await h.waitLoaded(page));
  await page.keyboard.press("F3");
  await h.sleep(800);
  const text = await h.overlayText(page);
  check(`${label}: practice flies the workshop's world`, text.includes(code), /u\/s · (\S+)/.exec(text)?.[1] ?? text.replace(/\n/g, " | "));
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem("ls.start") ?? "{}").shareCode ?? null);
  if (stored !== code) note(`stored start code ${stored}`);
  await h.shot(page, shotName);
  await h.sceneCheck(page, shotName, label);
  await h.gpuReport(page, label);
}
