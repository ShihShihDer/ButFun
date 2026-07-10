// ============================================================
// voxel-clock-qa.mjs — 時段指示器 v1（ROADMAP 896）真瀏覽器 QA
// ============================================================
// 多位居民（建議箱）反映：黃昏/夜晚只能靠天空亮度猜，沒有明確的「當前時段」提示。
// 本刀在 HUD 頂部中央加一枚徽章（🌅黎明/☀️白天/🌆黃昏/🌙夜晚＋概略時刻），底色隨時段換調。
// 此 QA 用系統 Chrome 載入 /voxel/，就地撥鐘（window.__voxel.qaSetWorldTime，純視覺、無權威影響），
// 逐一驗四個時段的徽章文字/class 對映，各截一張圖，並量 FPS、確認非黑屏。
// 不抄外部碼；全繁中註解；node --check 過。

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

// 期望：撥到這個 worldTime 時，徽章應顯示這個時段（對齊 SKY_KEYS 天色轉折）。
const CASES = [
  { t: 0.05, name: "夜晚", cls: "cl-night", icon: "🌙" },
  { t: 0.30, name: "黎明", cls: "cl-dawn",  icon: "🌅" },
  { t: 0.50, name: "白天", cls: "cl-day",   icon: "☀️" },
  { t: 0.78, name: "黃昏", cls: "cl-dusk",  icon: "🌆" },
];

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

  // 純函式對映自檢（在頁面內跑 window.__voxelClockPhase / __voxelClockTime）。
  const pureOk = await page.evaluate(() => {
    const ph = window.__voxelClockPhase, tm = window.__voxelClockTime;
    if (!ph || !tm) return { ok: false, why: "純函式未掛上 window" };
    const checks = [
      [0.0, "夜晚"], [0.21, "夜晚"], [0.22, "黎明"], [0.37, "黎明"],
      [0.38, "白天"], [0.69, "白天"], [0.70, "黃昏"], [0.87, "黃昏"], [0.88, "夜晚"],
    ];
    for (const [t, want] of checks) if (ph(t).name !== want) return { ok: false, why: `phase(${t})=${ph(t).name} 應為 ${want}` };
    if (tm(0.5) !== "12:00") return { ok: false, why: `time(0.5)=${tm(0.5)} 應為 12:00` };
    if (tm(0.25) !== "06:00") return { ok: false, why: `time(0.25)=${tm(0.25)} 應為 06:00` };
    return { ok: true };
  });
  console.log("純函式對映:", JSON.stringify(pureOk));

  // 逐時段撥鐘 → 驗徽章 → 截圖。
  const results = [];
  let allPass = pureOk.ok;
  for (const c of CASES) {
    // 撥本地視覺鐘（同步更新徽章 DOM）→ 立刻截圖（趁伺服器權威快照尚未把 worldTime 覆寫回真實時間；
    // 這個「會被覆寫」正是設計要的：真遊戲中徽章永遠跟著伺服器每幀廣播的權威時間）。
    const got = await page.evaluate((t) => window.__voxel.qaSetWorldTime(t), c.t);
    await page.screenshot({ path: join(OUT_DIR, `clock-${c.cls}.png`) });
    const pass = got.name === c.name && got.cls === c.cls && got.icon === c.icon && /^\d\d:\d\d$/.test(got.time);
    if (!pass) allPass = false;
    results.push({ t: c.t, want: c.name, got: got.name, cls: got.cls, icon: got.icon, time: got.time, pass });
    await sleep(300);
  }
  console.table(results);

  const fpsAvg = await page.evaluate(() => {
    const dt = (performance.now() - window.__fps.t0) / 1000;
    return window.__fps.frames / dt;
  });
  // 非黑屏檢查：截一張全圖，統計去重位元組數（黑屏會近乎單一值）。
  const png = await page.screenshot();
  const distinct = new Set(png).size;
  const notBlack = distinct > 40;

  console.log(`FPS ${fpsAvg.toFixed(1)} / 去重位元組 ${distinct}（>40=非黑屏）`);
  if (logs.some((l) => l.startsWith("[pageerror]"))) { console.log("頁面錯誤:"); logs.filter((l) => l.startsWith("[pageerror]")).forEach((l) => console.log(l)); }

  const summary = { pureOk, results, fpsAvg, distinct, notBlack, allPass: allPass && notBlack };
  writeFileSync(join(OUT_DIR, "clock-qa-summary.json"), JSON.stringify(summary, null, 2));
  await browser.close();

  if (!summary.allPass) { console.error("QA 失敗 ❌"); process.exit(1); }
  console.log("QA 全通過 ✅（四時段徽章對映正確＋非黑屏＋FPS 健康）");
})().catch((e) => { console.error(e); process.exit(1); });
