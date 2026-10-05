// Offline preview of the procedural sounds (M4 A6): renders every sound of
// web/src/audio/sounds and an announcer line per voice pack to WAV files,
// draws a spectrogram PNG per sound (log frequency, dB colour, waveform
// envelope on top) and writes a table of durations, levels and render times.
// Lets you review the sound set without a browser, and lets agents "look" at
// what they made.
//
//   node web/scripts/audio-preview.ts [--out <dir>] [--only <id-prefix>] [--rate <Hz>]
//
// Defaults: --out target/audio-preview, every sound, 48000 Hz.

import { mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { deflateSync } from "node:zlib";
import { defDuration, levels, renderSynth } from "../src/audio/dsp.ts";
import { BUILTIN_SOUNDS } from "../src/audio/sounds/index.ts";
import { PACKS } from "../src/audio/announcer/packs.ts";
import { fillLine } from "../src/audio/announcer/lines.ts";
import { renderSpeech } from "../src/audio/voice.ts";

const args = process.argv.slice(2);
const arg = (name: string, dflt: string): string => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 && args[i + 1] ? (args[i + 1] as string) : dflt;
};
const out = resolve(arg("out", join(import.meta.dirname, "..", "..", "target", "audio-preview")));
const only = arg("only", "");
const SR = Number(arg("rate", "48000"));
mkdirSync(out, { recursive: true });

function wav(samples: Float32Array, sr: number): Buffer {
  const n = samples.length;
  const b = Buffer.alloc(44 + n * 2);
  b.write("RIFF", 0);
  b.writeUInt32LE(36 + n * 2, 4);
  b.write("WAVEfmt ", 8);
  b.writeUInt32LE(16, 16);
  b.writeUInt16LE(1, 20);
  b.writeUInt16LE(1, 22);
  b.writeUInt32LE(sr, 24);
  b.writeUInt32LE(sr * 2, 28);
  b.writeUInt16LE(2, 32);
  b.writeUInt16LE(16, 34);
  b.write("data", 36);
  b.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) b.writeInt16LE(Math.round(Math.max(-1, Math.min(1, samples[i] ?? 0)) * 32767), 44 + i * 2);
  return b;
}

// ----- PNG ------------------------------------------------------------------------
const CRC = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buf: Buffer): number {
  let c = 0xffffffff;
  for (const x of buf) c = (CRC[(c ^ x) & 0xff] as number) ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type: string, data: Buffer): Buffer {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(td));
  return Buffer.concat([len, td, crc]);
}
function png(w: number, h: number, rgb: Uint8Array): Buffer {
  const raw = Buffer.alloc((w * 3 + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 3 + 1)] = 0;
    for (let i = 0; i < w * 3; i++) raw[y * (w * 3 + 1) + 1 + i] = rgb[y * w * 3 + i] ?? 0;
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}

// ----- spectrogram -----------------------------------------------------------------
function fft(re: Float64Array, im: Float64Array): void {
  const n = re.length;
  for (let i = 1, j = 0; i < n; i++) {
    let bit = n >> 1;
    for (; j & bit; bit >>= 1) j ^= bit;
    j ^= bit;
    if (i < j) {
      [re[i], re[j]] = [re[j] as number, re[i] as number];
      [im[i], im[j]] = [im[j] as number, im[i] as number];
    }
  }
  for (let len = 2; len <= n; len <<= 1) {
    const ang = (-2 * Math.PI) / len;
    for (let i = 0; i < n; i += len) {
      for (let k = 0; k < len / 2; k++) {
        const wr = Math.cos(ang * k);
        const wi = Math.sin(ang * k);
        const a = i + k;
        const b = a + len / 2;
        const xr = (re[b] as number) * wr - (im[b] as number) * wi;
        const xi = (re[b] as number) * wi + (im[b] as number) * wr;
        re[b] = (re[a] as number) - xr;
        im[b] = (im[a] as number) - xi;
        re[a] = (re[a] as number) + xr;
        im[a] = (im[a] as number) + xi;
      }
    }
  }
}

/** Inferno-like colour map 0..1. */
function color(t: number): [number, number, number] {
  const stops: [number, number, number][] = [
    [0, 0, 4],
    [40, 11, 84],
    [101, 21, 110],
    [159, 42, 99],
    [212, 72, 66],
    [245, 125, 21],
    [250, 193, 39],
    [252, 255, 164],
  ];
  const x = Math.max(0, Math.min(1, t)) * (stops.length - 1);
  const i = Math.min(stops.length - 2, Math.floor(x));
  const f = x - i;
  const a = stops[i] as [number, number, number];
  const b = stops[i + 1] as [number, number, number];
  return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f];
}

function spectrogram(s: Float32Array, sr: number): Buffer {
  const N = 2048;
  const hop = Math.max(64, Math.round(sr * 0.004));
  const frames = Math.max(1, Math.ceil(s.length / hop));
  const W = Math.min(1400, Math.max(200, frames));
  const H = 260;
  const WAVE = 70;
  const fLo = 30;
  const fHi = 18000;
  const rgb = new Uint8Array(W * (H + WAVE) * 3);
  const win = new Float64Array(N).map((_, i) => 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / (N - 1)));
  for (let x = 0; x < W; x++) {
    const centre = Math.round(((x + 0.5) / W) * s.length);
    const re = new Float64Array(N);
    const im = new Float64Array(N);
    for (let i = 0; i < N; i++) re[i] = (s[centre - N / 2 + i] ?? 0) * (win[i] as number);
    fft(re, im);
    for (let y = 0; y < H; y++) {
      const f = fLo * Math.pow(fHi / fLo, 1 - y / (H - 1));
      const bin = Math.min(N / 2 - 1, Math.max(1, Math.round((f / sr) * N)));
      const mag = Math.hypot(re[bin] as number, im[bin] as number) / (N / 4);
      const db = 20 * Math.log10(mag + 1e-9);
      const c = color((db + 90) / 90);
      const o = ((y + WAVE) * W + x) * 3;
      rgb[o] = c[0];
      rgb[o + 1] = c[1];
      rgb[o + 2] = c[2];
    }
    // Waveform peak strip.
    const a = Math.floor((x / W) * s.length);
    const b = Math.floor(((x + 1) / W) * s.length);
    let lo = 0;
    let hi = 0;
    for (let i = a; i < b; i++) {
      lo = Math.min(lo, s[i] ?? 0);
      hi = Math.max(hi, s[i] ?? 0);
    }
    for (let y = 0; y < WAVE; y++) {
      const v = 1 - (y / (WAVE - 1)) * 2;
      const on = v <= hi && v >= lo;
      const o = (y * W + x) * 3;
      const g = on ? 200 : y === WAVE >> 1 ? 60 : 18;
      rgb[o] = on ? 120 : g;
      rgb[o + 1] = on ? 220 : g;
      rgb[o + 2] = on ? 255 : g;
    }
  }
  // Octave grid lines (100 Hz, 1 kHz, 10 kHz).
  for (const f of [100, 1000, 10000]) {
    const y = Math.round((1 - Math.log(f / fLo) / Math.log(fHi / fLo)) * (H - 1)) + WAVE;
    for (let x = 0; x < W; x += 3) {
      const o = (y * W + x) * 3;
      rgb[o] = 90;
      rgb[o + 1] = 160;
      rgb[o + 2] = 90;
    }
  }
  return png(W, H + WAVE, rgb);
}

// ----- render ------------------------------------------------------------------------
const rows: string[] = ["| id | length (s) | peak | RMS dBFS | render (ms) |", "| --- | --- | --- | --- | --- |"];
let totalMs = 0;
for (const d of BUILTIN_SOUNDS) {
  if (only && !d.id.startsWith(only)) continue;
  const t0 = performance.now();
  const s = renderSynth(d.synth, SR, 1);
  const ms = performance.now() - t0;
  totalMs += ms;
  const lv = levels(s);
  rows.push(`| ${d.id} | ${defDuration(d.synth).toFixed(2)} | ${lv.peak.toFixed(2)} | ${(20 * Math.log10(lv.rms + 1e-9)).toFixed(1)} | ${ms.toFixed(1)} |`);
  writeFileSync(join(out, `${d.id}.wav`), wav(s, SR));
  writeFileSync(join(out, `${d.id}.png`), spectrogram(s, SR));
}
for (const pack of PACKS) {
  if (only && !`voice.${pack.id}`.startsWith(only)) continue;
  const line = fillLine(pack.lines["firstBlood"]?.[0] ?? "Hello there.", { killer: "Rivet", victim: "Sprocket" });
  const t0 = performance.now();
  const sp = renderSpeech(line, pack.voice, SR, 3);
  const ms = performance.now() - t0;
  totalMs += ms;
  const lv = levels(sp.samples);
  rows.push(`| voice.${pack.id} "${line}" | ${(sp.samples.length / SR).toFixed(2)} | ${lv.peak.toFixed(2)} | ${(20 * Math.log10(lv.rms + 1e-9)).toFixed(1)} | ${ms.toFixed(1)} |`);
  writeFileSync(join(out, `voice.${pack.id}.wav`), wav(sp.samples, SR));
  writeFileSync(join(out, `voice.${pack.id}.png`), spectrogram(sp.samples, SR));
}
rows.push(`| total | | | | ${totalMs.toFixed(0)} |`);
writeFileSync(join(out, "index.md"), rows.join("\n") + "\n");
console.log(rows.join("\n"));
console.log(`wrote ${out}`);
