// ============================================================
// voxel-edit-qa.mjs — voxel 放/挖方塊真實瀏覽器 QA（切片②）
// ============================================================
// 比照 voxel-qa.mjs：用 puppeteer-core 驅動系統 Chrome 載入 /voxel/，等地形載入後
// 1) 對準心瞄到的方塊「破壞」→ 驗證該座標真的變空氣（畫面變了）；
// 2) 選石頭、在面外側「放置」→ 驗證該座標真的變石頭；
// 同時量 FPS、確認非黑屏、截圖（破壞前/破壞後/放置後）。
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

const AIR = 0, STONE = 3;

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

  const logs = [];
  page.on("console", (m) => logs.push("[console] " + m.text()));
  page.on("pageerror", (e) => logs.push("[pageerror] " + e.message));

  await page.evaluateOnNewDocument(() => {
    window.__fps = { frames: 0, t0: performance.now() };
    const raf = window.requestAnimationFrame.bind(window);
    window.requestAnimationFrame = (cb) => raf((t) => { window.__fps.frames++; cb(t); });
  });

  console.log("載入", URL);
  await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 30000 });

  // 等 chunk + mesh 建好。
  await page.bringToFront();
  await sleep(6000);

  // 像真玩家那樣「拖曳把視角往下壓」對準腳「前」的地面（同時驗證真實滑鼠輸入路徑）。
  // 在畫布上由上往下拖（clientY 增加 → camPitch 增加 → 看得更下面）。每次小幅下壓，
  // 取第一個「放置點不在自己身體裡」的目標（避免對著正下方腳底、放置被身體保護擋掉）。
  async function aimDownAndTarget() {
    for (let i = 0; i < 16; i++) {
      await page.mouse.move(640, 360);
      await page.mouse.down();
      await page.mouse.move(640, 410, { steps: 4 }); // 每次 +50px ≈ +0.25rad 俯角
      await page.mouse.up();
      await sleep(300);
      const r = await page.evaluate(() => {
        const v = window.__voxel, t = v.target, p = v.player;
        if (!t) return { ok: false };
        const px = t.bx + t.nx, py = t.by + t.ny, pz = t.bz + t.nz;
        const inBody = px === Math.floor(p.x) && pz === Math.floor(p.z) &&
          (py === Math.floor(p.y) || py === Math.floor(p.y + 1));
        return { ok: !inBody, t: { bx: t.bx, by: t.by, bz: t.bz, nx: t.nx, ny: t.ny, nz: t.nz } };
      });
      if (r.ok) return r.t;
    }
    return null;
  }
  let tgt = await aimDownAndTarget();
  if (!tgt) { console.error("QA 失敗：準心一直沒對到方塊（地形沒載入？）"); await browser.close(); process.exit(1); }

  await page.screenshot({ path: join(OUT_DIR, "edit-1-before.png") });

  // ── 1) 破壞（doBreak 回傳實際被挖的座標，避免與下一幀 target 不一致）──
  const beforeBreak = await page.evaluate(([x, y, z]) => window.__voxel.getBlock(x, y, z), [tgt.bx, tgt.by, tgt.bz]);
  const bpos = await page.evaluate(() => window.__voxel.doBreak());
  await sleep(800); // 伺服器驗證 + 廣播 + 本地套用 + 重建 mesh
  const afterBreak = bpos
    ? await page.evaluate(([x, y, z]) => window.__voxel.getBlock(x, y, z), [bpos.x, bpos.y, bpos.z])
    : null;
  await page.screenshot({ path: join(OUT_DIR, "edit-2-after-break.png") });
  const breakOk = bpos && beforeBreak > 0 && afterBreak === AIR;
  console.log(`破壞 @ ${bpos ? `${bpos.x},${bpos.y},${bpos.z}` : "?"}: ${beforeBreak} → ${afterBreak}  ${breakOk ? "OK✅" : "FAIL❌"}`);

  // ── 2) 放置（選石頭；doPlace 回傳實際放置座標）──
  await page.evaluate((stone) => window.__voxel.selectSlotByBlock(stone), STONE);
  await sleep(200);
  const ppos = await page.evaluate(() => window.__voxel.doPlace());
  await sleep(800);
  const placed = ppos
    ? await page.evaluate(([x, y, z]) => window.__voxel.getBlock(x, y, z), [ppos.x, ppos.y, ppos.z])
    : null;
  await page.screenshot({ path: join(OUT_DIR, "edit-3-after-place.png") });
  const placeOk = ppos && placed === STONE;
  console.log(`放置 @ ${ppos ? `${ppos.x},${ppos.y},${ppos.z}` : "?"}: → ${placed}  ${placeOk ? "OK✅" : "FAIL❌"}`);

  // ── FPS + 非黑屏 ──
  const fpsAvg = await page.evaluate(() => {
    const dt = (performance.now() - window.__fps.t0) / 1000;
    return window.__fps.frames / dt;
  });
  const state = await page.evaluate(() => {
    const v = window.__voxel || {};
    return { chunks: v.chunks, meshes: v.meshes, selfFps: v.fps };
  });
  const png = await page.screenshot({ path: join(OUT_DIR, "edit-3-after-place.png") });
  const distinctBytes = new Set(png).size;

  console.log("\n──────── VOXEL 編輯 QA 報告 ────────");
  console.log("FPS(rAF平均):", fpsAvg.toFixed(1), " 自報:", (state.selfFps || 0).toFixed(1));
  console.log("chunks:", state.chunks, " meshes:", state.meshes);
  console.log("截圖:", OUT_DIR, "(edit-1-before / edit-2-after-break / edit-3-after-place)");
  if (logs.length) console.log("頁面訊息:\n" + logs.slice(0, 20).join("\n"));

  const pass = breakOk && placeOk &&
    state.chunks > 0 && state.meshes > 0 &&
    png.length > 8000 && distinctBytes > 80 && fpsAvg > 20;
  console.log("\n判定:", pass ? "PASS ✅（破壞+放置真的生效、非黑屏、FPS 健康）" : "CHECK ⚠️（見上方數據）");
  writeFileSync(join(OUT_DIR, "voxel-edit-qa.json"),
    JSON.stringify({ fpsAvg, state, breakOk, placeOk, beforeBreak, afterBreak, placed, pass }, null, 2));

  await browser.close();
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
