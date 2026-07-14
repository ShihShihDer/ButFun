// ============================================================
// voxel-riding-qa.mjs — 蒸汽獨輪車 v1（ROADMAP 976）真 WS 功能 QA
// ============================================================
// 真 WebSocket 直連隔離伺服器（記憶體模式、獨立 port），驗後端權威濫用防護＋前後端協定對齊：
//   (a) 沒有車時送 set_riding{riding:true} → 應收 riding_fail（伺服器真查背包，不信自報）
//   (b) qa_grant 一輛蒸汽獨輪車 → 再送 set_riding{riding:true} → 應收 riding_ok{riding:true}，
//       且 players 快照裡自己那筆 riding=true（players 陣列含自身，非僅其他玩家）
//   (c) 送 set_riding{riding:false}（下車，永遠放行）→ 應收 riding_ok{riding:false}，
//       players 快照 riding 回到 false
// 需要伺服器以 BUTFUN_QA_DEBUG=1 啟動（qa_grant 才生效；正式線上惰性忽略）。
// 不抄外部碼；全繁中註解；node --check 過。

import WebSocket from "ws";

const PORT = process.env.VQA_PORT || 48922;
const WS_URL = `ws://127.0.0.1:${PORT}/voxel/ws`;
const STEAM_UNICYCLE_ID = 115;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let pass = 0, fail = 0;
function check(label, ok, extra = "") {
  if (ok) { pass++; console.log(`  ✅ ${label}${extra ? "  " + extra : ""}`); }
  else { fail++; console.log(`  ❌ ${label}${extra ? "  " + extra : ""}`); }
}

const ws = new WebSocket(WS_URL);
const send = (o) => ws.send(JSON.stringify(o));
const st = { myId: null, ridingOk: [], ridingFail: [], myRidingSnapshot: null };
ws.on("message", (buf) => {
  let m; try { m = JSON.parse(buf.toString()); } catch { return; }
  switch (m.t) {
    case "welcome": st.myId = m.id; break;
    case "riding_ok": st.ridingOk.push(m); break;
    case "riding_fail": st.ridingFail.push(m); break;
    case "players": {
      const me = (m.players || []).find((p) => p.id === st.myId);
      if (me) st.myRidingSnapshot = !!me.riding;
      break;
    }
  }
});

await new Promise((resolve, reject) => {
  ws.on("open", () => { send({ t: "join", name: "獨輪車測試員" }); resolve(); });
  ws.on("error", reject);
});
await sleep(300);
console.log("① 沒有車時嘗試騎乘（應被伺服器擋下）…");
send({ t: "set_riding", riding: true });
await sleep(250);
check("沒有車 → 收到 riding_fail", st.ridingFail.length === 1, JSON.stringify(st.ridingFail[0] || {}));
check("沒有車 → 未收到 riding_ok", st.ridingOk.length === 0);

console.log("② QA 授予一輛蒸汽獨輪車後再試…");
send({ t: "qa_grant", item_id: STEAM_UNICYCLE_ID, count: 1 });
await sleep(150);
send({ t: "set_riding", riding: true });
await sleep(250);
check("持有車 → 收到 riding_ok{riding:true}", st.ridingOk.length === 1 && st.ridingOk[0].riding === true, JSON.stringify(st.ridingOk[0] || {}));
check("players 快照自身 riding=true", st.myRidingSnapshot === true, `snapshot=${st.myRidingSnapshot}`);

console.log("③ 下車（永遠放行，不需驗證）…");
send({ t: "set_riding", riding: false });
await sleep(250);
check("下車 → 收到 riding_ok{riding:false}", st.ridingOk.length === 2 && st.ridingOk[1].riding === false, JSON.stringify(st.ridingOk[1] || {}));
check("players 快照自身 riding=false", st.myRidingSnapshot === false, `snapshot=${st.myRidingSnapshot}`);

console.log(`\n結果：${pass} 通過，${fail} 失敗`);
ws.close();
process.exit(fail > 0 ? 1 : 0);
