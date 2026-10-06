// Wrong genre's 8-bit look (M4 C1, milestone-4-plan.md §3.2): the composite
// (shaders/post.ts) samples the frame in square cells (a quarter of the
// resolution at 1080p: 4 px cells) and maps every cell to the nearest of 16
// colours (the PICO-8 palette) after a 4x4 ordered dither, so the 3D frame
// reads as a console game from the 80s. `PostSettings.pixel` drives it;
// absent or `cell` 1 with `palette` 0 is off. The game eases it in steps
// (1, 2, 3, 4 px: an old console "powering on") through `pixelLookAt`.

/** Pixel look: cell edge in screen pixels, palette and dither share (0..1). */
export interface PixelLook {
  /** Cell edge (px); 1 = full resolution. */
  cell: number;
  /** How much of the 16-colour palette replaces the colour (0..1). */
  palette: number;
  /** Ordered dither strength (0..1). */
  dither: number;
}

export const NO_PIXEL: PixelLook = { cell: 1, palette: 0, dither: 0 };

/** The 16 colours (display sRGB bytes, PICO-8). */
const PICO8: ReadonlyArray<readonly [number, number, number]> = [
  [0x00, 0x00, 0x00],
  [0x1d, 0x2b, 0x53],
  [0x7e, 0x25, 0x53],
  [0x00, 0x87, 0x51],
  [0xab, 0x52, 0x36],
  [0x5f, 0x57, 0x4f],
  [0xc2, 0xc3, 0xc7],
  [0xff, 0xf1, 0xe8],
  [0xff, 0x00, 0x4d],
  [0xff, 0xa3, 0x00],
  [0xff, 0xec, 0x27],
  [0x00, 0xe4, 0x36],
  [0x29, 0xad, 0xff],
  [0x83, 0x76, 0x9c],
  [0xff, 0x77, 0xa8],
  [0xff, 0xcc, 0xaa],
];

/** The 16 colours (display sRGB 0..1). */
export const PIXEL_PALETTE: ReadonlyArray<readonly [number, number, number]> = PICO8.map(([r, g, b]) => [r / 255, g / 255, b / 255] as const);

/** Whether the composite runs the pixel branch. */
export function pixelActive(p: PixelLook | undefined): boolean {
  return !!p && (p.cell > 1 || p.palette > 0);
}

/** Cell edge for the quarter-resolution look at a drawing-buffer height (4 px at 1080p, at least 2). */
export function quarterCell(height: number): number {
  return Math.max(2, Math.round(height / 270));
}

/**
 * The look `amount` (0..1) of the way in: the cell grows in whole pixels
 * (1, 2, ... `full`), the palette and dither follow.
 */
export function pixelLookAt(amount: number, full: number): PixelLook {
  const a = Math.min(1, Math.max(0, amount));
  if (a <= 0) return NO_PIXEL;
  const cell = Math.max(1, Math.min(full, 1 + Math.floor(a * full)));
  return { cell, palette: Math.min(1, a * 1.6), dither: Math.min(1, a * 1.6) };
}

/** Uniform floats: [cell, palette, dither, on]. */
export function packPixel(p: PixelLook | undefined): [number, number, number, number] {
  if (!p || !pixelActive(p)) return [1, 0, 0, 0];
  return [Math.max(1, Math.round(p.cell)), Math.min(1, Math.max(0, p.palette)), Math.min(1, Math.max(0, p.dither)), 1];
}

/** Colour distance of the palette match: luma and two colour differences (YCbCr-like), so blues stay blue. */
export function paletteDistance(a: readonly [number, number, number], b: readonly [number, number, number]): number {
  const ya = 0.299 * a[0] + 0.587 * a[1] + 0.114 * a[2];
  const yb = 0.299 * b[0] + 0.587 * b[1] + 0.114 * b[2];
  const dy = ya - yb;
  const dcb = a[2] - ya - (b[2] - yb);
  const dcr = a[0] - ya - (b[0] - yb);
  return dy * dy + 0.55 * (dcb * dcb + dcr * dcr);
}

/** Nearest palette colour (the same rule as the shader), for tests. */
export function nearestPaletteColor(c: readonly [number, number, number]): readonly [number, number, number] {
  let best = PIXEL_PALETTE[0]!;
  let bd = Infinity;
  for (const p of PIXEL_PALETTE) {
    const d = paletteDistance(c, p);
    if (d < bd) {
      bd = d;
      best = p;
    }
  }
  return best;
}

const f = (x: number): string => x.toFixed(5);

/**
 * WGSL for the composite: `pixel_center(px)` (the cell's centre pixel) and
 * `pixel_palette(c, px)` (display sRGB in, quantized out). Reads
 * `post.pixel` (x cell, y palette, z dither, w on).
 */
export const PIXEL_WGSL = /* wgsl */ `
fn pixel_center(px: vec2f) -> vec2f {
  let cell = max(post.pixel.x, 1.0);
  return (floor(px / cell) + 0.5) * cell;
}

// 4x4 Bayer threshold in [0, 1) of a cell.
fn bayer4(c: vec2f) -> f32 {
  let x = u32(c.x) & 3u;
  let y = u32(c.y) & 3u;
  let m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
  return (m[y * 4u + x] + 0.5) / 16.0;
}

// Consoles of the 80s were bright and saturated: lift the mid-tones and
// push the saturation before snapping to the palette, so shadows read as
// the palette's navy and lit rock as its browns and peaches, not as mud.
fn pixel_pregrade(c: vec3f) -> vec3f {
  let lifted = pow(max(c, vec3f(0.0)), vec3f(0.72));
  let l = dot(lifted, vec3f(0.3, 0.59, 0.11));
  return clamp(vec3f(l) + (lifted - vec3f(l)) * 1.5, vec3f(0.0), vec3f(1.0));
}

fn pixel_palette(c0: vec3f, px: vec2f) -> vec3f {
  let c = mix(c0, pixel_pregrade(c0), post.pixel.y);
  var pal = array<vec3f, 16>(
${PIXEL_PALETTE.map((p) => `    vec3f(${f(p[0])}, ${f(p[1])}, ${f(p[2])})`).join(",\n")}
  );
  let cell = floor(px / max(post.pixel.x, 1.0));
  let d = c + vec3f((bayer4(cell) - 0.5) * 0.09 * post.pixel.z);
  var best = pal[0];
  var bd = 1e9;
  let yd = dot(d, vec3f(0.299, 0.587, 0.114));
  for (var i = 0u; i < 16u; i++) {
    let p = pal[i];
    let yp = dot(p, vec3f(0.299, 0.587, 0.114));
    let dy = yd - yp;
    let dcb = (d.b - yd) - (p.b - yp);
    let dcr = (d.r - yd) - (p.r - yp);
    let w = dy * dy + 0.55 * (dcb * dcb + dcr * dcr);
    if (w < bd) {
      bd = w;
      best = pal[i];
    }
  }
  // A faint scanline on the last pixel row of every cell (a CRT's grille).
  let row = px.y - cell.y * max(post.pixel.x, 1.0);
  let scan = select(1.0, 0.86, post.pixel.x >= 3.0 && row >= post.pixel.x - 1.0);
  return mix(c0, best * scan, post.pixel.y);
}
`;
