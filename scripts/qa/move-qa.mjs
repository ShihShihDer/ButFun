// move-qa.mjs — 無頭實玩 QA：真的連上遊戲 WebSocket 當一個玩家走路，量化移動／預測／卡頓。
//
// 目的：在「看不到 3D 畫面」的情況下，用量化數據診斷移動「飄／卡頓」到底是
//   ① 伺服器端速度／快照本身的問題，還是 ② 客戶端預測對帳（#799）的問題。
//
// 做兩件事：
//   1) 伺服器端真相：以玩家身分加入 → 持續送同一方向的 input → 收 snapshot →
//      記錄「自己」每筆權威座標 (x,y) 與到達時間 → 算出伺服器移動速度(px/s)、
//      快照間隔(ms)＋抖動、座標是否平順遞增。
//   2) 客戶端預測模擬：載入 web/wasm/world_core.wasm（比照 web/3d/main.js 用
//      step_player / step_out_x / step_out_y / run_mult），用同一串輸入以 60fps
//      跑客戶端預測，並套用現行 #799 對帳邏輯（每快照把 predBase 往權威拉
//      RECONCILE_CALIB=0.5、死區 24px、停下收斂）。記錄每幀預測座標、預測誤差、
//      前後幀位移（看快照邊界有沒有猛跳＝lurch＝卡頓）。
//
// 這是「量測工具」，不改遊戲本體。協議格式鏡像 web/3d/main.js 與 web/game.js：
//   · 加入：onopen 送 {type:"join", name, species}
//   · welcome 給 msg.id 當 myId、msg.world{width,height}
//   · 走路：送 {type:"input", up,down,left,right,run}（只在意圖改變時送）
//   · snapshot.players.find(p => p.id === myId) 就是「自己」的權威座標(世界 px)
//
// 用法：
//   node scripts/qa/move-qa.mjs [url] [seconds] [direction] [--run]
//   預設：url=wss://peregrine.but-fun.com/ws  seconds=8  direction=right
//   例：node scripts/qa/move-qa.mjs ws://localhost:3000/ws 6 right

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, "..", "..");

// ── 參數 ──
const argv = process.argv.slice(2);
const URL_DEFAULT = "wss://peregrine.but-fun.com/ws";
const url = argv.find((a) => a.startsWith("ws://") || a.startsWith("wss://")) || URL_DEFAULT;
const seconds = Number(argv.find((a) => /^\d+(\.\d+)?$/.test(a))) || 8;
const dirArg = argv.find((a) => ["up", "down", "left", "right"].includes(a)) || "right";
const RUN = argv.includes("--run");
const BOT_NAME = "QA機器人";

// ── #799 對帳常數（與 web/3d/main.js 同源，照抄勿改）──
const RECONCILE_JUMP_PX = 800;
const RECONCILE_DEADZONE_PX = 24;
const RECONCILE_CALIB = 0.5;
const RECONCILE_STOP_RATE = 12;

// 方向 → input 旗標 ＋ wasm mask（上1 下2 左4 右8，與伺服器 Player::step 同形）
const DIR_KEYS = {
  up: { up: true }, down: { down: true }, left: { left: true }, right: { right: true },
};
const DIR_MASK = { up: 1, down: 2, left: 4, right: 8 };

const now = () => performance.now();

// ── 載入 WebSocket：優先用 Node 22 內建全域，退回 repo 的 ws 套件 ──
async function getWebSocket() {
  if (typeof globalThis.WebSocket === "function") return globalThis.WebSocket;
  const mod = await import("ws");
  return mod.default || mod.WebSocket;
}

// ── 載入 world-core wasm（比照前端：純 instantiate、無 import 物件）。
// 候選路徑對齊 world-core-wasm.mjs：web/wasm 優先、退回 cargo 的 release 產物。 ──
async function loadWasm() {
  const candidates = [
    path.join(REPO_ROOT, "web", "wasm", "world_core.wasm"),
    path.join(REPO_ROOT, "target", "wasm32-unknown-unknown", "release", "world_core.wasm"),
  ];
  for (const p of candidates) {
    if (!fs.existsSync(p)) continue;
    const buf = fs.readFileSync(p);
    const { instance } = await WebAssembly.instantiate(buf, {});
    const ex = instance.exports;
    if (["step_player", "step_out_x", "step_out_y"].every((fn) => typeof ex[fn] === "function")) {
      console.log(`[wasm] 預測使用 ${p}（伺服器同一份實作）`);
      return ex;
    }
  }
  throw new Error("找不到含 step_player 的 world_core.wasm（先跑 scripts/build-wasm.sh）");
}

function stddev(arr) {
  if (arr.length < 2) return 0;
  const m = arr.reduce((a, b) => a + b, 0) / arr.length;
  const v = arr.reduce((a, b) => a + (b - m) * (b - m), 0) / arr.length;
  return Math.sqrt(v);
}
const avg = (arr) => (arr.length ? arr.reduce((a, b) => a + b, 0) / arr.length : 0);
const fmt = (x, d = 1) => (Number.isFinite(x) ? x.toFixed(d) : "—");

async function main() {
  const WS = await getWebSocket();
  const wasm = await loadWasm();
  const runMult = typeof wasm.run_mult === "function" ? wasm.run_mult() : 1.6;

  console.log(`# ButFun 移動 QA`);
  console.log(`連線：${url}`);
  console.log(`走路：方向=${dirArg}  時長=${seconds}s  跑步=${RUN}  wasm run_mult=${fmt(runMult, 3)}`);
  console.log(`對帳：CALIB=${RECONCILE_CALIB} DEADZONE=${RECONCILE_DEADZONE_PX}px JUMP=${RECONCILE_JUMP_PX}px STOP_RATE=${RECONCILE_STOP_RATE}\n`);

  // ── 狀態 ──
  let myId = null;
  let latestSelfWorld = null;       // 最近一筆自己的權威世界座標 {x,y}
  let selfAuthSeq = 0;              // 每收到一筆「含自己」的快照就 +1（通知預測對帳可校準）
  const authSamples = [];          // [{t, x, y}] 自己的權威座標時間序列
  let firstInputT = 0;             // 送出第一筆走路 input 的時刻
  let firstMoveT = 0;              // 自己權威座標首次明顯移動的時刻（RTT proxy）
  let startAuth = null;            // 開始走路那刻的權威座標（判斷「首次移動」基準）

  const ws = new WS(url);
  const inputKeys = { up: false, down: false, left: false, right: false, run: RUN };

  const recordSelf = (p) => {
    if (!p || typeof p.x !== "number" || typeof p.y !== "number") return;
    const t = now();
    latestSelfWorld = { x: p.x, y: p.y };
    selfAuthSeq++;
    authSamples.push({ t, x: p.x, y: p.y });
    // RTT proxy：走路開始後，權威座標首次離開起點 > 4px 的時刻
    if (firstInputT && !firstMoveT && startAuth) {
      if (Math.hypot(p.x - startAuth.x, p.y - startAuth.y) > 4) firstMoveT = t;
    }
  };

  ws.addEventListener("message", (ev) => {
    let msg;
    try { msg = JSON.parse(typeof ev.data === "string" ? ev.data : ev.data.toString()); }
    catch { return; }
    if (msg.type === "welcome") {
      myId = msg.id;
      const w = msg.world ? `${msg.world.width}x${msg.world.height}` : "?";
      console.log(`[welcome] myId=${myId} world=${w}`);
    } else if (msg.type === "snapshot" && Array.isArray(msg.players)) {
      recordSelf(msg.players.find((pl) => pl.id === myId));
    }
  });
  ws.addEventListener("error", (e) => {
    console.error("WebSocket error:", e && e.message ? e.message : e);
  });

  await new Promise((res, rej) => {
    const to = setTimeout(() => rej(new Error("連線逾時（10s）")), 10000);
    ws.addEventListener("open", () => { clearTimeout(to); res(); });
    ws.addEventListener("error", () => { clearTimeout(to); rej(new Error("連線失敗")); });
  });
  console.log("[open] 已連上，送出 join…");
  ws.send(JSON.stringify({ type: "join", name: BOT_NAME, species: "terran" }));

  // 等到收到自己的第一筆權威座標（最多 5s），確保預測有錨點
  await new Promise((res) => {
    const t0 = now();
    const iv = setInterval(() => {
      if (latestSelfWorld || now() - t0 > 5000) { clearInterval(iv); res(); }
    }, 30);
  });
  if (!latestSelfWorld) {
    console.error("等不到自己的權威座標（5s）。可能 join 沒被接受或沒有自己的快照。");
    ws.close();
    return;
  }
  console.log(`[anchor] 自己權威起點 x=${fmt(latestSelfWorld.x)} y=${fmt(latestSelfWorld.y)}\n`);

  // ── 開始走路：送 input（鏡像「只在意圖改變時送」，這裡意圖整段不變故送一次）──
  Object.assign(inputKeys, { up: false, down: false, left: false, right: false });
  Object.assign(inputKeys, DIR_KEYS[dirArg]);
  startAuth = { ...latestSelfWorld };
  firstInputT = now();
  ws.send(JSON.stringify({ type: "input", ...inputKeys }));

  // ── 客戶端預測模擬：60fps，套 #799 對帳 ──
  // predWorld = 預測基準（世界 px），錨在最近權威；每幀用 wasm step_player 外插當前輸入。
  const predWorld = { x: latestSelfWorld.x, y: latestSelfWorld.y };
  let lastCalibSeq = selfAuthSeq;   // 已消化的權威序號（錨定當幀算已消化，避免立刻又校準）
  let prevPred = { x: predWorld.x, y: predWorld.y };
  let lastFrameT = now();
  const frames = [];                // [{t, x, y, err, step, calibrated, lurch}]
  const mask = DIR_MASK[dirArg];

  const walkEndT = now() + seconds * 1000;
  await new Promise((res) => {
    const iv = setInterval(() => {
      const t = now();
      const dt = (t - lastFrameT) / 1000;
      lastFrameT = t;
      if (dt <= 0) return;

      // 1) 外插：從 predBase 用 wasm step_player 推進當前這一幀的輸入（含碰撞、跑步同源）
      const moveDt = dt * (inputKeys.run ? runMult : 1);
      wasm.step_player(predWorld.x, predWorld.y, mask, moveDt);
      predWorld.x = wasm.step_out_x();
      predWorld.y = wasm.step_out_y();

      // 2) 對帳（#799）：每快照把 predBase 往權威拉 CALIB；死區 24；停下收斂
      let calibrated = false, lurch = 0;
      const ex = latestSelfWorld.x - predWorld.x, ey = latestSelfWorld.y - predWorld.y;
      const err = Math.hypot(ex, ey);
      const moving = mask !== 0;
      if (err > RECONCILE_JUMP_PX) {
        predWorld.x = latestSelfWorld.x; predWorld.y = latestSelfWorld.y;
        calibrated = true; lurch = err; // 直接 snap（傳送級）
      } else {
        if (selfAuthSeq !== lastCalibSeq && err > RECONCILE_DEADZONE_PX) {
          predWorld.x += ex * RECONCILE_CALIB; predWorld.y += ey * RECONCILE_CALIB;
          calibrated = true; lurch = RECONCILE_CALIB * err; // 這一幀因校準額外位移的量
        }
        if (!moving) {
          const a = 1 - Math.exp(-dt * RECONCILE_STOP_RATE);
          predWorld.x += (latestSelfWorld.x - predWorld.x) * a;
          predWorld.y += (latestSelfWorld.y - predWorld.y) * a;
        }
      }
      lastCalibSeq = selfAuthSeq;

      // 3) 紀錄：本幀預測座標、誤差、與前一幀的位移（看快照邊界有沒有猛跳）
      const step = Math.hypot(predWorld.x - prevPred.x, predWorld.y - prevPred.y);
      frames.push({ t, x: predWorld.x, y: predWorld.y, err, step, calibrated, lurch });
      prevPred = { x: predWorld.x, y: predWorld.y };

      if (t >= walkEndT) { clearInterval(iv); res(); }
    }, 1000 / 60);
  });

  // 停止走路
  Object.assign(inputKeys, { up: false, down: false, left: false, right: false });
  ws.send(JSON.stringify({ type: "input", ...inputKeys }));
  ws.close();

  // ── 分析 ──
  const report = [];
  const P = (s = "") => { console.log(s); report.push(s); };

  P("\n========== 量測結果 ==========");

  // (1) 伺服器端真相
  P("\n[1] 伺服器端真相（自己的權威快照）");
  P(`收到含自己的快照數：${authSamples.length}`);
  const intervals = [];
  const speeds = [];
  let monotonic = true;
  const axisKey = (dirArg === "left" || dirArg === "right") ? "x" : "y";
  const axisSign = (dirArg === "right" || dirArg === "down") ? 1 : -1;
  for (let i = 1; i < authSamples.length; i++) {
    const a = authSamples[i - 1], b = authSamples[i];
    const dt = (b.t - a.t) / 1000;
    intervals.push(b.t - a.t);
    const dist = Math.hypot(b.x - a.x, b.y - a.y);
    if (dt > 0) speeds.push(dist / dt);
    if ((b[axisKey] - a[axisKey]) * axisSign < -1) monotonic = false; // 主軸倒退 > 1px
  }
  // 只取「明顯在動」的速度樣本（排除尚未起步／撞牆停住的 0）
  const movingSpeeds = speeds.filter((s) => s > 5);
  P(`快照間隔：平均 ${fmt(avg(intervals))}ms  抖動(stddev) ${fmt(stddev(intervals))}ms  min ${fmt(Math.min(...intervals))} max ${fmt(Math.max(...intervals))}`);
  P(`伺服器移動速度：平均 ${fmt(avg(movingSpeeds))} px/s（移動樣本 ${movingSpeeds.length}）  min ${fmt(movingSpeeds.length ? Math.min(...movingSpeeds) : 0)} max ${fmt(movingSpeeds.length ? Math.max(...movingSpeeds) : 0)}`);
  P(`座標沿 ${dirArg} 主軸單調遞增：${monotonic ? "是（平順）" : "否（有倒退／抖動）"}`);
  if (authSamples.length >= 2) {
    const total = Math.hypot(authSamples.at(-1).x - authSamples[0].x, authSamples.at(-1).y - authSamples[0].y);
    P(`總位移：${fmt(total)} px（起 ${fmt(authSamples[0].x)},${fmt(authSamples[0].y)} → 終 ${fmt(authSamples.at(-1).x)},${fmt(authSamples.at(-1).y)}）`);
  }

  // (2) 客戶端預測誤差
  P("\n[2] 客戶端預測（#799 對帳）誤差");
  P(`模擬幀數：${frames.length}（${fmt(frames.length / seconds)} fps）`);
  const errs = frames.map((f) => f.err);
  if (errs.length) {
    P(`預測誤差(預測 vs 最近權威)：平均 ${fmt(avg(errs))}px  min ${fmt(Math.min(...errs))}  max ${fmt(Math.max(...errs))}`);
    const tail = errs.slice(Math.floor(errs.length / 2)); // 穩定段（後半），避開起步暫態
    P(`穩定段(後半)平均誤差：${fmt(avg(tail))}px`);
  }

  // (3) 快照邊界 lurch（猛跳）
  P("\n[3] 每快照預測位置猛跳(lurch)＝卡頓嫌疑");
  const calibFrames = frames.filter((f) => f.calibrated);
  const lurches = calibFrames.map((f) => f.lurch);
  const normalSteps = frames.filter((f) => !f.calibrated).map((f) => f.step);
  const avgNormalStep = avg(normalSteps);
  P(`平常幀位移(非校準幀)：平均 ${fmt(avgNormalStep, 2)}px/幀（≈速度/60）`);
  if (lurches.length) {
    P(`校準幀數：${calibFrames.length}（每快照一次）`);
    P(`校準額外位移(lurch)：平均 ${fmt(avg(lurches))}px  max ${fmt(Math.max(...lurches))}px`);
    const calibSteps = calibFrames.map((f) => f.step); // 校準幀「總位移」＝平常移動 + lurch（螢幕上一幀跳多遠）
    const avgCalibStep = avg(calibSteps);
    P(`校準幀總位移：平均 ${fmt(avgCalibStep, 2)}px/幀  max ${fmt(Math.max(...calibSteps), 2)}px/幀`);
    P(`→ 卡頓比＝校準幀位移 / 平常幀位移 ≈ ${fmt(avgCalibStep / (avgNormalStep || 1), 1)}x（>3x 肉眼可見一頓一頓）`);
  } else {
    P("無校準幀（誤差一直在死區內，預測很穩）。");
  }

  // (4) RTT proxy
  P("\n[4] 延遲估計");
  if (firstInputT && firstMoveT) {
    P(`input→權威首次移動：${fmt(firstMoveT - firstInputT)}ms（≈ RTT + 一個 server tick；網路 RTT 約再扣半個 tick）`);
  } else {
    P("未量到 input→移動延遲（沒看到權威座標起步）。");
  }

  P("\n==============================");

  // 存一份到 scratchpad
  try {
    const outDir = "/tmp/claude-1000/-home-shihshih-ButFun/ef6ad408-d228-4f2b-9f33-dd6d6c332fde/scratchpad";
    fs.mkdirSync(outDir, { recursive: true });
    const stamp = new Date().toISOString().replace(/[:.]/g, "-");
    const outPath = path.join(outDir, `move-qa-${stamp}.txt`);
    fs.writeFileSync(outPath, report.join("\n") + "\n");
    console.log(`\n報告已存：${outPath}`);
  } catch (e) {
    console.warn("存報告失敗（不影響量測）：", e.message);
  }
}

main().then(() => process.exit(0)).catch((e) => {
  console.error("QA 失敗：", e && e.stack ? e.stack : e);
  process.exit(1);
});
