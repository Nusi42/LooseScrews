// Frame pipeline of the README recorder (record.mjs), pure and tested
// (tools/test/readme-media.test.mjs): decoding the browser's PNG screenshots to RGBA,
// drawing the pointer sprite and its click ring into a frame, and an animated GIF writer
// on top of gifenc that stores only what changed: each frame after the first covers the
// bounding box of the pixels that differ from what the viewer already sees, the rest of
// that box is transparent, and its palette is quantised from the changed pixels alone.

import { inflateSync } from "node:zlib";
import gifenc from "gifenc";

// gifenc ships CommonJS as its Node entry point (no named ESM exports there).
const { GIFEncoder, applyPalette, quantize } = gifenc;

// ----- PNG -------------------------------------------------------------------------

const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

/**
 * Decodes an 8-bit, non-interlaced RGB or RGBA PNG (what Chrome's screenshots are) to
 * `{ width, height, data }` with `data` RGBA (alpha 255 for RGB).
 */
export function decodePng(buf) {
  const b = buf instanceof Uint8Array ? buf : new Uint8Array(buf);
  for (let i = 0; i < 8; i++) if (b[i] !== PNG_SIGNATURE[i]) throw new Error("not a PNG");
  const view = new DataView(b.buffer, b.byteOffset, b.byteLength);
  let width = 0;
  let height = 0;
  let channels = 0;
  const idat = [];
  for (let p = 8; p < b.length; ) {
    const len = view.getUint32(p);
    const type = String.fromCharCode(b[p + 4], b[p + 5], b[p + 6], b[p + 7]);
    const body = b.subarray(p + 8, p + 8 + len);
    if (type === "IHDR") {
      width = view.getUint32(p + 8);
      height = view.getUint32(p + 12);
      const depth = body[8];
      const color = body[9];
      if (depth !== 8 || (color !== 2 && color !== 6) || body[12] !== 0) {
        throw new Error(`unsupported PNG (bit depth ${depth}, colour type ${color}, interlace ${body[12]})`);
      }
      channels = color === 6 ? 4 : 3;
    } else if (type === "IDAT") {
      idat.push(body);
    } else if (type === "IEND") {
      break;
    }
    p += 12 + len;
  }
  if (!channels) throw new Error("PNG without IHDR");
  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  const rows = new Uint8Array(stride * height);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    const src = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const out = rows.subarray(y * stride, (y + 1) * stride);
    const up = y > 0 ? rows.subarray((y - 1) * stride, y * stride) : null;
    for (let i = 0; i < stride; i++) {
      const a = i >= channels ? out[i - channels] : 0;
      const u = up ? up[i] : 0;
      const c = up && i >= channels ? up[i - channels] : 0;
      let v = src[i];
      if (filter === 1) v += a;
      else if (filter === 2) v += u;
      else if (filter === 3) v += (a + u) >> 1;
      else if (filter === 4) {
        const pa = Math.abs(u - c);
        const pb = Math.abs(a - c);
        const pc = Math.abs(a + u - 2 * c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? u : c;
      } else if (filter !== 0) throw new Error(`bad PNG filter ${filter}`);
      out[i] = v & 255;
    }
  }
  if (channels === 4) return { width, height, data: rows };
  const data = new Uint8Array(width * height * 4);
  for (let i = 0, j = 0; i < rows.length; i += 3, j += 4) {
    data[j] = rows[i];
    data[j + 1] = rows[i + 1];
    data[j + 2] = rows[i + 2];
    data[j + 3] = 255;
  }
  return { width, height, data };
}

// ----- pointer -----------------------------------------------------------------------

/** The pointer sprite (an arrow, tip at 0,0): K outline, W fill, anything else clear. */
export const POINTER_SPRITE = [
  "K",
  "KK",
  "KWK",
  "KWWK",
  "KWWWK",
  "KWWWWK",
  "KWWWWWK",
  "KWWWWWWK",
  "KWWWWWWWK",
  "KWWWWWWWWK",
  "KWWWWWWWWWK",
  "KWWWWWWKKKKK",
  "KWWWKWWK",
  "KWWKKWWK",
  "KWK..KWWK",
  "KK...KWWK",
  "K.....KWWK",
  "......KWWK",
  ".......KK",
];

const SPRITE_COLORS = { K: [16, 18, 24], W: [250, 250, 252] };

function blend(rgba, w, h, x, y, [r, g, b], alpha) {
  if (x < 0 || y < 0 || x >= w || y >= h) return;
  const i = (y * w + x) * 4;
  rgba[i] = Math.round(rgba[i] + (r - rgba[i]) * alpha);
  rgba[i + 1] = Math.round(rgba[i + 1] + (g - rgba[i + 1]) * alpha);
  rgba[i + 2] = Math.round(rgba[i + 2] + (b - rgba[i + 2]) * alpha);
}

/**
 * Draws the pointer into an RGBA frame of `w` x `h` with its tip at (`x`, `y`) (frame
 * pixels, rounded), each sprite pixel `scale` x `scale` frame pixels, with a soft shadow.
 */
export function drawPointer(rgba, w, h, x, y, scale = 1) {
  const x0 = Math.round(x);
  const y0 = Math.round(y);
  const s = Math.max(1, Math.round(scale));
  const paint = (dx, dy, color, alpha) => {
    for (let row = 0; row < POINTER_SPRITE.length; row++) {
      const line = POINTER_SPRITE[row];
      for (let col = 0; col < line.length; col++) {
        const c = line[col];
        if (c !== "K" && c !== "W") continue;
        for (let sy = 0; sy < s; sy++) {
          for (let sx = 0; sx < s; sx++) blend(rgba, w, h, x0 + col * s + sx + dx, y0 + row * s + sy + dy, color ?? SPRITE_COLORS[c], alpha);
        }
      }
    }
  };
  paint(s, s, [0, 0, 0], 0.35);
  paint(0, 0, null, 1);
}

/** Draws a ring (click feedback) of radius `r` and `thickness` px around (`cx`, `cy`). */
export function drawRing(rgba, w, h, cx, cy, r, { thickness = 2, color = [255, 179, 71], alpha = 1 } = {}) {
  const outer = r + thickness / 2;
  const inner = Math.max(0, r - thickness / 2);
  for (let y = Math.floor(cy - outer); y <= Math.ceil(cy + outer); y++) {
    for (let x = Math.floor(cx - outer); x <= Math.ceil(cx + outer); x++) {
      const d = Math.hypot(x - cx, y - cy);
      if (d >= inner && d <= outer) blend(rgba, w, h, x, y, color, alpha);
    }
  }
}

// ----- frame diffing -----------------------------------------------------------------

/**
 * Pixels of `next` that differ from `shown` by more than `tolerance` in any channel: a mask
 * (1 = changed) and their bounding box `{ x, y, w, h }`, or `rect: null` when none changed.
 */
export function changedPixels(shown, next, width, height, tolerance) {
  const mask = new Uint8Array(width * height);
  let x0 = width;
  let y0 = height;
  let x1 = -1;
  let y1 = -1;
  for (let y = 0, p = 0; y < height; y++) {
    for (let x = 0; x < width; x++, p++) {
      const i = p * 4;
      const d = Math.max(Math.abs(shown[i] - next[i]), Math.abs(shown[i + 1] - next[i + 1]), Math.abs(shown[i + 2] - next[i + 2]));
      if (d > tolerance) {
        mask[p] = 1;
        if (x < x0) x0 = x;
        if (x > x1) x1 = x;
        if (y < y0) y0 = y;
        if (y > y1) y1 = y;
      }
    }
  }
  return { mask, rect: x1 < 0 ? null : { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 } };
}

/**
 * Palette (at most `maxColors`, [r, g, b] entries) for the masked pixels of `rgba` inside
 * `rect`, and their palette indices in rect order; unmasked pixels get `fill`.
 */
export function quantizeMasked(rgba, width, mask, rect, maxColors, fill) {
  let n = 0;
  for (let y = rect.y; y < rect.y + rect.h; y++) for (let x = rect.x; x < rect.x + rect.w; x++) n += mask[y * width + x];
  const picked = new Uint8Array(n * 4);
  for (let y = rect.y, k = 0; y < rect.y + rect.h; y++) {
    for (let x = rect.x; x < rect.x + rect.w; x++) {
      const p = y * width + x;
      if (!mask[p]) continue;
      picked[k] = rgba[p * 4];
      picked[k + 1] = rgba[p * 4 + 1];
      picked[k + 2] = rgba[p * 4 + 2];
      picked[k + 3] = 255;
      k += 4;
    }
  }
  const palette = quantize(picked, maxColors);
  const picks = applyPalette(picked, palette);
  const index = new Uint8Array(rect.w * rect.h).fill(fill);
  for (let y = rect.y, k = 0, j = 0; y < rect.y + rect.h; y++) {
    for (let x = rect.x; x < rect.x + rect.w; x++, j++) if (mask[y * width + x]) index[j] = picks[k++];
  }
  return { palette, index };
}

// ----- GIF writer --------------------------------------------------------------------

/**
 * Animated GIF that stores frame differences. `add(rgba, durationMs)` appends a frame
 * shown for `durationMs`; `finish()` returns the file. A pixel is written again once it
 * differs by more than `tolerance` from the source colour it was last written from (not
 * from its quantised colour, so palette error alone never marks it changed); a frame
 * without such pixels only lengthens the previous one. Delays are rounded on the running
 * clock, so the total length stays exact.
 */
export class DiffGifWriter {
  constructor(width, height, { tolerance = 6, maxColors = 256 } = {}) {
    this.width = width;
    this.height = height;
    this.tolerance = tolerance;
    this.maxColors = Math.min(256, Math.max(2, maxColors));
    this.gif = GIFEncoder();
    /** What a viewer sees after the frames written so far (RGBA). */
    this.shown = null;
    /** The source colour each pixel was last written from (RGBA). */
    this.ref = null;
    this.clock = 0;
    this.last = null; // { gce, start } of the last written frame
    this.frames = 0;
  }

  add(rgba, durationMs) {
    const { width, height } = this;
    if (rgba.length !== width * height * 4) throw new Error(`frame is not ${width}x${height}`);
    const start = this.clock;
    this.clock += durationMs;
    if (!this.shown) {
      const full = { x: 0, y: 0, w: width, h: height };
      const all = new Uint8Array(width * height).fill(1);
      const { palette, index } = quantizeMasked(rgba, width, all, full, this.maxColors, 0);
      this.shown = new Uint8Array(rgba.length);
      this.ref = new Uint8Array(rgba.length);
      this.#write(rgba, index, full, palette, null, all, start);
      return;
    }
    const { mask, rect } = changedPixels(this.ref, rgba, width, height, this.tolerance);
    if (!rect) {
      this.#patchDelay();
      return;
    }
    const { palette, index } = quantizeMasked(rgba, width, mask, rect, this.maxColors - 1, 0);
    const transparent = palette.length;
    for (let j = 0, y = rect.y; y < rect.y + rect.h; y++) {
      for (let x = rect.x; x < rect.x + rect.w; x++, j++) if (!mask[y * width + x]) index[j] = transparent;
    }
    this.#write(rgba, index, rect, [...palette, [0, 0, 0]], transparent, mask, start);
  }

  finish() {
    this.gif.finish();
    return this.gif.bytes();
  }

  #write(rgba, index, rect, palette, transparent, mask, start) {
    const stream = this.gif.stream;
    const before = stream.bytesView().length;
    const first = this.frames === 0;
    this.gif.writeFrame(index, rect.w, rect.h, {
      palette,
      delay: this.#centis(start, this.clock) * 10,
      ...(transparent === null ? {} : { transparent: true, transparentIndex: transparent }),
      // Keep what the earlier frames drew (the transparent pixels show it).
      dispose: 1,
      repeat: 0,
    });
    const bytes = stream.bytesView();
    // gifenc writes every frame at 0,0: move this one to its rectangle. First frame: header
    // (6), screen descriptor (7), global colour table, looping extension (19), then the GCE.
    const gce = first ? before + 6 + 7 + 3 * (1 << Math.max(1, Math.ceil(Math.log2(palette.length)))) + 19 : before;
    if (bytes[gce] !== 0x21 || bytes[gce + 1] !== 0xf9 || bytes[gce + 8] !== 0x2c) throw new Error("unexpected gifenc frame layout");
    bytes[gce + 9] = rect.x & 255;
    bytes[gce + 10] = rect.x >> 8;
    bytes[gce + 11] = rect.y & 255;
    bytes[gce + 12] = rect.y >> 8;
    this.last = { gce, start };
    this.frames++;
    for (let j = 0, y = rect.y; y < rect.y + rect.h; y++) {
      for (let x = rect.x; x < rect.x + rect.w; x++, j++) {
        const p = y * this.width + x;
        if (!mask[p]) continue;
        const c = palette[index[j]];
        this.shown[p * 4] = c[0];
        this.shown[p * 4 + 1] = c[1];
        this.shown[p * 4 + 2] = c[2];
        this.shown[p * 4 + 3] = 255;
        this.ref.set(rgba.subarray(p * 4, p * 4 + 4), p * 4);
      }
    }
  }

  /** Lengthens the last written frame up to the current clock. */
  #patchDelay() {
    const bytes = this.gif.stream.bytesView();
    const cs = this.#centis(this.last.start, this.clock);
    bytes[this.last.gce + 4] = cs & 255;
    bytes[this.last.gce + 5] = cs >> 8;
  }

  #centis(start, end) {
    return Math.max(2, Math.round(end / 10) - Math.round(start / 10));
  }
}
