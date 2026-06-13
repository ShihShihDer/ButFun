// 長住型 AI 居民玩家：以真 WebSocket 連線當訪客玩家，**長時間（數小時/持續）住在遊戲裡生活**，
// 讓世界有人氣；同時**持續軟測**——抓只有長跑才浮現的 bug（伺服器卡死/斷流、卡住、經濟漂移）。
//
// 純 JS 啟發式人格、零 LLM、零瀏覽器（24/7 長跑用 ws 輕量協定，puppeteer 太重）。
// 參考 scripts/qa/functional-qa.mjs 的 WS 連線 / join / snapshot 解析 / 移動範本。
//
// 用法（吃 env 或 argv）：
//   BOT_NAME=露安 BOT_PERSONA=wanderer node scripts/qa/ai-resident-bot.mjs
//   node scripts/qa/ai-resident-bot.mjs <name> <persona> [ws-url]
//   人格 persona：wanderer | hunter | gatherer | socializer
//
// 安全鐵律：① 只連 localhost:3000。② 絕不訊息洪水（此 codebase WS 無限流，bot 必須自律）。
//           ③ 行為像「悠閒生活」不是「壓力轟炸」。
import { WebSocket } from "ws";
import { mkdirSync, appendFileSync } from "fs";

// ── 參數 ──────────────────────────────────────────────────────────────────
const NAME = process.env.BOT_NAME || process.argv[2] || "露安";
const PERSONA = (process.env.BOT_PERSONA || process.argv[3] || "wanderer").toLowerCase();
const URL = process.env.BOT_WS_URL || process.argv[4] || "ws://localhost:3000/ws";

// 安全鐵律 ①：只連 localhost:3000（其它一律拒連，避免誤連線上或外部）。
if (!/^wss?:\/\/(localhost|127\.0\.0\.1):3000(\/|$)/.test(URL)) {
  console.error(`[${NAME}] 安全拒連：只允許 localhost:3000，收到 ${URL}`);
  process.exit(2);
}
const VALID = ["wanderer", "hunter", "gatherer", "socializer"];
if (!VALID.includes(PERSONA)) {
  console.error(`[${NAME}] 未知人格 ${PERSONA}（要 ${VALID.join("/")}）`);
  process.exit(2);
}

// ── 記錄 ──────────────────────────────────────────────────────────────────
const LOG_DIR = "/tmp/butfun-residents";
mkdirSync(LOG_DIR, { recursive: true });
const LOG = `${LOG_DIR}/${NAME}.log`;
function log(line) {
  const ts = new Date().toISOString();
  const s = `${ts} [${NAME}/${PERSONA}] ${line}\n`;
  try { appendFileSync(LOG, s); } catch {}
  process.stdout.write(s);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const rand = (a, b) => a + Math.random() * (b - a);
const VILLAGE = [2344, 2296]; // 新手村主城世界座標（對齊 functional-qa）

// ── 共用狀態 ──────────────────────────────────────────────────────────────
const S = { x: 0, y: 0, ether: 0, hp: 0, maxHp: 100, inv: {},
  nodes: [], npcs: [], enemies: [], players: [], ready: false };
let myId = null;
let ws = null;
let send = () => {};
let lastSnapAt = 0;          // 最近一次收到 snapshot 的時間（卡死偵測）
let snapCount = 0;
let reconnects = 0;
let running = true;          // SIGTERM/SIGINT 後翻 false → 迴圈優雅退出
let stalledWarned = false;   // 卡死警告去抖（同一段斷流只記一次）

// 節流：移動意圖只在「方向改變」或 ≥200ms 才送（≤5/秒），絕不每幀狂送。
let lastInput = { up: false, down: false, left: false, right: false };
let lastInputSentAt = 0;
function sendInput(inp) {
  const now = Date.now();
  const changed = inp.up !== lastInput.up || inp.down !== lastInput.down ||
    inp.left !== lastInput.left || inp.right !== lastInput.right;
  if (!changed && now - lastInputSentAt < 200) return; // 沒變且未滿 200ms → 不送
  if (now - lastInputSentAt < 200) return;             // 硬上限 ≤5/秒
  lastInput = inp; lastInputSentAt = now;
  send({ type: "input", ...inp });
}
function stopMoving() { lastInput = { up: false, down: false, left: false, right: false };
  send({ type: "input", up: false, down: false, left: false, right: false }); }

// 朝目標 (tx,ty) 走一步（節流過的單發意圖，非阻塞——由人格迴圈反覆呼叫）。
function stepToward(tx, ty, deadzone = 8) {
  const dx = tx - S.x, dy = ty - S.y;
  if (Math.hypot(dx, dy) < deadzone * 3) { stopMoving(); return true; } // 已到
  sendInput({ right: dx > deadzone, left: dx < -deadzone, down: dy > deadzone, up: dy < -deadzone });
  return false;
}
const dist = (ax, ay, bx, by) => Math.hypot(ax - bx, ay - by);
const nearest = (arr) => arr.length
  ? arr.reduce((b, e) => dist(e.x, e.y, S.x, S.y) < dist(b.x, b.y, S.x, S.y) ? e : b)
  : null;

// ── 連線 + snapshot 解析（對齊 functional-qa 的 welcome/snapshot 協定）──────
function connect() {
  ws = new WebSocket(URL);
  ws.on("open", () => {
    log(`WS open → join name=${NAME}`);
    send = (o) => { try { ws.send(JSON.stringify(o)); } catch (e) { log(`send 失敗：${e.message}`); } };
    ws.send(JSON.stringify({ type: "join", name: NAME, species: "terran" }));
  });
  ws.on("message", (raw) => {
    let m; try { m = JSON.parse(raw); } catch { return; }
    if (m.type === "welcome") { myId = m.id; log(`welcome id=${myId}`); return; }
    if (m.type === "snapshot" && myId) {
      lastSnapAt = Date.now(); snapCount++;
      if (stalledWarned) { log("✅ snapshot 恢復（斷流結束）"); stalledWarned = false; }
      S.nodes = m.nodes || []; S.npcs = m.npcs || []; S.enemies = m.enemies || [];
      S.players = m.players || [];
      const me = S.players.find((p) => p.id === myId);
      if (me) {
        S.x = me.x; S.y = me.y; S.ether = me.ether; S.hp = me.hp;
        if (me.max_hp) S.maxHp = me.max_hp;
        S.inv = {}; for (const it of me.inventory || []) S.inv[it.item] = it.qty;
        if (!S.ready) { S.ready = true; log(`進場 (${S.x.toFixed(0)},${S.y.toFixed(0)}) HP=${S.hp} 乙太=${S.ether}`); }
      }
    }
  });
  ws.on("error", (e) => { log(`⚠️ WS error：${e.message}`); });
  ws.on("close", (code) => {
    log(`⚠️ WS close code=${code}`);
    S.ready = false; myId = null;
    if (running) scheduleReconnect();
  });
}

// 指數退避自動重連（別狂連）：2s, 4s, 8s … 上限 30s。
function scheduleReconnect() {
  reconnects++;
  const wait = Math.min(2000 * 2 ** Math.min(reconnects - 1, 4), 30000);
  log(`第 ${reconnects} 次重連，${(wait / 1000).toFixed(0)}s 後…`);
  setTimeout(() => { if (running) connect(); }, wait);
}

// ── 人格行為迴圈（每 ~1-3 秒一個決策，純啟發式）─────────────────────────────
let waypoint = null;                 // wanderer / socializer 的航點
let lastChatAt = 0;                  // socializer 聊天節流（≤1 句/30 秒）
const CANNED = ["今天天氣真好～", "這座城真熱鬧。", "有人一起去採集嗎？", "悠閒的一天。", "嗨，路過打個招呼。"];

// 在某中心點附近取一個隨機航點（城內外悠閒遊蕩用）。
function newWaypoint(cx, cy, radius) {
  const a = rand(0, Math.PI * 2), r = rand(radius * 0.3, radius);
  return [cx + Math.cos(a) * r, cy + Math.sin(a) * r];
}

function decideWanderer() {
  if (!waypoint || dist(waypoint[0], waypoint[1], S.x, S.y) < 40) {
    waypoint = newWaypoint(VILLAGE[0], VILLAGE[1], 900); // 城內外 ~900px 悠閒晃
    log(`新航點 (${waypoint[0].toFixed(0)},${waypoint[1].toFixed(0)})`);
  }
  stepToward(waypoint[0], waypoint[1]);
}

function decideHunter() {
  // 血低（<35%）就撤退回城療傷。
  if (S.hp > 0 && S.hp < S.maxHp * 0.35) {
    if (dist(VILLAGE[0], VILLAGE[1], S.x, S.y) > 220) { stepToward(VILLAGE[0], VILLAGE[1]); }
    else { stopMoving(); }
    return;
  }
  const enemy = nearest(S.enemies.filter((e) => e.alive !== false));
  if (enemy) {
    const d = dist(enemy.x, enemy.y, S.x, S.y);
    if (d > 48) stepToward(enemy.x, enemy.y);          // 走近
    else { stopMoving(); send({ type: "attack" }); }   // 靠近就觸發自動戰鬥、打一下
  } else {
    // 沒怪 → 在城外野地巡（朝隨機航點），等怪刷出來。
    if (!waypoint || dist(waypoint[0], waypoint[1], S.x, S.y) < 40)
      waypoint = newWaypoint(VILLAGE[0], VILLAGE[1], 1100);
    stepToward(waypoint[0], waypoint[1]);
  }
}

function decideGatherer() {
  const node = nearest(S.nodes.filter((n) => n.harvestable !== false));
  if (node) {
    const d = dist(node.x, node.y, S.x, S.y);
    if (d > 48) stepToward(node.x, node.y);
    else { stopMoving(); send({ type: "gather" }); } // 靠近採集（對齊 functional-qa 的 gather 動作）
  } else {
    // 視野內沒節點 → 悠閒換點找。
    if (!waypoint || dist(waypoint[0], waypoint[1], S.x, S.y) < 40)
      waypoint = newWaypoint(VILLAGE[0], VILLAGE[1], 1000);
    stepToward(waypoint[0], waypoint[1]);
  }
}

function decideSocializer() {
  // 在城鎮 NPC / 廣場間走動。
  const npc = nearest(S.npcs);
  const center = npc ? [npc.x, npc.y] : VILLAGE;
  if (!waypoint || dist(waypoint[0], waypoint[1], S.x, S.y) < 50)
    waypoint = newWaypoint(center[0], center[1], 250); // 廣場小範圍踱步
  stepToward(waypoint[0], waypoint[1]);
  // 低頻罐頭聊天（≤1 句/30 秒）——絕不洗版。
  const now = Date.now();
  if (now - lastChatAt > 30000 && Math.random() < 0.25) {
    lastChatAt = now;
    const text = CANNED[Math.floor(Math.random() * CANNED.length)];
    send({ type: "chat", text });
    log(`聊天：「${text}」`);
  }
}

const DECIDE = { wanderer: decideWanderer, hunter: decideHunter, gatherer: decideGatherer, socializer: decideSocializer };

// ── 異常偵測 ────────────────────────────────────────────────────────────────
let lastPos = [0, 0];
let lastMovedAt = Date.now();
let stuckWarned = false;

async function behaviorLoop() {
  await sleep(2000); // 給連線/進場一點時間
  while (running) {
    if (S.ready) {
      try { DECIDE[PERSONA](); } catch (e) { log(`決策例外：${e.message}`); }

      // ③ 卡住偵測：自己位置長時間（>45s）幾乎沒變 → 記「卡住」。
      if (dist(S.x, S.y, lastPos[0], lastPos[1]) > 12) { lastPos = [S.x, S.y]; lastMovedAt = Date.now(); stuckWarned = false; }
      else if (Date.now() - lastMovedAt > 45000 && !stuckWarned) {
        log(`⚠️ 卡住：位置 (${S.x.toFixed(0)},${S.y.toFixed(0)}) 逾 45s 沒移動`);
        stuckWarned = true;
        waypoint = null; // 換航點試著脫困
      }
    }
    await sleep(rand(1000, 3000)); // 每 ~1-3 秒一個決策
  }
}

// ① 斷流偵測：連續 >20 秒沒收到 snapshot → 記「疑似伺服器卡死/斷流」（長住軟測的核心價值）。
function watchdogLoop() {
  setInterval(() => {
    if (!running) return;
    if (lastSnapAt && Date.now() - lastSnapAt > 20000 && !stalledWarned) {
      const gap = ((Date.now() - lastSnapAt) / 1000).toFixed(0);
      log(`⚠️ 疑似伺服器卡死/斷流：已 ${gap}s 沒收到 snapshot`);
      stalledWarned = true;
    }
  }, 5000);
}

// ④ 每 ~60 秒心跳：時間 / 位置 / 血 / 在線狀態。
function heartbeatLoop() {
  setInterval(() => {
    if (!running) return;
    const online = ws && ws.readyState === WebSocket.OPEN ? "在線" : "離線";
    const age = lastSnapAt ? ((Date.now() - lastSnapAt) / 1000).toFixed(0) : "?";
    log(`💓 心跳 pos=(${S.x.toFixed(0)},${S.y.toFixed(0)}) HP=${S.hp}/${S.maxHp} 乙太=${S.ether} ${online} snap數=${snapCount}(${age}s前) 重連=${reconnects}`);
  }, 60000);
}

// ── 優雅關閉 ────────────────────────────────────────────────────────────────
function shutdown(sig) {
  if (!running) return;
  running = false;
  log(`收到 ${sig}，優雅關閉…`);
  try { stopMoving(); } catch {}
  try { if (ws && ws.readyState === WebSocket.OPEN) ws.close(1000, "bye"); } catch {}
  setTimeout(() => process.exit(0), 600); // 留時間把 close frame 送出
}
process.on("SIGTERM", () => shutdown("SIGTERM"));
process.on("SIGINT", () => shutdown("SIGINT"));
process.on("uncaughtException", (e) => { log(`未捕例外：${e.stack || e.message}`); });

// ── 啟動 ────────────────────────────────────────────────────────────────────
log(`啟動長住居民 → ${URL}`);
connect();
watchdogLoop();
heartbeatLoop();
behaviorLoop();
