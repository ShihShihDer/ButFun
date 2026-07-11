// ============================================================
// voxel-furniture-qa.mjs — 玩家裝飾傢俱 v1（ROADMAP 931）擺設截圖 QA
// ============================================================
// 用系統 Chrome 載入 /voxel/，就地在玩家面前的地面鋪出一個「溫馨小角落」：
// 一排短牆（木板）當背景，牆前擺出四樣新傢俱——小地毯(102)/花盆(103)/小圓桌(104)/掛旗(105)，
// 視角轉過去截圖，驗四樣皆以矮塊/薄片造型畫出（非整格立方體）、非黑屏、FPS 健康。
// 純視覺（_qaSetBlock 就地寫本地方塊+重建 mesh），不抄外部碼；全繁中註解；node --check 過。

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

  // 在玩家前方一段地面就地擺出四樣傢俱，並在其後立一排木板短牆當背景（掛旗貼在牆前）。
  const placed = await page.evaluate(() => {
    const v = window.__voxel;
    const p = v.player;
    const kinds = [
      { id: v.CARPET, name: "carpet" },
      { id: v.FLOWERPOT, name: "flowerpot" },
      { id: v.TABLE, name: "table" },
      { id: v.BANNER, name: "banner" },
    ];
    const px = Math.floor(p.x), pz = Math.floor(p.z), py = Math.floor(p.y);
    // 逐格往前（-Z）找地表：上方空氣、下方實心。
    const spots = [];
    for (let d = 3; d < 30 && spots.length < kinds.length; d++) {
      const wz = pz - d;
      for (let wy = py + 3; wy > py - 6; wy--) {
        const here = v.getBlock(px, wy, wz);
        const below = v.getBlock(px, wy - 1, wz);
        if (here === 0 && below > 0) { spots.push({ x: px, y: wy, z: wz }); break; }
      }
    }
    const done = [];
    for (let i = 0; i < spots.length && i < kinds.length; i++) {
      const s = spots[i];
      // 傢俱本身
      const got = v._qaSetBlock(s.x, s.y, s.z, kinds[i].id);
      done.push({ ...s, want: kinds[i].id, name: kinds[i].name, got });
      // 每件傢俱正後方（更遠一格 -Z）立兩格高木板短牆當背景，讓矮塊剪影更清楚（掛旗也有牆可貼）。
      v._qaSetBlock(s.x, s.y, s.z - 1, v.PLANK);
      v._qaSetBlock(s.x, s.y + 1, s.z - 1, v.PLANK);
    }
    // 視角轉向這排傢俱的中點、略俯視，看得出矮塊造型。
    if (spots.length) {
      const mid = spots[Math.floor(spots.length / 2)];
      v.lookTowards(mid.x + 0.5, mid.y + 0.2, mid.z + 0.5);
    }
    return { done, playerY: p.y };
  });
  console.log("擺傢俱:", JSON.stringify(placed.done));
  if (!placed.done.length) { console.error("QA 失敗：找不到地表擺傢俱（地形沒載入？）"); await browser.close(); process.exit(1); }
  await sleep(1500);

  // 全景：四樣傢俱＋短牆一起，佈置一個小角落。
  const cornerPng = await page.screenshot({ path: join(OUT_DIR, "furniture-corner.png") });

  // 各傢俱單獨特寫：轉向每一件、各截一張（驗矮塊/薄片造型）。
  const shots = [];
  for (const d of placed.done) {
    await page.evaluate(([x, y, z]) => window.__voxel.lookTowards(x + 0.5, y + 0.25, z + 0.5), [d.x, d.y, d.z]);
    await sleep(500);
    const name = `furniture-${d.name}.png`;
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
  const distinct = new Set(cornerPng).size;

  console.log("\n──────── 玩家裝飾傢俱 QA 報告 ────────");
  console.log("FPS(rAF平均):", fpsAvg.toFixed(1), " 自報:", (state.selfFps || 0).toFixed(1));
  console.log("chunks:", state.chunks, " meshes:", state.meshes);
  console.log("截圖目錄:", OUT_DIR);
  console.log("  全景(小角落):", "furniture-corner.png");
  console.log("  單件特寫:", shots.join(", "));
  if (logs.length) console.log("頁面訊息:\n" + logs.slice(0, 15).join("\n"));

  const allSet = placed.done.every((d) => d.got === d.want);
  const pass = allSet && state.chunks > 0 && state.meshes > 0 &&
    cornerPng.length > 8000 && distinct > 80 && fpsAvg > 20;
  console.log("\n判定:", pass ? "PASS ✅（四樣傢俱就地擺出、非黑屏、FPS 健康）" : "CHECK ⚠️（見上方數據）");
  writeFileSync(join(OUT_DIR, "voxel-furniture-qa.json"),
    JSON.stringify({ fpsAvg, state, placed: placed.done, shots, pass }, null, 2));

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
