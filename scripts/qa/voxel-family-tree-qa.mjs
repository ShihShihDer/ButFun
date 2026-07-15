// ============================================================
// voxel-family-tree-qa.mjs — 家族樹面板（自主提案切片，接續 708+927+928）真實瀏覽器 QA
// ============================================================
// 緣起：927 讓戀人結為連理（婚書落地）、928 讓已成婚夫妻共同迎來孩子（名冊記下
// co_parent/co_parent_name）——但這兩份早就持久化在案的資料，至今從沒有一處
// 把它們攤開成看得懂的「這座小社會誰跟誰成家、生了誰」。本切片加一個新的唯讀
// 後端端點 `/voxel/family` + 純前端合併渲染在交情網面板頂端。這支腳本用真瀏覽器：
//   1) 用合成資料直接驗證 buildFamilyLines 純函式（婚姻分組/親子分組/單親/去重/排序）；
//   2) 真的打 `/voxel/family`，確認回傳形狀正確（marriages/children 皆為陣列）；
//   3) 真的點擊 🧑‍🤝‍🧑 按鈕開面板（家族樹段落緊接小圈子渲染，世界剛起沒有婚姻/
//      親子資料時應整段留白、不佔面板空間）；
//   4) 用合成資料餵給 renderFamilySection，確認圖示/文案在 DOM 裡正確呈現；
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

  // ── 1) buildFamilyLines 純函式：婚姻分組/親子分組/單親/去重/排序 ──
  const lines = await page.evaluate(() => {
    const marriages = [
      { a: "露娜", b: "諾娃" }, // 已成婚、目前尚無孩子
    ];
    const children = [
      { name: "小雨", parent: "諾娃", co_parent: "露娜" }, // 與上面那對夫妻共同的孩子（順序反過來也要能對上）
      { name: "小星", parent: "露娜", co_parent: "諾娃" }, // 同一對夫妻的第二個孩子
      { name: "小溪", parent: "奧瑞", co_parent: null }, // 單親（舊資料，沒有 co_parent）
    ];
    return window.__voxel.buildFamilyLines(marriages, children);
  });
  check("共 2 組家族（露娜＆諾娃一組、奧瑞單親一組）", lines.length === 2, JSON.stringify(lines));
  const couple = lines.find((l) => l.parents.length === 2);
  check("露娜＆諾娃這組父母正確、孩子正確合併成 2 位且不重複分組",
    couple && couple.parents.includes("露娜") && couple.parents.includes("諾娃") &&
    couple.children.length === 2 && couple.children.includes("小雨") && couple.children.includes("小星"),
    JSON.stringify(couple));
  const single = lines.find((l) => l.parents.length === 1);
  check("奧瑞單親這組只有 1 位父母、1 位孩子",
    single && single.parents[0] === "奧瑞" && single.children.length === 1 && single.children[0] === "小溪",
    JSON.stringify(single));

  const marriedNoKids = await page.evaluate(() =>
    window.__voxel.buildFamilyLines([{ a: "賽勒", b: "米拉" }], [])
  );
  check("已成婚但尚無孩子的一對，仍會出現一列（children 空陣列）",
    marriedNoKids.length === 1 && marriedNoKids[0].children.length === 0,
    JSON.stringify(marriedNoKids));

  // ── 2) 真打後端 `/voxel/family`，確認回傳形狀正確 ──
  const apiData = await page.evaluate(async () => {
    const resp = await fetch("/voxel/family");
    if (!resp.ok) return null;
    return await resp.json();
  });
  check("`/voxel/family` 回傳物件含 marriages/children 兩個陣列",
    apiData && Array.isArray(apiData.marriages) && Array.isArray(apiData.children),
    JSON.stringify(apiData));

  // ── 3) 真的點擊 🧑‍🤝‍🧑 按鈕開面板（世界剛起多半還沒人結婚生子，familyBody 留白也是對的） ──
  const clicked = await page.evaluate(() => {
    const btn = document.getElementById("relationsBtn");
    if (!btn) return false;
    btn.dispatchEvent(new PointerEvent("click", { bubbles: true, cancelable: true }));
    return true;
  });
  await sleep(500);
  const visibleAfterClick = await page.evaluate(() => window.__voxel.relationsVisible);
  check("真的點擊 🧑‍🤝‍🧑 按鈕 → 面板開啟（家族樹段落與交情網共用同一個面板）", clicked && visibleAfterClick);
  await page.screenshot({ path: join(OUT_DIR, "family-tree-1-opened.png") });

  // ── 4) 用合成資料餵給 renderFamilySection，驗證圖示/文案在 DOM 正確呈現 ──
  const rendered = await page.evaluate(() => {
    const data = {
      marriages: [{ a: "露娜", b: "諾娃" }],
      children: [{ name: "小雨", parent: "諾娃", co_parent: "露娜" }],
    };
    window.__voxel.renderFamilySection(data);
    return [...document.querySelectorAll("#familyBody .family-row")].map((el) => ({
      icon: el.querySelector(".family-icon")?.textContent || "",
      names: el.querySelector(".family-names")?.textContent || "",
    }));
  });
  check("合成資料渲染 1 列（一對父母 + 孩子）", rendered.length === 1, JSON.stringify(rendered));
  check("圖示為 💍（一對父母）、文案含雙親姓名與孩子名字",
    rendered[0]?.icon === "💍" &&
    rendered[0]?.names.includes("露娜") && rendered[0]?.names.includes("諾娃") &&
    rendered[0]?.names.includes("小雨"),
    JSON.stringify(rendered[0]));

  const renderedSingle = await page.evaluate(() => {
    window.__voxel.renderFamilySection({ marriages: [], children: [{ name: "小溪", parent: "奧瑞", co_parent: null }] });
    return [...document.querySelectorAll("#familyBody .family-row")].map((el) => ({
      icon: el.querySelector(".family-icon")?.textContent || "",
      names: el.querySelector(".family-names")?.textContent || "",
    }));
  });
  check("單親圖示為 👶（非 💍）、文案含父母與孩子名字",
    renderedSingle[0]?.icon === "👶" &&
    renderedSingle[0]?.names.includes("奧瑞") && renderedSingle[0]?.names.includes("小溪"),
    JSON.stringify(renderedSingle[0]));

  const renderedEmpty = await page.evaluate(() => {
    window.__voxel.renderFamilySection({ marriages: [], children: [] });
    return document.getElementById("familyBody")?.innerHTML || "";
  });
  check("沒有任何家族資料時 familyBody 整段留白（不佔面板空間）", renderedEmpty === "", `innerHTML="${renderedEmpty}"`);

  // ── 5) 關閉面板 ──
  const closedOk = await page.evaluate(() => {
    window.__voxel.closeRelations();
    return !window.__voxel.relationsVisible;
  });
  check("關閉交情網面板", closedOk);

  const pass = fails.length === 0;
  console.log("\n──────── 家族樹面板 QA 報告 ────────");
  console.log(pass ? "判定: PASS ✅（分組純函式正確、後端 API 形狀正確、真點擊開面板、渲染圖示/文案正確、空資料整段留白）"
    : `判定: CHECK ⚠️ 失敗項目: ${fails.join("、")}`);
  if (logs.length) console.log("頁面訊息(節錄):\n" + logs.slice(0, 20).join("\n"));
  writeFileSync(join(OUT_DIR, "voxel-family-tree-qa.json"),
    JSON.stringify({ fails, lines, marriedNoKids, apiData, rendered, renderedSingle, renderedEmpty, pass }, null, 2));

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
