// ============================================================
// voxel-tool-wear-qa.mjs — 工具耐久＋鐵匠代客保養 v1（自主提案切片，ROADMAP 981）真 WS 功能 QA
// ============================================================
// 真 WebSocket 直連隔離伺服器（記憶體模式、獨立 port），驗後端權威濫用防護＋前後端協定對齊，
// 非純模擬：
//   ① 沒有工具時請求保養 → 應收 repair_fail「背包裡沒有這件工具」
//   ② qa_grant 一把鐵鎬（全新，未磨損）後請求保養 → 應收 repair_fail「還很堪用，不需要保養」
//      （伺服器真查耐久帳本，不信客戶端自報「壞了」）
//   ③ 傳送到地底、真的對天然石頭/礦脈方塊連續破壞（真實 Break 協定，非模擬耐久數值）——
//      直到磨到跨過門檻那一刻，應收到恰好一次 tool_worn 提示
//   ④ 傳送到最近的居民身邊、請求保養 → 應收 repair_ok（真扣乙太幣、真歸零耐久）
//   ⑤ 立刻再請求保養同一把鎬 → 應收 repair_fail「還很堪用」（耐久已歸零，非「太遠」擋下）
// 需要伺服器以 BUTFUN_QA_DEBUG=1 啟動（qa_grant 才生效；正式線上惰性忽略）。
// 不抄外部碼；全繁中註解；node --check 過。比照 voxel-busking-qa.mjs 同款結構。

import WebSocket from "ws";

const PORT = process.env.VQA_PORT || 49401;
const WS_URL = `ws://127.0.0.1:${PORT}/voxel/ws`;
const PICKAXE_IRON_ID = 34;
const PICKAXE_STONE_ID = 33; // 從未授予過，用來測「背包裡沒有這件工具」
const COIN_ID = 98;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let pass = 0, fail = 0;
function check(label, ok, extra = "") {
  if (ok) { pass++; console.log(`  ✅ ${label}${extra ? "  " + extra : ""}`); }
  else { fail++; console.log(`  ❌ ${label}${extra ? "  " + extra : ""}`); }
}

const ws = new WebSocket(WS_URL);
const send = (o) => ws.send(JSON.stringify(o));
const st = {
  myId: null, myPos: null, residents: [],
  repairOk: [], repairFail: [], toolWorn: [],
};
ws.on("message", (buf) => {
  let m; try { m = JSON.parse(buf.toString()); } catch { return; }
  switch (m.t) {
    case "welcome": st.myId = m.id; break;
    case "repair_ok": st.repairOk.push(m); break;
    case "repair_fail": st.repairFail.push(m); break;
    case "tool_worn": st.toolWorn.push(m); break;
    case "players": {
      const me = (m.players || []).find((p) => p.id === st.myId);
      if (me) st.myPos = { x: me.x, y: me.y, z: me.z };
      if (Array.isArray(m.residents) && m.residents.length > 0) st.residents = m.residents;
      break;
    }
  }
});

// 玩家背包/耐久帳本以顯示名為鍵、跨重啟持久化——用帶時間戳的獨一名字，
// 確保每次跑這支 QA 都是全新身分，不會被前一輪殘留的背包/磨損狀態污染判定。
const qaName = `耐久測試員_${Date.now()}`;
await new Promise((resolve, reject) => {
  ws.on("open", () => { send({ t: "join", name: qaName }); resolve(); });
  ws.on("error", reject);
});
await sleep(400); // 等第一輪 players 快照，拿到自身座標與居民清單

if (!st.myPos) { console.log("❌ 未收到自身座標快照，中止"); process.exit(1); }
if (st.residents.length === 0) { console.log("❌ 未收到居民清單，中止"); process.exit(1); }
// 凍結出生點座標（`st.myPos` 之後會隨每輪 players 廣播被自己的移動覆寫，
// ③ 算挖礦深度要用「出生時的地表高度」，別被①②的傳送污染）。
const spawnPos = { ...st.myPos };

// 觸及範圍已由既有系統（贈禮/交易…）反覆驗證過，非本刀重點——先傳送到居民身邊，
// 讓①②專心測「沒工具」「還很堪用」這兩個工具耐久 v1 才有的濫用防護分支。
const startRes = st.residents[0];
send({ t: "move", x: startRes.x, y: startRes.y, z: startRes.z, yaw: 0 });
await sleep(150);

console.log("① 沒有工具時請求保養（應被伺服器擋下）…");
send({ t: "repair_tool", resident_id: startRes.id, tool_id: PICKAXE_IRON_ID });
await sleep(250);
check("沒有工具 → 收到 repair_fail", st.repairFail.length === 1, JSON.stringify(st.repairFail[0] || {}));
check("沒有工具 → 未收到 repair_ok", st.repairOk.length === 0);

console.log("② QA 授予一把全新鐵鎬後請求保養（還很堪用，不該准）…");
send({ t: "qa_grant", item_id: PICKAXE_IRON_ID, count: 1 });
await sleep(150);
send({ t: "repair_tool", resident_id: startRes.id, tool_id: PICKAXE_IRON_ID });
await sleep(250);
check(
  "全新工具 → 收到 repair_fail（還很堪用）",
  st.repairFail.length === 2 && /堪用/.test(st.repairFail[1].reason || ""),
  JSON.stringify(st.repairFail[1] || {}),
);

console.log("③ 傳送到地底，真的挖天然方塊直到磨鈍（真實 break 協定，非模擬耐久數值）…");
// spawn_pos() 給的初始 y = 地表高度 h + 2；地表以下 h-3..h-1 是泥土，h-4 起才確定進入
// 石頭/礦脈可挖層——把自己傳送到那一層附近（Move 前端權威、伺服器照收，QA 慣用手法），
// 再對周圍一片方塊格挖礦，直到耐久跨過門檻（見 voxel_tool_wear::WORN_THRESHOLD_PCT）。
const groundH = spawnPos.y - 2;
const digY = Math.floor(groundH) - 4;
const baseX = Math.floor(spawnPos.x);
const baseZ = Math.floor(spawnPos.z);
send({ t: "move", x: baseX + 0.5, y: digY, z: baseZ + 0.5, yaw: 0 });
await sleep(150);

let dug = 0;
outer:
for (let depthOff = 0; depthOff < 5 && st.toolWorn.length === 0; depthOff++) {
  for (let dx = -3; dx <= 3 && st.toolWorn.length === 0; dx++) {
    for (let dz = -3; dz <= 3 && st.toolWorn.length === 0; dz++) {
      send({ t: "break", x: baseX + dx, y: digY - depthOff, z: baseZ + dz, tool: PICKAXE_IRON_ID });
      dug++;
      if (dug % 8 === 0) await sleep(120); // 小批次間隔，讓伺服器處理跟上
      if (st.toolWorn.length > 0) break outer;
    }
  }
}
await sleep(300);
check(`真實挖礦 ${dug} 次後跨過磨損門檻 → 收到 tool_worn`, st.toolWorn.length === 1, JSON.stringify(st.toolWorn[0] || {}));

console.log("④ 傳送到居民身邊請求保養（該修了，應該准）…");
const r = st.residents[0];
send({ t: "move", x: r.x, y: r.y, z: r.z, yaw: 0 });
send({ t: "qa_grant", item_id: COIN_ID, count: 20 }); // 保養要付乙太幣，QA 先發夠用的錢
await sleep(150);
const okBefore = st.repairOk.length;
send({ t: "repair_tool", resident_id: r.id, tool_id: PICKAXE_IRON_ID });
await sleep(300);
check("該修了 → 收到 repair_ok", st.repairOk.length === okBefore + 1, JSON.stringify(st.repairOk[okBefore] || {}));

console.log("⑤ 剛修好，立刻再請求保養同一把鎬（應為「還很堪用」，非太遠）…");
const failBefore = st.repairFail.length;
send({ t: "repair_tool", resident_id: r.id, tool_id: PICKAXE_IRON_ID });
await sleep(250);
check(
  "剛修好 → 再保養收到 repair_fail（還很堪用）",
  st.repairFail.length === failBefore + 1 && /堪用/.test(st.repairFail[failBefore]?.reason || ""),
  JSON.stringify(st.repairFail[failBefore] || {}),
);

ws.close();
console.log(`\n結果：${pass} 通過，${fail} 失敗`);
process.exit(fail > 0 ? 1 : 0);
