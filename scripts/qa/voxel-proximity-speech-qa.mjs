// voxel-proximity-speech-qa.mjs — embodied 靠近說話 v1 真瀏覽器 QA（直式 390×844）
// 驗證：
//   ① 打字說話 → 自己頭上冒泡（myBubbleVisible）。
//   ② 走近居民範圍說話 → 牠回話（lastTalkReply，泡泡在世界裡、非黑屏）。
//   ③ FPS 正常。
// 用法：VQA_URL=http://127.0.0.1:3009/voxel/?debug=1 node scripts/qa/voxel-proximity-speech-qa.mjs
// 不抄外部碼；全繁中註解。

import puppeteer from "puppeteer-core";
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const BASE_URL = process.env.VQA_URL || "http://127.0.0.1:3009/voxel/?debug=1";
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
  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU_ARGS });
  const page = await browser.newPage();
  await page.setUserAgent(
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) " +
    "AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"
  );
  await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 3, isMobile: true, hasTouch: true });

  const logs = [];
  page.on("pageerror", (e) => { logs.push("[pageerror] " + e.message); });

  console.log("載入", BASE_URL);
  await page.goto(BASE_URL, { waitUntil: "domcontentloaded", timeout: 30_000 });

  // 等待居民出現（場景就緒）。
  let resOk = false;
  for (let i = 0; i < 40; i++) {
    await sleep(500);
    const cnt = await page.evaluate(() => window.__voxel?.residentCount ?? 0).catch(() => 0);
    if (cnt > 0) { resOk = true; break; }
  }
  console.log("場景就緒 resOk=", resOk);
  if (!resOk) { console.warn("★ 居民沒出現"); await browser.close(); process.exit(1); }

  // ── ① 打字說話 → 自己頭上冒泡 ──────────────────────────────────────────────
  // 走真實 DOM：在常駐輸入列打字、按「說」。
  await page.type("#speakInput", "你好，這裡好安靜！");
  await page.click("#speakSend");
  await sleep(400);
  const b1 = await page.evaluate(() => ({
    visible: window.__voxel?.myBubbleVisible ?? false,
    text: window.__voxel?.myBubbleText ?? "",
  }));
  console.log("① 自己頭上泡泡:", JSON.stringify(b1));
  const shot1 = join(OUT_DIR, "voxel-proximity-1-self-bubble.png");
  await page.screenshot({ path: shot1 });

  // ── ② 走近居民範圍說話 → 牠回話 ────────────────────────────────────────────
  // 把玩家瞬移到第一位居民正前方（-z 方向，yaw=0 面對牠），等位置上報後範圍說話。
  const placed = await page.evaluate(() => {
    const info = window.__voxel?.residentInfo?.() || [];
    if (!info.length) return null;
    const r = info[0];
    const p = window.__voxel.player;
    p.x = r.x; p.z = r.z + 3; p.y = r.y; p.yaw = 0; // 站到 -z 前方 3 格、面向居民
    return { rid: r.id, name: r.name, x: r.x, y: r.y, z: r.z };
  });
  console.log("② 瞬移到居民旁:", JSON.stringify(placed));
  await sleep(900); // 等 move 上報（每 0.1s 一次）讓伺服器知道新位置

  // 範圍說話（不指定居民）→ 伺服器挑最近/面對者回話。
  await page.evaluate(() => window.__voxel?.speak?.("這附近可以種田嗎？我想探索看看"));
  await sleep(400);
  const shot2a = join(OUT_DIR, "voxel-proximity-2a-said.png");
  await page.screenshot({ path: shot2a });

  // 等回覆（LLM 未啟用時走罐頭後備，仍會回；最多 30 秒）。
  let reply = null;
  for (let i = 0; i < 60; i++) {
    await sleep(500);
    reply = await page.evaluate(() => window.__voxel?.lastTalkReply ?? null).catch(() => null);
    if (reply && reply !== "…") break;
  }
  console.log("② 居民回覆:", reply);
  // 居民頭上泡泡（世界裡）：等 tick 套用 say 後讀 residentInfo 看 say 是否出現。
  await sleep(800);
  const resSay = await page.evaluate(() => {
    const info = window.__voxel?.residentInfo?.() || [];
    return info.map((r) => ({ name: r.name, say: r.say, visible: r.visible }));
  });
  const anyResBubble = resSay.some((r) => r.say && r.say.length > 0);
  console.log("② 居民世界泡泡:", JSON.stringify(resSay));
  const shot2b = join(OUT_DIR, "voxel-proximity-2b-reply.png");
  await page.screenshot({ path: shot2b });

  // ── ③ FPS ───────────────────────────────────────────────────────────────
  await sleep(1500);
  let fpsSum = 0, n = 0;
  for (let i = 0; i < 6; i++) {
    await sleep(300);
    const f = await page.evaluate(() => window.__voxel?.fps ?? 0).catch(() => 0);
    if (f > 0) { fpsSum += f; n++; }
  }
  const fps = n ? Math.round(fpsSum / n) : 0;
  console.log("③ 平均 FPS:", fps);

  console.log("\n===== QA 結果 =====");
  console.log("① 打字→自己冒泡:", b1.visible ? "✓" : "✗", "(" + b1.text + ")");
  console.log("② 範圍說話→居民回話:", reply && reply !== "…" ? "✓" : "✗", "| 居民世界泡泡:", anyResBubble ? "✓" : "✗");
  console.log("③ FPS:", fps, fps >= 30 ? "✓" : "✗");
  console.log("截圖:", [shot1, shot2a, shot2b].join("\n      "));
  logs.forEach((l) => console.warn(l));

  await browser.close();
  const pass = b1.visible && reply && reply !== "…" && fps >= 30;
  if (!pass) { console.warn("★ QA 未全綠"); process.exit(1); }
  console.log("\n✓ QA PASS");
  process.exit(0);
})();
