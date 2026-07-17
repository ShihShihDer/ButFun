// ============================================================
// voxel-mobile-ux-qa.mjs — M6 玩家體驗前端（手機直式 390×844）真實瀏覽器 QA
// ============================================================
// 緣起：維護者親抱怨過手機按鈕擠。本切片在窄視口把玩家體驗做深：
//   C3① 挖鈕視覺蓄力環：長按挖掘時圍著鈕緣一圈 conic-gradient 隨進度填滿；
//   C3③ 面板側邊抽屜：背包/工作台/熔爐/箱子在手機直式從左緣滑入、不擋中央；
//   C3④ 觸控鈕不重疊：搖桿/挖/放/跳/快捷欄兩兩 bounding box 不相交、都在螢幕內；
//   C1  新手卡：迎新居民頭像＋故事氣泡＋四步清單，位置移到左上避開右上帳號列；
//   C4  交情可視化：交情列加進度條（visits/門檻）＋等級名＋升級解鎖提示。
// 這支用「真的 Chrome、真的 WebGL、iPhone 直式視口」去驗——手機版面的重疊、
// 抽屜擋不擋中央、蓄力環有沒有真的畫出來，只在窄視口才現形。
//
// 怎麼跑：先在本機起 voxel 伺服器（PORT=3000 ./target/release/butfun-server），再：
//        node scripts/qa/voxel-mobile-ux-qa.mjs
//   環境變數：VQA_URL（預設 http://127.0.0.1:3000/voxel/?debug=1）、VQA_OUT（截圖輸出目錄）
// 不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const URL = process.env.VQA_URL || "http://127.0.0.1:3000/voxel/?debug=1";
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const OUT_DIR = process.env.VQA_OUT || join(__dirname, "out-mobile-ux");
mkdirSync(OUT_DIR, { recursive: true });

const GPU_ARGS = [
  "--no-sandbox", "--disable-setuid-sandbox", "--ignore-gpu-blocklist",
  "--enable-gpu", "--enable-webgl", "--use-gl=angle", "--use-angle=gl",
  "--disable-dev-shm-usage",
  "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding",
];

// 兩個矩形是否相交（bounding box overlap 判定）。
function overlaps(a, b) {
  if (!a || !b) return false;
  return !(a.right <= b.left || b.right <= a.left || a.bottom <= b.top || b.bottom <= a.top);
}

let failures = 0;
function check(name, ok, detail) {
  console.log(`  ${ok ? "✅" : "❌"} ${name}${detail ? "  — " + detail : ""}`);
  if (!ok) failures++;
}

async function main() {
  console.log("═".repeat(60));
  console.log("M6 玩家體驗前端 QA（手機直式 390×844）");
  console.log(`目標: ${URL}`);
  console.log("═".repeat(60));

  const browser = await puppeteer.launch({
    executablePath: CHROME, headless: "new", args: GPU_ARGS,
    defaultViewport: { width: 390, height: 844, deviceScaleFactor: 2, isMobile: true, hasTouch: true },
  });
  try {
    const page = await browser.newPage();
    page.on("pageerror", (e) => console.log("  [pageerror]", String(e).split("\n")[0]));
    await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 60000 });
    await new Promise((r) => setTimeout(r, 5000));

    check("body.touch 已掛上（觸控排版生效）",
      await page.evaluate(() => document.body.classList.contains("touch")));

    // ── C3④ 觸控鈕不重疊 + 都在螢幕內 ──
    await page.evaluate(() => {
      const dig = document.getElementById("dig");
      if (dig && getComputedStyle(dig).display === "none") dig.style.display = "flex";
    });
    const boxes = await page.evaluate(() => {
      const out = {};
      for (const id of ["joy", "jump", "place", "dig", "hotbar"]) {
        const el = document.getElementById(id);
        if (!el) { out[id] = null; continue; }
        const r = el.getBoundingClientRect();
        out[id] = { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
      }
      return out;
    });
    const pairs = [["joy", "jump"], ["joy", "place"], ["joy", "dig"], ["jump", "place"],
      ["jump", "dig"], ["place", "dig"], ["joy", "hotbar"], ["jump", "hotbar"],
      ["place", "hotbar"], ["dig", "hotbar"]];
    const bad = pairs.filter(([a, b]) => overlaps(boxes[a], boxes[b])).map((p) => p.join("×"));
    check("C3④ 觸控鈕兩兩不重疊", bad.length === 0, bad.length ? "重疊: " + bad.join(", ") : "");
    const off = Object.entries(boxes).filter(([, b]) => b && (b.right > 390.5 || b.left < -0.5 || b.bottom > 844.5)).map(([id]) => id);
    check("C3④ 所有觸控鈕都在螢幕內", off.length === 0, off.length ? "越界: " + off.join(", ") : "");

    // ── C3① 挖鈕蓄力環 ──
    await page.evaluate(() => {
      const dig = document.getElementById("dig");
      if (dig) { dig.classList.add("charging"); dig.style.setProperty("--dig-progress", "0.6"); }
    });
    await new Promise((r) => setTimeout(r, 300));
    const ring = await page.evaluate(() => {
      const dig = document.getElementById("dig");
      const cs = getComputedStyle(dig, "::before");
      return { opacity: cs.opacity, conic: /conic|gradient/i.test(cs.backgroundImage || "") };
    });
    check("C3① 蓄力環 charging 時顯示 + 用 conic-gradient",
      ring.opacity === "1" && ring.conic, `opacity=${ring.opacity} conic=${ring.conic}`);
    await page.screenshot({ path: join(OUT_DIR, "dig-ring.png") });
    await page.evaluate(() => { const d = document.getElementById("dig"); if (d) { d.classList.remove("charging"); d.style.removeProperty("--dig-progress"); } });

    // ── C3③ 側邊抽屜（背包）──
    const drawer = await page.evaluate(() => {
      document.getElementById("bagBtn")?.click();
      const p = document.getElementById("bagPanel");
      const r = p.getBoundingClientRect();
      return { left: r.left, right: r.right, bottom: r.bottom, top: r.top };
    });
    check("C3③ 背包抽屜貼左緣（left≈0）", Math.abs(drawer.left) < 2, `left=${drawer.left}`);
    check("C3③ 背包抽屜貼滿高度（撐到底）", drawer.top < 2 && drawer.bottom > 840, `top=${drawer.top} bottom=${drawer.bottom}`);
    check("C3③ 背包抽屜留下右半螢幕（右緣 < 260，右側拇指鈕/世界不被蓋）", drawer.right < 260, `right=${drawer.right}`);
    await page.screenshot({ path: join(OUT_DIR, "bag-drawer.png") });
    await page.evaluate(() => document.getElementById("bagClose")?.click());

    // ── C1 新手卡 ──
    const onboard = await page.evaluate(() => {
      const v = window.__voxel || {};
      if (typeof v.renderOnboardCard !== "function") return { ok: false };
      v.renderOnboardCard({
        t: "onboard", greeter: "露娜", hint: "先走到我旁邊，點我說聲哈囉吧！",
        steps: [
          { label: "跟居民打招呼", done: true }, { label: "採集一些原料", done: false },
          { label: "合成第一個東西", done: false }, { label: "把方塊放到世界裡", done: false },
        ],
      });
      const card = document.getElementById("onboardCard");
      const r = card.getBoundingClientRect();
      return {
        ok: true, avatar: card.querySelector(".ob-avatar")?.textContent || "",
        bubble: !!card.querySelector(".ob-bubble"), steps: card.querySelectorAll(".ob-step").length,
        left: r.left, right: r.right,
      };
    });
    check("C1 新手卡有迎新頭像 + 故事氣泡 + 四步", onboard.ok && onboard.avatar && onboard.bubble && onboard.steps === 4,
      `avatar=${onboard.avatar} bubble=${onboard.bubble} steps=${onboard.steps}`);
    check("C1 新手卡在左上、右緣避開右上帳號列（right < 300）", onboard.left < 40 && onboard.right < 300,
      `left=${onboard.left} right=${onboard.right}`);
    await page.screenshot({ path: join(OUT_DIR, "onboard-card.png") });

    // ── C4 交情可視化 ──
    const bonds = await page.evaluate(() => {
      const v = window.__voxel || {};
      if (typeof v.bondProgress !== "function" || typeof v.renderRelationsPanel !== "function") return { ok: false };
      const p2 = v.bondProgress(2), p6 = v.bondProgress(6), p8 = v.bondProgress(8);
      v.renderRelationsPanel([
        { a: "露娜", b: "諾娃", tier: "stranger", visits: 2 },
        { a: "奧瑞", b: "賽勒", tier: "acquaintance", visits: 6 },
        { a: "露娜", b: "奧瑞", tier: "friend", visits: 12 },
      ]);
      const rows = [...document.getElementById("relationsBody").querySelectorAll(".relations-row")].map((row) => ({
        hasBar: !!row.querySelector(".relations-prog-fill"),
        count: row.querySelector(".relations-prog-count")?.textContent || "",
        unlock: row.querySelector(".relations-unlock")?.textContent || "",
      }));
      return { ok: true, p2, p6, p8, rows };
    });
    check("C4 bondProgress 門檻正確（2→還差1、6→還差2、8→滿級）",
      bonds.ok && bonds.p2.remaining === 1 && bonds.p6.remaining === 2 && bonds.p8.maxed === true,
      bonds.ok ? `p2=${bonds.p2.remaining} p6=${bonds.p6.remaining} p8.maxed=${bonds.p8.maxed}` : "未暴露");
    check("C4 每列有進度條 + 拜訪次數 + 升級/滿級提示",
      bonds.ok && bonds.rows.length === 3 && bonds.rows.every((r) => r.hasBar && r.count && r.unlock));
    await page.evaluate(() => {
      document.getElementById("menuBtn")?.click();
      document.getElementById("relationsBtn")?.click();
    });
    await new Promise((r) => setTimeout(r, 400));
    await page.screenshot({ path: join(OUT_DIR, "relations-bonds.png") });

    writeFileSync(join(OUT_DIR, "report.json"), JSON.stringify({ boxes, ring, drawer, onboard, bonds, failures }, null, 2));
    console.log("─".repeat(60));
    console.log(failures === 0 ? "全數通過 ✅" : `有 ${failures} 項未通過 ❌`);
    console.log(`截圖: ${OUT_DIR}`);
  } finally {
    await Promise.race([browser.close().catch(() => {}), new Promise((r) => setTimeout(r, 8000))]);
    try { const p = browser.process(); if (p) p.kill("SIGKILL"); } catch (_) { /* 已關 */ }
  }
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((e) => { console.error("QA 失敗:", e); process.exit(1); });
