// ============================================================
// voxel-held-item-qa.mjs — 手持工具可見 v1 真實瀏覽器 QA
// ============================================================
// 驗證：切熱鍵後 window.__voxel.myHeld 隨之換色/顯隱；伺服器收到 held 欄位並
// 廣播給其他連線（用兩個分頁互看）；空熱鍵格正確隱藏，無 console 錯誤。
// 不抄外部碼；全繁中註解；node --check 過。
import puppeteer from "puppeteer-core";
const BASE = process.env.VQA_URL || "http://127.0.0.1:19934/voxel/?debug=1";
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const GPU = ["--no-sandbox","--disable-setuid-sandbox","--ignore-gpu-blocklist","--enable-gpu",
  "--enable-webgl","--use-gl=angle","--use-angle=gl","--disable-dev-shm-usage"];

const results = [];
const check = (name, ok, extra) => { results.push({ name, ok: !!ok }); console.log(`  ${ok ? "✓" : "✗"} ${name}${extra != null ? "  ("+extra+")" : ""}`); };

(async () => {
  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU });
  const ctxA = await browser.createBrowserContext();
  const ctxB = await browser.createBrowserContext();
  const pageA = await ctxA.newPage();
  const pageB = await ctxB.newPage();
  const logsA = [];
  pageA.on("console", (m) => logsA.push("[c] " + m.text()));
  pageA.on("pageerror", (e) => logsA.push("[E] " + e.message));

  console.log("\n【手持工具可見 v1】");
  await pageA.goto(BASE, { waitUntil: "domcontentloaded", timeout: 30000 });
  await pageA.bringToFront();
  await sleep(4000);
  await pageB.goto(BASE, { waitUntil: "domcontentloaded", timeout: 30000 });
  await pageB.bringToFront();
  await sleep(4000);

  const before = await pageA.evaluate(() => window.__voxel.myHeld);
  check("初始熱鍵格有預設物品即顯示手持", before.visible === true, JSON.stringify(before));

  const afterSet = await pageA.evaluate(() => window.__voxel.setHeldItem(1));
  check("setHeldItem(1) 後 myHeld 顯示且 id 正確", afterSet.visible === true && afterSet.id === 1, JSON.stringify(afterSet));

  // A 送出帶 held 的 move（等 sendMove 節流 tick 過）→ B 應能在 others 中讀到同一個 id。
  await sleep(1500);
  const myIdA = await pageA.evaluate(() => window.__voxel.playerId);
  const seenByB = await pageB.evaluate((id) => window.__voxel.otherHeld(id), myIdA);
  check("B 分頁讀到 A 的 held 廣播", !!seenByB && seenByB.id === 1, JSON.stringify(seenByB));

  const cleared = await pageA.evaluate(() => window.__voxel.setHeldItem(null));
  check("清空熱鍵格後 myHeld 隱藏", cleared.visible === false, JSON.stringify(cleared));

  check("A 分頁無 console 錯誤", !logsA.some((l) => l.startsWith("[E]")), logsA.filter((l)=>l.startsWith("[E]")).join(" | "));

  await browser.close();
  const fail = results.filter((r) => !r.ok);
  console.log(`\n${results.length - fail.length}/${results.length} 通過`);
  process.exit(fail.length ? 1 : 0);
})();
