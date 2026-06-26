// ButFun — /3d/ Three.js spike
// 目標：維護者開一個網址，立刻看到「現在這個 2D 世界」在 3D 裡動（玩家／NPC 在跑、地形在）。
// 求「看得到、會動」，不求完整或精緻。
//
// 核心原則（與 2D 遊戲井水不犯河水）：
//   - 完全不改後端、不改 world-core、不改 web/game.js。後端是 2D 權威，
//     3D 只是「前端多一層讀同一份 WebSocket 快照來畫」。Z（高度）在前端補。
//   - 連線/加入/解析快照的邏輯，是把 web/game.js 的對應段「鏡像」成精簡唯讀版
//     （不 import 它、自己寫），只連線觀察、不送任何輸入。
//   - 沒風格的風格：實體全用盒子／膠囊／低多邊形程式生成，零美術資產。

import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";

// ---- 狀態浮層（連不上 / 沒資料時要看得到字，不白屏）----
const statusEl = document.getElementById("status");
function setStatus(text, isErr = false) {
  if (!statusEl) return;
  statusEl.textContent = text;
  statusEl.classList.toggle("err", !!isErr);
}

// ---- 世界座標 → 3D 場景座標 ----
// 後端世界是 6000×6000 像素（見 state.rs WORLD_WIDTH/HEIGHT），TILE_PX=32。
// Three.js 的 y 軸朝上，世界的 (x, y) 對應 3D 的 (x, z)：position = (x, 高度, y)。
// 縮放把 6000px 壓到約 300 個場景單位，並以世界中心為原點（鏡頭好擺、地面好對齊）。
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

const camera = new THREE.PerspectiveCamera(55, window.innerWidth / window.innerHeight, 0.5, 2000);
camera.position.set(0, 160, 200);

const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setSize(window.innerWidth, window.innerHeight);
// 手機友善：pixelRatio 設上限，免得高 DPI 手機算爆 GPU
renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
app.appendChild(renderer.domElement);

// OrbitControls：讓維護者用滑鼠／手指繞著看（手機觸控可轉可縮）
const controls = new OrbitControls(camera, renderer.domElement);
controls.enableDamping = true;
controls.dampingFactor = 0.08;
controls.maxPolarAngle = Math.PI * 0.49; // 不要轉到地平線以下
controls.target.set(0, 0, 0);

// 燈光：半球光給柔和環境色 + 一盞方向光給立體感
scene.add(new THREE.HemisphereLight(0xbfd4ff, 0x20303a, 1.1));
const sun = new THREE.DirectionalLight(0xffffff, 1.2);
sun.position.set(120, 200, 80);
scene.add(sun);

// 地面：一塊草綠平面 + 格線，給尺度感（biome 上色 spike 階段先略，保持簡單）
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
// 各類用不同形狀／顏色，一眼分得出玩家／NPC／野生動物／敵人／採集節點。
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
let centeredOnce = false;

// 通用 reconcile：依快照陣列建立／更新／移除某一類實體。
//   list      ：快照裡的陣列
//   map       ：對應的 Map
//   keyOf     ：算每筆的 key
//   create    ：(item) => group（新實體首次出現時造 mesh）
//   每筆都包 try/catch：單筆資料壞掉不該讓整個 render 掛掉。
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
        // 更新內插目標（位置平滑由 render loop 完成）
        g.userData.tx = sx(item.x);
        g.userData.tz = sz(item.y);
      } catch (e) {
        // 防呆：略過這一筆，繼續處理其他實體
        console.warn("reconcile 單筆失敗，已略過", e);
      }
    }
  }
  // 移除這次快照沒出現的（實體消失 → 移除 mesh）
  for (const [key, g] of map) {
    if (!seen.has(key)) { scene.remove(g); map.delete(key); }
  }
}

// 收到一則伺服器訊息（鏡像 game.js 的 handleServerMsg，只取 3D 要用的欄位）。
function handleServerMsg(msg) {
  switch (msg.type) {
    case "welcome":
      // 自己的 id（用來把自己畫成金色）＋ 世界尺寸（對齊地面與鏡頭）
      myId = msg.id;
      if (msg.world && typeof msg.world.width === "number") {
        worldW = msg.world.width; worldH = msg.world.height;
        worldCenter.x = worldW / 2; worldCenter.y = worldH / 2;
      }
      setStatus("已連上，等待世界快照…");
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
        (n) => { const g = makeNode(n.kind); return g; }
      );

      // 首次收到有玩家的快照時，把鏡頭對準玩家群的重心（之後交給使用者自由繞看）
      if (!centeredOnce && players.size > 0) {
        let cx = 0, cz = 0, k = 0;
        for (const g of players.values()) { cx += g.userData.tx; cz += g.userData.tz; k++; }
        if (k > 0) {
          controls.target.set(cx / k, 0, cz / k);
          camera.position.set(cx / k, 140, cz / k + 160);
          centeredOnce = true;
        }
      }

      const total = players.size + npcs.size + wildlife.size + enemies.size;
      setStatus(
        `世界已連上 · 快照 #${snapshotCount}\n` +
        `玩家 ${players.size} · NPC ${npcs.size} · 野生 ${wildlife.size} · 敵人 ${enemies.size} · 節點 ${nodes.size}` +
        (total === 0 ? "\n（目前世界上沒有可見實體，地形仍在）" : "")
      );
      break;
    }
    default:
      // 其他訊息類型（聊天、各種事件…）3D spike 不需要，忽略
      break;
  }
}

// ---- WebSocket 連線（鏡像 game.js：same-origin、/ws、訪客即連觀察）----
// game.js 的連線流程：proto 視 https 決定 wss/ws → new WebSocket(`${proto}://${host}/ws`)
//   → onopen 送 {type:"join", name, species}。這裡走最省事的訪客路徑、只觀察、不送輸入。
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
    // 訪客身分加入、唯讀觀察（伺服器允許訪客即連，名字／物種只是顯示用）
    ws.send(JSON.stringify({ type: "join", name: "3D觀察者", species: "terran" }));
    setStatus("已加入，等待世界快照…");
  };

  ws.onmessage = (ev) => {
    let msg;
    try {
      msg = JSON.parse(ev.data);
    } catch {
      // 後端有些世界事件走純字串（非 JSON），3D spike 直接忽略
      return;
    }
    try {
      handleServerMsg(msg);
    } catch (e) {
      // 韌性：處理單則訊息出錯不該讓連線或 render 掛掉
      console.warn("handleServerMsg 失敗", e);
    }
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

// ---- Render loop ----
const clock = new THREE.Clock();
function lerpEntities(map, k) {
  for (const g of map.values()) {
    g.position.x += (g.userData.tx - g.position.x) * k;
    g.position.z += (g.userData.tz - g.position.z) * k;
  }
}

function safeRender() {
  requestAnimationFrame(safeRender);
  try {
    const dt = clock.getDelta();
    // 內插係數隨幀時間調整，不同更新率都平滑（採目標 ~8/s 收斂）
    const k = Math.min(1, dt * 8);
    lerpEntities(players, k);
    lerpEntities(npcs, k);
    lerpEntities(wildlife, k);
    lerpEntities(enemies, k);
    // 節點是靜態地形物：直接吸附到目標座標（k=1），新節點進來立即就位
    lerpEntities(nodes, 1);
    controls.update();
    renderer.render(scene, camera);
  } catch (e) {
    // 護網：render 任一例外不該永久凍住畫面（rAF 已先排好下一幀）
    console.error("render 例外，已跳過本幀", e);
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
