// ============================================================
// fps-bisect.mjs — /3d/ FPS regression 二分搜尋（單版本量測單元）
// ============================================================
// 為什麼有這支：
//   玩家 iPhone 走路 FPS 卡死在 30（已排除低耗電模式、且「同手機早先版本能跑 60」）
//   → 這是程式碼 regression，不是硬體上限。砍 DPR/像素（#818）完全沒讓 FPS 動
//   → 八成不是 GPU 填充率瓶頸，而是「每幀 CPU/JS 太重」或 draw call 太多。
//
//   這支是「二分搜尋」的最小量測單元：給定一個 git commit（或工作樹）的
//   web/3d/main.js，在「固定 CPU 節流倍率」下，用真 Chrome + 真 WebGL 量
//   「走路 FPS / 靜止 FPS / draw call / 每幀 JS 時間」四個指標。
//   ws 仍連 prod 後端（比照 browser-qa.mjs），只把 main.js 用 request
//   interception 換成歷史版本——所以世界資料/物理/其他玩家全是真的。
//
//   CPU 節流（Emulation.setCPUThrottlingRate）把桌機 CPU 壓慢，
//   重現「手機慢 CPU → CPU-bound 掉幀」的症狀；用同一倍率掃所有歷史版本，
//   就能客觀看出「FPS 從 ~60 掉到 ~30」發生在哪兩個 commit 之間。
//
// 怎麼跑：
//   BISECT_SHA=<sha|WORKTREE> BISECT_THROTTLE=6 node scripts/qa/fps-bisect.mjs
//   會印一行 RESULT JSON（給 bisect 腳本 grep）＋人類可讀摘要。
//
//   環境變數：
//     BISECT_SHA       要量的 commit sha；"WORKTREE" = 直接用目前工作樹的檔
//     BISECT_THROTTLE  CPU 節流倍率（1=不節流，6=慢 6 倍…），預設 1
//     BFQA_URL         目標頁（預設 prod /3d/?debug=1）
//     BFQA_CHROME      chrome 路徑（預設 /usr/bin/google-chrome）
//     BFQA_IDLE_SECS   靜止取樣秒數（預設 6）
//     BFQA_MOVE_SECS   移動取樣秒數（預設 8）
//
// 不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(__dirname, "..", "..");

const SHA = process.env.BISECT_SHA || "WORKTREE";
const THROTTLE = Number(process.env.BISECT_THROTTLE || 1);
const URL = process.env.BFQA_URL || "https://peregrine.but-fun.com/3d/?debug=1";
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const IDLE_SECS = Number(process.env.BFQA_IDLE_SECS || 6);
const MOVE_SECS = Number(process.env.BFQA_MOVE_SECS || 8);

// 同 browser-qa.mjs：讓 WebGL 在 Linux headless 盡量吃硬體 GPU 的旗標。
const GPU_ARGS = [
  "--no-sandbox",
  "--disable-setuid-sandbox",
  "--ignore-gpu-blocklist",
  "--enable-gpu",
  "--enable-gpu-rasterization",
  "--enable-webgl",
  "--use-gl=angle",
  "--use-angle=gl",
  "--disable-dev-shm-usage",
  "--window-size=1280,800",
  "--disable-background-timer-throttling",
  "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding",
  "--disable-features=CalculateNativeWinOcclusion",
];

function fmt(n, d = 1) { return Number.isFinite(n) ? n.toFixed(d) : "n/a"; }

// 取某版本的 web/3d/main.js 原始碼（WORKTREE = 直接讀檔；否則 git show）。
function loadMainJs(sha) {
  if (sha === "WORKTREE") {
    return readFileSync(join(REPO_ROOT, "web", "3d", "main.js"), "utf8");
  }
  return execFileSync("git", ["show", `${sha}:web/3d/main.js`], {
    cwd: REPO_ROOT, maxBuffer: 64 * 1024 * 1024, encoding: "utf8",
  });
}

// 從每幀間隔(ms)算 FPS 統計。
function fpsStats(deltas) {
  const ds = deltas.filter((d) => d > 0);
  if (ds.length === 0) return { avg: NaN, min: NaN, p1: NaN, frames: 0 };
  const totalMs = ds.reduce((a, b) => a + b, 0);
  const avg = (ds.length / totalMs) * 1000;
  const inst = ds.map((d) => 1000 / d).sort((a, b) => a - b);
  return { avg, min: inst[0], p1: inst[Math.floor(inst.length * 0.01)], frames: ds.length };
}

// 讀 CDP Performance metrics（累積值），抓 ScriptDuration / TaskDuration（秒）。
async function perfMetrics(client) {
  const { metrics } = await client.send("Performance.getMetrics");
  const m = {};
  for (const x of metrics) m[x.name] = x.value;
  return m;
}

async function main() {
  const mainJs = loadMainJs(SHA);

  const browser = await puppeteer.launch({
    executablePath: CHROME,
    headless: "new",
    args: GPU_ARGS,
    defaultViewport: { width: 1280, height: 800 },
  });

  const result = { sha: SHA, throttle: THROTTLE, ok: false, pageErrors: [] };

  try {
    const page = await browser.newPage();
    await page.setCacheEnabled(false);
    page.on("pageerror", (e) => result.pageErrors.push(String(e).split("\n")[0]));

    const client = await page.target().createCDPSession();
    await client.send("Performance.enable");
    // CPU 節流：把桌機 CPU 壓慢，重現手機那種 CPU-bound 掉幀。
    await client.send("Emulation.setCPUThrottlingRate", { rate: THROTTLE });

    // request interception：把 /3d/main.js 換成指定版本，其餘照舊（three CDN/ws 不動）。
    await page.setRequestInterception(true);
    page.on("request", (req) => {
      const u = req.url();
      if (/\/3d\/main\.js(\?|$)/.test(u)) {
        req.respond({
          status: 200,
          contentType: "text/javascript; charset=utf-8",
          body: mainJs,
        });
      } else {
        req.continue();
      }
    });

    // 注入 rAF 幀率計數器 + draw call 探針（必須在頁面 script 之前）。
    await page.evaluateOnNewDocument(() => {
      const W = window;
      W.__bfqa = { drawCalls: 0, frameDeltas: [], sampling: false, _lastT: 0, gl: null };
      const origGetContext = HTMLCanvasElement.prototype.getContext;
      HTMLCanvasElement.prototype.getContext = function (type, attrs) {
        const ctx = origGetContext.call(this, type, attrs);
        try {
          if (ctx && /webgl/i.test(String(type)) && !W.__bfqa.gl) {
            W.__bfqa.gl = ctx;
            const wrap = (name) => {
              const orig = ctx[name];
              if (typeof orig !== "function") return;
              ctx[name] = function (...a) { W.__bfqa.drawCalls++; return orig.apply(this, a); };
            };
            wrap("drawElements"); wrap("drawArrays");
            wrap("drawElementsInstanced"); wrap("drawArraysInstanced");
          }
        } catch (e) { /* 探針失敗別擋遊戲 */ }
        return ctx;
      };
      const tick = (t) => {
        const q = W.__bfqa;
        if (q.sampling) { if (q._lastT) q.frameDeltas.push(t - q._lastT); q._lastT = t; }
        else { q._lastT = 0; }
        W.requestAnimationFrame(tick);
      };
      W.requestAnimationFrame(tick);
    });

    await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 60000 });
    // 給 Three.js 起 scene、WS 連上、世界資料進來（節流下放寬等待）。
    await new Promise((r) => setTimeout(r, 6000));

    const gl = await page.evaluate(() => {
      const q = window.__bfqa;
      const c = document.querySelector("canvas");
      return { hasGL: !!q.gl, hasCanvas: !!c, w: c ? c.width : 0, h: c ? c.height : 0 };
    });
    result.gl = gl;

    // ── 靜止取樣 ──
    const idleDraw0 = await page.evaluate(() => window.__bfqa.drawCalls);
    const idlePerf0 = await perfMetrics(client);
    await page.evaluate(() => { const q = window.__bfqa; q.frameDeltas = []; q._lastT = 0; q.sampling = true; });
    await new Promise((r) => setTimeout(r, IDLE_SECS * 1000));
    const idleDeltas = await page.evaluate(() => { window.__bfqa.sampling = false; return window.__bfqa.frameDeltas.slice(); });
    const idleDraw1 = await page.evaluate(() => window.__bfqa.drawCalls);
    const idlePerf1 = await perfMetrics(client);
    const idleFps = fpsStats(idleDeltas);
    const idleDrawPerFrame = idleFps.frames ? (idleDraw1 - idleDraw0) / idleFps.frames : NaN;
    const idleScriptMs = idleFps.frames ? ((idlePerf1.ScriptDuration - idlePerf0.ScriptDuration) * 1000) / idleFps.frames : NaN;

    // ── 移動取樣（真按 WASD 繞圈走）──
    await page.bringToFront();
    await page.evaluate(() => { const c = document.querySelector("canvas"); if (c) c.focus(); });
    const moveDraw0 = await page.evaluate(() => window.__bfqa.drawCalls);
    const movePerf0 = await perfMetrics(client);
    await page.evaluate(() => { const q = window.__bfqa; q.frameDeltas = []; q._lastT = 0; q.sampling = true; });
    const dirs = ["w", "d", "s", "a"];
    const endAt = Date.now() + MOVE_SECS * 1000;
    let i = 0;
    while (Date.now() < endAt) {
      const key = dirs[i % dirs.length]; i++;
      await page.keyboard.down(key);
      await new Promise((r) => setTimeout(r, 1000));
      await page.keyboard.up(key);
    }
    for (const k of dirs) { try { await page.keyboard.up(k); } catch (e) { /* 無妨 */ } }
    const moveDeltas = await page.evaluate(() => { window.__bfqa.sampling = false; return window.__bfqa.frameDeltas.slice(); });
    const moveDraw1 = await page.evaluate(() => window.__bfqa.drawCalls);
    const movePerf1 = await perfMetrics(client);
    const moveFps = fpsStats(moveDeltas);
    const moveDrawPerFrame = moveFps.frames ? (moveDraw1 - moveDraw0) / moveFps.frames : NaN;
    const moveScriptMs = moveFps.frames ? ((movePerf1.ScriptDuration - movePerf0.ScriptDuration) * 1000) / moveFps.frames : NaN;

    result.ok = gl.hasGL && (moveDraw1 > moveDraw0);
    result.idle = { fpsAvg: idleFps.avg, fpsMin: idleFps.min, frames: idleFps.frames, drawPerFrame: idleDrawPerFrame, scriptMsPerFrame: idleScriptMs };
    result.moving = { fpsAvg: moveFps.avg, fpsMin: moveFps.min, fpsP1: moveFps.p1, frames: moveFps.frames, drawPerFrame: moveDrawPerFrame, scriptMsPerFrame: moveScriptMs };

    console.log("─".repeat(64));
    console.log(`SHA=${SHA}  throttle=${THROTTLE}x  ${result.ok ? "✅真渲染" : "❌未渲染/疑無效"}`);
    console.log(`  canvas=${gl.w}×${gl.h}  hasGL=${gl.hasGL}  pageErrors=${result.pageErrors.length}`);
    if (result.pageErrors.length) console.log(`  err: ${result.pageErrors.slice(0, 3).join(" | ")}`);
    console.log(`  靜止: FPS avg=${fmt(idleFps.avg)} min=${fmt(idleFps.min)}  draw/幀=${fmt(idleDrawPerFrame, 0)}  JS/幀=${fmt(idleScriptMs, 2)}ms`);
    console.log(`  走路: FPS avg=${fmt(moveFps.avg)} min=${fmt(moveFps.min)} p1=${fmt(moveFps.p1)}  draw/幀=${fmt(moveDrawPerFrame, 0)}  JS/幀=${fmt(moveScriptMs, 2)}ms`);
    console.log("RESULT " + JSON.stringify(result));
  } catch (e) {
    result.error = String(e).split("\n")[0];
    console.log("RESULT " + JSON.stringify(result));
  } finally {
    await Promise.race([browser.close().catch(() => {}), new Promise((r) => setTimeout(r, 8000))]);
    try { const proc = browser.process(); if (proc) proc.kill("SIGKILL"); } catch (e) { /* 已關 */ }
  }
}

main();
