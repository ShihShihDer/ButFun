// ============================================================
// voxel-swim-out-qa.mjs — swim-out（麥塊式游出水）真瀏覽器物理 QA（#1200 bug 修）
// ============================================================
// 玩家實機回報「在水裡上不了地面」：陸地踏階只在 grounded 生效，游泳分支每幀強制
// grounded=false → 游泳中永遠蹬不上岸。本 QA 用 puppeteer-core 驅動系統 Chrome 載入
// /voxel/，以 _qaSetBlock 就地搭一個**確定性場景**（2 格深水池 + 1 格高岸 + 陸地台階），
// 然後用「真鍵盤輸入」驗證：
//   A. autoJump 開（預設）：水中貼岸按 W 推進 → 自動蹬上岸（最終 grounded=true 且站上岸塊）
//   B. autoJump 關：只按 W 上不去（尊重設定）；按住 跳+W 才爬上一格高的岸
//   C. 陸地無回歸：走路踏階（撞一格自動踏上）與跳躍手感照舊
// 截圖上岸前/後存 OUT_DIR。不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const URL = process.env.VQA_URL || "http://127.0.0.1:47317/voxel/?debug=1";
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const OUT_DIR = process.env.VQA_OUT || join(__dirname, "swim-out-out");
mkdirSync(OUT_DIR, { recursive: true });

const GPU_ARGS = [
  "--no-sandbox", "--disable-setuid-sandbox", "--ignore-gpu-blocklist",
  "--enable-gpu", "--enable-webgl", "--use-gl=angle", "--use-angle=gl",
  "--disable-dev-shm-usage", "--window-size=1280,800",
  "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding", "--disable-features=CalculateNativeWinOcclusion",
];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU_ARGS });
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 800 });
  page.on("pageerror", (e) => console.log("[pageerror]", e.message));

  console.log("載入", URL);
  await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await sleep(7000); // 等 chunk 載入 + mesh 建好

  // ── 就地搭確定性場景（純本地方塊，伺服器仍權威但不會主動覆蓋沒動過的 chunk）──
  // 佈局（相對玩家當前位置，保證 chunk 已載入）：
  //   x: bx0-1(池壁) | bx0..bx0+1(水池2格深) | bx0+2..bx0+5(岸,頂=H+2) | bx0+6(台階,頂=H+3) | bx0+7(擋牆)
  //   水面頂 = H+2（與岸頂同高）→ 玩家漂在水面時腳底 ≈ H+1.05，要「蹬上一格」才上岸。
  const scene = await page.evaluate(() => {
    const v = window.__voxel;
    const bx0 = Math.floor(v.player.x) + 4;
    const bz0 = Math.floor(v.player.z) - 1;
    const H = Math.floor(v.player.y);
    let fails = 0;
    const put = (x, y, z, id) => { if (v._qaSetBlock(x, y, z, id) !== id) fails++; };
    for (let x = bx0 - 1; x <= bx0 + 7; x++) {
      for (let z = bz0 - 1; z <= bz0 + 2; z++) {
        for (let y = H - 1; y <= H + 6; y++) {
          let id = 0; // 預設清空成空氣（把原生地形/樹葉都清掉，場景才確定）
          if (y === H - 1) id = v.PLANK;                                     // 池底/地基
          else if (y <= H + 1) {
            // H..H+1 這兩層：水池 or 岸/池壁
            const inPoolX = x >= bx0 && x <= bx0 + 1;
            const inPoolZ = z >= bz0 && z <= bz0 + 1;
            if (inPoolX && inPoolZ) id = v.WATER;                            // 2 格深水
            else id = v.PLANK;                                               // 池壁 + 岸（頂=H+2）
          } else if (y === H + 2 && x >= bx0 + 6) id = v.PLANK;              // 陸地台階（頂=H+3）
          else if (x === bx0 + 7 && y <= H + 5) id = v.PLANK;                // 盡頭擋牆
          put(x, y, z, id);
        }
      }
    }
    return { bx0, bz0, H, fails };
  });
  console.log("場景：", JSON.stringify(scene));
  if (scene.fails > 0) { console.log("場景擺放失敗格數 =", scene.fails, "（chunk 未載入？）放棄"); await browser.close(); process.exit(2); }
  await sleep(800); // 等 remesh

  const { bx0, bz0, H } = scene;
  const poolX = bx0 + 1.0, poolZ = bz0 + 1.0; // 水池中央（4 格水的交界，離兩側池壁最遠）
  const state = () => page.evaluate(() => {
    const v = window.__voxel, s = v.swimState;
    return { x: v.player.x, y: v.player.y, z: v.player.z, grounded: v.player.grounded,
             inWater: s.bodyInWater, vy: s.vy };
  });
  // 把玩家丟回水池中央、面向 +x（岸的方向）。fwd=(-sin yaw,0,-cos yaw) → yaw=-π/2 為 +x。
  const resetToPool = () => page.evaluate((p) => {
    const v = window.__voxel;
    v.setPlayerPos(p.x, p.y, p.z);
    v.setYaw(-Math.PI / 2);
  }, { x: poolX, y: H + 0.6, z: poolZ });

  // ── 純函式資格判定直測（前端鏡像 Rust swim_step_up_eligible）──
  const elig = await page.evaluate(() => {
    const f = window.__voxel.swimStepUpEligible;
    return {
      autoJump: f(true, true, false) === true,          // 游泳+自動跳躍 → 自動蹬
      jumpHeld: f(true, false, true) === true,          // 自動跳關+按跳 → 手動蹬
      respectOff: f(true, false, false) === false,      // 自動跳關+沒按跳 → 不蹬
      landNever: f(false, true, true) === false,        // 陸地一律 false（不碰陸地手感）
    };
  });
  console.log("資格判定純函式：", JSON.stringify(elig));

  // ── 場景 A：autoJump 開（預設），水中只按 W 貼岸推進 → 應自動蹬上岸 ──
  await page.evaluate(() => window.__voxel._qaSetAutoJump(true));
  await resetToPool();
  await page.bringToFront();
  await sleep(1200); // 先讓浮力把人托到水面漂著（最常見卡點：頭已出水、漂在水面）
  const beforeA = await state();
  await page.screenshot({ path: join(OUT_DIR, "swimout-before.png") });
  console.log("A 上岸前（水面漂浮）：", JSON.stringify(beforeA));
  await page.keyboard.down("KeyW");
  // 貼岸推進：輪詢直到蹬上岸落地（最多 8 秒），一上岸就停——截圖才會停在岸邊而不是走到盡頭牆。
  for (let i = 0; i < 27; i++) {
    const s = await state();
    if (s.grounded && !s.inWater && Math.floor(s.x) >= bx0 + 2) break;
    await sleep(300);
  }
  await page.keyboard.up("KeyW");
  await sleep(800);  // 放手讓重力把人穩穩放到岸上
  const afterA = await state();
  await page.screenshot({ path: join(OUT_DIR, "swimout-after.png") });
  console.log("A 上岸後：", JSON.stringify(afterA));

  // ── 場景 B1：autoJump 關，只按 W → 應該上不去（尊重「撞一格要手動跳」的設定）──
  await page.evaluate(() => window.__voxel._qaSetAutoJump(false));
  await resetToPool();
  await sleep(1200);
  await page.keyboard.down("KeyW");
  await sleep(2500);
  await page.keyboard.up("KeyW");
  const afterB1 = await state();
  console.log("B1 自動跳關+只按W：", JSON.stringify(afterB1));

  // ── 場景 B2：autoJump 仍關，按住 跳+W → 應能爬上一格高的岸（手動路徑）──
  await page.keyboard.down("Space");
  await page.keyboard.down("KeyW");
  await sleep(4000);
  await page.keyboard.up("KeyW");
  await page.keyboard.up("Space");
  await sleep(800);
  const afterB2 = await state();
  console.log("B2 自動跳關+跳+W：", JSON.stringify(afterB2));
  await page.evaluate(() => window.__voxel._qaSetAutoJump(true)); // 還原預設

  // ── 場景 C：陸地無回歸——站上岸（H+2），走路自動踏上台階（H+3），再原地跳一下 ──
  await page.evaluate((p) => {
    const v = window.__voxel;
    v.setPlayerPos(p.x, p.y, p.z);
    v.setYaw(-Math.PI / 2);
  }, { x: bx0 + 2.5, y: H + 2.05, z: bz0 + 0.5 });
  await sleep(500); // 落地
  const landStart = await state();
  await page.keyboard.down("KeyW");
  await sleep(2500); // 走過岸面、撞台階應自動踏上（既有陸地踏階）
  await page.keyboard.up("KeyW");
  await sleep(600);
  const landWalk = await state();
  console.log("C 陸地走路+踏階：", JSON.stringify(landWalk));
  // 原地跳：vy 應瞬間變正（8.2），落回 grounded。
  await page.keyboard.down("Space");
  await sleep(120);
  const midJump = await state();
  await page.keyboard.up("Space");
  await sleep(1500);
  const landEnd = await state();
  console.log("C 跳躍中 vy：", midJump.vy.toFixed(2), "落地：", JSON.stringify(landEnd));

  // ── 斷言彙整 ──
  const onShore = (s) => s.grounded === true && !s.inWater && s.y >= H + 2 - 0.02 && Math.floor(s.x) >= bx0 + 2;
  const report = {
    scene, beforeA, afterA, afterB1, afterB2, landStart, landWalk, midJump, landEnd,
    checks: {
      pureEligible: elig.autoJump && elig.jumpHeld && elig.respectOff && elig.landNever,
      floatingBefore: beforeA.inWater === true && beforeA.grounded === false, // 起點：真的漂在水裡
      autoSwimOut: onShore(afterA),                                   // A：自動蹬上岸站穩
      respectAutoJumpOff: afterB1.inWater === true && Math.floor(afterB1.x) < bx0 + 2, // B1：關了就上不去
      jumpHeldSwimOut: onShore(afterB2),                              // B2：跳+W 手動爬上岸
      // C：陸地踏階照舊（踏上 H+3 台階；著地收斂容忍幾公分的逐幀逼近誤差）
      landStepUpIntact: landWalk.grounded === true && landWalk.y >= H + 3 - 0.02 && landWalk.y < H + 3.15,
      // C：跳躍照舊（按跳瞬間 vy 轉正上升 或 已明顯離地，之後落回 grounded）
      landJumpIntact: (midJump.vy > 1 || midJump.y - landWalk.y > 0.3) && landEnd.grounded === true,
    },
  };
  report.pass = Object.values(report.checks).every(Boolean);
  writeFileSync(join(OUT_DIR, "swim-out-report.json"), JSON.stringify(report, null, 2));
  console.log("\n──────── swim-out QA 報告 ────────");
  console.log(JSON.stringify(report.checks, null, 2));
  console.log(report.pass ? "PASS ✅" : "FAIL ❌");
  await browser.close();
  process.exit(report.pass ? 0 : 1);
})();
