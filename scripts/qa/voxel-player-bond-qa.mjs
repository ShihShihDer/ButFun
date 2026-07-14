// ============================================================
// voxel-player-bond-qa.mjs — 玩家羈絆帳本 v1（自主提案切片，ROADMAP 985）真 WS 功能 QA
// ============================================================
// 真 WebSocket 直連隔離伺服器（記憶體模式、獨立 port），驗後端權威判定＋真的寫進持久化帳本，
// 非純模擬：
//   ① 甲乙兩位訪客同時加入（`spawn_pos()` 對訪客是確定性函式，天然落在同一出生點、彼此都在
//      並肩協作半徑 8 內，不必額外傳送），甲挖一塊天然方塊、命中並肩協作（827，既有機制）
//      → 應真的在 `data/voxel_player_bonds.jsonl` 多寫一行甲乙配對的 tick。
//   ② 同一輪冷卻內甲再挖一塊（仍命中 coop_bonus）→ 冷卻擋下第二次計入，帳本行數應保持 1
//      （防洗刷：並肩協作本身可以連續命中，但玩家羈絆 tick 不能被同一秒的重複事件洗版）。
//   ③ 甲丙是全新配對（不受甲乙冷卻影響）：甲把手上材料丟在原地，丙走近觸發既有掉落物
//      自動拾取（828）→ 應真的在帳本多寫一行甲丙配對的 tick。
// 直接讀伺服器程序的 `data/voxel_player_bonds.jsonl`（append-only delta，一行一次真實互動）
// 核對持久化真的落地，非只信賴協定回應。用帶時間戳的獨一名字，確保每次跑都是全新配對、
// 不會被前一輪殘留的帳本污染判定。
// 不抄外部碼；全繁中註解；node --check 過。比照 voxel-tool-wear-qa.mjs 同款結構。

import WebSocket from "ws";
import { readFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const PORT = process.env.VQA_PORT || 49911;
const WS_URL = `ws://127.0.0.1:${PORT}/voxel/ws`;
const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const BOND_FILE = resolve(REPO_ROOT, "data/voxel_player_bonds.jsonl");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let pass = 0, fail = 0;
function check(label, ok, extra = "") {
  if (ok) { pass++; console.log(`  ✅ ${label}${extra ? "  " + extra : ""}`); }
  else { fail++; console.log(`  ❌ ${label}${extra ? "  " + extra : ""}`); }
}

function bondTickCount(a, b) {
  if (!existsSync(BOND_FILE)) return 0;
  const lines = readFileSync(BOND_FILE, "utf8").split("\n").filter(Boolean);
  let n = 0;
  for (const line of lines) {
    let e; try { e = JSON.parse(line); } catch { continue; }
    if ((e.id_a === a && e.id_b === b) || (e.id_a === b && e.id_b === a)) n++;
  }
  return n;
}

function connect(name) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(WS_URL);
    const st = { myId: null, myPos: null, coopBonus: [], invUpdate: [] };
    ws.on("message", (buf) => {
      let m; try { m = JSON.parse(buf.toString()); } catch { return; }
      switch (m.t) {
        case "welcome": st.myId = m.id; break;
        case "coop_bonus": st.coopBonus.push(m); break;
        case "inv_update": st.invUpdate.push(m); break;
        case "players": {
          const me = (m.players || []).find((p) => p.id === st.myId);
          if (me) st.myPos = { x: me.x, y: me.y, z: me.z };
          break;
        }
      }
    });
    ws.on("open", () => { ws.send(JSON.stringify({ t: "join", name })); resolve({ ws, st, send: (o) => ws.send(JSON.stringify(o)) }); });
    ws.on("error", reject);
  });
}

const suffix = Date.now();
const nameA = `羈絆測試甲_${suffix}`;
const nameB = `羈絆測試乙_${suffix}`;
const nameC = `羈絆測試丙_${suffix}`;

const A = await connect(nameA);
const B = await connect(nameB);
const C = await connect(nameC);
await sleep(400); // 等第一輪 players 快照，拿到自身座標

if (!A.st.myPos || !B.st.myPos || !C.st.myPos) { console.log("❌ 未收到自身座標快照，中止"); process.exit(1); }

console.log("① 甲乙並肩挖天然方塊，命中並肩協作 → 應真的寫入一筆甲乙配對的玩家羈絆 tick…");
const gx = Math.floor(A.st.myPos.x);
const gy = Math.floor(A.st.myPos.y) - 2; // 出生點 y = 地表高度 + 2，往下兩格是地表（泥土/草）。
const gz = Math.floor(A.st.myPos.z);
let dug = 0;
outer1:
for (let depthOff = 0; depthOff < 4; depthOff++) {
  for (let dx = -2; dx <= 2; dx++) {
    for (let dz = -2; dz <= 2; dz++) {
      A.send({ t: "break", x: gx + dx, y: gy - depthOff, z: gz + dz, tool: 0 });
      dug++;
      await sleep(40);
      if (A.st.coopBonus.length > 0) break outer1;
    }
  }
}
await sleep(300);
check(`並肩協作命中（挖了 ${dug} 次後收到 coop_bonus）`, A.st.coopBonus.length >= 1, JSON.stringify(A.st.coopBonus[0] || {}));
const tick1 = bondTickCount(nameA, nameB);
check("甲乙帳本第一次計入 tick（帳本恰有 1 筆）", tick1 === 1, `實際 ${tick1} 筆`);

console.log("② 冷卻內立刻再挖一塊（仍命中 coop_bonus）→ 甲乙帳本不應多寫（防洗刷）…");
const bonusBefore = A.st.coopBonus.length;
let dug2 = 0;
outer2:
for (let depthOff = 0; depthOff < 4; depthOff++) {
  for (let dx = -2; dx <= 2; dx++) {
    for (let dz = -2; dz <= 2; dz++) {
      A.send({ t: "break", x: gx + dx, y: gy - depthOff - 4, z: gz + dz, tool: 0 });
      dug2++;
      await sleep(40);
      if (A.st.coopBonus.length > bonusBefore) break outer2;
    }
  }
}
await sleep(300);
check(`並肩協作再度命中（挖了 ${dug2} 次，證明協作加成本身仍持續運作）`, A.st.coopBonus.length > bonusBefore);
const tick1b = bondTickCount(nameA, nameB);
check("冷卻內第二次不計入（帳本仍是 1 筆，非 2 筆）", tick1b === 1, `實際 ${tick1b} 筆`);

console.log("③ 甲把材料丟在原地，丙（全新配對，不受①②冷卻影響）走近自動撿起 → 應寫入甲丙 tick…");
const itemId = A.st.invUpdate.at(-1)?.block_id;
if (typeof itemId !== "number") {
  console.log("❌ 未取得可丟下的材料 id，中止"); process.exit(1);
}
const dropX = Math.round(A.st.myPos.x), dropY = Math.round(A.st.myPos.y), dropZ = Math.round(A.st.myPos.z);
A.send({ t: "drop_item", x: dropX, y: dropY, z: dropZ, item_id: itemId, count: 1 });
await sleep(200);
// 丙走近掉落座標（Move handler 裡撿物與反瞬移守衛皆已由 828/既有系統驗證過，非本刀重點）。
C.send({ t: "move", x: dropX + 0.1, y: dropY, z: dropZ + 0.1, yaw: 0 });
await sleep(300);
const tickAC = bondTickCount(nameA, nameC);
check("甲丙帳本計入 tick（全新配對不受甲乙冷卻牽連）", tickAC === 1, `實際 ${tickAC} 筆`);
check("甲乙帳本未被甲丙事件污染（仍是 1 筆）", bondTickCount(nameA, nameB) === 1);

A.ws.close(); B.ws.close(); C.ws.close();
console.log(`\n結果：${pass} 通過，${fail} 失敗`);
process.exit(fail === 0 ? 0 : 1);
