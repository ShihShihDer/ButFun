// ============================================================
// voxel-second-crop-qa.mjs — 第二種作物 v1（胡蘿蔔）真實瀏覽器全流程 QA
// ============================================================
// 緣起：review 對 PR#926 提出質疑——HOTBAR_DEFAULT 固定陣列沒放
// CARROT_SEEDS/CARROT，擔心玩家「永遠選不到那一格」。但 HOTBAR 其實是動態的
// （main.js:1382 assignToHotbar），背包面板點物品格（.bag-inv-slot pointerdown，
// main.js:2566）就是呼叫它、指派進「目前選中的快捷欄格」；WHEAT 的 SEEDS 也從沒進過
// HOTBAR_DEFAULT，一路都靠這條路徑被玩家選中。這支腳本用真瀏覽器、真伺服器、
// 真背包庫存（不作弊注入）走一次完整流程，對背包/合成結果格「真的 dispatch
// PointerEvent」（不是直接呼叫內部函式），貼近玩家真實點擊路徑：
//   掃描附近地形找一塊平坦草地 →（用既有 setPlayerPos/setYaw/setCamPitch QA 鉤子
//   精準瞄準，避開海邊/斜坡等不平整地形）→ 挖草地拿胡蘿蔔種子 → 挖 2 層泥土 →
//   （背包點擊）合成農田土 →（背包點擊）指派農田土上快捷欄 → 放置農田土 →
//   （背包點擊）指派胡蘿蔔種子上快捷欄 → 種下 → 等成熟 → 收成
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

  // GRASS/DIRT 沒有透過 window.__voxel 暴露（現有 QA 慣例：voxel-edit-qa.mjs 對 STONE 也是
  // 直接寫死數字），對齊 src/voxel.rs 的 Block enum：Grass=1、Dirt=2。
  const GRASS = 1, DIRT = 2;
  const consts = await page.evaluate(() => {
    const v = window.__voxel;
    return { FARM_SOIL: v.FARM_SOIL, CARROT_SEEDS: v.CARROT_SEEDS, CARROT: v.CARROT };
  });
  consts.DIRT = DIRT;
  const CARROT_SEEDED_ID = 46, CARROT_MATURE_ID = 47;

  // ── 掃描附近找一塊「平坦草地」（草→土→土，非斜坡/懸崖）：直接讀已載入 chunk 資料，
  //    不靠滑鼠拖曳瞎猜，避免海邊沙地/地形起伏讓準心亂飄。──
  const flatGrass = await page.evaluate(([GRASS, DIRT]) => {
    const v = window.__voxel;
    const p = v.player;
    const px = Math.floor(p.x), pz = Math.floor(p.z), py = Math.floor(p.y);
    for (let r = 2; r <= 24; r++) {
      for (let dz = -r; dz <= r; dz++) {
        for (let dx = -r; dx <= r; dx++) {
          if (Math.max(Math.abs(dx), Math.abs(dz)) !== r) continue; // 只掃這一圈的邊界，由近到遠
          for (let dy = 6; dy >= -6; dy--) {
            const y = py + dy;
            if (v.getBlock(px + dx, y, pz + dz) !== GRASS) continue;
            if (v.getBlock(px + dx, y - 1, pz + dz) === DIRT && v.getBlock(px + dx, y - 2, pz + dz) === DIRT) {
              return { x: px + dx, y, z: pz + dz };
            }
          }
        }
      }
    }
    return null;
  }, [GRASS, DIRT]);
  if (!check("找到平坦草地方塊（草/土/土）", !!flatGrass, JSON.stringify(flatGrass))) {
    await browser.close(); process.exit(1);
  }

  /**
   * 解析式瞄準：站在目標正上方同一豎井裡（已挖開的欄位，不靠遠處斜看避開鄰近地形），
   * 水平只偏移一點點（0.35，仍在同一格內）讓俯角落在第一人稱夾角範圍內（不撞 ±83° 萬向鎖），
   * 幾乎垂直往下看。camPitch 正=往下看（main.js:1845 註解），反解時對齊 viewDir() 的
   * dir=(-sin(yaw)cosθ,-sinθ,-cos(yaw)cosθ) 慣例：pitch=asin(-uy)、yaw=atan2(-ux,-uz)。
   */
  async function aimAtBlock(bx, by, bz) {
    const off = 0.35;
    const px = bx + 0.5 + off, py = by + 1, pz = bz + 0.5; // 腳站在目標正上方一格（同豎井已挖開）
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
    // 至少等一個 rAF 讓 updateTarget() 依新 yaw/pitch 重算 target（target 只在渲染迴圈內更新）。
    await sleep(200);
    return page.evaluate(() => window.__voxel.target);
  }
  async function myInv() { return page.evaluate(() => window.__voxel.myInv); }
  async function blockAt(pos) {
    return pos ? page.evaluate(([x, y, z]) => window.__voxel.getBlock(x, y, z), [pos.x, pos.y, pos.z]) : null;
  }
  // 真的 dispatch PointerEvent（不是直呼叫內部函式）——貼近玩家真實點擊路徑。
  async function clickBagItemByName(name) {
    return page.evaluate((name) => {
      const slots = [...document.querySelectorAll("#bagInvGrid .bag-inv-slot")];
      const el = slots.find((s) => s.querySelector(".bag-inv-name")?.textContent === name);
      if (!el) return false;
      el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true }));
      return true;
    }, name);
  }
  async function clickBagResultSlot() {
    return page.evaluate(() => {
      const el = document.getElementById("bagResultSlot");
      if (!el) return false;
      el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true }));
      return true;
    });
  }

  // 準心真的對到「意圖瞄準的那一格」，不只是「有對到某一格」（避免鄰近地形誤中）。
  function targetIs(tgt, bx, by, bz) {
    return !!tgt && tgt.bx === bx && tgt.by === by && tgt.bz === bz;
  }

  await page.screenshot({ path: join(OUT_DIR, "carrot-0-start.png") });

  // ── 1) 挖草地（額外掉落胡蘿蔔種子，voxel_ws.rs:1154）──
  const grassTgt = await aimAtBlock(flatGrass.x, flatGrass.y, flatGrass.z);
  check("準心鎖定草地方塊", targetIs(grassTgt, flatGrass.x, flatGrass.y, flatGrass.z), JSON.stringify(grassTgt));
  const grassPos = await page.evaluate(() => window.__voxel.doBreak());
  await sleep(900);
  const afterGrass = await blockAt(grassPos);
  check("挖草地", !!grassPos && afterGrass === 0, `→ ${afterGrass}`);

  // ── 2) 挖 2 層泥土（草已清空，站在剛挖開的豎井裡往下看；每層重新精準瞄準）──
  const dirt1Tgt = await aimAtBlock(flatGrass.x, flatGrass.y - 1, flatGrass.z);
  check("準心鎖定泥土#1", targetIs(dirt1Tgt, flatGrass.x, flatGrass.y - 1, flatGrass.z), JSON.stringify(dirt1Tgt));
  const dirtBreak1 = await page.evaluate(() => window.__voxel.doBreak());
  await sleep(900);
  check("挖泥土#1", !!dirtBreak1 && (await blockAt(dirtBreak1)) === 0);

  const dirt2Tgt = await aimAtBlock(flatGrass.x, flatGrass.y - 2, flatGrass.z);
  check("準心鎖定泥土#2", targetIs(dirt2Tgt, flatGrass.x, flatGrass.y - 2, flatGrass.z), JSON.stringify(dirt2Tgt));
  const dirtBreak2 = await page.evaluate(() => window.__voxel.doBreak());
  await sleep(900);
  check("挖泥土#2", !!dirtBreak2 && (await blockAt(dirtBreak2)) === 0);

  const invAfterMining = await myInv();
  console.log("挖礦後背包:", JSON.stringify(invAfterMining));
  check("real 背包有 ≥2 泥土", (invAfterMining[consts.DIRT] || 0) >= 2);
  check("real 背包有 ≥1 胡蘿蔔種子（挖草地掉落）", (invAfterMining[consts.CARROT_SEEDS] || 0) >= 1);

  // ── 3) 背包合成：2 泥土 → 農田土（「till」配方）──
  await page.evaluate(() => window.__voxel.openBagPanel());
  await page.evaluate((DIRT) => window.__voxel.setBagGrid([DIRT, DIRT, 0, 0]), consts.DIRT);
  const match = await page.evaluate(() => window.__voxel.matchBagRecipe());
  check("背包 2 泥土湊出「till」配方且材料足夠", !!match && match.recipe.id === "till" && match.canCraft,
    JSON.stringify(match));
  await clickBagResultSlot();
  await sleep(900);
  const invAfterCraft = await myInv();
  check("真的點擊合成結果格 → real 背包拿到農田土", (invAfterCraft[consts.FARM_SOIL] || 0) >= 1,
    JSON.stringify(invAfterCraft));

  // ── 4) 背包點擊「農田土」→ 指派進目前選中的快捷欄格（真實玩家操作路徑）──
  const slotBefore = await page.evaluate(() => window.__voxel.selectedSlot);
  const clickedSoil = await clickBagItemByName("農田土");
  await sleep(200);
  const hotbarAfterSoil = await page.evaluate(() => window.__voxel.HOTBAR);
  check("真的點擊背包「農田土」→ 指派上快捷欄", clickedSoil && hotbarAfterSoil[slotBefore] === consts.FARM_SOIL,
    `slot ${slotBefore} = ${hotbarAfterSoil[slotBefore]}`);
  const selectedAfterSoil = await page.evaluate(() => window.__voxel.HOTBAR[window.__voxel.selectedSlot]);
  check("selectedBlock() 真的變成農田土（玩家選得到）", selectedAfterSoil === consts.FARM_SOIL);

  // ── 5) 放置農田土（瞄準洞底剩下的第 3 層泥土上表面，回填成農田土）──
  const floorTgt = await aimAtBlock(flatGrass.x, flatGrass.y - 3, flatGrass.z);
  check("準心鎖定洞底泥土（放置基準面）",
    targetIs(floorTgt, flatGrass.x, flatGrass.y - 3, flatGrass.z), JSON.stringify(floorTgt));
  const soilPos = await page.evaluate(() => window.__voxel.doPlace());
  await sleep(900);
  const soilBlock = await blockAt(soilPos);
  const soilOk = check("放置農田土", !!soilPos && soilBlock === consts.FARM_SOIL, `→ ${soilBlock}`);
  await page.screenshot({ path: join(OUT_DIR, "carrot-1-farmsoil-placed.png") });

  let selectOk = false, plantOk = false, matured = false, matureBlock = null;
  let harvestOk = false, invAfterHarvest = {};

  if (soilOk) {
    // ── 6) 背包點擊「胡蘿蔔種子」→ 指派進目前選中的快捷欄格 ──
    const clickedSeed = await clickBagItemByName("胡蘿蔔種子");
    await sleep(200);
    const selectedAfterSeed = await page.evaluate(() => window.__voxel.HOTBAR[window.__voxel.selectedSlot]);
    selectOk = check("真的點擊背包「胡蘿蔔種子」→ selectedBlock() 變成 CARROT_SEEDS（review 疑慮驗證）",
      clickedSeed && selectedAfterSeed === consts.CARROT_SEEDS, `selectedBlock()=${selectedAfterSeed}`);

    // ── 7) 種下胡蘿蔔種子（瞄準剛放的農田土本身）──
    const seedTgt = await aimAtBlock(soilPos.x, soilPos.y, soilPos.z);
    check("準心鎖定農田土本身", targetIs(seedTgt, soilPos.x, soilPos.y, soilPos.z), JSON.stringify(seedTgt));
    const plantPos = await page.evaluate(() => window.__voxel.doPlace());
    await sleep(900);
    const plantedBlock = await blockAt(plantPos);
    plantOk = check("種下胡蘿蔔種子", !!plantPos && plantedBlock === CARROT_SEEDED_ID, `→ ${plantedBlock}`);
    await page.screenshot({ path: join(OUT_DIR, "carrot-2-planted.png") });

    // ── 8) 等成熟（60 秒；tick_farm 每 15 秒才擲骰檢查一次，保守多等一輪到 80 秒）──
    if (plantOk) {
      console.log("等待胡蘿蔔成熟（最多 80 秒）...");
      for (let waited = 0; waited < 80000; waited += 5000) {
        await sleep(5000);
        matureBlock = await blockAt(plantPos);
        if (matureBlock === CARROT_MATURE_ID) { matured = true; break; }
      }
      check("胡蘿蔔真的長成熟", matured, `block=${matureBlock}`);
      await page.screenshot({ path: join(OUT_DIR, "carrot-3-mature.png") });

      // ── 9) 收成 ──
      if (matured) {
        const hpos = await page.evaluate(() => window.__voxel.doBreak());
        await sleep(900);
        invAfterHarvest = await myInv();
        const gotSeeds = (invAfterHarvest[consts.CARROT_SEEDS] || 0) > 0;
        const gotCarrot = (invAfterHarvest[consts.CARROT] || 0) > 0;
        harvestOk = check("收成拿到胡蘿蔔種子+胡蘿蔔", !!hpos && gotSeeds && gotCarrot, JSON.stringify(invAfterHarvest));
      }
      await page.screenshot({ path: join(OUT_DIR, "carrot-4-harvested.png") });
    }
  }

  const pass = fails.length === 0;
  console.log("\n──────── 第二種作物（胡蘿蔔）全流程 QA 報告 ────────");
  console.log(pass ? "判定: PASS ✅（真背包+真點擊：選種→種下→成熟→收成全流程真的能玩）"
    : `判定: CHECK ⚠️ 失敗項目: ${fails.join("、")}`);
  if (logs.length) console.log("頁面訊息(節錄):\n" + logs.slice(0, 20).join("\n"));
  writeFileSync(join(OUT_DIR, "voxel-second-crop-qa.json"),
    JSON.stringify({ fails, invAfterHarvest, pass }, null, 2));

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
