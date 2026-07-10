// ============================================================
// voxel-shadow-browser-qa.mjs — 暗影生物 v1 真實瀏覽器 QA（真渲染·真截圖·量 FPS）
// ============================================================
// 比照 voxel-qa.mjs：puppeteer-core 驅動系統 Chrome 載入隔離伺服器的 /voxel/，
// 搭配一條 Node WS「導演」連線（撥鐘入夜、點火把），拍下三張證據截圖：
//   (a) 夜裡暗處的暗影（半透明深色漂浮體＋微光邊）
//   (b) 火把光圈庇護（暗影止步/消散、光圈內安全）
//   (c) 挖擊戰鬥（受擊變淡/閃白 → 第三下化成輕煙）
// 同場量真實 rAF FPS（夜景＋暗影全開時仍須順暢）。
// 需要伺服器以 BUTFUN_QA_DEBUG=1 啟動；截圖存 VQA_SHOTS（預設 scratchpad）。
// 不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import WebSocket from "ws";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

const PORT = process.env.VQA_PORT || 3941;
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
let pass = 0, fail = 0;
function check(label, ok, extra = "") {
  if (ok) { pass++; console.log(`  ✅ ${label}${extra ? "  " + extra : ""}`); }
  else { fail++; console.log(`  ❌ ${label}${extra ? "  " + extra : ""}`); }
}

// 導演連線：撥鐘/授予/放火把用（世界是共享的，導演的動作瀏覽器端全看得到）。
function director(name) {
  const ws = new WebSocket(WS_URL);
  const st = { ws, ready: false, shadows: [], players: [] };
  ws.on("message", (buf) => {
    let m; try { m = JSON.parse(buf.toString()); } catch { return; }
    if (m.t === "welcome") st.ready = true;
    if (m.t === "players") { st.shadows = m.shadows || []; st.players = m.players || []; }
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

(async () => {
  console.log(`\n【暗影生物 真瀏覽器 QA】${URL}`);
  const dir = director("QA導演_" + Math.floor(Math.random() * 1e5));
  await waitFor(() => dir.ready, 5000);

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

  const pos = async () => page.evaluate(() => {
    const p = window.__voxel.player; return { x: p.x, y: p.y, z: p.z };
  });
  const shadowsInPage = async () => page.evaluate(() => window.__voxel.shadows);

  // ── 走出村莊庇護半徑（村莊 48 格內不生成暗影）─────────────
  console.log("(前置) 走出村莊庇護半徑（真的用鍵盤走）");
  const p0 = await pos();
  // 朝遠離原點的方向看，然後按住 W 直走（每 2 秒校正一次朝向）。
  await page.bringToFront();
  await page.keyboard.down("KeyW");
  for (let i = 0; i < 30; i++) {
    const p = await pos();
    if (Math.hypot(p.x, p.z) > 85) break;
    await page.evaluate(() => {
      const p = window.__voxel.player;
      const len = Math.hypot(p.x, p.z) || 1;
      // 朝「遠離原點」的方向走（原點即村莊一帶）。
      window.__voxel.lookTowards(p.x + (p.x / len) * 10, p.y + 1.5, p.z + (p.z / len) * 10);
    });
    await sleep(2000);
  }
  await page.keyboard.up("KeyW");
  const pFar = await pos();
  const distOut = Math.hypot(pFar.x, pFar.z).toFixed(1);
  check("玩家已走到村外暗野", Math.hypot(pFar.x, pFar.z) > 80, `離原點 ${distOut} 格（起點 ${Math.hypot(p0.x, p0.z).toFixed(1)}）`);

  // FPS 白天基線（同場景、同機況）：夜景+暗影的判定用「相對不倒退」——
  // 這台 QA 機與別的迴圈共用，絕對 FPS 隨主機負載浮動，絕對門檻會誤報。
  await page.evaluate(() => { window.__fps = { frames: 0, t0: performance.now() }; });
  await sleep(6000);
  const fpsDay = await page.evaluate(() => window.__fps.frames / ((performance.now() - window.__fps.t0) / 1000));
  console.log(`  （白天基線 rAF ${fpsDay.toFixed(1)} fps）`);

  // ── (a) 撥鐘入夜 → 等暗影漂近 → 轉頭看它 → 截圖 ───────────
  console.log("(a) 夜＋暗影截圖");
  send(dir, { t: "qa_set_time", time: 0.86 }); // 入夜起點：整夜約 200 秒，走路+等漂近才夠用
  // 導演瞬移到瀏覽器玩家旁（讓生成錨點/後續放火把都在鏡頭附近）。
  send(dir, { t: "move", x: pFar.x + 2, y: pFar.y, z: pFar.z + 2, yaw: 0 });
  // 主動迎向最近的暗影（暗影漂 1.6 格/秒偏慢，被動等會吃掉整個夜；玩家走過去更快）。
  const approach = async (goalDist, ms) => {
    const t0 = Date.now();
    let found = null;
    await page.keyboard.down("KeyW");
    while (Date.now() - t0 < ms) {
      const list = await shadowsInPage();
      const p = await pos();
      const withD = (list || [])
        .map((s) => ({ s, d: Math.hypot(s.x - p.x, s.z - p.z) }))
        .sort((a, b) => a.d - b.d);
      if (withD.length) {
        const { s, d } = withD[0];
        if (d <= goalDist) { found = s; break; }
        await page.evaluate((q) => window.__voxel.lookTowards(q.x, q.y, q.z), s);
      }
      await sleep(400);
    }
    await page.keyboard.up("KeyW");
    return found;
  };
  const near = await approach(12, 90000);
  check("夜裡有暗影（前端實體已渲染、走近拍得到）", !!near, near ? `id=${near.id} @(${near.x.toFixed(1)},${near.y.toFixed(1)},${near.z.toFixed(1)})` : "90 秒內無");
  if (near) {
    await page.evaluate((s) => window.__voxel.lookTowards(s.x, s.y, s.z), near);
    await sleep(400);
    await page.screenshot({ path: join(SHOTS, "a-shadow-night.png") });
    console.log("  📸", join(SHOTS, "a-shadow-night.png"));
  }

  // ── (b) 火把光圈庇護截圖 ──────────────────────────────────
  console.log("(b) 光圈庇護截圖");
  send(dir, { t: "qa_set_time", time: 0.9 }); // 重釘夜晚（防自然黎明清場）
  // 導演發火把、在瀏覽器玩家腳邊放一支（光圈 8 格罩住玩家＝安全區；靠近的暗影會止步/消散）。
  send(dir, { t: "qa_grant", item_id: 31, count: 4 });
  await sleep(400);
  const pNow = await pos();
  const tx = Math.floor(pNow.x) + 2, tz = Math.floor(pNow.z);
  send(dir, { t: "move", x: tx + 0.5, y: pNow.y, z: tz + 0.5, yaw: 0 });
  await sleep(300);
  // 從玩家腳的高度往上找一格空氣放（導演與玩家同高，reach 內）。
  for (let dy = 0; dy <= 2; dy++) send(dir, { t: "place", x: tx, y: Math.floor(pNow.y) + dy, z: tz, b: 31 });
  await sleep(1200);
  // 取景：先看向火把、倒退幾步拉開距離（貼著方塊/樹葉截圖會是一面黑牆）。
  await page.evaluate((t) => window.__voxel.lookTowards(t.x + 0.5, t.y + 0.7, t.z + 0.5), { x: tx, y: Math.floor(pNow.y), z: tz });
  await page.keyboard.down("KeyS"); await sleep(1400); await page.keyboard.up("KeyS");
  await page.evaluate((t) => window.__voxel.lookTowards(t.x + 0.5, t.y + 0.7, t.z + 0.5), { x: tx, y: Math.floor(pNow.y), z: tz });
  await sleep(600);
  await page.screenshot({ path: join(SHOTS, "b-shadow-light-shelter.png") });
  console.log("  📸", join(SHOTS, "b-shadow-light-shelter.png"));
  // 取完景走回火把腳邊（剛才為了取景倒退了幾步，可能退到光圈邊緣外）。
  await page.evaluate((t) => window.__voxel.lookTowards(t.x + 0.5, t.y + 0.7, t.z + 0.5), { x: tx, y: Math.floor(pNow.y), z: tz });
  await page.keyboard.down("KeyW"); await sleep(1100); await page.keyboard.up("KeyW");
  // 庇護驗證：光圈內站 8 秒，快照裡不該有暗影貼到玩家 3 格內（都會在光圈邊被擋/消散）。
  let breach = false;
  for (let i = 0; i < 16; i++) {
    const list = await shadowsInPage();
    const p = await pos();
    if ((list || []).some((s) => Math.hypot(s.x - p.x, s.z - p.z) < 3)) { breach = true; break; }
    await sleep(500);
  }
  check("光圈內 8 秒無暗影貼身（光=庇護）", !breach);

  // ── (c) 挖擊戰鬥截圖 ──────────────────────────────────────
  console.log("(c) 戰鬥截圖（挖擊 3 下消散）");
  send(dir, { t: "qa_set_time", time: 0.9 }); // 重釘夜晚
  // 走出光圈找下一隻獵物（按住 W 走 8 秒離開火把）。
  await page.evaluate(() => {
    const p = window.__voxel.player;
    const len = Math.hypot(p.x, p.z) || 1;
    window.__voxel.lookTowards(p.x + (p.x / len) * 10, p.y + 1.5, p.z + (p.z / len) * 10);
  });
  await page.keyboard.down("KeyW"); await sleep(8000); await page.keyboard.up("KeyW");
  const prey = await approach(6, 90000);
  check("有暗影進入挖擊距離", !!prey, prey ? `id=${prey.id}` : "90 秒內無");
  if (prey) {
    // 「跟著打」：暗影會漂移（有時改追附近居民而遠離）——距離 > 5.5 就追上去，
    // ≤ 5.5 才揮一下（伺服器 reach 驗證 ~7，留餘裕）。第 2 下後拍戰鬥截圖（受擊變淡），
    // 揮到消散為止（3 下有效即散；追不上/被拒的揮擊不計）。
    let swings = 0, tookShot = false, gone = false;
    for (let step = 0; step < 40 && !gone; step++) {
      const cur = (await shadowsInPage()).find((s) => s.id === prey.id);
      if (!cur) { gone = true; break; }
      const p = await pos();
      const d = Math.hypot(cur.x - p.x, cur.z - p.z);
      await page.evaluate((s) => window.__voxel.lookTowards(s.x, s.y, s.z), cur);
      if (d > 5.5) {
        await page.keyboard.down("KeyW"); await sleep(500); await page.keyboard.up("KeyW");
        continue;
      }
      await page.evaluate((s) => window.__voxel.qaShadowHit(s.id), cur);
      swings++;
      // 每一下都截一張（樹木偶爾擋鏡頭，多張裡總有清楚的戰鬥畫面）。
      if (swings <= 3) {
        await sleep(250); // 讓 hits 快照回來（受擊變淡/閃白看得到）
        const shotPath = join(SHOTS, `c-shadow-combat-${swings}.png`);
        await page.screenshot({ path: shotPath });
        console.log("  📸", shotPath);
        tookShot = true;
      }
      await sleep(450);
    }
    // 收尾確認：前端實體移除（快照 10Hz，緩衝 3 秒）。
    const removed = gone || !!(await waitFor(async () =>
      (!(await shadowsInPage()).find((s) => s.id === prey.id) ? true : null), 3000, 150));
    check("挖擊擊散：前端實體移除", removed, `有效揮擊約 ${swings} 下`);
    await page.screenshot({ path: join(SHOTS, "c2-shadow-dissipate.png") });
    console.log("  📸", join(SHOTS, "c2-shadow-dissipate.png"));
  }

  // ── FPS（夜景＋暗影全開，與白天基線相對比較）───────────────
  await page.evaluate(() => { window.__fps = { frames: 0, t0: performance.now() }; });
  await sleep(6000);
  const fps = await page.evaluate(() => window.__fps.frames / ((performance.now() - window.__fps.t0) / 1000));
  check("夜景＋暗影不掉幀（≥ 白天基線 − 8）", fps >= fpsDay - 8,
    `夜 ${fps.toFixed(1)} fps vs 日 ${fpsDay.toFixed(1)} fps${fps < 50 ? "（絕對值偏低=主機當下負載，非本功能）" : ""}`);

  send(dir, { t: "qa_set_time", time: 0.5 }); // 收尾撥回白天
  await sleep(300);
  dir.ws.close();
  await browser.close();

  console.log(`\n══════════════════════════════════════════`);
  console.log(`暗影生物 真瀏覽器 QA：${pass} 通過 / ${fail} 失敗`);
  console.log(`══════════════════════════════════════════`);
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
