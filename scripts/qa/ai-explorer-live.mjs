// 瀏覽器版長住 AI 探索者：用真 Chrome（headless）長時間住在 ButFun 裡自動探索生活，
// **核心目的＝持續抓前端錯誤**——render 例外 / console.error / pageerror / requestfailed。
// 這正是「用瀏覽器而非 WS」的理由：只有真實 canvas 渲染才會炸出 safeRender/safeDraw 的 [render] 例外。
//
// 人格：只做 1 個「探索者（explorer）」。純 JS 啟發式漫遊，零 LLM、零洪水。
// 參考 /tmp/butfun-browptest/drive.mjs 的連線 / 進場 / 移動 / hook console.error 攤平 stack / 截圖寫法。
//
// 用法：
//   RUN_MIN=60 node scripts/qa/ai-explorer-live.mjs        # 跑 60 分鐘後乾淨退出
//   node scripts/qa/ai-explorer-live.mjs                   # 同上（預設 60 分鐘）
//   收到 SIGTERM / SIGINT → 乾淨關 browser 後 exit。
//
// 安全鐵律：① 只連 localhost:3000（其它一律拒連）。② 單一實例、headless、行為悠閒不轟炸。
//           ③ 不碰玩家資料、不改遊戲碼。detached 啟動會繼續跑＝刻意（長住）。
import { mkdirSync, appendFileSync, readdirSync, unlinkSync } from "fs";
import { pathToFileURL } from "url";

// puppeteer-core 沒裝在 repo（重相依不入 repo）；裝在 /tmp/butfun-browptest（範本所在）。
// ESM 不吃 NODE_PATH，所以用絕對路徑動態載入；位置可用 EXPLORER_NODE_MODULES 覆蓋。
const NODE_MODULES = process.env.EXPLORER_NODE_MODULES || "/tmp/butfun-browptest/node_modules";
const { default: puppeteer } = await import(
  pathToFileURL(`${NODE_MODULES}/puppeteer-core/lib/puppeteer/puppeteer-core.js`).href
);

// ── 參數 ──────────────────────────────────────────────────────────────────
const URL = process.env.EXPLORER_URL || "http://localhost:3000/";
const RUN_MIN = parseFloat(process.env.RUN_MIN || "60");          // 總時長（分鐘）
const NAME = process.env.EXPLORER_NAME || "漫遊者·洛";
const CHROME = process.env.CHROME_BIN || "/usr/bin/google-chrome";

// 安全鐵律 ①：只連 localhost:3000（避免誤連線上 / 外部）。
if (!/^https?:\/\/(localhost|127\.0\.0\.1):3000(\/|$)/.test(URL)) {
  console.error(`[explorer] 安全拒連：只允許 localhost:3000，收到 ${URL}`);
  process.exit(2);
}

// ── 記錄 ──────────────────────────────────────────────────────────────────
const DIR = "/tmp/butfun-residents";
mkdirSync(DIR, { recursive: true });
const LOG = `${DIR}/explorer.log`;
const SHOT_KEEP = 10;     // 只留最近 N 張截圖，免塞爆磁碟。
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const ts = () => new Date().toISOString();

function log(line) {
  const s = `${ts()} [explorer/${NAME}] ${line}\n`;
  try { appendFileSync(LOG, s); } catch {}
  // 同步印到 stdout，前景測試時看得到；detached 時導進 boot.log。
  process.stdout.write(s);
}

// 抓到的錯誤分類計數（心跳時回報）。
const stats = { consoleErr: 0, render: 0, pageErr: 0, reqFail: 0 };

// 立即把抓到的前端錯誤 append 進 log（帶分類標籤 + 完整訊息 / stack）。
function captureErr(kind, text) {
  if (kind === "render") stats.render++;
  else if (kind === "pageerror") stats.pageErr++;
  else if (kind === "requestfailed") stats.reqFail++;
  else stats.consoleErr++;
  log(`!!! 前端錯誤[${kind}] ${text}`);
}

// 截圖：循環覆蓋、只留最近 SHOT_KEEP 張。
function pruneShots() {
  try {
    const shots = readdirSync(DIR)
      .filter((f) => f.startsWith("explorer-shot-") && f.endsWith(".png"))
      .sort();
    while (shots.length > SHOT_KEEP) {
      const old = shots.shift();
      try { unlinkSync(`${DIR}/${old}`); } catch {}
    }
  } catch {}
}

// ── 啟動瀏覽器：手機視窗 390x844 dpr3（手機最容易出前端 bug）──────────────
let browser;
let closing = false;

async function shutdown(reason) {
  if (closing) return;
  closing = true;
  log(`收到 ${reason}，乾淨關閉 browser…`);
  try { if (browser) await browser.close(); } catch {}
  log("已關閉，bye。");
  process.exit(0);
}
process.on("SIGTERM", () => shutdown("SIGTERM"));
process.on("SIGINT", () => shutdown("SIGINT"));

async function main() {
  log(`啟動探索者「${NAME}」→ ${URL}（預計跑 ${RUN_MIN} 分鐘，手機 390x844 dpr3 headless）`);

  browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: "new",
    args: ["--no-sandbox", "--disable-setuid-sandbox", "--disable-gpu", "--window-size=390,844"],
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 3, isMobile: true, hasTouch: true });

  // 在頁面任何腳本前 hook console.error，把 Error 物件的 stack 攤平成字串，
  // puppeteer 的 console listener 才抓得到完整堆疊（含 safeRender/safeDraw 的 [render] 例外）。
  await page.evaluateOnNewDocument(() => {
    const orig = console.error.bind(console);
    console.error = (...args) => {
      const flat = args
        .map((a) => (a && a.stack) ? a.stack : (typeof a === "object" ? JSON.stringify(a) : String(a)))
        .join(" ");
      orig("[[CAP]] " + flat);
    };
    window.addEventListener("error", (e) =>
      orig("[[CAP]] window.onerror: " + ((e.error && e.error.stack) || e.message)));
    window.addEventListener("unhandledrejection", (e) =>
      orig("[[CAP]] unhandledrejection: " + ((e.reason && e.reason.stack) || e.reason)));
    // 心跳偵測用：記最後一次 rAF 時間，主程式判定整頁凍結。
    window.__lastRaf = Date.now();
    const tick = () => { window.__lastRaf = Date.now(); requestAnimationFrame(tick); };
    requestAnimationFrame(tick);
  });

  // 持續錯誤擷取（核心）。
  page.on("console", (msg) => {
    let t = msg.text();
    if (t.startsWith("[[CAP]] ")) t = t.slice(8);
    const isRender = t.includes("[render") || t.includes("[draw");
    if (msg.type() === "error" || isRender || t.includes("onerror") || t.includes("unhandledrejection")) {
      captureErr(isRender ? "render" : "console", t);
    }
  });
  page.on("pageerror", (err) => captureErr("pageerror", err.stack || err.message));
  page.on("requestfailed", (req) => {
    // 忽略導航中止 / 字型等無害失敗；其餘記下（含 WS 失敗線索）。
    const f = req.failure();
    const reason = (f && f.errorText) || "?";
    if (reason === "net::ERR_ABORTED") return;
    captureErr("requestfailed", `${req.method()} ${req.url()} → ${reason}`);
  });

  // ── 進場：以訪客身份（填名字 → 按「以訪客進場」joinBtn）──────────────────
  log(`開啟頁面…`);
  await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await sleep(1500);
  await page.evaluate((nm) => {
    const n = document.getElementById("nameInput");
    if (n) n.value = nm;
  }, NAME);
  const joined = await page.evaluate(() => {
    const b = document.getElementById("joinBtn");
    if (b) { b.click(); return true; }
    return false;
  });
  log(`以訪客進場：${joined ? "已點 joinBtn" : "找不到 joinBtn（頁面結構可能變了）"}`);
  await sleep(4000); // 等 welcome + 首批 snapshot + 渲染起跑。

  // ── 探索行為迴圈：朝隨機航點長按方向鍵走一段 → 換下一個，城內外漫遊 ───────
  const t0 = Date.now();
  const endAt = t0 + RUN_MIN * 60 * 1000;
  const keys = ["ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp"];
  let lastShot = 0;
  let lastBeat = 0;
  let frozenLogged = false;
  let waypointKey = keys[Math.floor(Math.random() * keys.length)];
  let waypointUntil = 0;

  while (Date.now() < endAt && !closing) {
    const now = Date.now();

    // 換航點：每段走 3~7 秒朝同一方向（長按），到了再隨機換一個方向。
    if (now >= waypointUntil) {
      waypointKey = keys[Math.floor(Math.random() * keys.length)];
      waypointUntil = now + (3000 + Math.random() * 4000);
      // 偶爾（~1/5）停下觀察一會兒，不送輸入（理性節流＝不狂送）。
      if (Math.random() < 0.2) {
        await sleep(1500 + Math.random() * 2000);
        continue;
      }
    }

    // 長按一小段（~700ms）再放開，模擬連續移動；段間留空檔避免轟炸輸入。
    try {
      await page.keyboard.down(waypointKey);
      await sleep(700);
      await page.keyboard.up(waypointKey);
    } catch (e) {
      // 頁面若崩，keyboard 操作會 throw——記一筆嚴重異常，迴圈靠下方凍結偵測兜底。
      log(`!!! 嚴重：輸入操作失敗（頁面可能崩了）：${e.message}`);
    }
    await sleep(400 + Math.random() * 600);

    // 定時截圖（每 ~60 秒）：循環覆蓋、留最近 10 張。
    if (now - lastShot >= 60000) {
      lastShot = now;
      const stamp = ts().replace(/[:.]/g, "-");
      const p = `${DIR}/explorer-shot-${stamp}.png`;
      try { await page.screenshot({ path: p }); pruneShots(); } catch (e) {
        log(`截圖失敗：${e.message}`);
      }
    }

    // 定時心跳（每 ~60 秒）：時間 / 還活著 / 大概位置 / 錯誤累計。
    if (now - lastBeat >= 60000) {
      lastBeat = now;
      // 遊戲的 myId/players 在 IIFE 內、未掛 window，讀不到精確座標；
      // 改以「畫面是否還在動」當大概位置 / 活著的證據（rAF 新鮮度 + canvas 尺寸）。
      let liveness = "畫面狀態未知";
      let lastRaf = 0;
      try {
        const d = await page.evaluate(() => ({
          lastRaf: window.__lastRaf || 0,
          canvasW: document.getElementById("game")?.width || 0,
          hudHidden: document.getElementById("hud")?.classList.contains("hidden"),
        }));
        lastRaf = d.lastRaf;
        const rafAge = lastRaf ? Date.now() - lastRaf : -1;
        liveness = `畫面${rafAge >= 0 && rafAge < 2000 ? "在動" : "疑停"}(rAF ${rafAge}ms前,canvas ${d.canvasW}px,hud${d.hudHidden ? "隱" : "顯"})`;
      } catch {}
      const mins = ((now - t0) / 60000).toFixed(1);
      log(`心跳 t+${mins}min 還活著 ${liveness} 累計錯誤[console:${stats.consoleErr} render:${stats.render} pageerr:${stats.pageErr} reqfail:${stats.reqFail}]`);

      // 韌性：整頁壞掉（rAF 超過 30 秒沒動＝凍結）記一筆嚴重異常。
      if (lastRaf && now - lastRaf > 30000) {
        if (!frozenLogged) {
          log(`!!! 嚴重：頁面疑似凍結（rAF 已 ${Math.round((now - lastRaf) / 1000)}秒沒推進）——可能 render 永久停迴圈`);
          frozenLogged = true;
        }
      } else {
        frozenLogged = false;
      }
    }
  }

  log(`達總時長 ${RUN_MIN} 分鐘，乾淨退出（累計錯誤 console:${stats.consoleErr} render:${stats.render} pageerr:${stats.pageErr} reqfail:${stats.reqFail}）`);
  await shutdown("時間到");
}

main().catch(async (e) => {
  log(`!!! 主程式異常：${e.stack || e.message}`);
  await shutdown("異常");
});
