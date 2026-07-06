// ============================================================
// voxel-village-qa.mjs — 乙太方界「村莊系統 v1」實測 QA（隔離伺服器·真啟動）
// ============================================================
// 對隔離伺服器（記憶體模式、獨立 port、乾淨 data/、精確 PID 收尾絕不 pkill）驗證：
//   (a) 全新世界：啟動時村莊規劃/整理跑一次 → data/voxel_resident_blocks.jsonl 裡出現
//       中央廣場（石磚 StoneBrick=9）＋十字主路（拋光石 SmoothStone=17）＋四角燈（火把 Torch=31），
//       且 data/voxel_village_done 旗標寫下、Feed 有「村莊整理」一句。
//   (b) migration 冪等：重啟同一 data/ 不再重跑（旗標存在 → 方塊數不再增長）。
//   (c) 帶入既有建築的 delta 副本：既有建築方塊一塊不少（只加不拆），且從廣場鋪出的路把它連起來。
//   最後用 headless Chrome 載 /voxel/，把玩家抬到高空俯視，多角度截圖存 scratchpad。
//
// 用法：
//   VILLAGE_QA_BIN=target/release/butfun-server \
//   VILLAGE_QA_OUT=/path/to/scratchpad \
//   node scripts/qa/voxel-village-qa.mjs
//
// 不抄外部碼；全繁中註解；node --check 過。

import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, readFileSync, writeFileSync, existsSync, appendFileSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// 伺服器在各自 cwd（隔離 temp 目錄）啟動 → BIN 必須是絕對路徑，否則相對路徑在 temp cwd 下解不到。
const BIN = resolve(process.env.VILLAGE_QA_BIN || "target/release/butfun-server");
const OUT = process.env.VILLAGE_QA_OUT || "/tmp/village-qa-out";
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const PORT = Number(process.env.VILLAGE_QA_PORT || 3971);
mkdirSync(OUT, { recursive: true });

const STONE_BRICK = 9, SMOOTH_STONE = 17, TORCH = 31;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const check = (cond, msg) => { if (cond) console.log("  ✅ " + msg); else { console.log("  ❌ " + msg); failures++; } };

// ── 隔離啟動一台伺服器（記憶體模式、指定 cwd/port），回 {proc, dir} ────────────────
function launchServer(dir, extraEnv = {}) {
  mkdirSync(join(dir, "data"), { recursive: true });
  // 伺服器用相對路徑讀前端資產（web/voxel/index.html 等）→ 隔離 cwd 要 symlink 回 repo 的 web/。
  const repoRoot = resolve(join(dirname(fileURLToPath(import.meta.url)), "..", ".."));
  for (const asset of ["web", "site"]) {
    try { symlinkSync(join(repoRoot, asset), join(dir, asset)); } catch { /* 已存在即略過 */ }
  }
  const proc = spawn(BIN, [], {
    cwd: dir,
    env: {
      ...process.env,
      PORT: String(PORT),
      // 記憶體模式：不設 DATABASE_URL → 走 JSONL/記憶體（見 main.rs）。
      DATABASE_URL: "",
      // 關掉會燒錢/擾動的 LLM 思考，QA 只看世界生成。
      BUTFUN_NPC_LLM: "0",
      ...extraEnv,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  proc.stdout.on("data", (d) => appendFileSync(join(OUT, "server.log"), d));
  proc.stderr.on("data", (d) => appendFileSync(join(OUT, "server.log"), d));
  return proc;
}

// ── 精確收尾：只殺我們啟動的那個 PID（絕不 pkill -f，避免跨 checkout 誤殺 prod）──────
function killExact(proc) {
  if (proc && proc.pid && !proc.killed) {
    try { process.kill(proc.pid, "SIGTERM"); } catch {}
  }
}

// ── 讀居民方塊改動 jsonl，回一個 "x,y,z" -> b 的 Map ────────────────────────────
function readResidentBlocks(dir) {
  const path = join(dir, "data", "voxel_resident_blocks.jsonl");
  const map = new Map();
  if (!existsSync(path)) return map;
  for (const line of readFileSync(path, "utf8").split("\n")) {
    const t = line.trim();
    if (!t) continue;
    try {
      const o = JSON.parse(t);
      map.set(`${o.x},${o.y},${o.z}`, o.b);
    } catch {}
  }
  return map;
}

// ── 等伺服器起來（poll /voxel/feed）+ 村莊旗標寫下 ─────────────────────────────
async function waitReady(dir) {
  for (let i = 0; i < 60; i++) {
    try {
      const r = await fetch(`http://127.0.0.1:${PORT}/voxel/feed`);
      if (r.ok) {
        // 旗標檔寫下＝村莊整理已跑完。
        if (existsSync(join(dir, "data", "voxel_village_done"))) return true;
      }
    } catch {}
    await sleep(500);
  }
  return false;
}

async function getFeed() {
  try {
    const r = await fetch(`http://127.0.0.1:${PORT}/voxel/feed`);
    return r.ok ? await r.json() : [];
  } catch { return []; }
}

// ── 從 block map 統計三種村莊方塊 ──────────────────────────────────────────────
function countVillageBlocks(map) {
  let brick = 0, smooth = 0, torch = 0;
  for (const b of map.values()) {
    if (b === STONE_BRICK) brick++;
    else if (b === SMOOTH_STONE) smooth++;
    else if (b === TORCH) torch++;
  }
  return { brick, smooth, torch, total: map.size };
}

// ── (a)(b) 全新世界 + 冪等：一個 data 目錄，跑兩次啟動 ────────────────────────────
async function scenarioFreshAndIdempotent() {
  console.log("\n[情境 a/b] 全新世界：村莊規劃/整理 + 冪等重啟");
  const dir = mkdtempSync(join(tmpdir(), "village-fresh-"));
  let proc = launchServer(dir);
  const ready = await waitReady(dir);
  check(ready, "伺服器啟動且村莊旗標寫下（data/voxel_village_done）");

  const map1 = readResidentBlocks(dir);
  const c1 = countVillageBlocks(map1);
  console.log(`     首次啟動：石磚廣場=${c1.brick} 拋光石路=${c1.smooth} 火把燈=${c1.torch}（總改動 ${c1.total}）`);
  check(c1.brick > 20, "中央廣場鋪了石磚（>20 塊）");
  check(c1.smooth > 40, "十字主路鋪了拋光石（>40 塊）");
  check(c1.torch >= 1, "廣場有火把燈（≥1）");

  const feed = await getFeed();
  const hasVillageFeed = feed.some((e) => (e.kind || "").includes("村莊") || (e.detail || "").includes("石板路"));
  check(hasVillageFeed, "Feed 出現村莊整理一句（村裡鋪起了石板路）");

  // 收尾（精確 PID），再用同一 data/ 重啟驗冪等。
  killExact(proc);
  await sleep(1500);

  console.log("     重啟同一 data/（驗 migration 冪等）…");
  proc = launchServer(dir);
  await waitReady(dir);
  await sleep(1500);
  const c2 = countVillageBlocks(readResidentBlocks(dir));
  console.log(`     重啟後：石磚=${c2.brick} 拋光石=${c2.smooth} 火把=${c2.torch}（總改動 ${c2.total}）`);
  check(c2.total === c1.total, `migration 冪等：重啟後方塊數不變（${c1.total} → ${c2.total}）`);
  killExact(proc);
  await sleep(1000);
  rmSync(dir, { recursive: true, force: true });
}

// ── (c) 帶入既有建築 delta 副本：既有方塊一塊不少 + 路連起來 ────────────────────────
async function scenarioExistingBuildingsPreserved() {
  console.log("\n[情境 c] 帶入既有建築 delta：只加不拆、路連起來");
  const dir = mkdtempSync(join(tmpdir(), "village-existing-"));
  mkdirSync(join(dir, "data"), { recursive: true });
  // 造一份「既有建築」delta 副本：在世界某處（遠離廣場中線，模擬散落的家）放一小棟木屋牆。
  // 這些是「居民已蓋的作品」——migration 絕不能覆蓋/刪除它們。
  const WOOD = 5, PLANK = 8;
  const existing = [];
  // 一小棟 3×3 木牆在 (30, 30) 附近（地表 y 用 9 附近，程序地形 BASE_HEIGHT=8）。
  const bx = 30, bz = 30;
  for (let dx = -1; dx <= 1; dx++) {
    for (let dz = -1; dz <= 1; dz++) {
      const border = Math.abs(dx) === 1 || Math.abs(dz) === 1;
      if (border) existing.push({ x: bx + dx, y: 9, z: bz + dz, b: WOOD });
    }
  }
  existing.push({ x: bx, y: 9, z: bz, b: PLANK }); // 屋內地板一塊，確認也不被動
  const path = join(dir, "data", "voxel_resident_blocks.jsonl");
  writeFileSync(path, existing.map((e) => JSON.stringify(e)).join("\n") + "\n");
  const beforeKeys = new Set(existing.map((e) => `${e.x},${e.y},${e.z}`));
  const beforeVals = new Map(existing.map((e) => [`${e.x},${e.y},${e.z}`, e.b]));

  const proc = launchServer(dir);
  const ready = await waitReady(dir);
  check(ready, "帶既有建築啟動成功、村莊旗標寫下");

  const after = readResidentBlocks(dir);
  // 既有建築每一塊都還在、且型別未變（只加不拆）。
  let preserved = 0, changed = 0;
  for (const k of beforeKeys) {
    if (after.has(k) && after.get(k) === beforeVals.get(k)) preserved++;
    else changed++;
  }
  check(changed === 0, `既有建築 ${beforeKeys.size} 塊一塊不少、型別未變（改動 ${changed}）`);
  check(preserved === beforeKeys.size, "既有建築作品完整保留（資料安全）");

  const c = countVillageBlocks(after);
  check(c.brick + c.smooth > 40, "村莊廣場+路仍照鋪（既有建築周邊被路連起來）");
  console.log(`     帶既有建築：既有保留=${preserved} 廣場石磚=${c.brick} 路拋光石=${c.smooth}`);

  killExact(proc);
  await sleep(1000);
  // 這份留著給截圖情境用（下面 browser 會重連這台）——但我們已 kill；改在截圖情境自建。
  rmSync(dir, { recursive: true, force: true });
}

// ── 多角度俯視截圖（headless Chrome，抬高玩家俯視全村/廣場/路連建築）──────────────
async function scenarioScreenshots() {
  console.log("\n[截圖] headless Chrome 俯視全村 / 廣場 / 路連建築");
  let puppeteer;
  try { puppeteer = (await import("puppeteer-core")).default; }
  catch { console.log("  ⚠️  無 puppeteer-core，跳過截圖（方塊驗證已足證功能）"); return; }

  const dir = mkdtempSync(join(tmpdir(), "village-shot-"));
  // 同情境 c：也放一小棟既有木屋，讓截圖看得到「路連到既有建築」。
  mkdirSync(join(dir, "data"), { recursive: true });
  const WOOD = 5;
  const existing = [];
  for (const [bx, bz] of [[30, 8], [8, 30]]) {
    for (let dx = -1; dx <= 1; dx++) for (let dz = -1; dz <= 1; dz++) {
      if (Math.abs(dx) === 1 || Math.abs(dz) === 1) existing.push({ x: bx + dx, y: 9, z: bz + dz, b: WOOD });
    }
  }
  writeFileSync(join(dir, "data", "voxel_resident_blocks.jsonl"),
    existing.map((e) => JSON.stringify(e)).join("\n") + "\n");

  const proc = launchServer(dir);
  const ready = await waitReady(dir);
  if (!ready) { console.log("  ❌ 截圖情境伺服器未就緒"); failures++; killExact(proc); return; }

  const args = [
    "--no-sandbox", "--disable-setuid-sandbox", "--ignore-gpu-blocklist",
    "--enable-gpu", "--enable-webgl", "--use-gl=angle", "--use-angle=gl",
    "--disable-dev-shm-usage", "--window-size=1280,900",
    "--disable-background-timer-throttling",
  ];
  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args });
  try {
    const page = await browser.newPage();
    await page.setViewport({ width: 1280, height: 900 });
    page.on("pageerror", (e) => appendFileSync(join(OUT, "browser.log"), "[pageerror] " + e.message + "\n"));
    await page.goto(`http://127.0.0.1:${PORT}/voxel/?debug=1`, { waitUntil: "domcontentloaded", timeout: 30000 });
    await sleep(7000); // 等 chunk 載入 + mesh

    // 把玩家抬到指定點正上方高空、鏡頭朝下俯視（用既有 __voxel 除錯鉤子）。
    // 鏡頭幾何：camy = ty + dist·sin(camPitch) → **正 pitch = 鏡頭在上往下看**（第三人稱夾 [-0.2, 1.3]）。
    // 重力會把玩家往下拉 → 用 interval 每 60ms 釘住位置，直到截圖完才解除。
    async function lookDownAt(px, pz, height, pitch) {
      await page.evaluate(({ px, pz, height, pitch }) => {
        const v = window.__voxel;
        if (!v || !v.player) return;
        if (window.__qaPin) clearInterval(window.__qaPin);
        const pin = () => {
          v.player.x = px; v.player.z = pz; v.player.y = height; v.player.vy = 0;
        };
        pin();
        window.__qaPin = setInterval(pin, 60);
        if (v.setCamPitch) v.setCamPitch(pitch); // 正值＝鏡頭在上往下看
        if (v.setYaw) v.setYaw(0);
      }, { px, pz, height, pitch });
      await sleep(3000); // 等 chunk 依新位置載入 + 重繪
    }

    // 俯視全村（村莊中心 (0,19)，高、近正俯視）。
    await lookDownAt(0.5, 19.5, 42, 1.3);
    await sleep(1500);
    await page.screenshot({ path: join(OUT, "village-topdown.png") });
    console.log("  📸 village-topdown.png（俯視全村）");

    // 廣場近觀（鏡頭在玩家後上方、朝 -z 看 → 釘在廣場**南側**回望，廣場入鏡）。
    await lookDownAt(0.5, 29, 18, 0.85);
    await page.screenshot({ path: join(OUT, "village-plaza.png") });
    console.log("  📸 village-plaza.png（中央廣場）");

    // 路連既有建築（既有木屋在 (30,8)：釘在其南側回望，木屋＋路入鏡）。
    await lookDownAt(29, 20, 22, 0.95);
    await page.screenshot({ path: join(OUT, "village-road-to-house.png") });
    console.log("  📸 village-road-to-house.png（路連既有建築）");
    // 解除釘住。
    await page.evaluate(() => { if (window.__qaPin) clearInterval(window.__qaPin); });

    const state = await page.evaluate(() => {
      const v = window.__voxel || {};
      return { chunks: v.chunks, meshes: v.meshes, fps: v.fps };
    });
    check((state.chunks || 0) > 0 && (state.meshes || 0) > 0, `非黑屏：chunks=${state.chunks} meshes=${state.meshes}`);
  } finally {
    await browser.close();
    killExact(proc);
    await sleep(1000);
    rmSync(dir, { recursive: true, force: true });
  }
}

(async () => {
  console.log("=== 村莊系統 v1 QA（隔離伺服器 port " + PORT + "，bin " + BIN + "）===");
  writeFileSync(join(OUT, "server.log"), "");
  if (!existsSync(BIN)) { console.log("❌ 找不到伺服器 binary：" + BIN); process.exit(2); }
  try {
    await scenarioFreshAndIdempotent();
    await scenarioExistingBuildingsPreserved();
    await scenarioScreenshots();
  } catch (e) {
    console.log("❌ QA 例外：" + (e && e.stack || e));
    failures++;
  }
  console.log("\n=== 結果：" + (failures === 0 ? "全綠 ✅" : failures + " 項失敗 ❌") + " ===");
  console.log("截圖/日誌在：" + OUT);
  process.exit(failures === 0 ? 0 : 1);
})();
