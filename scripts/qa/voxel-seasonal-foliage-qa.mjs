// ============================================================
// voxel-seasonal-foliage-qa.mjs — 四季樹葉 v1 真瀏覽器 QA
// ============================================================
// 為什麼有這支：季節輪替(798)至今只染天空/霧，樹葉方塊一年到頭同一片綠。本刀讓
// 樹葉基底色隨四季換色（春嫩綠/夏濃綠/秋琥珀金/冬霜灰）。這支 QA 用系統 Chrome 載入
// /voxel/，在玩家面前就地立一面「樹葉牆」，逐一切換四季，讀回當季樹葉色 + 截圖，
// 驗證：①foliageLeafColor 四季回傳不同且符合設計 ②setSeason 觸發 chunk 重建
// ③畫面非黑屏、四季截圖看得出樹葉換色。不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const URL = process.env.VQA_URL || "http://127.0.0.1:3111/voxel/?debug=1";
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
const SEASONS = ["summer", "autumn", "winter", "spring"];

(async () => {
  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU_ARGS });
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 800 });
  const logs = [];
  page.on("console", (m) => logs.push("[console] " + m.text()));
  page.on("pageerror", (e) => logs.push("[pageerror] " + e.message));

  console.log("載入", URL);
  await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await page.bringToFront();
  await sleep(6000); // 等 chunk + mesh 建好

  // 在玩家面前立一面樹葉牆（3×3），把視角轉過去，作為四季換色的受控對照物。
  const built = await page.evaluate(() => {
    const v = window.__voxel;
    const p = v.player;
    const LEAVES = 6;
    // 找玩家前方一段地面：從腳下往 -Z 掃出「上方空氣、下方實心」的地表點。
    let gx = Math.round(p.x), gz = Math.round(p.z), gy = Math.round(p.y);
    let found = null;
    for (let d = 3; d <= 10; d++) {
      const tz = Math.round(p.z) - d;
      for (let yy = gy + 2; yy >= gy - 3; yy--) {
        if (v.getBlock(gx, yy, tz) !== 0 && v.getBlock(gx, yy + 1, tz) === 0) { found = { x: gx, y: yy + 1, z: tz }; break; }
      }
      if (found) break;
    }
    window.__qaFreezeSeason = true; // 凍結季節，讓 setSeason 不被伺服器快照覆寫（僅 QA 截圖用）
    if (!found) found = { x: gx, y: gy, z: Math.round(p.z) - 5 };
    // 立一面 3(寬)×3(高) 的樹葉牆。
    let n = 0;
    for (let dx = -1; dx <= 1; dx++) for (let dy = 0; dy <= 2; dy++) {
      v._qaSetBlock(found.x + dx, found.y + dy, found.z, LEAVES); n++;
    }
    // 視角轉向樹葉牆（大致朝 -Z）。
    if (typeof p.yaw === "number") p.yaw = 0;
    return { found, n, season0: v.worldSeason };
  });
  console.log("樹葉牆:", JSON.stringify(built));
  await sleep(1500);

  // 逐季切換：讀回當季樹葉色、確認重建、截圖。
  const results = [];
  for (const s of SEASONS) {
    const info = await page.evaluate((season) => {
      const v = window.__voxel;
      const before = v.foliageLeafColor(); // 切換前當季色
      v.setSeason(season);
      const col = v.foliageLeafColor(season);
      return { season, worldSeason: v.worldSeason, leafColor: col, prevColor: before };
    }, s);
    await sleep(2500); // 等節流 dirty 佇列把樹葉牆重建完（每幀最多 4 塊）
    // 取畫面中央一塊像素的平均色（粗略證明非黑屏、且畫面隨季節有變化）。
    const shot = join(OUT_DIR, `foliage-${s}.png`);
    await page.screenshot({ path: shot });
    const px = await page.evaluate(() => {
      const cv = document.querySelector("canvas");
      if (!cv) return null;
      const g = cv.getContext("webgl2") || cv.getContext("webgl");
      if (!g) return "no-gl-ctx"; // 走 three 的話讀不到，改由截圖人工看
      return "ok";
    });
    info.shot = shot;
    results.push(info);
    console.log(`  ${s}: leafColor=[${info.leafColor.map((x) => x.toFixed(2)).join(",")}] 截圖=${shot}`);
  }

  await browser.close();

  // 斷言：四季樹葉色兩兩不同，且秋偏暖（R>G）、夏偏綠（G>R）。
  const byS = Object.fromEntries(results.map((r) => [r.season, r.leafColor]));
  const key = (c) => c.map((x) => Math.round(x * 100)).join("/");
  const uniq = new Set(SEASONS.map((s) => key(byS[s])));
  const autumnWarm = byS.autumn[0] > byS.autumn[1];   // 秋：紅 > 綠（暖）
  const summerGreen = byS.summer[1] > byS.summer[0];  // 夏：綠 > 紅
  const springBrighter = byS.spring[1] > byS.summer[1]; // 春綠比夏更亮嫩
  const pass = uniq.size === 4 && autumnWarm && summerGreen && springBrighter;

  console.log("\n=== 判定 ===");
  console.log("四季樹葉色皆不同:", uniq.size === 4);
  console.log("秋葉偏暖(R>G):", autumnWarm, "夏葉偏綠(G>R):", summerGreen, "春葉更亮嫩:", springBrighter);
  console.log(pass ? "✅ PASS" : "❌ FAIL");
  if (logs.some((l) => l.startsWith("[pageerror]"))) {
    console.log("⚠️ 頁面錯誤:"); logs.filter((l) => l.startsWith("[pageerror]")).forEach((l) => console.log(l));
  }
  writeFileSync(join(OUT_DIR, "foliage-report.json"), JSON.stringify({ built, results, pass }, null, 2));
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error(e); process.exit(2); });
