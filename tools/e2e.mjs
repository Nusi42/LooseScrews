#!/usr/bin/env node
// End-to-end soak run: ls-server + N ls-bots over real WebSockets (or
// WebTransport with --transport wt), then checks the logs. Called by
// tools/e2e.ps1 and tools/e2e.sh after they built the release binaries and
// the web client; can also be run directly.
//
//   node tools/e2e.mjs --server-bin <ls-server> --bots-bin <ls-bots> [options]
//
// Options (defaults in brackets):
//   --code <share code>     world [LS0C-0000-0000-0000-8006-8001-8F9M]
//   --ruleset <file>        rule set [data/rulesets/paint_the_void.ron]
//   --count <n>             bots [7]
//   --duration <s>          bot run time [90; with --playlist: the rotation's length]
//   --port <n>              server port; 0 picks a free one [0]
//   --web-root <dir>        served web client [web/dist]
//   --stats-interval <s>    server stats window [5]
//   --p99-budget-us <n>     tick p99 limit [4000]
//   --min-debris <n>        networked debris pieces some stats window must
//                           reach (64 = the cap; 0 = no check) [0]
//   --max-full-per-join <n> full snapshots the bots may receive per join
//                           (with --playlist: per join or in-place match
//                           transition; 0 = no check) [0; 1 with --playlist]
//   --min-craters <n>       crater edits required (0 for levels where bots
//                           only paint) [1]
//   --log-dir <dir>         where server.log / bots.log go [target/e2e-logs]
//   --transport ws|wt       bot transport: WebSocket or WebTransport (then
//                           also checks /cert-hash and that the bots joined
//                           over WebTransport) [ws]
//   --playlist <p>          a rotation instead of one rule set (--code and
//                           --ruleset are then ignored): `quick` = every mode
//                           (Team Brawl, The Squeeze, Paint the Void, Hot
//                           Reactor, Heist, Last Color Standing) plus Instagib
//                           Brawl, each shortened to --match-secs, on random
//                           worlds (written to <log-dir>/playlist/);
//                           `default` = the server's built-in rotation at full
//                           length (the M3 soak: --duration 3600 --count 8);
//                           or a playlist file [none]
//   --match-secs <s>        match length of the quick playlist [45]
//   --warmup <s>            lobby warm-up; 0 together with --countdown 0
//                           turns the lobby off [0; 10 with --playlist]
//   --countdown <s>         lobby countdown [0; 2 with --playlist]
//   --intermission <s>      results time between matches [10; 3 with --playlist]
//   --directory <d>         `local` starts an ls-directory on a free port, a
//                           URL names a running one; the server registers
//                           with it and the server list is checked [none]
//   --directory-bin <path>  ls-directory binary [next to --server-bin]
//   --bots-directory auto|on|off
//                           bots find the server through the directory
//                           (`ls-bots --directory <url>`; auto: when the bots
//                           binary has the flag) [auto]
//   --private-rooms <spec>  private rooms next to the public one, each with
//                           its own bots: `rules:bots,...` with a built-in
//                           rule set id or LR code, e.g. `duel:2,team_brawl:4`
//                           (opened with POST /api/rooms; logs in
//                           bots-<room>.log) [none]
//   --welcome-probes on|off a one-bot join right after every match end and
//                           warm-up swap of the public room, i.e. while the
//                           next room is being built; checks its Welcome
//                           time (the server then gets one player slot
//                           more than --count) [on with --playlist, else off]
//   --welcome-budget-ms <n> longest Welcome wait of a probe [200]
//
// Checks: the server serves web/dist/index.html (and its assets); bots exit 0
// with every bot joined and no protocol errors or desyncs; the server logs no
// desync (input starvation) warning, no errors and no skipped ticks; tick p99
// stays under budget in every window with players; snapshots flowed; the
// server shut down cleanly (--exit-when-empty) with exit code 0.
// One rule set: terrain edits (including craters from combat) and paint
// coverage grew in the first match; the match timer progressed or completed.
// --playlist: every playlist entry played a match to its result (mode,
// winner, time left are printed), tick p99 per entry; transitions without
// reconnects: every bot joined exactly once, no disconnects, the bots' match
// restarts equal the players the server moved into each next room, nobody
// refused by a next room, full snapshots per join or transition within
// --max-full-per-join; craters in some match; paint grew in Paint the Void.
// --directory: the server is listed (name, address, players, protocol and
// data hash as its /api/info; with --playlist the listed mode and world
// follow the rotation) and removed when it shuts down.
// --private-rooms: per room, its bots joined once and stayed (no disconnects,
// protocol errors or desyncs), full snapshots per join or transition within
// --max-full-per-join, a match reached its result, tick p99 under budget.
// --welcome-probes: every probe got its Welcome within --welcome-budget-ms.

import { spawn, spawnSync } from "node:child_process";
import { createWriteStream, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createSocket } from "node:dgram";
import { createServer } from "node:net";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { assertWorkFolder } from "./lib/work-folder.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// ----- arguments ---------------------------------------------------------------

const defaults = {
  "server-bin": "",
  "bots-bin": "",
  code: "LS0C-0000-0000-0000-8006-8001-8F9M",
  ruleset: join(repoRoot, "data", "rulesets", "paint_the_void.ron"),
  count: "7",
  duration: "",
  port: "0",
  "web-root": join(repoRoot, "web", "dist"),
  "stats-interval": "5",
  "p99-budget-us": "4000",
  "min-craters": "1",
  "min-debris": "0",
  "max-full-per-join": "",
  "log-dir": join(repoRoot, "target", "e2e-logs"),
  transport: "ws",
  playlist: "",
  "match-secs": "45",
  warmup: "",
  countdown: "",
  intermission: "",
  directory: "",
  "directory-bin": "",
  "bots-directory": "auto",
  "private-rooms": "",
  "welcome-probes": "",
  "welcome-budget-ms": "200",
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
  if (!["auto", "on", "off"].includes(o["bots-directory"])) die("--bots-directory must be auto, on or off");
  if (!["", "on", "off"].includes(o["welcome-probes"])) die("--welcome-probes must be on or off");
  let privateRooms;
  try {
    privateRooms = parsePrivateRooms(o["private-rooms"]);
  } catch (e) {
    die(e.message);
  }
  const rotating = o.playlist !== "";
  if (rotating && o.playlist !== "quick" && o.playlist !== "default") {
    o.playlist = resolve(o.playlist);
    if (!existsSync(o.playlist)) die(`playlist not found: ${o.playlist}`);
  }
  const secs = (k, dflt) => {
    if (o[k] === "") return dflt;
    const n = Number(o[k]);
    if (!(n >= 0)) die(`--${k} must be a number >= 0`);
    return n;
  };
  let directoryBin = "";
  if (o.directory) {
    if (o.directory !== "local" && !/^https?:\/\/\S+$/.test(o.directory)) die("--directory must be `local` or an http(s) URL");
    if (o.directory === "local") {
      const exe = process.platform === "win32" ? ".exe" : "";
      directoryBin = resolve(o["directory-bin"] || join(dirname(o["server-bin"]), `ls-directory${exe}`));
      if (!existsSync(directoryBin)) die(`ls-directory not found: ${directoryBin} (cargo build --release -p ls-directory)`);
    }
  }
  return {
    serverBin: o["server-bin"],
    botsBin: o["bots-bin"],
    code: o.code,
    ruleset: o.ruleset,
    count: posInt(o.count, "count"),
    // 0: computed from the playlist (main).
    duration: o.duration === "" ? (rotating ? 0 : 90) : posNum(o.duration, "duration"),
    port: Number.parseInt(o.port, 10) || 0,
    webRoot: o["web-root"],
    statsInterval: posNum(o["stats-interval"], "stats-interval"),
    p99BudgetUs: posInt(o["p99-budget-us"], "p99-budget-us"),
    minCraters: Math.max(0, Number.parseInt(o["min-craters"], 10) || 0),
    minDebris: Math.max(0, Number.parseInt(o["min-debris"], 10) || 0),
    maxFullPerJoin:
      o["max-full-per-join"] === "" ? (rotating ? 1 : 0) : Math.max(0, Number(o["max-full-per-join"]) || 0),
    logDir: o["log-dir"],
    transport: o.transport,
    playlist: o.playlist,
    matchSecs: posNum(o["match-secs"], "match-secs"),
    warmup: secs("warmup", rotating ? 10 : 0),
    countdown: secs("countdown", rotating ? 2 : 0),
    intermission: secs("intermission", rotating ? 3 : 10),
    directory: o.directory,
    directoryBin,
    botsDirectory: o["bots-directory"],
    privateRooms,
    welcomeProbes: o["welcome-probes"] === "" ? rotating : o["welcome-probes"] === "on",
    welcomeBudgetMs: posNum(o["welcome-budget-ms"], "welcome-budget-ms"),
  };
}

/** `rules:bots,...` -> [{rules, bots}] (exported for tests). */
export function parsePrivateRooms(spec) {
  if (!spec) return [];
  return spec.split(",").map((part) => {
    const m = /^\s*([A-Za-z0-9_-]+):(\d+)\s*$/.exec(part);
    if (!m || !(Number(m[2]) > 0)) throw new Error(`--private-rooms: bad entry "${part}" (want rules:bots)`);
    return { rules: m[1], bots: Number(m[2]) };
  });
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

// ----- playlists ------------------------------------------------------------------

/**
 * The quick rotation: built-in rule set file, and mode settings to override
 * (`null` = --match-secs). Every mode of the default rotation, plus Instagib.
 */
export const QUICK_ENTRIES = [
  ["team_brawl", {}],
  // All five phases close inside the match: 3 s, then 5 x (3 s warning + 12 s close); phase 1 moves the
  // wall about 6 u/s (2.5x the default rule set: faster walls make bots mispredict, see the M3 plan).
  ["squeeze", { first_phase_secs: 3, warn_secs: 3, close_secs: 12, match_secs: "+60" }],
  ["paint_the_void", {}],
  ["hot_reactor", {}],
  ["heist", {}],
  // The stalemate drain starts half way through.
  ["last_color_standing", { stalemate_secs: "half", stalemate_drain_secs: 0.5 }],
  ["instagib_brawl", {}],
];

/** Sets `field: <number>` in a rule set's RON text; throws when the field is missing. */
export function setRonField(text, field, value) {
  const re = new RegExp(`(\\b${field}:\\s*)-?[0-9.]+`);
  if (!re.test(text)) throw new Error(`rule set has no ${field}`);
  return text.replace(re, `$1${Number.isInteger(value) ? value.toFixed(1) : value}`);
}

/** A built-in rule set's text with the quick playlist's overrides. */
export function quickRuleSet(text, overrides, matchSecs) {
  let out = setRonField(text, "match_secs", matchSecs);
  for (const [k, v] of Object.entries(overrides)) {
    let value = v;
    if (v === "+60") value = matchSecs + 60;
    else if (v === "half") value = Math.max(1, Math.round(matchSecs / 2));
    out = setRonField(out, k, value);
  }
  return out;
}

/** The longest a match of `text` can run (s), for the run-time estimate. */
export function matchLength(text) {
  const f = (k) => Number(new RegExp(`\\b${k}:\\s*([0-9.]+)`).exec(text)?.[1] ?? Number.NaN);
  let secs = f("match_secs");
  const first = f("first_phase_secs");
  if (Number.isFinite(first)) {
    // The Squeeze seals after its phases and a final close of 4 close times.
    const sealed = first + f("phases") * (f("warn_secs") + f("close_secs")) + 4 * f("close_secs");
    if (Number.isFinite(sealed)) secs = Math.min(secs, sealed);
  }
  return Number.isFinite(secs) ? secs : 600;
}

/**
 * Writes the quick playlist into `dir`; returns `{file, entries}` with each
 * entry's `source` (as the server logs it) and longest match length.
 */
function writeQuickPlaylist(dir, matchSecs) {
  mkdirSync(dir, { recursive: true });
  const entries = [];
  const lines = ["// Generated by tools/e2e.mjs --playlist quick: every mode, shortened.", "Playlist(", "    entries: ["];
  for (const [id, overrides] of QUICK_ENTRIES) {
    const src = readFileSync(join(repoRoot, "data", "rulesets", `${id}.ron`), "utf8");
    const text = quickRuleSet(src, overrides, matchSecs);
    const file = `${id}.ron`;
    writeFileSync(join(dir, file), text);
    lines.push(`        (rules: "${file}", world: Random),`);
    entries.push({ source: file, secs: matchLength(text) });
  }
  lines.push("    ],", ")", "");
  const file = join(dir, "quick.ron");
  writeFileSync(file, lines.join("\n"));
  return { file, entries };
}

/** The server's default rotation (`DEFAULT_ROTATION` in ls-server's playlist.rs). */
function defaultPlaylist(count) {
  const ids = [
    ["team_brawl", 4],
    ["squeeze", 2],
    ["paint_the_void", 1],
    ["hot_reactor", 2],
    ["heist", 2],
    ["last_color_standing", 2],
  ];
  return {
    file: null,
    entries: ids
      .filter(([, min]) => count >= min)
      .map(([id]) => ({
        // The server logs the built-in id; The Squeeze's is `the_squeeze` or `squeeze`.
        source: id === "squeeze" ? "the_squeeze|squeeze" : id,
        secs: matchLength(readFileSync(join(repoRoot, "data", "rulesets", `${id}.ron`), "utf8")),
      })),
  };
}

/** A playlist file's entries (rules strings), for the per-entry checks. */
function filePlaylist(file) {
  const text = readFileSync(file, "utf8");
  const entries = [...text.matchAll(/rules:\s*"([^"]+)"/g)].map((m) => {
    let secs = 600;
    const path = resolve(dirname(file), m[1]);
    const builtin = join(repoRoot, "data", "rulesets", `${m[1]}.ron`);
    for (const p of [path, builtin]) if (existsSync(p)) secs = matchLength(readFileSync(p, "utf8"));
    return { source: m[1], secs };
  });
  return { file, entries };
}

const sourceMatches = (source, logged) => source.split("|").includes(logged);

// ----- process helpers -----------------------------------------------------------

function freeTcpPort() {
  return new Promise((res, rej) => {
    const s = createServer();
    s.once("error", rej);
    s.listen(0, "127.0.0.1", () => {
      const { port } = s.address();
      s.close(() => res(port));
    });
  });
}

/** True when UDP `port` can be bound on 127.0.0.1. */
function udpFree(port) {
  return new Promise((res) => {
    const s = createSocket("udp4");
    s.once("error", () => {
      s.close();
      res(false);
    });
    s.bind(port, "127.0.0.1", () => s.close(() => res(true)));
  });
}

/**
 * A port free for TCP and UDP: the server binds WebTransport (UDP) on its
 * game port too, and Windows reserves UDP ranges (Hyper-V, WinNAT) that a
 * free TCP port may fall into (os error 10013).
 */
async function freePort() {
  for (let i = 0; i < 20; i++) {
    const port = await freeTcpPort();
    if (await udpFree(port)) return port;
  }
  return freeTcpPort();
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

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function getJson(url) {
  const r = await fetch(url, { signal: AbortSignal.timeout(5000) });
  if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
  return r.json();
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

/** The bots' totals line, or null. */
export function botTotals(text) {
  const m = text.match(
    /totals: joins (\d+) \(connect failures (\d+)\), disconnects (\d+), match restarts (\d+), inputs sent (\d+), snapshots (\d+)(?: \((\d+) delta, (\d+) full, (\d+) stale\))?/,
  );
  if (!m) return null;
  const n = m.slice(1).map((v) => (v === undefined ? null : Number(v)));
  return {
    joins: n[0],
    connectFailures: n[1],
    disconnects: n[2],
    restarts: n[3],
    inputs: n[4],
    snapshots: n[5],
    delta: n[6],
    full: n[7],
    stale: n[8],
  };
}

function checkBots(bots, exit, opts) {
  const text = bots.text.replace(ANSI, "");
  check("bots exited 0", exit.code === 0 && !exit.killed, exit.killed ? "killed after timeout" : `exit code ${exit.code}`);
  check("bots summary: result OK", /^result: OK$/m.test(text));
  const t = botTotals(text);
  if (!t) {
    check("bots summary parsed", false, "no totals line");
    return null;
  }
  if (opts.playlist) {
    // In-place transitions: one join per bot for the whole rotation.
    check(`all ${opts.count} bots joined exactly once, none dropped`, t.joins === opts.count && t.disconnects === 0,
      `joins ${t.joins}, disconnects ${t.disconnects}, in-place transitions ${t.restarts}`);
  } else {
    // Joins include rejoins after a match restart (runs longer than a match).
    check(`all ${opts.count} bots joined, none dropped`, t.joins >= opts.count && t.disconnects === 0, `joins ${t.joins}, disconnects ${t.disconnects}`);
  }
  if (t.full !== null) {
    const per = opts.playlist ? t.joins + t.restarts : t.joins;
    const what = opts.playlist ? "join or transition" : "join";
    note(`snapshots: ${t.delta} delta, ${t.full} full (${(t.full / Math.max(1, per)).toFixed(2)} per ${what}), ${t.stale} stale`);
    if (opts.maxFullPerJoin > 0) {
      check(`at most ${opts.maxFullPerJoin} full snapshot(s) per ${what}`, t.full <= opts.maxFullPerJoin * per, `${t.full} full snapshots for ${per} ${what}s`);
    }
  } else if (opts.maxFullPerJoin > 0) {
    check("bots summary: snapshot kinds parsed", false, "no '(delta, full, stale)' in the totals line");
  }
  const pe = text.match(/protocol errors (\d+), desyncs (\d+)/);
  check("bots saw no protocol errors or desyncs", !!pe && pe[1] === "0" && pe[2] === "0", pe ? pe[0] : "line missing");
  // 30 Hz snapshots: expect at least a third of the ideal count.
  const minSnaps = Math.floor((opts.count * opts.duration * 30) / 3);
  check("snapshots reached the bots", t.snapshots >= minSnaps, `${t.snapshots} snapshots (min ${minSnaps}), ${t.inputs} inputs sent`);
  return t;
}

/** Welcome times (ms) of the bots' "joined" lines, in order (exported for tests). */
export function welcomeTimes(text) {
  return text
    .replace(ANSI, "")
    .split(/\r?\n/)
    .filter((l) => l.includes(" joined"))
    .map((l) => num(parseFields(l).welcome_ms))
    .filter((v) => Number.isFinite(v));
}

/** Lines of room `label` with `marker`, as fields (exported for tests). */
export function roomLines(text, label, marker) {
  return text
    .replace(ANSI, "")
    .split(/\r?\n/)
    .filter((l) => l.includes(marker))
    .map(parseFields)
    .filter((f) => f.room === label);
}

/** Lines of the public room with `marker`, as fields. */
function publicLines(lines, marker) {
  return lines
    .filter((l) => l.includes(marker))
    .map(parseFields)
    .filter((f) => f.room === undefined || f.room === "public");
}

function checkServer(server, exit, opts) {
  const text = server.text.replace(ANSI, "");
  const lines = text.split(/\r?\n/);
  const windows = publicLines(lines, "tick stats");
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
  // A window reported right after a room swap counts only the new room's
  // traffic (the server's window counters restart with each room), and the
  // first snapshot of a room waits for each client's first input (its
  // session rebuild takes a few ticks): skip windows of a room younger than
  // a second.
  const flowing = active.filter((w) => !(num(w.tick) < 60));
  check("snapshots flowed", num(final.snapshots_sent) > 0 && flowing.every((w) => num(w.snapshot_kib_s) > 0),
    `${final.snapshots_sent} snapshots, ${(num(final.snapshot_bytes) / 1048576).toFixed(1)} MiB`);

  if (opts.playlist) return;
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

/**
 * The rotation from the server log: room swaps, finished matches and the
 * stats windows of each match (exported for tests).
 */
export function rotationFromLog(text) {
  const lines = text.replace(ANSI, "").split(/\r?\n/);
  const swaps = publicLines(lines, "next room swapped in");
  const overs = publicLines(lines, "match over; building the next room");
  const windows = publicLines(lines, "tick stats");
  // Match number -> playlist entry (match rooms only).
  const entryOf = new Map(swaps.filter((s) => s.warmup === "false").map((s) => [s.matches, s.entry]));
  return { swaps, overs, windows, entryOf, refused: lines.filter((l) => l.includes("not accepted by the next room")) };
}

function checkRotation(server, totals, opts, plan) {
  const { swaps, overs, windows, entryOf, refused } = rotationFromLog(server.text);
  const start = server.text.replace(ANSI, "").split(/\r?\n/).find((l) => l.includes("match ready"));
  // Without a lobby the initial room is match 1 (no swap logs its entry).
  const firstEntry = start ? parseFields(start).entry : undefined;
  if (opts.warmup === 0 && opts.countdown === 0 && firstEntry && !entryOf.has("1")) entryOf.set("1", firstEntry);
  for (const o of overs) {
    note(`match over: ${o.entry} (${o.mode}) winner ${o.winner_side}, ${num(o.left_s).toFixed(1)} s left, ${o.players} players; next ${o.next}`);
  }
  // Every playlist entry played a match to its result.
  for (const e of plan.entries) {
    const done = overs.filter((o) => sourceMatches(e.source, o.entry));
    const how = done.map((o) => `${o.mode}: ${o.winner_side === "draw" ? "draw" : `side ${o.winner_side} won`}${num(o.left_s) > 0.5 ? ` with ${num(o.left_s).toFixed(0)} s left` : " on time"}`);
    check(`${e.source.split("|").pop()}: a match reached its result`, done.length > 0, how.join("; ") || "no finished match");
  }
  // Tick p99 per entry (match windows only).
  const perEntry = new Map();
  for (const w of windows) {
    if (w.warmup !== "false" || !(num(w.players) > 0)) continue;
    const entry = entryOf.get(w.matches) ?? "?";
    const cur = perEntry.get(entry) ?? { p99: 0, max: 0, windows: 0, craters: 0, edits: 0 };
    cur.p99 = Math.max(cur.p99, num(w.p99_us));
    cur.max = Math.max(cur.max, num(w.max_us));
    cur.windows += 1;
    cur.craters = Math.max(cur.craters, num(w.craters));
    cur.edits = Math.max(cur.edits, num(w.edits));
    perEntry.set(entry, cur);
  }
  for (const [entry, s] of perEntry) {
    check(`${entry}: tick p99 < ${opts.p99BudgetUs} us`, s.p99 < opts.p99BudgetUs,
      `worst p99 ${s.p99} us, max ${s.max} us over ${s.windows} windows; ${s.edits} edits, ${s.craters} craters`);
  }
  if (opts.minCraters > 0) {
    const craters = Math.max(0, ...[...perEntry.values()].map((s) => s.craters));
    check(`craters carved in some match (>= ${opts.minCraters})`, craters >= opts.minCraters, `${craters} crater edits`);
  }
  // Paint the Void: paint grew within its match.
  const ptv = windows.filter((w) => w.warmup === "false" && /paint_the_void/.test(entryOf.get(w.matches) ?? ""));
  if (ptv.length >= 2) {
    const paint = (w) => num(w.paint_cells_0) + num(w.paint_cells_1);
    const byMatch = new Map();
    for (const w of ptv) byMatch.set(w.matches, [...(byMatch.get(w.matches) ?? []), w]);
    const grew = [...byMatch.values()].some((ws) => paint(ws[ws.length - 1]) > paint(ws[0]));
    check("Paint the Void: paint coverage grew", grew, [...byMatch.values()].map((ws) => `${paint(ws[0])} -> ${paint(ws[ws.length - 1])} cells`).join("; "));
  }
  // Transitions on the same connections.
  const moved = swaps.reduce((n, s) => n + (num(s.moved) || 0), 0);
  const matchSwaps = swaps.filter((s) => s.warmup === "false").length;
  note(`room swaps: ${swaps.length} (${matchSwaps} into a match room, ${swaps.length - matchSwaps} into a warm-up), ${moved} players moved, ${overs.length} matches finished`);
  check("every finished match was followed by the next room", swaps.length >= overs.length - 1 && overs.length > 0,
    `${overs.length} finished, ${swaps.length} swaps`);
  check("no player refused by a next room", refused.length === 0, refused[0]);
  if (totals) {
    // Bots stopping right at a swap may miss its Welcome (at most one per bot).
    const ok = totals.restarts <= moved && totals.restarts >= moved - opts.count && totals.restarts > 0;
    check("bots followed every room swap in place (match restarts = players moved)", ok,
      `bots: ${totals.restarts} in-place transitions; server: ${moved} players moved; joins ${totals.joins}`);
  }
}

/** Per private room: its bots and the room's server lines. */
function checkPrivateRooms(rooms, server, opts) {
  for (const r of rooms) {
    const name = `private ${r.rules} (${r.code ?? "not opened"})`;
    if (!r.code) {
      check(`${name}: opened`, false, r.error);
      continue;
    }
    const text = r.proc.text.replace(ANSI, "");
    check(`${name}: bots exited 0`, r.exit.code === 0 && !r.exit.killed, r.exit.killed ? "killed after timeout" : `exit code ${r.exit.code}`);
    const t = botTotals(text);
    if (!t) {
      check(`${name}: bots summary parsed`, false, "no totals line");
    } else {
      check(`${name}: all ${r.bots} bots joined exactly once, none dropped`, t.joins === r.bots && t.disconnects === 0,
        `joins ${t.joins}, disconnects ${t.disconnects}, in-place transitions ${t.restarts}`);
      const per = t.joins + t.restarts;
      if (t.full !== null && opts.maxFullPerJoin > 0) {
        check(`${name}: at most ${opts.maxFullPerJoin} full snapshot(s) per join or transition`, t.full <= opts.maxFullPerJoin * per,
          `${t.full} full snapshots for ${per} joins or transitions`);
      }
    }
    const pe = text.match(/protocol errors (\d+), desyncs (\d+)/);
    check(`${name}: no protocol errors or desyncs`, !!pe && pe[1] === "0" && pe[2] === "0", pe ? pe[0] : "line missing");
    const overs = roomLines(server.text, r.code, "match over; building the next room");
    check(`${name}: a match reached its result`, overs.length > 0,
      overs.map((o) => `${o.mode} winner ${o.winner_side}, ${num(o.left_s).toFixed(0)} s left`).join("; ") || "no finished match");
    const windows = roomLines(server.text, r.code, "tick stats").filter((w) => num(w.players) > 0);
    const p99 = Math.max(0, ...windows.map((w) => num(w.p99_us)));
    check(`${name}: tick p99 < ${opts.p99BudgetUs} us`, windows.length > 0 && p99 < opts.p99BudgetUs,
      `worst p99 ${p99} us, max ${Math.max(0, ...windows.map((w) => num(w.max_us)))} us over ${windows.length} windows`);
  }
}

function checkProbes(probes, opts) {
  const times = probes.map((p) => ({ ...p, ms: welcomeTimes(p.proc.text)[0] }));
  const joined = times.filter((p) => Number.isFinite(p.ms));
  const worst = joined.reduce((a, p) => (p.ms > (a?.ms ?? -1) ? p : a), null);
  check("welcome probes joined during next-room builds", probes.length > 0 && joined.length === probes.length,
    `${joined.length} of ${probes.length} probes joined`);
  if (joined.length) {
    const sorted = joined.map((p) => p.ms).sort((a, b) => a - b);
    check(`every probe's Welcome within ${opts.welcomeBudgetMs} ms`, worst.ms <= opts.welcomeBudgetMs,
      `median ${sorted[Math.floor(sorted.length / 2)].toFixed(1)} ms, worst ${worst.ms.toFixed(1)} ms (after ${worst.trigger})`);
  }
}

/** Polls the directory's `GET /servers` until `stop()`; returns the snapshots. */
async function pollDirectory(url, stop) {
  const snaps = [];
  while (!stop()) {
    try {
      const body = await getJson(`${url}/servers`);
      snaps.push({ t: Date.now(), servers: body.servers ?? [] });
    } catch (e) {
      snaps.push({ t: Date.now(), error: String(e?.message ?? e), servers: [] });
    }
    await sleep(2000);
  }
  return snaps;
}

function checkDirectory(snaps, info, address, afterShutdown, opts) {
  const mine = snaps.flatMap((s) => s.servers.filter((e) => e.server?.address === address).map((e) => e.server));
  check("directory lists the server", mine.length > 0, `${mine.length} of ${snaps.length} polls (${snaps.filter((s) => s.error).length} failed)`);
  if (!mine.length) return;
  const names = new Set(mine.map((s) => s.name));
  check("directory: name and region as given", names.size === 1 && names.has("e2e") && mine.every((s) => s.region === "local"), [...names].join(", "));
  const peak = Math.max(...mine.map((s) => s.players));
  check(`directory: players reached ${opts.count}`, peak >= opts.count, `peak ${peak}/${mine[0].max_players}`);
  if (info) {
    check("directory: protocol and data hash as /api/info", mine.every((s) => s.protocol === info.protocol && s.data_hash === info.data_hash),
      `protocol ${mine[0].protocol}, data hash ${mine[0].data_hash}; /api/info ${info.protocol}, ${info.data_hash}`);
  }
  const roster = Math.max(...mine.map((s) => (s.roster ?? []).length));
  note(`directory roster peak ${roster} (browser presence only: bots do not poll /api/presence)`);
  if (opts.playlist) {
    const modes = [...new Set(mine.map((s) => s.mode))];
    const worlds = new Set(mine.map((s) => s.map_code));
    check("directory: the listed mode and world follow the rotation", modes.length >= 2 && worlds.size >= 2, `modes ${modes.join(", ")}; ${worlds.size} worlds`);
  }
  check("directory: the server is removed when it shuts down", afterShutdown !== null && !afterShutdown.some((e) => e.server?.address === address),
    afterShutdown === null ? "directory unreachable after the run" : `${afterShutdown.length} server(s) listed after the shutdown`);
}

// ----- main ------------------------------------------------------------------------

async function main() {
  assertWorkFolder("tools/e2e.mjs");
  const opts = parseArgs(process.argv.slice(2));
  mkdirSync(opts.logDir, { recursive: true });
  const port = opts.port || (await freePort());
  const base = `http://127.0.0.1:${port}`;

  let plan = null;
  if (opts.playlist === "quick") plan = writeQuickPlaylist(join(opts.logDir, "playlist"), opts.matchSecs);
  else if (opts.playlist === "default") plan = defaultPlaylist(opts.count);
  else if (opts.playlist) plan = filePlaylist(opts.playlist);
  if (plan && opts.duration === 0) {
    // Every entry once: warm-up, countdown, the longest match, results, the
    // level build; plus the bots' staggered joins.
    const overhead = opts.warmup + opts.countdown + opts.intermission + 8;
    opts.duration = Math.ceil(plan.entries.reduce((s, e) => s + e.secs + overhead, 0) + 0.75 * opts.count + 10);
  }
  const what = plan ? `playlist ${opts.playlist === "quick" || opts.playlist === "default" ? opts.playlist : plan.file} (${plan.entries.length} entries)` : `code ${opts.code}`;
  console.log(`e2e: ${opts.count} bots (${opts.transport}) for ${opts.duration} s on ${base}, ${what}, logs in ${opts.logDir}`);

  // --- directory ---
  let directory = null;
  let directoryUrl = opts.directory && opts.directory !== "local" ? opts.directory.replace(/\/+$/, "") : "";
  if (opts.directory === "local") {
    const dport = await freePort();
    directory = run("directory", opts.directoryBin, ["--bind", `127.0.0.1:${dport}`, "--heartbeat", "1", "--ttl", "5"], join(opts.logDir, "directory.log"),
      (l) => /WARN|ERROR|listening/.test(l));
    if (!(await waitForLine(directory, /directory listening/, 30_000))) {
      check("directory started", false, "no 'directory listening' line within 30 s");
      directory.child.kill();
      return finish(opts);
    }
    directoryUrl = `http://127.0.0.1:${dport}`;
    check("directory started", true, directoryUrl);
  }

  const lobby = ["--warmup", String(opts.warmup), "--countdown", String(opts.countdown), "--intermission", String(opts.intermission)];
  const rules = plan ? (plan.file ? ["--playlist", plan.file] : []) : ["--code", opts.code, "--ruleset", opts.ruleset];
  const listing = directoryUrl
    ? ["--directory", directoryUrl, "--server-name", "e2e", "--region", "local", "--public-address", `127.0.0.1:${port}`]
    : [];
  const server = run(
    "server",
    opts.serverBin,
    [
      "--bind", `127.0.0.1:${port}`,
      ...lobby,
      ...rules,
      ...listing,
      "--web-root", opts.webRoot,
      "--stats-interval", String(opts.statsInterval),
      // A free slot for the Welcome probes next to the bots.
      ...(opts.welcomeProbes ? ["--max-players", String(Math.max(8, opts.count + 1))] : []),
      "--exit-when-empty",
    ],
    join(opts.logDir, "server.log"),
    (l) => /listening|WARN|ERROR|final stats|match over|registered/.test(l),
  );
  // Level generation takes ~2 s in release, ~30 s in debug.
  if (!(await waitForLine(server, /listening \(http \+ websocket/, 120_000))) {
    check("server started", false, "no 'listening' line within 120 s");
    server.child.kill();
    await server.exited;
    directory?.child.kill();
    return finish(opts);
  }
  check("server started", true);
  await httpChecks(base, opts.webRoot);
  if (opts.transport === "wt") await certHashCheck(base, port);
  let info = null;
  try {
    info = await getJson(`${base}/api/info`);
  } catch (e) {
    note(`GET /api/info failed: ${e?.message ?? e}`);
  }

  // Bots find the server through the directory when they can.
  let viaDirectory = false;
  if (directoryUrl && opts.botsDirectory !== "off") {
    const help = spawnSync(opts.botsBin, ["--help"], { encoding: "utf8", windowsHide: true });
    const has = /--directory\b/.test(`${help.stdout}${help.stderr}`);
    if (opts.botsDirectory === "on" && !has) {
      check("ls-bots supports --directory", false, "the bots binary has no --directory flag");
    }
    viaDirectory = has;
  }
  const target = viaDirectory ? ["--directory", directoryUrl] : ["--server", `ws://127.0.0.1:${port}/ws`];
  if (directoryUrl) note(`bots join ${viaDirectory ? `through the directory ${directoryUrl}` : "by address"}`);
  const bots = run(
    "bots",
    opts.botsBin,
    [...target, "--count", String(opts.count), "--duration", String(opts.duration), "--transport", opts.transport],
    join(opts.logDir, "bots.log"),
    (l) => /WARN|ERROR|^result:/.test(l) && !l.includes("slow bot step"),
  );
  // Private rooms, each with its own bots process.
  const wsUrl = `ws://127.0.0.1:${port}/ws`;
  const privateRooms = [];
  for (const [i, r] of opts.privateRooms.entries()) {
    const room = { ...r, code: null, error: "", proc: null, exit: null };
    privateRooms.push(room);
    try {
      const reply = await fetch(`${base}/api/rooms`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ rules_code: r.rules }),
        signal: AbortSignal.timeout(10_000),
      });
      const body = await reply.json();
      if (!reply.ok || !body.room) throw new Error(`HTTP ${reply.status}: ${JSON.stringify(body)}`);
      room.code = body.room;
    } catch (e) {
      room.error = String(e?.message ?? e);
      continue;
    }
    note(`private room ${room.code}: ${r.rules}, ${r.bots} bots`);
    room.proc = run(
      `room${i + 1}`,
      opts.botsBin,
      ["--server", wsUrl, "--room", room.code, "--count", String(r.bots), "--duration", String(opts.duration),
        "--transport", opts.transport, "--name-prefix", `room${i + 1}bot`, "--seed", String(1000 + i)],
      join(opts.logDir, `bots-${room.code}.log`),
      (l) => /WARN|ERROR|^result:/.test(l) && !l.includes("slow bot step"),
    );
  }

  // Welcome probes: a one-bot join right after the public room's match end
  // or warm-up swap (the next room is being built on a worker thread then).
  const probes = [];
  let probing = opts.welcomeProbes;
  if (probing) {
    mkdirSync(join(opts.logDir, "probes"), { recursive: true });
    let seen = 0;
    server.listeners.push(() => {
      if (!probing) return;
      const end = server.text.lastIndexOf("\n");
      if (end <= seen) return;
      const fresh = server.text.slice(seen, end).replace(ANSI, "").split(/\r?\n/);
      seen = end;
      for (const l of fresh) {
        const f = parseFields(l);
        if (f.room !== "public") continue;
        const trigger = l.includes("match over; building the next room")
          ? `match ${f.entry} over`
          : l.includes("next room swapped in") && f.warmup === "true"
            ? `warm-up swap (${f.entry})`
            : null;
        if (!trigger) continue;
        const n = probes.length + 1;
        const proc = run(
          `probe${n}`,
          opts.botsBin,
          ["--server", wsUrl, "--count", "1", "--duration", "1", "--stagger-ms", "0", "--transport", opts.transport,
            "--name-prefix", `probe${n}-`, "--seed", String(5000 + n)],
          join(opts.logDir, "probes", `probe${n}.log`),
          null,
        );
        probes.push({ trigger, proc });
      }
    });
  }

  let polling = Boolean(directoryUrl);
  const dirSnaps = directoryUrl ? pollDirectory(directoryUrl, () => !polling) : Promise.resolve([]);
  const botsExit = await waitExit(bots, (opts.duration + 60) * 1000);
  probing = false;
  for (const r of privateRooms) if (r.proc) r.exit = await waitExit(r.proc, 60_000);
  for (const p of probes) await waitExit(p.proc, 30_000);
  // --exit-when-empty: the server stops ~2 s after the last bot left.
  const serverExit = await waitExit(server, 30_000);
  polling = false;
  const snaps = await dirSnaps;
  let afterShutdown = null;
  if (directoryUrl) {
    try {
      afterShutdown = (await getJson(`${directoryUrl}/servers`)).servers ?? [];
    } catch {
      afterShutdown = null;
    }
  }
  if (directory) {
    directory.child.kill();
    await directory.exited;
  }

  const totals = checkBots(bots, botsExit, opts);
  checkServer(server, serverExit, opts);
  if (plan) checkRotation(server, totals, opts, plan);
  checkTransport(bots, server, opts);
  if (privateRooms.length) checkPrivateRooms(privateRooms, server, opts);
  if (opts.welcomeProbes) checkProbes(probes, opts);
  if (directoryUrl) checkDirectory(snaps, info, `127.0.0.1:${port}`, afterShutdown, opts);
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

// Run only as a script (the helpers are imported by tools/test).
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
