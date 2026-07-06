// ============================================================
// voxel-village-restore-qa.mjs — 乙太方界「村莊大修復」實測 QA（隔離伺服器·帶 prod delta 副本）
// ============================================================
// 對隔離伺服器（記憶體模式、獨立 port、精確 PID 收尾絕不 pkill）驗證村莊大修復：
//   - 帶 **prod delta 副本**（voxel_resident_blocks.jsonl + voxel_village_done）進隔離 data/。
//   - (before) 先預先寫 `.village_restored_v1` marker → migration 跳過 → 截「修復前」的爛村（大坑+淹水）。
//   - (after)  同一份副本、不寫 marker → migration 跑 → 截「修復後」（坑填平、水退、建築道路完好），
//              並比對「修復後」jsonl 比「修復前」多出的 append：統計回填(Air/實心)與清水(→Air)數量，
//              且驗證既有建築/道路方塊一塊不少（只加不拆）。
//
// 用法：
//   VILLAGE_RESTORE_QA_BIN=target/release/butfun-server \
//   VILLAGE_RESTORE_QA_OUT=/path/to/scratchpad \
//   VILLAGE_RESTORE_PROD_DATA=/home/shihshih/butfun-data \
//   node scripts/qa/voxel-village-restore-qa.mjs
//
// 不抄外部碼；全繁中註解；node --check 過。

import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, readFileSync, writeFileSync, existsSync, appendFileSync, symlinkSync, copyFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const BIN = resolve(process.env.VILLAGE_RESTORE_QA_BIN || "target/release/butfun-server");
const OUT = process.env.VILLAGE_RESTORE_QA_OUT || "/tmp/village-restore-qa-out";
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const PROD_DATA = process.env.VILLAGE_RESTORE_PROD_DATA || "/home/shihshih/butfun-data";
const PORT = Number(process.env.VILLAGE_RESTORE_QA_PORT || 3973);
mkdirSync(OUT, { recursive: true });

const AIR = 0;
// 保留清單常見型別（建材/功能/樹/農田）——修復後這些一塊都不該被動。
const PRESERVE = new Set([5, 6, 8, 9, 10, 11, 15, 16, 17, 31, 42, 43, 45, 66, 79]);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const check = (cond, msg) => { if (cond) console.log("  ✅ " + msg); else { console.log("  ❌ " + msg); failures++; } };

// 讀村莊中心旗標 "cx,cz"。
function readVillageCenter(dir) {
  const p = join(dir, "data", "voxel_village_done");
  if (!existsSync(p)) return null;
  const [cx, cz] = readFileSync(p, "utf8").trim().split(",").map(Number);
  return Number.isFinite(cx) && Number.isFinite(cz) ? [cx, cz] : null;
}

// 讀 resident blocks jsonl → Map "x,y,z" -> b（最後一筆 append 為準，比照伺服器 replay 語意）。
function readResidentBlocks(dir) {
  const path = join(dir, "data", "voxel_resident_blocks.jsonl");
  const map = new Map();
  if (!existsSync(path)) return map;
  for (const line of readFileSync(path, "utf8").split("\n")) {
    const t = line.trim();
    if (!t) continue;
    try { const o = JSON.parse(t); map.set(`${o.x},${o.y},${o.z}`, o.b); } catch {}
  }
  return map;
}

function launchServer(dir) {
  mkdirSync(join(dir, "data"), { recursive: true });
  const repoRoot = resolve(join(dirname(fileURLToPath(import.meta.url)), "..", ".."));
  for (const asset of ["web", "site"]) {
    try { symlinkSync(join(repoRoot, asset), join(dir, asset)); } catch {}
  }
  const proc = spawn(BIN, [], {
    cwd: dir,
    env: { ...process.env, PORT: String(PORT), DATABASE_URL: "", BUTFUN_NPC_LLM: "0" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  proc.stdout.on("data", (d) => appendFileSync(join(OUT, "server.log"), d));
  proc.stderr.on("data", (d) => appendFileSync(join(OUT, "server.log"), d));
  return proc;
}

function killExact(proc) {
  if (proc && proc.pid && !proc.killed) { try { process.kill(proc.pid, "SIGTERM"); } catch {} }
}

async function waitReady() {
  for (let i = 0; i < 90; i++) {
    try { const r = await fetch(`http://127.0.0.1:${PORT}/voxel/feed`); if (r.ok) return true; } catch {}
    await sleep(500);
  }
  return false;
}
async function getFeed() {
  try { const r = await fetch(`http://127.0.0.1:${PORT}/voxel/feed`); return r.ok ? await r.json() : []; } catch { return []; }
}

// 帶 prod delta 副本建一個隔離 data/：只複製修復需要的兩個檔（delta + 村莊中心旗標），
// 其餘 store 缺檔 = 空（向後相容），讓伺服器輕量啟動。preRestored=true 則預寫 marker（跳過 migration）。
function seedIsolatedData(dir, { preRestored }) {
  mkdirSync(join(dir, "data"), { recursive: true });
  copyFileSync(join(PROD_DATA, "voxel_resident_blocks.jsonl"), join(dir, "data", "voxel_resident_blocks.jsonl"));
  if (existsSync(join(PROD_DATA, "voxel_village_done"))) {
    copyFileSync(join(PROD_DATA, "voxel_village_done"), join(dir, "data", "voxel_village_done"));
  }
  // 舊坑修復 marker 也帶上（prod 已跑過），避免混入其他 migration 的 append 干擾統計。
  if (existsSync(join(PROD_DATA, ".gather_holes_migrated_v1"))) {
    copyFileSync(join(PROD_DATA, ".gather_holes_migrated_v1"), join(dir, "data", ".gather_holes_migrated_v1"));
  }
  if (preRestored) writeFileSync(join(dir, "data", ".village_restored_v1"), "1");
}

// headless Chrome 俯視某點截圖。
async function screenshotAt(page, px, pz, height, pitch, name) {
  await page.evaluate(({ px, pz, height, pitch }) => {
    const v = window.__voxel;
    if (!v || !v.player) return;
    if (window.__qaPin) clearInterval(window.__qaPin);
    const pin = () => { v.player.x = px; v.player.z = pz; v.player.y = height; v.player.vy = 0; };
    pin();
    window.__qaPin = setInterval(pin, 60);
    if (v.setCamPitch) v.setCamPitch(pitch);
    if (v.setYaw) v.setYaw(0);
  }, { px, pz, height, pitch });
  await sleep(3500);
  await page.screenshot({ path: join(OUT, name) });
  await page.evaluate(() => { if (window.__qaPin) clearInterval(window.__qaPin); });
  console.log("  📸 " + name);
}

// 跑一台伺服器 + 截圖（俯視全村 + 街景），回 { blocks: Map, center }。
async function runAndShoot(label, { preRestored }) {
  console.log(`\n[${label}] 帶 prod delta 副本啟動（preRestored=${preRestored}）`);
  const dir = mkdtempSync(join(tmpdir(), `village-restore-${label}-`));
  seedIsolatedData(dir, { preRestored });
  const before = readResidentBlocks(dir);
  const proc = launchServer(dir);
  const ready = await waitReady();
  check(ready, `${label}: 伺服器啟動就緒`);
  await sleep(3000); // 讓 migration 落地完成
  const center = readVillageCenter(dir) || [0, 0];
  const after = readResidentBlocks(dir);

  // 截圖：俯視全村（村莊中心正上方高空）+ 街景（斜俯）。
  let puppeteer = null;
  try { puppeteer = (await import("puppeteer-core")).default; } catch {}
  if (puppeteer && existsSync(CHROME)) {
    const args = [
      "--no-sandbox", "--disable-setuid-sandbox", "--ignore-gpu-blocklist",
      "--enable-gpu", "--enable-webgl", "--use-gl=angle", "--use-angle=gl",
      "--disable-dev-shm-usage", "--window-size=1280,900", "--disable-background-timer-throttling",
    ];
    const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args });
    try {
      const page = await browser.newPage();
      await page.setViewport({ width: 1280, height: 900 });
      page.on("pageerror", (e) => appendFileSync(join(OUT, "browser.log"), "[pageerror] " + e.message + "\n"));
      await page.goto(`http://127.0.0.1:${PORT}/voxel/?debug=1`, { waitUntil: "domcontentloaded", timeout: 30000 });
      await sleep(8000);
      const [cx, cz] = center;
      await screenshotAt(page, cx + 0.5, cz + 0.5, 48, 1.3, `restore-${label}-topdown.png`);
      await screenshotAt(page, cx + 0.5, cz + 12, 20, 0.85, `restore-${label}-street.png`);
      const state = await page.evaluate(() => { const v = window.__voxel || {}; return { chunks: v.chunks, meshes: v.meshes }; });
      check((state.chunks || 0) > 0 && (state.meshes || 0) > 0, `${label}: 非黑屏 chunks=${state.chunks} meshes=${state.meshes}`);
    } finally { await browser.close(); }
  } else {
    console.log("  ⚠️  無 puppeteer-core / Chrome，跳過截圖（統計仍驗）");
  }

  killExact(proc);
  await sleep(1200);
  return { dir, before, after, center };
}

(async () => {
  console.log("=== 村莊大修復 QA（隔離伺服器 port " + PORT + "，bin " + BIN + "）===");
  writeFileSync(join(OUT, "server.log"), "");
  writeFileSync(join(OUT, "browser.log"), "");
  if (!existsSync(BIN)) { console.log("❌ 找不到 binary：" + BIN); process.exit(2); }
  if (!existsSync(join(PROD_DATA, "voxel_resident_blocks.jsonl"))) {
    console.log("❌ 找不到 prod delta：" + join(PROD_DATA, "voxel_resident_blocks.jsonl")); process.exit(2);
  }

  // (before) 預寫 marker → migration 跳過 → 截爛村。
  const b = await runAndShoot("before", { preRestored: true });
  // (after) 不寫 marker → migration 跑 → 截修復後 + 統計。
  const a = await runAndShoot("after", { preRestored: false });

  // 統計：修復後相對修復前多出/改變的格。
  let refilled = 0, drained = 0, preservedTouched = 0;
  const beforeMap = a.before; // after 情境啟動前（＝原 prod 副本）
  const afterMap = a.after;
  for (const [k, bv] of afterMap) {
    const ov = beforeMap.get(k);
    if (ov === bv) continue; // 沒變
    if (bv === AIR) {
      // 變成 Air：排水（清流動水）。
      drained++;
    } else {
      // 變成實心：回填坑。
      refilled++;
    }
    // 保留清單方塊被改動 = 嚴重錯誤（絕不該動建築/道路/農田/功能方塊）。
    if (ov !== undefined && PRESERVE.has(ov)) preservedTouched++;
  }
  console.log(`\n[統計] 修復後相對 prod 副本：回填坑 ${refilled} 格、清流動水 ${drained} 格`);
  check(refilled > 0 || drained > 0, "大修復確實動了手（回填或清水 > 0）");
  check(preservedTouched === 0, `保留清單一塊不動（被誤動 ${preservedTouched} 塊建築/道路/農田/功能方塊）`);

  // Feed 有溫暖的修復一句。
  const feed = await getFeed();
  // 註：feed 端點在 after 伺服器已收尾，這裡讀不到；改在 server.log 找那句。
  const log = existsSync(join(OUT, "server.log")) ? readFileSync(join(OUT, "server.log"), "utf8") : "";
  check(log.includes("村莊大修復完成"), "server.log 出現「村莊大修復完成」統計行");

  rmSync(b.dir, { recursive: true, force: true });
  rmSync(a.dir, { recursive: true, force: true });
  console.log("\n=== 結果：" + (failures === 0 ? "全綠 ✅" : failures + " 項失敗 ❌") + " ===");
  console.log("截圖/日誌在：" + OUT);
  process.exit(failures === 0 ? 0 : 1);
})();
