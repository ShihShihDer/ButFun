// ============================================================
// voxel-flower-cross-qa.mjs — 裝飾植物十字貼片渲染 v1 前後對比 QA
// ============================================================
// 維護者玩到把一顆「藍色方塊」納悶成積木、打掉才發現是小花——本刀給裝飾類植物
// 換上十字貼片（cross-billboard）渲染。此 QA 用系統 Chrome 載入 /voxel/，就地擺出
// 三色野花（+樹苗/莓果叢）在玩家面前的地面，視角轉過去，分別截：
//   flower-before.png = 走舊的整格立方體（__qaCubePlants=on）——藍花是方塊
//   flower-after.png  = 走新的十字小花（__qaCubePlants=off）——一眼是插在地上的花
// 同時量 FPS、確認非黑屏。不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const URL = process.env.VQA_URL || "http://127.0.0.1:3000/voxel/?debug=1";
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

  await page.evaluateOnNewDocument(() => {
    window.__fps = { frames: 0, t0: performance.now() };
    const raf = window.requestAnimationFrame.bind(window);
    window.requestAnimationFrame = (cb) => raf((t) => { window.__fps.frames++; cb(t); });
  });

  console.log("載入", URL);
  await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await page.bringToFront();
  await sleep(6000); // 等 chunk + mesh 建好

  // 在玩家面前一排地面擺出：紅花、黃花、藍花、樹苗、莓果叢。
  // 先從玩家腳下往前掃出一段實心地面，把花插在地表方塊「上方」那一格。
  const placed = await page.evaluate(() => {
    const v = window.__voxel;
    const p = v.player;
    const RED = v.WILDFLOWER_RED, YEL = v.WILDFLOWER_YELLOW, BLU = v.WILDFLOWER_BLUE, BUSH = v.BERRY_BUSH;
    const SAP = 65;
    const kinds = [RED, YEL, BLU, SAP, BUSH];
    // 找玩家前方（-Z 方向，yaw=0 大致朝 -Z）一段地面。逐格往前找「上方是空氣、下方是實心」的地表。
    const px = Math.floor(p.x), pz = Math.floor(p.z), py = Math.floor(p.y);
    const spots = [];
    for (let d = 3; d < 30 && spots.length < kinds.length; d++) {
      const wz = pz - d;
      // 在該 (x,z) 縱向找地表：從玩家頭高往下掃第一個實心方塊，其上一格即地表花位。
      for (let wy = py + 3; wy > py - 6; wy--) {
        const here = v.getBlock(px, wy, wz);
        const below = v.getBlock(px, wy - 1, wz);
        if (here === 0 && below > 0) { // 空氣 + 下方實心 = 地表
          spots.push({ x: px, y: wy, z: wz });
          break;
        }
      }
    }
    const done = [];
    for (let i = 0; i < spots.length && i < kinds.length; i++) {
      const s = spots[i];
      const got = v._qaSetBlock(s.x, s.y, s.z, kinds[i]);
      done.push({ ...s, want: kinds[i], got });
    }
    // 視角轉向這排花的中點。
    if (spots.length) {
      const mid = spots[Math.floor(spots.length / 2)];
      v.lookTowards(mid.x + 0.5, mid.y + 0.3, mid.z + 0.5);
    }
    return { done, playerY: p.y };
  });
  console.log("擺花:", JSON.stringify(placed.done));
  if (!placed.done.length) { console.error("QA 失敗：找不到地表擺花（地形沒載入？）"); await browser.close(); process.exit(1); }
  await sleep(1200);

  // ── 改前：切回整格立方體 ──
  await page.evaluate(() => window.__voxel._qaSetCubePlants(true));
  await sleep(1500);
  const beforePng = await page.screenshot({ path: join(OUT_DIR, "flower-before.png") });

  // ── 改後：十字小花 ──
  await page.evaluate(() => window.__voxel._qaSetCubePlants(false));
  await sleep(1500);
  const afterPng = await page.screenshot({ path: join(OUT_DIR, "flower-after.png") });

  // 各花單獨特寫：轉向每一朵、各截一張（三色野花驗收核心）。
  const shots = [];
  for (const d of placed.done) {
    await page.evaluate(([x, y, z]) => window.__voxel.lookTowards(x + 0.5, y + 0.35, z + 0.5), [d.x, d.y, d.z]);
    await sleep(500);
    const name = `flower-after-${d.want}.png`;
    await page.screenshot({ path: join(OUT_DIR, name) });
    shots.push(name);
  }

  const fpsAvg = await page.evaluate(() => {
    const dt = (performance.now() - window.__fps.t0) / 1000;
    return window.__fps.frames / dt;
  });
  const state = await page.evaluate(() => {
    const v = window.__voxel || {};
    return { chunks: v.chunks, meshes: v.meshes, selfFps: v.fps };
  });
  const distinctBefore = new Set(beforePng).size, distinctAfter = new Set(afterPng).size;

  console.log("\n──────── 花朵十字貼片 QA 報告 ────────");
  console.log("FPS(rAF平均):", fpsAvg.toFixed(1), " 自報:", (state.selfFps || 0).toFixed(1));
  console.log("chunks:", state.chunks, " meshes:", state.meshes);
  console.log("截圖目錄:", OUT_DIR);
  console.log("  改前(方塊):", "flower-before.png", " 改後(十字):", "flower-after.png");
  console.log("  單花特寫:", shots.join(", "));
  if (logs.length) console.log("頁面訊息:\n" + logs.slice(0, 15).join("\n"));

  const allSet = placed.done.every((d) => d.got === d.want);
  const pass = allSet && state.chunks > 0 && state.meshes > 0 &&
    beforePng.length > 8000 && afterPng.length > 8000 &&
    distinctBefore > 80 && distinctAfter > 80 && fpsAvg > 20;
  console.log("\n判定:", pass ? "PASS ✅（花草就地擺出、前後皆非黑屏、FPS 健康）" : "CHECK ⚠️（見上方數據）");
  writeFileSync(join(OUT_DIR, "voxel-flower-cross-qa.json"),
    JSON.stringify({ fpsAvg, state, placed: placed.done, shots, pass }, null, 2));

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
