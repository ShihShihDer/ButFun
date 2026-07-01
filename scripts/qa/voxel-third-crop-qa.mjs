// ============================================================
// voxel-third-crop-qa.mjs — 第三種作物 v1（馬鈴薯）真實瀏覽器全流程 QA
// ============================================================
// 沿用第二種作物（胡蘿蔔，voxel-second-crop-qa.mjs）驗證過的手法：真瀏覽器、
// 真伺服器、真背包庫存（不作弊注入），對背包格「真的 dispatch PointerEvent」
// （不是直接呼叫內部函式），貼近玩家真實點擊路徑：
//   掃描附近地形找一塊平坦草地 → 挖草地 → 挖 2 層泥土（額外掉落馬鈴薯種子，
//   與胡蘿蔔種子取自草地區隔）→（背包點擊）合成農田土 →（背包點擊）指派
//   農田土上快捷欄 → 放置農田土 →（背包點擊）指派馬鈴薯種子上快捷欄 →
//   種下 → 等成熟（120 秒，比胡蘿蔔慢）→ 收成（驗證收成量 ×2，量大是特色）
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

  const GRASS = 1, DIRT = 2;
  const consts = await page.evaluate(() => {
    const v = window.__voxel;
    return { FARM_SOIL: v.FARM_SOIL, POTATO_SEEDS: v.POTATO_SEEDS, POTATO: v.POTATO };
  });
  consts.DIRT = DIRT;
  const POTATO_SEEDED_ID = 50, POTATO_MATURE_ID = 51;

  // ── 掃描附近找一塊「平坦草地」（草→土→土，非斜坡/懸崖）──
  const flatGrass = await page.evaluate(([GRASS, DIRT]) => {
    const v = window.__voxel;
    const p = v.player;
    const px = Math.floor(p.x), pz = Math.floor(p.z), py = Math.floor(p.y);
    for (let r = 2; r <= 24; r++) {
      for (let dz = -r; dz <= r; dz++) {
        for (let dx = -r; dx <= r; dx++) {
          if (Math.max(Math.abs(dx), Math.abs(dz)) !== r) continue;
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

  async function aimAtBlock(bx, by, bz) {
    const off = 0.35;
    const px = bx + 0.5 + off, py = by + 1, pz = bz + 0.5;
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
  async function myInv() { return page.evaluate(() => window.__voxel.myInv); }
  async function blockAt(pos) {
    return pos ? page.evaluate(([x, y, z]) => window.__voxel.getBlock(x, y, z), [pos.x, pos.y, pos.z]) : null;
  }
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

  function targetIs(tgt, bx, by, bz) {
    return !!tgt && tgt.bx === bx && tgt.by === by && tgt.bz === bz;
  }

  await page.screenshot({ path: join(OUT_DIR, "potato-0-start.png") });

  // ── 1) 挖草地（清出豎井，本身不掉馬鈴薯種子——種子來自泥土）──
  const grassTgt = await aimAtBlock(flatGrass.x, flatGrass.y, flatGrass.z);
  check("準心鎖定草地方塊", targetIs(grassTgt, flatGrass.x, flatGrass.y, flatGrass.z), JSON.stringify(grassTgt));
  const grassPos = await page.evaluate(() => window.__voxel.doBreak());
  await sleep(900);
  const afterGrass = await blockAt(grassPos);
  check("挖草地", !!grassPos && afterGrass === 0, `→ ${afterGrass}`);

  // ── 2) 挖 2 層泥土（額外掉落馬鈴薯種子，voxel_ws.rs 第三種作物 v1）──
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
  check("real 背包有 ≥1 馬鈴薯種子（挖泥土掉落）", (invAfterMining[consts.POTATO_SEEDS] || 0) >= 1);

  // ── 3) 背包合成：2 泥土 → 農田土（「till」配方；用剩下的泥土——注意上面已挖 2 層，
  //    需再挖第 3 層才夠合成用泥土，同時清出放置農田土的洞底）──
  const dirt3Tgt = await aimAtBlock(flatGrass.x, flatGrass.y - 3, flatGrass.z);
  const dirtBreak3 = await page.evaluate(() => window.__voxel.doBreak());
  await sleep(900);
  check("挖泥土#3（補足合成材料）", !!dirtBreak3);

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

  // ── 4) 背包點擊「農田土」→ 指派進目前選中的快捷欄格 ──
  const slotBefore = await page.evaluate(() => window.__voxel.selectedSlot);
  const clickedSoil = await clickBagItemByName("農田土");
  await sleep(200);
  const hotbarAfterSoil = await page.evaluate(() => window.__voxel.HOTBAR);
  check("真的點擊背包「農田土」→ 指派上快捷欄", clickedSoil && hotbarAfterSoil[slotBefore] === consts.FARM_SOIL,
    `slot ${slotBefore} = ${hotbarAfterSoil[slotBefore]}`);
  const selectedAfterSoil = await page.evaluate(() => window.__voxel.HOTBAR[window.__voxel.selectedSlot]);
  check("selectedBlock() 真的變成農田土（玩家選得到）", selectedAfterSoil === consts.FARM_SOIL);

  // ── 5) 放置農田土（瞄準洞底第 4 層泥土；洞較深，偶爾會鎖到鄰格側面而非正上方——
  //    只要「命中面 + 法向量」換算出的落點座標正確即可，不強求鎖到頂面本身）──
  const floorTgt = await aimAtBlock(flatGrass.x, flatGrass.y - 4, flatGrass.z);
  const floorLandsAt = floorTgt
    ? { x: floorTgt.bx + floorTgt.nx, y: floorTgt.by + floorTgt.ny, z: floorTgt.bz + floorTgt.nz }
    : null;
  check("準心鎖定洞底（換算落點座標正確）",
    !!floorLandsAt && floorLandsAt.x === flatGrass.x && floorLandsAt.y === flatGrass.y - 3 && floorLandsAt.z === flatGrass.z,
    JSON.stringify(floorTgt));
  const soilPos = await page.evaluate(() => window.__voxel.doPlace());
  await sleep(900);
  const soilBlock = await blockAt(soilPos);
  const soilOk = check("放置農田土", !!soilPos && soilBlock === consts.FARM_SOIL, `→ ${soilBlock}`);
  await page.screenshot({ path: join(OUT_DIR, "potato-1-farmsoil-placed.png") });

  let selectOk = false, plantOk = false, matured = false, matureBlock = null;
  let harvestOk = false, invAfterHarvest = {};

  if (soilOk) {
    // ── 6) 背包點擊「馬鈴薯種子」→ 指派進目前選中的快捷欄格 ──
    const clickedSeed = await clickBagItemByName("馬鈴薯種子");
    await sleep(200);
    const selectedAfterSeed = await page.evaluate(() => window.__voxel.HOTBAR[window.__voxel.selectedSlot]);
    selectOk = check("真的點擊背包「馬鈴薯種子」→ selectedBlock() 變成 POTATO_SEEDS",
      clickedSeed && selectedAfterSeed === consts.POTATO_SEEDS, `selectedBlock()=${selectedAfterSeed}`);

    // ── 7) 種下馬鈴薯種子（瞄準剛放的農田土本身）──
    const seedTgt = await aimAtBlock(soilPos.x, soilPos.y, soilPos.z);
    check("準心鎖定農田土本身", targetIs(seedTgt, soilPos.x, soilPos.y, soilPos.z), JSON.stringify(seedTgt));
    const plantPos = await page.evaluate(() => window.__voxel.doPlace());
    await sleep(900);
    const plantedBlock = await blockAt(plantPos);
    plantOk = check("種下馬鈴薯種子", !!plantPos && plantedBlock === POTATO_SEEDED_ID, `→ ${plantedBlock}`);
    await page.screenshot({ path: join(OUT_DIR, "potato-2-planted.png") });

    // ── 8) 等成熟（120 秒；tick_farm 每 15 秒才擲骰檢查一次，保守多等到 145 秒）──
    if (plantOk) {
      console.log("等待馬鈴薯成熟（最多 145 秒，比胡蘿蔔慢是特色）...");
      for (let waited = 0; waited < 145000; waited += 5000) {
        await sleep(5000);
        matureBlock = await blockAt(plantPos);
        if (matureBlock === POTATO_MATURE_ID) { matured = true; break; }
      }
      check("馬鈴薯真的長成熟", matured, `block=${matureBlock}`);
      await page.screenshot({ path: join(OUT_DIR, "potato-3-mature.png") });

      // ── 9) 收成（驗證量大特色：馬鈴薯 ×2）──
      if (matured) {
        const hpos = await page.evaluate(() => window.__voxel.doBreak());
        await sleep(900);
        invAfterHarvest = await myInv();
        const gotSeeds = (invAfterHarvest[consts.POTATO_SEEDS] || 0) > 0;
        const gotPotatoX2 = (invAfterHarvest[consts.POTATO] || 0) >= 2;
        harvestOk = check("收成拿到馬鈴薯種子+馬鈴薯×2（量大特色）", !!hpos && gotSeeds && gotPotatoX2,
          JSON.stringify(invAfterHarvest));
      }
      await page.screenshot({ path: join(OUT_DIR, "potato-4-harvested.png") });
    }
  }

  const pass = fails.length === 0;
  console.log("\n──────── 第三種作物（馬鈴薯）全流程 QA 報告 ────────");
  console.log(pass ? "判定: PASS ✅（真背包+真點擊：選種→種下→成熟→收成全流程真的能玩）"
    : `判定: CHECK ⚠️ 失敗項目: ${fails.join("、")}`);
  if (logs.length) console.log("頁面訊息(節錄):\n" + logs.slice(0, 20).join("\n"));
  writeFileSync(join(OUT_DIR, "voxel-third-crop-qa.json"),
    JSON.stringify({ fails, invAfterHarvest, pass }, null, 2));

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
