// Runs the real ls-wasm build under Node (skipped when ./pkg has not been
// built; `npm run build` builds it). Checks the determinism contract (golden
// hashes identical to the native fixture) and smoke-tests the game API.

import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import type { WasmApi } from "./api.ts";

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
  const ok = wasm.checkShareCode("ls04 0000 0000 000a k4m6 8001 8f1p");
  assert.equal(ok.ok, true);
  assert.equal(ok.code, "LS04-0000-0000-000A-K4M6-8001-8F1P");
  ok.free();
  const bad = wasm.checkShareCode("LS04-0000-0000-000A-K4M7-8001-8F1P");
  assert.equal(bad.ok, false);
  assert.match(bad.error, /checksum/i);
  bad.free();
  assert.equal(wasm.tickHz(), 60);
  assert.ok(wasm.helloPacket("tester", -1).length > 4);
  assert.deepEqual(wasm.weaponIds(), ["bolt_rocket", "paint_thrower", "rivet_gun", "scrap_laser"]);

  const code = wasm.randomShareCode(1, 2, 45, 35);
  const game = wasm.Game.practice(code, 1);
  try {
    assert.equal(game.isOnline, false);
    assert.equal(game.shareCode, code);
    const start = game.localPose();
    assert.ok(start && start.length === 11);
    const controls = new Float32Array([1, 0, 0, 0, 0, 0, 0, 0]);
    for (let i = 0; i < 30; i++) assert.deepEqual(game.frame(1 / 60, controls), []);
    const end = game.localPose()!;
    assert.ok(Math.hypot(end[0]! - start[0]!, end[1]! - start[1]!, end[2]! - start[2]!) > 1);
    assert.equal(game.remoteShips().length, 11, "one dummy");
    const mesh = game.meshNext(end[0]!, end[1]!, end[2]!);
    assert.ok(mesh);
    assert.match(mesh.key, /^-?\d+,-?\d+,-?\d+$/);
    assert.equal(mesh.positions().length, mesh.normals().length);
    assert.equal(mesh.teams().length * 3, mesh.positions().length);
    mesh.free();
    const hud = game.hud();
    assert.equal(hud.hull, hud.hullMax);
    hud.free();
  } finally {
    game.free();
  }
});
