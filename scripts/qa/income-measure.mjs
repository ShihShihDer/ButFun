// 乙太收入量測 bot（ROADMAP 40：數據驅動定價的量測工具）。
//
// 功能：連接遊戲伺服器，以訪客身份挖掘礦區 5 分鐘，統計乙太/分鐘收入。
// 用法：node scripts/qa/income-measure.mjs [ws-url] [duration-secs]
//
// 輸出範例：
//   [量測] 開始：挖礦 300 秒
//   [量測] 已挖 60s | 乙太 +45 | 45.0 乙太/分
//   [量測] 已挖 120s | 乙太 +92 | 46.0 乙太/分
//   [量測] 結束 | 總乙太 +228 | 平均 45.6 乙太/分
//   [量測] 每分鐘礦掉 ether 量：35.2，NPC 賣出：10.4

import { WebSocket } from "ws";
const URL = process.argv[2] || "ws://localhost:3000/ws";
const DURATION_SECS = parseInt(process.argv[3] || "300", 10);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// 礦區巡邏路徑（故鄉岩地，確定性礦石分布區附近）
const MINE_PATROL = [
  [2800, 2200], [2900, 2200], [3000, 2200],
  [3000, 2400], [2900, 2400], [2800, 2400],
];

let myId = null;
let S = { x: 0, y: 0, ether: 0, hp: 0, ready: false, terrain: new Map() };
let sendFn = null;
let startEther = 0;
let etherFromDigs = 0;
let etherFromNpc = 0;
let digCount = 0;

function connect() {
  return new Promise((res, rej) => {
    const ws = new WebSocket(URL);
    const t = setTimeout(() => rej(new Error("連線超時")), 12000);

    ws.on("open", () => {
      ws.send(JSON.stringify({ type: "join", name: "收入量測bot", species: "terran" }));
    });

    ws.on("message", (raw) => {
      let m;
      try { m = JSON.parse(raw); } catch { return; }

      if (m.type === "welcome") {
        myId = m.id;
        return;
      }

      if (m.type === "snapshot" && myId) {
        // 更新地形差異 map
        if (Array.isArray(m.terrain)) {
          for (const d of m.terrain) {
            S.terrain.set(`${d.cx},${d.cy},${d.tx},${d.ty}`, d.kind);
          }
        }
        const me = (m.players || []).find((p) => p.id === myId);
        if (me) {
          const prevEther = S.ether;
          S.x = me.x; S.y = me.y; S.ether = me.ether; S.hp = me.hp;
          if (!S.ready) {
            S.ready = true;
            startEther = me.ether;
            sendFn = (o) => ws.send(JSON.stringify(o));
            clearTimeout(t);
            res(ws);
          }
        }
      }
    });

    ws.on("error", rej);
  });
}

// 移動到目標座標（每 100ms 送一次移動，最多等 maxMs）
async function moveTo(tx, ty, maxMs = 5000) {
  const t0 = Date.now();
  while (Date.now() - t0 < maxMs) {
    const dx = tx - S.x, dy = ty - S.y;
    if (dx * dx + dy * dy < 32 * 32) return true;
    sendFn({ type: "move", dx: Math.sign(dx), dy: Math.sign(dy) });
    await sleep(100);
  }
  return false;
}

// 向某格發送挖掘請求
function digAt(wx, wy) {
  sendFn({ type: "dig", wx, wy });
  digCount++;
}

// 圍繞巡邏點挖掘（嘗試附近的格子）
async function mineAround(cx, cy) {
  const CELL = 32;
  for (let dy = -2; dy <= 2; dy++) {
    for (let dx = -2; dx <= 2; dx++) {
      const wx = cx + dx * CELL;
      const wy = cy + dy * CELL;
      digAt(wx, wy);
      await sleep(50);
    }
  }
}

async function main() {
  console.log(`[量測] 連線到 ${URL}`);
  const ws = await connect();
  console.log(`[量測] 開始：挖礦 ${DURATION_SECS} 秒 | 起始乙太 ${startEther}`);

  const t0 = Date.now();
  const REPORT_INTERVAL_MS = 60_000;
  let lastReport = t0;
  let patrolIdx = 0;

  while (Date.now() - t0 < DURATION_SECS * 1000) {
    // 移動到下一個巡邏點
    const [px, py] = MINE_PATROL[patrolIdx % MINE_PATROL.length];
    await moveTo(px, py, 3000);

    // 在附近挖掘
    await mineAround(S.x, S.y);
    patrolIdx++;

    // 定期報告
    const now = Date.now();
    if (now - lastReport >= REPORT_INTERVAL_MS) {
      const elapsed = (now - t0) / 1000;
      const gained = S.ether - startEther;
      const rate = (gained / elapsed * 60).toFixed(1);
      console.log(`[量測] 已挖 ${Math.round(elapsed)}s | 乙太 +${gained} | ${rate} 乙太/分 | 挖掘次數 ${digCount}`);
      lastReport = now;
    }

    await sleep(200);
  }

  const totalSecs = (Date.now() - t0) / 1000;
  const totalGained = S.ether - startEther;
  const avgRate = (totalGained / totalSecs * 60).toFixed(1);

  console.log(`\n[量測] ========== 結果 ==========`);
  console.log(`[量測] 時長：${Math.round(totalSecs)} 秒`);
  console.log(`[量測] 總乙太收入：+${totalGained}`);
  console.log(`[量測] 平均收入：${avgRate} 乙太/分鐘`);
  console.log(`[量測] 挖掘次數：${digCount}`);
  console.log(`[量測] ==============================`);
  console.log(`[量測] 定價參考：`);
  console.log(`[量測]   現況 ${avgRate} 乙太/分 → 攢 300 乙太（翠幽星直購）需 ${(300 / parseFloat(avgRate)).toFixed(1)} 分鐘`);
  console.log(`[量測]   購地 20 乙太 → ${(20 / parseFloat(avgRate)).toFixed(1)} 分鐘`);

  ws.close();
}

main().catch((e) => { console.error("[量測]", e.message); process.exit(1); });
