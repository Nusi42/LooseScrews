// The comedy event visitors of M4 C2 (docs/milestone-4-plan.md §3.2), built
// with the VQ4 modelling kit: the Tax drone (a boxy bureaucrat with a
// visor, a stern brow, a receipt printer, a necktie and a red-and-blue
// siren), the Pizza delivery drone (a cheerful red quadcopter with a
// propeller beanie, one big eye and claws for the box), the Customer
// survey drone (a teal call-centre drone with a headset holding up a
// five-star panel), the mimic (a supply crate with a hinged lid, teeth, a
// tongue and googly eyes) and the gossip face (a stone face with brows,
// eyes, lids, a nose and lipsticked lips that grows out of the rock).
//
// Moving parts are separate meshes so game/events/*.ts can pose them: rotor
// blades, siren domes, the tie, the receipt (grows along +Z with the
// instance `stretch`), the visor eyes, the pizza box and its logo, the
// survey panel and its stars, the mimic's lid and tongue, eyeballs, the
// face's brows, lids and jaw. Sizes are in u at instance scale 1 (the
// drones about 1.8 u across, their hit radius 0.9 u; the face at radius 1,
// scaled by the event's face radius). Forward is -Z, up +Y.

import {
  Model,
  PART_CANOPY,
  PART_ENAMEL,
  PART_FLESH,
  PART_GLOW,
  PART_LAMP,
  PART_LIVERY,
  PART_METAL,
  PART_PLAIN,
  PART_STONE,
  PART_TRIM,
  rotate,
  type V3,
} from "./kit.ts";

/** Ring of a closed profile revolved about Y (for ducts and rims): [radius, y] pairs. */
function ringY(m: Model, profile: Array<[number, number]>, center: V3, segments: number, part: number): void {
  m.lathe(profile, segments, { part, axis: "y", center, group: m.smooth(60) });
}

/** A rotor duct (metal ring with an inner wall) centred at `c`, radius `r`. */
function duct(m: Model, c: V3, r: number): void {
  const t = 0.035, h = 0.05;
  ringY(m, [[r - t, -h], [r + t, -h], [r + t + 0.01, 0], [r + t, h], [r - t, h], [r - t, -h]], c, 18, PART_METAL);
  // Hub and a strut across.
  m.lathe([[0, -0.03], [0.05, -0.03], [0.05, 0.03], [0, 0.03]], 10, { part: PART_TRIM, axis: "y", center: c, capStart: true, capEnd: true });
  m.box([c[0], c[1] - 0.02, c[2]], [r, 0.012, 0.02], { part: PART_TRIM });
}

/** Two-blade rotor in the XZ plane, about 1 u across at scale 1 (spun about Y by the caller). */
export function rotorModel(): Model {
  const m = new Model();
  for (const s of [1, -1]) {
    const blade = new Model().loft(
      [
        [[0.04, -0.008, -0.05], [0.04, 0.008, -0.05], [0.04, 0.012, 0.05], [0.04, -0.004, 0.05]],
        [[0.5, -0.004, -0.035], [0.5, 0.004, -0.035], [0.5, 0.006, 0.02], [0.5, -0.002, 0.02]],
      ],
      { part: PART_TRIM, capStart: true, capEnd: true },
    );
    m.add(blade, (p) => [p[0] * s, p[1], p[2] * s]);
  }
  m.lathe([[0, -0.03], [0.05, -0.03], [0.05, 0.03], [0, 0.03]], 8, { part: PART_METAL, axis: "y", capStart: true, capEnd: true });
  return m;
}

/** A siren dome (glowing hemisphere on a dark collar), tinted by the instance colour. */
export function sirenDomeModel(): Model {
  const m = new Model();
  const prof: Array<[number, number]> = [];
  for (let i = 0; i <= 6; i++) {
    const a = (i / 6) * (Math.PI / 2);
    prof.push([Math.max(0.0005, Math.cos(a) * 0.1), 0.03 + Math.sin(a) * 0.12]);
  }
  m.lathe(prof, 14, { part: PART_GLOW, axis: "y", group: m.smooth(70) });
  m.lathe([[0.11, 0], [0.115, 0.03], [0.1, 0.03]], 14, { part: PART_TRIM, axis: "y", capStart: true });
  return m;
}

/** Where the Tax drone's receipt comes out (body space; game/events/taxDrone.ts). */
export const TAX_SLOT: V3 = [0.3, -0.24, -0.56];

/** The Tax drone's body: a beige box of a bureaucrat with visor, frown, printer slot, seal, ducts and antenna. */
export function taxDroneModel(): Model {
  const m = new Model();
  // Body in the instance colour (a drab government beige), dark skirt.
  m.box([0, 0.02, 0], [0.6, 0.4, 0.48], { part: PART_LIVERY, bevel: 0.09 });
  m.box([0, -0.4, 0], [0.55, 0.05, 0.43], { part: PART_TRIM, bevel: 0.03 });
  // Visor across the front, its hood and a frowning brow (two bars in a V).
  m.box([0, 0.1, -0.47], [0.48, 0.15, 0.06], { part: PART_CANOPY, bevel: 0.05 });
  m.box([0, 0.28, -0.48], [0.52, 0.035, 0.07], { part: PART_TRIM, bevel: 0.015 });
  for (const s of [-1, 1]) {
    const brow = new Model().box([0, 0, 0], [0.19, 0.03, 0.03], { part: PART_TRIM, bevel: 0.01 });
    m.add(brow, (p) => {
      const r = rotate("z", s * 0.22)(p);
      return [r[0] + s * 0.2, r[1] + 0.335, r[2] - 0.53];
    });
  }
  // Receipt printer: a slot with metal lips low on the front, right of the tie.
  m.box([TAX_SLOT[0], TAX_SLOT[1], -0.5], [0.15, 0.05, 0.05], { part: PART_METAL, bevel: 0.015 });
  m.box([TAX_SLOT[0], TAX_SLOT[1], -0.552], [0.12, 0.012, 0.01], { part: PART_TRIM });
  // Official seal on both sides.
  const seal = new Model();
  seal.lathe([[0.15, 0], [0.16, 0.015], [0.12, 0.03], [0.0005, 0.03]], 18, { part: PART_METAL, axis: "x", center: [0.6, 0.02, 0.04], capStart: true, group: seal.smooth(40) });
  seal.lathe([[0.08, 0.03], [0.0005, 0.04]], 12, { part: PART_GLOW, axis: "x", center: [0.6, 0.02, 0.04] });
  m.add(seal);
  m.addMirrorX(seal);
  // Siren base on top and an antenna at the back.
  m.box([0, 0.45, 0.04], [0.3, 0.035, 0.11], { part: PART_TRIM, bevel: 0.02 });
  m.lathe([[0.012, 0], [0.012, 0.42], [0.0005, 0.43]], 6, { part: PART_METAL, axis: "y", center: [0.35, 0.42, 0.3] });
  m.sphere([0.35, 0.86, 0.3], 0.035, 1, PART_GLOW, m.smooth(70));
  // Four ducted rotors on arms.
  for (const sx of [-1, 1]) {
    for (const sz of [-1, 1]) {
      const c: V3 = [sx * 0.78, 0.22, sz * 0.46];
      duct(m, c, 0.22);
      m.box([sx * 0.62, 0.2, sz * 0.38], [0.08, 0.03, 0.04], { part: PART_TRIM, bevel: 0.01 });
    }
  }
  return m;
}

/** The Tax drone's eyes behind the visor: two slanted, half-shut slits (glow in the instance colour). */
export function taxEyesModel(): Model {
  const m = new Model();
  for (const s of [-1, 1]) {
    const eye = new Model().box([0, 0, 0], [0.11, 0.028, 0.01], { part: PART_GLOW, bevel: 0.008 });
    m.add(eye, (p) => {
      const r = rotate("z", s * 0.18)(p);
      return [r[0] + s * 0.19, r[1], r[2]];
    });
  }
  return m;
}

/** A necktie (knot at the origin, the blade hanging down -Y), coloured by the instance. */
export function tieModel(): Model {
  const m = new Model();
  m.loft(
    [
      [[-0.05, 0.03, -0.02], [0.05, 0.03, -0.02], [0.05, 0.03, 0.02], [-0.05, 0.03, 0.02]],
      [[-0.035, -0.05, -0.02], [0.035, -0.05, -0.02], [0.035, -0.05, 0.02], [-0.035, -0.05, 0.02]],
    ],
    { part: PART_PLAIN, capStart: true, capEnd: true },
  );
  const blade = (y: number, w: number): V3[] => [[-w, y, -0.008], [w, y, -0.008], [w, y, 0.008], [-w, y, 0.008]];
  m.loft([blade(-0.05, 0.03), blade(-0.2, 0.06), blade(-0.38, 0.085), blade(-0.44, 0.0005)], { part: PART_PLAIN, capStart: true });
  // A stripe across the blade.
  m.loft([blade(-0.2, 0.062).map((p): V3 => [p[0], p[1], p[2] * 1.4]), blade(-0.24, 0.068).map((p): V3 => [p[0], p[1], p[2] * 1.4])], { part: PART_TRIM, capStart: true, capEnd: true });
  return m;
}

/**
 * A printed receipt: a paper strip 1 u long along +Z from the slot (the
 * instance's `stretch` sets its length), slightly curled, with printed
 * lines and a torn end.
 */
export function receiptModel(): Model {
  const m = new Model();
  const w = 0.17, t = 0.006;
  const rows: V3[][] = [];
  const n = 10;
  for (let i = 0; i <= n; i++) {
    const z = i / n;
    // Curls a little toward -Y and back.
    const y = -0.08 * Math.sin(z * Math.PI * 0.9);
    rows.push([[-w, y - t, z], [w, y - t, z], [w, y + t, z], [-w, y + t, z]]);
  }
  m.loft(rows, { part: PART_PLAIN, capStart: true, capEnd: true });
  // Printed lines (dark bars on both faces), shorter and longer like text.
  for (let i = 1; i < 9; i++) {
    const z = i / 9.5;
    const y = -0.08 * Math.sin(z * Math.PI * 0.9);
    const len = 0.06 + 0.07 * ((i * 7) % 5) / 4;
    for (const face of [-1, 1]) {
      m.box([-0.08 + len, y + face * (t + 0.0015), z], [len, 0.0015, 0.012], { part: PART_TRIM });
    }
  }
  return m;
}

/** The Pizza drone: a red dome with a white band, a big eye, four rotors, claws and a propeller beanie. */
export function pizzaDroneModel(): Model {
  const m = new Model();
  const prof: Array<[number, number]> = [];
  for (let i = 0; i <= 10; i++) {
    const a = -Math.PI / 2 + (i / 10) * Math.PI;
    prof.push([Math.max(0.0005, Math.cos(a) * 0.46), Math.sin(a) * (a > 0 ? 0.34 : 0.2)]);
  }
  m.lathe(prof, 22, { part: PART_LIVERY, axis: "y", group: m.smooth(70) });
  // Band and skid ring.
  m.lathe([[0.462, -0.03], [0.475, 0], [0.462, 0.04]], 22, { part: PART_METAL, axis: "y", group: m.smooth(60) });
  // One big eye looking ahead (dark glass with a lamp highlight).
  m.sphere([0, 0.1, -0.36], [0.17, 0.17, 0.12], 2, PART_CANOPY, m.smooth(70));
  m.sphere([0.05, 0.16, -0.475], 0.035, 1, PART_LAMP, m.smooth(70));
  m.lathe([[0.18, 0], [0.2, 0.02], [0.18, 0.04]], 18, { part: PART_TRIM, axis: "z", center: [0, 0.1, -0.33], group: m.smooth(60) });
  // Arms and rotor ducts.
  for (let k = 0; k < 4; k++) {
    const a = Math.PI / 4 + (k * Math.PI) / 2;
    const c: V3 = [Math.cos(a) * 0.72, 0.12, Math.sin(a) * 0.72];
    duct(m, c, 0.2);
    const arm = new Model().box([0.5, 0, 0], [0.16, 0.03, 0.035], { part: PART_TRIM, bevel: 0.01 });
    m.add(arm, (p) => {
      const r = rotate("y", -a)(p);
      return [r[0], r[1] + 0.12, r[2]];
    });
  }
  // Beanie on top: a little cap with a button for the propeller.
  m.lathe([[0.17, 0.3], [0.16, 0.36], [0.1, 0.41], [0.0005, 0.42]], 16, { part: PART_LIVERY, axis: "y", group: m.smooth(60) });
  m.lathe([[0.012, 0.42], [0.012, 0.5]], 6, { part: PART_METAL, axis: "y" });
  // Claws gripping the box below.
  for (const s of [-1, 1]) {
    m.box([s * 0.33, -0.27, 0], [0.03, 0.1, 0.06], { part: PART_METAL, bevel: 0.01 });
    m.box([s * 0.3, -0.36, 0], [0.06, 0.02, 0.06], { part: PART_METAL, bevel: 0.008 });
  }
  return m;
}

/** A pizza box (cardboard in the instance colour, a dark seam) centred at the origin, 0.95 u wide. */
export function pizzaBoxModel(): Model {
  const m = new Model();
  m.box([0, 0, 0], [0.47, 0.055, 0.47], { part: PART_PLAIN, bevel: 0.012 });
  m.box([0, 0.0, 0], [0.475, 0.006, 0.475], { part: PART_TRIM });
  return m;
}

/** The pizza box's round logo (a flat disc in the instance colour with a glowing ring). */
export function pizzaLogoModel(): Model {
  const m = new Model();
  m.lathe([[0.0005, 0], [0.2, 0], [0.2, 0.008]], 20, { part: PART_PLAIN, axis: "y", capEnd: true });
  m.lathe([[0.2, 0.008], [0.23, 0.008], [0.23, 0.0], [0.2, 0.0]], 20, { part: PART_GLOW, axis: "y" });
  return m;
}

/** A slice of pizza lying flat (tip at the origin, crust at +Z 1 u away): cheese and pepperoni, the progress ring's pieces. */
export function pizzaSliceModel(): Model {
  const m = new Model();
  const half = Math.PI / 12;
  const n = 6;
  const arc = (r: number, y: number): V3[] => Array.from({ length: n + 1 }, (_, i) => {
    const a = -half + (2 * half * i) / n;
    return [Math.sin(a) * r, y, Math.cos(a) * r] as V3;
  });
  // Cheese wedge as a fan (top) and a thin side wall.
  const top = arc(0.92, 0.05);
  for (let i = 0; i < n; i++) m.triangle([0, 0.05, 0], top[i + 1]!, top[i]!, PART_PLAIN);
  const bot = arc(0.92, 0);
  for (let i = 0; i < n; i++) m.triangle([0, 0, 0], bot[i]!, bot[i + 1]!, PART_PLAIN);
  for (const s of [0, n]) {
    const a = top[s]!, b = bot[s]!;
    if (s === 0) m.quad([0, 0, 0], b, a, [0, 0.05, 0], PART_PLAIN);
    else m.quad([0, 0, 0], [0, 0.05, 0], a, b, PART_PLAIN);
  }
  // Pepperoni.
  for (const [x, z] of [[0.0, 0.32], [-0.1, 0.62], [0.12, 0.7]] as Array<[number, number]>) {
    m.lathe([[0.0005, 0.05], [0.07, 0.05], [0.075, 0.065], [0.0005, 0.07]], 10, { part: PART_FLESH, axis: "y", center: [x, 0, z] });
  }
  return m;
}

/** The crust of a pizza slice (a rounded roll along the slice's arc; drawn in the crust colour beside `pizzaSliceModel`). */
export function pizzaCrustModel(): Model {
  const m = new Model();
  const half = Math.PI / 12;
  const n = 6;
  const arc = (r: number, y: number): V3[] => Array.from({ length: n + 1 }, (_, i) => {
    const a = -half + (2 * half * i) / n;
    return [Math.sin(a) * r, y, Math.cos(a) * r] as V3;
  });
  const rolls: V3[][] = arc(0.97, 0.04).map((c) => {
    const out = [c[0], 0, c[2]] as V3;
    const l = Math.hypot(out[0], out[2]);
    const o: V3 = [out[0] / l, 0, out[2] / l];
    return Array.from({ length: 6 }, (_, k) => {
      const a = (k / 6) * Math.PI * 2;
      return [c[0] + o[0] * Math.cos(a) * 0.07, c[1] + Math.sin(a) * 0.06, c[2] + o[2] * Math.cos(a) * 0.07] as V3;
    });
  });
  m.loft(rolls, { part: PART_PLAIN, capStart: true, capEnd: true, group: m.smooth(70) });
  return m;
}

/**
 * The Customer survey drone: a small teal drone with a headset and a
 * microphone (the body only; the panel and stars hang below it).
 */
export function surveyDroneModel(): Model {
  const m = new Model();
  m.sphere([0, 0, 0], [0.32, 0.22, 0.28], 2, PART_LIVERY, m.smooth(70));
  // A friendly visor smile.
  m.box([0, 0.03, -0.26], [0.2, 0.06, 0.03], { part: PART_CANOPY, bevel: 0.025 });
  for (const s of [-1, 1]) m.sphere([s * 0.08, 0.05, -0.29], 0.025, 0, PART_LAMP);
  // Headset: a band over the top, ear cups and a microphone boom to the front.
  const band: V3[][] = [];
  for (let i = 0; i <= 12; i++) {
    const a = (i / 12) * Math.PI;
    const c: V3 = [Math.cos(a) * 0.34, Math.sin(a) * 0.26, 0.02];
    band.push([[c[0], c[1], c[2] - 0.03], [c[0] * 1.08, c[1] * 1.08, c[2] - 0.03], [c[0] * 1.08, c[1] * 1.08, c[2] + 0.03], [c[0], c[1], c[2] + 0.03]]);
  }
  m.loft(band, { part: PART_TRIM, capStart: true, capEnd: true });
  const cup = new Model();
  cup.lathe([[0.1, 0], [0.11, 0.03], [0.09, 0.06], [0.0005, 0.065]], 14, { part: PART_TRIM, axis: "x", center: [0.3, 0, 0.02], capStart: true, group: cup.smooth(50) });
  m.add(cup);
  m.addMirrorX(cup);
  const boom: V3[][] = [];
  for (let i = 0; i <= 6; i++) {
    const t = i / 6;
    const c: V3 = [-0.36 + 0.22 * t, -0.06 - 0.06 * t, -0.04 - 0.24 * t];
    boom.push([[c[0], c[1] - 0.012, c[2]], [c[0] + 0.012, c[1], c[2]], [c[0], c[1] + 0.012, c[2]], [c[0] - 0.012, c[1], c[2]]]);
  }
  m.loft(boom, { part: PART_METAL });
  m.sphere([-0.14, -0.12, -0.3], 0.035, 1, PART_TRIM, m.smooth(70));
  // Two rotors on outriggers and the struts to the panel.
  for (const s of [-1, 1]) {
    duct(m, [s * 0.52, 0.12, 0.05], 0.16);
    m.box([s * 0.4, 0.1, 0.05], [0.08, 0.02, 0.025], { part: PART_TRIM });
    m.box([s * 0.55, -0.3, 0.0], [0.02, 0.32, 0.02], { part: PART_METAL });
  }
  return m;
}

/** The survey panel: a cream board 2.5 x 0.95 u (front at -Z) with a dark frame and a line of "text". */
export function surveyPanelModel(): Model {
  const m = new Model();
  m.box([0, 0, 0], [1.25, 0.47, 0.03], { part: PART_PLAIN, bevel: 0.025 });
  // Frame.
  for (const s of [-1, 1]) {
    m.box([0, s * 0.475, 0], [1.27, 0.02, 0.045], { part: PART_TRIM, bevel: 0.01 });
    m.box([s * 1.265, 0, 0], [0.02, 0.475, 0.045], { part: PART_TRIM, bevel: 0.01 });
  }
  // "How was your match today?": word-shaped bars on a dark title strip.
  m.box([0, 0.33, -0.032], [1.15, 0.075, 0.004], { part: PART_TRIM });
  const words = [0.09, 0.08, 0.1, 0.13, 0.12, 0.15];
  let x = -0.82;
  for (const w of words) {
    m.box([x + w, 0.33, -0.038], [w, 0.022, 0.003], { part: PART_LAMP });
    x += 2 * w + 0.05;
  }
  // Column dividers between the five stars.
  for (let i = 1; i < 5; i++) m.box([-1.25 + i * 0.5, -0.06, -0.033], [0.004, 0.2, 0.003], { part: PART_TRIM });
  return m;
}

/** A five-pointed star facing -Z, about 0.36 u across (the survey's rating stars). */
export function starModel(): Model {
  const m = new Model();
  const pts = (z: number, s: number): V3[] =>
    Array.from({ length: 10 }, (_, i) => {
      const a = Math.PI / 2 + (i * Math.PI) / 5;
      const r = (i % 2 === 0 ? 0.18 : 0.075) * s;
      return [Math.cos(a) * r, Math.sin(a) * r, z] as V3;
    });
  m.loft([pts(0.02, 0.92), pts(0, 1), pts(-0.03, 0.8)], { part: PART_GLOW, capStart: true, capEnd: true });
  return m;
}

/**
 * The mimic's body: the bottom of a supply crate (panels in the pickup
 * colour, dark frame, a light strip) open on top, a dark mouth, a row of
 * teeth along the rim and a curled-up tongue inside. Its hinge line is at
 * the top back edge (y 0.22, z 0.55). Built Z-up (see `zUp`) so the
 * instance's `stretch` squashes and stretches it vertically.
 */
export function mimicBodyModel(): Model {
  const m = new Model();
  m.box([0, -0.17, 0], [0.55, 0.39, 0.55], { part: PART_PLAIN, bevel: 0.04 });
  // Plank grooves around the sides.
  for (const y of [-0.42, -0.2, 0.02]) {
    m.box([0, y, -0.553], [0.5, 0.008, 0.004], { part: PART_TRIM });
    m.box([0, y, 0.553], [0.5, 0.008, 0.004], { part: PART_TRIM });
    m.box([-0.553, y, 0], [0.004, 0.008, 0.5], { part: PART_TRIM });
    m.box([0.553, y, 0], [0.004, 0.008, 0.5], { part: PART_TRIM });
  }
  const w = 0.045;
  for (const a of [-1, 1]) {
    for (const b of [-1, 1]) {
      m.box([a * 0.53, -0.17, b * 0.53], [w, 0.4, w], { part: PART_TRIM, bevel: 0.01 });
      m.box([a * 0.53, -0.55, 0], [w, w, 0.55], { part: PART_TRIM, bevel: 0.01 });
      m.box([0, -0.55, b * 0.53], [0.55, w, w], { part: PART_TRIM, bevel: 0.01 });
    }
  }
  m.box([0, -0.31, -0.556], [0.3, 0.035, 0.01], { part: PART_GLOW });
  // The mouth: a dark pit with a pink floor.
  m.box([0, 0.215, 0], [0.5, 0.01, 0.5], { part: PART_TRIM });
  m.box([0, 0.225, 0.05], [0.36, 0.008, 0.36], { part: PART_FLESH, bevel: 0.006 });
  // Teeth around the rim (front and sides), pointing up.
  teethRow(m, 0.23, 1);
  return m.transformed(zUp);
}

/** Cones along the front and side rims of a 1.1 u box at height `y`, pointing along `dir` (1 up, -1 down). */
function teethRow(m: Model, y: number, dir: number): void {
  const tooth = (x: number, z: number, h: number): void => {
    const prof: Array<[number, number]> = [[0.055, y], [0.04, y + dir * h * 0.5], [0.0005, y + dir * h]];
    m.lathe(prof, 6, {
      part: PART_ENAMEL,
      axis: "y",
      center: [x, 0, z],
      capStart: true,
      group: m.smooth(60),
    });
  };
  for (let i = 0; i < 7; i++) tooth(-0.45 + i * 0.15, -0.47, i % 2 === 0 ? 0.16 : 0.12);
  for (const s of [-1, 1]) for (let i = 0; i < 4; i++) tooth(s * 0.47, -0.3 + i * 0.2, 0.11);
}

/** The mimic's lid with the hinge at the origin: it reaches -Z (closed) with teeth underneath and a light strip in front. */
export function mimicLidModel(): Model {
  const m = new Model();
  m.box([0, 0.12, -0.55], [0.57, 0.11, 0.57], { part: PART_PLAIN, bevel: 0.04 });
  for (const z of [-0.85, -0.55, -0.25]) m.box([0, 0.234, z], [0.52, 0.004, 0.008], { part: PART_TRIM });
  const w = 0.045;
  for (const a of [-1, 1]) {
    m.box([a * 0.55, 0.12, -0.55], [w, 0.12, 0.57], { part: PART_TRIM, bevel: 0.01 });
    m.box([0, 0.12, -0.55 + a * 0.55], [0.57, 0.12, w], { part: PART_TRIM, bevel: 0.01 });
  }
  m.box([0, 0.12, -1.13], [0.3, 0.035, 0.01], { part: PART_GLOW });
  // Teeth hanging from the front edge of the lid (in lid space: the rim is at z -1.02).
  const t = new Model();
  teethRow(t, 0, -1);
  m.add(t, (p) => [p[0], p[1], p[2] - 0.55]);
  return m;
}

/** The mimic's tongue: a pink flat lobe sticking out along -Z from the origin, 0.7 u long. */
export function tongueModel(): Model {
  const m = new Model();
  const rows: V3[][] = [];
  for (let i = 0; i <= 8; i++) {
    const t = i / 8;
    const w = 0.16 * Math.sin(Math.min(1, 0.25 + t) * Math.PI * 0.62) + 0.02;
    const z = -0.7 * t;
    const y = -0.08 * t * t;
    rows.push([[-w, y - 0.025, z], [w, y - 0.025, z], [w * 0.9, y + 0.03, z], [-w * 0.9, y + 0.03, z]]);
  }
  m.loft(rows, { part: PART_FLESH, capStart: true, capEnd: true, group: m.smooth(60) });
  // The centre groove.
  m.box([0, 0.03, -0.35], [0.008, 0.006, 0.28], { part: PART_TRIM });
  return m;
}

/** A googly eye looking along -Z: a white ball, an iris in the instance colour and a black pupil (radius 1 at scale 1). */
export function eyeModel(): Model {
  const m = new Model();
  m.sphere([0, 0, 0], 1, 2, PART_ENAMEL, m.smooth(70));
  m.lathe([[0.0005, -1.02], [0.5, -0.88], [0.56, -0.83]], 18, { part: PART_LIVERY, axis: "z", group: m.smooth(70) });
  m.lathe([[0.0005, -1.035], [0.26, -0.98]], 14, { part: PART_TRIM, axis: "z", group: m.smooth(70) });
  m.sphere([0.18, 0.2, -0.98], 0.07, 0, PART_LAMP);
  return m;
}

/**
 * The gossip face (radius 1 at scale 1, looking along -Z out of the rock):
 * a stone mask with cheeks, a big nose, eye sockets, an upper lip with
 * lipstick, the dark mouth and its upper teeth. Brows, eyes, lids and the
 * jaw are separate meshes.
 */
export function gossipFaceModel(): Model {
  const m = new Model();
  const g = m.smooth(55);
  // The mask, bulging out of the rock (its back reaches into it).
  m.sphere([0, 0, 0.25], [0.95, 1.12, 0.62], 2, PART_STONE, g);
  // Cheeks, chin, brow ridge and the nose.
  for (const s of [-1, 1]) {
    m.sphere([s * 0.5, -0.24, -0.18], [0.3, 0.26, 0.24], 2, PART_STONE, g);
    // Eye sockets: dark hollows behind the eyeballs.
    m.sphere([s * 0.33, 0.2, -0.24], [0.24, 0.2, 0.08], 1, PART_TRIM, g);
  }
  // A soft forehead under the (separate) brows.
  m.sphere([0, 0.5, -0.05], [0.55, 0.12, 0.2], 2, PART_STONE, g);
  m.sphere([0, -0.02, -0.42], [0.15, 0.3, 0.2], 2, PART_STONE, g);
  m.sphere([0, -0.18, -0.52], [0.17, 0.13, 0.13], 2, PART_STONE, g);
  for (const s of [-1, 1]) m.sphere([s * 0.08, -0.24, -0.58], [0.04, 0.025, 0.03], 0, PART_TRIM);
  // The mouth: a dark opening, upper teeth and the upper lip in lipstick.
  m.sphere([0, -0.52, -0.3], [0.34, 0.15, 0.12], 1, PART_TRIM, g);
  m.box([0, -0.47, -0.38], [0.2, 0.035, 0.03], { part: PART_ENAMEL, bevel: 0.012 });
  const lip: V3[][] = [];
  for (let i = 0; i <= 12; i++) {
    const t = i / 12;
    const x = -0.38 + 0.76 * t;
    // A cupid's bow: two humps.
    const y = -0.42 + 0.035 * Math.sin(t * Math.PI * 2) * Math.sin(t * Math.PI) + 0.02 * Math.sin(t * Math.PI);
    const z = -0.38 - 0.06 * Math.sin(t * Math.PI);
    const r = 0.02 + 0.05 * Math.sin(t * Math.PI);
    lip.push(Array.from({ length: 6 }, (_, k) => {
      const a = (k / 6) * Math.PI * 2;
      return [x, y + Math.sin(a) * r * 0.8, z + Math.cos(a) * r] as V3;
    }));
  }
  m.loft(lip, { part: PART_FLESH, capStart: true, capEnd: true, group: m.smooth(70) });
  return m;
}

/** A stone brow (an arched lump about 0.5 u wide, centred at the origin). */
export function browModel(): Model {
  const m = new Model();
  const rows: V3[][] = [];
  for (let i = 0; i <= 8; i++) {
    const t = i / 8;
    const x = -0.26 + 0.52 * t;
    const y = 0.05 * Math.sin(t * Math.PI);
    const r = 0.03 + 0.05 * Math.sin(t * Math.PI);
    rows.push(Array.from({ length: 6 }, (_, k) => {
      const a = (k / 6) * Math.PI * 2;
      return [x, y + Math.sin(a) * r, Math.cos(a) * r * 1.2] as V3;
    }));
  }
  m.loft(rows, { part: PART_STONE, capStart: true, capEnd: true, group: m.smooth(70) });
  return m;
}

/** A stone eyelid: the upper half of a shell around an eyeball of radius 1 (hinged at the eye's centre, closed at +90 degrees about X). */
export function lidModel(): Model {
  const m = new Model();
  const prof: Array<[number, number]> = [];
  for (let i = 0; i <= 8; i++) {
    const a = (i / 8) * (Math.PI / 2);
    prof.push([Math.max(0.0005, Math.cos(a) * 1.08), Math.sin(a) * 1.08]);
  }
  // A dome over the top of the eye, cut to the front half.
  const half = new Model().lathe(prof, 16, { part: PART_STONE, axis: "y", group: m.smooth(70), capStart: true });
  for (const t of half.tris) {
    if ((t.a[2] + t.b[2] + t.c[2]) / 3 < 0.2) m.triangle(t.a, t.b, t.c, t.part, 0);
  }
  return m;
}

/** The gossip face's jaw: a stone chin with the lower lip in lipstick, hinged at the origin (the mouth corners). */
export function jawModel(): Model {
  const m = new Model();
  m.sphere([0, -0.2, -0.1], [0.4, 0.2, 0.22], 2, PART_STONE, m.smooth(55));
  const lip: V3[][] = [];
  for (let i = 0; i <= 10; i++) {
    const t = i / 10;
    const x = -0.34 + 0.68 * t;
    const y = -0.04 - 0.05 * Math.sin(t * Math.PI);
    const z = -0.3 - 0.05 * Math.sin(t * Math.PI);
    const r = 0.02 + 0.06 * Math.sin(t * Math.PI);
    lip.push(Array.from({ length: 6 }, (_, k) => {
      const a = (k / 6) * Math.PI * 2;
      return [x, y + Math.sin(a) * r * 0.8, z + Math.cos(a) * r] as V3;
    }));
  }
  m.loft(lip, { part: PART_FLESH, capStart: true, capEnd: true, group: m.smooth(70) });
  m.box([0, 0.0, -0.24], [0.17, 0.03, 0.03], { part: PART_ENAMEL, bevel: 0.012 });
  return m;
}

/** A model built Y-up turned so that its up is +Z (an instance turned by `fromZUp` stands upright and `stretch` scales its height). */
export function zUp(p: V3): V3 {
  return [p[0], -p[2], p[1]];
}

/** Mesh kinds of the visitors (renderer/models/index.ts MESH_BUILDERS). */
export const VISITOR_MESH_KINDS = [
  "visitor_rotor",
  "visitor_siren",
  "visitor_eye",
  "tax_drone",
  "tax_eyes",
  "tax_tie",
  "tax_receipt",
  "pizza_drone",
  "pizza_box",
  "pizza_logo",
  "pizza_slice",
  "pizza_crust",
  "survey_drone",
  "survey_panel",
  "survey_star",
  "mimic_body",
  "mimic_lid",
  "mimic_tongue",
  "gossip_face",
  "gossip_brow",
  "gossip_lid",
  "gossip_jaw",
] as const;

export type VisitorMeshKind = (typeof VISITOR_MESH_KINDS)[number];

const MODELS: Readonly<Record<VisitorMeshKind, () => Model>> = {
  visitor_rotor: rotorModel,
  visitor_siren: sirenDomeModel,
  visitor_eye: eyeModel,
  tax_drone: taxDroneModel,
  tax_eyes: taxEyesModel,
  tax_tie: tieModel,
  tax_receipt: receiptModel,
  pizza_drone: pizzaDroneModel,
  pizza_box: pizzaBoxModel,
  pizza_logo: pizzaLogoModel,
  pizza_slice: pizzaSliceModel,
  pizza_crust: pizzaCrustModel,
  survey_drone: surveyDroneModel,
  survey_panel: surveyPanelModel,
  survey_star: starModel,
  mimic_body: mimicBodyModel,
  mimic_lid: mimicLidModel,
  mimic_tongue: tongueModel,
  gossip_face: gossipFaceModel,
  gossip_brow: browModel,
  gossip_lid: lidModel,
  gossip_jaw: jawModel,
};

/** The visitor models by mesh kind (for tests and the builders). */
export function visitorModel(kind: VisitorMeshKind): Model {
  return MODELS[kind]();
}
