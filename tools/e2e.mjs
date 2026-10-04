#!/usr/bin/env node
// End-to-end soak run: ls-server + N ls-bots over real WebSockets (or
// WebTransport with --transport wt), then checks
// the logs. Called by tools/e2e.ps1 and tools/e2e.sh after they built the
// release binaries and the web client; can also be run directly.
//
//   node tools/e2e.mjs --server-bin <ls-server> --bots-bin <ls-bots> [options]
//
// Options (defaults in brackets):
//   --code <share code>     world [LS08-0000-0000-0000-8006-8001-8F9G]
//   --ruleset <file>        rule set [data/rulesets/paint_the_void.ron]
//   --count <n>             bots [7]
//   --duration <s>          bot run time [90]
//   --port <n>              server port; 0 picks a free one [0]
//   --web-root <dir>        served web client [web/dist]
//   --stats-interval <s>    server stats window [5]
//   --p99-budget-us <n>     tick p99 limit [4000]
//   --min-debris <n>        networked debris pieces some stats window must
//                           reach (64 = the cap; 0 = no check) [0]
//   --max-full-per-join <n> full snapshots the bots may receive per join
//                           (0 = no check) [0]
//   --min-craters <n>       crater edits required (0 for levels where bots
//                           only paint) [1]
//   --log-dir <dir>         where server.log / bots.log go [target/e2e-logs]
//   --transport ws|wt       bot transport: WebSocket or WebTransport (then
//                           also checks /cert-hash and that the bots joined
//                           over WebTransport) [ws]
//
// Checks: the server serves web/dist/index.html (and its assets); bots exit 0
// with every bot joined and no protocol errors or desyncs; the server logs no
// desync (input starvation) warning, no errors and no skipped ticks; tick p99
// stays under budget in every window with players; snapshots flowed; terrain
// edits (including craters from combat) and paint coverage grew; the match
// timer progressed or completed; the server shut down cleanly
// (--exit-when-empty) with exit code 0.

import { spawn } from "node:child_process";
import { createWriteStream, existsSync, mkdirSync, readFileSync } from "node:fs";
import { createServer } from "node:net";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { assertWorkFolder } from "./lib/work-folder.mjs";

assertWorkFolder("tools/e2e.mjs");

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// ----- arguments ---------------------------------------------------------------

const defaults = {
  "server-bin": "",
  "bots-bin": "",
  code: "LS08-0000-0000-0000-8006-8001-8F9G",
  ruleset: join(repoRoot, "data", "rulesets", "paint_the_void.ron"),
  count: "7",
  duration: "90",
  port: "0",
  "web-root": join(repoRoot, "web", "dist"),
  "stats-interval": "5",
  "p99-budget-us": "4000",
  "min-craters": "1",
  "min-debris": "0",
  "max-full-per-join": "0",
  "log-dir": join(repoRoot, "target", "e2e-logs"),
  transport: "ws",
};

function parseArgs(argv) {
  const o = { ...defaults };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--help" || a === "-h") {
      const src = readFileSync(fileURLToPath(import.meta.url), "utf8").split(/\r?\n/).slice(1);
      const end = src.findIndex((l) => !l.startsWith("//"));
      console.log(src.slice(0, end).map((l) => l.replace(/^\/\/ ?/, "")).join("\n"));
      process.exit(0);
    }
    const key = a.replace(/^--/, "");
    if (!a.startsWith("--") || !(key in o)) die(`unknown argument ${a}`);
    const v = argv[++i];
    if (v === undefined) die(`missing value for ${a}`);
    o[key] = v;
  }
  if (!o["server-bin"] || !o["bots-bin"]) die("--server-bin and --bots-bin are required");
  for (const k of ["server-bin", "bots-bin", "ruleset", "web-root"]) {
    o[k] = resolve(o[k]);
    if (!existsSync(o[k])) die(`${k} not found: ${o[k]}`);
  }
  o["log-dir"] = resolve(o["log-dir"]);
  if (o.transport !== "ws" && o.transport !== "wt") die("--transport must be ws or wt");
  return {
    serverBin: o["server-bin"],
    botsBin: o["bots-bin"],
    code: o.code,
    ruleset: o.ruleset,
    count: posInt(o.count, "count"),
    duration: posNum(o.duration, "duration"),
    port: Number.parseInt(o.port, 10) || 0,
    webRoot: o["web-root"],
    statsInterval: posNum(o["stats-interval"], "stats-interval"),
    p99BudgetUs: posInt(o["p99-budget-us"], "p99-budget-us"),
    minCraters: Math.max(0, Number.parseInt(o["min-craters"], 10) || 0),
    minDebris: Math.max(0, Number.parseInt(o["min-debris"], 10) || 0),
    maxFullPerJoin: Math.max(0, Number(o["max-full-per-join"]) || 0),
    logDir: o["log-dir"],
    transport: o.transport,
  };
}

function posInt(v, name) {
  const n = Number.parseInt(v, 10);
  if (!(n > 0)) die(`--${name} must be a positive integer`);
  return n;
}

function posNum(v, name) {
  const n = Number(v);
  if (!(n > 0)) die(`--${name} must be a positive number`);
  return n;
}

function die(msg) {
  console.error(`e2e: ${msg}`);
  process.exit(2);
}

// ----- log parsing (exported for reuse) ----------------------------------------

const ANSI = /\x1b\[[0-9;]*m/g;

/** `key=value` fields of a tracing line (quoted values supported). */
export function parseFields(line) {
  const out = {};
  const re = /(\w+)=("(?:[^"\\]|\\.)*"|\S+)/g;
  for (const m of line.replace(ANSI, "").matchAll(re)) {
    let v = m[2];
    if (v.startsWith('"')) v = v.slice(1, -1);
    out[m[1]] = v;
  }
  return out;
}

const num = (v) => (v === undefined ? Number.NaN : Number(v));

// ----- process helpers -----------------------------------------------------------

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

/** Spawns `bin`, tees output to `logFile` and collects the text. */
function run(name, bin, args, logFile, echo) {
  const log = createWriteStream(logFile);
  const child = spawn(bin, args, {
    cwd: repoRoot,
    env: { ...process.env, NO_COLOR: "1", RUST_LOG: process.env.RUST_LOG ?? "info" },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  const proc = { name, child, text: "", exited: null, listeners: [] };
  const onData = (buf) => {
    const s = buf.toString();
    proc.text += s;
    log.write(buf);
    if (echo) for (const l of s.split(/\r?\n/)) if (echo(l)) console.log(`  [${name}] ${l.replace(ANSI, "")}`);
    for (const f of proc.listeners) f();
  };
  child.stdout.on("data", onData);
  child.stderr.on("data", onData);
  proc.exited = new Promise((res) => {
    child.on("error", (e) => {
      proc.text += `\nspawn error: ${e.message}\n`;
      res({ code: -1, signal: null });
    });
    child.on("exit", (code, signal) => {
      // Let the pipes drain before resolving.
      setTimeout(() => {
        log.end();
        res({ code, signal });
      }, 100);
    });
  });
  return proc;
}

/** Waits for `proc` to exit within `ms`; kills it otherwise. */
async function waitExit(proc, ms) {
  let timer;
  const timeout = new Promise((res) => {
    timer = setTimeout(() => res(null), ms);
  });
  const r = await Promise.race([proc.exited, timeout]);
  clearTimeout(timer);
  if (r) return { ...r, killed: false };
  proc.child.kill();
  return { ...(await proc.exited), killed: true };
}

function waitForLine(proc, re, ms) {
  return new Promise((res) => {
    const check = () => {
      if (re.test(proc.text.replace(ANSI, ""))) done(true);
    };
    const done = (v) => {
      clearTimeout(timer);
      proc.listeners = proc.listeners.filter((f) => f !== check);
      res(v);
    };
    const timer = setTimeout(() => done(false), ms);
    proc.listeners.push(check);
    proc.exited.then(() => done(re.test(proc.text.replace(ANSI, ""))));
    check();
  });
}

// ----- checks ----------------------------------------------------------------------

const results = [];
function check(name, ok, detail) {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? ` - ${detail}` : ""}`);
}

/** An informational line (not a check). */
function note(text) {
  console.log(`      ${text}`);
}

async function httpChecks(base, webRoot) {
  const indexPath = join(webRoot, "index.html");
  if (!existsSync(indexPath)) {
    check("HTTP serves web/dist/index.html", false, `${indexPath} missing (build the web client)`);
    return;
  }
  const expected = readFileSync(indexPath, "utf8");
  try {
    const r = await fetch(`${base}/`);
    const body = await r.text();
    const type = r.headers.get("content-type") ?? "";
    check(
      "HTTP GET / serves web/dist/index.html",
      r.status === 200 && type.startsWith("text/html") && body === expected,
      `status ${r.status}, ${type}, ${body.length} bytes${body === expected ? "" : " (content differs from web/dist/index.html)"}`,
    );
    // Assets referenced by index.html must be served too.
    const assets = [...expected.matchAll(/(?:src|href)="\/?([^"]+\.(?:js|css))"/g)].map((m) => m[1]);
    for (const a of assets) {
      const ar = await fetch(`${base}/${a}`);
      await ar.arrayBuffer();
      check(`HTTP GET /${a}`, ar.status === 200, `status ${ar.status}, ${ar.headers.get("content-type")}`);
    }
  } catch (e) {
    check("HTTP GET / serves web/dist/index.html", false, String(e?.message ?? e));
  }
}

/** WebTransport: `/cert-hash` publishes the certificate hash and UDP port. */
async function certHashCheck(base, port) {
  try {
    const r = await fetch(`${base}/cert-hash`);
    const body = await r.json();
    const ok =
      r.status === 200 &&
      body.algorithm === "sha-256" &&
      /^[0-9a-f]{64}$/.test(body.hash) &&
      body.port === port &&
      body.expires * 1000 > Date.now() &&
      body.expires * 1000 <= Date.now() + 14 * 86400_000;
    check("HTTP GET /cert-hash publishes the WebTransport certificate", ok, `status ${r.status}, ${JSON.stringify(body)}`);
  } catch (e) {
    check("HTTP GET /cert-hash publishes the WebTransport certificate", false, String(e?.message ?? e));
  }
}

function checkTransport(bots, server, opts) {
  const name = opts.transport === "wt" ? "webtransport" : "websocket";
  const joined = bots.text.replace(ANSI, "").split(/\r?\n/).filter((l) => l.includes(" joined"));
  const over = joined.filter((l) => parseFields(l).transport === name);
  check(`bots joined over ${name}`, joined.length > 0 && over.length === joined.length, `${over.length} of ${joined.length} joins`);
  const closed = server.text.replace(ANSI, "").split(/\r?\n/).filter((l) => l.includes(`${name} closed`));
  check(`server served the bots over ${name}`, closed.length >= opts.count, `${closed.length} ${name} sessions closed`);
}

function checkBots(bots, exit, opts) {
  const text = bots.text.replace(ANSI, "");
  check("bots exited 0", exit.code === 0 && !exit.killed, exit.killed ? "killed after timeout" : `exit code ${exit.code}`);
  check("bots summary: result OK", /^result: OK$/m.test(text));
  const joins = text.match(
    /totals: joins (\d+) \(connect failures (\d+)\), disconnects (\d+), match restarts (\d+), inputs sent (\d+), snapshots (\d+)/,
  );
  if (!joins) {
    check("bots summary parsed", false, "no totals line");
    return;
  }
  // Joins include rejoins after a match restart (runs longer than a match).
  const [, j, , disc, , inputs, snaps] = joins.map(Number);
  check(`all ${opts.count} bots joined, none dropped`, j >= opts.count && disc === 0, `joins ${j}, disconnects ${disc}`);
  const kinds = text.match(/snapshots \d+ \((\d+) delta, (\d+) full, (\d+) stale\)/);
  if (kinds) {
    const full = Number(kinds[2]);
    note(`snapshots: ${kinds[1]} delta, ${full} full (${(full / Math.max(1, j)).toFixed(2)} per join), ${kinds[3]} stale`);
    if (opts.maxFullPerJoin > 0) {
      check(`at most ${opts.maxFullPerJoin} full snapshot(s) per join`, full <= opts.maxFullPerJoin * j, `${full} full snapshots for ${j} joins`);
    }
  } else if (opts.maxFullPerJoin > 0) {
    check("bots summary: snapshot kinds parsed", false, "no '(delta, full, stale)' in the totals line");
  }
  const pe = text.match(/protocol errors (\d+), desyncs (\d+)/);
  check("bots saw no protocol errors or desyncs", !!pe && pe[1] === "0" && pe[2] === "0", pe ? pe[0] : "line missing");
  // 30 Hz snapshots: expect at least a third of the ideal count.
  const minSnaps = Math.floor((opts.count * opts.duration * 30) / 3);
  check("snapshots reached the bots", snaps >= minSnaps, `${snaps} snapshots (min ${minSnaps}), ${inputs} inputs sent`);
}

function checkServer(server, exit, opts) {
  const text = server.text.replace(ANSI, "");
  const lines = text.split(/\r?\n/);
  const windows = lines.filter((l) => l.includes("tick stats")).map(parseFields);
  const active = windows.filter((w) => num(w.players) > 0);
  const final = parseFields(lines.find((l) => l.includes("final stats")) ?? "");

  check("server shut down cleanly", exit.code === 0 && !exit.killed && /server stopped/.test(text) && "ticks" in final,
    exit.killed ? "killed after timeout" : `exit code ${exit.code}`);
  const errors = lines.filter((l) => / ERROR |panicked/.test(l));
  check("server logged no errors or panics", errors.length === 0, errors[0]);
  const starvation = lines.filter((l) => l.includes("input starvation"));
  const desyncs = num(final.desync_warnings);
  check("server reported no desyncs", starvation.length === 0 && desyncs === 0,
    `desync_warnings ${final.desync_warnings}${starvation[0] ? `; ${starvation[0]}` : ""}`);

  if (active.length < 2) {
    check("server stats windows with players", false, `${active.length} window(s); run longer or lower --stats-interval`);
    return;
  }
  const p99s = active.map((w) => num(w.p99_us));
  const worst = Math.max(...p99s);
  check(`tick p99 < ${opts.p99BudgetUs} us in every window`, worst < opts.p99BudgetUs,
    `worst ${worst} us, max ${Math.max(...active.map((w) => num(w.max_us)))} us over ${active.length} windows`);
  // Peak within each window (debris_peak), else the end-of-window sample.
  const debrisOf = (w) => ("debris_peak" in w ? num(w.debris_peak) : num(w.debris));
  const debris = Math.max(...active.map(debrisOf));
  if (opts.minDebris > 0) {
    check(`debris reached ${opts.minDebris} pieces`, debris >= opts.minDebris, `peak ${debris} networked debris pieces`);
  } else {
    note(`peak ${debris} networked debris pieces`);
  }
  // Tick p99 while the debris was at its peak.
  const atCap = active.filter((w) => debrisOf(w) >= Math.max(1, debris));
  if (atCap.length) note(`windows at peak debris: ${atCap.length}, worst p99 ${Math.max(...atCap.map((w) => num(w.p99_us)))} us`);
  const skipped = num(final.skipped_ticks);
  check("no skipped ticks", skipped === 0, `${final.skipped_ticks} skipped`);
  const peak = Math.max(...active.map((w) => num(w.players)));
  check(`server saw ${opts.count} players`, peak >= opts.count, `peak ${peak}`);
  check("snapshots flowed", num(final.snapshots_sent) > 0 && active.every((w) => num(w.snapshot_kib_s) > 0),
    `${final.snapshots_sent} snapshots, ${(num(final.snapshot_bytes) / 1048576).toFixed(1)} MiB`);

  // Growth checks compare windows of the first match (a long run may go on
  // into the next match, which starts from a fresh level).
  const first = active[0];
  const firstMatch = active.filter((w) => w.matches === first.matches);
  const last = firstMatch[firstMatch.length - 1];
  const edits = [num(first.edits), num(last.edits)];
  check("terrain edits grew", edits[1] > edits[0] && edits[1] > 0, `edits ${edits[0]} -> ${edits[1]}`);
  if (opts.minCraters > 0) {
    check(`craters carved (>= ${opts.minCraters})`, num(last.craters) >= opts.minCraters, `${last.craters} crater edits`);
  }
  const paint = (w) => num(w.paint_cells_0) + num(w.paint_cells_1);
  const pct = (w) => ((100 * paint(w)) / Math.max(1, num(w.surface_cells))).toFixed(2);
  check("paint coverage grew", paint(last) > paint(first),
    `${paint(first)} -> ${paint(last)} cells (${pct(first)}% -> ${pct(last)}%; team0 ${last.paint_cells_0}, team1 ${last.paint_cells_1})`);
  const completed = active.some((w) => w.match_over === "true") || num(final.matches) > 1;
  const left = [num(first.match_left_s), num(last.match_left_s)];
  check("match timer progressed or completed", completed || left[1] < left[0],
    completed ? "a match completed" : `time left ${left[0].toFixed(1)} s -> ${left[1].toFixed(1)} s`);
}

// ----- main ------------------------------------------------------------------------

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  mkdirSync(opts.logDir, { recursive: true });
  const port = opts.port || (await freePort());
  const base = `http://127.0.0.1:${port}`;
  console.log(`e2e: ${opts.count} bots (${opts.transport}) for ${opts.duration} s on ${base}, code ${opts.code}, logs in ${opts.logDir}`);

  const server = run(
    "server",
    opts.serverBin,
    [
      "--bind", `127.0.0.1:${port}`,
      // No lobby (warm-up with weapons off): bots play from the first second.
      "--warmup", "0", "--countdown", "0",
      "--code", opts.code,
      "--ruleset", opts.ruleset,
      "--web-root", opts.webRoot,
      "--stats-interval", String(opts.statsInterval),
      "--exit-when-empty",
    ],
    join(opts.logDir, "server.log"),
    (l) => /listening|WARN|ERROR|final stats/.test(l),
  );
  // Level generation takes ~2 s in release, ~30 s in debug.
  if (!(await waitForLine(server, /listening \(http \+ websocket/, 120_000))) {
    check("server started", false, "no 'listening' line within 120 s");
    server.child.kill();
    await server.exited;
    return finish(opts);
  }
  check("server started", true);
  await httpChecks(base, opts.webRoot);
  if (opts.transport === "wt") await certHashCheck(base, port);

  const bots = run(
    "bots",
    opts.botsBin,
    [
      "--server", `ws://127.0.0.1:${port}/ws`,
      "--count", String(opts.count),
      "--duration", String(opts.duration),
      "--transport", opts.transport,
    ],
    join(opts.logDir, "bots.log"),
    (l) => /WARN|ERROR|^result:/.test(l),
  );
  const botsExit = await waitExit(bots, (opts.duration + 60) * 1000);
  // --exit-when-empty: the server stops ~2 s after the last bot left.
  const serverExit = await waitExit(server, 30_000);

  checkBots(bots, botsExit, opts);
  checkServer(server, serverExit, opts);
  checkTransport(bots, server, opts);
  finish(opts);
}

function finish(opts) {
  const failed = results.filter((r) => !r.ok);
  console.log(
    failed.length === 0
      ? `\ne2e: all ${results.length} checks passed`
      : `\ne2e: ${failed.length} of ${results.length} checks failed (logs: ${opts.logDir})`,
  );
  process.exit(failed.length === 0 ? 0 : 1);
}

await main();
