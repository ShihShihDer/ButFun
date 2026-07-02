// ============================================================
// voxel-pave-qa.mjs — 乙太方界「鋪面任務」實測 QA（raw WebSocket，免瀏覽器）
// ============================================================
// 對隔離伺服器（記憶體模式、獨立 port、乾淨 data/）連 /voxel/ws，模擬玩家：
//   (a) 單人：對露娜說「幫我把這裡鋪成石磚地」→ 驗證她答應、備料（挖礦井/合成）、
//       地表方塊真的一塊塊變成石磚（b=9）、最後完工 Feed。
//   (b) 協調：換個地點說「是說100×100石磚地 找大家做的如何」（維護者原句）→
//       驗證誠實回覆（先從一塊開始鋪）、號召多位居民、多個子區出現石磚。
// 用法：BFQA_PORT=3111 node scripts/qa/voxel-pave-qa.mjs
// 不抄外部碼；全繁中註解；node --check 過。

const PORT = process.env.BFQA_PORT || "3111";
const WS_URL = `ws://127.0.0.1:${PORT}/voxel/ws`;
const FEED_URL = `http://127.0.0.1:${PORT}/voxel/feed`;
const STONE_BRICK = 9;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ── 共用：一條 ws 連線 + 訊息收集 ─────────────────────────────────────────────
function connect(name) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(WS_URL);
    const state = {
      ws,
      talks: [],            // {resident_id, name, reply}
      bricks: new Map(),    // "x,z" -> y（收到的石磚放置廣播）
      residents: new Map(), // id -> {x,y,z,name,say}
      raw: 0,
    };
    ws.addEventListener("open", () => {
      ws.send(JSON.stringify({ t: "join", name }));
      resolve(state);
    });
    ws.addEventListener("error", (e) => reject(new Error("ws error: " + e.message)));
    ws.addEventListener("message", (ev) => {
      state.raw++;
      let m;
      try { m = JSON.parse(ev.data); } catch { return; }
      if (m.t === "talk") state.talks.push(m);
      if (m.t === "block" && m.b === STONE_BRICK) state.bricks.set(`${m.x},${m.z}`, m.y);
      if (m.t === "players" && Array.isArray(m.residents)) {
        for (const r of m.residents) state.residents.set(r.id, r);
      }
    });
  });
}

const send = (st, obj) => st.ws.send(JSON.stringify(obj));

async function feed() {
  try {
    const res = await fetch(FEED_URL);
    return await res.json();
  } catch { return []; }
}

// 等待條件成立（每 500ms 查一次，逾時回 false）。
async function waitFor(cond, timeoutMs, label) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    if (await cond()) return true;
    await sleep(500);
  }
  console.log(`  ✗ 等候逾時（${Math.round(timeoutMs / 1000)}s）：${label}`);
  return false;
}

let failures = 0;
const check = (ok, label) => {
  console.log(`  ${ok ? "✓" : "✗"} ${label}`);
  if (!ok) failures++;
};

(async () => {
  console.log(`連線 ${WS_URL} …`);
  const st = await connect("鋪面QA");
  await sleep(1500); // 等第一波 players/residents 快照

  const luna = st.residents.get("vox_res_0");
  check(!!luna, `看得到露娜（vox_res_0）：${luna ? `(${luna.x.toFixed(1)}, ${luna.z.toFixed(1)})` : "無"}`);
  if (!luna) process.exit(1);

  // ── (a) 單人鋪面：站到露娜身邊，請她把這裡鋪成石磚地 ─────────────────────────
  console.log("\n[a] 單人鋪面：走到露娜旁 → 「幫我把這裡鋪成石磚地」");
  const ax = Math.floor(luna.x) + 2.5, az = Math.floor(luna.z) + 2.5;
  send(st, { t: "move", x: ax, y: luna.y, z: az, yaw: 0 });
  await sleep(300);
  send(st, { t: "talk", resident_id: "vox_res_0", text: "露娜，幫我把這裡鋪成石磚地" });

  const gotAccept = await waitFor(() => st.talks.length > 0, 10_000, "露娜回覆");
  const reply = st.talks[0]?.reply || "";
  check(gotAccept, `露娜口頭答應：「${reply}」`);
  check(/石磚/.test(reply) && /(備|採|料|鋪)/.test(reply), "回覆坦白要備料、材料正確（石磚）");

  // 目標範圍：玩家腳邊 7×7（PAVE_RADIUS=3）＝ 49 柱。等它真的變石磚。
  const cax = Math.floor(ax), caz = Math.floor(az);
  const inSiteA = ([k]) => {
    const [x, z] = k.split(",").map(Number);
    return Math.abs(x - cax) <= 3 && Math.abs(z - caz) <= 3;
  };
  const siteBricks = () => [...st.bricks.entries()].filter(inSiteA).length;
  // 完工門檻：49 柱中 ≥45 變石磚（她站的那 1-2 柱依可逃精神可跳過）。
  const paved = await waitFor(async () => siteBricks() >= 45, 300_000, "7×7 範圍鋪滿石磚");
  check(paved, `地表真的變石磚：範圍內收到 ${siteBricks()}/49 柱石磚放置廣播`);

  let fa = await feed();
  const kinds = (k, re) => fa.filter((e) => e.kind === k && re.test(e.text || e.detail || JSON.stringify(e)));
  check(kinds("鋪面", /答應|備料/).length > 0, "Feed：答應+備料");
  check(kinds("鋪面", /礦井/).length > 0, "Feed：開挖階梯礦井備石頭（誠實備料證據）");
  const doneA = await waitFor(async () => {
    fa = await feed();
    return kinds("鋪面", /鋪上石磚了/).length > 0;
  }, 60_000, "完工 Feed");
  check(doneA, "Feed：把那塊地鋪上石磚了（完工）");

  // ── (b) 協調鋪面：換個地點，用維護者原句叫大家鋪 100×100 ─────────────────────
  console.log("\n[b] 協調鋪面：維護者原句「是說100×100石磚地 找大家做的如何」");
  st.bricks.clear();
  st.talks.length = 0;
  // 移到離 (a) 工地遠一點的位置（避免範圍重疊），仍在露娜聽力範圍外沒關係——指名對話不受半徑限制。
  const bx = cax + 40.5, bz = caz + 40.5;
  send(st, { t: "move", x: bx, y: 20, z: bz, yaw: 0 });
  await sleep(500);
  send(st, { t: "talk", resident_id: "vox_res_0", text: "是說100×100石磚地 找大家做的如何" });

  const gotRally = await waitFor(() => st.talks.length > 0, 10_000, "露娜號召回覆");
  const rally = st.talks[0]?.reply || "";
  check(gotRally, `露娜回覆：「${rally}」`);
  check(/一塊/.test(rally), "誠實回應：先從一塊開始鋪、一塊一塊來（不拒絕、不吹牛）");

  fa = await feed();
  const rallyFeed = fa.find((e) => e.kind === "鋪面" && /號召/.test(JSON.stringify(e)));
  check(!!rallyFeed, `Feed：號召多位居民分工（${rallyFeed ? JSON.stringify(rallyFeed).slice(0, 80) : "無"}）`);

  // 協調子區：4 人 2×2、子區半徑 4 → 整片 18×18 置中於玩家。驗證「多個象限」都出現石磚。
  const quadrant = ([k]) => {
    const [x, z] = k.split(",").map(Number);
    if (Math.abs(x - bx) > 10 || Math.abs(z - bz) > 10) return null; // 不在這片工地
    return `${x < bx ? "W" : "E"}${z < bz ? "N" : "S"}`;
  };
  const quadsHit = () => new Set([...st.bricks.entries()].map(quadrant).filter(Boolean));
  const multi = await waitFor(
    async () => quadsHit().size >= 2 && st.bricks.size >= 30,
    420_000,
    "多子區出現石磚（分工證據）"
  );
  check(multi, `多子區真的在鋪：命中象限 ${[...quadsHit()].join("/") || "無"}、共 ${st.bricks.size} 柱石磚`);

  // 整片完工（等得到就加分；等不到不算失敗——分工證據已足）。
  const doneB = await waitFor(async () => {
    fa = await feed();
    return fa.some((e) => e.kind === "鋪面" && /齊心|鋪上了/.test(JSON.stringify(e)));
  }, 300_000, "協調完工 Feed（非硬性）");
  console.log(`  ${doneB ? "✓" : "△"} 協調整片完工 Feed：${doneB ? "有" : "未在時限內（分工證據已足）"}`);
  if (doneB) console.log(`    完工時共收到 ${st.bricks.size} 柱石磚`);

  // ── 收尾 ────────────────────────────────────────────────────────────────────
  console.log("\nFeed 鋪面相關事件：");
  fa = await feed();
  for (const e of fa.filter((e) => e.kind === "鋪面")) {
    console.log("  -", JSON.stringify(e));
  }
  st.ws.close();
  console.log(failures === 0 ? "\n★ 鋪面 QA 全數通過" : `\n★ 鋪面 QA 有 ${failures} 項失敗`);
  process.exit(failures === 0 ? 0 : 1);
})().catch((e) => {
  console.error("QA 執行錯誤：", e);
  process.exit(1);
});
