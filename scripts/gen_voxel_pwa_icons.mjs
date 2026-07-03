#!/usr/bin/env node
// 乙太方界 PWA 圖示產生器（純程式、零外部素材、零第三方相依）。
// 用 Node 內建 zlib 手寫 PNG：畫一個星空背景 + 等角乙太水晶方塊 + 光暈。
// 產出：192/512 一般版、192/512 maskable 版（內容留安全區）、180 apple-touch、32 favicon。
//
// 執行：node scripts/gen_voxel_pwa_icons.mjs
// 產物寫到 web/voxel/icons/。

import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const OUT_DIR = join(__dirname, "..", "web", "voxel", "icons");
mkdirSync(OUT_DIR, { recursive: true });

// ── 極簡 PNG 編碼（RGBA、無濾波）───────────────────────────────
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
  const typeBuf = Buffer.from(type, "ascii");
  const body = Buffer.concat([typeBuf, data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body), 0);
  return Buffer.concat([len, body, crc]);
}
function encodePNG(width, height, rgba) {
  const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // colour type RGBA
  ihdr[10] = 0; ihdr[11] = 0; ihdr[12] = 0;
  // 每列前面加一個 filter byte(0)
  const stride = width * 4;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0;
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, y * stride + stride);
  }
  const idat = deflateSync(raw, { level: 9 });
  return Buffer.concat([
    sig,
    chunk("IHDR", ihdr),
    chunk("IDAT", idat),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ── 繪圖工具 ───────────────────────────────────────────────
function makeCanvas(size) {
  return { size, buf: Buffer.alloc(size * size * 4) };
}
function setPx(cv, x, y, r, g, b, a) {
  x = Math.round(x); y = Math.round(y);
  if (x < 0 || y < 0 || x >= cv.size || y >= cv.size) return;
  const i = (y * cv.size + x) * 4;
  // alpha over 合成
  const sa = a / 255;
  const da = cv.buf[i + 3] / 255;
  const oa = sa + da * (1 - sa);
  if (oa === 0) return;
  cv.buf[i] = Math.round((r * sa + cv.buf[i] * da * (1 - sa)) / oa);
  cv.buf[i + 1] = Math.round((g * sa + cv.buf[i + 1] * da * (1 - sa)) / oa);
  cv.buf[i + 2] = Math.round((b * sa + cv.buf[i + 2] * da * (1 - sa)) / oa);
  cv.buf[i + 3] = Math.round(oa * 255);
}
// 凸多邊形填滿：對每個像素做「在所有邊內側」測試（頂點需逆時針一致）
function fillPoly(cv, pts, r, g, b, a) {
  let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
  for (const [px, py] of pts) {
    minX = Math.min(minX, px); maxX = Math.max(maxX, px);
    minY = Math.min(minY, py); maxY = Math.max(maxY, py);
  }
  for (let y = Math.floor(minY); y <= Math.ceil(maxY); y++) {
    for (let x = Math.floor(minX); x <= Math.ceil(maxX); x++) {
      let inside = true;
      for (let k = 0; k < pts.length; k++) {
        const [ax, ay] = pts[k];
        const [bx, by] = pts[(k + 1) % pts.length];
        const cross = (bx - ax) * (y + 0.5 - ay) - (by - ay) * (x + 0.5 - ax);
        if (cross < 0) { inside = false; break; }
      }
      if (inside) setPx(cv, x, y, r, g, b, a);
    }
  }
}

// ── 主場景：星空 + 等角乙太水晶方塊 + 光暈 ─────────────────────
// cubeScale：方塊佔畫布比例。maskable 版縮小以留安全區（內容集中在中央 ~80%）。
function drawScene(size, cubeScale) {
  const cv = makeCanvas(size);
  const cx = size / 2, cy = size / 2;

  // 背景：深藍→靛的垂直漸層（蒸汽龐克太空歌劇夜空）
  for (let y = 0; y < size; y++) {
    const t = y / size;
    const r = Math.round(14 + t * 12);   // 14→26
    const g = Math.round(24 + t * 20);   // 24→44
    const b = Math.round(48 + t * 40);   // 48→88
    for (let x = 0; x < size; x++) setPx(cv, x, y, r, g, b, 255);
  }

  // 星星：以固定 PRNG 佈點，跨尺寸一致
  let seed = 20260703;
  const rnd = () => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed / 0x7fffffff; };
  const nStars = Math.round(size * 0.5);
  for (let i = 0; i < nStars; i++) {
    const x = rnd() * size, y = rnd() * size;
    const br = 140 + Math.round(rnd() * 115);
    setPx(cv, x, y, 255, 255, 255, br);
    if (rnd() > 0.85) { // 少數較亮的星加十字光芒
      setPx(cv, x + 1, y, 255, 255, 255, br * 0.5);
      setPx(cv, x - 1, y, 255, 255, 255, br * 0.5);
      setPx(cv, x, y + 1, 255, 255, 255, br * 0.5);
      setPx(cv, x, y - 1, 255, 255, 255, br * 0.5);
    }
  }

  // 中央乙太水晶光暈（放射狀青色輝光）
  const glowR = size * cubeScale * 0.95;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const d = Math.hypot(x - cx, y - cy);
      if (d < glowR) {
        const f = 1 - d / glowR;
        setPx(cv, x, y, 79, 214, 200, Math.round(70 * f * f));
      }
    }
  }

  // 等角方塊（乙太水晶）：三面——頂亮、左中、右暗
  const s = size * cubeScale * 0.5; // 半寬
  const h = s * 0.55;               // 頂面斜高
  const top = [
    [cx, cy - h - s * 0.5],
    [cx + s, cy - s * 0.5],
    [cx, cy + h - s * 0.5],
    [cx - s, cy - s * 0.5],
  ];
  const left = [
    [cx - s, cy - s * 0.5],
    [cx, cy + h - s * 0.5],
    [cx, cy + h + s * 0.9],
    [cx - s, cy - s * 0.5 + s * 1.4],
  ];
  const right = [
    [cx + s, cy - s * 0.5],
    [cx, cy + h - s * 0.5],
    [cx, cy + h + s * 0.9],
    [cx + s, cy - s * 0.5 + s * 1.4],
  ];
  fillPoly(cv, top, 122, 240, 224, 255);   // 頂面：亮青
  fillPoly(cv, left, 47, 159, 176, 255);   // 左面：中青
  fillPoly(cv, right, 31, 111, 136, 255);  // 右面：暗青
  // 頂面高光小點
  setPx(cv, cx, cy - s * 0.5, 255, 255, 255, 200);

  return cv;
}

function write(name, size, cubeScale) {
  const cv = drawScene(size, cubeScale);
  const png = encodePNG(size, size, cv.buf);
  writeFileSync(join(OUT_DIR, name), png);
  console.log(`  寫出 ${name} (${size}x${size}, ${png.length} bytes)`);
}

console.log("產生乙太方界 PWA 圖示 →", OUT_DIR);
// 一般版：方塊佔滿較大（0.62）
write("icon-192.png", 192, 0.62);
write("icon-512.png", 512, 0.62);
// maskable 版：內容縮進安全區（0.44，含光暈仍在中央 80% 內）
write("icon-maskable-192.png", 192, 0.44);
write("icon-maskable-512.png", 512, 0.44);
// Apple 主畫面圖示（iOS 不吃 manifest icons，需獨立 180 版；不透明背景）
write("apple-touch-icon.png", 180, 0.6);
// favicon
write("favicon-32.png", 32, 0.66);
console.log("完成。");
