// ============================================================
// voxel-custom-dusk-qa.mjs — 村莊自發習俗·暮聚 v1 真實瀏覽器 QA（真渲染·真截圖）
// ============================================================
// 比照 voxel-shadow-browser-qa.mjs：puppeteer-core 驅動系統 Chrome 載入隔離伺服器的 /voxel/，
// 搭配一條 Node WS「導演」連線把世界時鐘撥到黃昏（qa_set_time），觀察村莊自發習俗「暮聚」：
//   - 黃昏時分，村子已有廣場中心（data/voxel_village_done）＋在場閒著的居民 → 不約而同晃到
//     村碑廣場（原點）邊聚著閒話家常。
//   - 驗證：①/voxel/feed 出現「村莊習俗」動態；②廣播的居民有人冒出暮聚閒聊泡泡；
//     ③居民聚攏到廣場中心附近；④拍下廣場邊聚會截圖。
// 需要伺服器以 BUTFUN_QA_DEBUG=1 啟動、cwd 下有 data/voxel_village_done=「0,0」。
// 截圖存 VQA_SHOTS（預設 scratchpad）。不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import WebSocket from "ws";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

const PORT = process.env.VQA_PORT || 3947;
const BASE = `http://127.0.0.1:${PORT}`;
const URL = process.env.VQA_URL || `${BASE}/voxel/?debug=1`;
const WS_URL = `ws://127.0.0.1:${PORT}/voxel/ws`;
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const SHOTS = process.env.VQA_SHOTS || "shots";
mkdirSync(SHOTS, { recursive: true });

// 暮聚閒聊泡泡池（須與 src/voxel_custom.rs::chatter_bubble 同步）。
const CHATTER = [
  "黃昏了，來廣場邊坐坐、說說話。",
  "今天過得怎麼樣？我來聽聽。",
  "每到這時候聚一聚，心裡就踏實。",
  "你看這夕陽，把村子都染暖了。",
  "大家都在，這一天就算圓滿了。",
  "在村碑邊閒聊幾句，真好。",
];

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

// 導演連線：撥鐘用（世界共享，導演撥的時間瀏覽器端全看得到）；同時收居民廣播。
function director(name) {
  const ws = new WebSocket(WS_URL);
  const st = { ws, ready: false, residents: [] };
  ws.on("message", (buf) => {
    let m; try { m = JSON.parse(buf.toString()); } catch { return; }
    if (m.t === "welcome") st.ready = true;
    if (m.t === "players") { st.residents = m.residents || []; }
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
async function fetchFeed() {
  try {
    const r = await fetch(`${BASE}/voxel/feed`);
    return await r.json();
  } catch { return []; }
}

(async () => {
  console.log(`\n【村莊自發習俗·暮聚 真瀏覽器 QA】${URL}`);
  const dir = director("QA導演_" + Math.floor(Math.random() * 1e5));
  await waitFor(() => dir.ready, 5000);

  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU_ARGS });
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 800 });
  await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await sleep(6000); // 等 chunk/mesh

  // 導演瞬移到廣場中心旁高處俯瞰（讓截圖能看見居民在村碑邊聚攏）。
  send(dir, { t: "move", x: 6, y: 40, z: 6, yaw: 0 });

  // ── 反覆把時鐘釘在黃昏（Dusk＝0.70–0.85），維持暮聚觸發窗口 ──────────
  // 每日黃昏至多一場；若這一 tick 在場閒人不足，last_custom_day 不會被設，下個 tick 續試——
  // 故只要維持黃昏、等居民手邊的採集/建造告一段落有空，暮聚遲早會開起來。
  console.log("(1) 把世界撥到黃昏，等暮聚自發湧現（最多 ~3 分鐘）");
  let feedHit = null, chatterHit = null, clustered = false;
  const t0 = Date.now();
  while (Date.now() - t0 < 180000) {
    send(dir, { t: "qa_set_time", time: 0.76 }); // 重釘黃昏，防時鐘自然滑進入夜
    await sleep(2500);

    // ① Feed：出現「村莊習俗」分類即命中。
    if (!feedHit) {
      const feed = await fetchFeed();
      feedHit = (feed || []).find((e) => e.kind === "村莊習俗");
    }
    // ② 居民廣播：有人冒出暮聚閒聊泡泡。
    if (!chatterHit) {
      chatterHit = (dir.residents || []).find((r) => CHATTER.includes((r.say || "").trim()));
    }
    // ③ 聚攏：≥2 位居民同時在廣場中心（原點）附近 10 格內。
    const near = (dir.residents || []).filter((r) => Math.hypot(r.x, r.z) < 10);
    if (near.length >= 2) clustered = true;

    // Feed 命中＝暮聚已宣告。被拉進的居民散在村中心四周數十格、要以夜間降速走進廣場（約數十秒），
    // 故 Feed 命中後再多等一陣讓他們真的走攏，才判定聚攏＋截圖（不在他們還在路上時就早退）。
    if (feedHit) break;
  }
  // 收束等待：暮聚宣告後，續釘黃昏、等居民走攏到廣場中心（最多 ~70 秒）。
  if (feedHit) {
    console.log("  …暮聚已宣告，等居民走攏廣場");
    const c0 = Date.now();
    while (Date.now() - c0 < 70000) {
      send(dir, { t: "qa_set_time", time: 0.78 });
      await sleep(2500);
      const near = (dir.residents || []).filter((r) => Math.hypot(r.x, r.z) < 10);
      if (near.length >= 2) clustered = true;
      if (!chatterHit) {
        chatterHit = (dir.residents || []).find((r) => CHATTER.includes((r.say || "").trim()));
      }
      if (clustered) break;
    }
  }

  check("暮聚上了城鎮動態牆（Feed 出現「村莊習俗」）", !!feedHit,
    feedHit ? `「${(feedHit.detail || "").slice(0, 40)}…」` : "3 分鐘內未出現");
  check("居民在廣場邊冒暮聚閒聊泡泡", !!chatterHit,
    chatterHit ? `${chatterHit.name}：「${chatterHit.say}」` : "未捕捉到（低頻，可能剛好沒骰中）");
  check("≥2 位居民聚攏到村碑廣場中心附近", clustered,
    `此刻原點 8 格內 ${(dir.residents || []).filter((r) => Math.hypot(r.x, r.z) < 8).length} 位`);

  // ── 截圖：從廣場上空俯瞰居民在村碑邊聚會 ──────────────────────────
  console.log("(2) 拍下廣場邊暮聚截圖");
  // 讓瀏覽器玩家看向廣場中心（村碑腳下），俯瞰聚會。
  await page.evaluate(() => {
    if (window.__voxel && window.__voxel.player) {
      // 移到廣場上空一點、俯視原點。
      const p = window.__voxel.player;
      p.x = 8; p.y = 38; p.z = 8;
    }
  });
  await page.evaluate(() => window.__voxel && window.__voxel.lookTowards && window.__voxel.lookTowards(0.5, 34, 0.5));
  await sleep(1500);
  const shot = join(SHOTS, "voxel-custom-dusk.png");
  await page.screenshot({ path: shot });
  console.log("  📸", shot);

  send(dir, { t: "qa_set_time", time: 0.5 }); // 收尾撥回白天
  await sleep(300);
  dir.ws.close();
  await browser.close();

  console.log(`\n══════════════════════════════════════════`);
  console.log(`暮聚 真瀏覽器 QA：${pass} 通過 / ${fail} 失敗`);
  console.log(`══════════════════════════════════════════`);
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
