// ============================================================
// voxel-ui-bottom-overlap-qa.mjs — 乙太方界「底部 UI 全元素不相交」真實瀏覽器 QA
// ============================================================
// 背景：#1015 的 voxel-ui-responsive-qa.mjs 抓了 #speakBar 的 box 卻沒放進兩兩檢查，
//       導致底部搖桿/輸入列/挖放/快捷欄互疊卻誤報 PASS（維護者實測抓到）。
//       #1094 的 statsBar（❤️🍗）又被桌機常駐 speakBar 壓住只露一條縫——因為當時只測手機直式
//       （speakBar 預設隱藏），沒測桌機。底部反覆疊圖的根因＝底部元素全各自絕對定位。
// 根治：底部中央三件（快捷欄／指標／輸入列）改由 #bottomStack flex 建構保證永不重疊。
// 本 QA 補足並擴成四視口（這次含桌機！），把「底部所有互動元素」兩兩 bounding rect 全部檢查、
//       逐對列印結果，含 statsBar。每視口各截「常駐態＋輸入列顯示態＋聊天窗展開態」供眼見為憑：
//   ① 桌機     1280×800 （DPR1, landscape, 非觸控＝speakBar 常駐顯示）
//   ② 手機直式 390×844  （DPR3, portrait）
//   ③ 平板直式 768×1024 （DPR2, portrait）
//   ④ 平板橫式 1024×768 （DPR2, landscape）
// 用 puppeteer-core 驅動系統 Chrome；截圖存 VQA_OUT（預設 scratchpad）。全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { writeFileSync, mkdirSync } from "node:fs";

const BASE_URL = process.env.VQA_URL || "http://127.0.0.1:3941/voxel/?debug=1";
const CHROME   = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const OUT_DIR  = process.env.VQA_OUT || "/tmp/claude-1000/-home-shihshih-ButFun/ef6ad408-d228-4f2b-9f33-dd6d6c332fde/scratchpad";
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

// 取元素的可視 bounding box（不可見 / display:none / 不存在 → null）
async function box(page, sel) {
  return page.evaluate((s) => {
    const el = document.querySelector(s);
    if (!el) return null;
    const cs = getComputedStyle(el);
    const r = el.getBoundingClientRect();
    const vis = r.width > 0 && r.height > 0 && cs.display !== "none" && cs.visibility !== "hidden";
    if (!vis) return null;
    return { top: r.top, left: r.left, bottom: r.bottom, right: r.right, width: r.width, height: r.height };
  }, sel);
}

// 兩 box 相交？（有共同面積才算，共邊不算）。任一為 null（該視口不可見）→ 不相交。
function intersects(a, b) {
  if (!a || !b) return false;
  return a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
}

function inView(b, vw, vh) {
  if (!b) return false;
  return b.left >= -1 && b.top >= -1 && b.right <= vw + 1 && b.bottom <= vh + 1;
}

async function clickSel(page, sel) {
  return page.evaluate((s) => { const el = document.querySelector(s); if (!el) return false; el.click(); return true; }, sel);
}

// 底部（及與底部拇指區相鄰）所有互動/顯示元素。中文名對應維護者清單。
// statsBar（❤️🍗 生存指標）加入：這正是被桌機常駐輸入列壓住的受害者，必須逐對驗。
const ELS = [
  { key: "joy",         sel: "#joy",         name: "搖桿" },
  { key: "speakBar",    sel: "#speakBar",    name: "輸入框" },
  { key: "statsBar",    sel: "#statsBar",    name: "指標條" },
  { key: "hotbar",      sel: "#hotbar",      name: "快捷欄" },
  { key: "speakToggle", sel: "#speakToggle", name: "說鈕" },
  { key: "dig",         sel: "#dig",         name: "挖鈕" },
  { key: "place",       sel: "#place",       name: "放置鈕" },
  { key: "jump",        sel: "#jump",        name: "跳鈕" },
  { key: "chatLog",     sel: "#chatLogHead", name: "聊天窗" },
  { key: "pwaInstall",  sel: "#pwaInstall",  name: "安裝鈕" },
];

async function collect(page) {
  const boxes = {};
  for (const e of ELS) boxes[e.key] = await box(page, e.sel);
  return boxes;
}

function nameOf(key) { return (ELS.find((e) => e.key === key) || {}).name || key; }

// 對「該視口實際可見」的元素做全兩兩檢查，回傳 { rows, fails }。
function pairCheck(boxes) {
  const keys = ELS.map((e) => e.key).filter((k) => boxes[k]); // 只查可見的
  const rows = [];
  const fails = [];
  for (let i = 0; i < keys.length; i++) {
    for (let j = i + 1; j < keys.length; j++) {
      const a = keys[i], b = keys[j];
      const hit = intersects(boxes[a], boxes[b]);
      rows.push({ pair: `${nameOf(a)}↔${nameOf(b)}`, intersect: hit });
      if (hit) fails.push(`${nameOf(a)}↔${nameOf(b)}`);
    }
  }
  const hidden = ELS.map((e) => e.key).filter((k) => !boxes[k]).map(nameOf);
  return { rows, fails, hidden };
}

// 印一份兩兩表 + 相交摘要（給某個「態」用：常駐態 / 輸入列顯示態 / 聊天窗展開態）
function printPairTable(title, boxes) {
  const { rows, fails, hidden } = pairCheck(boxes);
  console.log(`  ── ${title} 底部全元素兩兩相交檢查 ──`);
  for (const r of rows) console.log(`    ${r.intersect ? "✗ 相交" : "✓ 分開"}  ${r.pair}`);
  if (hidden.length) console.log(`    （此態隱藏／不佔位，不參與檢查）：${hidden.join("、")}`);
  console.log(`    相交數: ${fails.length}${fails.length ? " → " + fails.join(",") : ""}`);
  return { fails, hidden };
}

async function runViewport(browser, { label, vw, vh, dpr, tag, touch }) {
  console.log(`\n【${label} ${vw}×${vh} DPR${dpr}${touch ? " 觸控" : " 桌機"}】`);
  const page = await browser.newPage();
  // 桌機用桌面 UA（前端不加 body.touch→speakBar 常駐）；觸控用 iPad UA。
  await page.setUserAgent(
    touch
      ? "Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"
      : "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"
  );
  await page.setViewport({ width: vw, height: vh, deviceScaleFactor: dpr, isMobile: touch, hasTouch: touch });
  await injectFpsCounter(page);

  const logs = [];
  page.on("pageerror", (e) => logs.push("[pageerror] " + e.message));

  await page.goto(BASE_URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await page.bringToFront();
  await sleep(6000); // 等地形 + mesh + UI 就緒

  // ── 截圖 1：常駐畫面 ──
  //   桌機：speakBar 本就常駐顯示（無 body.touch）＝本輪根治的關鍵態。
  //   觸控：speakBar 預設收起，此態不含輸入框。
  const shotIdle = `${OUT_DIR}/${tag}-idle.png`;
  await page.screenshot({ path: shotIdle });

  // ── 常駐態：底部全元素兩兩檢查 ──
  const boxes = await collect(page);
  const { fails } = printPairTable("常駐態", boxes);

  // 都在畫面內（可見者）
  const inViewFails = ELS.filter((e) => boxes[e.key] && !inView(boxes[e.key], vw, vh)).map((e) => e.name);

  // tap target ≥44px（可見的拇指鍵；只在觸控時強制，桌機用滑鼠不強制）
  const TAP = ["joy", "jump", "place", "dig", "speakToggle"];
  const tapFails = [];
  if (touch) {
    for (const k of TAP) {
      const b = boxes[k];
      if (b && (b.width < 44 || b.height < 44)) tapFails.push(`${nameOf(k)}(${b.width.toFixed(0)}×${b.height.toFixed(0)})`);
    }
  }

  // ── 輸入列顯示態 ──
  //   觸控：點「💬 說」鈕展開；桌機：本就常駐，直接量測（不需點）。
  if (touch) {
    await clickSel(page, "#speakToggle");
    await sleep(400);
  }
  const speakOpen = await box(page, "#speakBar");
  const inputFocusable = await page.evaluate(() => {
    const el = document.querySelector("#speakInput");
    if (!el) return false;
    el.focus();
    return document.activeElement === el;
  });
  // 輸入列顯示態：再做一次全兩兩檢查（含 輸入框↔指標條↔快捷欄——桌機這裡最容易疊）
  const boxesSpeak = await collect(page);
  const { fails: speakFails } = printPairTable("輸入列顯示態", boxesSpeak);
  const shotExpand = `${OUT_DIR}/${tag}-speak-expanded.png`;
  await page.screenshot({ path: shotExpand });
  // 收回：觸控移除 .open；桌機讓輸入框失焦即可
  if (touch) await page.evaluate(() => { const sb = document.querySelector("#speakBar"); if (sb) sb.classList.remove("open"); });
  else await page.evaluate(() => { const el = document.querySelector("#speakInput"); if (el) el.blur(); });

  // ── 聊天窗展開態：點聊天窗標題展開歷史，驗它（左下）不撞堆疊/搖桿 ──
  await clickSel(page, "#chatLogHead");
  await sleep(300);
  const boxesChat = await collect(page);
  const { fails: chatFails } = printPairTable("聊天窗展開態", boxesChat);
  const shotChat = `${OUT_DIR}/${tag}-chatlog-expanded.png`;
  await page.screenshot({ path: shotChat });
  await clickSel(page, "#chatLogHead"); // 收回

  const fps = await readFps(page);
  const allFails = [...fails, ...speakFails, ...chatFails];
  const pass = allFails.length === 0 && inViewFails.length === 0 && tapFails.length === 0 &&
               !!speakOpen && inputFocusable && fps > 20;

  console.log(`  可見元素都在畫面內: ${inViewFails.length === 0 ? "✓" : "✗ " + inViewFails.join(",")}`);
  console.log(`  拇指鍵 tap≥44px: ${touch ? (tapFails.length === 0 ? "✓" : "✗ " + tapFails.join(",")) : "（桌機略）"}`);
  console.log(`  輸入列顯示: 輸入框可見 ${speakOpen ? "✓" : "✗"} / 可聚焦 ${inputFocusable ? "✓" : "✗"}`);
  console.log(`  三態相交合計: ${allFails.length}${allFails.length ? " → " + [...new Set(allFails)].join(",") : ""}`);
  console.log(`  FPS: ${fps.toFixed(1)}`);
  console.log(`  截圖: ${shotIdle}`);
  console.log(`        ${shotExpand}`);
  console.log(`        ${shotChat}`);
  if (logs.length) console.log("  頁面錯誤:\n  " + logs.slice(0, 6).join("\n  "));
  console.log(`  判定: ${pass ? "PASS ✅" : "FAIL ❌"}`);

  await page.close();
  return { label, pass, fails: allFails, inViewFails, tapFails, speakOpen: !!speakOpen, inputFocusable, fps,
           boxes, screenshots: [shotIdle, shotExpand, shotChat] };
}

(async () => {
  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU_ARGS });
  const results = {};
  // ① 桌機（含！這是本輪根治重點：桌機常駐 speakBar 不再壓 statsBar）
  results.desktop = await runViewport(browser, { label: "桌機", vw: 1280, vh: 800, dpr: 1, tag: "desktop-1280x800", touch: false });
  results.phone   = await runViewport(browser, { label: "手機直式", vw: 390, vh: 844, dpr: 3, tag: "phone-390x844", touch: true });
  results.tabletP = await runViewport(browser, { label: "平板直式", vw: 768, vh: 1024, dpr: 2, tag: "tablet-portrait-768x1024", touch: true });
  results.tabletL = await runViewport(browser, { label: "平板橫式", vw: 1024, vh: 768, dpr: 2, tag: "tablet-landscape-1024x768", touch: true });
  await browser.close();

  console.log("\n══════════════════════════════════════════");
  console.log("乙太方界 底部 UI 全元素不相交 QA 總結（四視口·含桌機）");
  console.log("══════════════════════════════════════════");
  for (const r of Object.values(results)) {
    console.log(`${r.label}: ${r.pass ? "PASS ✅" : "FAIL ❌"}  三態相交 ${r.fails.length}  FPS ${r.fps.toFixed(1)}`);
  }
  writeFileSync(`${OUT_DIR}/voxel-ui-bottom-overlap-qa.json`, JSON.stringify(results, null, 2));
  const allPass = Object.values(results).every((r) => r.pass);
  console.log(`\n整體判定: ${allPass ? "PASS ✅" : "FAIL ❌"}`);
  console.log(`截圖目錄: ${OUT_DIR}`);
  process.exit(allPass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
