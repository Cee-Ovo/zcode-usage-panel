#!/usr/bin/env node
/**
 * Generates src-tauri/icons/source.png (1024×1024) with ZERO dependencies:
 * a hand-rolled PNG encoder (zlib is built into Node) plus SDF rasterization
 * for a rounded-square "liquid glass" icon with a white Z glyph.
 *
 * Output is fed to `tauri icon` (see ensure-icons.mjs) which derives the
 * full icon set including icon.ico.
 */

import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const OUT = join(__dirname, "..", "src-tauri", "icons", "source.png");
const SIZE = 1024;

// ---- PNG encoder -----------------------------------------------------------

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body), 0);
  return Buffer.concat([len, body, crc]);
}

function encodePng(width, height, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // color type RGBA
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;
  // scanlines with filter byte 0
  const raw = Buffer.alloc(height * (1 + width * 4));
  for (let y = 0; y < height; y++) {
    raw[y * (1 + width * 4)] = 0;
    rgba.copy(raw, y * (1 + width * 4) + 1, y * width * 4, (y + 1) * width * 4);
  }
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ---- SDF drawing -----------------------------------------------------------

const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
const smooth = (d) => clamp(0.5 - d, 0, 1); // 1px antialias coverage

function sdRoundRect(px, py, cx, cy, hw, hh, r) {
  const qx = Math.abs(px - cx) - (hw - r);
  const qy = Math.abs(py - cy) - (hh - r);
  const ox = Math.max(qx, 0);
  const oy = Math.max(qy, 0);
  return Math.hypot(ox, oy) + Math.min(Math.max(qx, qy), 0) - r;
}

function sdSegment(px, py, ax, ay, bx, by) {
  const pax = px - ax,
    pay = py - ay,
    bax = bx - ax,
    bay = by - ay;
  const h = clamp((pax * bax + pay * bay) / (bax * bax + bay * bay), 0, 1);
  return Math.hypot(pax - bax * h, pay - bay * h);
}

function lerp(a, b, t) {
  return a + (b - a) * t;
}

const GRAD_TOP = [0xf2, 0xfa, 0xff];
const GRAD_MID = [0x9f, 0xd8, 0xff];
const GRAD_BOT = [0x35, 0x84, 0xf7];

function gradient(t) {
  const c =
    t < 0.55
      ? GRAD_TOP.map((v, i) => lerp(v, GRAD_MID[i], t / 0.55))
      : GRAD_MID.map((v, i) => lerp(v, GRAD_BOT[i], (t - 0.55) / 0.45));
  return c;
}

// Z glyph strokes (rounded caps)
const STROKES = [
  [368, 350, 656, 350], // top bar
  [368, 350, 656, 674], // diagonal
  [368, 674, 656, 674], // bottom bar
];
const STROKE_R = 44;

const rgba = Buffer.alloc(SIZE * SIZE * 4);
const cx = SIZE / 2,
  cy = SIZE / 2,
  hw = 440,
  hh = 440,
  radius = 200;

for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    const idx = (y * SIZE + x) * 4;
    const boxD = sdRoundRect(x + 0.5, y + 0.5, cx, cy, hw, hh, radius);
    const cover = smooth(boxD);
    if (cover <= 0) continue; // transparent corner
    const t = y / SIZE;
    let [r, g, b] = gradient(t);
    // glossy top highlight
    if (t < 0.34) {
      const hl = (0.34 - t) / 0.34 * 0.22;
      r = lerp(r, 255, hl);
      g = lerp(g, 255, hl);
      b = lerp(b, 255, hl);
    }
    // Z glyph in white
    let glyph = 0;
    for (const [ax, ay, bx, by] of STROKES) {
      const d = sdSegment(x + 0.5, y + 0.5, ax, ay, bx, by) - STROKE_R;
      glyph = Math.max(glyph, smooth(d));
    }
    r = lerp(r, 255, glyph);
    g = lerp(g, 255, glyph);
    b = lerp(b, 255, glyph);
    rgba[idx] = Math.round(r);
    rgba[idx + 1] = Math.round(g);
    rgba[idx + 2] = Math.round(b);
    rgba[idx + 3] = Math.round(cover * 255);
  }
}

mkdirSync(dirname(OUT), { recursive: true });
writeFileSync(OUT, encodePng(SIZE, SIZE, rgba));
console.log(`wrote ${OUT}`);
