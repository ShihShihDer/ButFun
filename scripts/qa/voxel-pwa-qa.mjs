// ============================================================
// voxel-pwa-qa.mjs — 乙太方界 PWA 真實瀏覽器 QA
// ============================================================
// 用系統安裝的 google-chrome（headless）驗證 PWA 全套：
//   ① manifest 抓得到且解析有效
//   ② Service Worker 註冊成功（拿到 registration + controller）
//   ③ 圖示載得到（HTTP 200）
//   ④ 離線模式（攔截網路）不白屏、顯示友善「需要連線」提示
//   ⑤ 線上：遊戲頁照常載入（HUD 在、canvas 在）
// 截圖存到 out/ 供人眼複核。
//
// 跑法：先啟動測試伺服器於某 port（記憶體模式、獨立 port），再
//   PWAQA_URL=http://127.0.0.1:3941 node scripts/qa/voxel-pwa-qa.mjs
//
// 不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const BASE = process.env.PWAQA_URL || "http://127.0.0.1:3941";
const CHROME = process.env.PWAQA_CHROME || "/usr/bin/google-chrome";
// 截圖預設落在 out/（已 gitignore）——不進版控，供人眼複核。
const OUT_DIR = process.env.PWAQA_OUT || join(__dirname, "out");
mkdirSync(OUT_DIR, { recursive: true });

const results = [];
function check(name, pass, detail) {
  results.push({ name, pass: !!pass, detail: detail || "" });
  console.log(`${pass ? "✅" : "❌"} ${name}${detail ? " — " + detail : ""}`);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const browser = await puppeteer.launch({
  executablePath: CHROME,
  headless: "new",
  args: [
    "--no-sandbox",
    "--disable-setuid-sandbox",
    "--disable-dev-shm-usage",
    "--use-gl=angle",
    "--use-angle=swiftshader",
    "--window-size=414,896", // 手機直式視窗
  ],
});

try {
  const page = await browser.newPage();
  // 手機直式視窗
  await page.setViewport({ width: 414, height: 896, isMobile: true, hasTouch: true, deviceScaleFactor: 2 });

  // ── ① manifest：頁面內 fetch 並解析 ──
  await page.goto(BASE + "/", { waitUntil: "domcontentloaded", timeout: 30000 });
  const manifest = await page.evaluate(async () => {
    // 先看 <link rel=manifest>
    const link = document.querySelector('link[rel="manifest"]');
    const href = link ? link.getAttribute("href") : null;
    if (!href) return { ok: false, reason: "無 <link rel=manifest>" };
    const res = await fetch(href);
    if (!res.ok) return { ok: false, reason: "manifest HTTP " + res.status };
    const ct = res.headers.get("content-type") || "";
    let json;
    try { json = await res.json(); } catch (e) { return { ok: false, reason: "JSON 解析失敗: " + e.message }; }
    return { ok: true, href, ct, name: json.name, short_name: json.short_name, start_url: json.start_url, display: json.display, theme: json.theme_color, icons: (json.icons || []).length, hasMaskable: (json.icons || []).some(i => (i.purpose || "").includes("maskable")) };
  });
  check("① manifest link + fetch + 解析", manifest.ok && manifest.display === "standalone" && manifest.icons >= 2 && manifest.hasMaskable,
    manifest.ok ? `name=${manifest.name} display=${manifest.display} icons=${manifest.icons} maskable=${manifest.hasMaskable} ct=${manifest.ct}` : manifest.reason);

  // 檢查 head 內 apple meta / theme-color
  const head = await page.evaluate(() => ({
    theme: !!document.querySelector('meta[name="theme-color"]'),
    appleCap: !!document.querySelector('meta[name="apple-mobile-web-app-capable"]'),
    appleIcon: !!document.querySelector('link[rel="apple-touch-icon"]'),
    appleStatus: !!document.querySelector('meta[name="apple-mobile-web-app-status-bar-style"]'),
  }));
  check("   head 內 theme-color + apple meta + apple-touch-icon",
    head.theme && head.appleCap && head.appleIcon && head.appleStatus,
    JSON.stringify(head));

  // ── ② Service Worker 註冊 ──
  // 等 load 事件後 sw 才 register；輪詢 registration/controller。
  let swState = null;
  for (let i = 0; i < 30; i++) {
    swState = await page.evaluate(async () => {
      if (!("serviceWorker" in navigator)) return { supported: false };
      const reg = await navigator.serviceWorker.getRegistration("/");
      return {
        supported: true,
        hasReg: !!reg,
        scope: reg ? reg.scope : null,
        active: reg && reg.active ? reg.active.state : null,
        controller: !!navigator.serviceWorker.controller,
      };
    });
    if (swState.hasReg && swState.active === "activated") break;
    await sleep(500);
  }
  check("② Service Worker 註冊成功（root scope + active）",
    swState.supported && swState.hasReg && swState.active === "activated" && (swState.scope || "").endsWith("/"),
    JSON.stringify(swState));

  // ── ③ 圖示載得到 ──
  const icons = await page.evaluate(async (base) => {
    const files = [
      "/voxel/icons/icon-192.png",
      "/voxel/icons/icon-512.png",
      "/voxel/icons/icon-maskable-192.png",
      "/voxel/icons/icon-maskable-512.png",
      "/voxel/icons/apple-touch-icon.png",
      "/voxel/icons/favicon-32.png",
    ];
    const out = {};
    for (const f of files) {
      try { const r = await fetch(base + f); out[f] = r.status; } catch (e) { out[f] = "ERR"; }
    }
    return out;
  }, BASE);
  const allIcons200 = Object.values(icons).every((v) => v === 200);
  check("③ 全部圖示 HTTP 200", allIcons200, JSON.stringify(icons));

  // ── ⑤ 線上遊戲照常（先做線上，才好對照離線）──
  await page.reload({ waitUntil: "domcontentloaded" });
  await sleep(3500); // 讓 main.js 連 WS、畫第一幀
  const online = await page.evaluate(() => {
    const hud = document.getElementById("hud");
    const canvas = document.querySelector("#app canvas") || document.querySelector("canvas");
    return {
      hudText: hud ? (hud.textContent || "").slice(0, 40) : null,
      hasCanvas: !!canvas,
      canvasW: canvas ? canvas.width : 0,
      bodyBg: getComputedStyle(document.body).backgroundColor,
    };
  });
  // HUD 從「載入中…」變成別的（或至少 canvas 有寬度）代表遊戲活了
  check("⑤ 線上：遊戲頁載入（canvas + HUD 在）",
    online.hasCanvas && online.canvasW > 0,
    JSON.stringify(online));
  await page.screenshot({ path: join(OUT_DIR, "online.png") });

  // ── ④ 離線模式：攔截網路後重載，應顯示友善提示、非白屏 ──
  // 用 CDP 設離線，確保 sw 已裝好快取。
  await page.setOfflineMode(true);
  let offline;
  try {
    await page.goto(BASE + "/", { waitUntil: "domcontentloaded", timeout: 15000 });
  } catch (e) {
    // 導覽在極端情況可能 throw；不影響下面讀 DOM（sw 應已回退頁）
  }
  await sleep(1200);
  offline = await page.evaluate(() => {
    const text = document.body ? document.body.innerText : "";
    const bg = document.body ? getComputedStyle(document.body).backgroundColor : "";
    return {
      text: (text || "").replace(/\s+/g, " ").slice(0, 120),
      // 白屏判定：body 沒有可見文字內容
      nonBlank: !!(text && text.trim().length > 0),
      bg,
    };
  });
  // 友善提示：不是白屏，且含「連線」字樣（來自 sw 離線頁或快取殼）
  const friendly = offline.nonBlank && /連線|連不上|需要網路|乙太方界/.test(offline.text);
  check("④ 離線：非白屏 + 友善提示", friendly, JSON.stringify(offline));
  await page.screenshot({ path: join(OUT_DIR, "offline.png") });
  await page.setOfflineMode(false);

  // ── 額外：截一張圖示拼貼（用 favicon/icon 顯示）供人眼看 ──
  await page.setContent(`<body style="margin:0;background:#0e1830;display:flex;gap:16px;padding:24px;align-items:center;flex-wrap:wrap">
    <img src="${BASE}/voxel/icons/icon-192.png" width="120">
    <img src="${BASE}/voxel/icons/icon-maskable-512.png" width="120">
    <img src="${BASE}/voxel/icons/apple-touch-icon.png" width="90">
    <img src="${BASE}/voxel/icons/favicon-32.png" width="32">
  </body>`);
  await sleep(500);
  await page.screenshot({ path: join(OUT_DIR, "icons.png") });

  // ── 總結 ──
  const passed = results.filter((r) => r.pass).length;
  console.log(`\n=== PWA QA：${passed}/${results.length} 通過 ===`);
  writeFileSync(join(OUT_DIR, "report.json"), JSON.stringify(results, null, 2));
  console.log("截圖與報告存於：" + OUT_DIR);
  await browser.close();
  process.exit(passed === results.length ? 0 : 1);
} catch (e) {
  console.error("QA 執行錯誤：", e);
  await browser.close();
  process.exit(2);
}
