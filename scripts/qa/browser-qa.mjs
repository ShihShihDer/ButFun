// ============================================================
// browser-qa.mjs — 真實瀏覽器 QA（真渲染 · 真量 FPS）
// ============================================================
// 為什麼有這支：
//   先前的 QA 走「無頭 Node 模擬」——不開瀏覽器、不畫圖、量不到 FPS，
//   結果「掉幀／卡頓」這類效能問題整類漏看（Node 跑得飛快不代表玩家手機畫得動）。
//   這支改成用「真的 Chrome、真的 WebGL、真的 requestAnimationFrame」去量，
//   讓「30Hz vs 15Hz 的掉幀」這種事第一次能被客觀抓到。
//
// 做了什麼：
//   1) 用 puppeteer-core 驅動「系統安裝的 google-chrome」（不另下載一份 chromium）。
//   2) 想盡辦法讓 WebGL 真的在硬體 GPU 上渲染，並「驗證不是黑屏」：
//        · 攔截 getContext 包住 WebGL，數真實 draw call；
//        · 讀 UNMASKED_RENDERER（GPU 字串）判斷硬體 vs 軟渲染（SwiftShader）；
//        · 用 gl.readPixels 在剛畫完的 rAF 取一小塊像素，算變異度 → 證明畫面有東西、非全黑。
//   3) 注入自己的 rAF 計數器量真實幀率（平均／最低／p1），
//      並交叉讀遊戲自己 ?debug=1 的 #dbgHud 讀數（FPS／預測誤差）。
//   4) 真的用鍵盤 WASD 讓角色走 ~12 秒，量「移動時 vs 靜止時」FPS 與自身預測誤差。
//
// 怎麼跑（預設就對）：
//        node scripts/qa/browser-qa.mjs
//   實測這台機器：headless（new headless）+ GPU 旗標 = 真的吃硬體 GPU
//   （ANGLE→Mesa Intel HD 530, OpenGL 4.6）、rAF 以 60Hz 真的在跑、能可靠截圖。
//   報告會印出 unmasked GPU 字串；若退成 SwiftShader/llvmpipe 軟渲染會明白標註
//   （絕對 FPS 偏低，但「相對掉幀」如 30Hz vs 15Hz 仍量得到）。
//
//   踩過的坑（留給後人）：「xvfb + 有頭 Chrome」雖能拿到硬體 GPU 字串，但虛擬顯示
//   無 vsync → requestAnimationFrame 整個凍住（量到 0 幀）＝不可用。故預設 headless。
//   逃生口：BFQA_HEADLESS=0 可切有頭（需自帶真的會刷新的 DISPLAY）。
//
// 環境變數：
//   BFQA_URL       目標頁（預設 prod /3d/?debug=1）
//   BFQA_HEADLESS  強制 1=headless / 0=有頭（預設：有 DISPLAY 就有頭、否則 headless）
//   BFQA_CHROME    google-chrome 路徑（預設 /usr/bin/google-chrome）
//   BFQA_SECS      靜止與移動各量幾秒（預設 12）
//   BFQA_OUT       截圖輸出目錄（預設 本檔旁 ./out）
//
// 不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { writeFileSync, mkdirSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));

const URL = process.env.BFQA_URL || "https://peregrine.but-fun.com/3d/?debug=1";
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const SAMPLE_SECS = Number(process.env.BFQA_SECS || 12);
const OUT_DIR = process.env.BFQA_OUT || join(__dirname, "out");

// 預設用 headless（new headless）：實測這台機器上，headless new 模式才會真的吃到
// 硬體 GPU（ANGLE→Mesa Intel）、rAF 以 60Hz 真的在跑、又能可靠截圖；
// 反而「xvfb + 有頭」因虛擬顯示無 vsync，requestAnimationFrame 整個凍住（量到 0 幀）＝不可用。
// 仍保留 BFQA_HEADLESS=0 切「有頭」的逃生口（需自帶可用的 DISPLAY，例如真桌面）。
const headlessEnv = process.env.BFQA_HEADLESS;
const HEADLESS = headlessEnv === "0" ? false : true;

// 讓 WebGL 在 Linux 上盡量吃到硬體 GPU 的旗標（有頭/headless 都加）。
// ANGLE→GL 後端 + 不理會 GPU 封鎖名單；headless 下這些是 WebGL 能不能動的關鍵。
const GPU_ARGS = [
  "--no-sandbox",
  "--disable-setuid-sandbox",
  "--ignore-gpu-blocklist",
  "--enable-gpu",
  "--enable-gpu-rasterization",
  "--enable-webgl",
  "--use-gl=angle",
  "--use-angle=gl",
  "--disable-dev-shm-usage", // /dev/shm 太小會讓 Chrome 崩，QA 機常見
  "--window-size=1280,800",
  // 防「分頁被當背景／被遮擋」而把 requestAnimationFrame 節流到趴下（否則量到 0 幀）：
  "--disable-background-timer-throttling",
  "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding",
  "--disable-features=CalculateNativeWinOcclusion",
];

function fmt(n, d = 1) { return Number.isFinite(n) ? n.toFixed(d) : "n/a"; }

// 從一串「每幀間隔(ms)」算出 FPS 統計：平均、最低（最慢那一幀的瞬時 FPS）、p1（最差 1%）。
function fpsStats(deltas) {
  const ds = deltas.filter((d) => d > 0);
  if (ds.length === 0) return { avg: NaN, min: NaN, p1: NaN, frames: 0 };
  const totalMs = ds.reduce((a, b) => a + b, 0);
  const avg = (ds.length / totalMs) * 1000;
  const inst = ds.map((d) => 1000 / d).sort((a, b) => a - b); // 瞬時 FPS 由小到大
  const min = inst[0];
  const p1 = inst[Math.floor(inst.length * 0.01)]; // 第 1 百分位（最差的 1%）
  return { avg, min, p1, frames: ds.length };
}

async function main() {
  if (!existsSync(OUT_DIR)) mkdirSync(OUT_DIR, { recursive: true });

  console.log("═".repeat(60));
  console.log("ButFun 真實瀏覽器 QA");
  console.log("═".repeat(60));
  console.log(`目標     : ${URL}`);
  console.log(`Chrome   : ${CHROME}`);
  console.log(`模式     : ${HEADLESS ? "headless" : "有頭(headful)"}` +
    (process.env.DISPLAY ? `  DISPLAY=${process.env.DISPLAY}` : "  （無 DISPLAY）"));
  console.log(`每段取樣 : ${SAMPLE_SECS}s`);
  console.log("─".repeat(60));

  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: HEADLESS ? "new" : false, // new headless 才吃硬體 GPU 且 rAF 真的跑
    args: GPU_ARGS,
    defaultViewport: { width: 1280, height: 800 },
  });

  const report = { url: URL, mode: HEADLESS ? "headless" : "headful", display: process.env.DISPLAY || null };

  try {
    const page = await browser.newPage();

    // 把瀏覽器 console.error / 頁面崩潰打出來，方便診斷（不靜默吞錯，守 render-loop-resilience 精神）。
    page.on("pageerror", (e) => console.log("  [pageerror]", String(e).split("\n")[0]));

    // ── 注入「渲染探針 + rAF 幀率計數器」，必須在頁面任何 script 之前裝好 ──
    // getContext 包一層：抓到第一個 WebGL context，並把它的 draw call 數起來。
    // 這是「真的有在畫」最直接的證據——Three.js 每幀會發很多 drawElements/drawArrays。
    await page.evaluateOnNewDocument(() => {
      const W = window;
      W.__bfqa = {
        gl: null,
        glAttrs: null,
        drawCalls: 0,
        unmaskedRenderer: null,
        unmaskedVendor: null,
        contextLost: false,
        // rAF 幀率：記錄每幀間隔，量真實幀率（不是遊戲自報的平滑值）。
        frameDeltas: [],
        sampling: false,
        _lastT: 0,
      };
      const origGetContext = HTMLCanvasElement.prototype.getContext;
      HTMLCanvasElement.prototype.getContext = function (type, attrs) {
        const ctx = origGetContext.call(this, type, attrs);
        try {
          if (ctx && /webgl/i.test(String(type)) && !W.__bfqa.gl) {
            W.__bfqa.gl = ctx;
            W.__bfqa.glAttrs = attrs || ctx.getContextAttributes() || null;
            // 讀 GPU 真名（判斷硬體 vs SwiftShader 軟渲染）。
            try {
              const ext = ctx.getExtension("WEBGL_debug_renderer_info");
              if (ext) {
                W.__bfqa.unmaskedRenderer = ctx.getParameter(ext.UNMASKED_RENDERER_WEBGL);
                W.__bfqa.unmaskedVendor = ctx.getParameter(ext.UNMASKED_VENDOR_WEBGL);
              }
            } catch (e) { /* 某些環境沒這擴充，無妨 */ }
            // 包住 draw call 計數。
            const wrap = (name) => {
              const orig = ctx[name];
              if (typeof orig !== "function") return;
              ctx[name] = function (...a) { W.__bfqa.drawCalls++; return orig.apply(this, a); };
            };
            wrap("drawElements");
            wrap("drawArrays");
            wrap("drawElementsInstanced");
            wrap("drawArraysInstanced");
            this.addEventListener("webglcontextlost", () => { W.__bfqa.contextLost = true; }, { once: true });
          }
        } catch (e) { /* 探針失敗也別擋住遊戲本體 */ }
        return ctx;
      };

      // rAF 幀率計數器：常駐掛著，sampling=true 時才記錄間隔。
      const tick = (t) => {
        const q = W.__bfqa;
        if (q.sampling) {
          if (q._lastT) q.frameDeltas.push(t - q._lastT);
          q._lastT = t;
        } else {
          q._lastT = 0;
        }
        W.requestAnimationFrame(tick);
      };
      W.requestAnimationFrame(tick);
    });

    // ── 載入 prod 頁面 ──
    console.log("載入頁面…");
    // 注意：live 遊戲有常駐 WebSocket，networkidle 永遠等不到 → 用 domcontentloaded + 固定等待。
    await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 60000 });

    // 給 Three.js 起 scene、WebSocket 連上、世界資料進來的時間。
    await new Promise((r) => setTimeout(r, 5000));

    // ── 驗證 1：WebGL 真的拿到 context + GPU 是誰 ──
    const glInfo = await page.evaluate(() => {
      const q = window.__bfqa;
      const canvas = document.querySelector("canvas");
      return {
        hasCanvas: !!canvas,
        canvasW: canvas ? canvas.width : 0,
        canvasH: canvas ? canvas.height : 0,
        hasGL: !!q.gl,
        attrs: q.glAttrs ? {
          alpha: q.glAttrs.alpha, antialias: q.glAttrs.antialias,
          preserveDrawingBuffer: q.glAttrs.preserveDrawingBuffer,
        } : null,
        unmaskedRenderer: q.unmaskedRenderer,
        unmaskedVendor: q.unmaskedVendor,
        contextLost: q.contextLost,
      };
    });
    report.gl = glInfo;
    const isSoftware = /swiftshader|llvmpipe|software/i.test(glInfo.unmaskedRenderer || "");
    report.software = isSoftware;

    console.log("─".repeat(60));
    console.log("【WebGL 驗證】");
    console.log(`  canvas       : ${glInfo.hasCanvas ? `${glInfo.canvasW}×${glInfo.canvasH}` : "無!"}`);
    console.log(`  WebGL context: ${glInfo.hasGL ? "有" : "無!"}  contextLost=${glInfo.contextLost}`);
    console.log(`  GPU(unmasked): ${glInfo.unmaskedRenderer || "(讀不到)"}`);
    console.log(`  渲染後端     : ${isSoftware ? "⚠️ 軟體渲染(SwiftShader/llvmpipe) — 絕對FPS偏低，相對掉幀仍有效" : "硬體 GPU"}`);

    // ── 靜止段：量 FPS 基準（不動）──
    console.log("─".repeat(60));
    console.log(`【靜止 ${SAMPLE_SECS}s】量基準 FPS…`);
    const idle = await sampleFps(page, SAMPLE_SECS);
    report.idle = idle;
    console.log(`  rAF FPS  avg=${fmt(idle.fps.avg)}  min=${fmt(idle.fps.min)}  p1=${fmt(idle.fps.p1)}  (frames=${idle.fps.frames})`);
    console.log(`  draw call: 起 ${idle.drawStart} → 迄 ${idle.drawEnd}  (期間 +${idle.drawEnd - idle.drawStart})`);
    console.log(`  遊戲自報 : FPS=${idle.hud.fps ?? "?"}  預測誤差=${idle.hud.predErr ?? "?"}px  線上=${idle.hud.online ?? "?"}`);

    // ── 驗證 2：真的截一張圖 + 讀像素變異度證明非黑屏 ──
    const shotPath = join(OUT_DIR, "render-idle.png");
    await safeShot(page, shotPath);
    const pix = await analyzeRenderPixels(page);
    report.screenshot = { path: shotPath, ...pix };
    console.log(`  截圖     : ${shotPath}`);
    console.log(`  像素證據 : 不重複顏色≈${pix.uniqueColors}  平均亮度=${fmt(pix.avgLuma)}  變異=${fmt(pix.variance)}  (${pix.method})  → ${pix.nonBlank ? "✅ 有畫面(非黑屏)" : "❌ 疑似黑屏/單色"}`);

    // ── 移動段：真的按 WASD 走 ~SAMPLE_SECS 秒，量移動時 FPS 與預測誤差 ──
    console.log("─".repeat(60));
    console.log(`【移動 ${SAMPLE_SECS}s】WASD 走起來，量移動時 FPS / 預測誤差…`);
    const moving = await sampleFpsWhileMoving(page, SAMPLE_SECS);
    report.moving = moving;
    console.log(`  rAF FPS  avg=${fmt(moving.fps.avg)}  min=${fmt(moving.fps.min)}  p1=${fmt(moving.fps.p1)}  (frames=${moving.fps.frames})`);
    console.log(`  draw call: 起 ${moving.drawStart} → 迄 ${moving.drawEnd}  (期間 +${moving.drawEnd - moving.drawStart})`);
    console.log(`  遊戲自報 : FPS=${moving.hud.fps ?? "?"}  預測誤差=${moving.hud.predErr ?? "?"}px  線上=${moving.hud.online ?? "?"}`);
    console.log(`  預測誤差取樣(px): [${moving.predErrSamples.join(", ")}]  → 平均 ${fmt(avg(moving.predErrSamples))}px`);

    const shotMove = join(OUT_DIR, "render-moving.png");
    await safeShot(page, shotMove);
    report.screenshotMoving = shotMove;
    console.log(`  移動截圖 : ${shotMove}`);

    // ── 結論 ──
    console.log("═".repeat(60));
    console.log("【結論】");
    const dropPct = (idle.fps.avg && moving.fps.avg)
      ? ((idle.fps.avg - moving.fps.avg) / idle.fps.avg) * 100 : NaN;
    console.log(`  靜止 FPS avg ${fmt(idle.fps.avg)}  →  移動 FPS avg ${fmt(moving.fps.avg)}  （移動掉幀 ${fmt(dropPct)}%）`);
    console.log(`  移動時最低 FPS = ${fmt(moving.fps.min)}（min）/ p1 = ${fmt(moving.fps.p1)}`);
    console.log(`  移動時自身預測誤差 ≈ ${fmt(avg(moving.predErrSamples))}px（健康區間 ~10-30）`);
    console.log(`  真渲染證據 = WebGL context ${glInfo.hasGL ? "✔" : "✘"} + draw call 持續增長 ${moving.drawEnd > moving.drawStart ? "✔" : "✘"} + 截圖非黑屏 ${pix.nonBlank ? "✔" : "✘"}`);
    if (isSoftware) console.log(`  ⚠️ 本次為軟體渲染：絕對 FPS 不等同手機/真 GPU，但「相對掉幀」仍可信。`);
    console.log("═".repeat(60));

    report.conclusion = {
      idleFpsAvg: idle.fps.avg, movingFpsAvg: moving.fps.avg,
      movingFpsMin: moving.fps.min, movingFpsP1: moving.fps.p1,
      dropPct, predErrMovingAvg: avg(moving.predErrSamples),
      realRender: glInfo.hasGL && (moving.drawEnd > moving.drawStart) && pix.nonBlank,
      software: isSoftware,
    };
    const reportPath = join(OUT_DIR, "report.json");
    writeFileSync(reportPath, JSON.stringify(report, null, 2));
    console.log(`完整報告 JSON: ${reportPath}`);
  } finally {
    // browser.close() 在 GPU 進程崩潰後可能卡死 → 加逾時保險，逾時就強殺進程。
    await Promise.race([
      browser.close().catch(() => {}),
      new Promise((r) => setTimeout(r, 8000)),
    ]);
    try { const proc = browser.process(); if (proc) proc.kill("SIGKILL"); } catch (e) { /* 已關閉 */ }
  }
}

// 截圖：截不到（GPU 合成偶發崩）就吞掉，別讓整份 QA 死在截圖上（像素證據另有 readPixels 那條）。
async function safeShot(page, path) {
  try {
    await Promise.race([
      page.screenshot({ path, captureBeyondViewport: false }),
      new Promise((_, rej) => setTimeout(() => rej(new Error("screenshot timeout 10s")), 10000)),
    ]);
  } catch (e) {
    console.log(`  截圖失敗（不致命，像素證據改看 readPixels）: ${String(e).split("\n")[0]}`);
  }
}

function avg(arr) {
  const xs = arr.filter((x) => Number.isFinite(x));
  return xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : NaN;
}

// 讀遊戲自己的 ?debug=1 #dbgHud 文字（FPS／預測誤差／線上人數）。
async function readGameHud(page) {
  return page.evaluate(() => {
    const el = document.getElementById("dbgHud");
    if (!el) return { fps: null, predErr: null, online: null, raw: null };
    const raw = el.textContent || "";
    const fps = (raw.match(/FPS\s+(\d+)/) || [])[1];
    const predErr = (raw.match(/預測誤差\s+(\d+)/) || [])[1];
    const online = (raw.match(/線上\s+(\d+)/) || [])[1];
    return {
      fps: fps != null ? Number(fps) : null,
      predErr: predErr != null ? Number(predErr) : null,
      online: online != null ? Number(online) : null,
      raw,
    };
  });
}

// 靜止取樣：開 rAF 計數一段時間，回傳 FPS 統計 + draw call 增量 + 遊戲自報讀數。
async function sampleFps(page, secs) {
  const drawStart = await page.evaluate(() => window.__bfqa.drawCalls);
  await page.evaluate(() => {
    window.__bfqa.frameDeltas = [];
    window.__bfqa._lastT = 0;
    window.__bfqa.sampling = true;
  });
  await new Promise((r) => setTimeout(r, secs * 1000));
  const deltas = await page.evaluate(() => {
    window.__bfqa.sampling = false;
    return window.__bfqa.frameDeltas.slice();
  });
  const drawEnd = await page.evaluate(() => window.__bfqa.drawCalls);
  const hud = await readGameHud(page);
  return { fps: fpsStats(deltas), drawStart, drawEnd, hud };
}

// 移動取樣：真的按住 WASD 讓角色繞圈走，期間量 FPS、定時記錄遊戲自報的預測誤差。
async function sampleFpsWhileMoving(page, secs) {
  const drawStart = await page.evaluate(() => window.__bfqa.drawCalls);
  await page.evaluate(() => {
    window.__bfqa.frameDeltas = [];
    window.__bfqa._lastT = 0;
    window.__bfqa.sampling = true;
  });
  await page.bringToFront();
  await page.evaluate(() => { const c = document.querySelector("canvas"); if (c) c.focus(); });

  // 走法：每段按住一個方向若干 ms，輪流 W→D→S→A 繞一圈，讓角色真的在世界裡移動，
  // 同時 render 要不斷重畫場景（移動才測得到「動態場景」下的掉幀）。
  const dirs = ["w", "d", "s", "a"];
  const predErrSamples = [];
  const endAt = Date.now() + secs * 1000;
  let i = 0;
  while (Date.now() < endAt) {
    const key = dirs[i % dirs.length];
    i++;
    await page.keyboard.down(key);
    // 按住約 1.2s，期間抓一次預測誤差讀數。
    await new Promise((r) => setTimeout(r, 600));
    const hud = await readGameHud(page);
    if (hud.predErr != null) predErrSamples.push(hud.predErr);
    await new Promise((r) => setTimeout(r, 600));
    await page.keyboard.up(key);
  }
  // 保險：把所有方向鍵放開。
  for (const k of dirs) { try { await page.keyboard.up(k); } catch (e) { /* 沒按過就放開無妨 */ } }

  const deltas = await page.evaluate(() => {
    window.__bfqa.sampling = false;
    return window.__bfqa.frameDeltas.slice();
  });
  const drawEnd = await page.evaluate(() => window.__bfqa.drawCalls);
  const hud = await readGameHud(page);
  return { fps: fpsStats(deltas), drawStart, drawEnd, hud, predErrSamples };
}

// 讀 WebGL 畫面像素，採樣算「不重複顏色數／亮度／變異」證明非黑屏。
// 遊戲沒開 preserveDrawingBuffer，所以在「剛畫完的 rAF 同一拍」內 readPixels 取色，
// 此時 buffer 還沒被下一幀清掉，能拿到真實畫面像素。
async function analyzeRenderPixels(page) {
  return page.evaluate(async () => {
    const gl = window.__bfqa.gl;
    if (!gl) return { uniqueColors: 0, avgLuma: 0, variance: 0, nonBlank: false, method: "no-gl" };
    return await new Promise((resolve) => {
      requestAnimationFrame(() => {
        try {
          const w = Math.min(gl.drawingBufferWidth, 200);
          const h = Math.min(gl.drawingBufferHeight, 200);
          const px = new Uint8Array(w * h * 4);
          gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, px);
          const colors = new Set();
          let sumL = 0, sumL2 = 0, n = 0;
          for (let p = 0; p < px.length; p += 4) {
            const r = px[p], g = px[p + 1], b = px[p + 2];
            colors.add((r >> 3) << 10 | (g >> 3) << 5 | (b >> 3)); // 量化成 15-bit 色
            const l = 0.299 * r + 0.587 * g + 0.114 * b;
            sumL += l; sumL2 += l * l; n++;
          }
          const avgLuma = sumL / n;
          const variance = sumL2 / n - avgLuma * avgLuma;
          resolve({
            uniqueColors: colors.size,
            avgLuma,
            variance,
            // 非黑屏判準：顏色種類夠多、且有亮度變異（不是整片同色）。
            nonBlank: colors.size > 8 && variance > 25,
            method: "gl.readPixels",
          });
        } catch (e) {
          resolve({ uniqueColors: 0, avgLuma: 0, variance: 0, nonBlank: false, method: "readPixels-failed:" + e });
        }
      });
    });
  });
}

main().catch((e) => { console.error("QA 失敗:", e); process.exit(1); });
