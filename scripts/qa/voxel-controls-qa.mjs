// ============================================================
// voxel-controls-qa.mjs — 操作大改真實瀏覽器 QA
// ============================================================
// 驗證「麥塊 Bedrock 式操作」：
//   ① 手機直式 390×844 準心+按鈕模式：拖曳只轉視角「不誤挖」、按挖鈕才挖、
//      設定面板開得起來調得動且持久化（重載保留）、（mock）手把偵測 + A 鍵跳。
//   ② 桌機鍵盤/滑鼠仍能玩：WASD 移動、數字鍵選欄、F5 切人稱、Esc 關面板、預設第一人稱。
// 用 puppeteer-core 驅動系統 Chrome；手機/桌機各用獨立 storage context 避免 localStorage 互汙。
// 不抄外部碼；全繁中註解；node --check 過。
import puppeteer from "puppeteer-core";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const BASE = process.env.VQA_URL || "http://127.0.0.1:3000/voxel/?debug=1";
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const OUT = process.env.VQA_OUT || join(__dirname, "out");
mkdirSync(OUT, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const GPU = ["--no-sandbox","--disable-setuid-sandbox","--ignore-gpu-blocklist","--enable-gpu",
  "--enable-webgl","--use-gl=angle","--use-angle=gl","--disable-dev-shm-usage",
  "--disable-background-timer-throttling","--disable-renderer-backgrounding"];

const results = { checks: [] };
const check = (name, ok, extra) => { results.checks.push({ name, ok: !!ok, extra }); console.log(`  ${ok ? "✓" : "✗"} ${name}${extra != null ? "  ("+extra+")" : ""}`); };

(async () => {
  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU });
  const newContext = async () => {
    if (browser.createBrowserContext) return browser.createBrowserContext();
    if (browser.createIncognitoBrowserContext) return browser.createIncognitoBrowserContext();
    return browser.defaultBrowserContext();
  };

  // ── ① 手機直式 390×844：準心+按鈕模式防誤觸 + 設定面板持久化（獨立 storage context）──
  console.log("\n【① 手機直式 390×844 準心+按鈕模式】");
  {
    const ctx = await newContext();
    const page = await ctx.newPage();
    // 進場前 mock 一個 Xbox 手把（headless 無實體手把），供手把偵測 QA。
    await page.evaluateOnNewDocument(() => {
      window.__mockPad = { id: "Mock Xbox 360 Controller", index: 0, connected: true,
        axes: [0,0,0,0], buttons: Array.from({length:16},()=>({pressed:false,value:0})),
        mapping: "standard", timestamp: 0 };
      navigator.getGamepads = () => [window.__mockPad, null, null, null];
    });
    await page.setUserAgent("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1");
    await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 3, isMobile: true, hasTouch: true });
    const logs = [];
    page.on("console", (m) => logs.push("[c] " + m.text()));
    page.on("pageerror", (e) => logs.push("[E] " + e.message));
    await page.goto(BASE, { waitUntil: "domcontentloaded", timeout: 30000 });
    await page.bringToFront();
    await sleep(6000);

    const st = await page.evaluate(() => ({
      touchMode: window.__voxel.settings.touchMode,
      chunks: window.__voxel.chunks, fps: window.__voxel.fps,
      digShown: getComputedStyle(document.getElementById("dig")).display !== "none",
      placeShown: getComputedStyle(document.getElementById("place")).display !== "none",
      joyShown: getComputedStyle(document.getElementById("joy")).display !== "none",
    }));
    check("預設為準心+按鈕模式", st.touchMode === "crosshair", st.touchMode);
    check("世界載入 chunks>0", st.chunks > 0, st.chunks);
    check("FPS>30", st.fps > 30, st.fps.toFixed(1));
    check("挖鈕顯示（準心模式）", st.digShown);
    check("放置鈕顯示", st.placeShown);
    check("搖桿顯示", st.joyShown);

    const cross = await page.evaluate(() => { const r = document.getElementById("crosshair").getBoundingClientRect(); return { cx: r.left + r.width/2, cy: r.top + r.height/2 }; });
    check("準心置中", Math.abs(cross.cx - 195) < 30 && Math.abs(cross.cy - 422) < 30, `${cross.cx.toFixed(0)},${cross.cy.toFixed(0)}`);

    // 防誤觸核心：畫面中央大幅拖曳 → 只應轉視角、不應挖（mining 恆 null）。
    await page.evaluate(() => { window.__yaw0 = window.__voxel.player.yaw; });
    const tsc = page.touchscreen;
    await tsc.touchStart(195, 400);
    await tsc.touchMove(250, 400);
    await tsc.touchMove(300, 420);
    await tsc.touchEnd();
    await sleep(300);
    const afterDrag = await page.evaluate(() => ({ yaw: window.__voxel.player.yaw, yaw0: window.__yaw0, mining: window.__voxel.mining, touchDigHeld: window.__voxel.touchDigHeld }));
    check("拖曳有轉視角（yaw 變了）", Math.abs(afterDrag.yaw - afterDrag.yaw0) > 0.01, (afterDrag.yaw - afterDrag.yaw0).toFixed(3));
    check("拖曳未觸發挖掘（mining=null）", afterDrag.mining === null);
    check("拖曳未按住挖（touchDigHeld=false）", afterDrag.touchDigHeld === false);

    // 按「挖鈕」才挖（等同按住挖鈕）。
    const dig = await page.evaluate(() => {
      const t = window.__voxel.target;
      const r = window.__voxel.touchDigStart();
      const miningAfter = window.__voxel.mining;
      window.__voxel.touchDigEnd();
      return { target: t, action: r, miningStarted: miningAfter != null };
    });
    check("按挖鈕啟動計時挖掘（有 target 時 mining!=null）", dig.target ? dig.miningStarted : true, `action=${dig.action}, hadTarget=${!!dig.target}`);

    // 設定面板：開得起來、改靈敏度、持久化。
    await page.evaluate(() => window.__voxel.openSettingsPanel());
    await sleep(200);
    check("設定面板開得起來", await page.evaluate(() => window.__voxel.settingsPanelVisible));
    await page.screenshot({ path: join(OUT, "controls-mobile-settings.png") });

    await page.evaluate(() => { const s = document.getElementById("setSensitivity"); s.value = "1.8"; s.dispatchEvent(new Event("input", { bubbles: true })); });
    const sens = await page.evaluate(() => ({ set: window.__voxel.settings.sensitivity, ls: JSON.parse(localStorage.getItem("butfun.voxel.settings.v1")||"{}").sensitivity }));
    check("靈敏度改為 1.8 並存入 localStorage", Math.abs(sens.set - 1.8) < 0.001 && Math.abs(sens.ls - 1.8) < 0.001, `set=${sens.set}, ls=${sens.ls}`);

    await page.evaluate(() => { const m = document.getElementById("setTouchMode"); m.value = "tap"; m.dispatchEvent(new Event("change", { bubbles: true })); });
    const tapMode = await page.evaluate(() => ({ mode: window.__voxel.settings.touchMode, digHidden: getComputedStyle(document.getElementById("dig")).display === "none" }));
    check("切點擊互動模式後挖鈕隱藏", tapMode.mode === "tap" && tapMode.digHidden);
    await page.evaluate(() => { const m = document.getElementById("setTouchMode"); m.value = "crosshair"; m.dispatchEvent(new Event("change", { bubbles: true })); window.__voxel.closeSettingsPanel(); });

    const gp = await page.evaluate(() => window.__voxel.pollGamepad(0.016));
    check("偵測到（mock）手把", gp.connected && /Xbox/.test(gp.name), gp.name);
    const jump = await page.evaluate(() => {
      window.__voxel.player.vy = 0; window.__voxel.player.grounded = true;
      window.__mockPad.buttons[0] = { pressed: true, value: 1 };
      window.__voxel.pollGamepad(0.016);
      const vy = window.__voxel.player.vy;
      window.__mockPad.buttons[0] = { pressed: false, value: 0 };
      window.__voxel.pollGamepad(0.016);
      return vy;
    });
    check("手把 A 鍵觸發跳（vy>0）", jump > 0, jump.toFixed(1));

    // 重載後設定持久化（真正的「重載保留」）。
    await page.reload({ waitUntil: "domcontentloaded" });
    await sleep(4000);
    const reloaded = await page.evaluate(() => ({ sens: window.__voxel.settings.sensitivity, mode: window.__voxel.settings.touchMode }));
    check("重載後靈敏度仍=1.8", Math.abs(reloaded.sens - 1.8) < 0.001, reloaded.sens);
    check("重載後仍為準心+按鈕模式", reloaded.mode === "crosshair", reloaded.mode);

    await page.evaluate(() => window.__voxel.closeSettingsPanel());
    await sleep(200);
    await page.screenshot({ path: join(OUT, "controls-mobile-crosshair.png") });
    results.pageErrors = logs.filter(l=>l.startsWith("[E]"));
    if (results.pageErrors.length) console.log("  頁面錯誤:\n  " + results.pageErrors.join("\n  "));
    await page.close();
  }

  // ── ② 桌機 1280×800 鍵盤/滑鼠（獨立 storage）──
  console.log("\n【② 桌機 1280×800 鍵盤/滑鼠】");
  {
    const ctx = await newContext();
    const page = await ctx.newPage();
    await page.setViewport({ width: 1280, height: 800, deviceScaleFactor: 1 });
    const logs = [];
    page.on("pageerror", (e) => logs.push("[E] " + e.message));
    await page.goto(BASE, { waitUntil: "domcontentloaded", timeout: 30000 });
    await page.bringToFront();
    await sleep(6000);

    const st = await page.evaluate(() => ({ chunks: window.__voxel.chunks, fps: window.__voxel.fps, view: window.__voxel.viewMode }));
    check("桌機世界載入", st.chunks > 0, st.chunks);
    check("桌機 FPS>30", st.fps > 30, st.fps.toFixed(1));
    check("桌機預設第一人稱", st.view === "first", st.view);

    const before = await page.evaluate(() => ({ x: window.__voxel.player.x, z: window.__voxel.player.z }));
    await page.keyboard.down("KeyW"); await sleep(700); await page.keyboard.up("KeyW");
    const after = await page.evaluate(() => ({ x: window.__voxel.player.x, z: window.__voxel.player.z }));
    const moved = Math.hypot(after.x - before.x, after.z - before.z);
    check("按 W 前進（位移>0.3）", moved > 0.3, moved.toFixed(2));

    await page.keyboard.press("Digit2");
    check("數字鍵 2 選到第 2 格", (await page.evaluate(() => window.__voxel.selectedSlot)) === 1);

    await page.keyboard.press("F5");
    check("F5 切到第三人稱", (await page.evaluate(() => window.__voxel.viewMode)) === "third");
    await page.keyboard.press("F5");

    await page.evaluate(() => window.__voxel.openSettingsPanel());
    await page.keyboard.press("Escape");
    check("Esc 關設定面板", await page.evaluate(() => !window.__voxel.settingsPanelVisible));

    await page.screenshot({ path: join(OUT, "controls-desktop.png") });
    if (logs.length) { console.log("  頁面錯誤:\n  " + logs.join("\n  ")); results.pageErrors = (results.pageErrors||[]).concat(logs); }
    await page.close();
  }

  await browser.close();
  const pass = results.checks.every(c => c.ok) && (!results.pageErrors || results.pageErrors.length === 0);
  console.log("\n══════════════════════════════════");
  console.log(`整體判定: ${pass ? "PASS ✅" : "FAIL ❌"}  (${results.checks.filter(c=>c.ok).length}/${results.checks.length} checks)`);
  writeFileSync(join(OUT, "voxel-controls-qa.json"), JSON.stringify(results, null, 2));
  process.exit(pass ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
