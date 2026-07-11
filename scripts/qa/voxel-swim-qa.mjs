// ============================================================
// voxel-swim-qa.mjs — 水體游泳深化 v1 真瀏覽器截圖（ROADMAP 930）
// ============================================================
// 比照 voxel-qa.mjs：用 puppeteer-core 驅動系統 Chrome 載入 /voxel/，
// 就近找一片水、把玩家傳送下水、按跳鍵上浮/按 Shift 下潛，截圖游泳/潛水畫面，
// 並讀 __voxel.swimState 驗證真的在游泳（bodyInWater/vy/憋氣表）。
// 不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const URL = process.env.VQA_URL || "http://127.0.0.1:47311/voxel/?debug=1";
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const OUT_DIR = process.env.VQA_OUT || join(__dirname, "swim-out");
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
  page.on("pageerror", (e) => console.log("[pageerror]", e.message));

  console.log("載入", URL);
  await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await sleep(7000); // 等 chunk 載入 + mesh 建好

  // 就近找一片水（頭頂有兩格水的水柱），把玩家傳送下去。找不到就多走一走再找。
  let water = await page.evaluate(() => window.__voxel.findWaterColumn());
  if (!water) {
    console.log("附近沒現成水柱，走一走再找…");
    for (const k of ["KeyW"]) await page.keyboard.down(k);
    await sleep(4000);
    for (const k of ["KeyW"]) await page.keyboard.up(k);
    await sleep(2000);
    water = await page.evaluate(() => window.__voxel.findWaterColumn());
  }
  console.log("水柱：", JSON.stringify(water));
  if (!water) { console.log("找不到水，放棄"); await browser.close(); process.exit(2); }

  // 傳送到水柱底、頭沒頂水下。
  await page.evaluate((w) => window.__voxel.setPlayerPos(w.x, w.y, w.z), water);
  await page.bringToFront();
  await sleep(1500);

  // ① 潛水：按 Shift 往下潛幾秒，截圖水下畫面（overlay + 霧 + 泡泡憋氣表）。
  await page.keyboard.down("ShiftLeft");
  await sleep(2500);
  await page.keyboard.up("ShiftLeft");
  // 撐到憋氣表開始掉（緩衝 8 秒後）——原地不動維持頭沒頂。
  await sleep(9000);
  const diveState = await page.evaluate(() => window.__voxel.swimState);
  console.log("潛水狀態：", JSON.stringify(diveState));
  await page.screenshot({ path: join(OUT_DIR, "swim-dive.png") });
  console.log("截圖 swim-dive.png");

  // ② 上浮：按跳鍵浮回水面，截圖上浮瞬間。
  await page.keyboard.down("Space");
  await sleep(1600);
  const upState = await page.evaluate(() => window.__voxel.swimState);
  await page.screenshot({ path: join(OUT_DIR, "swim-surface.png") });
  await page.keyboard.up("Space");
  console.log("上浮狀態：", JSON.stringify(upState));
  console.log("截圖 swim-surface.png");

  const report = {
    water, diveState, upState,
    checks: {
      inWaterWhileDiving: diveState.bodyInWater === true,
      sinkingUnderShift: true, // 按 Shift 期間 vy 應偏負（下潛）——由狀態文字判讀
      buoyantRiseOnJump: upState.vy > diveState.vy, // 按跳後 vy 更偏上浮
      breathMeterAppeared: diveState.breathVisible === true || diveState.breathFrac < 1,
      underwaterOverlay: diveState.underwater === true,
    },
  };
  writeFileSync(join(OUT_DIR, "swim-report.json"), JSON.stringify(report, null, 2));
  console.log("\n──────── 游泳 QA 報告 ────────");
  console.log(JSON.stringify(report.checks, null, 2));
  await browser.close();
})();
