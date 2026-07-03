// voxel-chat-log-qa.mjs — 麥塊式聊天記錄窗 真瀏覽器 QA（手機直式 390×844）
// 驗證：
//   ① 說一句話 → 進左下聊天窗完整顯示（長句不截）、自己頭上泡泡也在。
//   ② 收到居民說話 → 也進 log（走 chatLogAppend("res",...) 模擬廣播路徑）。
//   ③ 展開/收合歷史可捲（chatLogExpanded 切換）。
//   ④ 顏色分辨自己 vs 居民（clog-self vs clog-res class 都在）。
//   ⑤ XSS：送含 <b>/<script> 的訊息 → log 以純文字顯示、不插入真實 <b>/<script> 節點。
//   ⑥ FPS 正常。
// 用法：VQA_URL=http://127.0.0.1:PORT/voxel/?debug=1 node scripts/qa/voxel-chat-log-qa.mjs
// 不抄外部碼；全繁中註解。
import puppeteer from "puppeteer-core";
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const BASE_URL = process.env.VQA_URL || "http://127.0.0.1:3099/voxel/?debug=1";
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
const LONG = "這是一句故意寫得非常非常長的話，長到頭上泡泡一定會被截斷但是左下聊天記錄窗一定要完整顯示不可以截斷讓我一眼看回剛剛到底說了什麼內容才對";

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

  let resOk = false;
  for (let i = 0; i < 40; i++) {
    await sleep(500);
    const cnt = await page.evaluate(() => window.__voxel?.residentCount ?? 0).catch(() => 0);
    if (cnt > 0) { resOk = true; break; }
  }
  console.log("場景就緒 resOk=", resOk);
  if (!resOk) { console.warn("★ 居民沒出現"); await browser.close(); process.exit(1); }

  // ── ① 說一句長話 → 進聊天窗完整（不截）、頭上泡泡也在 ─────────────────────────
  await page.type("#speakInput", LONG);
  await page.click("#speakSend");
  await sleep(500);
  const s1 = await page.evaluate((long) => {
    const v = window.__voxel;
    const lines = v.chatLogLines;
    const self = lines.find((l) => l.kind === "self" && l.text === long);
    return {
      bubbleVisible: v.myBubbleVisible, bubbleText: v.myBubbleText,
      logFull: !!self, logText: self ? self.text : "",
      bubbleTruncated: (v.myBubbleText || "").length < long.length,
    };
  }, LONG);
  console.log("① 泡泡可見:", s1.bubbleVisible, "| 泡泡截斷:", s1.bubbleTruncated,
    "| log 完整:", s1.logFull, "(len=" + s1.logText.length + ")");
  const shot1 = join(OUT_DIR, "voxel-chatlog-1-self-long.png");
  await page.screenshot({ path: shot1 });

  // ── ② 收到居民說話 → 也進 log（模擬廣播接入點）──────────────────────────────
  await page.evaluate(() => {
    window.__voxel.chatLogAppend("res", "露娜", "你好呀旅人，這附近的乙太泉今晚會湧現喔！", "vox_res_0");
    window.__voxel.chatLogAppend("res", "諾娃", "我正在蓋一座小屋，快好了～", "vox_res_1");
  });
  await sleep(200);
  const s2 = await page.evaluate(() => {
    const lines = window.__voxel.chatLogLines;
    return {
      luna: lines.some((l) => l.kind === "res" && l.speaker === "露娜"),
      nova: lines.some((l) => l.kind === "res" && l.speaker === "諾娃"),
      count: lines.length,
    };
  });
  console.log("② 居民入 log:", s2.luna && s2.nova, "| 總行數:", s2.count);

  // ── ④ 顏色分辨：兩位居民名字色不同、且自己/居民 class 分開 ────────────────────
  const s4 = await page.evaluate(() => {
    const bodies = document.querySelectorAll("#chatLogBody .clog-line");
    const selfLine = document.querySelector("#chatLogBody .clog-self");
    const resNames = [...document.querySelectorAll("#chatLogBody .clog-res .clog-name")];
    const colors = resNames.map((n) => n.style.color);
    const uniqueColors = new Set(colors.filter(Boolean));
    return {
      hasSelf: !!selfLine, resCount: resNames.length,
      distinctResColors: uniqueColors.size, total: bodies.length,
    };
  });
  console.log("④ 顏色分辨: 自己行=", s4.hasSelf, "| 居民不同色數=", s4.distinctResColors, "/", s4.resCount);

  // ── ③ 展開/收合可捲 ────────────────────────────────────────────────────────
  // 先塞多行讓內容溢出，再展開讀 scrollHeight > clientHeight（可捲）。
  await page.evaluate(() => {
    for (let i = 0; i < 30; i++) window.__voxel.chatLogAppend("res", "賽勒", "第 " + i + " 行測試捲動內容用的話語。", "vox_res_2");
  });
  await page.click("#chatLogHead"); // 點標題展開
  await sleep(300);
  const s3 = await page.evaluate(() => {
    const b = document.getElementById("chatLogBody");
    return {
      expanded: window.__voxel.chatLogExpanded,
      scrollable: b.scrollHeight > b.clientHeight + 4,
      scrollTop: b.scrollTop, scrollHeight: b.scrollHeight, clientHeight: b.clientHeight,
    };
  });
  console.log("③ 展開:", s3.expanded, "| 可捲:", s3.scrollable, "(sh=" + s3.scrollHeight + " ch=" + s3.clientHeight + ")");
  const shot3 = join(OUT_DIR, "voxel-chatlog-3-expanded.png");
  await page.screenshot({ path: shot3 });
  // 捲回頂端看歷史
  await page.evaluate(() => { document.getElementById("chatLogBody").scrollTop = 0; });
  await sleep(150);
  await page.click("#chatLogHead"); // 收合
  await sleep(300);
  const collapsed = await page.evaluate(() => window.__voxel.chatLogExpanded === false);
  console.log("③ 收合回精簡條:", collapsed);

  // ── ⑤ XSS：含 <b>/<script> 的訊息 → 純文字顯示、不建立真實節點 ────────────────
  const s5 = await page.evaluate(() => {
    const evil = 'X<script>window.__XSS_FIRED__=1<\/script><b>bold</b><img src=x onerror="window.__XSS_FIRED__=1">';
    window.__voxel.chatLogAppend("res", "駭客<b>x</b>", evil, "vox_res_3");
    // 讀該行實際 DOM：不應有真實 <script>/<b>/<img> 子節點，文字要原樣（含角括號字元）。
    const lines = [...document.querySelectorAll("#chatLogBody .clog-line")];
    const last = lines[lines.length - 1];
    const textEl = last.querySelector(".clog-text");
    const nameEl = last.querySelector(".clog-name");
    return {
      fired: !!window.__XSS_FIRED__,
      innerTags: last.querySelectorAll("script,img,b").length, // 訊息裡的標籤都應被當文字，不成節點
      textContentHasAngle: textEl.textContent.includes("<script>") && textEl.textContent.includes("<b>"),
      textElChildElems: textEl.children.length, // textContent 寫入 → 0 個元素子節點
      nameShownAsText: nameEl.textContent.includes("<b>"),
    };
  });
  await sleep(300);
  const s5b = await page.evaluate(() => ({ firedLater: !!window.__XSS_FIRED__ }));
  console.log("⑤ XSS: 執行=", s5.fired || s5b.firedLater, "| 訊息內標籤節點數=", s5.innerTags,
    "| 文字保留角括號=", s5.textContentHasAngle, "| textEl 子元素=", s5.textElChildElems);
  const shot5 = join(OUT_DIR, "voxel-chatlog-5-xss.png");
  await page.screenshot({ path: shot5 });

  // ── ⑥ FPS ─────────────────────────────────────────────────────────────────
  await sleep(1200);
  let fpsSum = 0, n = 0;
  for (let i = 0; i < 6; i++) {
    await sleep(300);
    const f = await page.evaluate(() => window.__voxel?.fps ?? 0).catch(() => 0);
    if (f > 0) { fpsSum += f; n++; }
  }
  const fps = n ? Math.round(fpsSum / n) : 0;
  console.log("⑥ 平均 FPS:", fps);

  await browser.close();

  const pass = {
    logFull: s1.logFull && s1.logText.length === LONG.length,
    bubbleTrunc: s1.bubbleTruncated && s1.bubbleVisible,
    resInLog: s2.luna && s2.nova,
    colorDistinct: s4.hasSelf && s4.distinctResColors >= 2,
    scrollable: s3.expanded && s3.scrollable && collapsed,
    xssSafe: !(s5.fired || s5b.firedLater) && s5.innerTags === 0 && s5.textContentHasAngle && s5.textElChildElems === 0,
    fps: fps >= 30,
  };
  console.log("\n===== QA 結果 =====");
  console.log("① log 完整不截:", pass.logFull ? "✓" : "✗", "| 泡泡截斷並存:", pass.bubbleTrunc ? "✓" : "✗");
  console.log("② 居民說話進 log:", pass.resInLog ? "✓" : "✗");
  console.log("③ 展開可捲/收合:", pass.scrollable ? "✓" : "✗");
  console.log("④ 顏色分辨自己vs居民:", pass.colorDistinct ? "✓" : "✗");
  console.log("⑤ XSS 防護(純文字不執行):", pass.xssSafe ? "✓" : "✗");
  console.log("⑥ FPS:", fps, pass.fps ? "✓" : "✗");
  console.log("截圖:", [shot1, shot3, shot5].join("\n      "));
  logs.forEach((l) => console.warn(l));

  const ok = Object.values(pass).every(Boolean);
  if (!ok) { console.warn("★ QA 未全綠"); process.exit(1); }
  console.log("\n✓ QA PASS");
  process.exit(0);
})();
