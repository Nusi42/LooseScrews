// Runs the real ls-wasm build under Node (skipped when ./pkg has not been
// built; `npm run build` builds it). Checks the determinism contract (golden
// hashes identical to the native fixture) and smoke-tests the game API.

import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { parseEntities, parsePlayers, parseVisuals, loadoutState } from "../game/views.ts";
import { SHAPE_STYLES } from "../game/scene.ts";
import { CUE, parseCharge, parseCues, parseShipChoices } from "../game/mechanics.ts";
import {
  CONTROLS_LEN,
  DEBUG_STATS_LEN,
  MATCH_INFO_LEN,
  PROJECTILE_STRIDE,
  SCORE_STRIDE,
  SHIP_STRIDE,
  type WasmApi,
} from "./api.ts";

const pkgJsUrl = new URL("./pkg/ls_wasm.js", import.meta.url);
const pkgJs = fileURLToPath(pkgJsUrl);
const pkgWasm = fileURLToPath(new URL("./pkg/ls_wasm_bg.wasm", import.meta.url));
const fixture = fileURLToPath(new URL("../../../crates/ls-worldgen/tests/fixtures/golden_hashes.txt", import.meta.url));
const built = existsSync(pkgJs) && existsSync(pkgWasm);
const skip = built ? false : "ls-wasm package not built (npm run build:wasm)";

let api: WasmApi | null = null;
async function load(): Promise<WasmApi> {
  if (api) return api;
  const pkg = (await import(pkgJsUrl.href)) as WasmApi & { initSync(opts: { module: BufferSource }): unknown; init(): void };
  pkg.initSync({ module: readFileSync(pkgWasm) });
  pkg.init();
  api = pkg;
  return pkg;
}

test("WASM exports the match restart reason", { skip }, async () => {
  const wasm = await load();
  assert.match(wasm.matchRestartReason(), /match over/);
});

test("WASM golden hashes match the native fixture", { skip }, async () => {
  const wasm = await load();
  const lines = readFileSync(fixture, "utf8")
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l !== "" && !l.startsWith("#"));
  assert.ok(lines.length >= 1);
  // The first two codes keep the test quick; tools/ runs the full list.
  for (const line of lines.slice(0, 2)) {
    const [code = "", ...expected] = line.split(/\s+/);
    assert.deepEqual(wasm.goldenHashes(code), expected, code);
  }
});

test("WASM share-code check, hello packet and practice game", { skip }, async () => {
  const wasm = await load();
  const ok = wasm.checkShareCode("ls08 0000 0000 000a k4m6 8001 8f1t");
  assert.equal(ok.ok, true);
  assert.equal(ok.code, "LS08-0000-0000-000A-K4M6-8001-8F1T");
  ok.free();
  const bad = wasm.checkShareCode("LS08-0000-0000-000A-K4M7-8001-8F1T");
  assert.equal(bad.ok, false);
  assert.match(bad.error, /checksum/i);
  bad.free();
  assert.equal(wasm.tickHz(), 60);
  assert.ok(wasm.helloPacket("tester", -1).length > 4);
  const ids = wasm.weaponIds();
  assert.equal(ids.length, 27, "every design-doc weapon");
  assert.deepEqual(ids, [...ids].sort(), "weapon ids in ls-sim id order");
  // Every visual shape has a client style (new weapons need no client code).
  for (const shape of wasm.visualShapeNames()) assert.ok(SHAPE_STYLES[shape], `style for ${shape}`);
  assert.equal(wasm.entityKindNames()[0], "debris");

  const code = wasm.randomShareCode(1, 2, 45, 35);
  const game = wasm.Game.practice(code, 1);
  try {
    assert.equal(game.isOnline, false);
    assert.equal(game.shareCode, code);
    const start = game.localPose();
    assert.ok(start && start.length === 11);
    const controls = new Float32Array(CONTROLS_LEN);
    controls[0] = 1;
    for (let i = 0; i < 30; i++) assert.deepEqual(game.frame(1 / 60, controls), []);
    const end = game.localPose()!;
    assert.ok(Math.hypot(end[0]! - start[0]!, end[1]! - start[1]!, end[2]! - start[2]!) > 1);
    assert.equal(game.remoteShips().length, SHIP_STRIDE, "one dummy");
    const mesh = game.meshNext(end[0]!, end[1]!, end[2]!);
    assert.ok(mesh);
    assert.match(mesh.key, /^-?\d+,-?\d+,-?\d+$/);
    assert.equal(mesh.positions().length, mesh.normals().length);
    assert.equal(mesh.teams().length * 3, mesh.positions().length);
    mesh.free();
    const hud = game.hud();
    assert.equal(hud.hull, hud.hullMax);
    hud.free();
    // Overlay arrays in their documented layouts (wasm/api.ts).
    const stats = game.debugStats();
    assert.equal(stats.length, DEBUG_STATS_LEN);
    assert.equal(stats[0], -1, "no ping offline");
    const rows = game.scoreboard();
    assert.equal(rows.length, 2 * SCORE_STRIDE, "player and one dummy");
    assert.equal(rows[5], 1, "own row first (team 0) and flagged");
    const info = game.matchInfo();
    assert.equal(info.length, MATCH_INFO_LEN);
    assert.equal(info[0], 0, "running");
    assert.deepEqual(game.scoreboardNames().length, 2);

    // M2: visual table, entities, players and the weapon panel.
    const visuals = parseVisuals(game.visuals(), wasm.visualShapeNames());
    assert.ok(visuals.length > 8 + 2 * 27, "built-ins, then two per weapon, then gadgets");
    assert.equal(visuals[2]?.shape, "wreck");
    const entities = parseEntities(game.entities(), wasm.entityKindNames());
    for (const e of entities) assert.ok(visuals[e.visual], `visual of ${e.kind}`);
    const players = parsePlayers(game.playerInfo(), game.playerNames());
    assert.equal(players.length, 2);
    assert.ok(players.some((p) => p.name === "You"));
    const loadout = loadoutState(game.loadout());
    assert.ok(loadout.primaries.length >= 1);
    assert.equal(loadout.primaries[0]?.key, 1);
    assert.equal(loadout.primaries.filter((w) => w.selected).length, 1);

    // Fire: projectiles carry their weapon visual.
    controls[6] = 2; // fire primary
    for (let i = 0; i < 20; i++) game.frame(1 / 60, controls);
    const shots = game.projectiles();
    assert.ok(shots.length >= PROJECTILE_STRIDE);
    const visual = shots[8]!;
    assert.equal(visuals[visual]?.shape !== undefined, true);
  } finally {
    game.free();
  }
});

test("WASM mechanics API: ship choice, cues, gadget, charge, paint colour, mesh layers", { skip }, async () => {
  const wasm = await load();
  assert.deepEqual(wasm.gadgetIds?.(), ["cloak", "decoy", "shield"]);
  assert.ok((wasm.helloPacketAs?.("tester", -1, 0).length ?? 0) > 4);
  const game = wasm.Game.practiceAs!("LS08-0000-0000-000A-K4M6-8001-8F1T", 1, 2);
  try {
    const choices = parseShipChoices(game.shipChoices?.());
    assert.equal(choices.length, 4);
    assert.equal(choices.find((c) => c.current)?.name, "Hauler", "practiceAs flies the chosen type");
    assert.ok(choices.every((c) => c.allowed));
    assert.ok(choices[2]!.secondaries.length >= choices[0]!.secondaries.length, "more slots, more secondaries");
    assert.equal(game.selectShip?.(0), undefined, "practice sends no packet");
    assert.equal(parseShipChoices(game.shipChoices?.()).find((c) => c.next)?.name, "Skitter");

    // G switches the loadout Shield on: a gadget cue and the gadget view.
    const controls = new Float32Array(CONTROLS_LEN);
    controls[6] = 1 << 6;
    game.frame(1 / 60, controls);
    controls[6] = 0;
    game.frame(1 / 60, controls);
    const cues = parseCues(game.takeCues?.());
    assert.ok(cues.some((c) => c.kind === CUE.gadget && c.source === game.player));
    const gadget = game.gadget!();
    assert.equal(gadget.key, "shield");
    assert.equal(gadget.active, true);
    gadget.free();
    assert.equal(parseCharge(game.charge?.()), null, "not charging");
    assert.ok((game.nowTick?.() ?? 0) > 0);

    // The Paint Thrower (second loadout primary) selected: V (weapon mode,
    // bit 7) cycles its colour in the simulation.
    assert.equal(game.paintColor, 0, "Scrap Laser selected");
    controls[8] = 2;
    game.frame(1 / 60, controls);
    controls[8] = 0;
    game.frame(1 / 60, controls);
    assert.equal(game.paintColor, 1);
    controls[6] = 1 << 7;
    game.frame(1 / 60, controls);
    controls[6] = 0;
    game.frame(1 / 60, controls);
    assert.equal(game.paintColor, 2);
    assert.ok(parseCues(game.takeCues?.()).some((c) => c.kind === CUE.paintColor && c.value === 2));

    const mesh = game.meshNext(128, 64, 128);
    assert.ok(mesh);
    const n = mesh.teams().length;
    const materials = mesh.materials?.() ?? new Uint8Array(0);
    assert.equal(materials.length, n, "rock materials per vertex");
    assert.ok([...materials].every((m) => m <= 2));
    const colors = mesh.paintColors?.() ?? new Uint8Array(0);
    assert.ok(colors.length === 0 || colors.length === n);
    const ao = mesh.ao?.() ?? new Uint8Array(0);
    assert.equal(ao.length, n, "baked ambient occlusion per vertex");
    assert.ok(ao.some((a) => a < 255) && ao.some((a) => a > 200), "occlusion varies");
    mesh.free();
  } finally {
    game.free();
  }
});
