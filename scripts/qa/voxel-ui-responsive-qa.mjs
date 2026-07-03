// ============================================================
// voxel-ui-responsive-qa.mjs — voxel HUD 響應式整理真實瀏覽器 QA
// ============================================================
// 用 puppeteer-core 驅動系統 Chrome，在三個視口載入 /voxel/ 並驗證：
//   ① 手機直式 390×844（DPR3, portrait）
//   ② 平板直式 768×1024（DPR2, portrait）
//   ③ 平板橫式 1024×768（DPR2, landscape）
// 每個視口驗收：
//   (a) 關鍵按鈕（☰選單/背包/搖桿/跳/放置/快捷欄/左下聊天窗）都在畫面內、彼此不重疊；
//   (b) ☰ 選單點得開，收進選單的功能（動態/日記牆/羅盤/交情/技能/成就/人稱/操作設定）點得開對應面板；
//   (c) 關鍵按鈕 tap target ≥44px（好按、防誤按）。
// 截圖存至 VQA_OUT（預設 scratchpad），供維護者眼見為憑。不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const BASE_URL = process.env.VQA_URL || "http://127.0.0.1:3939/voxel/?debug=1";
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

async function readFps(page) {
  return page.evaluate(() => {
    const dt = (performance.now() - window.__fps.t0) / 1000;
    return window.__fps.frames / dt;
  });
}

function isNonBlack(png) {
  const distinct = new Set(png).size;
  return { ok: png.length > 8000 && distinct > 80, pngBytes: png.length, distinct };
}

// 取元素的可視 bounding box（不可見或不存在回 null）
async function box(page, sel) {
  return page.evaluate((s) => {
    const el = document.querySelector(s);
    if (!el) return null;
    const r = el.getBoundingClientRect();
    const vis = r.width > 0 && r.height > 0 &&
      getComputedStyle(el).display !== "none" && getComputedStyle(el).visibility !== "hidden";
    if (!vis) return null;
    return { top: r.top, left: r.left, bottom: r.bottom, right: r.right, width: r.width, height: r.height };
  }, sel);
}

// 兩 box 是否不重疊（任一軸無交集即可）；找不到元素當作不重疊（跳過）
function noOverlap(a, b) {
  if (!a || !b) return true;
  return a.right <= b.left || b.right <= a.left || a.bottom <= b.top || b.bottom <= a.top;
}

// box 是否完全在視口內（容 1px 誤差）
function inView(b, vw, vh) {
  if (!b) return false;
  return b.left >= -1 && b.top >= -1 && b.right <= vw + 1 && b.bottom <= vh + 1;
}

// 面板是否顯示中（display !== none）
async function panelVisible(page, sel) {
  return page.evaluate((s) => {
    const el = document.querySelector(s);
    if (!el) return false;
    return getComputedStyle(el).display !== "none";
  }, sel);
}

async function clickSel(page, sel) {
  return page.evaluate((s) => {
    const el = document.querySelector(s);
    if (!el) return false;
    el.click();
    return true;
  }, sel);
}

// 收進選單的功能鈕 → 對應面板 selector
const MENU_ITEMS = [
  { btn: "#feedBtn",        panel: "#feed",            name: "動態" },
  { btn: "#diaryWallBtn",   panel: "#diaryWall",       name: "日記牆" },
  { btn: "#compassBtn",     panel: "#compassPanel",    name: "羅盤" },
  { btn: "#relationsBtn",   panel: "#relationsPanel",  name: "交情" },
  { btn: "#skillsBtn",      panel: "#skillsPanel",     name: "技能" },
  { btn: "#milestonesBtn",  panel: "#milestonesPanel", name: "成就" },
  { btn: "#gearBtn",        panel: "#settingsPanel",   name: "操作設定" },
];

async function runViewport(browser, { label, vw, vh, dpr, tag }) {
  console.log(`\n【${label} ${vw}×${vh} DPR${dpr}】`);
  const page = await browser.newPage();
  await page.setUserAgent(
    "Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) AppleWebKit/605.1.15 " +
    "(KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"
  );
  await page.setViewport({ width: vw, height: vh, deviceScaleFactor: dpr, isMobile: true, hasTouch: true });
  await injectFpsCounter(page);

  const logs = [];
  page.on("console", (m) => logs.push("[console] " + m.text()));
  page.on("pageerror", (e) => logs.push("[pageerror] " + e.message));

  await page.goto(BASE_URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await page.bringToFront();
  await sleep(6000); // 等地形 + mesh 建好

  // ── 截圖 1：常駐畫面（選單收起）──
  const shot1 = join(OUT_DIR, `${tag}-idle.png`);
  const png1 = await page.screenshot({ path: shot1 });
  const pix1 = isNonBlack(png1);

  // ── (a) 關鍵按鈕位置 / 不重疊 ──
  const menuBtn = await box(page, "#menuBtn");
  const bagBtn  = await box(page, "#bagBtn");
  const hotbar  = await box(page, "#hotbar");
  const joy     = await box(page, "#joy");
  const jump    = await box(page, "#jump");
  const place   = await box(page, "#place");
  const chatLog = await box(page, "#chatLogHead"); // 左下聊天窗標題條
  const speak   = await box(page, "#speakBar");

  const keyBtns = { menuBtn, bagBtn, hotbar, joy, jump, place };
  const allInView = Object.entries(keyBtns).every(([, b]) => inView(b, vw, vh));

  // 兩兩重疊檢查（只查該視口實際可見的關鍵互動元件）
  const pairs = [
    ["menuBtn", "bagBtn"], ["menuBtn", "hotbar"],
    ["joy", "hotbar"], ["jump", "hotbar"], ["place", "hotbar"],
    ["joy", "jump"], ["joy", "place"], ["jump", "place"],
    ["joy", "bagBtn"], ["jump", "bagBtn"],
  ];
  const overlapFails = [];
  for (const [x, y] of pairs) {
    if (keyBtns[x] && keyBtns[y] && !noOverlap(keyBtns[x], keyBtns[y])) overlapFails.push(`${x}↔${y}`);
  }
  // 左下聊天窗 vs 搖桿 / 快捷欄（重點：#1014 上線後的三方擠壓）
  if (chatLog && joy && !noOverlap(chatLog, joy)) overlapFails.push("chatLog↔joy");
  if (chatLog && hotbar && !noOverlap(chatLog, hotbar)) overlapFails.push("chatLog↔hotbar");
  const noOverlaps = overlapFails.length === 0;

  // ── (c) tap target ≥44px ──
  const tapTargets = { menuBtn, bagBtn, jump, place };
  const tapFails = [];
  for (const [k, b] of Object.entries(tapTargets)) {
    if (b && (b.width < 44 || b.height < 44)) tapFails.push(`${k}(${b.width.toFixed(0)}×${b.height.toFixed(0)})`);
  }
  const tapOk = tapFails.length === 0;

  // ── (b) 開 ☰ 選單，逐一驗證收進選單的功能點得開 ──
  await clickSel(page, "#menuBtn");
  await sleep(350);
  const drawerBox = await box(page, "#menuDrawer");
  const drawerOpen = !!drawerBox;
  const drawerInView = inView(drawerBox, vw, vh);

  // 截圖 2：選單展開
  const shot2 = join(OUT_DIR, `${tag}-menu-open.png`);
  await page.screenshot({ path: shot2 });

  // 抽屜內每個項目 tap target ≥44px
  const itemTapFails = [];
  for (const it of MENU_ITEMS) {
    const b = await box(page, it.btn);
    if (b && (b.height < 44)) itemTapFails.push(`${it.name}(h${b.height.toFixed(0)})`);
  }
  // 人稱鈕
  {
    const b = await box(page, "#viewBtn");
    if (b && b.height < 44) itemTapFails.push(`人稱(h${b.height.toFixed(0)})`);
  }

  // 逐一點開 → 驗面板顯示 → 關面板 → 重開選單
  const openFails = [];
  for (const it of MENU_ITEMS) {
    await clickSel(page, "#menuBtn"); // 確保選單開著（上一輪點項目會收起選單）
    await sleep(150);
    await clickSel(page, it.btn);
    await sleep(300);
    const vis = await panelVisible(page, it.panel);
    if (!vis) openFails.push(it.name);
    // 關面板：面板多半點自身背景遮罩或 Esc 可關；統一用 Esc + 點各自 close
    await page.keyboard.press("Escape");
    await sleep(120);
    // 保險：若還開著，直接把它藏起以免擋下一輪（不影響驗收結果，只清場）
    await page.evaluate((s) => { const e = document.querySelector(s); if (e && getComputedStyle(e).display !== "none") e.style.display = "none"; }, it.panel);
    await sleep(60);
  }
  // 人稱切換：點了不開面板，但應能觸發（不報錯即可）
  await clickSel(page, "#menuBtn"); await sleep(120);
  const viewToggled = await page.evaluate(() => {
    const el = document.querySelector("#viewBtn");
    if (!el) return false;
    const before = el.textContent;
    el.click();
    return el.textContent !== before; // 文字在第一/三人稱間切換
  });

  const fps = await readFps(page);
  const menuOk = drawerOpen && drawerInView && openFails.length === 0 && viewToggled;

  const pass = pix1.ok && allInView && noOverlaps && tapOk && menuOk &&
    itemTapFails.length === 0 && fps > 20;

  console.log(`  FPS: ${fps.toFixed(1)}  非黑屏: ${pix1.ok ? "✓" : "✗"} (${pix1.pngBytes}B, 色${pix1.distinct})`);
  console.log(`  關鍵按鈕都在畫面內: ${allInView ? "✓" : "✗"}`);
  console.log(`  無重疊: ${noOverlaps ? "✓" : "✗ " + overlapFails.join(",")}`);
  console.log(`  tap target ≥44px: ${tapOk ? "✓" : "✗ " + tapFails.join(",")}`);
  console.log(`  ☰ 選單開得起來/在畫面內: ${drawerOpen ? "✓" : "✗"}/${drawerInView ? "✓" : "✗"}`);
  console.log(`  選單項目都點得開面板: ${openFails.length === 0 ? "✓" : "✗ 打不開:" + openFails.join(",")}`);
  console.log(`  選單項目 tap≥44px: ${itemTapFails.length === 0 ? "✓" : "✗ " + itemTapFails.join(",")}`);
  console.log(`  人稱切換可用: ${viewToggled ? "✓" : "✗"}`);
  console.log(`  截圖: ${shot1}`);
  console.log(`         ${shot2}`);
  if (logs.length) console.log("  頁面訊息:\n  " + logs.slice(0, 8).join("\n  "));
  console.log(`  判定: ${pass ? "PASS ✅" : "FAIL ❌"}`);

  await page.close();
  return {
    label, pass, fps, allInView, noOverlaps, overlapFails, tapOk, tapFails,
    drawerOpen, drawerInView, openFails, itemTapFails, viewToggled,
    screenshots: [shot1, shot2],
    boxes: { menuBtn, bagBtn, hotbar, joy, jump, place, chatLog, speak, drawer: drawerBox },
  };
}

(async () => {
  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU_ARGS });
  const results = {};
  results.phone   = await runViewport(browser, { label: "手機直式", vw: 390, vh: 844, dpr: 3, tag: "phone-390x844" });
  results.tabletP = await runViewport(browser, { label: "平板直式", vw: 768, vh: 1024, dpr: 2, tag: "tablet-portrait-768x1024" });
  results.tabletL = await runViewport(browser, { label: "平板橫式", vw: 1024, vh: 768, dpr: 2, tag: "tablet-landscape-1024x768" });
  await browser.close();

  console.log("\n══════════════════════════════════════════");
  console.log("VOXEL UI 響應式整理 QA 總結");
  console.log("══════════════════════════════════════════");
  for (const [k, r] of Object.entries(results)) {
    console.log(`${r.label}: ${r.pass ? "PASS ✅" : "FAIL ❌"}  FPS ${r.fps.toFixed(1)}`);
  }
  writeFileSync(join(OUT_DIR, "voxel-ui-responsive-qa.json"), JSON.stringify(results, null, 2));
  const allPass = Object.values(results).every((r) => r.pass);
  console.log(`\n整體判定: ${allPass ? "PASS ✅" : "FAIL ❌"}`);
  console.log(`截圖目錄: ${OUT_DIR}`);
  process.exit(allPass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
