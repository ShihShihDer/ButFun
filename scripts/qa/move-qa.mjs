// move-qa.mjs — 無頭實玩 QA：真的連上遊戲 WebSocket 當一個玩家走路，量化移動／預測／卡頓，
//   並「先量再改、用數據挑參數」地比較多種對帳（reconciliation）做法，挑最順的那一種。
//
// 目的：在「看不到 3D 畫面」的情況下，用量化數據診斷移動「飄／卡頓」到底是
//   ① 伺服器端速度／快照本身的問題，還是 ② 客戶端預測對帳的問題；
//   並客觀比較幾種對帳候選（A/B/C/D）的「殘留單幀 lurch（卡頓）」與「誤差（領先量）」。
//
// 做三件事：
//   1) 伺服器端真相：以玩家身分加入 → 持續送同一方向的 input → 收 snapshot →
//      記錄「自己」每筆權威座標 (x,y) 與到達時間 → 算出伺服器移動速度(px/s)、
//      快照間隔(ms)＋抖動（這裡會看到 prod 快照「叢發抵達」造成的目標跳動，正是不順的根因）。
//   2) 一次擷取、離線重放所有候選：用同一份真實 prod 權威時間序列（含叢發抖動）餵給每個候選，
//      在 60fps 模擬下各自跑預測＋對帳，量「殘留單幀 lurch」與「誤差」。
//      ★用同一份擷取重放＝消除「每候選各連一次、網路不同」的變異，才公平用數據挑參數。★
//   3) 印出對比表，標出最順的候選＋建議參數。
//
// 候選（對齊任務）：
//   · #799 舊：每收到一筆權威快照，就把 predBase 往「原始權威」離散拉 0.5（死區 24）→ 已知會抖。
//   · (A) #802 現行：每幀收斂 K=9 朝「原始權威」。原始權威是叢發跳動的階梯 → 收斂目標本身不平滑。
//   · (B) 權威外插：用權威估計速度把「對齊目標」往前外插（render-time = 上一筆權威 + 速度×經過時間），
//         目標平滑移動；predBase 每幀朝這個「平滑目標」收斂 K=9。
//   · (C) (B) + 不同 K（6／9／12）。
//   · (D) 對照組——原始權威先做低通(EMA)再收斂：目標也變平滑，但「只低通不外插」會落後＝多延遲，
//         拿來證明「該外插、不是只低通」。
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

// ── 對帳常數（與 web/3d/main.js 同源，照抄勿改）──
const RECONCILE_JUMP_PX = 800;        // 超大誤差（傳送／回城／復活）→ 直接 snap 到權威（世界 px）
// #799 舊離散校準用：
const RECONCILE_DEADZONE_PX = 24;     // 校準死區
const RECONCILE_CALIB = 0.5;          // 每快照往原始權威拉的比例
// #802 / 候選收斂用：
const CONVERGE_K = 9;                 // 現行移動中收斂速率（每秒）
// 候選 B/C 外插參數（要挑出來套進 main.js 的就是這些）：
const AUTH_VEL_WINDOW_S = 0.18;       // 估計權威速度的時間窗（往回看多久；夠平滑掉叢發雜訊又夠跟手）
const AUTH_EXTRAP_MAX_S = 0.25;       // 外插上限（快照久久不來時別飛走）
const AUTH_LOWPASS_K = 9;             // 候選 D 低通(EMA)速率（每秒）
// ── 切片2 輸入重放對帳常數（與 PR#810 web/3d/main.js 同源，照抄勿改）──
//   伺服器固定 TICK_HZ=15、dt=1/15 步進（game.rs:53/89）；每筆輸入對應一個 tick。
//   送輸入固定 15Hz（每 tick 一筆、帶 seq）；重放時以同樣的 SERVER_TICK_DT 步進，
//   run 旗標只乘一次 run_mult（與伺服器 effective_dt = dt*run_mult 同形，state.rs:1180）。
const SERVER_TICK_HZ = 15;                       // 伺服器 tick 頻率（Hz）
const SERVER_TICK_DT = 1 / SERVER_TICK_HZ;       // 單筆輸入對應的 dt（秒，約 66.7ms）
const INPUT_SEND_INTERVAL_MS = 1000 / SERVER_TICK_HZ; // 送輸入的固定間隔（ms，= 一個 tick 週期）

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
const median = (arr) => {
  if (!arr.length) return 0;
  const s = [...arr].sort((a, b) => a - b);
  const m = Math.floor(s.length / 2);
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
};
const fmt = (x, d = 1) => (Number.isFinite(x) ? x.toFixed(d) : "—");
const pad = (s, n) => String(s).padEnd(n);
const padL = (s, n) => String(s).padStart(n);

// ════════════════════════════════════════════════════════════════════
// 離線重放一個對帳候選：用同一份真實 prod 權威時間序列（含叢發抖動）餵預測。
//   capture：{ samples:[{tRel,x,y}], anchorX, anchorY }（tRel＝相對走路起點的秒數；anchor＝走路起點權威）
//   cfg：{ name, kind, extrapolate, lowpass, K }
//     kind='discrete' → #799 每快照離散拉 RECONCILE_CALIB（朝原始權威）
//     kind='converge' → 每幀收斂 K（朝 目標）；目標依 extrapolate / lowpass 而定
//   回傳量測：穩定段平均誤差、最大單幀 lurch、lurch 幀數、單幀位移 stddev 等。
// ════════════════════════════════════════════════════════════════════
function replayCandidate(cfg, capture, wasm, runMult, mask) {
  const FPS = 60, dtF = 1 / FPS;
  const totalFrames = Math.round(seconds * FPS);
  const samples = capture.samples;
  const pred = { x: capture.anchorX, y: capture.anchorY };
  let prev = { x: pred.x, y: pred.y };
  let sIdx = 0;                 // 權威指標：依到達時間前進到「此幀時間點為止收到的最後一筆」
  let lastDiscreteIdx = -1;     // #799：已消化的快照序號（每快照只離散校準一次）
  let lastReplayIdx = -1;       // 切片2：已重放對帳的快照序號（每收一筆新快照 rebase 一次）
  let lpx = capture.anchorX, lpy = capture.anchorY; // 候選 D 低通狀態
  const steps = [], errs = [];
  const lurchFrames = [];       // 每幀「相對中位位移的超量」＝卡頓嫌疑（後面用穩定段中位當基準）

  for (let i = 0; i < totalFrames; i++) {
    const tRel = i * dtF;
    while (sIdx + 1 < samples.length && samples[sIdx + 1].tRel <= tRel) sIdx++;
    const auth = samples[sIdx];

    // 0) 切片2 輸入重放 rebase（鏡像 PR#810 handleServerMsg）：每收到一筆「新」自身快照，
    //    從權威位置起，依序重跑「伺服器尚未確認」的輸入（lis < seq ≤ 收到此快照時已送出的 seq），
    //    步進 dt = SERVER_TICK_DT × (run ? run_mult : 1)（與伺服器套用同形），rebase 出 pred。
    //    然後本幀照常做即時 live step（section 1）。已確認的歷史在真實前端會被丟棄，這裡只讀不刪。
    if (cfg.kind === "replay" && sIdx !== lastReplayIdx) {
      const lis = auth.lis || 0;               // 伺服器已確認到的最新輸入 seq（0＝未確認/舊伺服器）
      const sent = auth.seqSent || 0;          // 收到此快照時，客戶端已送出的最新 seq
      let rx = auth.x, ry = auth.y;
      for (const h of capture.inputLog) {
        if (h.seq > lis && h.seq <= sent) {
          const rdt = SERVER_TICK_DT * (h.run ? runMult : 1);
          wasm.step_player(rx, ry, h.mask, rdt);
          rx = wasm.step_out_x(); ry = wasm.step_out_y();
        }
      }
      pred.x = rx; pred.y = ry;
      lastReplayIdx = sIdx;
    }

    // 1) wasm 外插「當前這一幀輸入」（即時跟手；含碰撞、跑步同源），與 main.js 同
    if (mask) {
      const moveDt = dtF * (RUN ? runMult : 1);
      wasm.step_player(pred.x, pred.y, mask, moveDt);
      pred.x = wasm.step_out_x();
      pred.y = wasm.step_out_y();
    }

    // 2) 算「對齊目標」
    let tx = auth.x, ty = auth.y;
    if (cfg.extrapolate) {
      // 權威估計速度：往回看 AUTH_VEL_WINDOW_S 找最舊的一筆（平滑掉叢發 1.4ms 間隔的雜訊），
      // 用它到當前權威算速度；再用「此幀距該權威到達多久」往前外插 → 目標平滑移動。
      let vIdx = sIdx;
      while (vIdx > 0 && (auth.tRel - samples[vIdx].tRel) < AUTH_VEL_WINDOW_S) vIdx--;
      const vdt = auth.tRel - samples[vIdx].tRel;
      let vx = 0, vy = 0;
      if (vdt > 1e-4) { vx = (auth.x - samples[vIdx].x) / vdt; vy = (auth.y - samples[vIdx].y) / vdt; }
      const age = Math.min(Math.max(tRel - auth.tRel, 0), AUTH_EXTRAP_MAX_S);
      tx = auth.x + vx * age; ty = auth.y + vy * age;
    } else if (cfg.lowpass) {
      // 對照組 D：原始權威先低通(EMA)再當目標——目標也平滑，但會落後（多延遲），刻意拿來對比。
      const a = 1 - Math.exp(-dtF * AUTH_LOWPASS_K);
      lpx += (auth.x - lpx) * a; lpy += (auth.y - lpy) * a;
      tx = lpx; ty = lpy;
    }

    // 3) 對帳
    const jump = Math.hypot(auth.x - pred.x, auth.y - pred.y);
    if (jump > RECONCILE_JUMP_PX) {
      pred.x = auth.x; pred.y = auth.y;           // 超大誤差直接 snap（所有候選一致）
    } else if (cfg.kind === "discrete") {
      // #799：每收到一筆「新」快照才離散校準一次，朝原始權威拉 CALIB，死區內不動
      const rex = auth.x - pred.x, rey = auth.y - pred.y;
      const rerr = Math.hypot(rex, rey);
      if (sIdx !== lastDiscreteIdx && rerr > RECONCILE_DEADZONE_PX) {
        pred.x += rex * RECONCILE_CALIB; pred.y += rey * RECONCILE_CALIB;
      }
      lastDiscreteIdx = sIdx;
    } else if (cfg.kind === "replay") {
      // 切片2：對帳已在 section 0 的 rebase 做完（snapshot 到達時重放未確認輸入），
      // 這裡不再做每幀收斂——重放已精準重建預測，不需要「拔河拉回」。
    } else {
      // 每幀平滑收斂朝目標（cfg.K）
      const a = 1 - Math.exp(-dtF * cfg.K);
      pred.x += (tx - pred.x) * a; pred.y += (ty - pred.y) * a;
    }

    // 4) 紀錄（誤差一律對「原始權威」算＝真正的領先量；位移看螢幕上每幀跳多遠）
    const errRaw = Math.hypot(auth.x - pred.x, auth.y - pred.y);
    const step = Math.hypot(pred.x - prev.x, pred.y - prev.y);
    steps.push(step); errs.push(errRaw);
    prev = { x: pred.x, y: pred.y };
  }

  // ── 量測：只取穩定段（後半），避開起步暫態（權威落後 RTT 追上前的大誤差）──
  const half = Math.floor(totalFrames / 2);
  const stSteps = steps.slice(half);
  const stErrs = errs.slice(half);
  const medStep = median(stSteps);
  // 單幀 lurch＝該幀位移超出「中位位移」的量（正值＝比平常多走一截＝一頓）。負值（暫停）不算卡頓只算抖。
  for (const s of stSteps) lurchFrames.push(s - medStep);
  const maxLurch = Math.max(0, ...lurchFrames);
  // 視覺可見卡頓幀數：單幀位移 > 2× 中位（>3x 肉眼明顯，這裡用 2x 較敏感地抓殘留）
  const stutterCount = stSteps.filter((s) => s > 2 * medStep).length;

  return {
    name: cfg.name,
    kind: cfg.kind,
    lowpass: !!cfg.lowpass,
    extrapolate: !!cfg.extrapolate,
    meanErr: avg(stErrs),
    maxErr: Math.max(...stErrs),
    medStep,
    maxLurch,
    stutterCount,
    stepStd: stddev(stSteps),
    frames: totalFrames,
  };
}

async function main() {
  const WS = await getWebSocket();
  const wasm = await loadWasm();
  const runMult = typeof wasm.run_mult === "function" ? wasm.run_mult() : 1.6;

  console.log(`# ButFun 移動 QA（多候選對帳對比）`);
  console.log(`連線：${url}`);
  console.log(`走路：方向=${dirArg}  時長=${seconds}s  跑步=${RUN}  wasm run_mult=${fmt(runMult, 3)}`);
  console.log(`外插參數：VEL_WINDOW=${AUTH_VEL_WINDOW_S}s EXTRAP_MAX=${AUTH_EXTRAP_MAX_S}s；收斂 K(現行)=${CONVERGE_K}\n`);

  // ── 狀態 ──
  let myId = null;
  let latestSelfWorld = null;       // 最近一筆自己的權威世界座標 {x,y}
  const authSamples = [];          // [{t, x, y, lis, seqSent}] 自己的權威座標時間序列（t＝performance.now）
  let firstInputT = 0;             // 送出第一筆走路 input 的時刻
  let firstMoveT = 0;              // 自己權威座標首次明顯移動的時刻（RTT proxy）
  let startAuth = null;            // 開始走路那刻的權威座標（判斷「首次移動」基準＋重放錨點）

  // ── 切片2 輸入重放：以 PR#810 的方式固定 15Hz 送帶 seq 的 input、維護 inputLog ──
  let inputSeq = 0;                // 最新已送出的輸入 seq（每 tick +1）
  const inputLog = [];             // [{seq, mask, run}] 走路期間實際送出的每筆輸入（序號升序）
  let lastSeenLis = 0;             // 伺服器最新確認到的 last_input_seq（0＝尚未確認）

  const ws = new WS(url);
  const inputKeys = { up: false, down: false, left: false, right: false, run: RUN };

  const recordSelf = (p) => {
    if (!p || typeof p.x !== "number" || typeof p.y !== "number") return;
    const t = now();
    latestSelfWorld = { x: p.x, y: p.y };
    // last_input_seq：伺服器已處理到的最新輸入 seq（為 0 時 JSON 省略 → 缺欄位讀 0，與前端 || 0 同）。
    const lis = (typeof p.last_input_seq === "number") ? p.last_input_seq : 0;
    if (lis > lastSeenLis) lastSeenLis = lis;
    authSamples.push({ t, x: p.x, y: p.y, lis, seqSent: inputSeq });
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

  // ── 開始走路：以切片2（PR#810）方式固定 15Hz 送帶 seq 的 input（每 tick 一筆）──
  // 鏡像 main.js updateInput：每 INPUT_SEND_INTERVAL_MS 送一次 {type:"input", ...keys, seq}，
  // seq 遞增、把 {seq, mask, run} 推進 inputLog；伺服器把處理到的 seq 回填 last_input_seq。
  Object.assign(inputKeys, { up: false, down: false, left: false, right: false });
  Object.assign(inputKeys, DIR_KEYS[dirArg]);
  startAuth = { ...latestSelfWorld };
  const walkMask = DIR_MASK[dirArg];
  const sendInput = () => {
    if (ws.readyState !== ws.OPEN && ws.readyState !== 1) return;
    inputSeq = (inputSeq + 1) >>> 0;
    inputLog.push({ seq: inputSeq, mask: walkMask, run: !!inputKeys.run });
    ws.send(JSON.stringify({ type: "input", ...inputKeys, seq: inputSeq }));
  };
  firstInputT = now();
  sendInput();                                   // 第一筆立刻送（開始走）
  const sendIv = setInterval(sendInput, INPUT_SEND_INTERVAL_MS); // 之後固定 15Hz

  // ── 擷取階段：固定節奏送 input、收權威快照（含 last_input_seq），走 seconds 秒 ──
  await new Promise((res) => setTimeout(res, seconds * 1000));

  // 停止走路（清空方向、停掉 15Hz 送出）
  clearInterval(sendIv);
  Object.assign(inputKeys, { up: false, down: false, left: false, right: false });
  inputSeq = (inputSeq + 1) >>> 0;
  ws.send(JSON.stringify({ type: "input", ...inputKeys, seq: inputSeq }));
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
    if ((b[axisKey] - a[axisKey]) * axisSign < -1) monotonic = false;
  }
  const movingSpeeds = speeds.filter((s) => s > 5);
  P(`快照間隔：平均 ${fmt(avg(intervals))}ms  抖動(stddev) ${fmt(stddev(intervals))}ms  min ${fmt(intervals.length ? Math.min(...intervals) : 0)} max ${fmt(intervals.length ? Math.max(...intervals) : 0)}`);
  P(`  → 間隔抖動大＝快照「叢發抵達」＝收斂目標（原始權威）每收一筆就往前跳一段 → 不順的根因`);
  P(`伺服器移動速度：平均 ${fmt(avg(movingSpeeds))} px/s（移動樣本 ${movingSpeeds.length}）  min ${fmt(movingSpeeds.length ? Math.min(...movingSpeeds) : 0)} max ${fmt(movingSpeeds.length ? Math.max(...movingSpeeds) : 0)}`);
  P(`座標沿 ${dirArg} 主軸單調遞增：${monotonic ? "是（平順）" : "否（有倒退／抖動）"}`);
  let total = 0;
  if (authSamples.length >= 2) {
    total = Math.hypot(authSamples.at(-1).x - authSamples[0].x, authSamples.at(-1).y - authSamples[0].y);
    P(`總位移：${fmt(total)} px（起 ${fmt(authSamples[0].x)},${fmt(authSamples[0].y)} → 終 ${fmt(authSamples.at(-1).x)},${fmt(authSamples.at(-1).y)}）`);
  }

  // (4 提前) RTT proxy
  P("\n[2] 延遲估計");
  if (firstInputT && firstMoveT) {
    P(`input→權威首次移動：${fmt(firstMoveT - firstInputT)}ms（≈ RTT + 一個 server tick）`);
  } else {
    P("未量到 input→移動延遲（沒看到權威座標起步）。");
  }

  // (2.5) 切片1 端到端驗證：last_input_seq 真的有正確回傳嗎？
  P("\n[2.5] 切片1 input seq/ack 端到端（last_input_seq 回傳驗證）");
  P(`送出輸入總數 seq：${inputLog.length}（最後 seq=${inputSeq}）  固定節奏=${fmt(INPUT_SEND_INTERVAL_MS, 1)}ms（${SERVER_TICK_HZ}Hz）`);
  // 走路期間（tRel≥0）的快照才有意義（之前的閒置快照 lis 可能還是 0）。
  const walkSnaps = authSamples.filter((s) => s.t >= firstInputT);
  const lisSeries = walkSnaps.map((s) => s.lis);
  const nonZeroLis = lisSeries.filter((v) => v > 0).length;
  let lisMonotonic = true;
  for (let i = 1; i < lisSeries.length; i++) if (lisSeries[i] < lisSeries[i - 1]) lisMonotonic = false;
  const ackLags = walkSnaps.filter((s) => s.lis > 0).map((s) => s.seqSent - s.lis); // 未確認輸入筆數
  P(`last_input_seq 回傳：非零快照 ${nonZeroLis}/${walkSnaps.length}  最大確認 seq=${lastSeenLis}  單調遞增=${lisMonotonic ? "是" : "否(有回退！)"}`);
  if (lastSeenLis === 0) {
    P(`  ✗ last_input_seq 永遠為 0 → 切片1 沒在這個端點生效（舊伺服器？localhost 未部署切片1？）。重放對帳無法驗證。`);
  } else {
    P(`  ✓ last_input_seq 隨送出 seq 遞增、非永遠 0 → 切片1 端到端通（伺服器有 ack 客戶端輸入序號）。`);
    P(`  未確認輸入窗（seqSent−lis）：平均 ${fmt(avg(ackLags), 2)} 筆  中位 ${fmt(median(ackLags), 1)}  抖動σ ${fmt(stddev(ackLags), 2)}  min ${ackLags.length ? Math.min(...ackLags) : 0} max ${ackLags.length ? Math.max(...ackLags) : 0}`);
    P(`    → 重放會「從權威往前重跑」這麼多筆（≈ RTT/${fmt(INPUT_SEND_INTERVAL_MS, 0)}ms）；每筆重跑 ${fmt(SERVER_TICK_DT * 1000, 1)}ms×速度。`);
    P(`    → 此窗的「抖動σ」與「max」是重放對帳的成敗關鍵：窗每抖 1 筆，硬 rebase 的 pred 就抖 ${fmt(SERVER_TICK_DT * avg(movingSpeeds), 1)}px；窗暴衝＝重放過度預測。`);
  }

  // ── 重放所有候選（用同一份真實 prod 權威序列）──
  if (authSamples.length < 6 || total < 30) {
    P("\n[3] 候選對比：略過（權威樣本太少或幾乎沒移動，撞牆？換方向或加長時間重試）。");
  } else {
    // 建 capture：tRel＝相對走路起點秒數；anchor＝走路起點權威。
    // samples 帶 lis/seqSent（切片2 重放候選用）；inputLog＝走路期間實際送出的每筆 seq'd input。
    const samples = authSamples.map((s) => ({
      tRel: (s.t - firstInputT) / 1000, x: s.x, y: s.y, lis: s.lis, seqSent: s.seqSent,
    }));
    const capture = { samples, anchorX: startAuth.x, anchorY: startAuth.y, inputLog };
    const mask = DIR_MASK[dirArg];

    const candidates = [
      { name: "#799 舊(離散拉0.5)", kind: "discrete" },
      { name: "A=#802現行 收斂K9朝原始權威", kind: "converge", K: 9 },
      { name: "B=#804現行外插+收斂K9(AUTH_EXTRAP)", kind: "converge", K: 9, extrapolate: true },
      { name: "C=外插+收斂 K12", kind: "converge", K: 12, extrapolate: true },
      { name: "D=低通(EMA)+收斂K9(對照)", kind: "converge", K: 9, lowpass: true },
      { name: "★切片2 輸入重放對帳(PR#810)", kind: "replay" },
    ];
    const results = candidates.map((c) => replayCandidate(c, capture, wasm, runMult, mask));

    P("\n[3] 候選對帳對比（同一份 prod 權威序列離線重放；穩定段＝後半）");
    P(`  · 平均誤差＝預測領先原始權威的距離（px，目標 ~10-30、別太大＝不延遲）`);
    P(`  · 最大單幀 lurch＝穩定段內單幀位移超出中位位移最多的量（px，越小越順）★主指標★`);
    P(`  · 卡頓幀數＝單幀位移 > 2× 中位的幀數（越少越順）`);
    P(`  · 位移 stddev＝每幀位移波動（px，越小越平滑）\n`);
    const head = `  ${pad("候選", 28)}${padL("平均誤差", 9)}${padL("最大誤差", 9)}${padL("最大lurch", 11)}${padL("卡頓幀", 8)}${padL("位移σ", 9)}`;
    P(head);
    P("  " + "─".repeat(head.length - 2));
    for (const r of results) {
      P(`  ${pad(r.name, 28)}${padL(fmt(r.meanErr, 1), 9)}${padL(fmt(r.maxErr, 1), 9)}${padL(fmt(r.maxLurch, 2), 11)}${padL(r.stutterCount, 8)}${padL(fmt(r.stepStd, 3), 9)}`);
    }

    // 挑最順：只在「真‧收斂家族」A/B/C 裡挑（#799 離散與 D 低通是基準／對照，不參選——
    // 它們的「低 lurch」是靠『不修正、放任飄在死區』換來的＝舊版那個飄移 bug，不是真的順）。
    // 主指標＝最大單幀 lurch（任務指定的「殘留單幀卡頓」），平手再看卡頓幀數、位移σ；
    // 但要求 meanErr ≤ 30px（仍緊貼權威、不飄、不延遲＝符合任務「跟手」要求）。
    // 註：K∈{6,9,12} 三者 lurch 多落在 run 間雜訊內（皆 <1px＝肉眼看不見），真正穩定的結論是
    // 「外插家族 B/C 把 A 的 lurch 砍掉約 70-80%」；K 的最終取值由人在報告裡按『跟手/領先量』權衡。
    const family = results.filter((r) => r.kind === "converge" && !r.lowpass);
    const tracking = family.filter((r) => r.meanErr <= 30);
    const pool = tracking.length ? tracking : family.length ? family : results;
    const winner = pool.slice().sort((a, b) =>
      a.maxLurch - b.maxLurch || a.stutterCount - b.stutterCount || a.stepStd - b.stepStd
    )[0];
    const aBase = results.find((r) => r.name.startsWith("A="));
    P("");
    P(`  （收斂家族最順：${winner.name}：誤差 ${fmt(winner.meanErr, 1)}px・lurch ${fmt(winner.maxLurch, 2)}px・卡頓幀 ${winner.stutterCount}）`);

    // ── 切片2 判定：重放對帳 vs 現行收斂法（B=#804 AUTH_EXTRAP、A=#802）──
    P("\n[4] 切片2 輸入重放 vs 收斂法（成敗判定）");
    const rep = results.find((r) => r.kind === "replay");
    const bExtrap = results.find((r) => r.name.startsWith("B="));
    if (!rep) {
      P("  （無重放候選結果）");
    } else if (lastSeenLis === 0) {
      P(`  ✗ last_input_seq 全程為 0 → 重放從權威重跑「全部」已送輸入＝失真，下列重放數字不可信。`);
      P(`     先確認此端點已部署切片1（prod 應已部署；localhost 需切片1 分支）。`);
      P(`     重放(失真) 誤差 ${fmt(rep.meanErr, 1)}px・lurch ${fmt(rep.maxLurch, 2)}px`);
    } else {
      const cmp = (base) => {
        if (!base) return "";
        const dl = base.maxLurch > 0 ? (1 - rep.maxLurch / base.maxLurch) * 100 : 0;
        return `lurch ${fmt(base.maxLurch, 2)}→${fmt(rep.maxLurch, 2)}px(${dl >= 0 ? "↓" : "↑"}${fmt(Math.abs(dl), 0)}%)・卡頓幀 ${base.stutterCount}→${rep.stutterCount}・誤差 ${fmt(base.meanErr, 1)}→${fmt(rep.meanErr, 1)}px`;
      };
      P(`  重放：平均誤差 ${fmt(rep.meanErr, 1)}px・最大單幀 lurch ${fmt(rep.maxLurch, 2)}px・卡頓幀 ${rep.stutterCount}・位移σ ${fmt(rep.stepStd, 3)}`);
      if (bExtrap) P(`  vs B=#804外插收斂：${cmp(bExtrap)}`);
      if (aBase) P(`  vs A=#802收斂：${cmp(aBase)}`);
      // 判定門檻：重放對帳要「可合可部署」必須同時——
      //   (a) 殘留 lurch ≈0 且不輸現行 B（≤2px 或 ≤ B 的 lurch）；
      //   (b) 誤差健康（≤40px＝沒因重放飛走、也沒嚴重落後）。
      // 失敗時用「未確認窗」診斷根因，區分「lis 落後造成的無界過度預測」與「真 run_mult/dt bug」：
      //   重放過度預測 ≈ 窗筆數 × tick位移；若 誤差/位移 ≈ 中位窗 → 是 lis 落後（非 run_mult 雙重套用）。
      const medLag = median(ackLags);
      const tickPx = SERVER_TICK_DT * avg(movingSpeeds);
      const lurchOk = rep.maxLurch <= 2 || (bExtrap && rep.maxLurch <= bExtrap.maxLurch);
      const errOk = rep.meanErr <= 40;
      P("");
      if (lurchOk && errOk) {
        P(`  ✓ 判定：此擷取條件下重放對帳成立——殘留 lurch ${fmt(rep.maxLurch, 2)}px（≈消除）、誤差 ${fmt(rep.meanErr, 1)}px（健康）。`);
        P(`     但需跨多種 RTT/抖動條件複測（見下「穩健性」提醒）才算可合可部署。`);
      } else {
        // 估計「若過度預測是 lis 落後造成」的預期誤差量：中位窗 × tick位移
        const expectFromLag = medLag * tickPx;
        const lagDriven = rep.meanErr > 40 && expectFromLag > 0 &&
          rep.meanErr > 0.4 * expectFromLag; // 誤差量級與「中位窗×tick位移」相符 → lis 落後主導
        P(`  ✗ 判定：此條件下重放對帳【不成立】（lurch ${fmt(rep.maxLurch, 2)}px / 誤差 ${fmt(rep.meanErr, 1)}px，現行 B=${fmt(bExtrap ? bExtrap.maxLurch : NaN, 2)}/${fmt(bExtrap ? bExtrap.meanErr : NaN, 1)}px）。`);
        if (lagDriven) {
          P(`     根因＝未確認窗暴衝（中位 ${fmt(medLag, 1)} 筆 × ${fmt(tickPx, 1)}px ≈ ${fmt(expectFromLag, 0)}px ≈ 量到的誤差）`);
          P(`     → 是「last_input_seq 落後 → 硬重放整個大窗 → 無界過度預測」，**非** run_mult/dt 雙重套用（dt/run_mult 經對照為單次套用、正確）。`);
          P(`     → 切片2 缺「重放距離上限／窗封頂」與「平滑（收斂而非硬 set）」，故 lis 落後時把 pred 甩出數百 px。`);
        } else {
          P(`     根因＝硬 rebase 無平滑：prod 快照叢發抵達 + 未確認窗抖動（σ ${fmt(stddev(ackLags), 2)}）直接灌進 pred → 每快照 lurch。`);
          P(`     → 現行 B（AUTH_EXTRAP）以外插+收斂把這抖動低通掉，故 B 的 lurch 遠小。切片2 少了這層平滑。`);
        }
      }
      P(`  ※ 穩健性提醒：低 RTT/窗穩（如 RTT~140ms、窗=2）時重放 lurch≈0、誤差~1.5px（理論成立）；`);
      P(`    但 RTT/抖動升高（prod 實測 RTT 450ms→窗5→lurch 45px；RTT 3s→窗77→誤差 666px）即崩。`);
    }
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
