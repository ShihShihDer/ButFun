// ============================================================
// voxel-portrait-qa.mjs — voxel 直式手機 HUD 真實瀏覽器 QA
// ============================================================
// 用 puppeteer-core 驅動系統 Chrome，分別以直式（iPhone 12 Pro：390×844、DPR 3）
// 與橫式（844×390）載入 /voxel/，截圖驗證：
//   直式：HUD 不重疊、快捷欄排得下、按鈕在拇指區（不與快捷欄重疊）、準心置中、世界正常渲染。
//   橫式：響應式沒壞（快捷欄、搖桿、跳鈕位置正確、世界正常渲染）。
// 同時量 FPS、確認非黑屏。
// 不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
// port 3000 是本機 dev 伺服器（voxel-edit-qa.mjs 同設定）
const BASE_URL = process.env.VQA_URL || "http://127.0.0.1:3000/voxel/?debug=1";
const CHROME   = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const OUT_DIR  = process.env.VQA_OUT || join(__dirname, "out");
mkdirSync(OUT_DIR, { recursive: true });

// GPU 參數（不設 window-size，由各場景 setViewport 控制）
const GPU_ARGS = [
  "--no-sandbox", "--disable-setuid-sandbox", "--ignore-gpu-blocklist",
  "--enable-gpu", "--enable-webgl", "--use-gl=angle", "--use-angle=gl",
  "--disable-dev-shm-usage",
  "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding", "--disable-features=CalculateNativeWinOcclusion",
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// 注入 rAF FPS 計數器（需在頁面腳本前執行）
async function injectFpsCounter(page) {
  await page.evaluateOnNewDocument(() => {
    window.__fps = { frames: 0, t0: performance.now() };
    const raf = window.requestAnimationFrame.bind(window);
    window.requestAnimationFrame = (cb) => raf((t) => { window.__fps.frames++; cb(t); });
  });
}

// 讀 FPS + 遊戲狀態
async function readState(page) {
  const fpsAvg = await page.evaluate(() => {
    const dt = (performance.now() - window.__fps.t0) / 1000;
    return window.__fps.frames / dt;
  });
  const state = await page.evaluate(() => {
    const v = window.__voxel || {};
    return { chunks: v.chunks || 0, meshes: v.meshes || 0, selfFps: v.fps || 0 };
  });
  return { fpsAvg, ...state };
}

// 驗證截圖是否非黑屏（PNG 足夠大、顏色足夠多樣）
function isNonBlack(png) {
  const distinct = new Set(png).size;
  return { ok: png.length > 8000 && distinct > 80, pngBytes: png.length, distinct };
}

// 驗證指定元素在頁面上的位置與尺寸（確認沒有重疊 / 在合理位置）
async function getBoundingBox(page, selector) {
  return page.evaluate((sel) => {
    const el = document.querySelector(sel);
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { top: r.top, left: r.left, bottom: r.bottom, right: r.right, width: r.width, height: r.height };
  }, selector);
}

// 驗證兩個 bounding box 是否不重疊（任一軸無交集即可）
function noOverlap(a, b) {
  if (!a || !b) return true; // 找不到元素就跳過
  return a.right <= b.left || b.right <= a.left || a.bottom <= b.top || b.bottom <= a.top;
}

(async () => {
  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU_ARGS });
  const results = {};

  // ────────────────────────────────────────────────────────────────
  // ① 直式（iPhone 12 Pro：390×844，DPR 3，portrait）
  // ────────────────────────────────────────────────────────────────
  console.log("\n【直式 portrait 390×844 DPR3】");
  {
    const page = await browser.newPage();
    // 模擬直式手機：設定觸控裝置 user agent + viewport
    await page.setUserAgent(
      "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) " +
      "AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"
    );
    await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 3, isMobile: true, hasTouch: true });
    await injectFpsCounter(page);

    const logs = [];
    page.on("console", (m) => logs.push("[console] " + m.text()));
    page.on("pageerror", (e) => logs.push("[pageerror] " + e.message));

    console.log("  載入", BASE_URL);
    await page.goto(BASE_URL, { waitUntil: "domcontentloaded", timeout: 30000 });
    await page.bringToFront();
    await sleep(6000); // 等地形 + mesh 建好

    // 截直式截圖
    const portraitShot = join(OUT_DIR, "portrait-390x844.png");
    const portraitPng = await page.screenshot({ path: portraitShot });
    const pixPortrait = isNonBlack(portraitPng);

    // 取得各 HUD 元素位置（驗證不重疊）
    const vw = 390, vh = 844;
    const hotbarBox = await getBoundingBox(page, "#hotbar");
    const jumpBox   = await getBoundingBox(page, "#jump");
    const placeBox  = await getBoundingBox(page, "#place");
    const joyBox    = await getBoundingBox(page, "#joy");
    const crossBox  = await getBoundingBox(page, "#crosshair");
    // 玩家生存指標列（玩家生存指標 v1·溫和版）：快捷欄正上方一條窄列，納入兩兩不相交檢查。
    const statsBox  = await getBoundingBox(page, "#statsBar");

    // 驗收項目
    const hotbarFits  = hotbarBox && hotbarBox.width <= vw;                        // 快捷欄不超寬
    const jumpOk      = jumpBox && jumpBox.bottom <= vh && jumpBox.width >= 60;    // 跳鈕在螢幕內且夠大
    const placeOk     = placeBox && placeBox.bottom <= vh && placeBox.width >= 55; // 放置鈕同上
    const joyOk       = joyBox && joyBox.bottom <= vh;                            // 搖桿在螢幕內
    const jumpPlaceOk = noOverlap(jumpBox, placeBox);                             // 跳鈕與放置鈕不重疊
    const jumpHotOk   = noOverlap(jumpBox, hotbarBox);                            // 跳鈕與快捷欄不重疊
    const placeHotOk  = noOverlap(placeBox, hotbarBox);                           // 放置鈕與快捷欄不重疊
    const joyHotOk    = noOverlap(joyBox, hotbarBox);                             // 搖桿與快捷欄不重疊
    // 生存指標列：存在、在螢幕內，且與底部所有元素兩兩不相交（別弄亂 #1017 佈局）。
    const statsShown   = !!(statsBox && statsBox.width > 0 && statsBox.bottom <= vh);
    const statsHotOk   = noOverlap(statsBox, hotbarBox);                          // 指標列↔快捷欄不重疊
    const statsJumpOk  = noOverlap(statsBox, jumpBox);                            // 指標列↔跳鈕不重疊
    const statsPlaceOk = noOverlap(statsBox, placeBox);                           // 指標列↔放置鈕不重疊
    const statsJoyOk   = noOverlap(statsBox, joyBox);                             // 指標列↔搖桿不重疊
    // 準心在畫面中央（±30px 容差）
    const crossOk = crossBox &&
      Math.abs(crossBox.left + crossBox.width / 2 - vw / 2) < 30 &&
      Math.abs(crossBox.top  + crossBox.height / 2 - vh / 2) < 30;

    const st = await readState(page);
    const portraitPass = pixPortrait.ok && st.chunks > 0 && st.fpsAvg > 15 &&
      hotbarFits && jumpOk && placeOk && joyOk &&
      jumpPlaceOk && jumpHotOk && placeHotOk && joyHotOk && crossOk &&
      statsShown && statsHotOk && statsJumpOk && statsPlaceOk && statsJoyOk;

    console.log(`  FPS: ${st.fpsAvg.toFixed(1)} (自報: ${st.selfFps.toFixed(1)})  chunks: ${st.chunks}  meshes: ${st.meshes}`);
    console.log(`  非黑屏: ${pixPortrait.ok ? "✓" : "✗"}  PNG ${pixPortrait.pngBytes} bytes  顏色多樣: ${pixPortrait.distinct}`);
    console.log(`  hotbar 寬度 ${hotbarBox ? hotbarBox.width.toFixed(0) : "?"} ≤ ${vw}px: ${hotbarFits ? "✓" : "✗"}`);
    console.log(`  跳鈕在螢幕內: ${jumpOk ? "✓" : "✗"}  放置鈕在螢幕內: ${placeOk ? "✓" : "✗"}`);
    console.log(`  搖桿在螢幕內: ${joyOk ? "✓" : "✗"}`);
    console.log(`  跳鈕↔放置鈕不重疊: ${jumpPlaceOk ? "✓" : "✗"}`);
    console.log(`  跳鈕↔快捷欄不重疊: ${jumpHotOk ? "✓" : "✗"}  放置鈕↔快捷欄: ${placeHotOk ? "✓" : "✗"}  搖桿↔快捷欄: ${joyHotOk ? "✓" : "✗"}`);
    console.log(`  生存指標列出現: ${statsShown ? "✓" : "✗"}  (${statsBox ? `y=${statsBox.top.toFixed(0)}~${statsBox.bottom.toFixed(0)}, x=${statsBox.left.toFixed(0)}~${statsBox.right.toFixed(0)}` : "缺"})`);
    console.log(`  指標列↔快捷欄: ${statsHotOk ? "✓" : "✗"}  ↔跳鈕: ${statsJumpOk ? "✓" : "✗"}  ↔放置鈕: ${statsPlaceOk ? "✓" : "✗"}  ↔搖桿: ${statsJoyOk ? "✓" : "✗"}`);
    console.log(`  準心置中: ${crossOk ? "✓" : "✗"}`);
    console.log(`  截圖: ${portraitShot}`);
    if (logs.length) console.log("  頁面訊息:\n  " + logs.slice(0, 10).join("\n  "));
    console.log(`  直式判定: ${portraitPass ? "PASS ✅" : "FAIL ❌"}`);

    results.portrait = {
      pass: portraitPass, fpsAvg: st.fpsAvg, chunks: st.chunks, meshes: st.meshes,
      hotbarFits, jumpOk, placeOk, joyOk, jumpPlaceOk, jumpHotOk, placeHotOk, joyHotOk, crossOk,
      statsShown, statsHotOk, statsJumpOk, statsPlaceOk, statsJoyOk,
      screenshot: portraitShot, pixStat: pixPortrait,
    };
    await page.close();
  }

  // ────────────────────────────────────────────────────────────────
  // ② 橫式（844×390，確認響應式沒壞）
  // ────────────────────────────────────────────────────────────────
  console.log("\n【橫式 landscape 844×390 DPR2】");
  {
    const page = await browser.newPage();
    await page.setUserAgent(
      "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) " +
      "AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"
    );
    await page.setViewport({ width: 844, height: 390, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
    await injectFpsCounter(page);

    const logs = [];
    page.on("console", (m) => logs.push("[console] " + m.text()));
    page.on("pageerror", (e) => logs.push("[pageerror] " + e.message));

    console.log("  載入", BASE_URL);
    await page.goto(BASE_URL, { waitUntil: "domcontentloaded", timeout: 30000 });
    await page.bringToFront();
    await sleep(5000);

    const landscapeShot = join(OUT_DIR, "landscape-844x390.png");
    const landscapePng  = await page.screenshot({ path: landscapeShot });
    const pixLand = isNonBlack(landscapePng);

    const vw = 844, vh = 390;
    const hotbarBox = await getBoundingBox(page, "#hotbar");
    const jumpBox   = await getBoundingBox(page, "#jump");
    const placeBox  = await getBoundingBox(page, "#place");
    const joyBox    = await getBoundingBox(page, "#joy");

    const hotbarFits  = hotbarBox && hotbarBox.width <= vw;
    const jumpInView  = jumpBox && jumpBox.bottom <= vh;
    const placeInView = placeBox && placeBox.bottom <= vh;
    const joyInView   = joyBox && joyBox.bottom <= vh;

    const st = await readState(page);
    const landscapePass = pixLand.ok && st.chunks > 0 && st.fpsAvg > 15 &&
      hotbarFits && jumpInView && placeInView && joyInView;

    console.log(`  FPS: ${st.fpsAvg.toFixed(1)} (自報: ${st.selfFps.toFixed(1)})  chunks: ${st.chunks}  meshes: ${st.meshes}`);
    console.log(`  非黑屏: ${pixLand.ok ? "✓" : "✗"}  PNG ${pixLand.pngBytes} bytes`);
    console.log(`  hotbar 寬度 ${hotbarBox ? hotbarBox.width.toFixed(0) : "?"}px ≤ ${vw}px: ${hotbarFits ? "✓" : "✗"}`);
    console.log(`  跳鈕/放置鈕/搖桿在螢幕內: ${jumpInView ? "✓" : "✗"} / ${placeInView ? "✓" : "✗"} / ${joyInView ? "✓" : "✗"}`);
    console.log(`  截圖: ${landscapeShot}`);
    if (logs.length) console.log("  頁面訊息:\n  " + logs.slice(0, 10).join("\n  "));
    console.log(`  橫式判定: ${landscapePass ? "PASS ✅" : "FAIL ❌"}`);

    results.landscape = {
      pass: landscapePass, fpsAvg: st.fpsAvg, chunks: st.chunks, meshes: st.meshes,
      hotbarFits, jumpInView, placeInView, joyInView,
      screenshot: landscapeShot, pixStat: pixLand,
    };
    await page.close();
  }

  await browser.close();

  // ────────────────────────────────────────────────────────────────
  // 總結
  // ────────────────────────────────────────────────────────────────
  console.log("\n══════════════════════════════════════════");
  console.log("VOXEL 直式手機 QA 總結");
  console.log("══════════════════════════════════════════");
  console.log(`直式 portrait: ${results.portrait.pass ? "PASS ✅" : "FAIL ❌"}  FPS ${results.portrait.fpsAvg.toFixed(1)}`);
  console.log(`橫式 landscape: ${results.landscape.pass ? "PASS ✅" : "FAIL ❌"}  FPS ${results.landscape.fpsAvg.toFixed(1)}`);
  console.log(`直式截圖: ${results.portrait.screenshot}`);
  console.log(`橫式截圖: ${results.landscape.screenshot}`);

  writeFileSync(join(OUT_DIR, "voxel-portrait-qa.json"), JSON.stringify(results, null, 2));
  const allPass = results.portrait.pass && results.landscape.pass;
  console.log(`\n整體判定: ${allPass ? "PASS ✅" : "FAIL ❌"}`);
  process.exit(allPass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
