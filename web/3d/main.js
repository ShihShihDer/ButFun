// ButFun — /3d/ 可操控的真實世界 3D 客戶端
// 目標：維護者（手機也行）開這個網址 → 以一個「真正的玩家」身分加入**真實 ButFun 世界** →
//        用搖桿／WASD 操控自己的角色在 live 世界裡走動（旁邊有真實的 NPC、敵人、其他玩家、
//        節點地形）→ 第三人稱鏡頭跟著自己。從「繞著看」升級成「在裡面走」。
//
// 核心原則（與 2D 遊戲井水不犯河水）：
//   - 伺服器是 2D 權威：完全不改後端／world-core／web/game.js／play3d。
//     這頁只是「前端多一層讀同一份 WebSocket 快照來畫，並送 Input 表達移動意圖」。
//   - 加入／送 Input／找出自己，都是把 web/game.js 的對應協議「鏡像」成精簡版（不 import）：
//       · 加入：onopen 送 {type:"join", name, species}（訪客即可動，伺服器不擋訪客 Input）。
//       · 移動：搖桿／按鍵 → 換算成「相對鏡頭」的移動意圖 → 對應成世界四向布林 →
//               送 {type:"input", up,down,left,right,run}；自己的位置以伺服器回來的快照為準。
//       · 找出自己：welcome 給的 msg.id 當 myId，快照裡 players.find(p => p.id === myId)。
//   - 跳躍：伺服器是 2D 的，跳只是**前端視覺**（本地高度，cosmetic），不影響玩法。已標註。
//   - 沒風格的風格：實體全用盒子／膠囊／低多邊形程式生成，零美術資產。

import * as THREE from "three";

// ---- 錯誤浮層：任何例外不白屏，把訊息顯示出來（紅字）----
const errEl = document.getElementById("err");
function showErr(msg) {
  if (!errEl) return;
  errEl.style.display = "flex";
  errEl.textContent = "出錯了，但不白屏：\n" + msg;
}
window.addEventListener("error", (e) => showErr(e.message || String(e.error || e)));
window.addEventListener("unhandledrejection", (e) => showErr(String(e.reason)));

// ---- 浮層：狀態（連線／找不到自己）＋ HUD（線上人數／自己名字／提示）----
const statusEl = document.getElementById("status");
const hudEl = document.getElementById("hud");
function setStatus(text, isErr = false) {
  if (!statusEl) return;
  statusEl.textContent = text;
  statusEl.classList.toggle("err", !!isErr);
}

// ---- 世界座標 → 3D 場景座標 ----
// 後端世界是 6000×6000 像素（見 state.rs WORLD_WIDTH/HEIGHT），TILE_PX=32。
// Three.js 的 y 軸朝上，世界的 (x, y) 對應 3D 的 (x, z)：position = (x, 高度, y)。
// 縮放把 6000px 壓到約 300 個場景單位，並以世界中心為原點。
// 軸向對應（操控換算會用到）：3D +x = 世界 right(+x)；3D +z = 世界 down(+y)。
const WORLD_SCALE = 0.05;
let worldW = 6000, worldH = 6000; // 收到 welcome.world 會以權威值覆蓋
const worldCenter = { x: worldW / 2, y: worldH / 2 };
function sx(x) { return (x - worldCenter.x) * WORLD_SCALE; }
function sz(y) { return (y - worldCenter.y) * WORLD_SCALE; }

// ---- Three.js 基礎場景 ----
const app = document.getElementById("app");
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x0d1117);
scene.fog = new THREE.Fog(0x0d1117, 250, 600);

const camera = new THREE.PerspectiveCamera(58, window.innerWidth / window.innerHeight, 0.5, 2000);
camera.position.set(0, 60, 80);

const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setSize(window.innerWidth, window.innerHeight);
// 手機友善：pixelRatio 設上限，免得高 DPI 手機算爆 GPU
renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
app.appendChild(renderer.domElement);

// 燈光：半球光給柔和環境色 + 一盞方向光給立體感
scene.add(new THREE.HemisphereLight(0xbfd4ff, 0x20303a, 1.1));
const sun = new THREE.DirectionalLight(0xffffff, 1.2);
sun.position.set(120, 200, 80);
scene.add(sun);

// 地面：一塊草綠平面 + 格線，給尺度感
{
  const size = worldW * WORLD_SCALE;
  const ground = new THREE.Mesh(
    new THREE.PlaneGeometry(size, size),
    new THREE.MeshLambertMaterial({ color: 0x3f6b3f })
  );
  ground.rotation.x = -Math.PI / 2; // 平面預設在 XY，轉成水平地面（XZ）
  ground.position.y = -0.5;
  scene.add(ground);

  const grid = new THREE.GridHelper(size, 30, 0x2a4a2a, 0x2a4a2a);
  grid.position.y = -0.45;
  scene.add(grid);
}

// ---- 名字標籤（canvas 貼圖做成 sprite，浮在實體頭上）----
function makeLabel(text) {
  const canvas = document.createElement("canvas");
  canvas.width = 256; canvas.height = 64;
  const ctx = canvas.getContext("2d");
  ctx.font = "bold 28px system-ui, sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.lineWidth = 5; ctx.strokeStyle = "rgba(0,0,0,0.8)";
  ctx.strokeText(text, 128, 32);
  ctx.fillStyle = "#ffffff";
  ctx.fillText(text, 128, 32);
  const tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  const sprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: false }));
  sprite.scale.set(16, 4, 1);
  sprite.position.y = 9;
  return sprite;
}

// ---- 實體 mesh 工廠（低多邊形）----
const SELF_COLOR = 0xffd54a;     // 自己：金色膠囊
const PLAYER_COLOR = 0x4aa3ff;   // 其他玩家：藍色膠囊
const NPC_COLOR = 0xd8b070;      // NPC：暖棕盒子
const WILDLIFE_COLOR = 0x7fd87f; // 野生動物：綠色小盒
const ENEMY_COLOR = 0xff5a5a;    // 敵人：紅色盒子

function makeCapsule(color, h = 7) {
  const m = new THREE.Mesh(
    new THREE.CapsuleGeometry(2, h, 4, 8),
    new THREE.MeshLambertMaterial({ color })
  );
  m.position.y = h / 2 + 2;
  return m;
}
function makeBox(color, w, h, d) {
  const m = new THREE.Mesh(
    new THREE.BoxGeometry(w, h, d),
    new THREE.MeshLambertMaterial({ color })
  );
  m.position.y = h / 2;
  return m;
}

// 把一個「身體 mesh」包成帶座標內插狀態的 group，並可選擇加名字標籤。
function makeEntity(body, label) {
  const g = new THREE.Group();
  g.add(body);
  if (label) g.add(makeLabel(label));
  // 內插：tx/tz 是目標場景座標，render loop 每幀往目標 lerp，動起來才不頓
  g.userData.tx = g.position.x;
  g.userData.tz = g.position.z;
  scene.add(g);
  return g;
}

// 採集節點（樹／石／乙太礦）：靜態地形物，給世界一點「地形在」的實感
function makeNode(kind) {
  let mesh;
  if (kind === "tree") {
    mesh = new THREE.Mesh(new THREE.ConeGeometry(3, 9, 6), new THREE.MeshLambertMaterial({ color: 0x2f7d32 }));
    mesh.position.y = 4.5;
  } else if (kind === "ether_ore") {
    mesh = new THREE.Mesh(new THREE.OctahedronGeometry(3), new THREE.MeshLambertMaterial({ color: 0xb060ff }));
    mesh.position.y = 3;
  } else { // rock 與其他
    mesh = new THREE.Mesh(new THREE.DodecahedronGeometry(2.6), new THREE.MeshLambertMaterial({ color: 0x9aa0a6 }));
    mesh.position.y = 2.6;
  }
  const g = new THREE.Group();
  g.add(mesh);
  scene.add(g);
  return g;
}

// 各類實體用各自的 Map 追蹤（id → group），快照進來時 reconcile。
const players = new Map();
const npcs = new Map();
const wildlife = new Map();
const enemies = new Map();
const nodes = new Map(); // key 用座標字串（節點無穩定 id）

let myId = null;
let snapshotCount = 0;
let firstFollowDone = false;
let missingSelfWarned = false;

// 通用 reconcile：依快照陣列建立／更新／移除某一類實體。每筆都包 try/catch，
// 單筆資料壞掉不該讓整個 render 掛掉。
function reconcile(list, map, keyOf, create) {
  const seen = new Set();
  if (Array.isArray(list)) {
    for (const item of list) {
      try {
        if (typeof item.x !== "number" || typeof item.y !== "number") continue;
        const key = keyOf(item);
        seen.add(key);
        let g = map.get(key);
        if (!g) { g = create(item); map.set(key, g); }
        g.userData.tx = sx(item.x);
        g.userData.tz = sz(item.y);
      } catch (e) {
        console.warn("reconcile 單筆失敗，已略過", e);
      }
    }
  }
  for (const [key, g] of map) {
    if (!seen.has(key)) { scene.remove(g); map.delete(key); }
  }
}

// 收到一則伺服器訊息（鏡像 game.js 的 handleServerMsg，只取 3D 要用的欄位）。
function handleServerMsg(msg) {
  switch (msg.type) {
    case "welcome":
      // 自己的 id（用來把自己畫成金色＋鏡頭跟隨）＋ 世界尺寸（對齊地面與鏡頭）
      myId = msg.id;
      if (msg.world && typeof msg.world.width === "number") {
        worldW = msg.world.width; worldH = msg.world.height;
        worldCenter.x = worldW / 2; worldCenter.y = worldH / 2;
      }
      setStatus("已以玩家身分加入，等待世界快照…");
      break;
    case "snapshot": {
      snapshotCount++;
      // 玩家：膠囊（自己金色、別人藍色），帶名字標籤
      reconcile(
        msg.players, players,
        (p) => p.id,
        (p) => makeEntity(makeCapsule(p.id === myId ? SELF_COLOR : PLAYER_COLOR), p.name || "玩家")
      );
      // NPC（含居民／商人）：暖棕盒子，帶名字
      reconcile(
        msg.npcs, npcs,
        (n) => n.id,
        (n) => makeEntity(makeBox(NPC_COLOR, 4, 8, 4), n.name || "NPC")
      );
      // 野生動物：綠色小盒（不加標籤，避免太雜）
      reconcile(
        msg.wildlife, wildlife,
        (w) => "w" + w.id,
        () => makeEntity(makeBox(WILDLIFE_COLOR, 3, 3, 5))
      );
      // 敵人：紅色盒子；被打倒（alive=false）就當作消失移除
      reconcile(
        Array.isArray(msg.enemies) ? msg.enemies.filter((e) => e.alive !== false) : [],
        enemies,
        (e) => e.eid || (e.x + "_" + e.y),
        () => makeEntity(makeBox(ENEMY_COLOR, 4, 6, 4))
      );
      // 採集節點（樹／石／乙太礦）：以座標當 key（節點無穩定 id）
      reconcile(
        msg.nodes, nodes,
        (n) => n.kind + "@" + Math.round(n.x) + "," + Math.round(n.y),
        (n) => makeNode(n.kind)
      );

      // 找出「自己」：快照裡 id === myId 的那個玩家。沒找到就提示（不白屏）。
      const meGroup = myId ? players.get(myId) : null;
      if (meGroup) {
        // 第一份含自己的快照：把鏡頭直接吸到自己身上（之後交給跟隨鏡頭平滑跟）
        if (!firstFollowDone) {
          camera.position.set(meGroup.userData.tx, 60, meGroup.userData.tz + 70);
          firstFollowDone = true;
        }
        missingSelfWarned = false;
      } else if (myId && snapshotCount > 3 && !missingSelfWarned) {
        missingSelfWarned = true;
        setStatus("已連上，但在快照裡找不到自己（myId=" + myId + "）。\n世界仍在顯示，移動可能未生效。", true);
      }

      // HUD：線上人數／自己名字／操作提示
      const meItem = Array.isArray(msg.players) ? msg.players.find((p) => p.id === myId) : null;
      const myName = meItem ? (meItem.name || "玩家") : "（加入中…）";
      hudEl.innerHTML =
        `<b>${myName}</b> · 線上 ${players.size} 人\n` +
        `NPC ${npcs.size} · 野生 ${wildlife.size} · 敵人 ${enemies.size}\n` +
        `${isTouch ? "搖桿移動 · 右側拖曳轉鏡頭 · 跳鈕跳" : "WASD 移動 · 拖曳轉鏡頭 · 空白鍵跳"}`;

      setStatus(
        `真實世界已連上 · 快照 #${snapshotCount}` +
        (meGroup ? "（鏡頭跟著你）" : "")
      );
      break;
    }
    default:
      // 其他訊息類型（聊天、各種事件…）這頁不需要，忽略
      break;
  }
}

// ============================================================
// 操控：鍵盤 + 搖桿 → 相對鏡頭的移動意圖 → 世界四向布林 → 送 Input
// ============================================================
// inputKeys：目前要送給伺服器的方向意圖（與 game.js 的 input 協議同形）。
const inputKeys = { up: false, down: false, left: false, right: false, run: false };
let lastSentInput = "";        // 與 game.js 一樣：只在意圖改變時送，省流量
const heldKeys = Object.create(null); // 鍵盤方向意圖（up/down/left/right）
let runHeld = false;           // Shift 跑
const joy = { active: false, x: 0, y: 0 }; // 搖桿輸出 [-1,1]，x=右、y=下

// 鏡頭角度（第三人稱）：camYaw 一開始把鏡頭擺在 +z（世界往下方）看向自己。
let camYaw = 0;
let camPitch = 0.6;
const PITCH_MIN = 0.18, PITCH_MAX = 1.25;
let camDist = 70;
const DIST_MIN = 24, DIST_MAX = 160;

// 跳躍（純前端視覺，cosmetic）：只給自己的膠囊本地補一個垂直 bob，不影響伺服器 2D 玩法。
let jumpZ = 0;        // 視覺高度（場景單位）
let jumpV = 0;        // 視覺垂直速度
let wantJump = false; // 這一幀有沒有按跳
const JUMP_V = 26, JUMP_G = 70; // 視覺跳的初速／重力（純好看）

// ---- 鍵盤（桌機）----
function keyToDir(e) {
  switch (e.code) {
    case "KeyW": case "ArrowUp": return "up";
    case "KeyS": case "ArrowDown": return "down";
    case "KeyA": case "ArrowLeft": return "left";
    case "KeyD": case "ArrowRight": return "right";
  }
  return null;
}
window.addEventListener("keydown", (e) => {
  if (e.code === "Space") { wantJump = true; e.preventDefault(); return; }
  if (e.code === "ShiftLeft" || e.code === "ShiftRight") { runHeld = true; return; }
  const dir = keyToDir(e);
  if (dir) { heldKeys[dir] = true; e.preventDefault(); }
});
window.addEventListener("keyup", (e) => {
  if (e.code === "ShiftLeft" || e.code === "ShiftRight") { runHeld = false; return; }
  const dir = keyToDir(e);
  if (dir) heldKeys[dir] = false;
});

// ---- 滑鼠拖曳轉鏡頭（桌機）----
let dragging = false, lastMX = 0, lastMY = 0;
renderer.domElement.addEventListener("mousedown", (e) => {
  dragging = true; lastMX = e.clientX; lastMY = e.clientY;
});
window.addEventListener("mouseup", () => { dragging = false; });
window.addEventListener("mousemove", (e) => {
  if (!dragging) return;
  camYaw -= (e.clientX - lastMX) * 0.005;
  camPitch += (e.clientY - lastMY) * 0.005;
  camPitch = Math.max(PITCH_MIN, Math.min(PITCH_MAX, camPitch));
  lastMX = e.clientX; lastMY = e.clientY;
});
// 滾輪縮放（桌機）：調整第三人稱距離
renderer.domElement.addEventListener("wheel", (e) => {
  camDist = Math.max(DIST_MIN, Math.min(DIST_MAX, camDist + e.deltaY * 0.05));
  e.preventDefault();
}, { passive: false });

// ---- 觸控：偵測到觸控裝置就顯示搖桿/跳鈕（沿用 play3d 的控制法）----
const isTouch = ("ontouchstart" in window) || (navigator.maxTouchPoints > 0);
if (isTouch) {
  const touchUI = document.getElementById("touch");
  if (touchUI) touchUI.style.display = "block";

  // 左下虛擬搖桿
  const joyEl = document.getElementById("joy");
  const nubEl = document.getElementById("joyNub");
  let joyId = null;
  const JOY_R = 35; // 旋鈕可移動半徑（px）
  function joyStart(t) { joyId = t.identifier; joy.active = true; joyMove(t); }
  function joyMove(t) {
    const r = joyEl.getBoundingClientRect();
    let dx = t.clientX - (r.left + r.width / 2);
    let dy = t.clientY - (r.top + r.height / 2);
    const len = Math.hypot(dx, dy) || 1;
    const clamped = Math.min(len, JOY_R);
    const nx = (dx / len), ny = (dy / len);
    nubEl.style.left = (35 + nx * clamped) + "px";
    nubEl.style.top = (35 + ny * clamped) + "px";
    joy.x = nx * (clamped / JOY_R);
    joy.y = ny * (clamped / JOY_R);
  }
  function joyEnd() {
    joyId = null; joy.active = false; joy.x = 0; joy.y = 0;
    nubEl.style.left = "35px"; nubEl.style.top = "35px";
  }
  joyEl.addEventListener("touchstart", (e) => {
    e.preventDefault();
    if (joyId === null) joyStart(e.changedTouches[0]);
  }, { passive: false });
  joyEl.addEventListener("touchmove", (e) => {
    e.preventDefault();
    for (const t of e.changedTouches) if (t.identifier === joyId) joyMove(t);
  }, { passive: false });
  joyEl.addEventListener("touchend", (e) => {
    for (const t of e.changedTouches) if (t.identifier === joyId) joyEnd();
  });
  joyEl.addEventListener("touchcancel", joyEnd);

  // 右下跳躍鈕
  const jumpEl = document.getElementById("jump");
  jumpEl.addEventListener("touchstart", (e) => { e.preventDefault(); wantJump = true; }, { passive: false });

  // 右半邊拖曳轉鏡頭（避開搖桿/跳鈕；用 identifier 同時並存）
  let camId = null, camLX = 0, camLY = 0;
  function isOnUI(t) {
    const el = document.elementFromPoint(t.clientX, t.clientY);
    return el && (el.closest("#joy") || el.closest("#jump"));
  }
  window.addEventListener("touchstart", (e) => {
    if (camId !== null) return;
    for (const t of e.changedTouches) {
      if (isOnUI(t)) continue;
      camId = t.identifier; camLX = t.clientX; camLY = t.clientY; break;
    }
  }, { passive: true });
  window.addEventListener("touchmove", (e) => {
    for (const t of e.changedTouches) {
      if (t.identifier !== camId) continue;
      camYaw -= (t.clientX - camLX) * 0.006;
      camPitch += (t.clientY - camLY) * 0.006;
      camPitch = Math.max(PITCH_MIN, Math.min(PITCH_MAX, camPitch));
      camLX = t.clientX; camLY = t.clientY;
    }
  }, { passive: true });
  window.addEventListener("touchend", (e) => {
    for (const t of e.changedTouches) if (t.identifier === camId) camId = null;
  });
}

// 把搖桿／按鍵的「相對鏡頭移動意圖」換算成世界四向布林，並在改變時送 Input。
//   1) 收集 inF（前/後）inR（左/右），來自鍵盤與搖桿。
//   2) 用 camYaw 把它投影成 3D 世界平面方向 (mx, mz)。
//   3) 3D +x = 世界 right、3D +z = 世界 down(+y) → 換成 up/down/left/right 布林。
// 伺服器只認四向布林（每 tick 整合位置），故 8 向意圖會被量化成最接近的四向組合。
function updateInput() {
  let inF = 0, inR = 0;
  if (heldKeys.up) inF += 1;
  if (heldKeys.down) inF -= 1;
  if (heldKeys.right) inR += 1;
  if (heldKeys.left) inR -= 1;
  if (joy.active) { inF += -joy.y; inR += joy.x; } // 搖桿上＝前進
  const inLen = Math.hypot(inF, inR);
  if (inLen > 1) { inF /= inLen; inR /= inLen; }

  // 鏡頭朝向投影到地面：forward 指向鏡頭看出去的方向
  const fwdX = -Math.sin(camYaw), fwdZ = -Math.cos(camYaw);
  const rightX = -fwdZ, rightZ = fwdX;
  const mx = fwdX * inF + rightX * inR; // 3D x 分量（=世界 x）
  const mz = fwdZ * inF + rightZ * inR; // 3D z 分量（=世界 y）

  // 量化成四向布林（門檻避免微小漂移誤觸）
  const TH = 0.35;
  inputKeys.right = mx > TH;
  inputKeys.left = mx < -TH;
  inputKeys.down = mz > TH;   // 3D +z = 世界 +y = 「下」
  inputKeys.up = mz < -TH;    // 3D -z = 世界 -y = 「上」
  inputKeys.run = runHeld || (joy.active && inLen > 0.92); // 搖桿推到底＝跑

  // 只在意圖改變時送（鏡像 game.js sendInputIfChanged）
  const sig = `${inputKeys.up}${inputKeys.down}${inputKeys.left}${inputKeys.right}${inputKeys.run}`;
  if (sig !== lastSentInput && ws && ws.readyState === WebSocket.OPEN) {
    lastSentInput = sig;
    ws.send(JSON.stringify({ type: "input", ...inputKeys }));
  }
}

// ============================================================
// WebSocket 連線（鏡像 game.js：same-origin、/ws、join 後送 Input）
// ============================================================
let ws = null;
let reconnectTimer = null;
let reconnectAttempts = 0;

function connect() {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  try {
    ws = new WebSocket(`${proto}://${location.host}/ws`);
  } catch (e) {
    setStatus("無法建立 WebSocket 連線：" + e.message, true);
    scheduleReconnect();
    return;
  }

  ws.onopen = () => {
    reconnectAttempts = 0;
    // 以玩家身分加入（訪客即可動：伺服器不擋訪客 Input）。名字／物種只是顯示用。
    ws.send(JSON.stringify({ type: "join", name: "3D玩家", species: "terran" }));
    setStatus("已加入，等待世界快照…");
    // 重連後清掉上次的 input 簽章，下一幀 updateInput 會把意圖重送給新連線
    lastSentInput = "";
  };

  ws.onmessage = (ev) => {
    let msg;
    try { msg = JSON.parse(ev.data); }
    catch { return; } // 後端有些世界事件走純字串（非 JSON），忽略
    try { handleServerMsg(msg); }
    catch (e) { console.warn("handleServerMsg 失敗", e); }
  };

  ws.onclose = () => {
    setStatus("與伺服器連線中斷，重連中…", true);
    scheduleReconnect();
  };
  ws.onerror = () => { try { ws.close(); } catch {} };
}

// 指數退避重連（鏡像 game.js 精簡版）：0.5s、1s、2s…上限 8s
function scheduleReconnect() {
  if (reconnectTimer !== null) return;
  const delay = Math.min(8000, 500 * Math.pow(2, reconnectAttempts));
  reconnectAttempts++;
  reconnectTimer = setTimeout(() => { reconnectTimer = null; connect(); }, delay);
}

// ============================================================
// Render loop
// ============================================================
const clock = new THREE.Clock();
function lerpEntities(map, k, skipKey) {
  for (const [key, g] of map) {
    if (skipKey !== undefined && key === skipKey) continue; // 自己另外處理（要疊視覺跳）
    g.position.x += (g.userData.tx - g.position.x) * k;
    g.position.z += (g.userData.tz - g.position.z) * k;
  }
}

function safeRender() {
  requestAnimationFrame(safeRender);
  try {
    const dt = Math.min(0.05, clock.getDelta());
    // 每幀把目前操控意圖換算並（在改變時）送出
    updateInput();

    // 視覺跳（cosmetic）：只動自己膠囊的本地高度，不送伺服器
    if (wantJump && jumpZ <= 0.01) { jumpV = JUMP_V; }
    wantJump = false;
    if (jumpV !== 0 || jumpZ > 0) {
      jumpV -= JUMP_G * dt;
      jumpZ += jumpV * dt;
      if (jumpZ < 0) { jumpZ = 0; jumpV = 0; }
    }

    // 內插係數隨幀時間調整，不同更新率都平滑（採目標 ~8/s 收斂）
    const k = Math.min(1, dt * 8);
    const meGroup = myId ? players.get(myId) : null;
    lerpEntities(players, k, myId);
    lerpEntities(npcs, k);
    lerpEntities(wildlife, k);
    lerpEntities(enemies, k);
    lerpEntities(nodes, 1); // 節點靜態：直接吸附

    // 自己：位置以伺服器快照為準（內插），再疊上視覺跳的高度
    if (meGroup) {
      meGroup.position.x += (meGroup.userData.tx - meGroup.position.x) * k;
      meGroup.position.z += (meGroup.userData.tz - meGroup.position.z) * k;
      meGroup.position.y = jumpZ;

      // 第三人稱跟隨鏡頭：在自己後方、平滑 lerp
      const cx = Math.sin(camYaw) * Math.cos(camPitch) * camDist;
      const cz = Math.cos(camYaw) * Math.cos(camPitch) * camDist;
      const cy = Math.sin(camPitch) * camDist + 8;
      const tx = meGroup.position.x, ty = meGroup.position.y + 6, tz = meGroup.position.z;
      const desired = new THREE.Vector3(tx + cx, ty + cy, tz + cz);
      camera.position.lerp(desired, Math.min(1, dt * 6));
      camera.lookAt(tx, ty, tz);
    }

    renderer.render(scene, camera);
  } catch (e) {
    // 護網：render 任一例外不該永久凍住畫面（rAF 已先排好下一幀）
    console.error("render 例外，已跳過本幀", e);
    showErr(String(e && e.message ? e.message : e));
  }
}

window.addEventListener("resize", () => {
  camera.aspect = window.innerWidth / window.innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(window.innerWidth, window.innerHeight);
});

// 啟動
setStatus("連線中…");
connect();
safeRender();
