// Mechanics UI (M2 wave C): cues to effects, charge meter, gadget line,
// paint colour, eyeball camera, cloak and shield, ship bay, terrain vertex
// packing, and the session plumbing (SelectShip sent reliably, V cycles).

import assert from "node:assert/strict";
import { test } from "node:test";
import { emptyInput, type InputState } from "../input/input.ts";
import { KEY_BINDINGS } from "../input/bindings.ts";
import * as q from "../math/quat.ts";
import { expandTeams, MeshError, validateMesh } from "../renderer/mesh.ts";
import { MATERIAL_SHIFT, PAINT_COLOR_SHIFT, SURFACE_SHIFT } from "../renderer/shaders.ts";
import { chargeModel, loadoutLines } from "../ui/hud.ts";
import { bannerText } from "../ui/effects.ts";
import { shipPanelModel } from "../ui/shipSelect.ts";
import { GameStub } from "../wasm/stub.ts";
import type { GadgetHandle } from "../wasm/api.ts";
import { ShipCamera } from "./camera.ts";
import {
  chargeLabel,
  CUE,
  EffectsState,
  emptyGadget,
  eyeballPose,
  gadgetDetail,
  gadgetKey,
  guidedProjectile,
  parseCharge,
  parseCues,
  parseShipChoices,
  PROJ_GUIDED,
  shakeOffset,
  shipCardLines,
  shipForKey,
  shipPositionLookup,
  shockwaveShake,
  tectonicZone,
  type Cue,
  type EffectsContext,
  type ShipChoice,
} from "./mechanics.ts";
import { addEffects, buildScene, CLOAK_OPACITY, DECOY_RETURNING, SHIELD_SCALE } from "./scene.ts";
import { GameSession, type PacketLink } from "./session.ts";
import { BUTTONS, ControlState } from "./controls.ts";
import type { EntityState, ProjectileState, ShipState } from "./views.ts";
import { emptyScene } from "../renderer/renderer.ts";

const ME = 2;
const origin = { x: 0, y: 0, z: 0 };

function ctx(over: Partial<EffectsContext> = {}): EffectsContext {
  return { player: ME, camera: origin, shipPos: (id) => (id === ME ? origin : id === 5 ? { x: 10, y: 0, z: 0 } : undefined), ...over };
}

function cue(over: Partial<Cue>): Cue {
  return { kind: 0, source: 255, target: 255, position: origin, value: 0, tick: 0, ...over };
}

test("cues decode from the flat array", () => {
  const c = parseCues(new Float64Array([CUE.blinded, 5, ME, 1, 2, 3, 120, 900, CUE.beam, ME, 255, 4, 5, 6, 18, 901, 7]));
  assert.equal(c.length, 2, "trailing partial entry ignored");
  assert.deepEqual(c[0], { kind: CUE.blinded, source: 5, target: ME, position: { x: 1, y: 2, z: 3 }, value: 120, tick: 900 });
  assert.equal(c[1]!.kind, CUE.beam);
  assert.deepEqual(parseCues(undefined), []);
});

test("Flash Pod whiteout: full while blinded, fades at the end, only for the blinded pilot", () => {
  const fx = new EffectsState();
  fx.apply([cue({ kind: CUE.blinded, target: 7, value: 120 })], 0, ctx());
  assert.equal(fx.frame(0.5, ctx()).whiteout, 0, "someone else is blinded");
  fx.apply([cue({ kind: CUE.blinded, target: ME, value: 120 })], 1, ctx());
  assert.equal(fx.frame(1.5, ctx()).whiteout, 1);
  const fading = fx.frame(2.8, ctx()).whiteout;
  assert.ok(fading > 0 && fading < 1, `${fading}`);
  assert.equal(fx.frame(3.01, ctx()).whiteout, 0);
});

test("banners: excuse me for the target, Tectonic launch and crack warning with the zone, overcharge", () => {
  const fx = new EffectsState();
  fx.apply([cue({ kind: CUE.politeWarning, source: 5, target: 9, value: 90 })], 0, ctx());
  assert.equal(fx.frame(0.1, ctx()).banners.length, 0, "warning meant for someone else");
  fx.apply([cue({ kind: CUE.politeWarning, source: 5, target: ME, value: 90 })], 0, ctx());
  let b = fx.frame(0.5, ctx()).banners;
  assert.equal(b.length, 1);
  assert.match(b[0]!.text, /Excuse me/);
  assert.ok(Math.abs(b[0]!.countdown! - 1.0) < 1e-9);
  assert.match(bannerText(b[0]!), /1\.0 s$/);
  assert.equal(fx.frame(1.6, ctx()).banners.length, 0, "expires with the charge");

  fx.apply([cue({ kind: CUE.tectonicLaunch, source: 5 })], 2, ctx());
  fx.apply([cue({ kind: CUE.tectonicArmed, source: 5, value: 90, position: { x: 30, y: 0, z: 0 } })], 2, ctx());
  const f = fx.frame(2.2, ctx());
  const texts = f.banners.map((x) => x.text).join(" | ");
  assert.match(texts, /Tectonic Charge launched/);
  assert.match(texts, /Ground cracking.*collapse zone/);
  assert.ok(f.shake > 0, "rumble");
  assert.ok(f.flashes.some((x) => x.position.x === 30), "crack marker at the charge");

  fx.apply([cue({ kind: CUE.overcharge, source: ME, value: 10 })], 3, ctx());
  const o = fx.frame(3.1, ctx());
  assert.ok(o.overcharging);
  assert.ok(o.banners.some((x) => /Overcharge/.test(x.text)));
  fx.apply([cue({ kind: CUE.chargeStart, source: ME })], 4, ctx());
  assert.equal(fx.frame(4.1, ctx()).overcharging, false, "a new charge clears it");
});

test("screen shake by Tectonic zone and distance", () => {
  assert.equal(tectonicZone(2), "core");
  assert.equal(tectonicZone(20), "damage");
  assert.equal(tectonicZone(30), "collapse");
  assert.equal(tectonicZone(50), "shockwave");
  assert.equal(tectonicZone(80), "outside");
  const s = [2, 20, 30, 50, 100].map((d) => shockwaveShake(d, 60));
  for (let i = 1; i < s.length; i++) assert.ok(s[i]! < s[i - 1]!, `shake falls off: ${s}`);
  assert.ok(s[4]! > 0, "level-wide jostle from a Tectonic shockwave");
  assert.equal(shockwaveShake(20, 8), 0, "a small shockwave far away does not shake");
  assert.equal(shockwaveShake(1, 0), 0);

  const fx = new EffectsState();
  fx.apply([cue({ kind: CUE.shockwave, value: 60, position: { x: 3, y: 0, z: 0 } })], 0, ctx());
  const near = fx.frame(0.1, ctx()).shake;
  const later = fx.frame(0.9, ctx()).shake;
  assert.ok(near > later && later > 0, `${near} ${later}`);
  assert.equal(fx.frame(1.3, ctx()).shake, 0);
  assert.deepEqual(shakeOffset(0, 1), { x: 0, y: 0 });
  const off = shakeOffset(0.5, 1.234);
  assert.ok(Math.abs(off.x) <= 0.5 && Math.abs(off.y) <= 0.5);
});

test("Leech Arc beams run from the shooter's ship to the arc end while the cue lasts", () => {
  const fx = new EffectsState();
  fx.apply([cue({ kind: CUE.beam, source: 5, value: 18, position: { x: 20, y: 0, z: 0 } })], 0, ctx());
  fx.apply([cue({ kind: CUE.beam, source: 44, value: 18 })], 0, ctx());
  const f = fx.frame(0.1, ctx());
  assert.equal(f.beams.length, 1, "no beam from an unknown ship");
  assert.deepEqual(f.beams[0]!.from, { x: 10, y: 0, z: 0 });
  assert.deepEqual(f.beams[0]!.to, { x: 20, y: 0, z: 0 });
  assert.equal(fx.frame(0.31, ctx()).beams.length, 0);
  const scene = emptyScene();
  addEffects(scene, f.beams, f.flashes, 0);
  const bolts = scene.translucent.bolt ?? [];
  assert.equal(bolts.length, 2, "two strands");
  assert.ok(Math.abs((bolts[0]!.stretch ?? 1) * bolts[0]!.scale - 10) < 1e-6, "spans the 10 u gap");
});

test("muzzle flash, decoy return, swap and rewind cues", () => {
  const fx = new EffectsState();
  fx.apply(
    [
      cue({ kind: CUE.muzzleFlash, source: 5, position: { x: 1, y: 1, z: 1 }, value: 30 }),
      cue({ kind: CUE.decoyReturn, source: ME, position: { x: 2, y: 0, z: 0 } }),
      cue({ kind: CUE.swapped, source: 5, target: ME, position: { x: 3, y: 0, z: 0 } }),
      cue({ kind: CUE.rewound, source: ME, target: ME, value: 180, position: { x: 4, y: 0, z: 0 } }),
    ],
    0,
    ctx(),
  );
  const f = fx.frame(0.05, ctx());
  assert.ok(f.flashes.some((x) => x.position.x === 1), "muzzle flash");
  assert.ok(f.flashes.some((x) => x.position.x === 2), "decoy return");
  assert.ok(f.banners.some((x) => /Decoy/.test(x.text)));
  assert.ok(f.banners.some((x) => /Swapped/.test(x.text)));
  assert.ok(Math.abs(f.weaponsOffline - 2.95) < 1e-9);
  assert.equal(fx.frame(0.2, ctx()).flashes.some((x) => x.position.x === 1), false, "the muzzle flash is brief");
});

test("charge meter decodes and labels the charge", () => {
  assert.equal(parseCharge(new Float64Array(0)), null);
  const c = parseCharge(new Float64Array([9, 0.5, 1, 2, 0, 0, 0, 1]))!;
  assert.deepEqual({ w: c.weapon, f: c.fraction, r: c.ready, a: c.autoRelease }, { w: 9, f: 0.5, r: true, a: false });
  assert.equal(chargeLabel("Fusion Charger", c), "Fusion Charger: 50%");
  const polite = parseCharge(new Float64Array([20, 0.4, 0.6, 1.5, 1.5, 1, 0, 1]))!;
  assert.equal(polite.ready, false);
  assert.match(chargeLabel("Polite Laser", polite), /charging/);
  const over = parseCharge(new Float64Array([9, 1, 2.5, 2, 0, 0, 1, 1]))!;
  assert.match(chargeLabel("Fusion Charger", over), /OVERCHARGE/);
  assert.deepEqual(chargeModel({ name: "Fusion Charger", meter: over }), { label: "Fusion Charger: OVERCHARGE, release!", percent: 100, state: "overcharge" });
  assert.equal(chargeModel(null), null);
});

test("gadget line shows on/off, ready and the cooldown", () => {
  const g = { ...emptyGadget(), id: 1, key: "decoy", name: "Decoy", toggle: false };
  assert.equal(gadgetDetail(g), "ready");
  assert.equal(gadgetDetail({ ...g, cooldownLeft: 3.24 }), "3.2 s");
  assert.equal(gadgetDetail({ ...g, key: "shield", name: "Shield", toggle: true }), "off");
  assert.equal(gadgetDetail({ ...g, active: true }), "on");
  assert.equal(gadgetDetail(emptyGadget()), "");
  const lines = loadoutLines({ primaries: [], secondaries: [], gadget: "Decoy", gadgetActive: false }, { ...g, cooldownLeft: 1.5 });
  assert.deepEqual(lines.map((l) => [l.key, l.name, l.detail]), [["G", "Decoy", "1.5 s"]]);
  assert.equal(gadgetKey(["cloak", "decoy", "shield"], 0), "cloak");
  assert.equal(gadgetKey(["cloak"], 255), "");
});

test("paint colour: V is bound and sends the weapon mode button for one sample", () => {
  assert.equal(KEY_BINDINGS["KeyV"], "paintColor");
  assert.equal(KEY_BINDINGS["KeyB"], "shipSelect");
  assert.equal(BUTTONS.weaponMode, 1 << 7, "ls_sim::Buttons::WEAPON_MODE");
  const c = new ControlState(false);
  assert.equal((c.update(input({ cyclePaintColor: true }))[6] ?? 0) & BUTTONS.weaponMode, BUTTONS.weaponMode);
  assert.equal((c.update(input({}))[6] ?? 0) & BUTTONS.weaponMode, 0);
});

function shot(over: Partial<ProjectileState>): ProjectileState {
  return { kind: 0, owner: ME, position: { x: 1, y: 2, z: 3 }, velocity: { x: 0, y: 0, z: -40 }, visual: 0, state: 0, ...over };
}

test("eyeball camera rides the own guided missile, looking along its flight", () => {
  const shots = [shot({ owner: 5, state: PROJ_GUIDED }), shot({}), shot({ state: PROJ_GUIDED, velocity: { x: 30, y: 0, z: 0 } })];
  const g = guidedProjectile(shots, ME);
  assert.equal(g, shots[2]);
  assert.equal(guidedProjectile(shots.slice(0, 2), ME), null);
  const pose = eyeballPose(g!, { x: 0, y: 1, z: 0 });
  const f = q.forward(pose.orientation);
  assert.ok(Math.abs(f.x - 1) < 1e-6, `${JSON.stringify(f)}`);
  assert.ok(Math.abs(q.up(pose.orientation).y - 1) < 1e-6, "keeps the ship's up");
  // Straight up with an up hint along +Y still gives a valid orientation.
  const vertical = eyeballPose(shot({ velocity: { x: 0, y: 10, z: 0 } }), { x: 0, y: 1, z: 0 });
  assert.ok(Math.abs(q.forward(vertical.orientation).y - 1) < 1e-6);

  const cam = new ShipCamera(origin);
  const alive = { position: { x: 9, y: 9, z: 9 }, orientation: q.identity(), velocity: origin, alive: true };
  assert.deepEqual(cam.update(alive, pose).position, g!.position, "the ride replaces the ship view");
  assert.deepEqual(cam.update(alive).position, { x: 9, y: 9, z: 9 }, "and returns after the hit");
  const shaken = cam.update(alive, null, { x: 0.5, y: 0 });
  assert.ok(Math.abs(shaken.position.x - 9.5) < 1e-9, "shake along the camera's right axis");
  assert.deepEqual(cam.current.position, { x: 9, y: 9, z: 9 }, "shake does not move the held view");

  // The ridden missile is not drawn in front of the camera.
  const scene = buildScene([], shots, [], [], () => undefined, 0, { hideShot: (p) => p === g });
  const drawn = Object.values(scene.opaque).reduce((n, xs) => n + (xs?.length ?? 0), 0);
  assert.equal(drawn, 2);
});

function ship(over: Partial<ShipState>): ShipState {
  return {
    id: 5, team: 0, alive: true, boosting: false, position: origin, orientation: q.identity(), shipType: 1, radius: 0.5, gadgetActive: true, ...over,
  };
}

test("cloak shimmer, shield bubble, decoy return glow and ghost flicker", () => {
  const gadgets = new Map([[5, "cloak"], [6, "shield"], [7, "decoy"]]);
  const scene = buildScene(
    [ship({ id: 5 }), ship({ id: 6 }), ship({ id: 8, gadgetActive: false })],
    [],
    [],
    [],
    () => undefined,
    0,
    { gadgetOf: (id) => gadgets.get(id) ?? "" },
  );
  const translucentShips = scene.translucent.ship_tinker ?? [];
  assert.equal(translucentShips.length, 1, "the cloaked ship shimmers");
  assert.ok((translucentShips[0]!.opacity ?? 1) <= CLOAK_OPACITY);
  assert.equal((scene.opaque.ship_tinker ?? []).length, 2, "shielded and plain ships are solid");
  const bubbles = scene.translucent.sphere ?? [];
  assert.equal(bubbles.length, 1);
  assert.equal(bubbles[0]!.scale, SHIELD_SCALE);

  const entity = (over: Partial<EntityState>): EntityState => ({
    id: 1, kind: "decoy", visual: 0, owner: ME, team: 0, position: origin, velocity: origin, rotation: q.identity(), state: [100, 1, 0, 0], radius: 0, ...over,
  });
  const visuals = [{ shape: "decoy", color: [1, 1, 1] as [number, number, number], scale: 1 }, { shape: "ghost", color: [1, 1, 1] as [number, number, number], scale: 1 }];
  const calm = buildScene([], [], [entity({})], visuals, () => undefined, 0);
  assert.equal((calm.translucent.sphere ?? []).length, 0);
  const back = buildScene([], [], [entity({ state: [100, 1 | DECOY_RETURNING, 0, 0] })], visuals, () => undefined, 0.1);
  assert.equal((back.translucent.sphere ?? []).length, 1, "a returning decoy glows");
  const ghosts = [0.0, 0.12].map((t) => buildScene([], [], [entity({ kind: "ghost", visual: 1 })], visuals, () => undefined, t).translucent.ship_tinker?.[0]?.opacity ?? 0);
  assert.notEqual(ghosts[0], ghosts[1], "ghosts flicker");
});

const CHOICES: ShipChoice[] = [
  { shipType: 0, name: "Skitter", allowed: true, current: false, next: true, hull: 70, topSpeedPct: 98, damageMult: 0.85, secondarySlots: 1, radius: 0.4, turnRate: 14, canPassScrap: false, primaries: ["Scrap Laser"], secondaries: [["Bolt Rocket", 3]], gadget: "" },
  { shipType: 1, name: "Tinker", allowed: true, current: true, next: false, hull: 100, topSpeedPct: 90, damageMult: 1, secondarySlots: 2, radius: 0.5, turnRate: 10, canPassScrap: false, primaries: ["Scrap Laser"], secondaries: [["Bolt Rocket", 3]], gadget: "Shield" },
  { shipType: 2, name: "Hauler", allowed: false, current: false, next: false, hull: 160, topSpeedPct: 70, damageMult: 1.2, secondarySlots: 3, radius: 0.65, turnRate: 6, canPassScrap: false, primaries: [], secondaries: [], gadget: "" },
  { shipType: 3, name: "Tug", allowed: true, current: false, next: false, hull: 110, topSpeedPct: 80, damageMult: 0.9, secondarySlots: 2, radius: 0.55, turnRate: 8, canPassScrap: true, primaries: [], secondaries: [], gadget: "" },
];

test("ship bay: choices, loadout preview, keys and the panel model", () => {
  assert.deepEqual(parseShipChoices(JSON.stringify(CHOICES)), CHOICES);
  assert.deepEqual(parseShipChoices("nope"), []);
  assert.deepEqual(parseShipChoices(undefined), []);
  const lines = shipCardLines(CHOICES[0]!);
  assert.match(lines[0]!, /Hull 70 · speed 98% of laser · damage ×0.85/);
  assert.match(lines[1]!, /^1 secondary slot$/);
  assert.equal(lines[3], "Secondaries: Bolt Rocket ×3");
  assert.match(shipCardLines(CHOICES[3]!)[1]!, /passes scrap/);
  assert.match(shipCardLines(CHOICES[1]!).at(-1)!, /Gadget: Shield/);
  assert.equal(shipForKey(CHOICES, 1)?.name, "Skitter");
  assert.equal(shipForKey(CHOICES, 3), null, "not allowed");
  assert.equal(shipForKey(CHOICES, 9), null);

  assert.equal(shipPanelModel(CHOICES, false, false).visible, false);
  const dead = shipPanelModel(CHOICES, false, true);
  assert.ok(dead.visible);
  assert.match(dead.title, /Destroyed/);
  assert.match(dead.hint, /Respawning as Skitter/);
  assert.equal(dead.cards[2]!.allowed, false);
  const open = shipPanelModel(CHOICES, true, false, "nope");
  assert.match(open.hint, /B to close/);
  assert.equal(open.error, "nope");
  assert.equal(shipPanelModel([], true, true).visible, false);
});

test("terrain vertices pack materials and paint colours; bad lengths are rejected", () => {
  const packed = expandTeams(new Uint8Array([0, 255]), new Uint8Array([1, 8]), new Uint8Array([0, 2]), new Uint8Array([3, 0]));
  assert.equal(packed[0], (0 | (1 << SURFACE_SHIFT) | (1 << MATERIAL_SHIFT) | (3 << PAINT_COLOR_SHIFT)) >>> 0);
  assert.equal(packed[1], (255 | (8 << SURFACE_SHIFT) | (3 << MATERIAL_SHIFT)) >>> 0);
  assert.deepEqual([...expandTeams(new Uint8Array([4]))], [4], "absent layers leave the old packing");
  const mesh = { positions: new Float32Array(9), normals: new Float32Array(9), indices: new Uint32Array([0, 1, 2]), team: new Uint8Array(3) };
  validateMesh({ ...mesh, materials: new Uint8Array(3), paintColors: new Uint8Array(0) });
  assert.throws(() => validateMesh({ ...mesh, paintColors: new Uint8Array(2) }), MeshError);
  assert.throws(() => validateMesh({ ...mesh, materials: new Uint8Array(4) }), MeshError);
});

test("ship positions for beams: own pose for the player, living remote ships otherwise", () => {
  const pose = { position: { x: 1, y: 1, z: 1 }, orientation: q.identity(), velocity: origin, alive: true };
  const look = shipPositionLookup(ME, pose, [ship({ id: 5, position: { x: 4, y: 0, z: 0 } }), ship({ id: 6, alive: false })]);
  assert.deepEqual(look(ME), { x: 1, y: 1, z: 1 });
  assert.deepEqual(look(5), { x: 4, y: 0, z: 0 });
  assert.equal(look(6), undefined);
  assert.equal(look(255), undefined);
});

class MechanicsGame extends GameStub {
  cues = new Float64Array([CUE.politeWarning, 5, 0, 0, 0, 0, 60, 10]);
  color = 1;
  picked: number[] = [];
  override player = 0;
  takeCues(): Float64Array {
    const c = this.cues;
    this.cues = new Float64Array(0);
    return c;
  }
  charge(): Float64Array { return new Float64Array([9, 0.25, 0.5, 2, 0, 0, 0, 1]); }
  gadget(): GadgetHandle {
    return { id: 2, key: "shield", name: "Shield", active: true, toggle: true, cooldownLeft: 0, cooldownSecs: 1, energyCost: 5, drainPerSec: 2, free: () => {} };
  }
  shipChoices(): string { return JSON.stringify(CHOICES); }
  selectShip(t: number): Uint8Array | undefined {
    if (t === 2) throw new Error("the rule set does not allow the Hauler");
    this.picked.push(t);
    return new Uint8Array([9, t]);
  }
  get paintColor(): number { return this.color; }
  override frame(dt: number, controls: Float32Array): unknown[] {
    // The weapon mode button (V) cycles the colour, like the simulation.
    if (((controls[6] ?? 0) & BUTTONS.weaponMode) !== 0) this.color = (this.color % 3) + 1;
    return super.frame(dt, controls);
  }
}

class Link implements PacketLink {
  isOpen = true;
  closeReason: string | null = null;
  sent: Array<[number[], boolean]> = [];
  send(p: Uint8Array, droppable = false): void { this.sent.push([Array.from(p), droppable]); }
  drain(): Uint8Array[] { return []; }
  close(): void { this.isOpen = false; }
}

function input(over: Partial<InputState>): InputState {
  return { ...emptyInput(), pointerLocked: true, ...over };
}

test("session: cues, charge, gadget and paint colour each frame; V cycles; SelectShip goes out reliably", () => {
  const game = new MechanicsGame();
  const link = new Link();
  const s = new GameSession(game, link, { entityKinds: [], shapes: [] });
  const r = s.frame(1 / 60, input({}));
  assert.equal(r.cues.length, 1);
  assert.equal(r.cues[0]!.kind, CUE.politeWarning);
  assert.equal(r.charge?.fraction, 0.25);
  assert.equal(r.gadget.key, "shield");
  assert.equal(r.paintColor, 1);
  assert.equal(s.frame(1 / 60, input({ cyclePaintColor: true })).paintColor, 2);
  assert.equal(s.frame(1 / 60, input({})).cues.length, 0);

  assert.equal(s.shipChoices().length, 4);
  assert.equal(s.selectShip(0), null);
  assert.deepEqual(link.sent.at(-1), [[9, 0], false], "SelectShip is sent reliably");
  assert.match(s.selectShip(2) ?? "", /does not allow/);
  assert.deepEqual(game.picked, [0]);

  // Older games without the mechanics API still work.
  const plain = new GameSession(new GameStub(), null, { entityKinds: [], shapes: [] });
  const p = plain.frame(1 / 60, input({ cyclePaintColor: true }));
  assert.deepEqual([p.cues, p.charge, p.paintColor, p.gadget.name], [[], null, 0, ""]);
  assert.match(plain.selectShip(1) ?? "", /not available/);
});
