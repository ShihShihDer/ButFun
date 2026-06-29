// ============================================================
// voxel-residents-qa.mjs — 乙太方界 AI 居民 真實瀏覽器 QA（切片③）
// ============================================================
// 用 puppeteer-core 驅動系統 Chrome，以直式手機（iPhone 12 Pro：390×844、DPR 3）載入
// /voxel/，驗證：
//   1. 非黑屏、chunk 已載入、FPS 沒崩（>15）。
//   2. 看得到 AI 居民（residentCount > 0），且有名字。
//   3. 居民「會走動」：間隔取兩次位置快照，至少一位明顯位移。
// 截圖存出來、印出居民位置/名字/說的話與 FPS。
// 不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const BASE_URL = process.env.VQA_URL || "http://127.0.0.1:3000/voxel/?debug=1";
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const OUT_DIR = process.env.VQA_OUT || join(__dirname, "out");
mkdirSync(OUT_DIR, { recursive: true });

const GPU_ARGS = [
  "--no-sandbox", "--disable-setuid-sandbox", "--ignore-gpu-blocklist",
  "--enable-gpu", "--enable-webgl", "--use-gl=angle", "--use-angle=gl",
  "--disable-dev-shm-usage",
  "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding", "--disable-features=CalculateNativeWinOcclusion",
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function injectFpsCounter(page) {
  await page.evaluateOnNewDocument(() => {
    window.__fps = { frames: 0, t0: performance.now() };
    const raf = window.requestAnimationFrame.bind(window);
    window.requestAnimationFrame = (cb) => raf((t) => { window.__fps.frames++; cb(t); });
  });
}

function isNonBlack(png) {
  const distinct = new Set(png).size;
  return { ok: png.length > 8000 && distinct > 80, pngBytes: png.length, distinct };
}

(async () => {
  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU_ARGS });
  const page = await browser.newPage();
  await page.setUserAgent(
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) " +
    "AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"
  );
  await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 3, isMobile: true, hasTouch: true });
  await injectFpsCounter(page);

  const logs = [];
  page.on("console", (m) => logs.push("[console] " + m.text()));
  page.on("pageerror", (e) => logs.push("[pageerror] " + e.message));

  console.log("載入", BASE_URL);
  await page.goto(BASE_URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await page.bringToFront();
  await sleep(6000); // 等地形 + 居民快照進來

  // 第一次居民快照。
  const snap1 = await page.evaluate(() => {
    const v = window.__voxel || {};
    return { count: v.residentCount || 0, info: (v.residentInfo ? v.residentInfo() : []) };
  });

  await sleep(4000); // 讓居民走幾秒

  const snap2 = await page.evaluate(() => {
    const v = window.__voxel || {};
    return { count: v.residentCount || 0, info: (v.residentInfo ? v.residentInfo() : []) };
  });

  // 截圖。
  const shot = join(OUT_DIR, "residents-390x844.png");
  const png = await page.screenshot({ path: shot });
  const pix = isNonBlack(png);

  const st = await page.evaluate(() => {
    const v = window.__voxel || {};
    const dt = (performance.now() - window.__fps.t0) / 1000;
    return { chunks: v.chunks || 0, meshes: v.meshes || 0, fpsAvg: window.__fps.frames / dt };
  });

  // 居民是否有走動：比對兩次快照中同名居民的位移，取最大值。
  let maxMove = 0;
  for (const a of snap1.info) {
    const b = snap2.info.find((r) => r.name === a.name);
    if (!b) continue;
    const d = Math.hypot(b.x - a.x, b.z - a.z);
    if (d > maxMove) maxMove = d;
  }
  const allNamed = snap2.info.length > 0 && snap2.info.every((r) => r.name && r.name.length > 0);

  const pass = pix.ok && st.chunks > 0 && st.fpsAvg > 15 && snap2.count > 0 && allNamed && maxMove > 0.3;

  console.log(`\nFPS: ${st.fpsAvg.toFixed(1)}  chunks: ${st.chunks}  meshes: ${st.meshes}`);
  console.log(`非黑屏: ${pix.ok ? "✓" : "✗"}  PNG ${pix.pngBytes} bytes  顏色多樣: ${pix.distinct}`);
  console.log(`居民數: ${snap2.count}  全有名字: ${allNamed ? "✓" : "✗"}`);
  console.log(`居民最大位移（4 秒）: ${maxMove.toFixed(2)} 方塊  會走動: ${maxMove > 0.3 ? "✓" : "✗"}`);
  console.log("居民快照:");
  for (const r of snap2.info) {
    console.log(`  ${r.name} @ (${r.x.toFixed(1)}, ${r.y.toFixed(1)}, ${r.z.toFixed(1)}) visible=${r.visible} say="${r.say}"`);
  }
  console.log(`截圖: ${shot}`);
  if (logs.length) console.log("頁面訊息:\n  " + logs.slice(0, 12).join("\n  "));
  console.log(`\n整體判定: ${pass ? "PASS ✅" : "FAIL ❌"}`);

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
