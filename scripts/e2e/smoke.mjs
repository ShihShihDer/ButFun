// E2E 煙霧測試:模擬玩家進場、移動、聊天、(訪客流程下)驗 WebSocket 核心邏輯。
// 訪客 id 隨機,目前 PositionStore 只記已登入玩家;但仍能驗:
//   1. 進場 → welcome → snapshot 出現自己
//   2. 移動意圖 → 位置真的改變
//   3. 同一 session 移動後再收快照 → 看得到新位置
//   4. 兩個訪客 → 互相看得到對方
// (有 staging 用的 Google OAuth 後可再加「已登入跨重連位置保留」測試。)
//
// 用法:node smoke.mjs <ws-url>
//     node smoke.mjs ws://localhost:3001/ws

import { WebSocket } from "ws";

const URL = process.argv[2] || "ws://localhost:3001/ws";

const wait = (ms) => new Promise((r) => setTimeout(r, ms));

function newGuest(name, species = "terran") {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(URL);
    let me = null;
    let last = null;
    const players = new Map();
    const chat = [];
    const t = setTimeout(() => reject(new Error(`${name} 超時`)), 8000);

    ws.on("open", () => ws.send(JSON.stringify({ type: "join", name, species })));
    ws.on("message", (d) => {
      const m = JSON.parse(d.toString());
      if (m.type === "welcome") {
        me = m.id;
      } else if (m.type === "snapshot") {
        for (const p of m.players) players.set(p.id, p);
        if (me) {
          const self = m.players.find((x) => x.id === me);
          if (self) last = { x: self.x, y: self.y, ether: self.ether ?? 0 };
        }
      } else if (m.type === "chat") {
        chat.push(m);
      }
    });
    ws.on("error", (e) => reject(e));

    setTimeout(() => {
      if (!me) return reject(new Error(`${name} 沒收到 welcome`));
      clearTimeout(t);
      resolve({
        ws, name,
        get id() { return me; },
        get pos() { return last; },
        get players() { return players; },
        get chat() { return chat; },
        sendInput: (k) => ws.send(JSON.stringify({ type: "input", ...k })),
        sendChat: (text) => ws.send(JSON.stringify({ type: "chat", text })),
        close: () => ws.close(),
      });
    }, 1200);
  });
}

let passed = 0;
let failed = 0;
function check(label, cond, detail = "") {
  if (cond) {
    console.log(`✅ ${label}`);
    passed++;
  } else {
    console.log(`❌ ${label} ${detail}`);
    failed++;
  }
}

console.log(`E2E smoke against ${URL}`);

// --- 測試 1:訪客單人進場 + 移動 ---
const alice = await newGuest("艾達");
check("訪客拿到 welcome 與 id", !!alice.id);
check("快照看得到自己", !!alice.pos, `pos=${JSON.stringify(alice.pos)}`);

const startX = alice.pos.x;
alice.sendInput({ right: true });
await wait(1500);
alice.sendInput({});
await wait(400);
check(
  "移動意圖實際讓位置往右改變",
  alice.pos.x > startX + 100,
  `start=${startX.toFixed(0)} end=${alice.pos.x.toFixed(0)}`
);

// --- 測試 2:第二個訪客進場,雙方互看得到 ---
const bob = await newGuest("博特");
await wait(800);
check("Bob 快照看得到 Alice", bob.players.has(alice.id));
check("Alice 快照看得到 Bob", alice.players.has(bob.id));

// --- 測試 3:聊天能跨玩家送達 ---
bob.sendChat("哈囉艾達");
await wait(500);
check(
  "Alice 收得到 Bob 的聊天",
  alice.chat.some((c) => c.text === "哈囉艾達"),
  `chat=${JSON.stringify(alice.chat.map(c => c.text))}`
);

alice.close();
bob.close();

console.log(`\nresults: ${passed} passed, ${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
