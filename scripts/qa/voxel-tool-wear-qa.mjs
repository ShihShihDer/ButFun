// ============================================================
// voxel-tool-wear-qa.mjs — 工具耐久＋鐵匠代客保養 v1（自主提案切片，ROADMAP 981/982）真 WS 功能 QA
// ============================================================
// 真 WebSocket 直連隔離伺服器（記憶體模式、獨立 port），驗後端權威濫用防護＋前後端協定對齊，
// 非純模擬：
//   ① 沒有工具時請求保養 → 應收 repair_fail「背包裡沒有這件工具」
//   ② qa_grant 一把鐵鎬（全新，未磨損）後請求保養 → 應收 repair_fail「還很堪用，不需要保養」
//      （伺服器真查耐久帳本，不信客戶端自報「壞了」）
//   ③ 傳送到地底、真的對天然石頭/礦脈方塊連續破壞（真實 Break 協定，非模擬耐久數值）——
//      直到磨到跨過門檻那一刻，應收到恰好一次 tool_worn 提示；期間應持續收到 tool_durability
//      即時耐久（982 v1.1 新增：耐久第一次看得見），且跨門檻那一刻的 pct ≤ 25%
//   ③.5（982 v1.1 新增）：重新連線同一身分（模擬玩家重登）→ 應收到 tool_wear_sync，內含這把
//      仍在磨損中的鐵鎬與正確的剩餘耐久百分比（登入時還原，不必再揮一次工具才看得到）
//   ④ 傳送到最近的居民身邊、請求保養 → 應收 repair_ok（真扣乙太幣、真歸零耐久、pct=100）
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

const st = {
  myId: null, myPos: null, residents: [],
  repairOk: [], repairFail: [], toolWorn: [], toolDurability: [], toolWearSync: [],
};
function makeHandler() {
  return (buf) => {
    let m; try { m = JSON.parse(buf.toString()); } catch { return; }
    switch (m.t) {
      case "welcome": st.myId = m.id; break;
      case "repair_ok": st.repairOk.push(m); break;
      case "repair_fail": st.repairFail.push(m); break;
      case "tool_worn": st.toolWorn.push(m); break;
      case "tool_durability": st.toolDurability.push(m); break;
      case "tool_wear_sync": st.toolWearSync.push(m); break;
      case "players": {
        const me = (m.players || []).find((p) => p.id === st.myId);
        if (me) st.myPos = { x: me.x, y: me.y, z: me.z };
        if (Array.isArray(m.residents) && m.residents.length > 0) st.residents = m.residents;
        break;
      }
    }
  };
}

// 玩家背包/耐久帳本以顯示名為鍵、跨重啟持久化——用帶時間戳的獨一名字，
// 確保每次跑這支 QA 都是全新身分，不會被前一輪殘留的背包/磨損狀態污染判定。
const qaName = `耐久測試員_${Date.now()}`;
let ws = new WebSocket(WS_URL);
let send = (o) => ws.send(JSON.stringify(o));
ws.on("message", makeHandler());
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
// 鐵階磨損最慢（WEAR_PER_USE_IRON=1，982 v1.1 材質分級新語意）——見底門檻要真挖穿約 75 顆
// 才會磨鈍，比舊石階節奏（每次磨 2、需 38 顆）多將近一倍。**兩個座標限制須同時守住**：
// ①`can_break` 的 `REACH`（6 格＋1 餘裕）——單一挖掘點離目前站位太遠會被伺服器悄悄拒絕
// （曾誤以為擴大搜尋半徑能治好terrain局部稀疏，結果超出 REACH 全數落空）；②Move 的反瞬移
// 上限 `MAX_MOVE_STEP`（64 格／單則）——換點不能跳太遠。做法：分多個「站點」，每站只在
// REACH 內挖一小片（比照 v1 原始範圍），站點之間用安全距離的 Move 換位，站點夠多＋分散，
// 就算某一站剛好是前幾輪 QA 挖空的舊坑，換下一站也能找到新鮮方塊。
const groundH = spawnPos.y - 2;
const digY = Math.floor(groundH) - 4;
const spawnX = Math.floor(spawnPos.x);
const spawnZ = Math.floor(spawnPos.z);
// 站點間距 30 格（遠小於 MAX_MOVE_STEP=64，近到單次 Move 一定合法），沿螺旋擴散，
// 用時間戳選一個起始方向，降低與前幾輪重跑時剛好落在同一批舊坑的機率。
const SITE_OFFSETS = [[0, 0], [30, 0], [-30, 0], [0, 30], [0, -30], [30, 30], [-30, -30], [30, -30]];
const startDir = Date.now() % SITE_OFFSETS.length;

let dug = 0;
outer:
for (let s = 0; s < SITE_OFFSETS.length && st.toolWorn.length === 0; s++) {
  const [ox, oz] = SITE_OFFSETS[(s + startDir) % SITE_OFFSETS.length];
  const baseX = spawnX + ox;
  const baseZ = spawnZ + oz;
  send({ t: "move", x: baseX + 0.5, y: digY, z: baseZ + 0.5, yaw: 0 });
  await sleep(150);
  for (let depthOff = 0; depthOff < 5 && st.toolWorn.length === 0; depthOff++) {
    for (let dx = -3; dx <= 3 && st.toolWorn.length === 0; dx++) {
      for (let dz = -3; dz <= 3 && st.toolWorn.length === 0; dz++) {
        send({ t: "break", x: baseX + dx, y: digY - depthOff, z: baseZ + dz, tool: PICKAXE_IRON_ID });
        dug++;
        if (dug % 15 === 0) await sleep(120); // 小批次間隔，讓伺服器處理跟上
        if (st.toolWorn.length > 0) break outer;
      }
    }
  }
}
await sleep(300);
check(`真實挖礦 ${dug} 次後跨過磨損門檻 → 收到 tool_worn`, st.toolWorn.length === 1, JSON.stringify(st.toolWorn[0] || {}));
// 982 v1.1：耐久第一次看得見——挖礦期間應持續收到即時 tool_durability，跨門檻那一刻的
// pct 應 ≤ 25%（WORN_THRESHOLD_PCT），且沿路非遞增（只會磨損、不會無中生有變耐用）。
check("挖礦期間收到過即時 tool_durability", st.toolDurability.length > 0, `共 ${st.toolDurability.length} 則`);
const lastPct = st.toolDurability.at(-1)?.pct;
check("跨門檻那一刻剩餘耐久 ≤ 25%", typeof lastPct === "number" && lastPct <= 25, `pct=${lastPct}`);
const pcts = st.toolDurability.map((m) => m.pct);
check("耐久沿路非遞增（只會磨損）", pcts.every((p, i) => i === 0 || p <= pcts[i - 1]), JSON.stringify(pcts));

console.log("③.5 同身分開第二條連線（模擬玩家重登）驗證 tool_wear_sync 還原磨損中的工具…");
// 注意：**先開新連線、後關舊連線**，不能反過來——訪客（無帳號）背包/耐久快取是
// session-scoped，`cleanup_guest_by_name` 會在「此名已無任何在線連線在用」時主動清空
// （M5 訪客斷線清理，擋訪客反覆連新名塞爆 map；帳號玩家的資料則一律保留，不受影響）。
// 若先關舊連線，狀態會被合法清空，之後 tool_wear_sync 收到空清單是正確行為、不是 bug；
// 這裡測的是「重登當下狀態還在」這件事本身，所以刻意保持舊連線活著到拿到 sync 之後再關。
const wornPctBeforeReconnect = lastPct;
const ws2 = new WebSocket(WS_URL);
const send2 = (o) => ws2.send(JSON.stringify(o));
ws2.on("message", makeHandler());
await new Promise((resolve, reject) => {
  ws2.on("open", () => { send2({ t: "join", name: qaName }); resolve(); });
  ws2.on("error", reject);
});
await sleep(400);
const syncedIron = (st.toolWearSync.at(-1)?.items || []).find((it) => it.tool_id === PICKAXE_IRON_ID);
check(
  "重登收到 tool_wear_sync、鐵鎬耐久與離線前一致",
  st.toolWearSync.length > 0 && !!syncedIron && syncedIron.pct === wornPctBeforeReconnect,
  JSON.stringify({ synced: syncedIron, expect: wornPctBeforeReconnect }),
);
// 驗證完成，換手：關掉舊連線，剩下這條（同一身分）繼續跑後續步驟。
ws.close();
ws = ws2;
send = send2;
await sleep(150);

console.log("④ 傳送到居民身邊請求保養（該修了，應該准）…");
const r = st.residents[0];
send({ t: "move", x: r.x, y: r.y, z: r.z, yaw: 0 });
send({ t: "qa_grant", item_id: COIN_ID, count: 20 }); // 保養要付乙太幣，QA 先發夠用的錢
await sleep(150);
const okBefore = st.repairOk.length;
send({ t: "repair_tool", resident_id: r.id, tool_id: PICKAXE_IRON_ID });
await sleep(300);
check("該修了 → 收到 repair_ok", st.repairOk.length === okBefore + 1, JSON.stringify(st.repairOk[okBefore] || {}));
check("保養完成 → repair_ok 附帶 pct=100", st.repairOk.at(-1)?.pct === 100, JSON.stringify(st.repairOk.at(-1) || {}));

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
