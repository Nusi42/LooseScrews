// tools/readme-media/media.mjs: PNG decoding, the pointer sprite and click ring, frame
// diffing and the GIF writer (decoded back here and composited like a browser does).

import assert from "node:assert/strict";
import { test } from "node:test";
import { deflateSync } from "node:zlib";

import { changedPixels, decodePng, DiffGifWriter, drawPointer, drawRing, POINTER_SPRITE, quantizeMasked } from "../readme-media/media.mjs";

// ----- helpers ---------------------------------------------------------------------

/** A PNG of `rgba` (`channels` 3 or 4) with filter `filters[y % n]` on row y. */
function encodePng(width, height, rgba, channels, filters) {
  const stride = width * channels;
  const rows = new Uint8Array(height * stride);
  for (let p = 0, q = 0; p < width * height; p++) for (let c = 0; c < channels; c++) rows[q++] = rgba[p * 4 + c];
  const raw = new Uint8Array(height * (stride + 1));
  for (let y = 0; y < height; y++) {
    const f = filters[y % filters.length];
    raw[y * (stride + 1)] = f;
    for (let i = 0; i < stride; i++) {
      const x = rows[y * stride + i];
      const a = i >= channels ? rows[y * stride + i - channels] : 0;
      const u = y > 0 ? rows[(y - 1) * stride + i] : 0;
      const c = y > 0 && i >= channels ? rows[(y - 1) * stride + i - channels] : 0;
      const pa = Math.abs(u - c);
      const pb = Math.abs(a - c);
      const pc = Math.abs(a + u - 2 * c);
      const paeth = pa <= pb && pa <= pc ? a : pb <= pc ? u : c;
      const pred = [0, a, u, (a + u) >> 1, paeth][f];
      raw[y * (stride + 1) + 1 + i] = (x - pred) & 255;
    }
  }
  const chunk = (type, body) => {
    const b = Buffer.alloc(12 + body.length);
    b.writeUInt32BE(body.length, 0);
    b.write(type, 4, "latin1");
    Buffer.from(body).copy(b, 8);
    return b; // CRC left 0: the decoder does not check it
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = channels === 4 ? 6 : 2;
  const z = deflateSync(raw);
  // Two IDAT chunks: the decoder must join them.
  const half = z.length >> 1;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", z.subarray(0, half)),
    chunk("IDAT", z.subarray(half)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function noise(width, height, seed, alpha = 255) {
  let s = seed >>> 0;
  const out = new Uint8Array(width * height * 4);
  for (let i = 0; i < out.length; i++) {
    s = (s * 1664525 + 1013904223) >>> 0;
    out[i] = (i & 3) === 3 ? alpha : s >>> 24;
  }
  return out;
}

/** Decodes a GIF into the composited RGBA frames and each frame's rectangle and delay (cs). */
function decodeGif(bytes) {
  const b = Buffer.from(bytes);
  assert.equal(b.toString("latin1", 0, 6), "GIF89a");
  const width = b.readUInt16LE(6);
  const height = b.readUInt16LE(8);
  let p = 13;
  let global = null;
  if (b[10] & 0x80) {
    const n = 1 << ((b[10] & 7) + 1);
    global = [];
    for (let i = 0; i < n; i++) global.push([b[p + 3 * i], b[p + 3 * i + 1], b[p + 3 * i + 2]]);
    p += 3 * n;
  }
  const canvas = new Uint8Array(width * height * 4);
  const frames = [];
  let gce = null;
  const subBlocks = () => {
    const parts = [];
    while (b[p] !== 0) {
      parts.push(b.subarray(p + 1, p + 1 + b[p]));
      p += 1 + b[p];
    }
    p++;
    return Buffer.concat(parts);
  };
  for (;;) {
    const tag = b[p++];
    if (tag === 0x3b) break;
    if (tag === 0x21) {
      const label = b[p++];
      const data = subBlocks();
      if (label === 0xf9) gce = { dispose: (data[0] >> 2) & 7, transparent: data[0] & 1 ? data[3] : -1, delay: data.readUInt16LE(1) };
      continue;
    }
    assert.equal(tag, 0x2c, `block 0x${tag.toString(16)}`);
    const x = b.readUInt16LE(p);
    const y = b.readUInt16LE(p + 2);
    const w = b.readUInt16LE(p + 4);
    const h = b.readUInt16LE(p + 6);
    const flags = b[p + 8];
    p += 9;
    let palette = global;
    if (flags & 0x80) {
      const n = 1 << ((flags & 7) + 1);
      palette = [];
      for (let i = 0; i < n; i++) palette.push([b[p + 3 * i], b[p + 3 * i + 1], b[p + 3 * i + 2]]);
      p += 3 * n;
    }
    const minCode = b[p++];
    const index = lzwDecode(subBlocks(), minCode, w * h);
    for (let j = 0; j < w * h; j++) {
      if (index[j] === gce?.transparent) continue;
      const q = ((y + Math.floor(j / w)) * width + x + (j % w)) * 4;
      const c = palette[index[j]];
      canvas[q] = c[0];
      canvas[q + 1] = c[1];
      canvas[q + 2] = c[2];
      canvas[q + 3] = 255;
    }
    frames.push({ rect: { x, y, w, h }, delay: gce?.delay ?? 0, dispose: gce?.dispose ?? 0, rgba: canvas.slice() });
    gce = null;
  }
  return { width, height, frames };
}

function lzwDecode(data, minCode, count) {
  const out = new Uint8Array(count);
  const clear = 1 << minCode;
  const end = clear + 1;
  let size = minCode + 1;
  let dict = [];
  const reset = () => {
    dict = [];
    for (let i = 0; i < clear; i++) dict.push([i]);
    dict.push(null, null);
    size = minCode + 1;
  };
  reset();
  let bits = 0;
  let acc = 0;
  let prev = null;
  let n = 0;
  for (let i = 0; i < data.length && n < count; i++) {
    acc |= data[i] << bits;
    bits += 8;
    while (bits >= size && n < count) {
      const code = acc & ((1 << size) - 1);
      acc >>= size;
      bits -= size;
      if (code === clear) {
        reset();
        prev = null;
        continue;
      }
      if (code === end) return out;
      let entry;
      if (code < dict.length && dict[code]) entry = dict[code];
      else entry = [...prev, prev[0]];
      for (const v of entry) out[n++] = v;
      if (prev) dict.push([...prev, entry[0]]);
      if (dict.length === 1 << size && size < 12) size++;
      prev = entry;
    }
  }
  return out;
}

const maxDiff = (a, b) => {
  let m = 0;
  for (let i = 0; i < a.length; i++) if ((i & 3) !== 3) m = Math.max(m, Math.abs(a[i] - b[i]));
  return m;
};

// ----- tests -----------------------------------------------------------------------

test("decodePng reads RGB and RGBA screenshots with every row filter", () => {
  const w = 7;
  const h = 6;
  const rgba = noise(w, h, 7, 200);
  for (const channels of [3, 4]) {
    const png = decodePng(encodePng(w, h, rgba, channels, [0, 1, 2, 3, 4]));
    assert.equal(png.width, w);
    assert.equal(png.height, h);
    for (let i = 0; i < rgba.length; i++) assert.equal(png.data[i], (i & 3) === 3 && channels === 3 ? 255 : rgba[i], `byte ${i} (${channels} channels)`);
  }
  assert.throws(() => decodePng(Buffer.from("not a png at all")), /not a PNG/);
});

test("the pointer is drawn with its tip at the given point, clipped at the frame edge", () => {
  const w = 40;
  const h = 40;
  const frame = new Uint8Array(w * h * 4).fill(128);
  drawPointer(frame, w, h, 10, 5);
  const px = (x, y) => [...frame.subarray((y * w + x) * 4, (y * w + x) * 4 + 3)];
  assert.deepEqual(px(10, 5), [16, 18, 24], "the tip is outline");
  assert.deepEqual(px(11, 7), [250, 250, 252], "inside the arrow is white");
  assert.deepEqual(px(9, 5), [128, 128, 128], "left of the tip is untouched");
  assert.deepEqual(px(30, 5), [128, 128, 128], "right of the arrow is untouched");
  // Scale 2: the sprite covers twice the rows.
  const big = new Uint8Array(w * h * 4).fill(128);
  drawPointer(big, w, h, 0, 0, 2);
  const rows = POINTER_SPRITE.length;
  assert.deepEqual([...big.subarray(((2 * rows - 1) * w + 14) * 4, ((2 * rows - 1) * w + 14) * 4 + 3)], [16, 18, 24]);
  // Off the edge: no throw, nothing outside the buffer.
  const edge = new Uint8Array(w * h * 4).fill(128);
  drawPointer(edge, w, h, w - 2, h - 3, 3);
  assert.equal(edge.length, w * h * 4);
});

test("the click ring colours a circle of the given radius and leaves its centre", () => {
  const w = 32;
  const frame = new Uint8Array(w * w * 4);
  drawRing(frame, w, w, 16, 16, 8, { thickness: 2, color: [255, 179, 71] });
  const at = (x, y) => frame[(y * w + x) * 4];
  assert.equal(at(24, 16), 255);
  assert.equal(at(16, 8), 255);
  assert.equal(at(16, 16), 0);
  assert.equal(at(28, 16), 0);
});

test("changedPixels masks only differences above the tolerance and bounds them", () => {
  const w = 10;
  const h = 8;
  const a = new Uint8Array(w * h * 4).fill(50);
  const b = a.slice();
  b[(2 * w + 3) * 4] = 50 + 5; // within tolerance 6
  b[(4 * w + 6) * 4 + 1] = 50 + 20;
  b[(6 * w + 2) * 4 + 2] = 50 - 30;
  const { mask, rect } = changedPixels(a, b, w, h, 6);
  assert.deepEqual(rect, { x: 2, y: 4, w: 5, h: 3 });
  assert.equal(mask.reduce((s, v) => s + v, 0), 2);
  assert.equal(changedPixels(a, a, w, h, 0).rect, null);
});

test("quantizeMasked builds the palette from the masked pixels only", () => {
  const w = 4;
  const rgba = new Uint8Array(w * w * 4);
  const mask = new Uint8Array(w * w);
  for (let p = 0; p < w * w; p++) {
    const red = p % 2 === 0;
    rgba.set(red ? [255, 0, 0, 255] : [0, 0, 255, 255], p * 4);
    mask[p] = red ? 1 : 0;
  }
  const { palette, index } = quantizeMasked(rgba, w, mask, { x: 0, y: 0, w, h: w }, 16, 9);
  assert.equal(palette.length, 1, "blue is never masked, so only red is in the palette");
  assert.deepEqual(palette[0].slice(0, 3), [255, 0, 0]);
  for (let p = 0; p < w * w; p++) assert.equal(index[p], p % 2 === 0 ? 0 : 9);
});

test("DiffGifWriter stores changed rectangles that composite back to the frames", () => {
  const w = 48;
  const h = 32;
  const base = new Uint8Array(w * h * 4);
  for (let p = 0; p < w * h; p++) base.set([20 + (p % w) * 4, 30 + Math.floor(p / w) * 6, 90, 255], p * 4);
  const f1 = base.slice();
  drawPointer(f1, w, h, 10, 4);
  const f2 = f1.slice(); // unchanged: lengthens frame 1
  const f3 = base.slice();
  drawPointer(f3, w, h, 30, 12);
  drawRing(f3, w, h, 30, 12, 5);
  const writer = new DiffGifWriter(w, h, { tolerance: 4 });
  writer.add(base, 100);
  writer.add(f1, 80);
  writer.add(f2, 80);
  writer.add(f3, 120);
  const gif = decodeGif(writer.finish());
  assert.equal(gif.width, w);
  assert.equal(gif.height, h);
  assert.equal(gif.frames.length, 3, "the unchanged frame is merged into the one before");
  assert.deepEqual(gif.frames[0].rect, { x: 0, y: 0, w, h });
  assert.deepEqual(gif.frames.map((f) => f.delay), [10, 16, 12]);
  assert.ok(gif.frames.every((f) => f.dispose === 1), "frames are kept, not cleared");
  // Frame 2 covers the old pointer (erased in frame 3) and the new one with its ring, not the whole frame.
  const r = gif.frames[2].rect;
  assert.ok(r.x <= 10 && r.y <= 4 && r.x + r.w >= 30 + 6 && r.w * r.h < w * h, JSON.stringify(r));
  const r1 = gif.frames[1].rect;
  assert.ok(r1.x >= 10 && r1.y >= 4 && r1.w <= POINTER_SPRITE[11].length + 1 && r1.h <= POINTER_SPRITE.length + 1, JSON.stringify(r1));
  // What a viewer sees matches each source frame within the tolerance plus quantisation error.
  for (const [i, src] of [base, f1, f3].entries()) assert.ok(maxDiff(gif.frames[i].rgba, src) <= 12, `frame ${i}: ${maxDiff(gif.frames[i].rgba, src)}`);
  assert.deepEqual([...gif.frames[2].rgba], [...writer.shown], "the writer's model of the shown image is exact");
});

test("DiffGifWriter keeps the running clock: rounded delays add up to the total", () => {
  const w = 8;
  const h = 8;
  const writer = new DiffGifWriter(w, h);
  for (let i = 0; i < 9; i++) writer.add(noise(w, h, i + 1), 83);
  const gif = decodeGif(writer.finish());
  assert.equal(gif.frames.length, 9);
  assert.equal(gif.frames.reduce((s, f) => s + f.delay, 0), Math.round((9 * 83) / 10));
  assert.throws(() => writer.add(new Uint8Array(4), 10), /not 8x8/);
});
