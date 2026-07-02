// ============================================================
// voxel-furnace-grid-qa.mjs — 熔爐冶煉格子化 v1 真實瀏覽器全流程 QA
// ============================================================
// 緣起：熔爐面板 v1（ROADMAP 666）是「配方清單+冶煉按鈕」，跟背包 2×2（664）/
// 工作台 3×3（665）「點物品拿起→點格子放入」的格子式手感不一致。本切片
// （ROADMAP 712）把熔爐面板改成同一套格子互動，這支腳本用真瀏覽器、真伺服器、
// 真背包庫存（挖礦拿真石頭，不作弊注入）走一次完整流程，對物品欄/冶煉格/
// 結果格「真的 dispatch PointerEvent」（不是直接呼叫內部函式），貼近玩家真實
// 點擊路徑：挖石頭 ×3 →（熔爐面板點擊）把 3 顆石頭分別放進 3 格冶煉格 →
// 結果格亮起「拋光石」→ 點結果格 → 真的冶煉成功、背包拿到拋光石。
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

  const STONE = 3;
  const consts = await page.evaluate(() => ({ SMOOTH_STONE: window.__voxel.SMOOTH_STONE }));

  // ── 瞄準指定世界座標的方塊：把玩家瞬移到正上方、視線朝下鎖定該方塊（比照既有
  //    voxel-second-crop-qa.mjs 手法，逐格垂直下鑽時每層都重新精準瞄準）。
  //    offX/offZ：玩家站位相對方塊中心的水平偏移，用來避開「準心視線切過鄰欄邊界」──
  //    相鄰欄的草/土/石層厚度不一定完全一致，即使表面同高，往下鑽到石層時偏移
  //    若剛好朝向較高的鄰欄，視線會被那面牆攔截、raycast 永遠鎖定錯誤座標
  //    （實測復現：西鄰石層起點比目標欄高 1 格，固定 off=0.35 朝西直接卡死整欄）。──
  async function aimAtBlock(bx, by, bz, offX = 0.15, offZ = 0) {
    const px = bx + 0.5 + offX, py = by + 1, pz = bz + 0.5 + offZ;
    await page.evaluate(([px, py, pz, bx, by, bz]) => {
      const v = window.__voxel;
      v.setPlayerPos(px, py, pz);
      const originY = py + 1.5;
      let ux = (bx + 0.5) - px, uy = (by + 0.5) - originY, uz = (bz + 0.5) - pz;
      const len = Math.sqrt(ux * ux + uy * uy + uz * uz);
      ux /= len; uy /= len; uz /= len;
      const yaw = Math.atan2(-ux, -uz);
      const pitch = Math.asin(Math.max(-1, Math.min(1, -uy)));
      v.setYaw(yaw);
      v.setCamPitch(pitch);
    }, [px, py, pz, bx, by, bz]);
    await sleep(200);
    return page.evaluate(() => window.__voxel.target);
  }
  // 依序試幾組水平偏移，命中哪組就用哪組（東西南北四面都試過才放棄這一格）。
  // 偏移量必須小於「0.5 - 角色半寬(PW=0.3)」＝0.2，否則玩家 AABB 會跨出目標欄位邊界，
  // 一旦鄰欄地形較高就會被 unstuckIfNeeded 頂飛到鄰欄地表（實測復現：off=0.35 時
  // 0.35+0.3=0.65>0.5，跨出邊界，導致瞄準深層方塊時整層 target 鎖死在鄰欄）。
  const AIM_OFFSETS = [[0.15, 0], [-0.15, 0], [0, 0.15], [0, -0.15]];
  async function aimAtBlockRetry(bx, by, bz) {
    for (const [offX, offZ] of AIM_OFFSETS) {
      const tgt = await aimAtBlock(bx, by, bz, offX, offZ);
      if (targetIs(tgt, bx, by, bz)) return tgt;
    }
    return null;
  }
  async function myInv() { return page.evaluate(() => window.__voxel.myInv); }
  function targetIs(tgt, bx, by, bz) {
    return !!tgt && tgt.bx === bx && tgt.by === by && tgt.bz === bz;
  }

  // ── 1) 挖石頭 ×3（垂直下鑽同一欄，草/土層挖過即穿透見石層，沿用既有手法）──
  // 稍微遠離出生點再找欄（出生點附近可能已有居民蓋家/種田留下的 delta，非天然地形，
  // 石頭密度不可預期）；只移 48 格（≈3 個 chunk）留在既有串流半徑內，避免跳太遠
  // 導致新區塊來不及載入、getBlock 一路回 0 誤判成空氣（曾在此踩過雷）。
  await page.evaluate(() => {
    const v = window.__voxel;
    v.setPlayerPos(v.player.x + 48, v.player.y, v.player.z + 48);
  });
  await sleep(6000); // 等新區塊 chunk 串流進來

  // 盲目瞬移後直接原地下鑽曾踩雷：即使表面同高，鄰欄的草/土/石分層厚度仍可能
  // 差 1 格（例如鄰欄石層起點比目標欄高或低 1 格）——瞄準向下的視線帶有水平偏移
  // （見 aimAtBlock 的 off），偏移一旦朝向那個「厚度不同」的鄰欄，就會被鄰欄
  // 攔截或穿越到鄰欄的空隙裡，raycast 永遠鎖定錯誤座標或直接落空（實測復現兩種
  // 症狀都發生過）。改成要求東西南北四鄰欄「逐層完全同值」（不只是表面同高，
  // 往下 12 格內每一層的方塊 id 都跟中心欄一致），才能保證瞄準時的水平偏移量
  // （最大 0.15 格，遠小於角色半寬邊界 0.2）無論偏向哪個方向都不會咬到分層差異；
  // 並要求往下 20 格內至少有 3 顆石頭，避開沙漠/水底這類淺層無石的地形。
  const col = await page.evaluate(() => {
    const v = window.__voxel;
    const p = v.player;
    const px = Math.floor(p.x), pz = Math.floor(p.z);
    function topY(x, z) {
      for (let y = 20; y >= -20; y--) {
        const b = v.getBlock(x, y, z);
        if (b > 0) return y;
        if (b < 0) return null; // 未載入的鄰居——這個候選欄先跳過
      }
      return null;
    }
    function columnMatches(cx, cz, nx, nz, topOfCenter) {
      for (let dy2 = 0; dy2 < 12; dy2++) {
        const y = topOfCenter - dy2;
        if (v.getBlock(nx, y, nz) !== v.getBlock(cx, y, cz)) return false;
      }
      return true;
    }
    for (let r = 1; r <= 40; r++) {
      for (let dz = -r; dz <= r; dz++) {
        for (let dx = -r; dx <= r; dx++) {
          if (Math.max(Math.abs(dx), Math.abs(dz)) !== r) continue;
          const x = px + dx, z = pz + dz;
          const cy = topY(x, z);
          if (cy === null) continue;
          const wy = topY(x - 1, z), ey = topY(x + 1, z), ny = topY(x, z - 1), sy = topY(x, z + 1);
          if (wy !== cy || ey !== cy || ny !== cy || sy !== cy) continue;
          if (!columnMatches(x, z, x - 1, z, cy) || !columnMatches(x, z, x + 1, z, cy)
            || !columnMatches(x, z, x, z - 1, cy) || !columnMatches(x, z, x, z + 1, cy)) continue;
          let stoneBelow = 0;
          for (let dy2 = 0; dy2 < 20; dy2++) {
            if (v.getBlock(x, cy - dy2, z) === 3) stoneBelow++;
          }
          if (stoneBelow < 3) continue;
          return { x, y: cy, z };
        }
      }
    }
    return null;
  });
  if (!check("找到四鄰同高＋往下有石層的欄（避開窪地/裂縫）", !!col, JSON.stringify(col))) {
    await browser.close(); process.exit(1);
  }
  const start = { x: col.x, y: col.y + 1, z: col.z };
  let stoneCount = 0;
  for (let dy = 0; dy >= -30 && stoneCount < 3; dy--) {
    const y = start.y + dy;
    const before = await page.evaluate(([x, y, z]) => window.__voxel.getBlock(x, y, z), [start.x, y, start.z]);
    if (before <= 0) continue; // 空氣(0)或未載入(-1)跳過
    const tgt = await aimAtBlockRetry(start.x, y, start.z);
    if (!targetIs(tgt, start.x, y, start.z)) continue;
    await page.evaluate(() => window.__voxel.doBreak());
    await sleep(700);
    if (before === STONE) stoneCount++;
  }
  const invAfterMining = await myInv();
  check("垂直下鑽真的挖到 ≥3 顆石頭", (invAfterMining[STONE] || 0) >= 3, JSON.stringify(invAfterMining));

  await page.screenshot({ path: join(OUT_DIR, "furnace-0-mined.png") });

  // ── 2) 開熔爐面板（不必真的放置熔爐方塊——面板本身不驗證是否靠近熔爐，
  //    冶煉請求由伺服器 Craft handler 統一驗證材料，與是否放了熔爐世界方塊無關；
  //    這與既有 wbBtn/bagBtn 直接開面板走的是同一條 QA 路徑）──
  await page.evaluate(() => window.__voxel.openFurnacePanel());
  await sleep(300);
  check("熔爐面板真的開啟", await page.evaluate(() => window.__voxel.furnacePanelVisible));

  // ── 3) 真的 dispatch PointerEvent 點物品欄「石頭」×3 次，每次放進一格冶煉格 ──
  async function clickFurnaceInvStone() {
    return page.evaluate((name) => {
      const slots = [...document.querySelectorAll("#furnaceInvGrid .bag-inv-slot")];
      const el = slots.find((s) => s.querySelector(".bag-inv-name")?.textContent === name);
      if (!el) return false;
      el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true }));
      return true;
    }, "石");
  }
  async function clickFurnaceGridSlot(i) {
    return page.evaluate((i) => {
      const slots = [...document.querySelectorAll("#furnaceGrid2 .furnace-grid-slot")];
      const el = slots[i];
      if (!el) return false;
      el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true }));
      return true;
    }, i);
  }
  for (let i = 0; i < 3; i++) {
    const pickedOk = check(`第 ${i + 1} 次點物品欄「石頭」`, await clickFurnaceInvStone());
    await sleep(100);
    check(`第 ${i + 1} 次點冶煉格 #${i} 放入`, pickedOk && await clickFurnaceGridSlot(i));
    await sleep(100);
  }
  const gridAfterPlace = await page.evaluate(() => window.__voxel.furnaceGrid);
  check("3 格冶煉格真的都填滿石頭（真點擊，非注入）",
    gridAfterPlace.length === 3 && gridAfterPlace.every((b) => b === STONE), JSON.stringify(gridAfterPlace));

  await page.screenshot({ path: join(OUT_DIR, "furnace-1-grid-filled.png") });

  // ── 4) 結果格應顯示「拋光石」配方且材料足夠（純函式 matchFurnaceRecipe，非猜測）──
  const match = await page.evaluate(() => window.__voxel.matchFurnaceRecipe());
  check("matchFurnaceRecipe 湊出 smelt_stone 且材料足夠", !!match && match.recipe.id === "smelt_stone" && match.canCraft,
    JSON.stringify(match));
  const resultHasClass = await page.evaluate(() => {
    const el = document.getElementById("furnaceResultSlot");
    return el ? el.classList.contains("has-result") : false;
  });
  check("結果格 DOM 真的亮起 has-result", resultHasClass);

  // ── 5) 真的 dispatch PointerEvent 點結果格 → 送出冶煉請求 ──
  const invBeforeSmelt = await myInv();
  await page.evaluate(() => {
    document.getElementById("furnaceResultSlot")
      .dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true }));
  });
  await sleep(900);
  const invAfterSmelt = await myInv();
  check("真的點擊結果格 → real 背包扣掉 3 顆石頭",
    (invAfterSmelt[STONE] || 0) === (invBeforeSmelt[STONE] || 0) - 3, JSON.stringify(invAfterSmelt));
  check("真的點擊結果格 → real 背包拿到拋光石 ×3",
    (invAfterSmelt[consts.SMOOTH_STONE] || 0) >= 3, JSON.stringify(invAfterSmelt));

  await page.screenshot({ path: join(OUT_DIR, "furnace-2-smelted.png") });

  // ── 6) 清除冶煉格按鈕真的清空（真點擊）──
  await page.evaluate((STONE) => window.__voxel.setFurnaceGrid([STONE, STONE, STONE]), STONE);
  await page.evaluate(() => {
    document.getElementById("furnaceClearBtn")
      .dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true }));
  });
  await sleep(200);
  const gridAfterClear = await page.evaluate(() => window.__voxel.furnaceGrid);
  check("清除鈕真的清空冶煉格", gridAfterClear.every((b) => b === 0), JSON.stringify(gridAfterClear));

  const pass = fails.length === 0;
  console.log("\n──────── 熔爐冶煉格子化 v1 QA 報告 ────────");
  console.log(pass ? "判定: PASS ✅（真挖礦+真點擊：拿起石頭→填滿冶煉格→冶煉→拿到拋光石全流程真的能玩）"
    : `判定: CHECK ⚠️ 失敗項目: ${fails.join("、")}`);
  if (logs.length) console.log("頁面訊息(節錄):\n" + logs.slice(0, 20).join("\n"));
  writeFileSync(join(OUT_DIR, "voxel-furnace-grid-qa.json"),
    JSON.stringify({ fails, invAfterSmelt, pass }, null, 2));

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
