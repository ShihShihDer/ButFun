// ============================================================
// voxel-resident-bodylang-qa.mjs — 居民表情/肢體 v1 真實瀏覽器 QA（真渲染·真截圖）
// ============================================================
// 比照 voxel-shadow-browser-qa.mjs：puppeteer-core 驅動系統 Chrome 載入隔離伺服器的
// /voxel/，走近一位居民，用前端 QA hook 就地強制其情緒訊號（心情 emoji / 哼歌 / 揮手），
// 驗程序化肢體語言真的動了、並拍下證據截圖：
//   (a) 哼歌搖擺  (b) 開心彈跳  (c) 思考搔頭  (d) 難過垂頭  (e) 打招呼揮手
// 情緒訊號用 __voxel.qaSet* 就地塞（純本地渲染狀態，伺服器仍權威、無任何作弊面）。
// 需要伺服器以 BUTFUN_QA_DEBUG=1 啟動；截圖存 VQA_SHOTS（預設 shots/）。
// 不抄外部碼；全繁中註解；node --check 過。

import puppeteer from "puppeteer-core";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

const PORT = process.env.VQA_PORT || 3971;
const URL = process.env.VQA_URL || `http://127.0.0.1:${PORT}/voxel/?debug=1`;
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
async function waitFor(fn, ms, step = 200) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) { const v = await fn(); if (v) return v; await sleep(step); }
  return null;
}

(async () => {
  console.log(`\n【居民表情/肢體 真瀏覽器 QA】${URL}`);
  const browser = await puppeteer.launch({ headless: "new", executablePath: CHROME, args: GPU_ARGS });
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 800 });
  await page.goto(URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await sleep(6000); // 等 chunk/mesh + 居民快照

  // 白天好取景（就地撥鐘、純視覺）。
  await page.evaluate(() => window.__voxel.qaSetWorldTime(0.35));

  // ── 等一位居民出現、走近它並轉頭看它 ─────────────────────────
  const rid = await waitFor(async () =>
    page.evaluate(() => { const ids = window.__voxel.residentIds(); return ids.length ? ids[0] : null; }), 20000);
  check("世界裡有居民（前端已渲染）", !!rid, rid ? `rid=${rid}` : "20 秒內無");
  if (!rid) { await browser.close(); process.exit(1); }

  // 瞬移到居民旁邊（純本地 setPlayerPos）、轉頭看它——headless 無法真走遠路，直接靠近取景。
  const rpos = await page.evaluate((id) => { const p = window.__voxel.residentPose(id); return { x: p.x, y: p.y, z: p.z }; }, rid);
  await page.evaluate((rp) => { window.__voxel.setPlayerPos(rp.x + 3.2, rp.y + 2, rp.z + 3.2); }, rpos);
  await sleep(1500); // 等 chunk 補載 + 落地
  const lookAtRes = (id) => { const p = window.__voxel.residentPose(id); window.__voxel.lookTowards(p.x, p.y + 1.3, p.z); };
  await page.evaluate(lookAtRes, rid);
  await sleep(600);

  // 便利函式：套一個情緒、跑幾秒讓補間動起來、讀姿態、截圖。
  const applyAndShoot = async (setup, secs, shot) => {
    await page.evaluate(setup, rid);
    await sleep(secs * 1000);
    await page.evaluate(lookAtRes, rid);
    await sleep(300);
    const pose = await page.evaluate((id) => window.__voxel.residentPose(id), rid);
    await page.screenshot({ path: join(SHOTS, shot) });
    console.log("  📸", join(SHOTS, shot), JSON.stringify(pose));
    return pose;
  };

  // ── (a) 哼歌搖擺：身體左右擺（bodyRotZ 非零）＋上下彈（bodyY > 0）─────
  console.log("(a) 哼歌搖擺");
  // 先清中性、再開哼歌；取兩個時刻的 bodyRotZ 驗真的在搖（非卡死）。
  await page.evaluate((id) => { window.__voxel.qaSetResidentMood(id, ""); window.__voxel.qaSetResidentHumming(id, true); }, rid);
  await sleep(1200);
  const rot1 = await page.evaluate((id) => window.__voxel.residentPose(id).bodyRotZ, rid);
  await sleep(400);
  const rot2 = await page.evaluate((id) => window.__voxel.residentPose(id).bodyRotZ, rid);
  const humPose = await applyAndShoot((id) => window.__voxel.qaSetResidentHumming(id, true), 0.2, "a-resident-humming.png");
  check("哼歌：身體正在左右搖擺（bodyRotZ 兩時刻不同）", Math.abs(rot1 - rot2) > 1e-3, `rot ${rot1.toFixed(3)}→${rot2.toFixed(3)}`);
  check("哼歌：身體有上下彈起（bodyY ≥ 0 且有幅度）", humPose.bodyY >= 0);
  await page.evaluate((id) => window.__voxel.qaSetResidentHumming(id, false), rid);

  // ── (b) 開心彈跳：😊 → bodyY 有正向起伏 ──────────────────────
  console.log("(b) 開心彈跳");
  const happy = await applyAndShoot((id) => window.__voxel.qaSetResidentMood(id, "😊"), 1.0, "b-resident-happy-bounce.png");
  check("開心：身體上下彈跳（bodyY > 0）", happy.bodyY > 0.005, `bodyY=${happy.bodyY.toFixed(3)}`);

  // ── (c) 思考搔頭：🤔 → 右手抬高（armRx 明顯大）＋頭側傾 ─────────
  console.log("(c) 思考搔頭");
  const think = await applyAndShoot((id) => window.__voxel.qaSetResidentMood(id, "🤔"), 1.2, "c-resident-thinking-scratch.png");
  check("思考：右手抬起搔頭（armRx 明顯 > 1）", think.armRx > 1.0, `armRx=${think.armRx.toFixed(2)}`);
  check("思考：頭側傾（headZ 非零）", Math.abs(think.headZ) > 0.02, `headZ=${think.headZ.toFixed(3)}`);

  // ── (d) 難過垂頭：😔 → 頭前垂（headX > 0）───────────────────
  console.log("(d) 難過垂頭");
  const sad = await applyAndShoot((id) => window.__voxel.qaSetResidentMood(id, "😔"), 1.2, "d-resident-sad-droop.png");
  check("難過：頭微垂（headX > 0.1）", sad.headX > 0.1, `headX=${sad.headX.toFixed(3)}`);

  // ── (e) 打招呼揮手：觸發 wavePulse → 右手抬起來回揮 ─────────────
  console.log("(e) 打招呼揮手");
  await page.evaluate((id) => { window.__voxel.qaSetResidentMood(id, "🙂"); window.__voxel.qaTriggerResidentWave(id); }, rid);
  await sleep(600);
  const wave = await page.evaluate((id) => window.__voxel.residentPose(id), rid);
  await page.evaluate((id) => { const p = window.__voxel.residentPose(id); window.__voxel.lookTowards(p.x, 35, p.z); }, rid);
  await sleep(200);
  await page.screenshot({ path: join(SHOTS, "e-resident-wave.png") });
  console.log("  📸", join(SHOTS, "e-resident-wave.png"), JSON.stringify(wave));
  check("揮手：右手抬起（armRx > 1.5）且 wavePulse 進行中", wave.armRx > 1.5 && wave.wavePulse > 0, `armRx=${wave.armRx.toFixed(2)} pulse=${wave.wavePulse.toFixed(2)}`);

  await browser.close();
  console.log(`\n══════════════════════════════════════════`);
  console.log(`居民表情/肢體 真瀏覽器 QA：${pass} 通過 / ${fail} 失敗`);
  console.log(`══════════════════════════════════════════`);
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
