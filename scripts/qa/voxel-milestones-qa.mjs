// ============================================================
// voxel-milestones-qa.mjs — 玩家里程碑 v1（ROADMAP 724）真實瀏覽器 QA
// ============================================================
// 緣起：居民有技能簿（719）、交情網（708）可回頭翻閱自己的成長，玩家的療癒循環
// （採集→合成→蓋造→種田→贈禮→交易→熟識→安眠）至今卻沒有任何一處能回頭看看
// 「我走了多遠」。本切片加一個新的唯讀後端端點 `/voxel/milestones` + 純前端面板，
// 達成瞬間再由 `milestone_unlocked` WS 訊息單播一則慶祝提示。這支腳本用真瀏覽器：
//   1) 真的打 `/voxel/milestones`（無 player）確認回傳 8 枚里程碑、欄位齊全、皆未達成；
//   2) 真的點擊 🏅 按鈕（dispatch PointerEvent）開面板，檢查真實渲染出的列數與進度文字；
//   3) 真的挖一塊方塊（`window.__voxel.doBreak()` 送真實 `break` WS 訊息）→
//      確認觸發「初次採集」慶祝提示 + `/voxel/milestones?player=` 查得到 earned:true；
//   4) 真的放一塊方塊（`doPlace()`）→ 確認「初次建造」同樣真的解鎖；
//   5) 合成資料驗證渲染（已達成/未達成兩種列的視覺區別）；
//   6) 關閉面板。
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

  // ── 1) 真打後端 `/voxel/milestones`（無 player），確認回傳 8 枚里程碑、欄位齊全、皆未達成 ──
  const apiRows = await page.evaluate(async () => {
    const resp = await fetch("/voxel/milestones");
    if (!resp.ok) return null;
    return await resp.json();
  });
  check("`/voxel/milestones` 回傳陣列", Array.isArray(apiRows), JSON.stringify(apiRows));
  check("共 8 枚里程碑", Array.isArray(apiRows) && apiRows.length === 8, `length=${apiRows?.length}`);
  check(
    "每枚里程碑都有非空 id/name_zh/desc_zh/icon 且 earned 為布林",
    Array.isArray(apiRows) && apiRows.every((r) =>
      typeof r.id === "string" && r.id.length > 0 &&
      typeof r.name_zh === "string" && r.name_zh.length > 0 &&
      typeof r.desc_zh === "string" && r.desc_zh.length > 0 &&
      typeof r.icon === "string" && r.icon.length > 0 &&
      typeof r.earned === "boolean"
    ),
    JSON.stringify(apiRows)
  );
  check("無 player 查詢時全部未達成", Array.isArray(apiRows) && apiRows.every((r) => r.earned === false));

  // ── 2) 真的點擊 🏅 按鈕開面板，檢查真實渲染 ──
  const clicked = await page.evaluate(() => {
    const btn = document.getElementById("milestonesBtn");
    if (!btn) return false;
    btn.dispatchEvent(new PointerEvent("click", { bubbles: true, cancelable: true }));
    return true;
  });
  await sleep(500);
  const visibleAfterClick = await page.evaluate(() => window.__voxel.milestonesVisible);
  check("真的點擊 🏅 按鈕 → 面板開啟", clicked && visibleAfterClick);
  await page.screenshot({ path: join(OUT_DIR, "miles-1-opened.png") });

  const liveRowCount = await page.evaluate(() =>
    document.querySelectorAll("#milestonesBody .miles-row").length
  );
  check("面板真的渲染出 8 列（讀真實 API 回應）", liveRowCount === 8, `liveRowCount=${liveRowCount}`);
  const progressText = await page.evaluate(() =>
    document.querySelector("#milestonesBody .miles-progress")?.textContent || ""
  );
  check("進度文字顯示「已達成 0 / 8」（世界剛連上，這位玩家還沒達成任何項目）",
    progressText.includes("0") && progressText.includes("8"), progressText);

  // ── 3) 真的挖一塊方塊，觸發「初次採集」──
  // 比照 voxel-edit-qa.mjs：不只要「對到方塊」，還要確認放置點不在自己身體裡
  // （否則挖開後緊接著的 place 會被 placeAtTarget() 的防卡死防呆靜默忽略）。
  // 這支腳本挖完後還要再瞄一次找新目標，每次呼叫前先把 camPitch 重設回基準值
  // （setCamPitch，繞過滑鼠直接歸零），避免兩次呼叫的下壓角度疊加。
  // **鎖紀律（滑鼠版）**：一旦瀏覽器真的進入 pointer lock，movementY 是相對「上一次
  // 送出的座標」算出來的差值——如果每輪都先歸位到同一起點 (640,360) 再拖到同一終點
  // (640,410)，兩輪之間的位移會完全抵銷成淨零，pitch 永遠卡住不動（挖完後第二次呼叫
  // 曾因此死鎖，實測抓出）。改成「持續往下拖、不歸位」，每輪都是全新的正向位移。
  async function aimDownAndTarget() {
    await page.evaluate(() => window.__voxel.setCamPitch(0.35));
    await page.mouse.move(640, 360);
    await page.mouse.down();
    let y = 360;
    let found = false;
    for (let i = 0; i < 16; i++) {
      y += 20;
      await page.mouse.move(640, y, { steps: 2 });
      await sleep(250);
      const ok = await page.evaluate(() => {
        const v = window.__voxel, t = v.target, p = v.player;
        if (!t) return false;
        const px = t.bx + t.nx, py = t.by + t.ny, pz = t.bz + t.nz;
        const inBody = px === Math.floor(p.x) && pz === Math.floor(p.z) &&
          (py === Math.floor(p.y) || py === Math.floor(p.y + 1));
        return !inBody;
      });
      if (ok) { found = true; break; }
    }
    await page.mouse.up();
    return found;
  }
  const aimed = await aimDownAndTarget();
  check("準心對到方塊（地形已載入，放置點不在自己身體裡）", aimed);

  const myName = await page.evaluate(() => window.__voxel.myName);
  const bpos = await page.evaluate(() => window.__voxel.doBreak());
  await sleep(800);
  check("真的送出 break 訊息（有回傳座標）", !!bpos, JSON.stringify(bpos));

  const rowsAfterMine = await page.evaluate(async (player) => {
    const resp = await fetch(`/voxel/milestones?player=${encodeURIComponent(player)}`);
    return resp.ok ? await resp.json() : null;
  }, myName);
  const mineEarned = rowsAfterMine?.find((r) => r.id === "first_mine")?.earned === true;
  check("挖礦成功後「初次採集」真的解鎖（後端持久化查得到）", mineEarned, JSON.stringify(rowsAfterMine));

  const toastAfterMine = await page.evaluate(() => document.getElementById("msg")?.textContent || "");
  check("挖礦成功彈出「成就達成：初次採集」慶祝提示", toastAfterMine.includes("初次採集"), toastAfterMine);

  // ── 4) 真的放一塊方塊，觸發「初次建造」──
  // 挑剛挖到、背包裡真的有的方塊種類選進快捷欄（比照 voxel-edit-qa.mjs 手法），
  // 避免 selectedBlock() 還停在空氣格導致 placeAtTarget() 靜默略過。
  const minedBlockId = await page.evaluate(() => {
    const inv = window.__voxel.myInv;
    const ids = Object.keys(inv).map(Number).filter((id) => inv[id] > 0);
    return ids.length > 0 ? ids[0] : null;
  });
  check("挖礦後背包真的有材料可選", minedBlockId !== null, `minedBlockId=${minedBlockId}`);
  if (minedBlockId !== null) {
    await page.evaluate((b) => window.__voxel.selectSlotByBlock(b), minedBlockId);
    await sleep(200);
  }
  // 注意：原本瞄準的方塊若在觸及距離（RAY_MAX=6）邊緣被挖空，射線會繼續往前，
  // 深處若超出觸及距離就會落空（target=null）——這是正常的挖礦手感，不是
  // bug。比照 voxel-edit-qa.mjs 精神：重新拖曳瞄準，找一個「當下真的能放」的
  // 新目標（多半就是剛挖出的洞口本身或緊鄰的方塊）。
  const reaimed = await aimDownAndTarget();
  check("挖出後重新瞄準仍能找到可放置的目標", reaimed);
  const ppos = await page.evaluate(() => window.__voxel.doPlace());
  await sleep(800);
  check("真的送出 place 訊息（有回傳座標）", !!ppos, JSON.stringify(ppos));

  const rowsAfterPlace = await page.evaluate(async (player) => {
    const resp = await fetch(`/voxel/milestones?player=${encodeURIComponent(player)}`);
    return resp.ok ? await resp.json() : null;
  }, myName);
  const placeEarned = rowsAfterPlace?.find((r) => r.id === "first_place")?.earned === true;
  check("放置成功後「初次建造」真的解鎖（後端持久化查得到）", placeEarned, JSON.stringify(rowsAfterPlace));

  await page.screenshot({ path: join(OUT_DIR, "miles-2-after-mine-place.png") });

  // ── 5) 合成資料驗證已達成/未達成的視覺區別 ──
  const rendered = await page.evaluate(() => {
    const rows = [
      { id: "a", name_zh: "已達成項目", desc_zh: "測試用", icon: "✅", earned: true },
      { id: "b", name_zh: "未達成項目", desc_zh: "測試用", icon: "🔒", earned: false },
    ];
    window.__voxel.renderMilestonesPanel(rows);
    return [...document.querySelectorAll("#milestonesBody .miles-row")].map((el) => ({
      name: el.querySelector(".miles-name")?.textContent || "",
      earnedClass: el.classList.contains("miles-earned"),
      lockedClass: el.classList.contains("miles-locked"),
    }));
  });
  check("合成資料渲染 2 列", rendered.length === 2, JSON.stringify(rendered));
  check("已達成列套用 miles-earned（非灰階鎖定）",
    rendered[0]?.name === "已達成項目" && rendered[0]?.earnedClass && !rendered[0]?.lockedClass,
    JSON.stringify(rendered[0]));
  check("未達成列套用 miles-locked（灰階鎖定）",
    rendered[1]?.name === "未達成項目" && rendered[1]?.lockedClass && !rendered[1]?.earnedClass,
    JSON.stringify(rendered[1]));

  // ── 6) 關閉面板 ──
  const closedOk = await page.evaluate(() => {
    window.__voxel.closeMilestones();
    return !window.__voxel.milestonesVisible;
  });
  check("關閉里程碑面板", closedOk);

  const pass = fails.length === 0;
  console.log("\n──────── 玩家里程碑 v1 QA 報告 ────────");
  console.log(pass
    ? "判定: PASS ✅（API 8 枚里程碑欄位齊全、真點擊開面板、真實挖礦/放置觸發解鎖+慶祝提示、已達成/未達成視覺正確）"
    : `判定: CHECK ⚠️ 失敗項目: ${fails.join("、")}`);
  if (logs.length) console.log("頁面訊息(節錄):\n" + logs.slice(0, 20).join("\n"));
  writeFileSync(join(OUT_DIR, "voxel-milestones-qa.json"),
    JSON.stringify({ fails, apiRows, liveRowCount, rowsAfterMine, rowsAfterPlace, rendered, pass }, null, 2));

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
