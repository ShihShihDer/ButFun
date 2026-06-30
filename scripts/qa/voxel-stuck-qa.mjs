// ============================================================
// voxel-stuck-qa.mjs — 乙太方界「玩家卡地裡」脫困 + Google 登入入口 真實瀏覽器 QA
// ============================================================
// 用 puppeteer-core 驅動系統 Chrome，以直式（iPhone 12 Pro：390×844、DPR 3）載入 /voxel/，驗證：
//   ① 脫困純函式（aabbHitsSolid / unstuckY）餵假地形行為正確、收斂穩定（不上彈）。
//   ② 真地形脫困：把玩家「埋進」地表下 → unstuck 後不再與實心重疊（修卡地裡核心）。
//   ③ 走動數秒（含靠近 chunk 邊緣）後，玩家始終沒被埋住（overlapping 抽樣全 false）。
//   ④ 登入入口（右上角 #acct chip）存在、訪客顯示「🔑 登入」、非黑屏、FPS 正常。
// 同時截直式截圖。不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const BASE_URL = process.env.VQA_URL || "http://127.0.0.1:3000/voxel/?debug=1";
const CHROME   = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const OUT_DIR  = process.env.VQA_OUT || join(__dirname, "out");
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
  const results = {};
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
  await sleep(6000); // 等地形 + mesh 建好、出生點落穩

  // ── ① 脫困純函式（假地形）──────────────────────────────────────────────────
  const pure = await page.evaluate(() => {
    const v = window.__voxel;
    const solid = (bx, by, bz) => by < 10; // y<10 全實心，其餘空氣
    const hit = v.aabbHitsSolid(5.5, 8, 5.5, solid);    // 腳底 8 在實心 → true
    const clear = v.aabbHitsSolid(5.5, 10, 5.5, solid); // 腳底 10 在空氣 → false
    const ny = v.unstuckY(5.5, 5, 5.5, solid);          // 卡在 5 → 上抬到 10
    const stable = v.unstuckY(5.5, ny, 5.5, solid);     // 再跑一次不動（收斂）
    const noStick = v.unstuckY(5.5, 12, 5.5, solid);    // 沒卡 → 原值 12 返回
    return { hit, clear, ny, stable, converged: stable === ny, noStick };
  });
  const purePass = pure.hit === true && pure.clear === false && pure.ny === 10 &&
    pure.converged && pure.noStick === 12;
  console.log(`① 純函式：hit=${pure.hit} clear=${pure.clear} unstuckY→${pure.ny} 收斂=${pure.converged} 沒卡不動=${pure.noStick === 12} → ${purePass ? "PASS ✅" : "FAIL ❌"}`);
  results.pure = { ...pure, pass: purePass };

  // ── ② 真地形脫困：把玩家埋進地表下，驗證 unstuck 後不再重疊 ───────────────────
  const burial = await page.evaluate(() => {
    const v = window.__voxel;
    const px = v.player.x, pz = v.player.z;
    // 找該 (x,z) 的地表（從高往低第一個實心非水格）。
    let surf = null;
    for (let y = 45; y >= 0; y--) {
      const b = v.getBlock(Math.floor(px), y, Math.floor(pz));
      if (b > 0 && b !== 7) { surf = y; break; }
    }
    if (surf == null) return { ok: false, reason: "找不到地表" };
    // 把玩家腳底壓到地表下 2 格（AABB 必與實心重疊＝模擬出生/chunk 載入被埋）。
    v.setPlayerPos(px, surf - 2, pz);
    const before = v.overlapping;          // 應為 true（被埋）
    const moved = v.unstuckNow();           // 脫困一次
    const after = v.overlapping;            // 應為 false（已頂出）
    return { ok: before && !after, before, after, moved, surf, yAfter: v.player.y };
  });
  const burialPass = burial.ok === true && burial.before === true && burial.after === false;
  console.log(`② 真地形脫困：埋住前 overlapping=${burial.before} → 脫困後 overlapping=${burial.after}（地表 y=${burial.surf}，脫困後 y=${burial.yAfter?.toFixed?.(2)}）→ ${burialPass ? "PASS ✅" : "FAIL ❌"}`);
  results.burial = { ...burial, pass: burialPass };

  // ── ③ 走動數秒，抽樣玩家是否曾被埋住（含靠近 chunk 邊緣）────────────────────
  // 連續按方向鍵走動，期間每 200ms 抽樣一次 overlapping；脫困每幀作用 → 抽樣應全 false。
  let stuckSamples = 0, totalSamples = 0;
  const dirs = ["KeyW", "KeyD", "KeyS", "KeyA"];
  for (let d = 0; d < dirs.length; d++) {
    await page.keyboard.down(dirs[d]);
    for (let i = 0; i < 8; i++) {
      await sleep(200);
      const o = await page.evaluate(() => window.__voxel.overlapping);
      totalSamples++;
      if (o) stuckSamples++;
    }
    await page.keyboard.up(dirs[d]);
  }
  const walkPass = stuckSamples === 0;
  console.log(`③ 走動 ${totalSamples} 次抽樣：被埋住 ${stuckSamples} 次 → ${walkPass ? "PASS ✅" : "FAIL ❌"}`);
  results.walk = { stuckSamples, totalSamples, pass: walkPass };

  // ── ④ 登入入口 + 非黑屏 + FPS ──────────────────────────────────────────────
  const acct = await page.evaluate(() => {
    const el = document.getElementById("acct");
    const btn = el && el.querySelector("button");
    const v = window.__voxel || {};
    return {
      present: !!el,
      btnText: btn ? btn.textContent : null,
      isLoggedIn: !!v.isLoggedIn,
    };
  });
  const acctPass = acct.present && typeof acct.btnText === "string" && acct.btnText.includes("登入");
  console.log(`④ 登入 chip：存在=${acct.present} 文字="${acct.btnText}" 登入態=${acct.isLoggedIn} → ${acctPass ? "PASS ✅" : "FAIL ❌"}`);
  results.acct = { ...acct, pass: acctPass };

  const shot = join(OUT_DIR, "voxel-stuck-login-390x844.png");
  const png = await page.screenshot({ path: shot });
  const pix = isNonBlack(png);
  const fpsAvg = await page.evaluate(() => {
    const dt = (performance.now() - window.__fps.t0) / 1000;
    return window.__fps.frames / dt;
  });
  const st = await page.evaluate(() => {
    const v = window.__voxel || {};
    return { chunks: v.chunks || 0, selfFps: v.fps || 0, grounded: v.player ? v.player.grounded : null };
  });
  const renderPass = pix.ok && st.chunks > 0 && fpsAvg > 15;
  console.log(`渲染：非黑屏=${pix.ok}（${pix.pngBytes}B）chunks=${st.chunks} FPS=${fpsAvg.toFixed(1)}（自報 ${st.selfFps.toFixed(1)}）grounded=${st.grounded} → ${renderPass ? "PASS ✅" : "FAIL ❌"}`);
  console.log(`截圖：${shot}`);
  results.render = { pix, fpsAvg, ...st, pass: renderPass, screenshot: shot };

  if (logs.length) console.log("頁面訊息:\n  " + logs.slice(0, 12).join("\n  "));

  await browser.close();

  const allPass = purePass && burialPass && walkPass && acctPass && renderPass;
  console.log("\n══════════════════════════════════════════");
  console.log(`乙太方界 脫困 + 登入 QA 總結：${allPass ? "PASS ✅" : "FAIL ❌"}`);
  console.log("══════════════════════════════════════════");
  writeFileSync(join(OUT_DIR, "voxel-stuck-qa.json"), JSON.stringify(results, null, 2));
  process.exit(allPass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
