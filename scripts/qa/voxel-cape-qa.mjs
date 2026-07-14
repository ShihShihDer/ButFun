// ============================================================
// voxel-cape-qa.mjs — 染坊·彩色披風 v1 真 WS 功能 QA
// ============================================================
// 真 WebSocket 直連隔離伺服器（記憶體模式、獨立 port），驗後端權威濫用防護＋前後端協定對齊：
//   (a) 沒選中披風、也沒有披風時送 set_cape{wearing:true} → 應收 cape_fail（沒選中）
//   (b) qa_grant 一件紅披風，但熱鍵格仍未選中它 → 再送 set_cape{wearing:true} → 仍應收
//       cape_fail（伺服器讀「目前熱鍵選中的物品」而非「背包裡有沒有」，兩者都要真才准）
//   (c) 送 move 帶 held=CAPE_RED_ID（模擬前端熱鍵選中紅披風）→ 再送 set_cape{wearing:true}
//       → 應收 cape_ok{cape:117}，且 players 快照裡自己那筆 cape=117
//   (d) 送 set_cape{wearing:false}（脫下，永遠放行）→ 應收 cape_ok{cape:null}，
//       players 快照 cape 欄位消失（additive 欄位，未穿=不序列化）
// 需要伺服器以 BUTFUN_QA_DEBUG=1 啟動（qa_grant 才生效；正式線上惰性忽略）。
// 不抄外部碼；全繁中註解；node --check 過。比照 voxel-riding-qa.mjs/voxel-busking-qa.mjs 同款結構。

import WebSocket from "ws";

const PORT = process.env.VQA_PORT || 49411;
const WS_URL = `ws://127.0.0.1:${PORT}/voxel/ws`;
const CAPE_RED_ID = 117;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let pass = 0, fail = 0;
function check(label, ok, extra = "") {
  if (ok) { pass++; console.log(`  ✅ ${label}${extra ? "  " + extra : ""}`); }
  else { fail++; console.log(`  ❌ ${label}${extra ? "  " + extra : ""}`); }
}

const ws = new WebSocket(WS_URL);
const send = (o) => ws.send(JSON.stringify(o));
const st = { myId: null, capeOk: [], capeFail: [], myCapeSnapshot: "unset", lastSnapshot: null };
ws.on("message", (buf) => {
  let m; try { m = JSON.parse(buf.toString()); } catch { return; }
  switch (m.t) {
    case "welcome": st.myId = m.id; break;
    case "cape_ok": st.capeOk.push(m); break;
    case "cape_fail": st.capeFail.push(m); break;
    case "players": {
      const me = (m.players || []).find((p) => p.id === st.myId);
      if (me) { st.myCapeSnapshot = ("cape" in me) ? me.cape : null; st.lastSnapshot = me; }
      break;
    }
  }
});

await new Promise((resolve, reject) => {
  ws.on("open", () => { send({ t: "join", name: "染坊測試員" }); resolve(); });
  ws.on("error", reject);
});
await sleep(300);

console.log("① 沒選中披風、也沒有披風時嘗試穿上（應被伺服器擋下）…");
send({ t: "set_cape", wearing: true });
await sleep(250);
check("沒選中披風 → 收到 cape_fail", st.capeFail.length === 1, JSON.stringify(st.capeFail[0] || {}));
check("沒選中披風 → 未收到 cape_ok", st.capeOk.length === 0);

console.log("② QA 授予一件紅披風，但熱鍵格仍未選中它，再試一次…");
send({ t: "qa_grant", item_id: CAPE_RED_ID, count: 1 });
await sleep(150);
send({ t: "set_cape", wearing: true });
await sleep(250);
check("真持有但未熱鍵選中 → 仍收到 cape_fail（伺服器認熱鍵選中，不是背包）", st.capeFail.length === 2, JSON.stringify(st.capeFail[1] || {}));
check("仍未收到 cape_ok", st.capeOk.length === 0);

console.log("③ 熱鍵格選中紅披風（隨 move 自報 held）後再試…");
send({ t: "move", x: 0, y: 64, z: 0, yaw: 0, held: CAPE_RED_ID });
await sleep(150);
send({ t: "set_cape", wearing: true });
await sleep(250);
check("熱鍵選中且真持有 → 收到 cape_ok{cape:117}", st.capeOk.length === 1 && st.capeOk[0].cape === CAPE_RED_ID, JSON.stringify(st.capeOk[0] || {}));
check("players 快照自身 cape=117", st.myCapeSnapshot === CAPE_RED_ID, `snapshot=${st.myCapeSnapshot}`);

console.log("④ 脫下披風（永遠放行，不需驗證）…");
send({ t: "set_cape", wearing: false });
await sleep(250);
check("脫下 → 收到 cape_ok{cape:null}", st.capeOk.length === 2 && st.capeOk[1].cape === null, JSON.stringify(st.capeOk[1] || {}));
check("players 快照自身 cape 欄位消失（additive，未穿不序列化）", st.myCapeSnapshot === null || st.myCapeSnapshot === undefined, `snapshot=${st.myCapeSnapshot}`);

console.log(`\n結果：${pass} 通過，${fail} 失敗`);
ws.close();
process.exit(fail > 0 ? 1 : 0);
