// ============================================================
// voxel-busking-qa.mjs — 街頭手風琴 v1（ROADMAP 977）真 WS 功能 QA
// ============================================================
// 真 WebSocket 直連隔離伺服器（記憶體模式、獨立 port），驗後端權威濫用防護＋前後端協定對齊：
//   (a) 沒有手風琴時送 set_performing{performing:true} → 應收 performing_fail（伺服器真查
//       背包，不信自報）
//   (b) qa_grant 一把街頭手風琴 → 再送 set_performing{performing:true} → 應收
//       performing_ok{performing:true}，且 players 快照裡自己那筆 performing=true
//   (c) 送 set_performing{performing:false}（收起，永遠放行）→ 應收
//       performing_ok{performing:false}，players 快照 performing 回到 false
// 需要伺服器以 BUTFUN_QA_DEBUG=1 啟動（qa_grant 才生效；正式線上惰性忽略）。
// 不抄外部碼；全繁中註解；node --check 過。比照 voxel-riding-qa.mjs 同款結構。

import WebSocket from "ws";

const PORT = process.env.VQA_PORT || 48923;
const WS_URL = `ws://127.0.0.1:${PORT}/voxel/ws`;
const STREET_ACCORDION_ID = 116;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let pass = 0, fail = 0;
function check(label, ok, extra = "") {
  if (ok) { pass++; console.log(`  ✅ ${label}${extra ? "  " + extra : ""}`); }
  else { fail++; console.log(`  ❌ ${label}${extra ? "  " + extra : ""}`); }
}

const ws = new WebSocket(WS_URL);
const send = (o) => ws.send(JSON.stringify(o));
const st = { myId: null, performOk: [], performFail: [], myPerformSnapshot: null };
ws.on("message", (buf) => {
  let m; try { m = JSON.parse(buf.toString()); } catch { return; }
  switch (m.t) {
    case "welcome": st.myId = m.id; break;
    case "performing_ok": st.performOk.push(m); break;
    case "performing_fail": st.performFail.push(m); break;
    case "players": {
      const me = (m.players || []).find((p) => p.id === st.myId);
      if (me) st.myPerformSnapshot = !!me.performing;
      break;
    }
  }
});

await new Promise((resolve, reject) => {
  ws.on("open", () => { send({ t: "join", name: "手風琴測試員" }); resolve(); });
  ws.on("error", reject);
});
await sleep(300);
console.log("① 沒有手風琴時嘗試開演（應被伺服器擋下）…");
send({ t: "set_performing", performing: true });
await sleep(250);
check("沒有手風琴 → 收到 performing_fail", st.performFail.length === 1, JSON.stringify(st.performFail[0] || {}));
check("沒有手風琴 → 未收到 performing_ok", st.performOk.length === 0);

console.log("② QA 授予一把街頭手風琴後再試…");
send({ t: "qa_grant", item_id: STREET_ACCORDION_ID, count: 1 });
await sleep(150);
send({ t: "set_performing", performing: true });
await sleep(250);
check("持有手風琴 → 收到 performing_ok{performing:true}", st.performOk.length === 1 && st.performOk[0].performing === true, JSON.stringify(st.performOk[0] || {}));
check("players 快照自身 performing=true", st.myPerformSnapshot === true, `snapshot=${st.myPerformSnapshot}`);

console.log("③ 收起演奏（永遠放行，不需驗證）…");
send({ t: "set_performing", performing: false });
await sleep(250);
check("收起 → 收到 performing_ok{performing:false}", st.performOk.length === 2 && st.performOk[1].performing === false, JSON.stringify(st.performOk[1] || {}));
check("players 快照自身 performing=false", st.myPerformSnapshot === false, `snapshot=${st.myPerformSnapshot}`);

console.log(`\n結果：${pass} 通過，${fail} 失敗`);
ws.close();
process.exit(fail > 0 ? 1 : 0);
