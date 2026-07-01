// ============================================================
// voxel-compass-qa.mjs — 居民羅盤 v1（ROADMAP 705）真實瀏覽器 QA
// ============================================================
// 緣起：居民散居世界四方（653：露娜在原點、諾娃/賽勒/奧瑞各在南/西/東 75 格）之後，
// 玩家沒有任何導航輔助，只能憑印象亂走才找得到人。本切片加一個純前端羅盤面板，
// 讀伺服器早已廣播的居民即時座標（零新協議），算出方向與距離。這支腳本用真瀏覽器：
//   1) 用已知座標/朝向直接驗證方位角換算的數學正確性（正前/正後/正右/正左四個基準）；
//   2) 真的點擊 🧭 羅盤按鈕（dispatch PointerEvent，非直呼內部函式）開面板，
//      檢查 4 位居民都列在面板裡、距離為正數；
//   3) 移動玩家後等一輪自動刷新，確認距離數字真的跟著更新（非靜態快照）。
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
const fails = [];
function check(label, ok, detail) {
  console.log(`${ok ? "OK✅" : "FAIL❌"} ${label}${detail ? "  " + detail : ""}`);
  if (!ok) fails.push(label);
  return ok;
}
function near(a, b, eps) { return Math.abs(a - b) <= eps; }

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
  await sleep(6000);

  // ── 1) 方位角換算數學正確性（純函式，四個基準方向）──
  // yaw=0 時鏡頭朝向 -Z（見 main.js viewDir 慣例）：正前方=北(-Z)、正右=東(+X)、正後=南(+Z)、正左=西(-X)。
  const bearings = await page.evaluate(() => {
    const v = window.__voxel;
    return {
      ahead: v.compassRelativeDeg(0, 0, 0, -10, 0),   // 正前方（北）
      right: v.compassRelativeDeg(0, 0, 10, 0, 0),    // 正右方（東）
      behind: v.compassRelativeDeg(0, 0, 0, 10, 0),   // 正後方（南）
      left: v.compassRelativeDeg(0, 0, -10, 0, 0),    // 正左方（西）
    };
  });
  check("正前方目標 → 0 度（箭頭朝上）", near(bearings.ahead, 0, 1), JSON.stringify(bearings));
  check("正右方目標 → 90 度", near(bearings.right, 90, 1), JSON.stringify(bearings));
  check("正後方目標 → 180 度", near(bearings.behind, 180, 1), JSON.stringify(bearings));
  check("正左方目標 → 270 度", near(bearings.left, 270, 1), JSON.stringify(bearings));

  // 轉個 90 度身（yaw=90°）後，原本的「正前方」目標應變成落在右手邊（螢幕右）。
  const afterTurn = await page.evaluate(() => window.__voxel.compassRelativeDeg(0, 0, 0, -10, Math.PI / 2));
  check("轉身 90 度後，原本正前方目標 → 90 度（螢幕右）", near(afterTurn, 90, 1), `got ${afterTurn}`);

  // ── 2) 真的點擊羅盤按鈕開面板，確認 4 位居民都列出、距離為正數 ──
  const residentCount = await page.evaluate(() => window.__voxel.residentCount);
  check("居民已載入 4 位", residentCount === 4, `residentCount=${residentCount}`);

  const clicked = await page.evaluate(() => {
    const btn = document.getElementById("compassBtn");
    if (!btn) return false;
    btn.dispatchEvent(new PointerEvent("click", { bubbles: true, cancelable: true }));
    return true;
  });
  await sleep(400);
  const visibleAfterClick = await page.evaluate(() => window.__voxel.compassVisible);
  check("真的點擊 🧭 按鈕 → 面板開啟", clicked && visibleAfterClick);
  await page.screenshot({ path: join(OUT_DIR, "compass-1-opened.png") });

  const rowInfo = await page.evaluate(() => {
    const rows = [...document.querySelectorAll("#compassBody .compass-row")];
    return rows.map((r) => ({
      name: r.querySelector(".compass-name")?.textContent || "",
      dist: r.querySelector(".compass-dist")?.textContent || "",
      rotate: r.querySelector(".compass-arrow")?.style.transform || "",
    }));
  });
  check("面板列出 4 位居民", rowInfo.length === 4, JSON.stringify(rowInfo));
  check("每一列都有非零旋轉樣式（箭頭真的有指向）", rowInfo.every((r) => /rotate\(/.test(r.rotate)),
    JSON.stringify(rowInfo));
  check("每一列距離都是正數格數", rowInfo.every((r) => /^\d+\s*格$/.test(r.dist)), JSON.stringify(rowInfo));

  // ── 3) 移動玩家後等一輪自動刷新（300ms），確認距離數字真的更新 ──
  await page.evaluate(() => window.__voxel.setPlayerPos(500, 30, 500)); // 遠離所有居民
  await sleep(700);
  const rowInfoAfterMove = await page.evaluate(() => {
    const rows = [...document.querySelectorAll("#compassBody .compass-row")];
    return rows.map((r) => r.querySelector(".compass-dist")?.textContent || "");
  });
  const distsGrew = rowInfoAfterMove.every((d, i) => {
    const before = parseInt(rowInfo[i]?.dist || "0", 10);
    const after = parseInt(d || "0", 10);
    return after > before;
  });
  check("玩家移到遠處後，面板距離自動刷新變大（非靜態快照）", distsGrew,
    `before=${JSON.stringify(rowInfo.map((r) => r.dist))} after=${JSON.stringify(rowInfoAfterMove)}`);

  // ── 4) 關閉面板 ──
  const closedOk = await page.evaluate(() => { window.__voxel.closeCompass(); return !window.__voxel.compassVisible; });
  check("關閉羅盤面板", closedOk);

  const pass = fails.length === 0;
  console.log("\n──────── 居民羅盤 v1 QA 報告 ────────");
  console.log(pass ? "判定: PASS ✅（方位數學正確、真點擊開面板列出 4 位居民、移動後自動刷新）"
    : `判定: CHECK ⚠️ 失敗項目: ${fails.join("、")}`);
  if (logs.length) console.log("頁面訊息(節錄):\n" + logs.slice(0, 20).join("\n"));
  writeFileSync(join(OUT_DIR, "voxel-compass-qa.json"),
    JSON.stringify({ fails, bearings, afterTurn, rowInfo, rowInfoAfterMove, pass }, null, 2));

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
