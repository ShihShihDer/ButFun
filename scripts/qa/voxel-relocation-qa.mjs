// ============================================================
// voxel-relocation-qa.mjs — 乙太方界「居民搬新家（引導式都更）」實測 QA
// ============================================================
// 隔離伺服器（記憶體模式、獨立 port、精確 PID 收尾絕不 pkill、BUTFUN_RELOC_FAST=1 加速節奏）。
// 佈置一個「散落舊家」的世界：用 `butfun-server dump-house`（與伺服器同一份確定性函式）
// 生出 vox_res_0 / vox_res_1 兩座村外舊家的方塊，寫進隔離 data/；舊家旁再放「玩家物」
// 哨兵方塊（箱子/告示牌/木板平台）。啟動後觀察：
//   1) 第一位（vox_res_0）完整走完：認領地塊 → 蓋新家 → 拆舊家 → 材料入包 → 家域遷移。
//   2) 玩家哨兵方塊與鄰居（vox_res_1）舊家在第一位搬完當下毫髮無傷。
//   3) 第二位接著開始（錯開：start ts 晚於第一位 done）。
//   4) 第二位進行中重啟伺服器 → 中斷可恢復（重啟後接著搬到完成）。
//   5) 前後對比截圖（舊家址 before/after、新家址 after、村中心）存 OUT。
//
// 用法：
//   RELOC_QA_BIN=target/release/butfun-server \
//   RELOC_QA_OUT=/path/to/scratchpad \
//   node scripts/qa/voxel-relocation-qa.mjs
//
// 不抄外部碼；全繁中註解；node --check 過。

import { spawn, execFileSync } from "node:child_process";
import {
  mkdirSync, mkdtempSync, readFileSync, writeFileSync, existsSync, appendFileSync, symlinkSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const BIN = resolve(process.env.RELOC_QA_BIN || "target/release/butfun-server");
const OUT = process.env.RELOC_QA_OUT || "/tmp/reloc-qa-out";
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const PORT = Number(process.env.RELOC_QA_PORT || 3977);
mkdirSync(OUT, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const check = (cond, msg) => { if (cond) console.log("  ✅ " + msg); else { console.log("  ❌ " + msg); failures++; } };

// 兩座村外舊家的平面錨點（村中心約 (0,19)，地塊最遠 |x|≈80、|z|≈83——這兩點 Chebyshev 遠超 10）。
const OLD_A = { rid: "vox_res_0", x: -120, z: 40 };
const OLD_B = { rid: "vox_res_1", x: 115, z: -30 };

// 用伺服器自己的 dump-house 子指令取「與建造/拆除引擎逐塊一致」的舊家方塊清單。
function dumpHouse(rid, cx, cz) {
  const out = execFileSync(BIN, ["dump-house", rid, String(cx), String(cz)], { encoding: "utf8" });
  return JSON.parse(out.trim());
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

// 讀 feed jsonl（持久檔，比 HTTP 端點好比對時間先後）。
function readFeed(dir) {
  const path = join(dir, "data", "voxel_feed.jsonl");
  if (!existsSync(path)) return [];
  return readFileSync(path, "utf8").split("\n").filter(Boolean).map((l) => {
    try { return JSON.parse(l); } catch { return null; }
  }).filter(Boolean);
}

function launchServer(dir) {
  const repoRoot = resolve(join(dirname(fileURLToPath(import.meta.url)), "..", ".."));
  for (const asset of ["web", "site"]) {
    try { symlinkSync(join(repoRoot, asset), join(dir, asset)); } catch {}
  }
  const proc = spawn(BIN, [], {
    cwd: dir,
    env: {
      ...process.env, PORT: String(PORT), DATABASE_URL: "", BUTFUN_NPC_LLM: "0",
      BUTFUN_RELOC_FAST: "1",
    },
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
  for (let i = 0; i < 120; i++) {
    try { const r = await fetch(`http://127.0.0.1:${PORT}/voxel/feed`); if (r.ok) return true; } catch {}
    await sleep(500);
  }
  return false;
}

// 等 feed 出現符合條件的事件（回該事件；逾時回 null）。
async function waitFeed(dir, pred, timeoutMs, label) {
  const t0 = Date.now();
  for (;;) {
    const hit = readFeed(dir).find(pred);
    if (hit) return hit;
    if (Date.now() - t0 > timeoutMs) { console.log("  ⏰ 等不到：" + label); return null; }
    await sleep(1500);
  }
}

// headless Chrome 俯視某點截圖（比照 voxel-village-restore-qa 的作法）。
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

async function openBrowser() {
  let puppeteer = null;
  try { puppeteer = (await import("puppeteer-core")).default; } catch {}
  if (!puppeteer || !existsSync(CHROME)) { console.log("  ⚠️  無 puppeteer-core / Chrome，跳過截圖"); return null; }
  const args = [
    "--no-sandbox", "--disable-setuid-sandbox", "--ignore-gpu-blocklist",
    "--enable-gpu", "--enable-webgl", "--use-gl=angle", "--use-angle=gl",
    "--disable-dev-shm-usage", "--window-size=1280,900", "--disable-background-timer-throttling",
  ];
  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args });
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 900 });
  page.on("pageerror", (e) => appendFileSync(join(OUT, "browser.log"), "[pageerror] " + e.message + "\n"));
  await page.goto(`http://127.0.0.1:${PORT}/voxel/?debug=1`, { waitUntil: "domcontentloaded", timeout: 30000 });
  await sleep(8000);
  return { browser, page };
}

(async () => {
  console.log("=== 居民搬新家（引導式都更）QA（隔離伺服器 port " + PORT + "，bin " + BIN + "）===");
  writeFileSync(join(OUT, "server.log"), "");
  writeFileSync(join(OUT, "browser.log"), "");
  if (!existsSync(BIN)) { console.log("❌ 找不到 binary：" + BIN); process.exit(2); }

  // ── 佈置隔離世界：兩座村外舊家 + 玩家哨兵方塊 ─────────────────────────────
  const dir = mkdtempSync(join(tmpdir(), "reloc-qa-"));
  mkdirSync(join(dir, "data"), { recursive: true });
  const houseA = dumpHouse(OLD_A.rid, OLD_A.x, OLD_A.z);
  const houseB = dumpHouse(OLD_B.rid, OLD_B.x, OLD_B.z);
  check(houseA.blocks.length >= 30, `舊家A 方塊清單合理（${houseA.blocks.length} 塊，cy=${houseA.cy}）`);
  check(houseB.blocks.length >= 30, `舊家B 方塊清單合理（${houseB.blocks.length} 塊，cy=${houseB.cy}）`);

  // 玩家哨兵：箱子(42)/告示牌(66)/木板(8)平台，緊鄰舊家A 東側——搬家拆除絕不可動它們。
  const sentinels = [
    { x: OLD_A.x + 6, y: houseA.cy, z: OLD_A.z, b: 42 },
    { x: OLD_A.x + 7, y: houseA.cy, z: OLD_A.z, b: 66 },
    { x: OLD_A.x + 6, y: houseA.cy, z: OLD_A.z + 1, b: 8 },
    { x: OLD_A.x + 7, y: houseA.cy, z: OLD_A.z + 1, b: 8 },
  ];

  // data/voxel_resident_blocks.jsonl：兩座舊家 + 哨兵（append-only，伺服器啟動 replay）。
  const blockLines = [...houseA.blocks, ...houseB.blocks, ...sentinels]
    .map((b) => JSON.stringify({ x: b.x, y: b.y, z: b.z, b: b.b })).join("\n") + "\n";
  writeFileSync(join(dir, "data", "voxel_resident_blocks.jsonl"), blockLines);

  // data/voxel_goals.jsonl：兩位居民「小屋已完工」的持久事實（含錨點——搬家名單判定的輸入）。
  const goalLines = [
    { resident: OLD_A.rid, kind: "house", seq: 0, x: OLD_A.x, y: houseA.cy, z: OLD_A.z, expansion: false, anchor_only: false },
    { resident: OLD_B.rid, kind: "house", seq: 1, x: OLD_B.x, y: houseB.cy, z: OLD_B.z, expansion: false, anchor_only: false },
  ].map((o) => JSON.stringify(o)).join("\n") + "\n";
  writeFileSync(join(dir, "data", "voxel_goals.jsonl"), goalLines);

  // ── 啟動 + before 截圖 ────────────────────────────────────────────────────
  let proc = launchServer(dir);
  check(await waitReady(), "伺服器啟動就緒");
  let ui = await openBrowser();
  if (ui) {
    await screenshotAt(ui.page, OLD_A.x + 0.5, OLD_A.z + 6, 22, 0.9, "reloc-before-oldhomeA.png");
    await screenshotAt(ui.page, OLD_A.x + 0.5, OLD_A.z + 0.5, 40, 1.3, "reloc-before-oldhomeA-topdown.png");
    await screenshotAt(ui.page, 0.5, 19.5, 52, 1.3, "reloc-before-village-topdown.png");
    await ui.browser.close();
    ui = null;
  }

  // ── 第一位完整走完：動工 → 新家完工 → 回舊家拆 → 完成 ─────────────────────
  const startA = await waitFeed(dir, (e) => e.kind === "都更搬家" && e.detail.includes("開始把家搬到村裡"), 120000, "第一位搬家動工");
  check(!!startA, "Feed：第一位開始把家搬到村裡的新地塊");
  const demolishA = await waitFeed(dir, (e) => e.kind === "都更搬家" && e.detail.includes("回舊家把材料"), 300000, "第一位新家完工回舊家拆料");
  check(!!demolishA, "Feed：第一位新家蓋好、回舊家拆料");
  const doneA = await waitFeed(dir, (e) => e.kind === "都更搬家" && e.detail.includes("新家就在村裡的路旁"), 300000, "第一位搬家完成");
  check(!!doneA, "Feed：第一位搬家完成（舊家材料帶走）");
  const firstMover = startA ? startA.resident : "";

  // 第一位完成當下的世界快照：舊家A 拆光、哨兵與舊家B 毫髮無傷。
  if (doneA) {
    await sleep(2000); // 讓最後幾筆 append 落地
    const world = readResidentBlocks(dir);
    let removedA = 0;
    for (const b of houseA.blocks) {
      const cur = world.get(`${b.x},${b.y},${b.z}`);
      if (cur !== b.b) removedA++;
    }
    check(removedA >= houseA.blocks.length * 0.8,
      `舊家A 幾乎拆光（${removedA}/${houseA.blocks.length} 格已非原方塊）`);
    let sentinelTouched = 0;
    for (const s of sentinels) {
      if (world.get(`${s.x},${s.y},${s.z}`) !== s.b) sentinelTouched++;
    }
    check(sentinelTouched === 0, `玩家哨兵方塊（箱子/告示牌/平台）毫髮無傷（被動 ${sentinelTouched} 塊）`);
    let neighborTouched = 0;
    for (const b of houseB.blocks) {
      if (world.get(`${b.x},${b.y},${b.z}`) !== b.b) neighborTouched++;
    }
    check(neighborTouched === 0, `鄰居舊家B 在第一位搬完當下毫髮無傷（被動 ${neighborTouched} 塊）`);
    // 材料入包（server.log 的完成行帶 bag_total）。
    const log = readFileSync(join(OUT, "server.log"), "utf8");
    const m = log.match(/都更搬家：完成.*/) || log.match(/bag_total=(\d+).*都更搬家：完成/);
    const bagLine = log.split("\n").find((l) => l.includes("都更搬家：完成"));
    const bag = bagLine && bagLine.match(/bag_total=(\d+)/);
    check(!!bag && Number(bag[1]) > 0, `拆下的材料回收入她的背包（bag_total=${bag ? bag[1] : "?"}）`);
    void m;
  }

  // ── 第二位接著開始（錯開）＋ 中斷可恢復 ───────────────────────────────────
  const startB = await waitFeed(dir, (e) =>
    e.kind === "都更搬家" && e.detail.includes("開始把家搬到村裡") && e.resident !== firstMover,
    180000, "第二位搬家動工");
  check(!!startB, "Feed：第二位接著開始（錯開，一次只有一位）");
  if (startB && doneA) {
    check(startB.ts >= doneA.ts, `第二位動工不早於第一位完成（${startB.ts} ≥ ${doneA.ts}）`);
  }

  // 第二位進行中：重啟伺服器 → 驗中斷可恢復（拆除冪等重算、計畫續蓋）。
  console.log("[重啟] 第二位搬家進行中，SIGTERM 後重啟同一份 data/ …");
  killExact(proc);
  await sleep(2500);
  proc = launchServer(dir);
  check(await waitReady(), "重啟後伺服器就緒（同一份 data/）");
  const doneB = await waitFeed(dir, (e) =>
    e.kind === "都更搬家" && e.detail.includes("新家就在村裡的路旁") && e.resident !== firstMover,
    420000, "第二位搬家完成（跨重啟）");
  check(!!doneB, "Feed：第二位跨重啟仍完成搬家（中斷可恢復）");
  if (doneB) {
    await sleep(2000);
    const world = readResidentBlocks(dir);
    let removedB = 0;
    for (const b of houseB.blocks) {
      if (world.get(`${b.x},${b.y},${b.z}`) !== b.b) removedB++;
    }
    check(removedB >= houseB.blocks.length * 0.8,
      `舊家B 幾乎拆光（${removedB}/${houseB.blocks.length} 格已非原方塊）`);
    let sentinelTouched = 0;
    for (const s of sentinels) {
      if (world.get(`${s.x},${s.y},${s.z}`) !== s.b) sentinelTouched++;
    }
    check(sentinelTouched === 0, "全程結束後玩家哨兵方塊仍毫髮無傷");
  }

  // ── after 截圖：舊家址（拆光）、新家址（村裡地塊上）、村中心 ─────────────────
  // 新家錨點從搬家記錄讀（jsonl 持久檔）。
  let newA = null;
  try {
    const recs = readFileSync(join(dir, "data", "voxel_relocations.jsonl"), "utf8")
      .split("\n").filter(Boolean).map((l) => JSON.parse(l));
    const doneRec = recs.filter((r) => r.phase === "done");
    if (doneRec.length) newA = doneRec[0];
    check(doneRec.length >= 1, `搬家進度 jsonl 有 done 記錄（${doneRec.length} 筆）`);
  } catch { check(false, "讀不到 voxel_relocations.jsonl"); }
  ui = await openBrowser();
  if (ui) {
    await screenshotAt(ui.page, OLD_A.x + 0.5, OLD_A.z + 6, 22, 0.9, "reloc-after-oldhomeA.png");
    await screenshotAt(ui.page, OLD_A.x + 0.5, OLD_A.z + 0.5, 40, 1.3, "reloc-after-oldhomeA-topdown.png");
    if (newA) {
      await screenshotAt(ui.page, newA.new_x + 0.5, newA.new_z + 8, 22, 0.9, "reloc-after-newhomeA.png");
    }
    await screenshotAt(ui.page, 0.5, 19.5, 52, 1.3, "reloc-after-village-topdown.png");
    await ui.browser.close();
  }

  killExact(proc);
  await sleep(1200);
  console.log("\n=== 結果：" + (failures === 0 ? "全綠 ✅" : failures + " 項失敗 ❌") + " ===");
  console.log("截圖/日誌在：" + OUT + "（隔離 data/ 在 " + dir + "）");
  process.exit(failures === 0 ? 0 : 1);
})();
