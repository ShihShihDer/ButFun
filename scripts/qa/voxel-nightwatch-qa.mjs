// ============================================================
// voxel-nightwatch-qa.mjs — 夜裡點燈守望 v1 WS 功能 QA
// ============================================================
// 用真 WebSocket 直連隔離伺服器（記憶體/jsonl 模式、獨立 port），驗後端權威湧現行為：
//   (a) 撥鐘入夜（Evening=居民仍醒著）+ 站到村外遠方居民旁 → 暗處生成暗影、漂近該居民
//   (b) 該居民朝暗處就近點起火把守望（收到 block(b=31) 廣播，且落點在某位居民附近）
//   (c) 有居民冒「點燈守望」台詞（彼此提醒）
//   (d) 點燈不洗版：本場火把落點彼此有最小間距、總數在夜間上限（24）內
//   (e) 撥回白天 → 暗影整批消散（不再點燈）
// 需要伺服器以 BUTFUN_QA_DEBUG=1 啟動（qa_set_time 才生效；正式線上惰性忽略）。
// 湧現行為（暗影漂移路徑隨機）故關鍵斷言採 best-effort 長逾時；純邏輯正確性另有單元測試把關。
// 不抄外部碼；全繁中註解；node --check 過。

import WebSocket from "ws";

const PORT = process.env.VQA_PORT || 8390;
const WS_URL = `ws://127.0.0.1:${PORT}/voxel/ws`;
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

function makeClient(name) {
  const ws = new WebSocket(WS_URL);
  const st = {
    ws, name, ready: false, welcome: null,
    shadows: [], residents: [],
    torches: [],   // 連線後收到的每一則 block(b=31)＝本場新點的守望火把
    sayWatch: [],  // 觀測到冒「點燈守望」台詞的居民
  };
  const watchWords = ["點上燈", "添一盞", "有燈就不怕", "把燈點亮"];
  ws.on("message", (buf) => {
    let m; try { m = JSON.parse(buf.toString()); } catch { return; }
    switch (m.t) {
      case "welcome": st.welcome = m; st.ready = true; break;
      case "players":
        st.shadows = m.shadows || [];
        st.residents = m.residents || [];
        for (const r of st.residents) {
          if (r.say && watchWords.some((w) => r.say.includes(w)))
            st.sayWatch.push({ name: r.name, say: r.say });
        }
        break;
      case "block":
        // 村莊初始的 4 盞廣場燈在本連線前就放好廣播過了，這裡收不到 → 不混入。
        if (m.b === TORCH) st.torches.push({ x: m.x, y: m.y, z: m.z });
        break;
    }
  });
  return st;
}
const send = (c, o) => c.ws.send(JSON.stringify(o));
async function waitReady(c, ms = 5000) {
  const t0 = Date.now();
  while (!c.ready && Date.now() - t0 < ms) await sleep(50);
  return c.ready;
}
async function waitFor(fn, ms, step = 200) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) { const v = fn(); if (v) return v; await sleep(step); }
  return null;
}
function nearestResident(residents, x, z) {
  let best = null;
  for (const r of residents) {
    const d = Math.hypot(r.x - x, r.z - z);
    if (!best || d < best.d) best = { r, d };
  }
  return best;
}

(async () => {
  console.log(`\n【夜裡點燈守望 WS 功能 QA】連 ${WS_URL}`);
  const NAME = "QA守夜人_" + Math.floor(Math.random() * 100000);
  const c = makeClient(NAME);
  await new Promise((res, rej) => { c.ws.on("open", res); c.ws.on("error", rej); });
  send(c, { t: "join", name: NAME });
  const ok = await waitReady(c);
  check("進場收到 welcome", ok);
  if (!ok) { c.ws.close(); process.exit(2); }
  const spawn = c.welcome.spawn;
  await sleep(600);

  // 找一位遠離村莊庇護半徑（>55 格）的醒著居民，站到他家旁——那裡才會生成暗影、且他會點燈守望。
  const remote = await waitFor(() => (c.residents.find((r) => Math.hypot(r.x, r.z) > 55) || null), 8000);
  check("找到村外遠方居民當守望現場", !!remote, remote ? `${remote.name} @(${remote.x.toFixed(0)},${remote.z.toFixed(0)})` : "無");
  const site = remote || { x: spawn.x + 150, z: spawn.z, y: spawn.y };

  // ── (a) 入夜（Evening）+ 站到遠方居民旁 → 暗處生成暗影、漂近 ─────
  console.log("\n(a) 入夜（Evening）→ 暗處生成暗影、漂近醒著的遠方居民");
  send(c, { t: "move", x: site.x + 3, y: site.y ?? spawn.y, z: site.z + 3, yaw: 0 });
  await sleep(400);
  send(c, { t: "qa_set_time", time: 0.86 }); // 入夜起點（整夜約 200 秒真實時間）
  const spawned = await waitFor(() => (c.shadows.length > 0 ? c.shadows : null), 40000);
  check("夜裡暗處生成暗影", !!spawned, spawned ? `場上 ${c.shadows.length} 隻，第一隻 @(${spawned[0].x.toFixed(1)},${spawned[0].z.toFixed(1)})` : "40 秒無生成");

  // ── (b) 居民朝暗處就近點起守望火把（block b=31 落在某居民附近）───
  console.log("\n(b) 居民主動點燈守望（block(b=31) 落在某位居民附近）");
  // 湧現需時：暗影得漂進居民 16 格通知半徑、且就近有暗處可放。守望檢查每 4 秒一次。給足 3 分鐘。
  const gotTorch = await waitFor(() => {
    for (const t of c.torches) {
      const nr = nearestResident(c.residents, t.x, t.z);
      if (nr && nr.d <= 8) return { t, nr }; // 落在某居民 ~8 格內＝該居民點的守望燈
    }
    return null;
  }, 180000);
  check("居民朝暗處點起守望火把（落點在某位居民附近）", !!gotTorch,
    gotTorch ? `火把@(${gotTorch.t.x},${gotTorch.t.y},${gotTorch.t.z}) 最近居民 ${gotTorch.nr.r.name} d=${gotTorch.nr.d.toFixed(1)}` : "3 分鐘內未觀測到");

  // ── (c) 彼此提醒：有居民冒「點燈守望」台詞 ────────────────────
  console.log("\n(c) 彼此提醒（有居民冒點燈守望台詞）");
  softCheck("有居民冒『點燈守望』台詞", c.sayWatch.length > 0,
    c.sayWatch.length ? `「${c.sayWatch[c.sayWatch.length - 1].say}」（${c.sayWatch[c.sayWatch.length - 1].name}）` : "未觀測到");

  // 再守一會，讓更多居民陸續點燈（觀測收斂/間距）。
  await sleep(30000);

  // ── (d) 不洗版：落點彼此有最小間距、總數在夜間上限內 ───────────
  console.log("\n(d) 點燈不洗版（落點有最小間距、總數 ≤ 夜間上限 24）");
  const ts = c.torches;
  let minGap = Infinity;
  for (let i = 0; i < ts.length; i++)
    for (let j = i + 1; j < ts.length; j++)
      minGap = Math.min(minGap, Math.hypot(ts[i].x - ts[j].x, ts[i].z - ts[j].z));
  console.log(`  （本場新點守望火把共 ${ts.length} 盞，落點最小水平間距 ${Number.isFinite(minGap) ? minGap.toFixed(1) : "n/a"}）`);
  check("一夜點燈總數在上限內（≤ 24，不鋪滿世界）", ts.length <= 24, `count=${ts.length}`);
  if (ts.length >= 2) softCheck("守望燈彼此有間距（不擠成一團）", minGap >= 3, `minGap=${minGap.toFixed(1)}`);
  check("療癒底線：居民一位都沒少（只會怕/點燈、不會死）", c.residents.length >= 4, `居民=${c.residents.length}`);

  // ── (e) 黎明 → 暗影整批消散（不再點燈）─────────────────────────
  console.log("\n(e) 撥回白天 → 暗影整批消散");
  send(c, { t: "qa_set_time", time: 0.5 });
  const cleared = await waitFor(() => (c.shadows.length === 0 ? true : null), 4000);
  check("撥回白天後暗影全部消散", !!cleared, `shadows=${c.shadows.length}`);

  await sleep(300);
  c.ws.close();
  await sleep(200);

  console.log(`\n══════════════════════════════════════════`);
  console.log(`夜裡點燈守望 WS 功能 QA：${pass} 通過 / ${fail} 失敗 / ${warn} 警告`);
  console.log(`══════════════════════════════════════════`);
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
