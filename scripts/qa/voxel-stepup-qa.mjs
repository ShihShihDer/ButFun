// ============================================================
// voxel-stepup-qa.mjs — 踏階平滑 QA
// ============================================================
// 真瀏覽器驅動（puppeteer），朝上坡走數秒，逐幀記錄視覺 Y（visualY = player.y - stepSmooth），
// 量出「最大單幀 visualY 上升量」——修前 ≈ 1.05（瞬跳一格），修後應遠小於 0.5 格。
// 同時確認往下/重力路徑仍平順、非黑屏、FPS 不崩。
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

  // 注入逐幀 Y 記錄器：每次 rAF callback 執行前讀 visualY（視覺 Y = 平滑後的 Y）。
  // visualY 從 window.__voxel.visualY 讀取（main.js 已暴露）。
  // 同時記錄物理 Y（player.y）對比，確認物理仍正常。
  await page.evaluateOnNewDocument(() => {
    window.__stepQA = { visualYs: [], physYs: [], fpsFrames: 0, t0: performance.now() };
    const raf = window.requestAnimationFrame.bind(window);
    window.requestAnimationFrame = (cb) => raf((t) => {
      window.__stepQA.fpsFrames++;
      const v = window.__voxel;
      if (v) {
        // visualY：踏階平滑後的視覺 Y；physY：物理 Y（player.y）
        const vy = (typeof v.visualY === "number") ? v.visualY : (v.player ? v.player.y : NaN);
        const py = v.player ? v.player.y : NaN;
        if (!isNaN(vy)) window.__stepQA.visualYs.push(vy);
        if (!isNaN(py)) window.__stepQA.physYs.push(py);
      }
      cb(t);
    });
  });

  console.log("載入", URL);
  await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 30000 });

  // 等 chunk 載入 + mesh 建好
  await sleep(6000);

  // 朝前走（持續按 W）同時向右偏（D），嘗試走上地形台階，共約 8 秒。
  await page.bringToFront();
  for (const k of ["KeyW", "KeyD"]) await page.keyboard.down(k);
  await sleep(4000);
  for (const k of ["KeyW", "KeyD"]) await page.keyboard.up(k);
  for (const k of ["KeyW", "KeyA"]) await page.keyboard.down(k);
  await sleep(4000);
  for (const k of ["KeyW", "KeyA"]) await page.keyboard.up(k);

  // 從頁面讀取記錄的 Y 序列，分析最大單幀跳變
  const result = await page.evaluate(() => {
    const qa = window.__stepQA;
    const vs = qa.visualYs;
    const ps = qa.physYs;

    // 最大單幀 visualY 上升量（踏階平滑指標——修前 ≈ 1.05，修後應遠小於 1.0）
    let maxUpVisual = 0, maxUpPhys = 0;
    // 最大單幀 visualY 下降量（重力指標——下降仍應連續、不應瞬跳）
    // 排除 > 2 格的大幅下降（spawn 傳送/落地重置屬正常瞬移，不算重力失常）
    let maxDownVisual = 0;
    let teleportEvents = 0;   // 超過 2 格的瞬跳（spawn/重置，正常行為）
    let totalUpSteps = 0; // 上升幀數（visualY 有增加的幀）

    for (let i = 1; i < vs.length; i++) {
      const dv = vs[i] - vs[i - 1];
      if (dv > 0) { if (dv > maxUpVisual) maxUpVisual = dv; totalUpSteps++; }
      if (dv < -2) { teleportEvents++; }  // spawn/重置瞬移
      else if (dv < 0) { if (-dv > maxDownVisual) maxDownVisual = -dv; }
    }
    for (let i = 1; i < ps.length; i++) {
      const dp = ps[i] - ps[i - 1];
      if (dp > 0 && dp > maxUpPhys) maxUpPhys = dp;
    }

    const dt = (performance.now() - qa.t0) / 1000;
    return {
      fpsAvg: qa.fpsFrames / dt,
      totalFrames: vs.length,
      maxUpVisual:    +maxUpVisual.toFixed(4),
      maxUpPhys:      +maxUpPhys.toFixed(4),
      maxDownVisual:  +maxDownVisual.toFixed(4),
      totalUpSteps,
      teleportEvents,
      yMin: vs.length ? +Math.min(...vs).toFixed(3) : NaN,
      yMax: vs.length ? +Math.max(...vs).toFixed(3) : NaN,
    };
  });

  // 非黑屏確認
  const shot = join(OUT_DIR, "voxel-stepup.png");
  const png = await page.screenshot({ path: shot });
  const distinctBytes = new Set(png).size;

  // 遊戲自報狀態
  const state = await page.evaluate(() => {
    const v = window.__voxel || {};
    const p = v.player || {};
    return { chunks: v.chunks, meshes: v.meshes, selfFps: v.fps, x: p.x, y: p.y };
  });

  console.log("\n──────── 踏階平滑 QA 報告 ────────");
  console.log(`FPS(rAF平均): ${result.fpsAvg.toFixed(1)}  遊戲自報: ${(state.selfFps || 0).toFixed(1)}`);
  console.log(`總幀數: ${result.totalFrames}  chunks: ${state.chunks}  meshes: ${state.meshes}`);
  console.log(`visualY 範圍: ${result.yMin} ~ ${result.yMax}`);
  console.log(`上升幀數(有踏階): ${result.totalUpSteps}  spawn/重置瞬跳事件: ${result.teleportEvents}`);
  console.log("");
  console.log("─── 關鍵指標 ───────────────────────");
  console.log(`最大單幀 visualY 上升量: ${result.maxUpVisual.toFixed(4)} 格  (修前≈1.05，修後應<0.3)`);
  console.log(`最大單幀 physY  上升量:  ${result.maxUpPhys.toFixed(4)} 格  (物理應瞬跳≈1.05，正常)`);
  console.log(`最大單幀 visualY 下降量: ${result.maxDownVisual.toFixed(4)} 格  (重力物理 < 1.5/幀，排除 spawn 瞬移)`);
  console.log(`spawn/重置瞬跳事件: ${result.teleportEvents} 次 (> 2格，正常現象，不計入重力判定)`);
  console.log(`截圖像素多樣度: ${distinctBytes} (非黑屏 > 80)`);
  if (logs.length) console.log("頁面訊息:\n" + logs.slice(0, 10).join("\n"));

  // 判定標準：
  //   1. visualY 最大單幀上升 < 0.5 格（修前≈1.05，修後顯著改善）
  //   2. visualY 最大單幀下降 < 1.6 格（物理 dy 上限 1.5；排除 spawn 瞬移後應在此範圍）
  //   3. 非黑屏（distinctBytes > 80）
  //   4. FPS > 20
  const smoothPass = result.maxUpVisual < 0.5;
  const gravityPass = result.maxDownVisual < 1.6; // 物理 dy clamp = 1.5；排除 spawn 瞬移
  const screenPass = distinctBytes > 80;
  const fpsPass = result.fpsAvg > 20;
  const pass = smoothPass && gravityPass && screenPass && fpsPass;

  console.log("\n判定:");
  console.log(`  踏階平滑: ${smoothPass ? "PASS ✅" : "FAIL ❌"}（maxUpVisual=${result.maxUpVisual.toFixed(4)} < 0.5）`);
  console.log(`  重力連續: ${gravityPass ? "PASS ✅" : "FAIL ❌"}（maxDownVisual排除瞬移=${result.maxDownVisual.toFixed(4)} < 1.6）`);
  console.log(`  非黑屏:   ${screenPass ? "PASS ✅" : "FAIL ❌"}（distinctBytes=${distinctBytes} > 80）`);
  console.log(`  FPS健康:  ${fpsPass ? "PASS ✅" : "FAIL ❌"}（fpsAvg=${result.fpsAvg.toFixed(1)} > 20）`);
  console.log(`\n總判定: ${pass ? "PASS ✅（踏階平滑、重力連續、非黑屏、FPS 健康）" : "FAIL ❌（見上方明細）"}`);

  const report = { result, state, pixStat: { distinctBytes, pngBytes: png.length }, pass };
  writeFileSync(join(OUT_DIR, "voxel-stepup-qa.json"), JSON.stringify(report, null, 2));
  console.log("截圖:", shot);

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
