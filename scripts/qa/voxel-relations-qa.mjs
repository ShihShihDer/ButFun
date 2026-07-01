// ============================================================
// voxel-relations-qa.mjs — 居民交情網 v1（ROADMAP 708）真實瀏覽器 QA
// ============================================================
// 緣起：居民彼此拜訪（671）很久前就悄悄累積情誼（672：陌生→相識→老朋友），
// 驅動問候語/八卦轉述/互助蓋家，但玩家從沒有任何管道看見「這座小社會到底誰跟
// 誰要好」。本切片加一個新的唯讀後端端點 `/voxel/relations` + 純前端面板，把這份
// 隱形的資料攤開給玩家看。這支腳本用真瀏覽器：
//   1) 用合成資料直接驗證排序純函式（老朋友優先、同層級依拜訪次數排序）；
//   2) 真的打 `/voxel/relations`，確認回傳 4 位居民兩兩組合共 6 對、欄位齊全；
//   3) 真的點擊 🧑‍🤝‍🧑 按鈕（dispatch PointerEvent）開面板，檢查渲染出的列數與內容；
//   4) 用合成資料（含三種層級混合）餵給 renderRelationsPanel，確認圖示/文案/排序在 DOM 裡正確呈現；
//   5) 關閉面板。
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

  // ── 1) 排序純函式：老朋友優先、同層級依拜訪次數多到少 ──
  const sorted = await page.evaluate(() => {
    const rows = [
      { a: "賽勒", b: "奧瑞", tier: "stranger", visits: 0 },
      { a: "露娜", b: "諾娃", tier: "friend", visits: 12 },
      { a: "露娜", b: "賽勒", tier: "acquaintance", visits: 4 },
      { a: "諾娃", b: "奧瑞", tier: "friend", visits: 20 },
    ];
    return window.__voxel.sortRelationRows(rows).map((r) => `${r.a}-${r.b}:${r.tier}:${r.visits}`);
  });
  check(
    "排序：老朋友優先且高拜訪次數在前，同層級後才輪到相識/陌生",
    sorted.join(",") === "諾娃-奧瑞:friend:20,露娜-諾娃:friend:12,露娜-賽勒:acquaintance:4,賽勒-奧瑞:stranger:0",
    JSON.stringify(sorted)
  );

  // ── 2) 真打後端 `/voxel/relations`，確認 4 位居民兩兩組合共 6 對、欄位齊全 ──
  const apiRows = await page.evaluate(async () => {
    const resp = await fetch("/voxel/relations");
    if (!resp.ok) return null;
    return await resp.json();
  });
  check("`/voxel/relations` 回傳陣列", Array.isArray(apiRows), JSON.stringify(apiRows));
  check("共 6 對居民組合（4 選 2）", Array.isArray(apiRows) && apiRows.length === 6,
    `length=${apiRows?.length}`);
  const validTiers = ["stranger", "acquaintance", "friend"];
  check(
    "每對都有 a/b 名字 + 合法 tier + 非負 visits",
    Array.isArray(apiRows) && apiRows.every((r) =>
      typeof r.a === "string" && r.a.length > 0 &&
      typeof r.b === "string" && r.b.length > 0 &&
      validTiers.includes(r.tier) &&
      typeof r.visits === "number" && r.visits >= 0
    ),
    JSON.stringify(apiRows)
  );

  // ── 3) 真的點擊 🧑‍🤝‍🧑 按鈕開面板，檢查列出真實資料（此刻世界剛起、多半陌生也沒關係，重點是列數對得上） ──
  const clicked = await page.evaluate(() => {
    const btn = document.getElementById("relationsBtn");
    if (!btn) return false;
    btn.dispatchEvent(new PointerEvent("click", { bubbles: true, cancelable: true }));
    return true;
  });
  await sleep(500);
  const visibleAfterClick = await page.evaluate(() => window.__voxel.relationsVisible);
  check("真的點擊 🧑‍🤝‍🧑 按鈕 → 面板開啟", clicked && visibleAfterClick);
  await page.screenshot({ path: join(OUT_DIR, "relations-1-opened.png") });

  const liveRowCount = await page.evaluate(() =>
    document.querySelectorAll("#relationsBody .relations-row").length
  );
  check("面板真的渲染出 6 列（讀真實 API 回應）", liveRowCount === 6, `liveRowCount=${liveRowCount}`);

  // ── 4) 用合成資料（三層級混合）餵給 renderRelationsPanel，驗證圖示/文案/排序在 DOM 正確呈現 ──
  const rendered = await page.evaluate(() => {
    const rows = [
      { a: "賽勒", b: "奧瑞", tier: "stranger", visits: 0 },
      { a: "露娜", b: "諾娃", tier: "friend", visits: 12 },
      { a: "露娜", b: "賽勒", tier: "acquaintance", visits: 4 },
    ];
    window.__voxel.renderRelationsPanel(rows);
    return [...document.querySelectorAll("#relationsBody .relations-row")].map((el) => ({
      icon: el.querySelector(".relations-icon")?.textContent || "",
      names: el.querySelector(".relations-names")?.textContent || "",
      tierText: el.querySelector(".relations-tier")?.textContent || "",
      dimmed: el.classList.contains("tier-stranger"),
    }));
  });
  check("合成資料渲染 3 列", rendered.length === 3, JSON.stringify(rendered));
  check("老朋友列排第一、圖示為🤝、文案為「老朋友」",
    rendered[0]?.names.includes("露娜") && rendered[0]?.names.includes("諾娃") &&
    rendered[0]?.icon === "🤝" && rendered[0]?.tierText === "老朋友",
    JSON.stringify(rendered[0]));
  check("相識列圖示為🙂、文案為「相識」",
    rendered[1]?.icon === "🙂" && rendered[1]?.tierText === "相識", JSON.stringify(rendered[1]));
  check("陌生列排最後且視覺淡化（tier-stranger class）",
    rendered[2]?.tierText === "陌生" && rendered[2]?.dimmed === true, JSON.stringify(rendered[2]));

  // ── 5) 關閉面板 ──
  const closedOk = await page.evaluate(() => {
    window.__voxel.closeRelations();
    return !window.__voxel.relationsVisible;
  });
  check("關閉交情網面板", closedOk);

  const pass = fails.length === 0;
  console.log("\n──────── 居民交情網 v1 QA 報告 ────────");
  console.log(pass ? "判定: PASS ✅（排序純函式正確、後端 API 6 對欄位齊全、真點擊開面板、渲染圖示/文案/排序正確）"
    : `判定: CHECK ⚠️ 失敗項目: ${fails.join("、")}`);
  if (logs.length) console.log("頁面訊息(節錄):\n" + logs.slice(0, 20).join("\n"));
  writeFileSync(join(OUT_DIR, "voxel-relations-qa.json"),
    JSON.stringify({ fails, sorted, apiRows, liveRowCount, rendered, pass }, null, 2));

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
