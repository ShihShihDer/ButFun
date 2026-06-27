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

// ============================================================
// Netcode 調校常數（現代做法，全部可調）
// ============================================================
// 後端 15Hz 廣播權威快照（~66ms 一筆）。直接「lerp 追最新位置」會抽動／過衝，
// 別人看起來瞬移亂飛、自己走起來鈍。下面三招是業界標準做法（自己寫碼，不抄外部碼）：
//
// 1) 實體內插（entity interpolation, render-in-the-past）— 給「別人」用
//    每個遠端實體緩衝最近數筆快照（位置＋到達時間），渲染時取「現在 − 內插延遲」
//    這個時間點，在包夾它的兩筆快照之間線性內插 → 平滑、不過衝、不抽動。
const INTERP_DELAY_MS = 100;   // 內插延遲（render-in-past），約 1.5 個 15Hz 間隔
const SNAP_BUFFER_MAX = 12;    // 每個遠端實體保留的快照樣本上限（夠覆蓋延遲即可）

// 2) 自身客戶端預測 + 對帳（client-side prediction + reconciliation）— 給「自己」用
//    輸入時本地立即往同方向推（零延遲手感）；權威快照回來時平滑拉回（不硬瞬移）。
//    後端 PLAYER_SPEED=180px/s × WORLD_SCALE(0.05) = 9 場景單位/秒。
const PREDICT_SCENE_SPEED = 9; // 自身預測速度（場景單位/秒），會由權威快照自我校準
const RECONCILE_RATE = 5;      // 小誤差柔和拉回速率（每秒，越大越快貼回權威）
const RECONCILE_SNAP_DIST = 50;// 誤差超過此距離（場景單位，視為瞬移／重生）→ 快速拉回
const RECONCILE_SNAP_RATE = 18;// 大誤差時的快速拉回速率（仍平滑，不硬瞬移）

// 3) AOI 進出淡入淡出 — 實體第一次出現淡入、離開快照淡出再移除，不啪一下彈出/消失。
const FADE_RATE = 6;           // 淡入淡出速率（每秒，1-exp 收斂）

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
  sprite.position.y = 12; // 浮在火柴人頭頂之上（頭頂約 y=10.6）
  return sprite;
}

// ---- 實體 mesh 工廠（低多邊形，零美術資產）----
const SELF_COLOR = 0xffd54a;     // 自己：金色火柴人
const PLAYER_COLOR = 0x4aa3ff;   // 其他玩家：藍色火柴人
const NPC_COLOR = 0xd8b070;      // NPC／居民：暖棕火柴人
const WILDLIFE_COLOR = 0x7fd87f; // 野生動物：綠色小盒（盒子，跟「人」一眼區分）
const ENEMY_COLOR = 0xff5a5a;    // 敵人：紅色盒子（盒子，跟「人」一眼區分）

// ---- 程序化火柴人（stickman）----
// 人形＝純幾何組裝：球當頭、膠囊當軀幹、細圓柱當四肢，零美術資產。
// 套用對象＝玩家（自己＋別人）＋ NPC／居民；敵人／野生動物／節點維持盒子等好區分。
//
// 比例（場景單位，可調）。腳底約落在 y=0（站在地面），整體高度與舊膠囊相近。
const SK = {
  HIP_Y: 4.6, THIGH_LEN: 2.4, SHIN_LEN: 2.2, HIP_HALF_W: 0.7,
  SHOULDER_Y: 7.8, SHOULDER_HALF_W: 1.35, UPPER_ARM_LEN: 2.0, FORE_ARM_LEN: 1.9,
  TORSO_Y: 6.2, HEAD_Y: 9.3, HEAD_R: 1.3,
};

// 共用幾何（全模組只建一次 → 幾十個火柴人也不重建頂點、不爆記憶體）。
// 圓柱預設沿 +Y、以原點為中心；做四肢時讓 mesh 往下沉半截，關節樞紐就落在「上端」。
const SK_GEO = {
  thigh: new THREE.CylinderGeometry(0.50, 0.45, SK.THIGH_LEN, 6),
  shin:  new THREE.CylinderGeometry(0.45, 0.34, SK.SHIN_LEN, 6),
  upper: new THREE.CylinderGeometry(0.42, 0.38, SK.UPPER_ARM_LEN, 6),
  fore:  new THREE.CylinderGeometry(0.38, 0.30, SK.FORE_ARM_LEN, 6),
  torso: new THREE.CapsuleGeometry(1.0, 1.6, 3, 6),
  head:  new THREE.SphereGeometry(SK.HEAD_R, 10, 8),
};

// 一節肢體：回傳一個「樞紐 group」（樞紐在關節處），子 mesh 往下掛半截。
// 旋轉樞紐的 rotation.x 就能讓這節肢體前後擺。
function skSegment(geo, mat, len) {
  const pivot = new THREE.Group();
  const mesh = new THREE.Mesh(geo, mat);
  mesh.position.y = -len / 2; // 上端對齊樞紐原點
  pivot.add(mesh);
  return pivot;
}

// 組一隻火柴人：回傳一個 group，內含可動關節（大腿/小腿/上臂/前臂/頭/軀幹）放 userData.joints
// 供走路動畫驅動。共用幾何省效能；材質「每隻一份」（單色）——這樣 AOI 淡入淡出能各自獨立調 opacity。
function makeStickman(color) {
  const g = new THREE.Group();
  const mat = new THREE.MeshLambertMaterial({ color });

  // 軀幹 + 頭（直接掛在 group 上，固定不動）
  const torso = new THREE.Mesh(SK_GEO.torso, mat); torso.position.y = SK.TORSO_Y;
  const head = new THREE.Mesh(SK_GEO.head, mat); head.position.y = SK.HEAD_Y;
  g.add(torso, head);

  // 腿：大腿樞紐在髖部，小腿樞紐掛在大腿下端（膝蓋）
  function leg(sign) {
    const thigh = skSegment(SK_GEO.thigh, mat, SK.THIGH_LEN);
    thigh.position.set(sign * SK.HIP_HALF_W, SK.HIP_Y, 0);
    const shin = skSegment(SK_GEO.shin, mat, SK.SHIN_LEN);
    shin.position.y = -SK.THIGH_LEN; // 膝蓋＝大腿下端
    thigh.add(shin);
    g.add(thigh);
    return { thigh, shin };
  }
  // 臂：上臂樞紐在肩，前臂樞紐掛在上臂下端（手肘）
  function arm(sign) {
    const upper = skSegment(SK_GEO.upper, mat, SK.UPPER_ARM_LEN);
    upper.position.set(sign * SK.SHOULDER_HALF_W, SK.SHOULDER_Y, 0);
    const fore = skSegment(SK_GEO.fore, mat, SK.FORE_ARM_LEN);
    fore.position.y = -SK.UPPER_ARM_LEN; // 手肘＝上臂下端
    upper.add(fore);
    g.add(upper);
    return { upper, fore };
  }
  const legL = leg(1), legR = leg(-1);
  const armL = arm(1), armR = arm(-1);

  g.userData.isStickman = true;
  g.userData.joints = {
    torso, head,
    legL_thigh: legL.thigh, legL_shin: legL.shin,
    legR_thigh: legR.thigh, legR_shin: legR.shin,
    armL_upper: armL.upper, armL_fore: armL.fore,
    armR_upper: armR.upper, armR_fore: armR.fore,
  };
  g.userData.phase = Math.random() * 6.28; // 各自相位，整群不會整齊劃一
  g.userData.walkW = 0;                     // 走路權重（平滑進出站姿）
  return g;
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
  // 火柴人：記住身體 group，走路動畫要驅動它的關節（盒子實體沒有，會被跳過）
  if (body && body.userData && body.userData.isStickman) g.userData.stick = body;
  if (label) g.add(makeLabel(label));
  // tx/tz：最新一筆快照的目標場景座標（內插資料不足時的 lerp 退路）
  g.userData.tx = g.position.x;
  g.userData.tz = g.position.z;
  initNetState(g);
  scene.add(g);
  return g;
}

// 初始化一個 group 的 netcode 狀態：快照緩衝 + 淡入淡出。
function initNetState(g) {
  g.userData.buf = [];          // 快照樣本緩衝 {t,x,z}，給 render-in-past 內插
  g.userData.fade = 0;          // 淡入淡出當前值（0=透明且縮小，1=完整）
  g.userData.fadeTarget = 1;    // 目標：新生 → 淡入到 1
  g.userData.removing = false;  // 離開 AOI 時設 true，淡出完才真正移除
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
  g.userData.tx = g.position.x;
  g.userData.tz = g.position.z;
  initNetState(g); // 節點也走 AOI 淡入淡出（靜態，不做內插/轉身/起伏）
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

// 自身客戶端預測 + 對帳的狀態（場景座標）：權威座標來自快照，自己的 mesh 平滑拉回它。
let selfAuthX = 0, selfAuthZ = 0;         // 最新權威座標（場景單位）
let selfHasAuth = false;                  // 是否已收到過自己的權威座標
let lastSelfAuthT = 0;                     // 上一筆自己權威快照的到達時間（performance.now）
let selfMeasuredSpeed = PREDICT_SCENE_SPEED; // 實測自身速度（場景單位/秒），由快照自我校準

// 通用 reconcile：依快照陣列建立／更新／淡出某一類實體。每筆都包 try/catch，
// 單筆資料壞掉不該讓整個 render 掛掉。
// recvT＝這份快照的到達時間（performance.now，全類共用一個值，時間軸才一致），
// 內插用它把每筆位置標上時間戳。
function reconcile(list, map, keyOf, create, recvT) {
  const seen = new Set();
  if (Array.isArray(list)) {
    for (const item of list) {
      try {
        if (typeof item.x !== "number" || typeof item.y !== "number") continue;
        const key = keyOf(item);
        seen.add(key);
        const tx = sx(item.x), tz = sz(item.y);
        let g = map.get(key);
        if (!g) {
          // 新生：就地出現在第一筆位置（別從原點飛入），並啟動淡入
          g = create(item); map.set(key, g);
          g.position.x = tx; g.position.z = tz;
          g.userData.fade = 0; g.userData.fadeTarget = 1; g.userData.removing = false;
        } else if (g.userData.removing) {
          // 曾離開 AOI 正在淡出、現在又回來 → 取消移除、重新淡入
          g.userData.removing = false; g.userData.fadeTarget = 1;
        }
        g.userData.tx = tx;
        g.userData.tz = tz;
        // 推一筆帶時間戳的快照樣本進緩衝（render-in-past 內插用）
        const buf = g.userData.buf || (g.userData.buf = []);
        buf.push({ t: recvT, x: tx, z: tz });
        if (buf.length > SNAP_BUFFER_MAX) buf.shift();
      } catch (e) {
        console.warn("reconcile 單筆失敗，已略過", e);
      }
    }
  }
  // 沒在這份快照出現的 → 標記淡出（不立即刪），淡完才在 render 移除（AOI 不啪一下消失）
  for (const [key, g] of map) {
    if (!seen.has(key) && !g.userData.removing) {
      g.userData.removing = true;
      g.userData.fadeTarget = 0;
    }
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
      // 這份快照的到達時間：全類共用一個時間戳，內插時間軸才一致。
      const recvT = performance.now();
      // 玩家：火柴人（自己金色、別人藍色），帶名字標籤
      reconcile(
        msg.players, players,
        (p) => p.id,
        (p) => makeEntity(makeStickman(p.id === myId ? SELF_COLOR : PLAYER_COLOR), p.name || "玩家"),
        recvT
      );
      // NPC（含居民／商人）：暖棕火柴人，帶名字
      reconcile(
        msg.npcs, npcs,
        (n) => n.id,
        (n) => makeEntity(makeStickman(NPC_COLOR), n.name || "NPC"),
        recvT
      );
      // 野生動物：綠色小盒（不加標籤，避免太雜）
      reconcile(
        msg.wildlife, wildlife,
        (w) => "w" + w.id,
        () => makeEntity(makeBox(WILDLIFE_COLOR, 3, 3, 5)),
        recvT
      );
      // 敵人：紅色盒子；被打倒（alive=false）就當作消失移除
      reconcile(
        Array.isArray(msg.enemies) ? msg.enemies.filter((e) => e.alive !== false) : [],
        enemies,
        (e) => e.eid || (e.x + "_" + e.y),
        () => makeEntity(makeBox(ENEMY_COLOR, 4, 6, 4)),
        recvT
      );
      // 採集節點（樹／石／乙太礦）：以座標當 key（節點無穩定 id）
      reconcile(
        msg.nodes, nodes,
        (n) => n.kind + "@" + Math.round(n.x) + "," + Math.round(n.y),
        (n) => makeNode(n.kind),
        recvT
      );

      // 自己的權威座標：給客戶端預測對帳用；順便用相鄰快照實測移動速度自我校準
      // （含跑步／加速／載具，全自動適應，不必硬寫死）。
      const meAuth = Array.isArray(msg.players) ? msg.players.find((p) => p.id === myId) : null;
      if (meAuth && typeof meAuth.x === "number" && typeof meAuth.y === "number") {
        const nx = sx(meAuth.x), nz = sz(meAuth.y);
        const moving = inputKeys.up || inputKeys.down || inputKeys.left || inputKeys.right;
        if (selfHasAuth && moving) {
          const dtSnap = (recvT - lastSelfAuthT) / 1000;
          if (dtSnap > 0.001) {
            const obs = Math.hypot(nx - selfAuthX, nz - selfAuthZ) / dtSnap;
            // 合理範圍才採信（過濾瞬移／傳送造成的離群值），EMA 平滑校準
            if (obs > 0.5 && obs < 60) selfMeasuredSpeed += (obs - selfMeasuredSpeed) * 0.25;
          }
        }
        selfAuthX = nx; selfAuthZ = nz; selfHasAuth = true; lastSelfAuthT = recvT;
      }

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
let camPitch = 0.55;
const PITCH_MIN = 0.18, PITCH_MAX = 1.25;
let camDist = 48;     // 預設拉近一點，角色佔畫面比例像 Genshin 的第三人稱（原 70 太遠太小）
const DIST_MIN = 22, DIST_MAX = 150;

// 跳躍（純前端視覺，cosmetic）：只給自己的膠囊本地補一個垂直 bob，不影響伺服器 2D 玩法。
let jumpZ = 0;        // 視覺高度（場景單位）
let jumpV = 0;        // 視覺垂直速度
let wantJump = false; // 這一幀有沒有按跳
const JUMP_V = 28, JUMP_G = 82; // 視覺跳的初速／重力（調成俐落一點的拋物線，純好看）

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
    // deadzone + 類比 magnitude：小幅晃動不誤觸；推到底才觸發 run（見 updateInput）
    let mag = clamped / JOY_R;
    const DEAD = 0.18;
    mag = mag < DEAD ? 0 : (mag - DEAD) / (1 - DEAD);
    joy.x = nx * mag;
    joy.y = ny * mag;
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

// ---- 程序化走路動畫常數（純視覺，可調）----
// sine 波驅動關節擺動：大腿前後擺、手臂反相擺、膝/肘自然彎，相位隨移動速度推進。
const GAIT = {
  THIGH: 0.85,  // 大腿前後擺幅（弧度）
  KNEE: 0.9,    // 小腿（膝蓋）彎曲幅度
  ARM: 0.7,     // 上臂擺幅（與同側腿反相）
  ELBOW: 0.4,   // 前臂（手肘）彎曲幅度
  LEAN: 0.08,   // 走路時整體略前傾（弧度）
  FREQ: 0.85,   // 相位推進係數（× 移動速度 → 走越快步頻越快）
  IDLE_FREQ: 0.6, // 站定時殘留的最小推進，讓關節平滑收回站姿
};

// 程序化走路：用 sine 波擺動火柴人關節。speed＝場景單位/秒；停下時平滑回站姿。
// 盒子實體（無 stick）直接 return，不受影響。
function animateStickman(g, speed, dt) {
  const stick = g.userData.stick;
  if (!stick) return;
  const j = stick.userData.joints;
  // 走路權重平滑進出（避免一停就僵、一動就跳）
  const moving = speed > 0.6 ? 1 : 0;
  stick.userData.walkW += (moving - stick.userData.walkW) * Math.min(1, dt * 6);
  const w = stick.userData.walkW;
  // 相位隨移動距離推進；殘留一點最小步頻讓站定也能把擺幅平滑歸零
  stick.userData.phase += (speed * GAIT.FREQ + GAIT.IDLE_FREQ) * dt;
  const ph = stick.userData.phase;
  const s = Math.sin(ph);
  // 腿：左右反相前後擺；膝蓋只往一個方向彎（clamp）→ 抬腿那側自然屈膝
  j.legL_thigh.rotation.x = s * GAIT.THIGH * w;
  j.legR_thigh.rotation.x = -s * GAIT.THIGH * w;
  j.legL_shin.rotation.x = Math.max(0, -s) * GAIT.KNEE * w;
  j.legR_shin.rotation.x = Math.max(0, s) * GAIT.KNEE * w;
  // 手臂：與同側腿反相擺；前臂微彎（帶一點常態屈肘）
  j.armL_upper.rotation.x = -s * GAIT.ARM * w;
  j.armR_upper.rotation.x = s * GAIT.ARM * w;
  j.armL_fore.rotation.x = (Math.max(0, s) * GAIT.ELBOW + 0.12) * w;
  j.armR_fore.rotation.x = (Math.max(0, -s) * GAIT.ELBOW + 0.12) * w;
  // 走路時整體略前傾（樞紐在腳底附近），停下回正
  stick.rotation.x = GAIT.LEAN * w;
}

// 朝移動方向平滑轉身 + 走動起伏（讓角色不再僵硬滑行，呈現層手感參考成熟第三人稱）。
// 只動「呈現」：rotation 與本地 y bob，完全不碰伺服器權威的 x/z 位置。
function faceAndBob(g, dx, dz, dt, t) {
  const speed = Math.hypot(dx, dz) / Math.max(dt, 1e-3); // 場景單位/秒
  if (speed > 0.6) {
    const target = Math.atan2(dx, dz); // 朝實際移動方向
    let d = target - g.rotation.y;
    while (d > Math.PI) d -= Math.PI * 2;
    while (d < -Math.PI) d += Math.PI * 2;
    g.rotation.y += d * Math.min(1, dt * 10); // 平滑轉，不瞬間甩頭
  }
  // 走路時上下微彈，停下淡出（bob 權重平滑過渡，避免一停就僵）
  const moving = speed > 0.6 ? 1 : 0;
  g.userData.bobW = (g.userData.bobW || 0) + (moving - (g.userData.bobW || 0)) * Math.min(1, dt * 6);
  if (g.userData.phase === undefined) g.userData.phase = Math.random() * 6.28; // 各自相位，不會整齊劃一
  g.position.y = Math.abs(Math.sin(t * 9 + g.userData.phase)) * 0.7 * g.userData.bobW;
  // 火柴人：sine 波走路（盒子無 stick 會被跳過）
  animateStickman(g, speed, dt);
}

// 在快照緩衝裡取 renderTime（= 現在 − 內插延遲）這個時間點的位置：
// 找包夾它的兩筆樣本線性內插（render-in-past）。找不到包夾回傳 bracketed:false，
// 呼叫端就退回原本的「lerp 追最新」行為（資料不足時不 throw、不亂飛）。
function sampleBufferAt(buf, renderTime) {
  if (!buf || buf.length === 0) return null;
  if (buf.length === 1) return { x: buf[0].x, z: buf[0].z, bracketed: false };
  // renderTime 比最舊樣本還舊（剛出現、歷史不足）→ 資料不足
  if (renderTime <= buf[0].t) return { x: buf[0].x, z: buf[0].z, bracketed: false };
  const last = buf[buf.length - 1];
  // renderTime 比最新樣本還新（斷線／離開 AOI、沒新資料）→ 資料不足（飢餓）
  if (renderTime >= last.t) return { x: last.x, z: last.z, bracketed: false };
  for (let i = 0; i < buf.length - 1; i++) {
    const a = buf[i], b = buf[i + 1];
    if (renderTime >= a.t && renderTime <= b.t) {
      const span = b.t - a.t;
      const f = span > 1e-6 ? (renderTime - a.t) / span : 0;
      return { x: a.x + (b.x - a.x) * f, z: a.z + (b.z - a.z) * f, bracketed: true };
    }
  }
  return { x: last.x, z: last.z, bracketed: false };
}

// 把淡入淡出推進一格並套到 mesh：縮放（pop 感）＋ 透明度。
// 回傳 true 表示「已淡出完畢、可移除」。
function updateFade(g, dt) {
  const cur = g.userData.fade ?? 1;
  const tgt = g.userData.fadeTarget ?? 1;
  const nf = cur + (tgt - cur) * Math.min(1, dt * FADE_RATE);
  g.userData.fade = nf;
  const sc = 0.55 + 0.45 * nf;       // 淡入時從小長到正常，淡出時縮回（柔和不啪一下）
  g.scale.setScalar(sc);
  // 遞迴調所有子 mesh/sprite 透明度（火柴人是巢狀群組，材質藏在多層深處）。
  // 材質「每隻實體一份」，所以這裡改 opacity 只影響本實體、不會牽連同色的別人。
  g.traverse((obj) => {
    const mat = obj.material;
    if (!mat) return;
    if (!mat.transparent) mat.transparent = true;
    mat.opacity = nf;
  });
  return g.userData.removing && nf < 0.02;
}

// 遠端實體（別的玩家／NPC／敵人／野生動物／節點）：正規實體內插 + 淡入淡出 + 離開移除。
// renderTime＝現在 − 內插延遲；kFallback＝資料不足時退回的 lerp 係數。
function updateRemoteEntities(map, scene_, renderTime, animate, dt, t, skipKey, kFallback) {
  for (const [key, g] of map) {
    if (skipKey !== undefined && key === skipKey) continue; // 自己走預測，另外處理
    const ox = g.position.x, oz = g.position.z;
    const s = sampleBufferAt(g.userData.buf, renderTime);
    if (s && s.bracketed) {
      // 正規內插：在包夾的兩筆快照之間，平滑、不過衝、不抽動
      g.position.x = s.x;
      g.position.z = s.z;
    } else {
      // 資料不足（剛出現／飢餓／AOI 邊緣）→ 退回原本的 lerp 追最新目標
      g.position.x += (g.userData.tx - g.position.x) * kFallback;
      g.position.z += (g.userData.tz - g.position.z) * kFallback;
    }
    if (animate) faceAndBob(g, g.position.x - ox, g.position.z - oz, dt, t);
    if (updateFade(g, dt)) { scene_.remove(g); map.delete(key); }
  }
}

// 自身客戶端預測 + 平滑對帳：輸入時本地立即往同方向推（零延遲），權威回來時柔和拉回。
// 不改協議、不改伺服器——這是伺服器權威下的純視覺預測。
function updateSelfPrediction(g, dt) {
  // 預測位移：用「正要送出的世界四向意圖」立刻往前推（與伺服器一致地對角正規化）。
  let dx = (inputKeys.right ? 1 : 0) - (inputKeys.left ? 1 : 0);
  let dz = (inputKeys.down ? 1 : 0) - (inputKeys.up ? 1 : 0);
  const len = Math.hypot(dx, dz);
  if (len > 0) {
    dx /= len; dz /= len;
    // selfMeasuredSpeed 已含跑步／加速／載具（由快照實測），不必再乘 run 倍率
    g.position.x += dx * selfMeasuredSpeed * dt;
    g.position.z += dz * selfMeasuredSpeed * dt;
  }
  // 對帳：往權威平滑拉回。差太多（瞬移／重生）→ 快速但仍平滑；否則小幅柔和修正。
  if (selfHasAuth) {
    const ex = selfAuthX - g.position.x;
    const ez = selfAuthZ - g.position.z;
    const err = Math.hypot(ex, ez);
    const rate = err > RECONCILE_SNAP_DIST ? RECONCILE_SNAP_RATE : RECONCILE_RATE;
    const a = 1 - Math.exp(-dt * rate); // 1-exp → 跟幀率無關
    g.position.x += ex * a;
    g.position.z += ez * a;
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

    // 退路 lerp 係數（資料不足時用）：隨幀時間調整，不同更新率都平滑（~8/s 收斂）
    const k = Math.min(1, dt * 8);
    const t = clock.elapsedTime;
    // render-in-past 的目標時間點：現在 − 內插延遲（在過去重建別人的位置 → 平滑）
    const renderTime = performance.now() - INTERP_DELAY_MS;
    const meGroup = myId ? players.get(myId) : null;
    // 別人／NPC／敵人／野生動物：正規實體內插（自己 myId 跳過，走預測）
    updateRemoteEntities(players, scene, renderTime, true, dt, t, myId, k);
    updateRemoteEntities(npcs, scene, renderTime, true, dt, t, undefined, k);
    updateRemoteEntities(wildlife, scene, renderTime, true, dt, t, undefined, k);
    updateRemoteEntities(enemies, scene, renderTime, true, dt, t, undefined, k);
    // 節點靜態：不轉身/起伏；位置吸最新目標（內插對靜態無差），仍走 AOI 淡入淡出
    updateRemoteEntities(nodes, scene, renderTime, false, dt, t, undefined, 1);

    // 自己：客戶端預測（零延遲）+ 平滑對帳權威，再疊上視覺跳的高度
    if (meGroup) {
      const ox = meGroup.position.x, oz = meGroup.position.z;
      updateSelfPrediction(meGroup, dt);
      meGroup.position.y = jumpZ;

      // 自己也朝移動方向平滑轉身（呈現層；位置仍對帳伺服器權威）
      const sdx = meGroup.position.x - ox, sdz = meGroup.position.z - oz;
      const sSpeed = Math.hypot(sdx, sdz) / Math.max(dt, 1e-3);
      if (sSpeed > 0.6) {
        const target = Math.atan2(sdx, sdz);
        let d = target - meGroup.rotation.y;
        while (d > Math.PI) d -= Math.PI * 2;
        while (d < -Math.PI) d += Math.PI * 2;
        meGroup.rotation.y += d * Math.min(1, dt * 10);
      }
      // 自己的火柴人走路動畫（自己走預測、不經 faceAndBob，這裡單獨驅動關節）
      animateStickman(meGroup, sSpeed, dt);

      // 第三人稱跟隨鏡頭：在自己後方、平滑跟隨（damping 用 1-exp → 跟幀率無關，像 Genshin 的滑順）
      const cx = Math.sin(camYaw) * Math.cos(camPitch) * camDist;
      const cz = Math.cos(camYaw) * Math.cos(camPitch) * camDist;
      const cy = Math.sin(camPitch) * camDist + 8;
      const tx = meGroup.position.x, ty = meGroup.position.y + 6, tz = meGroup.position.z;
      let desiredY = ty + cy;
      if (desiredY < 2) desiredY = 2; // 別讓鏡頭沉到地面下
      const desired = new THREE.Vector3(tx + cx, desiredY, tz + cz);
      camera.position.lerp(desired, 1 - Math.exp(-dt * 6));
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
