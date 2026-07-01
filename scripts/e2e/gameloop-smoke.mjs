// 遊戲迴圈冒煙測試：連 /voxel/ws → 斷言快照含自己 id → 斷言 time_of_day 有推進。
// 專門偵測「HTTP /healthz 還活著、但遊戲迴圈 tokio task 已靜默炸死」這一型事故。
//
// 用法：node gameloop-smoke.mjs <ws-url>
//       node gameloop-smoke.mjs ws://localhost:3000/voxel/ws
//
// 2D 已封存、/ws 路由已移除（PR #874）——這支腳本只測現行的 voxel 協定
// （{"t":"join"} / {"t":"welcome","id":...} / {"t":"players","players":[...],"time_of_day":...}，
// 見 src/voxel_ws.rs）。voxel 沒有整數 tick 欄位，改用 time_of_day 單調前進當「迴圈還活著」的證據
// （日夜循環極長，12 秒窗口內只有極罕見的跨日翻轉邊界會誤判——可接受）。
//
// 成功 exit 0；失敗 exit 1（可接進 deploy.sh 回滾判斷）。

import { WebSocket } from "ws";

const URL = process.argv[2] || "ws://localhost:3000/voxel/ws";
const TIMEOUT_MS = 12_000;

function run() {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(URL);
    const timer = setTimeout(() => {
      ws.close();
      reject(new Error(`超時（${TIMEOUT_MS}ms）：未能收到足夠快照`));
    }, TIMEOUT_MS);

    let myId = null;
    let firstTod = null;
    let done = false;

    ws.on("open", () => {
      ws.send(JSON.stringify({ t: "join", name: "煙霧哨兵" }));
    });

    ws.on("message", (raw) => {
      if (done) return;
      let msg;
      try { msg = JSON.parse(raw.toString()); } catch { return; }

      if (msg.t === "welcome") {
        myId = msg.id;
        return;
      }

      if (msg.t === "players" && myId) {
        const hasSelf = Array.isArray(msg.players) && msg.players.some((p) => p.id === myId);
        const tod = typeof msg.time_of_day === "number" ? msg.time_of_day : null;

        if (firstTod === null) {
          // 第一幀：確認自己出現在快照裡
          if (!hasSelf) {
            clearTimeout(timer);
            ws.close();
            reject(new Error(`第一幀快照找不到自己的 id（${myId}）`));
            return;
          }
          if (tod === null) {
            clearTimeout(timer);
            ws.close();
            reject(new Error("快照缺少 time_of_day 欄位，無法驗證遊戲迴圈"));
            return;
          }
          firstTod = tod;
          return;
        }

        // 第二幀：確認 time_of_day 有推進（遊戲迴圈還活著）。跨日翻轉會變小，容忍差值極小
        // 的情況也算通過（連線內短短幾秒仍收到新的一幀，代表廣播迴圈沒死）。
        if (tod !== null && tod !== firstTod) {
          done = true;
          clearTimeout(timer);
          ws.close();
          resolve({ firstTod, secondTod: tod });
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
    `遊戲迴圈推進（time_of_day ${result.firstTod} → ${result.secondTod}）`,
    result.secondTod !== result.firstTod,
  );
} catch (err) {
  check("遊戲迴圈活著", false, err.message);
}

console.log(`\n結果：${passed} 通過，${failed} 失敗`);
process.exit(failed === 0 ? 0 : 1);
