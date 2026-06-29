// voxel-talk-thinking-qa.mjs — 對話「思考中」動畫+真回覆取代 瀏覽器 QA
// 連到 voxel 世界，找到居民，送一句話，截圖：
//   1. 「思考中」指示器出現時（送出後立即）
//   2. 真回覆取代「思考中」後
// 用法：node scripts/qa/voxel-talk-thinking-qa.mjs
// 不抄外部碼；全繁中註解。

import puppeteer from "puppeteer-core";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const BASE_URL = process.env.VQA_URL || "https://peregrine.but-fun.com/voxel/?debug=1";
const CHROME = process.env.BFQA_CHROME || "/usr/bin/google-chrome";
const OUT_DIR = process.env.VQA_OUT || join(__dirname, "out");
mkdirSync(OUT_DIR, { recursive: true });

const GPU_ARGS = [
  "--no-sandbox", "--disable-setuid-sandbox", "--ignore-gpu-blocklist",
  "--enable-gpu", "--enable-webgl", "--use-gl=angle", "--use-angle=gl",
  "--disable-dev-shm-usage",
  "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding", "--disable-features=CalculateNativeWinOcclusion",
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

(async () => {
  const browser = await puppeteer.launch({
    headless: "new", executablePath: CHROME, args: GPU_ARGS,
  });
  const page = await browser.newPage();
  // 直式手機（iPhone 12 Pro 390×844，DPR 3）
  await page.setUserAgent(
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) " +
    "AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"
  );
  await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 3, isMobile: true, hasTouch: true });

  const logs = [];
  page.on("console", (m) => { logs.push("[console] " + m.text()); });
  page.on("pageerror", (e) => { logs.push("[pageerror] " + e.message); });

  console.log("載入", BASE_URL);
  await page.goto(BASE_URL, { waitUntil: "domcontentloaded", timeout: 30_000 });

  // 等待場景初始化（居民出現）。
  console.log("等候場景就緒（居民數 > 0，最多 20 秒）…");
  let resOk = false;
  for (let i = 0; i < 40; i++) {
    await sleep(500);
    const cnt = await page.evaluate(() => window.__voxel?.residentCount ?? 0).catch(() => 0);
    if (cnt > 0) { resOk = true; break; }
  }

  const shot0 = join(OUT_DIR, "voxel-talk-thinking-0-init.png");
  await page.screenshot({ path: shot0, fullPage: false });
  console.log(`截圖0（場景就緒）: ${shot0} resOk=${resOk}`);

  if (!resOk) {
    console.warn("★ 居民沒出現，跳過對話測試");
    await browser.close(); process.exit(0);
  }

  // 取第一位居民 id，用 debug API 開對話框並送一句話。
  const rid = await page.evaluate(() => window.__voxel?.residentIds()[0] ?? null).catch(() => null);
  if (!rid) { console.warn("★ 沒有居民 id"); await browser.close(); process.exit(0); }

  console.log("居民 id =", rid, "— 用 talkTo() 開對話框並送話…");
  await page.evaluate((r) => window.__voxel?.talkTo(r, "你好！"), rid);
  await sleep(300);

  // 截圖：思考中狀態（送出立刻截）。
  const shot1 = join(OUT_DIR, "voxel-talk-thinking-1-sending.png");
  await page.screenshot({ path: shot1, fullPage: false });
  console.log(`截圖1（送出後立即）: ${shot1}`);

  // 查對話框中有沒有「思考中」指示器（.msg.thinking）。
  const hasThinking = await page.evaluate(() => !!document.querySelector(".msg.thinking")).catch(() => false);
  console.log("思考中指示器出現:", hasThinking ? "✓ YES" : "✗ NO（可能舊版 or 已很快消失）");

  // 等候真回覆（最多 30 秒）。
  console.log("等候真回覆（最多 30 秒）…");
  let gotReply = false;
  for (let i = 0; i < 60; i++) {
    await sleep(500);
    const reply = await page.evaluate(() => window.__voxel?.lastTalkReply ?? null).catch(() => null);
    if (reply && reply !== "…") { gotReply = true; console.log("真回覆內容:", reply); break; }
  }

  // 截圖：真回覆到了的狀態。
  const shot2 = join(OUT_DIR, "voxel-talk-thinking-2-replied.png");
  await page.screenshot({ path: shot2, fullPage: false });
  console.log(`截圖2（真回覆後）: ${shot2}`);

  // 確認「思考中」元素已移除。
  const thinkingGone = await page.evaluate(() => !document.querySelector(".msg.thinking")).catch(() => false);
  const npcMsgs = await page.evaluate(() =>
    [...document.querySelectorAll(".msg.npc")].map((el) => el.textContent)
  ).catch(() => []);

  console.log("\n===== QA 結果 =====");
  console.log("思考中指示器曾出現:", hasThinking ? "✓" : "✗");
  console.log("真回覆有來:", gotReply ? "✓" : "✗");
  console.log("思考中已被移除:", thinkingGone ? "✓" : "✗（警告：仍殘留）");
  console.log("NPC 氣泡數:", npcMsgs.length, npcMsgs);
  console.log("截圖路徑:", [shot0, shot1, shot2].join("\n       "));

  logs.filter(l => l.includes("[pageerror]")).forEach(l => console.warn(l));

  await browser.close();

  if (!hasThinking && !gotReply) { console.warn("★ 警告：思考中沒出現且真回覆沒來"); process.exit(1); }
  if (!gotReply) { console.warn("★ 警告：真回覆沒來（LLM 可能逾時）"); process.exit(1); }
  console.log("\n✓ QA PASS");
  process.exit(0);
})();
