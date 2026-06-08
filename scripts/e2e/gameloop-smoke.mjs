// 遊戲迴圈冒煙測試：連 WS → 斷言快照含自己 id → 斷言兩幀 tick 有推進。
// 專門偵測「HTTP /healthz 還活著、但遊戲迴圈 tokio task 已靜默炸死」這一型事故。
//
// 用法：node gameloop-smoke.mjs <ws-url>
//       node gameloop-smoke.mjs ws://localhost:3000/ws
//
// 成功 exit 0；失敗 exit 1（可接進 deploy.sh 回滾判斷）。

import { WebSocket } from "ws";

const URL = process.argv[2] || "ws://localhost:3000/ws";
const TIMEOUT_MS = 12_000;

function run() {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(URL);
    const timer = setTimeout(() => {
      ws.close();
      reject(new Error(`超時（${TIMEOUT_MS}ms）：未能收到足夠快照`));
    }, TIMEOUT_MS);

    let myId = null;
    let firstTick = null;
    let done = false;

    ws.on("open", () => {
      ws.send(JSON.stringify({ type: "join", name: "煙霧哨兵", species: "terran" }));
    });

    ws.on("message", (raw) => {
      if (done) return;
      let msg;
      try { msg = JSON.parse(raw.toString()); } catch { return; }

      if (msg.type === "welcome") {
        myId = msg.id;
        return;
      }

      if (msg.type === "snapshot" && myId) {
        const hasSelf = msg.players && msg.players.some((p) => p.id === myId);
        const tick = typeof msg.tick === "number" ? msg.tick : null;

        if (firstTick === null) {
          // 第一幀：確認自己出現在快照裡
          if (!hasSelf) {
            clearTimeout(timer);
            ws.close();
            reject(new Error(`第一幀快照找不到自己的 id（${myId}）`));
            return;
          }
          if (tick === null) {
            clearTimeout(timer);
            ws.close();
            reject(new Error("快照缺少 tick 欄位，無法驗證遊戲迴圈"));
            return;
          }
          firstTick = tick;
          return;
        }

        // 第二幀：確認 tick 有推進（遊戲迴圈還活著）
        if (tick !== null && tick > firstTick) {
          done = true;
          clearTimeout(timer);
          ws.close();
          resolve({ firstTick, secondTick: tick });
        }
      }
    });

    ws.on("error", (e) => {
      clearTimeout(timer);
      reject(e);
    });
  });
}

let passed = 0;
let failed = 0;
function check(label, ok, detail = "") {
  if (ok) { console.log(`✅ ${label}`); passed++; }
  else     { console.error(`❌ ${label}${detail ? " — " + detail : ""}`); failed++; }
}

console.log(`[gameloop-smoke] 連線 ${URL}`);

try {
  const result = await run();
  check("收到含自身 id 的快照", true);
  check(
    `遊戲迴圈推進（tick ${result.firstTick} → ${result.secondTick}）`,
    result.secondTick > result.firstTick,
  );
} catch (err) {
  check("遊戲迴圈活著", false, err.message);
}

console.log(`\n結果：${passed} 通過，${failed} 失敗`);
process.exit(failed === 0 ? 0 : 1);
