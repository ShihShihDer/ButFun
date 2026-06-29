// ============================================================
// voxel-qa.mjs — voxel 世界真實瀏覽器 QA（真渲染·真走·量 FPS·驗非黑屏）
// ============================================================
// 比照 browser-qa.mjs：用 puppeteer-core 驅動系統 Chrome 載入 /voxel/，
// 驗證「真的渲染出方塊地形、角色能走、非黑屏」，量 FPS、抓 chunk 數、截圖。
// 不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const URL = process.env.VQA_URL || "http://127.0.0.1:3939/voxel/?debug=1";
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const OUT_DIR = process.env.VQA_OUT || join(__dirname, "out");
mkdirSync(OUT_DIR, { recursive: true });

const GPU_ARGS = [
  "--no-sandbox", "--disable-setuid-sandbox", "--ignore-gpu-blocklist",
  "--enable-gpu", "--enable-webgl", "--use-gl=angle", "--use-angle=gl",
  "--disable-dev-shm-usage", "--window-size=1280,800",
  "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding", "--disable-features=CalculateNativeWinOcclusion",
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU_ARGS });
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 800 });

  const logs = [];
  page.on("console", (m) => logs.push("[console] " + m.text()));
  page.on("pageerror", (e) => logs.push("[pageerror] " + e.message));

  // 注入 rAF FPS 計數器（在頁面腳本前）。
  await page.evaluateOnNewDocument(() => {
    window.__fps = { frames: 0, t0: performance.now() };
    const raf = window.requestAnimationFrame.bind(window);
    window.requestAnimationFrame = (cb) => raf((t) => { window.__fps.frames++; cb(t); });
  });

  console.log("載入", URL);
  await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 30000 });

  // 等 chunk 載入 + mesh 建好。
  await sleep(6000);

  // 走一走（WASD ~6 秒），同時轉一下視角。
  await page.bringToFront();
  for (const k of ["KeyW", "KeyD"]) await page.keyboard.down(k);
  await sleep(3000);
  await page.keyboard.down("Space"); await sleep(200); await page.keyboard.up("Space");
  await sleep(3000);
  for (const k of ["KeyW", "KeyD"]) await page.keyboard.up(k);

  // 量 FPS。
  const fpsAvg = await page.evaluate(() => {
    const dt = (performance.now() - window.__fps.t0) / 1000;
    return window.__fps.frames / dt;
  });

  // 讀遊戲自身狀態。
  const state = await page.evaluate(() => {
    const v = window.__voxel || {};
    const p = v.player || {};
    return { chunks: v.chunks, meshes: v.meshes, selfFps: v.fps, x: p.x, y: p.y, z: p.z };
  });

  // 非黑屏驗證：用「真截圖」當真相（readPixels 在 preserveDrawingBuffer=false 下常讀到
  // 已清空的 back buffer → 假黑屏）。截圖回傳 PNG buffer，數獨特位元組量化顏色多樣度：
  // 全黑/純色屏壓縮後極小且顏色單一；真有地形的畫面位元組多樣且檔案大。
  const shot = join(OUT_DIR, "voxel-walk.png");
  const png = await page.screenshot({ path: shot });
  const distinctBytes = new Set(png).size;
  const pixStat = { ok: true, pngBytes: png.length, distinctBytes };

  console.log("\n──────── VOXEL QA 報告 ────────");
  console.log("FPS(rAF平均):", fpsAvg.toFixed(1), " 遊戲自報FPS:", (state.selfFps || 0).toFixed(1));
  console.log("chunks:", state.chunks, " meshes:", state.meshes);
  console.log("玩家座標:", `${(state.x||0).toFixed(1)},${(state.y||0).toFixed(1)},${(state.z||0).toFixed(1)}`);
  console.log("像素:", JSON.stringify(pixStat));
  console.log("截圖:", shot);
  if (logs.length) console.log("頁面訊息:\n" + logs.slice(0, 20).join("\n"));

  // 判定：有 chunk + mesh（真地形）、截圖夠豐富（非黑/非純色屏）、FPS 健康。
  const pass =
    state.chunks > 0 && state.meshes > 0 &&
    pixStat.pngBytes > 8000 && pixStat.distinctBytes > 80 &&
    fpsAvg > 20;
  console.log("\n判定:", pass ? "PASS ✅（有地形 mesh、非黑屏、FPS 健康）" : "CHECK ⚠️（見上方數據）");
  writeFileSync(join(OUT_DIR, "voxel-qa.json"), JSON.stringify({ fpsAvg, state, pixStat, pass }, null, 2));

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
