// ============================================================
// voxel-reconnect-qa.mjs — 無痛重連 QA
// 模擬 WS 斷線（如部署重啟），驗重連成功、橫幅安靜、畫面未凍結。
//
// 策略：
//   - 用現有 dev 後端（VQA_URL/port 3001）提供 WS 服務
//   - 以 puppeteer request interception 把 main.js 換成 worktree 新版
//   - 不需啟動額外 server，也不需 pkill
// ============================================================

import puppeteer from "puppeteer-core";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
// 專案根（scripts/qa/../../ = 專案根）
const PROJECT_ROOT = join(__dirname, "..", "..");

// worktree 的新版 main.js（含無痛重連修改）
const NEW_MAIN_JS_PATH = join(PROJECT_ROOT, "web", "voxel", "main.js");

const BASE_URL = process.env.VQA_URL || "http://127.0.0.1:3001/voxel/?debug=1";
const CHROME   = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const OUT_DIR  = process.env.VQA_OUT || join(__dirname, "out");
mkdirSync(OUT_DIR, { recursive: true });

const GPU_ARGS = [
  "--no-sandbox", "--disable-setuid-sandbox", "--ignore-gpu-blocklist",
  "--enable-gpu", "--enable-webgl", "--use-gl=angle", "--use-angle=gl",
  "--disable-dev-shm-usage", "--window-size=1280,800",
  "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding", "--disable-features=CalculateNativeWinOcclusion",
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  let pass = true;
  const log  = (...a) => console.log("[reconnect-qa]", ...a);
  const fail = (msg)  => { console.error("[FAIL]", msg); pass = false; };
  const ok   = (msg)  => log("[OK]", msg);

  // 讀取 worktree 的新 main.js
  let newMainJs;
  try {
    newMainJs = readFileSync(NEW_MAIN_JS_PATH, "utf8");
    log("讀取新 main.js OK（worktree 版，", newMainJs.length, "bytes）");
  } catch (e) {
    console.error("無法讀取", NEW_MAIN_JS_PATH, ":", e.message);
    process.exit(1);
  }

  const browser = await puppeteer.launch({
    headless: "new", executablePath: CHROME, args: GPU_ARGS,
  });
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 800 });

  // 攔截 main.js 請求，換成 worktree 新版（測試我們的重連修改）
  await page.setRequestInterception(true);
  page.on("request", (req) => {
    const u = req.url();
    if (u.includes("main.js")) {
      req.respond({
        status: 200,
        contentType: "application/javascript; charset=utf-8",
        body: newMainJs,
      });
    } else {
      req.continue();
    }
  });

  // 收集主控台訊息
  const consoleLogs = [];
  page.on("console", (m) => {
    const txt = m.text();
    consoleLogs.push(txt);
    if (txt.startsWith("[qa]") || txt.startsWith("[voxel]") || txt.includes("重連") || txt.includes("WS"))
      log("[browser]", txt);
  });
  page.on("pageerror", (e) => {
    consoleLogs.push("[pageerror] " + e.message);
    log("[pageerror]", e.message);
  });

  // 在所有 JS 前注入 WebSocket 代理：把最新 WS 實例暴露給測試。
  // 只添 addEventListener，不覆蓋 onopen/onclose 屬性，不影響遊戲邏輯。
  await page.evaluateOnNewDocument(() => {
    const OrigWS = window.WebSocket;
    window.__wsConnected    = false;
    window.__wsDisconnected = false;
    window.__wsReconnected  = false;
    window.__wsLatest       = null;

    function WSProxy(...args) {
      const inst = new OrigWS(...args);
      window.__wsLatest = inst;
      inst.addEventListener("open", () => {
        if (window.__wsDisconnected) {
          window.__wsReconnected = true;
          console.log("[qa] WS 重連成功");
        }
        window.__wsConnected = true;
        console.log("[qa] WS 連線建立（readyState=" + inst.readyState + "）");
      });
      inst.addEventListener("close", (ev) => {
        if (window.__wsConnected && !window.__wsDisconnected) {
          window.__wsDisconnected = true;
          console.log("[qa] WS 斷線（code=" + ev.code + "）");
        }
      });
      return inst;
    }
    WSProxy.prototype   = OrigWS.prototype;
    WSProxy.CONNECTING  = OrigWS.CONNECTING;
    WSProxy.OPEN        = OrigWS.OPEN;
    WSProxy.CLOSING     = OrigWS.CLOSING;
    WSProxy.CLOSED      = OrigWS.CLOSED;
    window.WebSocket = WSProxy;
  });

  log("載入", BASE_URL, "（main.js 已換成 worktree 新版）");
  await page.goto(BASE_URL, { waitUntil: "domcontentloaded", timeout: 30000 });

  // ── 步驟 1：等首次 WS 連線建立（最多 12s）────────────────────────────
  log("等待首次 WS 連線…");
  let connected = false;
  for (let i = 0; i < 24; i++) {
    connected = await page.evaluate(() => window.__wsConnected);
    if (connected) break;
    await sleep(500);
  }
  if (!connected) {
    fail("首次 WS 連線逾時（12s）——後端可能未啟動？");
    writeFileSync(join(OUT_DIR, "reconnect-timeout.png"), await page.screenshot());
    await browser.close(); process.exit(1);
  }
  ok("首次 WS 連線建立");

  // 等地形與 spawn 處理完
  await sleep(5000);

  // ── 步驟 2：記錄斷線前狀態 ───────────────────────────────────────────
  const hudBefore = await page.evaluate(() => {
    const hud = document.getElementById("hud");
    return hud ? hud.textContent.trim().slice(0, 100) : "(hud not found)";
  });
  log("斷線前 HUD：", hudBefore);
  writeFileSync(join(OUT_DIR, "reconnect-1-before.png"), await page.screenshot());
  ok("截圖：斷線前 → reconnect-1-before.png");

  // ── 步驟 3：模擬部署重啟（強制關閉 WS）───────────────────────────────
  const closed = await page.evaluate(() => {
    const ws = window.__wsLatest;
    if (ws && ws.readyState === 1 /* OPEN */) {
      ws.close(3001, "qa-simulate-restart");
      console.log("[qa] 強制關閉 WS（code 1001），模擬部署重啟");
      return true;
    }
    console.log("[qa] WS 不在 OPEN 態，readyState=", ws ? ws.readyState : "null");
    return false;
  });
  if (!closed) {
    fail("無法模擬斷線：WS 不在 OPEN 態");
    await browser.close(); process.exit(1);
  }

  // ── 步驟 4：安靜期驗證（0.5s 內橫幅不應出現）────────────────────────
  await sleep(500);
  const errAt500ms = await page.evaluate(() => {
    const e = document.getElementById("err");
    if (!e) return false;
    const d = window.getComputedStyle(e).display;
    return d !== "none";
  });
  writeFileSync(join(OUT_DIR, "reconnect-2-quiet.png"), await page.screenshot());
  if (errAt500ms) {
    fail("斷線後 0.5s 橫幅即出現（安靜期失效）→ 部署重啟玩家立刻被嚇到");
  } else {
    ok("安靜期 OK：0.5s 時橫幅未出現 → reconnect-2-quiet.png");
  }

  // ── 步驟 5：等待重連成功（最多 10s）──────────────────────────────────
  log("等待重連…");
  let reconnected = false;
  for (let i = 0; i < 20; i++) {
    reconnected = await page.evaluate(() => window.__wsReconnected);
    if (reconnected) break;
    await sleep(500);
  }

  if (!reconnected) {
    fail("10s 內未重連成功");
    writeFileSync(join(OUT_DIR, "reconnect-fail.png"), await page.screenshot());
  } else {
    ok("重連成功（10s 內）！");
    await sleep(1500); // 等 welcome handler 處理完
    writeFileSync(join(OUT_DIR, "reconnect-3-after.png"), await page.screenshot());
    ok("截圖：重連後 → reconnect-3-after.png");

    // ── 步驟 6：驗重連後橫幅已隱藏 ─────────────────────────────────
    const errAfter = await page.evaluate(() => {
      const e = document.getElementById("err");
      if (!e) return false;
      const d = window.getComputedStyle(e).display;
      return d !== "none";
    });
    if (errAfter) {
      fail("重連後橫幅仍可見（onopen 應已隱藏 errEl）");
    } else {
      ok("重連後橫幅已隱藏（或安靜期內完成，從未出現）");
    }

    // ── 步驟 7：驗 HUD 仍正常（畫面未凍結）──────────────────────────
    const hudAfter = await page.evaluate(() => {
      const hud = document.getElementById("hud");
      return hud ? hud.textContent.trim().slice(0, 100) : "(hud not found)";
    });
    log("重連後 HUD：", hudAfter);
    if (!hudAfter || hudAfter === "(hud not found)" || hudAfter.length < 3) {
      fail("重連後 HUD 為空，畫面可能已凍結");
    } else {
      ok("重連後 HUD 正常：" + hudAfter);
    }
  }

  await browser.close();

  // 輸出摘要
  if (pass) {
    log("\n=== 重連 QA 全部通過 ===");
    process.exit(0);
  } else {
    log("\n=== 重連 QA 有失敗項 ===");
    log("截圖已存至:", OUT_DIR);
    process.exit(1);
  }
})();
