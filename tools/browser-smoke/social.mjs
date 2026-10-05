// Browser smoke test, social phase (M3 package C3): server list, party invite
// link, custom-match invite link and a duel challenge relayed through a
// directory, against a local ls-directory and two ls-servers (A: Paint the
// Void, B: Team Brawl), with three players in separate browser contexts (each
// its own localStorage, so its own player identity). Headless only, with the
// fake pointer lock like every smoke page. Called by smoke.mjs.
//
//   1. Stranger, then Lead play on A (online; two teams, one each).
//   2. Party: Lead opens "Party & duels", creates a party invite; Friend opens
//      the invite link and lands in Lead's room on Lead's team (2 v 1: team
//      balance allows it; alone against nobody the party would be split).
//   3. Custom match: an invite link with a world and a rule set code opens a
//      private room with that world on A.
//   4. Server list: Challenger opens B's server list (?servers): both servers
//      with mode, players and ping, Lead in A's roster.
//   5. Duel: Challenger challenges Lead from the list with its own seed; the
//      challenge is relayed through the directory (Lead is on A, Challenger's
//      page on B); Lead's toast -> Accept; both follow the invite to a private
//      two-player room on B with the challenger's seed and play there.

import { join } from "node:path";

/** A world for the duel (the challenger's seed) and one for the custom match. */
const DUEL_SEED = "LS0G-0000-003F-AVFQ-FJ4T-QG0B-G6X3";
const CUSTOM_WORLD = "LS0G-0000-0912-MJMT-MEK8-TJ6Z-CJPF";

/**
 * Runs the phase. `h` carries smoke.mjs's helpers and state (see the call in
 * smoke.mjs); every check goes through `h.check`.
 */
export async function socialPhase(h) {
  const { check, note, sleep, opts } = h;
  const procs = [];
  const contexts = [];
  try {
    // --- directory and two servers ---
    const dport = await h.freePort();
    const dirUrl = `http://127.0.0.1:${dport}`;
    const dir = h.run(h.directoryBin, ["--bind", `127.0.0.1:${dport}`, "--heartbeat", "1", "--ttl", "5"], join(opts.outDir, "directory.log"));
    procs.push(dir);
    if (!check("social: ls-directory starts", await h.waitForLine(dir, /directory listening/, 30_000), dirUrl)) return;
    const servers = {};
    for (const [key, name, ruleset] of [
      ["A", "SmokeA", "paint_the_void"],
      ["B", "SmokeB", "team_brawl"],
    ]) {
      const port = await h.freePort();
      const args = [
        "--bind", `127.0.0.1:${port}`, "--warmup", "0", "--countdown", "0", "--web-root", opts.webRoot,
        "--code", opts.code, "--ruleset", ruleset,
        "--directory", dirUrl, "--server-name", name, "--region", `smoke-${key.toLowerCase()}`, "--public-address", `127.0.0.1:${port}`,
      ];
      if (opts.transport === "ws") args.push("--no-webtransport");
      const proc = h.run(h.serverBin, args, join(opts.outDir, `server-social-${key}.log`));
      procs.push(proc);
      servers[key] = { name, port, base: `http://127.0.0.1:${port}`, address: `127.0.0.1:${port}`, proc };
    }
    for (const s of Object.values(servers)) {
      if (!check(`social: server ${s.name} starts and registers`, await h.waitForLine(s.proc, /registered with the directory/, 60_000), s.address)) return;
    }
    const A = servers.A;
    const B = servers.B;
    const rulesCode = /listening \(http.*rules_code=(\S+)/.exec(A.proc.text)?.[1] ?? "";

    const newContextPage = async () => {
      const ctx = await h.browser.createBrowserContext();
      contexts.push(ctx);
      const page = await ctx.newPage();
      await page.evaluateOnNewDocument(h.gpuInstrumentation);
      await page.evaluateOnNewDocument(h.fakePointerLock);
      h.watchPage(page, h.consoleLog, () => "social");
      return page;
    };
    const q = opts.transport === "auto" ? "" : `transport=${opts.transport}`;
    const withQuery = (url) => (q ? `${url}${url.includes("?") ? "&" : "?"}${q}` : url);

    /** Start screen -> Play online as `name`; the page must show the start card. */
    const playOnline = async (page, name) => {
      await page.waitForSelector(".start-card", { timeout: 30_000 });
      await page.$eval('input[name="name"]', (el, v) => (el.value = v), name);
      await page.click("button.play:not(.secondary)");
      await page.waitForFunction(() => document.querySelector(".error-screen") || document.querySelector(".hud:not(.hidden)"), { timeout: 90_000 });
      return page.$eval("#ui", (ui) => ui.querySelector(".error-screen")?.textContent ?? null);
    };
    /** "player joined" fields of `name` in a server log (the last one). */
    const joined = (proc, name) => {
      const lines = proc.text.split(/\r?\n/).filter((l) => l.includes("player joined") && l.includes(`name=${name}`));
      const l = lines[lines.length - 1] ?? "";
      const f = {};
      for (const m of l.matchAll(/(\w+)=("(?:[^"\\]|\\.)*"|\S+)/g)) f[m[1]] = m[2].replace(/^"|"$/g, "");
      return lines.length ? f : null;
    };
    const info = async (s) => (await fetch(`${s.base}/api/info`)).json();
    const waitUrl = async (page, re, ms) => {
      const end = Date.now() + ms;
      while (Date.now() < end) {
        if (re.test(page.url())) return true;
        await sleep(100);
      }
      return false;
    };

    // --- 1: Stranger and Lead play on A ---
    const stranger = await newContextPage();
    await stranger.goto(withQuery(`${A.base}/`), { waitUntil: "load" });
    const strangerErr = await playOnline(stranger, "SmokeStranger");
    check("social: Stranger joins server A", strangerErr === null, strangerErr ?? "");
    const lead = await newContextPage();
    await lead.goto(withQuery(`${A.base}/`), { waitUntil: "load" });
    const leadErr = await playOnline(lead, "SmokeLead");
    if (!check("social: Lead joins server A", leadErr === null, leadErr ?? "")) return;
    check("social: A logs Lead's join", await h.waitForLine(A.proc, /player joined.*name=SmokeLead/, 10_000));

    // --- 2: party invite ---
    await lead.click(".ls-social-btn");
    await lead.waitForSelector(".ls-social-panel button", { timeout: 5000 });
    await lead.evaluate(() => {
      for (const b of document.querySelectorAll(".ls-social-panel button")) if (b.textContent === "Create party invite") b.click();
    });
    const link = await lead
      .waitForFunction(() => document.querySelector(".ls-social-panel input")?.value || null, { timeout: 10_000 })
      .then((v) => v.jsonValue())
      .catch(() => null);
    const partyCode = /[?&]party=(P-[0-9A-Z]{6})/.exec(link ?? "")?.[1] ?? null;
    const linkJoin = link ? new URL(link).searchParams.get("join") : null;
    check("social: the party panel makes an invite link with server and party code", Boolean(partyCode) && linkJoin === A.address, link ?? "no link");
    await h.shot(lead, "social-party-panel");
    if (partyCode) {
      const friend = await newContextPage();
      await friend.goto(link, { waitUntil: "load" });
      const err = await playOnline(friend, "SmokeFriend");
      if (check("social: Friend joins through the party invite link", err === null, err ?? "")) {
        await h.waitForLine(A.proc, /player joined.*name=SmokeFriend/, 10_000);
        const jl = joined(A.proc, "SmokeLead");
        const jf = joined(A.proc, "SmokeFriend");
        check("social: the invite puts Friend in Lead's room and team",
          jf !== null && jl !== null && jf.party === partyCode && jf.room === jl.room && jf.team === jl.team,
          `Lead room ${jl?.room} team ${jl?.team}; Friend room ${jf?.room} team ${jf?.team} party ${jf?.party}`);
        await h.shot(friend, "social-party-friend");
      }
      await stranger.close();
      // --- 3: custom match invite (world + rule set code, no party) ---
      const custom = withQuery(`${A.base}/?join=${A.address}&code=${CUSTOM_WORLD}${rulesCode ? `&rules=${rulesCode}` : ""}`);
      await friend.goto(custom, { waitUntil: "load" });
      const cerr = await playOnline(friend, "SmokeFriend");
      if (check("social: a custom-match invite link (world + rules) joins", cerr === null, cerr ?? "")) {
        let room = null;
        for (let i = 0; i < 50 && !room; i++) {
          room = (await info(A)).rooms.find((r) => r.label !== "public" && r.share_code === CUSTOM_WORLD && r.players === 1) ?? null;
          if (!room) await sleep(200);
        }
        check("social: the custom match is a private room with the link's world", Boolean(room), room ? `${room.label}: ${room.share_code}, ${room.ruleset}, ${room.players} player` : JSON.stringify((await info(A)).rooms));
      }
      await friend.close();
    }

    // --- 4: server list (on B's page, so Lead on A is challenged across servers) ---
    const chal = await newContextPage();
    await chal.goto(withQuery(`${B.base}/?servers`), { waitUntil: "load" });
    const listed = await h.waitIn(
      chal,
      (a, b) => {
        const rows = [...document.querySelectorAll(".ls-servers tr")].map((r) => r.textContent ?? "");
        const ra = rows.find((t) => t.includes(a));
        const rb = rows.find((t) => t.includes(b));
        return Boolean(ra && rb && /\d+ ms/.test(ra) && /\d+ ms/.test(rb));
      },
      20_000,
      "SmokeA",
      "SmokeB",
    );
    const rowText = async () => chal.$$eval(".ls-servers tr", (rows) => rows.map((r) => [...r.children].map((c) => c.textContent?.trim()).join(" | ")));
    check("social: the server list shows both servers with ping", listed, (await rowText()).slice(1).join(" || "));
    // Lead's roster entry arrives with A's next heartbeat after Lead's presence poll.
    let roster = false;
    for (let i = 0; i < 15 && !roster; i++) {
      roster = await chal.evaluate(() => [...document.querySelectorAll(".ls-servers .roster div")].some((d) => /SmokeLead/.test(d.textContent ?? "")));
      if (!roster) {
        await chal.evaluate(() => [...document.querySelectorAll(".ls-servers button")].find((b) => b.textContent === "Refresh")?.click());
        await sleep(1500);
      }
    }
    const rows = await rowText();
    const rowA = rows.find((r) => r.includes("SmokeA")) ?? "";
    const rowB = rows.find((r) => r.includes("SmokeB")) ?? "";
    check("social: the list names mode, players and region", /Paint the Void/.test(rowA) && /smoke-a/.test(rowA) && /Team Brawl/.test(rowB) && /\d+\/\d+/.test(rowA), `${rowA} || ${rowB}`);
    check("social: Lead is in A's roster with a Duel button", roster);
    await h.shot(chal, "social-server-list");

    // --- 5: duel through the directory ---
    if (roster) {
      await chal.$eval(".ls-servers input", (el, v) => {
        el.value = v;
      }, DUEL_SEED);
      await chal.evaluate(() => {
        for (const d of document.querySelectorAll(".ls-servers .roster div")) {
          if (/SmokeLead/.test(d.textContent ?? "")) d.querySelector("button")?.click();
        }
      });
      const sent = await h.waitIn(chal, () => /Challenge sent/.test(document.querySelector(".ls-servers .muted")?.textContent ?? ""), 10_000);
      check("social: the challenge is sent (relayed: Lead plays on another server)", sent, await h.visibleText(chal, ".ls-servers .muted"));
      check("social: B relays the challenge through the directory", await h.waitForLine(B.proc, /duel challenge relayed through the directory/, 10_000));
      const toast = await h.waitIn(lead, () => [...document.querySelectorAll(".ls-toast")].some((t) => /challenges you to a duel/.test(t.textContent ?? "")), 20_000);
      check("social: Lead gets the challenge toast", toast, await h.visibleText(lead, ".ls-toasts"));
      if (toast) {
        await h.shot(lead, "social-duel-toast");
        await lead.evaluate(() => {
          for (const b of document.querySelectorAll(".ls-toast button")) if (b.textContent === "Accept") b.click();
        });
        const [leadMoved, chalMoved] = await Promise.all([waitUrl(lead, /[?&]room=R-/, 20_000), waitUrl(chal, /[?&]room=R-/, 20_000)]);
        const room = /[?&]room=(R-[0-9A-Z]+)/.exec(lead.url())?.[1] ?? "";
        check("social: accepting moves both to the duel room's invite link", leadMoved && chalMoved && chal.url().includes(room) && new URL(lead.url()).searchParams.get("join") === B.address,
          `Lead ${lead.url()} · Challenger ${chal.url()}`);
        if (leadMoved && chalMoved) {
          const [e1, e2] = await Promise.all([playOnline(lead, "SmokeLead"), playOnline(chal, "SmokeChal")]);
          check("social: both play in the duel", e1 === null && e2 === null, `${e1 ?? ""} ${e2 ?? ""}`.trim());
          let duel = null;
          for (let i = 0; i < 50 && !duel; i++) {
            duel = (await info(B)).rooms.find((r) => r.label === room && r.players === 2) ?? null;
            if (!duel) await sleep(200);
          }
          check("social: the duel is a private two-player room on the challenger's server with the challenger's seed",
            Boolean(duel) && duel.share_code === DUEL_SEED && duel.max_players === 2,
            duel ? `${duel.label}: ${duel.share_code}, ${duel.mode}, ${duel.players}/${duel.max_players}` : JSON.stringify((await info(B)).rooms));
          // The challenge and its answer stay with the server and the directory
          // until they expire: the duel pages must neither offer it again nor
          // follow the answer once more (a reload). Two presence/directory polls.
          await Promise.all([lead, chal].map((pg) => pg.evaluate(() => (window.__lsPage = 1))));
          await h.waitIn(lead, () => !/Loading terrain/.test(document.querySelector(".hud-status")?.textContent ?? ""), 30_000);
          await sleep(9000);
          const again = await lead.evaluate(() => [...document.querySelectorAll(".ls-toast")].some((t) => /challenges you/.test(t.textContent ?? "")));
          check("social: the answered challenge is not offered again in the duel", !again, await h.visibleText(lead, ".ls-toasts"));
          const stayed = await Promise.all([lead, chal].map((pg) => pg.evaluate(() => window.__lsPage === 1 && Boolean(document.querySelector(".hud:not(.hidden)"))).catch(() => false)));
          check("social: both stay in the duel (no reload)", stayed.every(Boolean), `Lead ${stayed[0]}, Challenger ${stayed[1]}`);
          await h.shot(lead, "social-duel");
          await h.gpuReport(lead, "social");
        }
      }
    }
    for (const s of [A, B]) {
      const errors = s.proc.text.split(/\r?\n/).filter((l) => /\bERROR\b|panicked/.test(l));
      check(`social: server ${s.name} logs no errors or panics`, errors.length === 0, errors.slice(0, 3).join(" | "));
    }
    note(`social: directory ${dirUrl}, A ${A.address} (rules ${rulesCode}), B ${B.address}`);
  } finally {
    for (const c of contexts) await c.close().catch(() => {});
    for (const p of procs) await h.stop(p);
  }
}
