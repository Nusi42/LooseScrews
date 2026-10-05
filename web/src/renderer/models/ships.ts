// The four hulls (ls-sim ShipType order: Skitter, Tinker, Hauler, Tug),
// built with the modelling kit. Each hull is made of 3-4 closed pieces
// (wrecks throw exactly these pieces apart, see wrecks.ts) and is scaled so
// its farthest vertex sits on HULL_RADIUS: at instance scale
// `radius / TINKER_RADIUS` (game/scene.ts) the hull then fills the ship's
// collision sphere without poking out of it.
//
// Material parts (kit.ts PART_*): HULL is the painted body (team-tinted
// gunmetal), LIVERY the team colour stripes, TRIM dark details, METAL bare
// metal, CANOPY glass, ENGINE the glowing nozzles (scaled by the instance's
// `glow`, i.e. boosting), NAV_* the blinking navigation lights (red port,
// green starboard, white tail strobe).

import {
  chain,
  hullRing,
  Model,
  PART_CANOPY,
  PART_ENGINE,
  PART_HULL,
  PART_LAMP,
  PART_LIVERY,
  PART_METAL,
  PART_NAV_PORT,
  PART_NAV_STARBOARD,
  PART_NAV_TAIL,
  PART_TRIM,
  rotate,
  scale,
  translate,
  type PartFn,
  type V3,
} from "./kit.ts";

/** Hull extent at instance scale 1: the Tinker's hitbox radius (views.ts TINKER_RADIUS). */
export const HULL_RADIUS = 0.5;

/** A hull and the pieces it breaks into. */
export interface Hull {
  name: string;
  /** The whole ship (union of the pieces). */
  model: Model;
  /** 3-4 closed pieces for the wreck. */
  pieces: Model[];
}

/** Engine nozzle pointing to +Z (rear) whose mouth is at `c` (z = exit plane). */
function nozzle(m: Model, c: V3, r: number, len: number, shell = PART_METAL, segments = 12): void {
  const z0 = c[2] - len;
  m.lathe(
    [
      [r * 0.7, z0],
      [r, z0 + len * 0.35],
      [r * 1.1, c[2]],
      [r * 0.82, c[2]],
      [r * 0.8, c[2] - len * 0.6],
    ],
    segments,
    { center: [c[0], c[1], 0], part: (seg) => (seg === 3 ? PART_ENGINE : seg === 2 ? PART_TRIM : shell), group: m.smooth(40), capStart: true },
  );
  // Glowing core, a dome recessed in the bell (its normals give the glow a hot centre).
  m.lathe(
    [
      [r * 0.81, c[2] - len * 0.7],
      [r * 0.6, c[2] - len * 0.2],
      [0, c[2] - len * 0.1],
    ],
    segments,
    { center: [c[0], c[1], 0], part: PART_ENGINE, group: m.smooth(80), capStart: true },
  );
}

/** Small navigation light. */
function navLight(m: Model, c: V3, part: number, r = 0.016): void {
  m.sphere(c, r, 0, part, m.smooth(80));
}

/**
 * Thin wing or fin as a loft of airfoil sections along +X: `sections` give
 * [x, leading edge z, trailing edge z, thickness, y].
 */
function wing(m: Model, sections: readonly (readonly [number, number, number, number, number])[], part: PartFn | number = PART_HULL): void {
  const rings = sections.map(([x, le, te, t, y]): V3[] => {
    const c = te - le;
    return [
      [x, y, le],
      [x, y + t, le + 0.3 * c],
      [x, y + t * 0.45, te - 0.08 * c],
      [x, y, te],
      [x, y - t * 0.35, te - 0.08 * c],
      [x, y - t * 0.6, le + 0.3 * c],
    ];
  });
  m.loft(rings, { part, group: m.flat(), capStart: true, capEnd: true });
}

/** Box from `a` to `b` (its long axis) with a `w` x `h` section. */
function strut(m: Model, a: V3, b: V3, w: number, h: number, part: number, bevel = 0.004): void {
  const d: V3 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
  const len = Math.hypot(d[0], d[1], d[2]);
  const yaw = Math.atan2(d[0], d[2]);
  const pitch = -Math.asin(d[1] / (len || 1));
  const piece = new Model().box([0, 0, 0], [w / 2, h / 2, len / 2], { part, bevel });
  m.add(piece, chain(rotate("x", pitch), rotate("y", yaw), translate([(a[0] + b[0]) / 2, (a[1] + b[1]) / 2, (a[2] + b[2]) / 2])));
}

/** Fits the pieces so the hull's farthest vertex is at HULL_RADIUS. */
function finish(name: string, pieces: Model[]): Hull {
  const all = new Model();
  for (const p of pieces) all.add(p);
  const s = HULL_RADIUS / all.maxRadius();
  const fitted = pieces.map((p) => p.transformed(scale(s)));
  const model = new Model();
  for (const p of fitted) model.add(p);
  return { name, model, pieces: fitted };
}

/** Builds a starboard part and its port mirror image as one piece each. */
function mirrored(build: (m: Model) => void): [Model, Model] {
  const star = new Model();
  build(star);
  const port = new Model().addMirrorX(star);
  return [port, star];
}

const N_SKITTER = 12;

/** Skitter: a needle interceptor with swept wings, twin canted fins and one big engine. */
export function skitterHull(): Hull {
  // Top spine stripe (sides around 90 degrees), dark radome tip and belly.
  const skin: PartFn = (_seg, side) => {
    if (side === 2 || side === 3) return PART_LIVERY;
    if (side >= 8 && side <= 9) return PART_TRIM;
    return PART_HULL;
  };
  const front = new Model();
  const g = front.smooth(55);
  front.loft(
    [
      hullRing(-0.58, 0.004, 0.003, 0.003, N_SKITTER, 2.2),
      hullRing(-0.5, 0.026, 0.022, 0.018, N_SKITTER, 2.2),
      hullRing(-0.34, 0.052, 0.045, 0.034, N_SKITTER, 2.2),
      hullRing(-0.16, 0.07, 0.06, 0.044, N_SKITTER, 2.2),
      hullRing(0.02, 0.08, 0.066, 0.05, N_SKITTER, 2.2),
    ],
    { part: (seg, side) => (seg === 0 ? PART_TRIM : skin(seg, side)), group: g, capEnd: true },
  );
  front.sphere([0, 0.05, -0.17], [0.042, 0.04, 0.13], 2, PART_CANOPY, front.smooth(80));
  for (const x of [-0.068, 0.068]) {
    front.lathe([[0, -0.3], [0.007, -0.29], [0.008, -0.05], [0.012, 0.0]], 8, { center: [x, -0.03, 0], part: PART_METAL, group: front.smooth(60), capEnd: true });
  }

  const rear = new Model();
  rear.loft(
    [
      hullRing(0.02, 0.08, 0.066, 0.05, N_SKITTER, 2.2),
      hullRing(0.2, 0.086, 0.062, 0.05, N_SKITTER, 2.2),
      hullRing(0.36, 0.08, 0.056, 0.046, N_SKITTER, 2.2),
      hullRing(0.42, 0.07, 0.05, 0.04, N_SKITTER, 2.2),
    ],
    { part: skin, group: rear.smooth(55), capStart: true, capEnd: true },
  );
  nozzle(rear, [0, 0, 0.5], 0.058, 0.1);
  // Side intakes.
  for (const x of [-0.08, 0.08]) rear.box([x, -0.012, 0.1], [0.014, 0.018, 0.08], { part: PART_TRIM, bevel: 0.007 });
  // Twin canted fins, white strobes on top.
  const [finPort, finStar] = mirrored((m) => {
    const fin = new Model();
    wing(fin, [[0, 0.16, 0.42, 0.016, 0], [0.13, 0.32, 0.43, 0.006, 0]], (seg) => (seg === 0 ? PART_HULL : PART_LIVERY));
    m.add(fin, chain(rotate("z", Math.PI / 2 - 0.35), translate([0.04, 0.045, 0])));
    navLight(m, [0.04 + 0.13 * Math.cos(Math.PI / 2 - 0.35), 0.045 + 0.13 * Math.sin(Math.PI / 2 - 0.35), 0.39], PART_NAV_TAIL, 0.012);
  });
  rear.add(finPort).add(finStar);

  // Swept wings with livery tips and nav lights.
  const [wingPort, wingStar] = mirrored((m) => {
    wing(
      m,
      [
        [0.05, -0.02, 0.36, 0.02, -0.01],
        [0.17, 0.17, 0.39, 0.012, -0.005],
        [0.23, 0.27, 0.4, 0.009, 0],
        [0.28, 0.33, 0.41, 0.006, 0],
      ],
      (seg) => (seg === 2 ? PART_LIVERY : PART_HULL),
    );
    // Wing tip missile rail.
    m.lathe([[0, 0.18], [0.008, 0.2], [0.009, 0.4], [0.006, 0.43]], 8, { center: [0.28, -0.004, 0], part: PART_METAL, group: m.smooth(60), capEnd: true });
    navLight(m, [0.282, 0.012, 0.36], PART_NAV_STARBOARD);
  });
  return finish("Skitter", [front, rear, wingPort, wingStar]);
}

const N_TINKER = 14;

/** Tinker: a balanced wedge with a bubble canopy and two engine pods on pylons. */
export function tinkerHull(): Hull {
  const skin: PartFn = (seg, side) => {
    if (seg <= 2 && (side === 2 || side === 4)) return PART_LIVERY;
    if (side >= 9 && side <= 11) return PART_TRIM;
    return PART_HULL;
  };
  const front = new Model();
  front.loft(
    [
      hullRing(-0.56, 0.03, 0.008, 0.008, N_TINKER, 3),
      hullRing(-0.48, 0.07, 0.022, 0.018, N_TINKER, 3),
      hullRing(-0.32, 0.12, 0.045, 0.034, N_TINKER, 3),
      hullRing(-0.14, 0.16, 0.075, 0.05, N_TINKER, 3),
      hullRing(0.04, 0.18, 0.09, 0.06, N_TINKER, 3),
    ],
    { part: skin, group: front.smooth(42), capStart: true, capEnd: true },
  );
  front.sphere([0, 0.075, -0.17], [0.06, 0.055, 0.14], 2, PART_CANOPY, front.smooth(80));
  // Canopy frame spine.
  front.box([0, 0.128, -0.17], [0.006, 0.004, 0.1], { part: PART_TRIM });
  // Chin sensor.
  front.lathe([[0, -0.47], [0.02, -0.44], [0.022, -0.36], [0.018, -0.3]], 10, { center: [0, -0.03, 0], part: PART_METAL, group: front.smooth(60), capEnd: true });

  const rear = new Model();
  rear.loft(
    [
      hullRing(0.04, 0.18, 0.09, 0.06, N_TINKER, 3),
      hullRing(0.24, 0.17, 0.086, 0.06, N_TINKER, 3),
      hullRing(0.4, 0.145, 0.07, 0.05, N_TINKER, 3),
    ],
    { part: (seg, side) => skin(seg + 4, side), group: rear.smooth(42), capStart: true, capEnd: true },
  );
  for (const x of [-0.065, 0.065]) nozzle(rear, [x, -0.005, 0.47], 0.045, 0.09);
  // Dorsal fin with the tail strobe.
  const fin = new Model();
  wing(fin, [[0, 0.12, 0.4, 0.014, 0], [0.1, 0.3, 0.42, 0.006, 0]], (seg) => (seg === 0 ? PART_HULL : PART_LIVERY));
  rear.add(fin, chain(rotate("z", Math.PI / 2), translate([0, 0.075, 0])));
  navLight(rear, [0, 0.178, 0.39], PART_NAV_TAIL, 0.012);
  // Vents on the back.
  for (const z of [0.12, 0.18, 0.24]) rear.box([0, 0.09, z], [0.07, 0.006, 0.012], { part: PART_TRIM });

  const [podPort, podStar] = mirrored((m) => {
    const x = 0.255;
    m.lathe(
      [
        [0, -0.33],
        [0.03, -0.31],
        [0.055, -0.24],
        [0.066, -0.14],
        [0.068, 0.1],
        [0.066, 0.3],
        [0.058, 0.36],
      ],
      12,
      {
        center: [x, -0.01, 0],
        sy: 0.9,
        part: (seg) => (seg === 0 ? PART_METAL : seg === 2 ? PART_LIVERY : PART_HULL),
        group: m.smooth(40),
        capEnd: true,
      },
    );
    nozzle(m, [x, -0.01, 0.44], 0.05, 0.09);
    strut(m, [0.12, -0.005, 0.08], [x - 0.04, -0.01, 0.12], 0.17, 0.022, PART_TRIM);
    // Pod gun.
    m.lathe([[0, -0.42], [0.009, -0.41], [0.01, -0.3]], 8, { center: [x, -0.055, 0], part: PART_METAL, group: m.smooth(60), capEnd: true });
    navLight(m, [x + 0.068, 0, -0.05], PART_NAV_STARBOARD);
  });
  return finish("Tinker", [front, rear, podPort, podStar]);
}

const N_HAULER = 16;

/** Hauler: a blocky armoured freighter with side armour plates, a cargo spine and four engines. */
export function haulerHull(): Hull {
  const block = (z: number, rx: number, up: number, down: number) => hullRing(z, rx, up, down, N_HAULER, 7);
  const belly = (side: number) => side >= 11 && side <= 13;
  const front = new Model();
  front.loft(
    [block(-0.5, 0.11, 0.07, 0.06), block(-0.44, 0.17, 0.12, 0.1), block(-0.36, 0.2, 0.14, 0.12), block(-0.1, 0.21, 0.145, 0.125)],
    { part: (_seg, side) => (belly(side) ? PART_TRIM : PART_HULL), capPart: PART_HULL, group: front.smooth(30), capStart: true, capEnd: true },
  );
  // Raised cockpit block with a faceted armoured window.
  front.box([0, 0.16, -0.26], [0.1, 0.04, 0.11], { part: PART_HULL, bevel: 0.02 });
  front.sphere([0, 0.17, -0.34], [0.08, 0.035, 0.06], 1, PART_CANOPY, front.flat());
  // Bumper bars on the nose.
  for (const y of [-0.04, 0.03]) front.box([0, y, -0.505], [0.08, 0.012, 0.015], { part: PART_METAL, bevel: 0.004 });
  // Floodlights.
  for (const x of [-0.13, 0.13]) front.box([x, 0.09, -0.45], [0.02, 0.014, 0.01], { part: PART_LAMP, bevel: 0.004 });

  const rear = new Model();
  rear.loft(
    [block(-0.1, 0.21, 0.145, 0.125), block(-0.02, 0.215, 0.15, 0.13), block(0.22, 0.22, 0.15, 0.13), block(0.38, 0.2, 0.135, 0.12)],
    { part: (seg, side) => (seg === 0 ? PART_LIVERY : belly(side) ? PART_TRIM : PART_HULL), group: rear.smooth(30), capStart: true, capEnd: true },
  );
  for (const x of [-0.1, 0.1]) for (const y of [-0.06, 0.07]) nozzle(rear, [x, y, 0.46], 0.052, 0.09, PART_TRIM, 10);
  // Cargo spine with ribs.
  rear.box([0, 0.17, 0.12], [0.07, 0.03, 0.2], { part: PART_TRIM, bevel: 0.01 });
  for (const z of [-0.02, 0.08, 0.18, 0.28]) rear.box([0, 0.19, z], [0.08, 0.018, 0.014], { part: PART_METAL });
  navLight(rear, [0, 0.215, 0.3], PART_NAV_TAIL, 0.014);
  // Belly cargo container.
  rear.box([0, -0.15, 0.1], [0.12, 0.03, 0.17], { part: PART_HULL, bevel: 0.012 });

  const [platePort, plateStar] = mirrored((m) => {
    m.box([0.235, 0.0, 0.0], [0.028, 0.115, 0.27], { part: PART_HULL, bevel: 0.016 });
    m.box([0.266, 0.075, 0.0], [0.006, 0.016, 0.22], { part: PART_LIVERY, bevel: 0.003 });
    m.box([0.266, -0.075, 0.0], [0.006, 0.016, 0.22], { part: PART_LIVERY, bevel: 0.003 });
    // Bolted rivet heads.
    for (const z of [-0.2, -0.07, 0.07, 0.2]) m.box([0.266, 0.0, z], [0.006, 0.012, 0.012], { part: PART_METAL });
    // Side thruster block.
    m.box([0.24, -0.1, 0.24], [0.03, 0.03, 0.05], { part: PART_TRIM, bevel: 0.006 });
    navLight(m, [0.27, 0.05, -0.26], PART_NAV_STARBOARD, 0.018);
  });
  return finish("Hauler", [front, rear, platePort, plateStar]);
}

const N_TUG = 16;

/** Tug: a wide flat body with a nose winch, two grapple arms and big outrigger engines. */
export function tugHull(): Hull {
  const skin: PartFn = (_seg, side) => {
    if (side >= 11 && side <= 13) return PART_TRIM;
    return PART_HULL;
  };
  const front = new Model();
  front.loft(
    [
      hullRing(-0.27, 0.16, 0.04, 0.035, N_TUG, 2.6),
      hullRing(-0.2, 0.25, 0.08, 0.06, N_TUG, 2.6),
      hullRing(-0.05, 0.29, 0.1, 0.075, N_TUG, 2.6),
      hullRing(0.1, 0.3, 0.1, 0.08, N_TUG, 2.6),
    ],
    { part: skin, group: front.smooth(40), capStart: true, capEnd: true },
  );
  front.sphere([0, 0.085, -0.1], [0.075, 0.05, 0.1], 1, PART_CANOPY, front.smooth(80));
  // Winch hub on the nose: drum, cable guide and a livery band.
  front.lathe(
    [[0, -0.4], [0.03, -0.39], [0.045, -0.35], [0.06, -0.33], [0.06, -0.29], [0.07, -0.26]],
    14,
    { part: (seg) => (seg === 3 ? PART_LIVERY : PART_METAL), group: front.smooth(40), capEnd: true },
  );
  // Floodlights over the winch.
  for (const x of [-0.11, 0.11]) front.box([x, 0.04, -0.24], [0.022, 0.014, 0.012], { part: PART_LAMP, bevel: 0.004 });

  const rear = new Model();
  rear.loft(
    [hullRing(0.1, 0.3, 0.1, 0.08, N_TUG, 2.6), hullRing(0.24, 0.28, 0.09, 0.07, N_TUG, 2.6), hullRing(0.34, 0.22, 0.07, 0.06, N_TUG, 2.6)],
    {
      // Two wide stripes across the back.
      part: (seg, side) => (seg === 0 && side >= 2 && side <= 5 ? PART_LIVERY : skin(seg, side)),
      group: rear.smooth(40),
      capStart: true,
      capEnd: true,
    },
  );
  nozzle(rear, [0, 0, 0.42], 0.05, 0.09);
  navLight(rear, [0, 0.105, 0.28], PART_NAV_TAIL, 0.013);
  // Outrigger engine pods.
  for (const x of [-0.27, 0.27]) {
    rear.lathe([[0, -0.05], [0.05, -0.02], [0.075, 0.06], [0.078, 0.3], [0.07, 0.36]], 12, {
      center: [x, -0.02, 0],
      part: (seg) => (seg === 1 ? PART_LIVERY : PART_HULL),
      group: rear.smooth(40),
      capEnd: true,
    });
    nozzle(rear, [x, -0.02, 0.43], 0.062, 0.09);
    rear.box([x + Math.sign(x) * 0.077, -0.02, 0.17], [0.005, 0.018, 0.08], { part: PART_TRIM });
  }
  navLight(rear, [-0.35, -0.02, 0.15], PART_NAV_PORT);
  navLight(rear, [0.35, -0.02, 0.15], PART_NAV_STARBOARD);

  const [armPort, armStar] = mirrored((m) => {
    const shoulder: V3 = [0.2, -0.01, -0.12];
    const elbow: V3 = [0.31, -0.01, -0.3];
    const wrist: V3 = [0.2, -0.01, -0.44];
    m.sphere(shoulder, 0.04, 0, PART_METAL, m.smooth(60));
    strut(m, shoulder, elbow, 0.05, 0.045, PART_HULL, 0.008);
    m.sphere(elbow, 0.036, 1, PART_LIVERY, m.smooth(60));
    strut(m, elbow, wrist, 0.04, 0.038, PART_TRIM, 0.006);
    m.sphere(wrist, 0.03, 0, PART_METAL, m.smooth(60));
    // Three-pronged claw.
    for (const [dx, dy] of [[-0.035, 0], [0.02, 0.03], [0.02, -0.03]] as const) {
      strut(m, wrist, [wrist[0] + dx, wrist[1] + dy, wrist[2] - 0.065], 0.014, 0.014, PART_METAL, 0);
    }
    // Hydraulic piston along the upper arm.
    strut(m, [0.215, 0.03, -0.14], [0.3, 0.03, -0.27], 0.014, 0.014, PART_METAL, 0);
  });
  return finish("Tug", [front, rear, armPort, armStar]);
}

/** Hull builders in ls-sim ShipType order (Skitter 0, Tinker 1, Hauler 2, Tug 3). */
export const HULL_BUILDERS: readonly (() => Hull)[] = [skitterHull, tinkerHull, haulerHull, tugHull];

const hullCache = new Map<number, Hull>();

/** The hull of a ship type (cached; unknown types get the Tinker). */
export function hullOf(shipType: number): Hull {
  const t = shipType >= 0 && shipType < HULL_BUILDERS.length ? shipType : 1;
  let h = hullCache.get(t);
  if (!h) hullCache.set(t, (h = HULL_BUILDERS[t]!()));
  return h;
}
