// ============================================================
// voxel-skills-qa.mjs — 居民技能簿 v1（ROADMAP 719）真實瀏覽器 QA
// ============================================================
// 緣起：技能發明（716）+ 傳授（717）至今只透過稍縱即逝的 Feed 文字曝光（「露娜
// 教了我『燒玻璃』這招！」播報一過就沒了），玩家從沒有任何管道能回頭查「這座小
// 社會現在誰會什麼」。本切片加一個新的唯讀後端端點 `/voxel/skills` + 純前端面板，
// 把這份隱形的資料攤開給玩家看（跟 708 交情網同一手法）。這支腳本用真瀏覽器：
//   1) 真的打 `/voxel/skills`，確認回傳 4 位居民、欄位齊全（name + skills 陣列）；
//   2) 真的點擊 🧠 按鈕（dispatch PointerEvent）開面板，檢查真實渲染的列數；
//   3) 用合成資料（含有技能/無技能兩種居民）餵給 renderSkillsPanel，確認技能
//      chip／「尚未發明任何技能」空狀態在 DOM 裡正確呈現；
//   4) 關閉面板。
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

  // ── 1) 真打後端 `/voxel/skills`，確認回傳 4 位居民、欄位齊全 ──
  const apiRows = await page.evaluate(async () => {
    const resp = await fetch("/voxel/skills");
    if (!resp.ok) return null;
    return await resp.json();
  });
  check("`/voxel/skills` 回傳陣列", Array.isArray(apiRows), JSON.stringify(apiRows));
  check("共 4 位居民", Array.isArray(apiRows) && apiRows.length === 4, `length=${apiRows?.length}`);
  check(
    "每位居民都有非空 name + skills 陣列",
    Array.isArray(apiRows) && apiRows.every((r) =>
      typeof r.name === "string" && r.name.length > 0 && Array.isArray(r.skills)
    ),
    JSON.stringify(apiRows)
  );

  // ── 2) 真的點擊 🧠 按鈕開面板，檢查真實渲染出 4 列（世界剛起，多半還沒發明技能也沒關係） ──
  const clicked = await page.evaluate(() => {
    const btn = document.getElementById("skillsBtn");
    if (!btn) return false;
    btn.dispatchEvent(new PointerEvent("click", { bubbles: true, cancelable: true }));
    return true;
  });
  await sleep(500);
  const visibleAfterClick = await page.evaluate(() => window.__voxel.skillsVisible);
  check("真的點擊 🧠 按鈕 → 面板開啟", clicked && visibleAfterClick);
  await page.screenshot({ path: join(OUT_DIR, "skills-1-opened.png") });

  const liveRowCount = await page.evaluate(() =>
    document.querySelectorAll("#skillsBody .skills-row").length
  );
  check("面板真的渲染出 4 列（讀真實 API 回應）", liveRowCount === 4, `liveRowCount=${liveRowCount}`);

  // ── 3) 合成資料（有技能/無技能混合），驗證 chip／空狀態在 DOM 正確呈現 ──
  const rendered = await page.evaluate(() => {
    const rows = [
      { name: "露娜", skills: ["燒玻璃", "曬乾磚"] },
      { name: "諾娃", skills: [] },
    ];
    window.__voxel.renderSkillsPanel(rows);
    return [...document.querySelectorAll("#skillsBody .skills-row")].map((el) => ({
      name: el.querySelector(".skills-name")?.textContent || "",
      chips: [...el.querySelectorAll(".skills-chip")].map((c) => c.textContent),
      noneText: el.querySelector(".skills-none")?.textContent || "",
    }));
  });
  check("合成資料渲染 2 列", rendered.length === 2, JSON.stringify(rendered));
  check("露娜列顯示 2 枚技能 chip（燒玻璃/曬乾磚）",
    rendered[0]?.name === "露娜" &&
    rendered[0]?.chips.length === 2 &&
    rendered[0]?.chips.includes("燒玻璃") &&
    rendered[0]?.chips.includes("曬乾磚"),
    JSON.stringify(rendered[0]));
  check("諾娃列（無技能）顯示「尚未發明任何技能」空狀態、零 chip",
    rendered[1]?.name === "諾娃" &&
    rendered[1]?.chips.length === 0 &&
    rendered[1]?.noneText === "尚未發明任何技能",
    JSON.stringify(rendered[1]));

  // ── 4) 關閉面板 ──
  const closedOk = await page.evaluate(() => {
    window.__voxel.closeSkills();
    return !window.__voxel.skillsVisible;
  });
  check("關閉技能簿面板", closedOk);

  const pass = fails.length === 0;
  console.log("\n──────── 居民技能簿 v1 QA 報告 ────────");
  console.log(pass ? "判定: PASS ✅（後端 API 4 位居民欄位齊全、真點擊開面板、技能 chip/空狀態渲染正確）"
    : `判定: CHECK ⚠️ 失敗項目: ${fails.join("、")}`);
  if (logs.length) console.log("頁面訊息(節錄):\n" + logs.slice(0, 20).join("\n"));
  writeFileSync(join(OUT_DIR, "voxel-skills-qa.json"),
    JSON.stringify({ fails, apiRows, liveRowCount, rendered, pass }, null, 2));

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
