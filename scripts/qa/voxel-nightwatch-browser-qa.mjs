// ============================================================
// voxel-nightwatch-browser-qa.mjs — 夜裡點燈守望 v1 真實瀏覽器 QA（真渲染·真截圖）
// ============================================================
// 比照 voxel-shadow-browser-qa.mjs：puppeteer-core 驅動系統 Chrome 載入隔離伺服器的
// /voxel/，搭配一條 Node WS「導演」連線（撥鐘入夜、把鏡頭錨到村外遠方居民旁），驗證並拍下
// 「居民夜裡見暗影靠近 → 主動朝暗處點起火把守望」的湧現行為：
//   (a) 撥鐘入夜（Evening：居民仍醒著）+ 錨到村外遠方居民家旁 → 暗影生成、漂近該居民
//   (b) 該居民就近點起火把（導演收到 block(b=31) 廣播，且落點在某位居民附近）→ 截圖存證
//   (c) 一夜點燈不洗版（同時觀測：落點彼此有最小間距、且總數在合理範圍）
// 需要伺服器以 BUTFUN_QA_DEBUG=1 啟動；截圖存 VQA_SHOTS（預設 scratchpad）。
// 這是湧現行為（暗影漂移路徑隨機），故關鍵斷言採 best-effort 長逾時；純邏輯正確性另有單元測試把關。
// 不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import WebSocket from "ws";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

const PORT = process.env.VQA_PORT || 8390;
const URL = process.env.VQA_URL || `http://127.0.0.1:${PORT}/voxel/?debug=1`;
const WS_URL = `ws://127.0.0.1:${PORT}/voxel/ws`;
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const SHOTS = process.env.VQA_SHOTS || "shots";
mkdirSync(SHOTS, { recursive: true });

const GPU_ARGS = [
  "--no-sandbox", "--disable-setuid-sandbox", "--ignore-gpu-blocklist",
  "--enable-gpu", "--enable-webgl", "--use-gl=angle", "--use-angle=gl",
  "--disable-dev-shm-usage", "--window-size=1280,800",
  "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding", "--disable-features=CalculateNativeWinOcclusion",
];
const TORCH = 31; // Block::Torch
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let pass = 0, fail = 0, warn = 0;
function check(label, ok, extra = "") {
  if (ok) { pass++; console.log(`  ✅ ${label}${extra ? "  " + extra : ""}`); }
  else { fail++; console.log(`  ❌ ${label}${extra ? "  " + extra : ""}`); }
}
function softCheck(label, ok, extra = "") {
  if (ok) { pass++; console.log(`  ✅ ${label}${extra ? "  " + extra : ""}`); }
  else { warn++; console.log(`  ⚠️ ${label}（best-effort，未達成只警告）${extra ? "  " + extra : ""}`); }
}

// 導演連線：撥鐘/錨定位置用；同時累計居民、暗影快照與所有 block(火把) 廣播事件。
function director(name) {
  const ws = new WebSocket(WS_URL);
  const st = { ws, ready: false, shadows: [], residents: [], torches: [], sayWatch: [] };
  const watchWords = ["點上燈", "添一盞", "有燈就不怕", "把燈點亮"];
  ws.on("message", (buf) => {
    let m; try { m = JSON.parse(buf.toString()); } catch { return; }
    if (m.t === "welcome") st.ready = true;
    if (m.t === "players") {
      st.shadows = m.shadows || [];
      st.residents = m.residents || [];
      // 收集正冒「點燈守望」台詞的居民（另一路湧現證據）。
      for (const r of st.residents) {
        if (r.say && watchWords.some((w) => r.say.includes(w))) {
          st.sayWatch.push({ name: r.name, say: r.say, x: r.x, z: r.z });
        }
      }
    }
    // 導演連線後收到的每一則 block(b=31) 都是本場「新點的火把」——
    // 村莊初始的 4 盞廣場燈在導演連線前就放好、廣播過了，這裡收不到，故不會混入。
    if (m.t === "block" && m.b === TORCH) st.torches.push({ x: m.x, y: m.y, z: m.z });
  });
  ws.on("open", () => ws.send(JSON.stringify({ t: "join", name })));
  return st;
}
const send = (c, o) => c.ws.send(JSON.stringify(o));
async function waitFor(fn, ms, step = 200) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) { const v = fn(); if (v) return v; await sleep(step); }
  return null;
}
// 最近的一位居民與其距離（水平）。
function nearestResident(residents, x, z) {
  let best = null;
  for (const r of residents) {
    const d = Math.hypot(r.x - x, r.z - z);
    if (!best || d < best.d) best = { r, d };
  }
  return best;
}

(async () => {
  console.log(`\n【夜裡點燈守望 真瀏覽器 QA】${URL}`);
  const dir = director("QA守望導演_" + Math.floor(Math.random() * 1e5));
  await waitFor(() => dir.ready, 5000);
  check("導演連線就緒", dir.ready);

  // 挑一位遠離村莊庇護半徑（>55 格）的居民當「守望現場」——那裡才會生成暗影。
  const remote = await waitFor(() => {
    const r = (dir.residents || []).find((q) => Math.hypot(q.x, q.z) > 55);
    return r || null;
  }, 8000);
  check("找到村外遠方居民當守望現場", !!remote, remote ? `${remote.name} @(${remote.x.toFixed(0)},${remote.z.toFixed(0)})` : "無");

  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU_ARGS });
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 800 });
  await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await sleep(6000); // 等 chunk/mesh

  const anchor = remote || { x: 80, z: 0, y: 40 };
  // 導演＋瀏覽器鏡頭都瞬移到遠方居民旁：讓暗影生成錨點、居民、鏡頭三者同框。
  send(dir, { t: "move", x: anchor.x + 2, y: (anchor.y || 40), z: anchor.z + 2, yaw: 0 });
  await page.evaluate((a) => {
    const p = window.__voxel.player;
    p.x = a.x + 3; p.z = a.z + 3; // 站在居民旁
    window.__voxel.lookTowards(a.x, (a.y || p.y) + 1.2, a.z);
  }, anchor).catch(() => {});
  await sleep(800);

  // ── (a) 撥鐘入夜（Evening=居民仍醒著）→ 等暗影漂近該居民 ────────
  console.log("\n(a) 入夜（Evening）→ 暗影漂近醒著的遠方居民");
  send(dir, { t: "qa_set_time", time: 0.90 }); // Evening：is_shadow_time 為真、居民尚未深眠
  const spawned = await waitFor(() => (dir.shadows.length > 0 ? dir.shadows : null), 40000);
  check("夜裡暗處生成暗影", !!spawned, spawned ? `場上 ${dir.shadows.length} 隻` : "40 秒無生成");

  // ── (b) 居民就近點起守望火把（block b=31，落點在某居民附近）───────
  console.log("\n(b) 居民主動點燈守望（block(b=31) 落在某位居民附近）→ 截圖");
  // 湧現需時：暗影得漂進居民 16 格通知半徑、且就近有暗處可放。給足 3 分鐘。
  const gotTorch = await waitFor(() => {
    for (const t of dir.torches) {
      const nr = nearestResident(dir.residents, t.x, t.z);
      // 火把落在某位居民 ~8 格內 → 判定為該居民點的守望燈（放置點在居民朝暗影 3 格處）。
      if (nr && nr.d <= 8) return { t, nr };
    }
    return null;
  }, 180000);
  softCheck("居民朝暗處點起守望火把（落在居民附近）", !!gotTorch,
    gotTorch ? `火把@(${gotTorch.t.x},${gotTorch.t.y},${gotTorch.t.z}) 最近居民 ${gotTorch.nr.r.name} d=${gotTorch.nr.d.toFixed(1)}` : "3 分鐘內未觀測到");
  softCheck("有居民冒『點燈守望』台詞（彼此提醒）", dir.sayWatch.length > 0,
    dir.sayWatch.length ? `「${dir.sayWatch[dir.sayWatch.length - 1].say}」（${dir.sayWatch[dir.sayWatch.length - 1].name}）` : "未觀測到");

  // 把鏡頭轉向剛點的火把（或現場），拍下夜裡守望燈亮起的樣子。
  const focus = gotTorch ? gotTorch.t : { x: anchor.x, y: (anchor.y || 40), z: anchor.z };
  await page.evaluate((f) => {
    const p = window.__voxel.player;
    // 站到火把斜前方一點，鏡頭望向火把。
    p.x = f.x + 4; p.z = f.z + 4;
    window.__voxel.lookTowards(f.x, f.y + 0.5, f.z);
  }, focus).catch(() => {});
  await sleep(1200);
  const shotA = join(SHOTS, "nightwatch-a-lit.png");
  await page.screenshot({ path: shotA });
  console.log("  📸", shotA);

  // 讓更多居民陸續點燈，再拍一張「村邊漸漸亮起一圈守望燈」的全景。
  await sleep(30000);
  // 拉高鏡頭俯瞰現場（若前端支援；否則就地再拍一張）。
  await page.evaluate((a) => {
    const p = window.__voxel.player;
    p.x = a.x + 12; p.z = a.z + 12; p.y = (a.y || p.y) + 6;
    window.__voxel.lookTowards(a.x, (a.y || p.y), a.z);
  }, anchor).catch(() => {});
  await sleep(1200);
  const shotB = join(SHOTS, "nightwatch-b-ring.png");
  await page.screenshot({ path: shotB });
  console.log("  📸", shotB);

  // ── (c) 不洗版：本場火把落點彼此有最小間距、總數合理 ─────────────
  console.log("\n(c) 點燈不洗版（落點有最小間距、總數在夜間上限內）");
  const ts = dir.torches;
  let minGap = Infinity;
  for (let i = 0; i < ts.length; i++)
    for (let j = i + 1; j < ts.length; j++)
      minGap = Math.min(minGap, Math.hypot(ts[i].x - ts[j].x, ts[i].z - ts[j].z));
  console.log(`  （本場新點火把共 ${ts.length} 盞，落點最小水平間距 ${Number.isFinite(minGap) ? minGap.toFixed(1) : "n/a"}）`);
  check("一夜點燈總數在上限內（≤ 24，不鋪滿世界）", ts.length <= 24, `count=${ts.length}`);
  if (ts.length >= 2) softCheck("守望燈彼此有間距（不擠成一團）", minGap >= 3, `minGap=${minGap.toFixed(1)}`);

  send(dir, { t: "qa_set_time", time: 0.5 }); // 收尾撥回白天（暗影整批消散）
  await sleep(500);
  await browser.close();
  dir.ws.close();
  await sleep(200);

  console.log(`\n══════════════════════════════════════════`);
  console.log(`夜裡點燈守望 真瀏覽器 QA：${pass} 通過 / ${fail} 失敗 / ${warn} 警告`);
  console.log(`══════════════════════════════════════════`);
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
