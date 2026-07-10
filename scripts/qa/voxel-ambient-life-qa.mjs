// ============================================================
// voxel-ambient-life-qa.mjs — 世界環境氛圍 v1（905）真實瀏覽器 QA
// ============================================================
// 自包含：自起隔離伺服器（換埠、記憶體模式、BUTFUN_QA_DEBUG=1）→ 換埠冒煙
//   （/healthz、/voxel/、/voxel/main.js 皆 200）→ puppeteer-core 驅動系統 Chrome
//   真渲染載入 /voxel/ → 導演 WS 撥鐘 → 拍下兩張證據截圖：
//     (a) 白天草原/花叢邊的蝴蝶
//     (b) 夜裡水邊/暗處的螢火蟲微光
//   同場量真實 rAF FPS（白天 vs 夜間全開，驗無回歸）→ 精確 PID SIGTERM 收尾。
// 不抄外部碼；全繁中註解；node --check 過。
//
// 用法：BFQA_CHROME=/usr/bin/google-chrome node scripts/qa/voxel-ambient-life-qa.mjs
// 需先 cargo build（debug 或 release 皆可，見 BIN 解析）。

import puppeteer from "puppeteer-core";
import WebSocket from "ws";
import { spawn } from "node:child_process";
import { mkdirSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO = join(__dirname, "..", "..");
const PORT = Number(process.env.VQA_PORT || 39457);
const BASE = `http://127.0.0.1:${PORT}`;
const URL = `${BASE}/voxel/?debug=1`;
const WS_URL = `ws://127.0.0.1:${PORT}/voxel/ws`;
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const SHOTS = process.env.VQA_SHOTS || join(REPO, "scratchpad", "ambient-shots");
mkdirSync(SHOTS, { recursive: true });

const BIN_REL = join(REPO, "target", "release", "butfun-server");
const BIN_DBG = join(REPO, "target", "debug", "butfun-server");
const BIN = existsSync(BIN_REL) ? BIN_REL : BIN_DBG;

const GPU_ARGS = [
  "--no-sandbox", "--disable-setuid-sandbox", "--ignore-gpu-blocklist",
  "--enable-gpu", "--enable-webgl", "--use-gl=angle", "--use-angle=gl",
  "--disable-dev-shm-usage", "--window-size=1280,800",
  "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding", "--disable-features=CalculateNativeWinOcclusion",
];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let pass = 0, fail = 0;
function check(label, ok, extra = "") {
  if (ok) { pass++; console.log(`  ✅ ${label}${extra ? "  " + extra : ""}`); }
  else { fail++; console.log(`  ❌ ${label}${extra ? "  " + extra : ""}`); }
}

// 導演連線：撥鐘用（世界共享，導演撥鐘瀏覽器端也跟著換時段）。
function director(name) {
  const ws = new WebSocket(WS_URL);
  const st = { ws, ready: false };
  ws.on("message", (buf) => {
    let m; try { m = JSON.parse(buf.toString()); } catch { return; }
    if (m.t === "welcome") st.ready = true;
  });
  ws.on("open", () => ws.send(JSON.stringify({ t: "join", name })));
  return st;
}
const send = (c, o) => c.ws.send(JSON.stringify(o));
async function waitFor(fn, ms, step = 200) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) { const v = await fn(); if (v) return v; await sleep(step); }
  return null;
}

let server = null;
async function startServer() {
  console.log(`【啟動隔離伺服器】${BIN}  PORT=${PORT}（記憶體模式・QA_DEBUG）`);
  server = spawn(BIN, [], {
    cwd: REPO,
    env: { ...process.env, PORT: String(PORT), BUTFUN_QA_DEBUG: "1", RUST_LOG: "warn" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  server.stdout.on("data", () => {});
  server.stderr.on("data", () => {});
  const up = await waitFor(async () => {
    try { const r = await fetch(`${BASE}/healthz`); return r.ok; } catch { return false; }
  }, 20000, 400);
  if (!up) throw new Error("伺服器 20 秒內未就緒");
  console.log(`  伺服器就緒 pid=${server.pid}`);
}
function stopServer() {
  if (server && server.pid) {
    try { process.kill(server.pid, "SIGTERM"); } catch {}
  }
}

(async () => {
  console.log(`\n【世界環境氛圍 v1（905）真瀏覽器 QA】`);
  await startServer();

  // ── 換埠冒煙：三條關鍵路由皆 200 ─────────────────────────
  for (const [label, path] of [["/healthz", "/healthz"], ["/voxel/", "/voxel/"], ["/voxel/main.js", "/voxel/main.js"]]) {
    let ok = false, code = 0;
    try { const r = await fetch(`${BASE}${path}`); code = r.status; ok = r.ok; } catch {}
    check(`冒煙 ${label} 回 200`, ok, `HTTP ${code}`);
  }

  const dir = director("QA導演_" + Math.floor(Math.random() * 1e5));
  await waitFor(async () => dir.ready, 5000);

  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU_ARGS });
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 800 });
  await page.evaluateOnNewDocument(() => {
    window.__fps = { frames: 0, t0: performance.now() };
    const raf = window.requestAnimationFrame.bind(window);
    window.requestAnimationFrame = (cb) => raf((t) => { window.__fps.frames++; cb(t); });
  });
  await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await sleep(6000); // 等 chunk/mesh

  const counts = async () => page.evaluate(() => window.__voxel.ambientCounts);
  const lookGround = async () => page.evaluate(() => {
    const p = window.__voxel.player;
    // 朝前下方地面看，讓貼近地表的環境微生物入鏡。
    window.__voxel.lookTowards(p.x + 6, p.y - 1.5, p.z + 6);
  });
  // 把視角轉向最近的一隻該型微生物（取景保證入鏡）。
  const frameNearest = async (kind) => page.evaluate((k) => {
    const p = window.__voxel.player;
    const list = window.__voxel.ambientCounts[k];
    if (!list || !list.length) return false;
    let best = list[0], bd = Infinity;
    for (const c of list) { const d = (c.x - p.x) ** 2 + (c.z - p.z) ** 2; if (d < bd) { bd = d; best = c; } }
    window.__voxel.lookTowards(best.x, best.y, best.z);
    return true;
  }, kind);

  // ── (a) 白天蝴蝶 ─────────────────────────────────────────
  console.log("(a) 白天蝴蝶截圖");
  send(dir, { t: "qa_set_time", time: 0.5 }); // 正午：nightFactor=0 → 出蝴蝶
  await sleep(600);
  await lookGround();
  // 等蝴蝶累積（節流 0.55 秒/次生成一隻，等到數隻同框畫面才夠有感）。
  const bOk = await waitFor(async () => ((await counts()).butterflies >= 3 ? true : null), 25000, 500);
  const cDay = await counts();
  check("白天草地/花叢邊生出蝴蝶（≥3 隻同框）", !!bOk, `蝴蝶 ${cDay.butterflies} 隻／螢火蟲 ${cDay.fireflies} 隻`);
  await frameNearest("butterflyPos");
  await sleep(800);
  await page.screenshot({ path: join(SHOTS, "a-butterflies-day.png") });
  console.log("  📸", join(SHOTS, "a-butterflies-day.png"));

  // 白天 FPS 基線
  await page.evaluate(() => { window.__fps = { frames: 0, t0: performance.now() }; });
  await sleep(5000);
  const fpsDay = await page.evaluate(() => window.__fps.frames / ((performance.now() - window.__fps.t0) / 1000));
  console.log(`  （白天 rAF ${fpsDay.toFixed(1)} fps，蝴蝶全開）`);

  // ── (b) 夜間螢火蟲 ───────────────────────────────────────
  console.log("(b) 夜間螢火蟲截圖");
  send(dir, { t: "qa_set_time", time: 0.0 }); // 深夜：nightFactor=1 → 出螢火蟲（蝴蝶淡出）
  await sleep(800);
  await lookGround();
  const fOk = await waitFor(async () => ((await counts()).fireflies >= 4 ? true : null), 25000, 500);
  const cNight = await counts();
  check("夜裡水邊/暗處浮出螢火蟲微光（≥4 點同框）", !!fOk, `螢火蟲 ${cNight.fireflies} 隻／蝴蝶 ${cNight.butterflies} 隻`);
  check("換到夜間後蝴蝶淡出回收", cNight.butterflies <= cDay.butterflies);
  await frameNearest("fireflyPos");
  await sleep(800);
  await page.screenshot({ path: join(SHOTS, "b-fireflies-night.png") });
  console.log("  📸", join(SHOTS, "b-fireflies-night.png"));

  // 夜間 FPS（螢火蟲全開，與白天相對比較——同機況、絕對值隨主機負載浮動）
  await page.evaluate(() => { window.__fps = { frames: 0, t0: performance.now() }; });
  await sleep(5000);
  const fpsNight = await page.evaluate(() => window.__fps.frames / ((performance.now() - window.__fps.t0) / 1000));
  check("夜間螢火蟲全開不掉幀（≥ 白天基線 − 8）", fpsNight >= fpsDay - 8,
    `夜 ${fpsNight.toFixed(1)} fps vs 日 ${fpsDay.toFixed(1)} fps${fpsNight < 45 ? "（絕對值偏低=主機負載，非本功能）" : ""}`);

  send(dir, { t: "qa_set_time", time: 0.5 });
  await sleep(300);
  dir.ws.close();
  await browser.close();

  console.log(`\n══════════════════════════════════════════`);
  console.log(`世界環境氛圍 v1 真瀏覽器 QA：${pass} 通過 / ${fail} 失敗`);
  console.log(`══════════════════════════════════════════`);
  stopServer();
  await sleep(500);
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); stopServer(); process.exit(2); });
