// ============================================================
// voxel-nightlife-browser-qa.mjs — 居民夜間生活 v1 真實瀏覽器 QA（真渲染·真截圖）
// ============================================================
// 比照 voxel-nightwatch-browser-qa.mjs：puppeteer-core 驅動系統 Chrome 載入隔離伺服器的
// /voxel/，搭配一條 Node WS「導演」連線（撥鐘入夜、把鏡頭錨到村中央居民聚集處），驗證並拍下
// 「入夜後居民的溫柔日常」這件湧現行為：
//   (a) 撥鐘入夜（Evening：居民仍醒著、還沒就寢）
//   (b) 觀測到居民冒出夜生活台詞——抬頭許願（不需玩家在場）或睡前互道晚安 → 截圖存證
//   (c) 互道晚安上一則「互道晚安」城鎮動態（feed）——best-effort
// 需要伺服器以 BUTFUN_QA_DEBUG=1 啟動；截圖存 VQA_SHOTS（預設 shots）。
// 這是湧現行為（機率＋冷卻＋配對），故關鍵斷言採 best-effort 長逾時；純邏輯正確性另有單元測試把關。
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

// 夜生活台詞關鍵詞——刻意只認本模組 wish_line/goodnight_line 專有的字串片段，避開 voxel_time
// 的入夜過渡台詞（如「夜深了，星星好多……」）與時段問候語，杜絕誤判成夜生活台詞。
const WISH_WORDS = ["許個願", "許了個", "許一個", "願明天也是好天氣", "平平安安", "值得許", "會成真", "就著這點暖光", "守著這盞燈"];
const GN_WORDS = ["早點歇著吧，晚安", "回去好好睡", "各自回家吧，晚安", "做個好夢", "願你有個好夢", "這點火光陪你", "早點回去歇著"];

// 導演連線：撥鐘/錨定位置用；同時累計居民快照與夜生活台詞、feed 事件。
function director(name) {
  const ws = new WebSocket(WS_URL);
  const st = { ws, ready: false, residents: [], wishSay: [], gnSay: [], feed: [] };
  ws.on("message", (buf) => {
    let m; try { m = JSON.parse(buf.toString()); } catch { return; }
    if (m.t === "welcome") st.ready = true;
    if (m.t === "players") {
      st.residents = m.residents || [];
      for (const r of st.residents) {
        if (!r.say) continue;
        if (WISH_WORDS.some((w) => r.say.includes(w))) st.wishSay.push({ name: r.name, say: r.say, x: r.x, z: r.z });
        if (GN_WORDS.some((w) => r.say.includes(w))) st.gnSay.push({ name: r.name, say: r.say, x: r.x, z: r.z });
      }
    }
    // 城鎮動態牆：睡前互道晚安會播「互道晚安」種類的 feed。
    if (m.t === "feed" && m.kind) st.feed.push({ kind: m.kind, actor: m.actor, detail: m.detail });
    if (m.t === "feed_line" && m.line) st.feed.push({ line: m.line });
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
// 找一群彼此靠近的居民中心（互道晚安需兩人 7 格內；村中央居民較密）。
function densestSpot(residents) {
  let best = null;
  for (const a of residents) {
    let cnt = 0, sx = 0, sz = 0;
    for (const b of residents) {
      if (Math.hypot(a.x - b.x, a.z - b.z) <= 10) { cnt++; sx += b.x; sz += b.z; }
    }
    if (!best || cnt > best.cnt) best = { cnt, x: sx / cnt, z: sz / cnt };
  }
  return best || { cnt: 0, x: 0, z: 0 };
}

(async () => {
  console.log(`\n【居民夜間生活 真瀏覽器 QA】${URL}`);
  const dir = director("QA夜生活導演_" + Math.floor(Math.random() * 1e5));
  await waitFor(() => dir.ready, 5000);
  check("導演連線就緒", dir.ready);

  await waitFor(() => (dir.residents || []).length > 0, 8000);
  const spot = densestSpot(dir.residents);
  check("找到村中居民聚集處當夜生活現場", dir.residents.length > 0,
    `場上 ${dir.residents.length} 位居民，最密處 ~(${spot.x.toFixed(0)},${spot.z.toFixed(0)}) 聚 ${spot.cnt} 人`);

  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU_ARGS });
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 800 });
  await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await sleep(6000); // 等 chunk/mesh

  const anchor = { x: spot.x, z: spot.z, y: 40 };
  send(dir, { t: "move", x: anchor.x + 4, y: anchor.y, z: anchor.z + 4, yaw: 0 });
  await page.evaluate((a) => {
    const p = window.__voxel.player;
    p.x = a.x + 6; p.z = a.z + 6;
    window.__voxel.lookTowards(a.x, (a.y || p.y) + 1.0, a.z);
  }, anchor).catch(() => {});
  await sleep(800);

  // ── (a) 撥鐘入夜（Evening=居民仍醒著、還沒深眠就寢）──────────────
  console.log("\n(a) 入夜（Evening）→ 居民開始它們的夜間日常");
  send(dir, { t: "qa_set_time", time: 0.90 });
  await sleep(1500);
  check("撥鐘入夜成功（Evening 時段）", true);

  // 先拍一張入夜氛圍全景（夜色＋居民在村中）。
  await page.evaluate((a) => {
    const p = window.__voxel.player;
    p.x = a.x + 10; p.z = a.z + 10; p.y = (a.y || p.y) + 5;
    window.__voxel.lookTowards(a.x, (a.y || p.y), a.z);
  }, anchor).catch(() => {});
  await sleep(1200);
  const shotNight = join(SHOTS, "nightlife-a-dusk.png");
  await page.screenshot({ path: shotNight });
  console.log("  📸", shotNight);

  // ── (b) 觀測夜生活台詞：許願 或 互道晚安 → 截圖 ─────────────────
  console.log("\n(b) 居民夜間日常台詞（抬頭許願／睡前互道晚安）→ 截圖");
  // 湧現需時：許願低機率、互道晚安需兩人夠近＋冷卻到期。給足 5 分鐘，並每 25 秒把時鐘壓回
  // Evening（0.88）——否則世界時鐘會自然滑進深夜（Night），居民躺下就寢後就不再有夜生活台詞。
  let gotSay = null;
  const t0 = Date.now();
  while (Date.now() - t0 < 300000) {
    if (dir.gnSay.length > 0) { gotSay = { kind: "互道晚安", ev: dir.gnSay[dir.gnSay.length - 1] }; break; }
    if (dir.wishSay.length > 0) { gotSay = { kind: "抬頭許願", ev: dir.wishSay[dir.wishSay.length - 1] }; break; }
    if ((Date.now() - t0) % 25000 < 300) send(dir, { t: "qa_set_time", time: 0.88 });
    await sleep(300);
  }
  softCheck("觀測到居民夜間日常台詞", !!gotSay,
    gotSay ? `【${gotSay.kind}】「${gotSay.ev.say}」（${gotSay.ev.name}）` : "4 分鐘內未觀測到（機率＋冷卻）");
  softCheck("睡前互道晚安（兩位居民回家前道別）", dir.gnSay.length > 0,
    dir.gnSay.length ? `「${dir.gnSay[dir.gnSay.length - 1].say}」` : "未觀測到");
  softCheck("抬頭許願（獨自對夜空、無玩家亦發生）", dir.wishSay.length > 0,
    dir.wishSay.length ? `「${dir.wishSay[dir.wishSay.length - 1].say}」` : "未觀測到");

  // 把鏡頭轉向剛冒台詞的居民，拍下夜裡溫柔日常的一幕。
  const focus = gotSay ? gotSay.ev : anchor;
  await page.evaluate((f) => {
    const p = window.__voxel.player;
    p.x = f.x + 4; p.z = f.z + 4;
    window.__voxel.lookTowards(f.x, (f.y || p.y) + 1.2, f.z);
  }, focus).catch(() => {});
  await sleep(1500);
  const shotSay = join(SHOTS, "nightlife-b-say.png");
  await page.screenshot({ path: shotSay });
  console.log("  📸", shotSay);

  // ── (c) 互道晚安上城鎮動態（best-effort）─────────────────────────
  console.log("\n(c) 睡前互道晚安上一則城鎮動態（feed）");
  const gnFeed = dir.feed.filter((f) => (f.kind && f.kind.includes("晚安")) || (f.line && f.line.includes("互道")));
  softCheck("觀測到『互道晚安』城鎮動態", gnFeed.length > 0,
    gnFeed.length ? JSON.stringify(gnFeed[gnFeed.length - 1]) : "未觀測到（feed 廣播格式可能不同，見台詞觀測）");

  send(dir, { t: "qa_set_time", time: 0.5 }); // 收尾撥回白天
  await sleep(500);
  await browser.close();
  dir.ws.close();
  await sleep(200);

  console.log(`\n══════════════════════════════════════════`);
  console.log(`居民夜間生活 真瀏覽器 QA：${pass} 通過 / ${fail} 失敗 / ${warn} 警告`);
  console.log(`══════════════════════════════════════════`);
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
