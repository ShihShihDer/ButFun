// ============================================================
// ButFun Voxel 前端（AI 生態世界 voxel 基底·切片①）
// ============================================================
// 後端權威、前端只渲染：收伺服器串來的 chunk（方塊資料）→ 合併 mesh（面剔除）→
// 玩家能走在地形上（重力 + voxel 逐軸 AABB 碰撞）→ 第三人稱鏡頭跟隨 + 鍵盤/觸控。
//
// 效能鐵律：**一個 chunk 一個合併 BufferGeometry**（面剔除去掉看不見的內面），
// 絕不每方塊一個 mesh（記取 #614 教訓）。
//
// 全隔離：只連 /voxel/ws、用 voxel 自己的 JSON 協定，不碰現有 2D/3D 任何東西。
// 不抄外部碼；全繁中註解；node --check 過。

import * as THREE from "three";

// ── 常數（與後端 voxel.rs 對齊）──────────────────────────────────────────────
const CHUNK = 16; // 一 chunk 邊長（方塊數），與 voxel::CHUNK 一致
// 方塊型別（對齊 Block enum）
const AIR = 0, GRASS = 1, DIRT = 2, STONE = 3, SAND = 4, WOOD = 5, LEAVES = 6, WATER = 7;
// 合成台 v1（ROADMAP 658）——玩家合成而得，不自然生成
const PLANK = 8, STONE_BRICK = 9, GLASS = 10;
// 種田 v1（ROADMAP 659）——農地狀態方塊 + 種子物品
const FARM_SOIL = 11, FARM_SOIL_SEEDED = 12, WHEAT_MATURE = 13;
const SEEDS = 14; // 純物品（無對應方塊），從葉片/收割掉落
// 方塊顏色（程序生成、純色；不用任何外部美術資產）
const COLOR = {
  [GRASS]:             [0.36, 0.66, 0.27],
  [DIRT]:              [0.55, 0.40, 0.26],
  [STONE]:             [0.50, 0.50, 0.52],
  [SAND]:              [0.85, 0.78, 0.55],
  [WOOD]:              [0.45, 0.31, 0.18],
  [LEAVES]:            [0.27, 0.55, 0.27],
  [WATER]:             [0.20, 0.45, 0.85],
  // 合成方塊：比自然原料更精緻（淺色調）
  [PLANK]:             [0.78, 0.62, 0.42], // 木板——淺棕，比原木明亮
  [STONE_BRICK]:       [0.62, 0.59, 0.56], // 石磚——均勻灰，比原石精緻
  [GLASS]:             [0.82, 0.93, 0.98], // 玻璃——淡藍，像磨砂玻璃
  // 種田 v1：農地三態——顏色漸層暗示成長進度
  [FARM_SOIL]:         [0.38, 0.24, 0.12], // 農田土——深棕，耕過的泥土
  [FARM_SOIL_SEEDED]:  [0.32, 0.42, 0.20], // 幼苗——帶綠的深色，種子萌芽中
  [WHEAT_MATURE]:      [0.88, 0.76, 0.22], // 成熟小麥——金黃色，可收割
};

const DEBUG = location.search.includes("debug");
// 觸控裝置偵測（用於顯示精簡 HUD 文字 + 啟用搖桿/跳鈕/放置鈕）
const isTouch = "ontouchstart" in window || navigator.maxTouchPoints > 0;
const hudEl = document.getElementById("hud");
const dbgEl = document.getElementById("dbg");
const errEl = document.getElementById("err");
function showErr(msg) { if (errEl) { errEl.textContent = msg; errEl.style.display = "block"; } }

// ── Three.js 場景 ──────────────────────────────────────────────────────────
const app = document.getElementById("app");
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x87b7e0);
scene.fog = new THREE.Fog(0x87b7e0, 40, 120);

const camera = new THREE.PerspectiveCamera(70, window.innerWidth / window.innerHeight, 0.1, 1000);

const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
renderer.setSize(window.innerWidth, window.innerHeight);
app.appendChild(renderer.domElement);

// 半球光（天空/地面）給全向環境光（保證永不全黑），加一盞方向光做陰影感。
// hemi 存起來以便晝夜循環 v1 動態調整強度。
const hemi = new THREE.HemisphereLight(0xcfe8ff, 0x6b7a55, 1.15);
scene.add(hemi);
const sun = new THREE.DirectionalLight(0xfff3da, 0.65);
sun.position.set(40, 80, 25);
scene.add(sun);

// ── 晝夜循環 v1 ─────────────────────────────────────────────────────────────
// time_of_day：0.0=午夜、0.25=黎明、0.5=正午、0.75=黃昏、1.0=午夜（循環）。
// 由伺服器每幀廣播，前端只負責渲染（天空色/太陽位置/光強度）。
let worldTime = 0.42; // 預設白天，伺服器推播後更新

// 天空關鍵幀：[time, skyHex, sunColorHex, sunIntensity, hemiIntensity]
// 每兩個鄰近幀之間做線性插值。
const SKY_KEYS = [
  [0.00, 0x060d1a, 0x1a2d45, 0.03, 0.30], // 深夜
  [0.18, 0x0d1b30, 0x1a2d45, 0.05, 0.40], // 深夜末
  [0.22, 0xd4603a, 0xd4603a, 0.30, 0.65], // 黎明前橙紅
  [0.30, 0xf0a060, 0xf0c060, 0.50, 0.90], // 清晨金黃
  [0.38, 0x87b7e0, 0xfff3da, 0.65, 1.15], // 白晝湛藍
  [0.62, 0x87b7e0, 0xfff3da, 0.65, 1.15], // 白晝湛藍（延續）
  [0.70, 0xf08040, 0xff8c40, 0.45, 0.90], // 傍晚橙
  [0.80, 0xc04020, 0xff6020, 0.18, 0.55], // 黃昏深紅
  [0.88, 0x1a0d20, 0x1a2d45, 0.04, 0.35], // 入夜過渡
  [1.00, 0x060d1a, 0x1a2d45, 0.03, 0.30], // 深夜（循環對齊 t=0）
];

function _hc(hex) {
  return [(hex >> 16 & 0xff) / 255, (hex >> 8 & 0xff) / 255, (hex & 0xff) / 255];
}

// 更新天空背景色、霧色、太陽方向/顏色、半球光強度。
function updateSkyAndLight(t) {
  // 找所在的插值段。
  let i = 0;
  while (i < SKY_KEYS.length - 2 && SKY_KEYS[i + 1][0] <= t) i++;
  const [t0, sky0, sun0, si0, hi0] = SKY_KEYS[i];
  const [t1, sky1, sun1, si1, hi1] = SKY_KEYS[i + 1];
  const f = t1 > t0 ? Math.max(0, Math.min(1, (t - t0) / (t1 - t0))) : 0;

  // 插值天空色並套用到背景+霧。
  const [sr0, sg0, sb0] = _hc(sky0);
  const [sr1, sg1, sb1] = _hc(sky1);
  const sr = sr0 + (sr1 - sr0) * f;
  const sg = sg0 + (sg1 - sg0) * f;
  const sb = sb0 + (sb1 - sb0) * f;
  scene.background.setRGB(sr, sg, sb);
  scene.fog.color.setRGB(sr, sg, sb);

  // 插值太陽色與強度。
  const [ur0, ug0, ub0] = _hc(sun0);
  const [ur1, ug1, ub1] = _hc(sun1);
  sun.color.setRGB(ur0 + (ur1 - ur0) * f, ug0 + (ug1 - ug0) * f, ub0 + (ub1 - ub0) * f);
  sun.intensity = si0 + (si1 - si0) * f;

  // 太陽軌跡：t=0.25 日出（東）、t=0.5 正午（頂）、t=0.75 日落（西）。
  const ang = (t - 0.25) * Math.PI * 2;
  sun.position.set(-Math.cos(ang) * 80, Math.sin(ang) * 80, 25);

  // 半球光強度。
  hemi.intensity = hi0 + (hi1 - hi0) * f;
}

// 初始套用，讓進場就是白天而非等第一幀快照。
updateSkyAndLight(worldTime);

// 方塊用 Lambert + 頂點色（每方塊上色），對光反應但靠半球光保底不黑。
// DoubleSide：切片① 求穩，避免任一面纏繞方向算錯被背面剔除成破洞/黑屏（perf 微讓步，之後可收回 FrontSide）。
const opaqueMat = new THREE.MeshLambertMaterial({ vertexColors: true, side: THREE.DoubleSide });
const waterMat = new THREE.MeshLambertMaterial({ color: 0x2f6fd0, transparent: true, opacity: 0.55, side: THREE.DoubleSide });

// ── 世界資料：chunk 方塊 + mesh ─────────────────────────────────────────────
const chunks = new Map();      // "cx,cy,cz" -> Uint8Array(4096)
const meshes = new Map();      // "cx,cy,cz" -> { solid: Mesh|null, water: Mesh|null }
const dirty = new Set();       // 待重建 mesh 的 chunk key
const requested = new Set();   // 已向伺服器要過的 column "cx,cz"

function ckey(cx, cy, cz) { return cx + "," + cy + "," + cz; }

function b64ToBytes(b64) {
  const bin = atob(b64);
  const arr = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
  return arr;
}

// 任一世界座標的方塊原值：未載入回 -1，地心（y<0）回石頭（對齊後端基岩）。
function getRaw(wx, wy, wz) {
  if (wy < 0) return STONE;
  const cx = Math.floor(wx / CHUNK), cy = Math.floor(wy / CHUNK), cz = Math.floor(wz / CHUNK);
  const ch = chunks.get(ckey(cx, cy, cz));
  if (!ch) return -1;
  const lx = wx - cx * CHUNK, ly = wy - cy * CHUNK, lz = wz - cz * CHUNK;
  return ch[lx + lz * CHUNK + ly * CHUNK * CHUNK];
}

// 碰撞用：未載入(-1)視為空（不擋路、不卡人）；水與空氣不實心。
function solidCollide(wx, wy, wz) {
  const r = getRaw(wx, wy, wz);
  return r > 0 && r !== WATER; // -1/AIR/WATER → false
}

// ── 六面定義（外向法線；用 DoubleSide 材質保險，避免纏繞方向把面剔成黑屏）──────
const FACES = [
  { n: [1, 0, 0],  v: [[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]], d: [1, 0, 0] },
  { n: [-1, 0, 0], v: [[0, 0, 1], [0, 1, 1], [0, 1, 0], [0, 0, 0]], d: [-1, 0, 0] },
  { n: [0, 1, 0],  v: [[0, 1, 1], [1, 1, 1], [1, 1, 0], [0, 1, 0]], d: [0, 1, 0] },
  { n: [0, -1, 0], v: [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]], d: [0, -1, 0] },
  { n: [0, 0, 1],  v: [[1, 0, 1], [1, 1, 1], [0, 1, 1], [0, 0, 1]], d: [0, 0, 1] },
  { n: [0, 0, -1], v: [[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 0, 0]], d: [0, 0, -1] },
];

// 不透明面是否該畫：相鄰是空氣或水（看得到）才畫；未載入(-1)當作實心 → 不畫（避免世界邊緣冒出一面牆，等鄰塊串到再補）。
function faceVisibleOpaque(nx, ny, nz) {
  const r = getRaw(nx, ny, nz);
  if (r === -1) return false;
  return r === AIR || r === WATER;
}
// 水面只朝空氣畫（露出水面那一片），鄰格未載入時不畫。
function faceVisibleWater(nx, ny, nz) {
  const r = getRaw(nx, ny, nz);
  return r === AIR;
}

// 重建一個 chunk 的合併 mesh（不透明 + 水各一個 geometry）。
function rebuildChunk(key) {
  const [cx, cy, cz] = key.split(",").map(Number);
  const ch = chunks.get(key);
  const old = meshes.get(key);
  if (old) {
    if (old.solid) { scene.remove(old.solid); old.solid.geometry.dispose(); }
    if (old.water) { scene.remove(old.water); old.water.geometry.dispose(); }
    meshes.delete(key);
  }
  if (!ch) return;

  const pos = [], norm = [], col = [], idx = [];
  const wpos = [], wnorm = [], widx = [];
  const baseX = cx * CHUNK, baseY = cy * CHUNK, baseZ = cz * CHUNK;

  for (let ly = 0; ly < CHUNK; ly++) {
    for (let lz = 0; lz < CHUNK; lz++) {
      for (let lx = 0; lx < CHUNK; lx++) {
        const b = ch[lx + lz * CHUNK + ly * CHUNK * CHUNK];
        if (b === AIR) continue;
        const wx = baseX + lx, wy = baseY + ly, wz = baseZ + lz;
        if (b === WATER) {
          for (const f of FACES) {
            if (!faceVisibleWater(wx + f.d[0], wy + f.d[1], wz + f.d[2])) continue;
            emitFace(wpos, wnorm, null, widx, lx, ly, lz, f, null);
          }
        } else {
          const c = COLOR[b] || COLOR[STONE];
          for (const f of FACES) {
            if (!faceVisibleOpaque(wx + f.d[0], wy + f.d[1], wz + f.d[2])) continue;
            emitFace(pos, norm, col, idx, lx, ly, lz, f, c);
          }
        }
      }
    }
  }

  const entry = { solid: null, water: null };
  if (idx.length) {
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
    g.setAttribute("normal", new THREE.Float32BufferAttribute(norm, 3));
    g.setAttribute("color", new THREE.Float32BufferAttribute(col, 3));
    g.setIndex(idx);
    const m = new THREE.Mesh(g, opaqueMat);
    m.position.set(baseX, baseY, baseZ);
    scene.add(m);
    entry.solid = m;
  }
  if (widx.length) {
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.Float32BufferAttribute(wpos, 3));
    g.setAttribute("normal", new THREE.Float32BufferAttribute(wnorm, 3));
    g.setIndex(widx);
    const m = new THREE.Mesh(g, waterMat);
    m.position.set(baseX, baseY, baseZ);
    scene.add(m);
    entry.water = m;
  }
  meshes.set(key, entry);
}

// 把一個面（4 頂點、2 三角）推進陣列。座標用 chunk 局部（mesh 自身有 position 偏移）。
function emitFace(pos, norm, col, idx, lx, ly, lz, f, c) {
  const start = pos.length / 3;
  for (const v of f.v) {
    pos.push(lx + v[0], ly + v[1], lz + v[2]);
    norm.push(f.n[0], f.n[1], f.n[2]);
    if (col && c) col.push(c[0], c[1], c[2]);
  }
  idx.push(start, start + 1, start + 2, start, start + 2, start + 3);
}

// 把一個 chunk 連同鄰塊標記為待重建（鄰塊也要重算面剔除）。
function markDirty(cx, cy, cz) {
  dirty.add(ckey(cx, cy, cz));
  dirty.add(ckey(cx + 1, cy, cz)); dirty.add(ckey(cx - 1, cy, cz));
  dirty.add(ckey(cx, cy + 1, cz)); dirty.add(ckey(cx, cy - 1, cz));
  dirty.add(ckey(cx, cy, cz + 1)); dirty.add(ckey(cx, cy, cz - 1));
}

// ── 玩家狀態（前端權威預測；位置同步回伺服器給別人看）──────────────────────
const player = { x: 0.5, y: 30, z: 0.5, vy: 0, grounded: false, yaw: 0 };
const PW = 0.3, PH = 1.7; // 半寬 / 身高

// 踏階視覺補間：物理 Y 瞬到位（碰撞正確、避免穿模），視覺 Y 平滑跟上（消除閃爍/瞬跳）。
// stepSmooth 是「視覺比物理落後多少格」，踏階時累積、每幀指數衰減歸零。
// 往下/重力完全不受影響（stepSmooth 只在踏階成功時累積，永遠 >= 0）。
let stepSmooth = 0;
// 衰減速率（格/秒）；可調：10 ≈ 0.3 秒內視覺追上物理，夠快看得出「走上去」、不拖泥帶水。
const STEP_SMOOTH_K = 10;
let myId = null;
let myName = "旅人";

// 好感度 v1（ROADMAP 656）：我與各居民的互動記憶筆數（連線後從 /voxel/affinity 拉取）。
// key = resident_id, value = count (0=陌生人, 1-2=相識, 3+=友人)
const myAffinity = new Map();

// 玩家身體（第三人稱可見的小方塊角色）
const bodyGeo = new THREE.BoxGeometry(0.6, PH, 0.6);
const bodyMat = new THREE.MeshLambertMaterial({ color: 0xffcf6b });
const bodyMesh = new THREE.Mesh(bodyGeo, bodyMat);
scene.add(bodyMesh);

// 其他玩家
const others = new Map(); // id -> Mesh
const otherMat = new THREE.MeshLambertMaterial({ color: 0x8fd0ff });

// ── 乙太方界 AI 居民（切片③）────────────────────────────────────────────────
// 後端權威：居民的位置/名字/說的話都由 /voxel/ws 的 players 快照帶來，前端只渲染。
// FPS 鐵律（記取 #614/#820）：居民少（~4 位）、共用幾何/材質、頭頂名牌與泡泡用快取貼圖，
// 文字沒變就不重建貼圖；遠處（超過霧距）整個 group 隱藏，零渲染負擔。
const residents = new Map(); // id -> { group, label, bubble, lastName, lastSay }
// 居民配色（暖棕，與自己金色/別的玩家藍色一眼區分）。共用材質/幾何省記憶體。
const RES_BODY_MAT = new THREE.MeshLambertMaterial({ color: 0xd8b070 });
const RES_HEAD_MAT = new THREE.MeshLambertMaterial({ color: 0xe8c89a });
const RES_TORSO_GEO = new THREE.BoxGeometry(0.5, 1.0, 0.32);
const RES_HEAD_GEO = new THREE.BoxGeometry(0.42, 0.42, 0.42);
const RES_VISIBLE_DIST = 110; // 超過此距離（接近霧盡頭）隱藏，省繪製

// 文字貼圖 sprite（名牌/泡泡共用工廠）。bubble=true 用柔色圓底（像在說話），否則白描邊名牌。
function makeTextSprite(text, bubble) {
  const canvas = document.createElement("canvas");
  canvas.width = 256; canvas.height = 64;
  const ctx = canvas.getContext("2d");
  ctx.font = "bold 26px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  let label = text;
  if (label.length > 16) label = label.slice(0, 15) + "…";
  if (bubble) {
    const tw = Math.min(248, ctx.measureText(label).width + 28);
    ctx.fillStyle = "rgba(245,248,255,0.92)";
    const bx = 128 - tw / 2, bw = tw, by = 14, bh = 38, rr = 10;
    ctx.beginPath();
    ctx.moveTo(bx + rr, by);
    ctx.arcTo(bx + bw, by, bx + bw, by + bh, rr);
    ctx.arcTo(bx + bw, by + bh, bx, by + bh, rr);
    ctx.arcTo(bx, by + bh, bx, by, rr);
    ctx.arcTo(bx, by, bx + bw, by, rr);
    ctx.closePath(); ctx.fill();
    ctx.fillStyle = "#243044";
    ctx.fillText(label, 128, by + bh / 2 + 1);
  } else {
    ctx.lineWidth = 5; ctx.strokeStyle = "rgba(0,0,0,0.8)";
    ctx.strokeText(label, 128, 32);
    ctx.fillStyle = "#fff7e6";
    ctx.fillText(label, 128, 32);
  }
  const tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  const sprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: false }));
  // 世界單位：方塊尺度，名牌約 2 寬 0.5 高，浮在頭頂。
  sprite.scale.set(2.4, 0.6, 1);
  return sprite;
}
function setSpriteText(sprite, text, bubble) {
  const fresh = makeTextSprite(text, bubble);
  if (sprite.material.map) sprite.material.map.dispose();
  sprite.material.map = fresh.material.map;
  sprite.material.needsUpdate = true;
}

// 居民「夢想副標籤」sprite 工廠——名牌之下、小字 dim 暖色，顯示玩家種下的心願。
function makeDesireSprite(text) {
  const canvas = document.createElement("canvas");
  canvas.width = 256; canvas.height = 44;
  const ctx = canvas.getContext("2d");
  ctx.font = "italic 18px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  let label = text;
  if (label.length > 18) label = label.slice(0, 17) + "…";
  ctx.lineWidth = 4; ctx.strokeStyle = "rgba(0,0,0,0.55)";
  ctx.strokeText(label, 128, 22);
  ctx.fillStyle = "rgba(255,220,140,0.88)";
  ctx.fillText(label, 128, 22);
  const tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  const sprite = new THREE.Sprite(
    new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: false })
  );
  sprite.scale.set(2.2, 0.48, 1);
  return sprite;
}
function setDesireText(sprite, text) {
  const fresh = makeDesireSprite(text);
  if (sprite.material.map) sprite.material.map.dispose();
  sprite.material.map = fresh.material.map;
  sprite.material.needsUpdate = true;
}

// ── 好感度指示燈（ROADMAP 656）──────────────────────────────────────────────

/** 依好感度計數回傳指示燈 emoji。0=無, 1-2=淡藍心(相識), 3+=金心(友人)。純函式、可測。 */
function affinityEmoji(count) {
  if (count <= 0) return "";
  if (count <= 2) return "💙";
  return "💛";
}

/** 製作好感度指示燈 sprite（小字 emoji，居名牌正上方）。 */
function makeAffinitySprite(emoji) {
  const canvas = document.createElement("canvas");
  canvas.width = 64; canvas.height = 40;
  const ctx = canvas.getContext("2d");
  ctx.font = "24px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  if (emoji) ctx.fillText(emoji, 32, 20);
  const tex = new THREE.CanvasTexture(canvas);
  const sprite = new THREE.Sprite(
    new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: false })
  );
  sprite.scale.set(0.7, 0.44, 1);
  return sprite;
}

function setAffinityEmoji(sprite, emoji) {
  const fresh = makeAffinitySprite(emoji);
  if (sprite.material.map) sprite.material.map.dispose();
  sprite.material.map = fresh.material.map;
  sprite.material.needsUpdate = true;
}

/** 從後端拉取玩家與各居民的好感度計數 → 更新 myAffinity Map。
 *  連線後取一次；每次對話後再更新，讓指示燈即時反映互動。零 LLM。 */
async function refreshAffinity() {
  if (!myName || myName === "旅人") return;
  try {
    const resp = await fetch(`/voxel/affinity?player=${encodeURIComponent(myName)}`);
    if (!resp.ok) return;
    const data = await resp.json();
    for (const [rid, count] of Object.entries(data)) {
      myAffinity.set(rid, typeof count === "number" ? count : 0);
    }
  } catch (_) { /* 網路問題忽略 */ }
}

// 建一位居民的可見實體（簡單 voxel 人形：軀幹 + 頭 + 名牌 + 夢想副標籤 + 泡泡）。
// group.userData.rid 記居民 id，供點選 raycast 反查「點到的是哪位居民」。
function buildResident(id, name) {
  const group = new THREE.Group();
  group.userData.rid = id;
  const torso = new THREE.Mesh(RES_TORSO_GEO, RES_BODY_MAT);
  torso.position.y = 0.5; // 腳底在 group 原點，軀幹中心 0.5
  group.add(torso);
  const head = new THREE.Mesh(RES_HEAD_GEO, RES_HEAD_MAT);
  head.position.y = 1.25;
  group.add(head);
  const label = makeTextSprite(name, false);
  label.position.y = 2.0;
  group.add(label);
  // 夢想副標籤：名牌下方，有心願時才顯示（玩家看到「我說的話種下了什麼」）。
  const desireLabel = makeDesireSprite("");
  desireLabel.position.y = 1.6;
  desireLabel.visible = false;
  group.add(desireLabel);
  const bubble = makeTextSprite("", true);
  bubble.position.y = 2.55;
  bubble.visible = false;
  group.add(bubble);
  // 好感度指示燈（ROADMAP 656）：有好感才顯示，偏置在名牌右側不覆蓋名字。
  const affinityIndicator = makeAffinitySprite("");
  affinityIndicator.position.set(0.85, 2.05, 0);
  affinityIndicator.visible = false;
  group.add(affinityIndicator);
  scene.add(group);
  return { group, label, desireLabel, bubble, affinityIndicator, lastName: name, lastSay: "", lastDesire: "", lastAffinity: "" };
}

// 依伺服器快照更新所有居民（位置/朝向/名字/說的話）。新出現的就建、消失的就移除。
function updateResidents(list) {
  const seen = new Set();
  for (const r of list) {
    seen.add(r.id);
    let ent = residents.get(r.id);
    if (!ent) { ent = buildResident(r.id, r.name); residents.set(r.id, ent); }
    ent.group.position.set(r.x, r.y, r.z);
    ent.group.rotation.y = r.yaw || 0;
    if (r.name !== ent.lastName) { setSpriteText(ent.label, r.name, false); ent.lastName = r.name; }
    // 夢想副標籤：有 desire 就顯示「💭 心願文字」，沒有就隱藏。
    const desire = r.desire || "";
    if (desire !== ent.lastDesire) {
      ent.lastDesire = desire;
      if (desire) { setDesireText(ent.desireLabel, "💭 " + desire); ent.desireLabel.visible = true; }
      else { ent.desireLabel.visible = false; }
    }
    const say = r.say || "";
    if (say !== ent.lastSay) {
      ent.lastSay = say;
      if (say) { setSpriteText(ent.bubble, say, true); ent.bubble.visible = true; }
      else { ent.bubble.visible = false; }
    }
    // 好感度指示燈（ROADMAP 656）：依 myAffinity 決定顯示哪種心型（sig 保護不重建貼圖）。
    const affCount = myAffinity.get(r.id) || 0;
    const emoji = affinityEmoji(affCount);
    if (emoji !== ent.lastAffinity) {
      ent.lastAffinity = emoji;
      if (emoji) { setAffinityEmoji(ent.affinityIndicator, emoji); ent.affinityIndicator.visible = true; }
      else { ent.affinityIndicator.visible = false; }
    }
    // 距離 LOD：遠到接近霧盡頭就整個隱藏（省繪製，不崩 FPS）。
    const dx = r.x - player.x, dz = r.z - player.z;
    ent.group.visible = (dx * dx + dz * dz) < (RES_VISIBLE_DIST * RES_VISIBLE_DIST);
  }
  for (const [id, ent] of residents) {
    if (!seen.has(id)) { scene.remove(ent.group); residents.delete(id); }
  }
}

// ── 點居民 → 對話（raycast 點選 + 直式對話框）────────────────────────────────
// 點到居民（在互動距離內）就開對話框；送出 → 後端以該居民人設呼 LLM → 回 talk 訊息。
const raycaster = new THREE.Raycaster();
const TALK_REACH = 16; // 可對話的最遠距離（方塊）：太遠的居民點不到，貼近「在你附近的人」
// 從螢幕座標 raycast 找命中的居民 id（命中且在 TALK_REACH 內才回 id，否則 null）。
function pickResident(clientX, clientY) {
  const rect = renderer.domElement.getBoundingClientRect();
  const ndc = new THREE.Vector2(
    ((clientX - rect.left) / rect.width) * 2 - 1,
    -((clientY - rect.top) / rect.height) * 2 + 1
  );
  raycaster.setFromCamera(ndc, camera);
  const pickables = [];
  for (const ent of residents.values()) {
    if (ent.group.visible) ent.group.traverse((o) => { if (o.isMesh) pickables.push(o); });
  }
  if (!pickables.length) return null;
  const hits = raycaster.intersectObjects(pickables, false);
  if (!hits.length || hits[0].distance > TALK_REACH) return null;
  // 沿父鏈往上找帶 rid 的 group。
  let obj = hits[0].object;
  while (obj && !(obj.userData && obj.userData.rid)) obj = obj.parent;
  return obj && obj.userData ? obj.userData.rid : null;
}

// 對話框 DOM + 狀態。
const chatEl = document.getElementById("chat");
const chatTitleEl = document.getElementById("chatTitle");
const chatLogEl = document.getElementById("chatLog");
const chatQuickEl = document.getElementById("chatQuick");
const chatInputEl = document.getElementById("chatInput");
const chatSendEl = document.getElementById("chatSend");
let chatRid = null;          // 目前對話的居民 id
let lastTalkReply = null;    // 最近一次居民回覆（QA 用）
let thinkingEl = null;       // 目前正在顯示的「思考中」動畫元素（null 代表沒有）

function appendMsg(kind, text) {
  if (!chatLogEl) return;
  const d = document.createElement("div");
  d.className = "msg " + kind;
  d.textContent = text;
  chatLogEl.appendChild(d);
  chatLogEl.scrollTop = chatLogEl.scrollHeight;
}

// 顯示「思考中」動畫指示器（居民收到訊息後立即顯示，等真回覆取代）。
// 若已有一個思考中元素（連發），先移除舊的再建新的。
function showThinking(name) {
  if (!chatLogEl) return;
  removeThinking(); // 清掉上一輪殘留
  const d = document.createElement("div");
  d.className = "msg thinking";
  // 顯示「居民名 思考中」並帶跳動點點（純 CSS animation，不用 JS timer）。
  const label = document.createElement("span");
  label.className = "thinking-label";
  label.textContent = (name || "居民") + " 思考中";
  const dots = document.createElement("span");
  dots.className = "thinking-dots";
  dots.setAttribute("aria-hidden", "true");
  d.appendChild(label);
  d.appendChild(dots);
  chatLogEl.appendChild(d);
  chatLogEl.scrollTop = chatLogEl.scrollHeight;
  thinkingEl = d;
}

// 移除「思考中」動畫元素（真回覆到了後呼叫）。
function removeThinking() {
  if (thinkingEl && thinkingEl.parentNode) {
    thinkingEl.parentNode.removeChild(thinkingEl);
  }
  thinkingEl = null;
}

// 開對話框（換對象就清空對話紀錄）。
function openChat(rid, name) {
  if (!chatEl) return;
  if (chatRid !== rid) {
    // 換居民：清空對話、移除思考中指示器、顯示「你走近了…」
    thinkingEl = null; // 舊元素連同 innerHTML 一起清掉，不用再 removeChild
    chatLogEl.innerHTML = "";
    appendMsg("sys", "你走近了 " + (name || "居民"));
  }
  chatRid = rid;
  chatTitleEl.textContent = name || "居民";
  chatEl.style.display = "flex";
  updateGiftBtn(); // 贈禮 v1：更新按鈕顯示哪件物品
}
function closeChat() { if (chatEl) chatEl.style.display = "none"; }

// 送一句話給目前對話的居民。
function sendTalk(text) {
  const t = (text || "").trim();
  if (!t || !chatRid || !wsReady) return;
  ws.send(JSON.stringify({ t: "talk", resident_id: chatRid, text: t.slice(0, 200) }));
  appendMsg("me", "你：" + t);
}

if (chatEl) {
  document.getElementById("chatClose").addEventListener("click", closeChat);
  chatSendEl.addEventListener("click", () => { sendTalk(chatInputEl.value); chatInputEl.value = ""; });
  chatInputEl.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { sendTalk(chatInputEl.value); chatInputEl.value = ""; e.preventDefault(); }
  });
  // 快捷句：不用打字也能互動（手機友善）。
  for (const q of ["你好！", "你在做什麼？", "這裡是哪裡？"]) {
    const b = document.createElement("div");
    b.className = "qbtn"; b.textContent = q;
    b.addEventListener("click", () => sendTalk(q));
    chatQuickEl.appendChild(b);
  }
  // 日記鈕：開啟當前對話居民的日記。
  const diaryBtnEl = document.getElementById("chatDiary");
  if (diaryBtnEl) diaryBtnEl.addEventListener("click", () => { if (chatRid) openDiary(chatRid); });
  // 贈禮鈕：送背包最多的一件給當前居民（ROADMAP 660）。
  const giftBtnEl = document.getElementById("chatGift");
  if (giftBtnEl) giftBtnEl.addEventListener("click", () => { if (chatRid) trySendGift(); });
}

// ── 居民贈禮 v1（ROADMAP 660）────────────────────────────────────────────────
// 把採來的材料化作一份心意送給居民；居民記得你的照料，好感度 +2。

/// 不可作為禮物的 block_id（純 inventory 物品 / 不合語意送出）。
const GIFT_EXCLUDED = new Set([0, 7, 12]); // Air / Water / FarmSoilSeeded（已種幼苗）

/**
 * 從背包（myInv: Map<blockId, count>）挑出最佳禮物。
 * 策略：選存量最多的可贈物品（最不稀缺）；同量以 blockId 小者優先（確定性）。
 * 空背包或無可贈物品回 null。
 * 確定性純函式，壞值（非 Map / 空 Map）安全回 null。
 * @param {Map<number,number>} inv
 * @returns {{ blockId: number, count: number }|null}
 */
function giftPickItem(inv) {
  if (!(inv instanceof Map)) return null;
  let best = null;
  for (const [bid, cnt] of inv) {
    if (GIFT_EXCLUDED.has(bid) || cnt <= 0) continue;
    if (!best || cnt > best.count || (cnt === best.count && bid < best.blockId)) {
      best = { blockId: bid, count: cnt };
    }
  }
  return best;
}

/** 更新「🎁 贈禮」按鈕顯示（呼叫於開對話框 / inv 改變後）。 */
function updateGiftBtn() {
  const el = document.getElementById("chatGift");
  if (!el) return;
  const pick = giftPickItem(myInv);
  if (!pick) {
    el.textContent = "🎁 贈禮";
    el.classList.add("gift-empty");
  } else {
    const iname = BLOCK_NAME[pick.blockId] || "物品";
    el.textContent = "🎁 贈" + iname;
    el.classList.remove("gift-empty");
  }
}

let lastGiftMs = 0; // 贈禮本地冷卻（防連按）

/** 執行贈禮：消耗最多的那件物品送給當前居民（ROADMAP 660）。 */
function trySendGift() {
  if (!wsReady || !chatRid) return;
  const now = Date.now();
  if (now - lastGiftMs < 1500) return; // 1.5 秒本地冷卻
  const pick = giftPickItem(myInv);
  if (!pick) {
    appendMsg("sys", "背包是空的，先去採集一些材料吧～");
    return;
  }
  lastGiftMs = now;
  ws.send(JSON.stringify({ t: "gift", resident_id: chatRid, item_id: pick.blockId }));
}

// ── 居民日記面板（ROADMAP 650）────────────────────────────────────────────────
// 玩家可在聊天框點「📖 日記」看居民的記憶足跡與當前心願——把 AI 的內在生活變成可讀的故事。

const diaryEl = document.getElementById("diary");
const diaryTitleEl = document.getElementById("diaryTitle");
const diaryBodyEl = document.getElementById("diaryBody");

/** 開啟指定居民的日記面板（fetch /voxel/diary → 過濾 resident_id → 渲染）。 */
async function openDiary(rid) {
  if (!diaryEl || !diaryBodyEl) return;
  // 取居民顯示名（從 residents Map 讀）。
  const ent = residents.get(rid);
  const name = (ent && ent.lastName) || rid;
  if (diaryTitleEl) diaryTitleEl.textContent = name + " 的日記";
  diaryBodyEl.innerHTML = '<div class="diary-empty">載入中…</div>';
  diaryEl.style.display = "flex";
  try {
    const resp = await fetch("/voxel/diary");
    if (!resp.ok) throw new Error("diary fetch failed: " + resp.status);
    const pages = await resp.json();
    const page = Array.isArray(pages) ? pages.find(p => p.resident_id === rid) : null;
    renderDiaryPage(page, name);
  } catch (e) {
    diaryBodyEl.innerHTML = '<div class="diary-empty">無法讀取日記。</div>';
  }
}

/** 把 DiaryPage 資料渲染進日記面板。 */
function renderDiaryPage(page, name) {
  if (!diaryBodyEl) return;
  diaryBodyEl.innerHTML = "";

  // 心願區塊。
  const desireSection = document.createElement("div");
  if (page && page.desire) {
    desireSection.innerHTML =
      '<div class="diary-desire-label">💭 當前心願</div>' +
      '<div class="diary-desire">「' + escHtml(page.desire) + '」</div>';
  } else {
    desireSection.innerHTML =
      '<div class="diary-no-desire">還沒有心願……等待旅人的話語種下第一顆夢想。</div>';
  }
  diaryBodyEl.appendChild(desireSection);

  // 記憶條目。
  const entries = (page && Array.isArray(page.entries)) ? page.entries : [];
  const memSection = document.createElement("div");
  const secTitle = document.createElement("div");
  secTitle.className = "diary-section-title";
  secTitle.textContent = entries.length ? "📝 記憶片段（最新在前）" : "📝 記憶片段";
  memSection.appendChild(secTitle);
  if (entries.length === 0) {
    const empty = document.createElement("div");
    empty.className = "diary-empty";
    empty.textContent = name + " 還沒有任何記憶……來跟她說說話吧。";
    memSection.appendChild(empty);
  } else {
    for (const e of entries) {
      const row = document.createElement("div");
      row.className = "diary-entry";
      row.innerHTML =
        '<span class="diary-entry-player">👤 ' + escHtml(e.player) + '</span>' +
        '<span class="diary-entry-text">' + escHtml(e.text) + '</span>';
      memSection.appendChild(row);
    }
  }
  diaryBodyEl.appendChild(memSection);
}

/** 純函式：轉義 HTML 特殊字元，避免記憶摘要插入 XSS（防護邊界）。 */
function escHtml(str) {
  if (typeof str !== "string") return "";
  return str.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
            .replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}

function closeDiary() { if (diaryEl) diaryEl.style.display = "none"; }

if (diaryEl) {
  const closeBtn = document.getElementById("diaryClose");
  if (closeBtn) closeBtn.addEventListener("click", closeDiary);
  // 點面板外（背景）也關閉。
  diaryEl.addEventListener("click", (e) => { if (e.target === diaryEl) closeDiary(); });
}

// 對外暴露日記相關方法給 QA 使用。
export const __diaryTest = {
  escHtml,
  renderDiaryPage: (page, name) => { renderDiaryPage(page, name); return diaryBodyEl && diaryBodyEl.innerHTML; },
};

// ── 城鎮動態 Feed（ROADMAP 655）────────────────────────────────────────────────
const feedEl = document.getElementById("feed");
const feedBodyEl = document.getElementById("feedBody");
const feedBtnEl = document.getElementById("feedBtn");

/** 把 Unix 秒換算成「X 分鐘前」繁中字串。
 * @param {number} tsSecs 事件的 Unix 秒時戳。
 * @returns {string}
 */
export function formatRelativeTime(tsSecs) {
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - tsSecs);
  if (diff < 60)     return "剛才";
  if (diff < 3600)   return `${Math.floor(diff / 60)} 分鐘前`;
  if (diff < 86400)  return `${Math.floor(diff / 3600)} 小時前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

/** 各事件類型對應的 emoji 提示。 */
const KIND_EMOJI = {
  "新心願":  "💭",
  "念頭種下": "✨",
  "鄰里閒聊": "💬",
  "蓋家動工": "🏗️",
  "蓋家完工": "🏠",
};

/** 把後端回傳的 FeedEvent 陣列渲染到 #feedBody。
 * @param {Array<{ts_secs:number, kind:string, resident:string, detail:string}>} events
 */
export function renderFeed(events) {
  if (!feedBodyEl) return;
  if (!events || events.length === 0) {
    feedBodyEl.innerHTML = '<div class="feed-empty">還沒有動態……等居民開始活動後這裡就會熱鬧起來。</div>';
    return;
  }
  feedBodyEl.innerHTML = "";
  for (const ev of events) {
    const item = document.createElement("div");
    const kindSlug = (ev.kind || "").replace(/[^a-zA-Z一-鿿]/g, "");
    item.className = "feed-item kind-" + escHtml(ev.kind || "");
    const emoji = KIND_EMOJI[ev.kind] || "📌";
    item.innerHTML =
      '<div class="feed-who">' +
        '<span class="feed-who-kind">' + emoji + " " + escHtml(ev.kind || "") + "・" + escHtml(ev.resident || "") + '</span>' +
        '<span class="feed-who-time">' + formatRelativeTime(ev.ts_secs || 0) + '</span>' +
      '</div>' +
      '<div class="feed-detail">' + escHtml(ev.detail || "") + '</div>';
    feedBodyEl.appendChild(item);
  }
}

let feedVisible = false;
let feedRefreshTimer = null;

/** 開啟動態 Feed 面板並立刻抓取最新動態。 */
async function openFeed() {
  if (!feedEl) return;
  feedVisible = true;
  feedEl.style.display = "flex";
  await refreshFeed();
  // 每 30 秒自動刷新一次（面板開著時）。
  if (feedRefreshTimer) clearInterval(feedRefreshTimer);
  feedRefreshTimer = setInterval(() => { if (feedVisible) refreshFeed(); }, 30_000);
}

/** 關閉動態 Feed 面板。 */
function closeFeed() {
  feedVisible = false;
  if (feedEl) feedEl.style.display = "none";
  if (feedRefreshTimer) { clearInterval(feedRefreshTimer); feedRefreshTimer = null; }
}

/** 向後端抓最新 Feed 並重新渲染。 */
async function refreshFeed() {
  if (!feedBodyEl) return;
  try {
    const resp = await fetch("/voxel/feed");
    if (!resp.ok) throw new Error("feed fetch failed: " + resp.status);
    const events = await resp.json();
    renderFeed(events);
  } catch (err) {
    if (feedBodyEl) feedBodyEl.innerHTML = '<div class="feed-empty">無法讀取動態。</div>';
  }
}

// 綁定按鈕與關閉事件。
if (feedBtnEl) feedBtnEl.addEventListener("click", () => {
  feedVisible ? closeFeed() : openFeed();
});
if (feedEl) {
  const closeBtn = document.getElementById("feedClose");
  if (closeBtn) closeBtn.addEventListener("click", closeFeed);
}

// ── 準心選取 + 高亮外框（MCPE 風）──────────────────────────────────────────────
// 選中方塊的線框外框（略大一點點避免 z-fighting）。對準時顯示、沒對到時隱藏。
const highlight = new THREE.LineSegments(
  new THREE.EdgesGeometry(new THREE.BoxGeometry(1.002, 1.002, 1.002)),
  new THREE.LineBasicMaterial({ color: 0x101014, transparent: true, opacity: 0.9 })
);
highlight.visible = false;
scene.add(highlight);
// 目前準心對準的方塊：{ bx,by,bz（命中方塊）, nx,ny,nz（命中面法線，放置往這方向偏一格）}
let target = null;

// ── 快捷欄（選要放的方塊型別）+ 背包存量（採集 v1）───────────────────────────
// 種田 v1（ROADMAP 659）：加入農田土 + 種子（種子為純物品，特殊 Plant 動作）
// 快捷欄 11 格：GRASS DIRT STONE WOOD SAND LEAVES PLANK STONE_BRICK GLASS FARM_SOIL SEEDS
// 鍵盤 1–9 對應前 9 格；FARM_SOIL(10)、SEEDS(11) 以滑鼠/觸控點選
const HOTBAR = [GRASS, DIRT, STONE, WOOD, SAND, LEAVES, PLANK, STONE_BRICK, GLASS, FARM_SOIL, SEEDS];
const BLOCK_NAME = {
  [GRASS]: "草", [DIRT]: "土", [STONE]: "石", [WOOD]: "木", [SAND]: "沙", [LEAVES]: "葉",
  [PLANK]: "木板", [STONE_BRICK]: "石磚", [GLASS]: "玻璃",
  // 種田 v1
  [FARM_SOIL]: "農田土", [FARM_SOIL_SEEDED]: "幼苗", [WHEAT_MATURE]: "成熟小麥",
  [SEEDS]: "種子",
};
let selectedSlot = 0; // HOTBAR 索引
const hotbarEl = document.getElementById("hotbar");
// 本地材料存量（block_id → count）；由 inv_sync / inv_update 伺服器訊息維護。
const myInv = new Map();

/** 更新熱鍵欄的材料數量顯示（只改 .cnt 文字，不重建整個 DOM）。 */
function updateInvHud() {
  if (!hotbarEl) return;
  HOTBAR.forEach((b, i) => {
    const slot = hotbarEl.children[i];
    if (!slot) return;
    const cnt = slot.querySelector(".cnt");
    const n = myInv.get(b) || 0;
    if (cnt) cnt.textContent = n > 0 ? "×" + n : "";
    slot.classList.toggle("empty", n === 0);
  });
}

function buildHotbar() {
  if (!hotbarEl) return;
  hotbarEl.innerHTML = "";
  HOTBAR.forEach((b, i) => {
    const slot = document.createElement("div");
    slot.className = "slot" + (i === selectedSlot ? " sel" : "") + " empty";
    const sw = document.createElement("div");
    sw.className = "sw";
    const c = COLOR[b] || COLOR[STONE];
    sw.style.background = `rgb(${(c[0] * 255) | 0},${(c[1] * 255) | 0},${(c[2] * 255) | 0})`;
    const lbl = document.createElement("div");
    lbl.textContent = (i + 1) + " " + (BLOCK_NAME[b] || "?");
    const cnt = document.createElement("div");
    cnt.className = "cnt";
    slot.appendChild(sw); slot.appendChild(lbl); slot.appendChild(cnt);
    slot.addEventListener("pointerdown", (e) => { selectSlot(i); e.stopPropagation(); });
    hotbarEl.appendChild(slot);
  });
}
function selectSlot(i) {
  selectedSlot = ((i % HOTBAR.length) + HOTBAR.length) % HOTBAR.length;
  for (let k = 0; k < hotbarEl.children.length; k++) {
    hotbarEl.children[k].classList.toggle("sel", k === selectedSlot);
  }
}
function selectedBlock() { return HOTBAR[selectedSlot]; }
buildHotbar();
// 數字鍵 1..6 切快捷欄
addEventListener("keydown", (e) => {
  if (e.target && e.target.tagName === "INPUT") return; // 對話輸入中不搶鍵
  const n = parseInt(e.key, 10);
  if (n >= 1 && n <= HOTBAR.length) selectSlot(n - 1);
});

// AABB 是否與任一實心方塊重疊（碰撞核心）。
function overlaps() {
  const x0 = Math.floor(player.x - PW), x1 = Math.floor(player.x + PW);
  const y0 = Math.floor(player.y), y1 = Math.floor(player.y + PH - 0.01);
  const z0 = Math.floor(player.z - PW), z1 = Math.floor(player.z + PW);
  for (let bx = x0; bx <= x1; bx++)
    for (let by = y0; by <= y1; by++)
      for (let bz = z0; bz <= z1; bz++)
        if (solidCollide(bx, by, bz)) return true;
  return false;
}

// 水平移動一軸：撞牆就回退；若站在地上，試著踏上 1 格高台階（讓走斜坡/小丘順暢）。
// 踏上台階時：物理 Y 瞬間到位（碰撞/重力繼續正確運作），同時累積 stepSmooth 讓視覺 Y
// 從原地平滑抬升（update() 每幀指數衰減），消除以往「瞬間彈跳一格」的閃爍感。
function moveAxis(axis, delta) {
  if (delta === 0) return;
  const prev = player[axis];
  player[axis] += delta;
  if (!overlaps()) return;
  if (player.grounded) {
    const py = player.y;
    player.y += 1.05;
    if (!overlaps()) {
      // 踏上台階成功：物理 Y 已到位；累積視覺補間偏移（visualY 由 update() 平滑追上）
      stepSmooth += player.y - py;
      return;
    }
    player.y = py;
  }
  player[axis] = prev; // 完全擋住 → 回退
}

// ── Voxel raycast（自寫 DDA 體素行進；不抄外部碼）──────────────────────────────
// 從原點 (ox,oy,oz) 沿單位方向 (dx,dy,dz) 一格一格走，回傳第一個非空氣/非水的實心方塊，
// 連同「進入該方塊時跨過的面法線」(nx,ny,nz)——放置時往這方向偏一格即面外側。
const RAY_MAX = 6.0; // 觸及距離（與後端 REACH 對齊）
function raycastVoxel(ox, oy, oz, dx, dy, dz) {
  let bx = Math.floor(ox), by = Math.floor(oy), bz = Math.floor(oz);
  const stepX = dx > 0 ? 1 : -1, stepY = dy > 0 ? 1 : -1, stepZ = dz > 0 ? 1 : -1;
  const tDeltaX = dx !== 0 ? Math.abs(1 / dx) : Infinity;
  const tDeltaY = dy !== 0 ? Math.abs(1 / dy) : Infinity;
  const tDeltaZ = dz !== 0 ? Math.abs(1 / dz) : Infinity;
  // 到下一個格界的參數距離。
  const fx = dx > 0 ? (bx + 1 - ox) : (ox - bx);
  const fy = dy > 0 ? (by + 1 - oy) : (oy - by);
  const fz = dz > 0 ? (bz + 1 - oz) : (oz - bz);
  let tMaxX = dx !== 0 ? fx * tDeltaX : Infinity;
  let tMaxY = dy !== 0 ? fy * tDeltaY : Infinity;
  let tMaxZ = dz !== 0 ? fz * tDeltaZ : Infinity;
  let nx = 0, ny = 0, nz = 0, t = 0;
  for (let guard = 0; guard < 64; guard++) {
    const r = getRaw(bx, by, bz);
    if (r > 0 && r !== WATER) return { bx, by, bz, nx, ny, nz };
    if (tMaxX < tMaxY && tMaxX < tMaxZ) {
      bx += stepX; t = tMaxX; tMaxX += tDeltaX; nx = -stepX; ny = 0; nz = 0;
    } else if (tMaxY < tMaxZ) {
      by += stepY; t = tMaxY; tMaxY += tDeltaY; nx = 0; ny = -stepY; nz = 0;
    } else {
      bz += stepZ; t = tMaxZ; tMaxZ += tDeltaZ; nx = 0; ny = 0; nz = -stepZ;
    }
    if (t > RAY_MAX) break;
  }
  return null;
}

// 視線方向（含俯仰）：從鏡頭中心穿過準心的方向 = 鏡頭看向 target 的方向。
function viewDir() {
  // 與 update() 的鏡頭擺位一致：鏡頭在玩家後上方、看向玩家頭頂。
  const tx = player.x, ty = player.y + 1.3, tz = player.z;
  const dist = 6.0, cp = Math.cos(camPitch), sp = Math.sin(camPitch);
  const camx = tx + Math.sin(player.yaw) * dist * cp;
  const camy = ty + dist * sp;
  const camz = tz + Math.cos(player.yaw) * dist * cp;
  const d = new THREE.Vector3(tx - camx, ty - camy, tz - camz);
  d.normalize();
  return d;
}

// 更新準心對準的方塊（每幀算）：從玩家眼睛沿視線 raycast。
function updateTarget() {
  const dir = viewDir();
  const eye = { x: player.x, y: player.y + 1.5, z: player.z };
  target = raycastVoxel(eye.x, eye.y, eye.z, dir.x, dir.y, dir.z);
  if (target) {
    highlight.visible = true;
    highlight.position.set(target.bx + 0.5, target.by + 0.5, target.bz + 0.5);
  } else {
    highlight.visible = false;
  }
}

// 本地套用一個方塊更新（伺服器廣播 / 樂觀預測共用）：改 chunk 資料 + 標記受影響 chunk 重建。
// 只重建該 chunk（及鄰塊，邊界面剔除用），絕不整個世界重建（延續切片① FPS 鐵律）。
function setLocalBlock(wx, wy, wz, b) {
  const cx = Math.floor(wx / CHUNK), cy = Math.floor(wy / CHUNK), cz = Math.floor(wz / CHUNK);
  const ch = chunks.get(ckey(cx, cy, cz));
  if (!ch) return; // 該 chunk 還沒載入——之後串流會帶正確（含 delta）的版本。
  const lx = wx - cx * CHUNK, ly = wy - cy * CHUNK, lz = wz - cz * CHUNK;
  ch[lx + lz * CHUNK + ly * CHUNK * CHUNK] = b;
  markDirty(cx, cy, cz); // markDirty 只標該 chunk + 6 鄰塊
}

// 破壞準心對準的方塊：送 break（伺服器驗證後廣播 → setLocalBlock 套用）。回傳被挖座標或 null。
function breakAtTarget() {
  if (!target || !wsReady) return null;
  const c = { x: target.bx, y: target.by, z: target.bz };
  ws.send(JSON.stringify({ t: "break", x: c.x, y: c.y, z: c.z }));
  return c;
}
// 在準心方塊的「面外側」放一個方塊：座標 = 命中方塊 + 命中面法線。回傳放置座標或 null。
// 種田 v1：若持有種子且命中的是農田土 → 送 plant（種植動作），而非一般 place。
function placeAtTarget() {
  if (!target || !wsReady) return null;
  // 種子的特殊種植動作：目標是農田土本身（不偏移到面外側）。
  if (selectedBlock() === SEEDS) {
    const hitRaw = getRaw(target.bx, target.by, target.bz);
    if (hitRaw === FARM_SOIL) {
      ws.send(JSON.stringify({ t: "plant", x: target.bx, y: target.by, z: target.bz }));
      return { x: target.bx, y: target.by, z: target.bz };
    }
    // 種子只能種在農田土上——其他方塊靜默忽略。
    return null;
  }
  // 一般放置：在命中方塊的面外側放置。
  const px = target.bx + target.nx, py = target.by + target.ny, pz = target.bz + target.nz;
  // 別把方塊放進自己身體（避免卡死）。
  if (px === Math.floor(player.x) && pz === Math.floor(player.z) &&
      (py === Math.floor(player.y) || py === Math.floor(player.y + 1))) return null;
  ws.send(JSON.stringify({ t: "place", x: px, y: py, z: pz, b: selectedBlock() }));
  return { x: px, y: py, z: pz };
}

// ── 輸入 ───────────────────────────────────────────────────────────────────
const keys = {};
addEventListener("keydown", (e) => {
  if (e.target && e.target.tagName === "INPUT") return; // 對話輸入中不觸發移動
  keys[e.code] = true; if (e.code === "Space") e.preventDefault();
});
addEventListener("keyup", (e) => { keys[e.code] = false; });

// 滑鼠：拖曳轉鏡頭；「點一下」（位移很小）＝對準心動作。左鍵破壞、右鍵放置（MCPE 範式）。
let camPitch = 0.35;
let dragging = false, lastX = 0, lastY = 0;
let downX = 0, downY = 0, downBtn = 0, moved = 0;
const TAP_PX = 6; // 位移小於此視為「點擊」而非拖曳
renderer.domElement.addEventListener("pointerdown", (e) => {
  dragging = true; lastX = e.clientX; lastY = e.clientY;
  downX = e.clientX; downY = e.clientY; downBtn = e.button; moved = 0;
});
addEventListener("pointerup", (e) => {
  if (dragging && moved < TAP_PX) {
    // 點擊：右鍵放；左鍵先看是否點到居民（開對話），否則挖。
    if (downBtn === 2) {
      placeAtTarget();
    } else {
      const rid = pickResident(e.clientX, e.clientY);
      if (rid) { const ent = residents.get(rid); openChat(rid, ent && ent.lastName); }
      else breakAtTarget();
    }
  }
  dragging = false;
});
addEventListener("pointermove", (e) => {
  if (!dragging) return;
  moved += Math.abs(e.clientX - lastX) + Math.abs(e.clientY - lastY);
  player.yaw -= (e.clientX - lastX) * 0.005;
  camPitch = Math.max(-0.2, Math.min(1.3, camPitch + (e.clientY - lastY) * 0.005));
  lastX = e.clientX; lastY = e.clientY;
});
// 右鍵放置：擋掉瀏覽器選單。
renderer.domElement.addEventListener("contextmenu", (e) => e.preventDefault());

// 觸控搖桿（isTouch 常數已在頁首定義）
const touchEl = document.getElementById("touch");
let joyVec = { x: 0, y: 0 };
if (isTouch) {
  if (touchEl) touchEl.style.display = "block";
  const joy = document.getElementById("joy"), nub = document.getElementById("joyNub");
  let joyId = null, jcx = 0, jcy = 0;
  joy.addEventListener("touchstart", (e) => {
    const t = e.changedTouches[0]; joyId = t.identifier;
    const r = joy.getBoundingClientRect(); jcx = r.left + r.width / 2; jcy = r.top + r.height / 2;
    e.preventDefault();
  }, { passive: false });
  addEventListener("touchmove", (e) => {
    for (const t of e.changedTouches) {
      if (t.identifier !== joyId) continue;
      let dx = (t.clientX - jcx) / 50, dy = (t.clientY - jcy) / 50;
      dx = Math.max(-1, Math.min(1, dx)); dy = Math.max(-1, Math.min(1, dy));
      joyVec.x = dx; joyVec.y = dy;
      nub.style.left = (35 + dx * 30) + "px"; nub.style.top = (35 + dy * 30) + "px";
    }
  }, { passive: false });
  addEventListener("touchend", (e) => {
    for (const t of e.changedTouches) if (t.identifier === joyId) { joyId = null; joyVec = { x: 0, y: 0 }; nub.style.left = "35px"; nub.style.top = "35px"; }
  });
  // 視角轉動：在非搖桿區拖曳。「點一下」（位移小）＝對準心破壞（MCPE 點破壞範式）。
  let camId = null, cx0 = 0, cy0 = 0, camMoved = 0;
  renderer.domElement.addEventListener("touchstart", (e) => {
    const t = e.changedTouches[0]; camId = t.identifier; cx0 = t.clientX; cy0 = t.clientY; camMoved = 0;
  });
  renderer.domElement.addEventListener("touchmove", (e) => {
    for (const t of e.changedTouches) {
      if (t.identifier !== camId) continue;
      camMoved += Math.abs(t.clientX - cx0) + Math.abs(t.clientY - cy0);
      player.yaw -= (t.clientX - cx0) * 0.006;
      camPitch = Math.max(-0.2, Math.min(1.3, camPitch + (t.clientY - cy0) * 0.006));
      cx0 = t.clientX; cy0 = t.clientY;
    }
  }, { passive: false });
  renderer.domElement.addEventListener("touchend", (e) => {
    for (const t of e.changedTouches) {
      if (t.identifier !== camId) continue;
      if (camMoved < 8) {
        // 輕點：先看是否點到居民（開對話），否則挖。
        const rid = pickResident(t.clientX, t.clientY);
        if (rid) { const ent = residents.get(rid); openChat(rid, ent && ent.lastName); }
        else breakAtTarget();
      }
      camId = null;
    }
  });
  const jumpBtn = document.getElementById("jump");
  jumpBtn.addEventListener("touchstart", (e) => { tryJump(); e.preventDefault(); }, { passive: false });
  const placeBtn = document.getElementById("place");
  placeBtn.addEventListener("touchstart", (e) => { placeAtTarget(); e.preventDefault(); }, { passive: false });
}

function tryJump() { if (player.grounded) { player.vy = 8.2; player.grounded = false; } }

// ── WebSocket（/voxel/ws）─────────────────────────────────────────────────
let ws = null, wsReady = false;
function connect() {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  ws = new WebSocket(`${proto}://${location.host}/voxel/ws`);
  ws.onopen = () => {
    wsReady = true;
    let nm = "旅人";
    try { nm = localStorage.getItem("butfun_name") || "旅人"; } catch (e) { /* ignore */ }
    ws.send(JSON.stringify({ t: "join", name: nm }));
  };
  ws.onmessage = (ev) => {
    let m; try { m = JSON.parse(ev.data); } catch (e) { return; }
    if (m.t === "welcome") {
      myId = m.id; myName = m.name || "旅人";
      player.x = m.spawn.x; player.y = m.spawn.y; player.z = m.spawn.z;
      // 好感度 v1：連線後立即拉取與各居民的好感度，讓指示燈盡快亮起。
      refreshAffinity();
    } else if (m.t === "chunks") {
      for (const c of m.chunks) {
        const key = ckey(c.cx, c.cy, c.cz);
        chunks.set(key, b64ToBytes(c.data));
        markDirty(c.cx, c.cy, c.cz);
      }
    } else if (m.t === "block") {
      // 伺服器權威方塊更新（破壞/放置）：本地套用 + 只重建受影響 chunk。
      setLocalBlock(m.x, m.y, m.z, m.b);
    } else if (m.t === "players") {
      const seen = new Set();
      for (const p of m.players) {
        if (p.id === myId) continue;
        seen.add(p.id);
        let mesh = others.get(p.id);
        if (!mesh) { mesh = new THREE.Mesh(bodyGeo, otherMat); scene.add(mesh); others.set(p.id, mesh); }
        mesh.position.set(p.x, p.y + PH / 2, p.z);
        mesh.rotation.y = p.yaw || 0;
      }
      for (const [id, mesh] of others) if (!seen.has(id)) { scene.remove(mesh); others.delete(id); }
      // 乙太方界 AI 居民（與玩家分開的陣列）：位置/名字/說的話。
      if (m.residents) updateResidents(m.residents);
      // 晝夜循環 v1：伺服器每幀帶 time_of_day(0.0–1.0)，前端據此更新天空/光照。
      if (typeof m.time_of_day === "number") {
        worldTime = m.time_of_day;
        updateSkyAndLight(worldTime);
      }
    } else if (m.t === "talk") {
      // 居民對話回覆（單播）：
      //   thinking:true → 立即佔位（後端一收到就送），顯示動畫「思考中」指示器，不當一般氣泡。
      //   thinking 不存在（預設 false）→ LLM 真回覆，移除思考中指示器，顯示正常回覆氣泡。
      if (m.thinking) {
        showThinking(m.name); // 顯示「露娜 思考中 ●●●」動畫
      } else {
        removeThinking();     // 真回覆到了，先移除「思考中」
        lastTalkReply = m.reply || "";
        appendMsg("npc", (m.name || "居民") + "：" + lastTalkReply);
        // 好感度 v1：對話後更新好感度（後端可能已累積新記憶），讓指示燈即時升燈。
        refreshAffinity();
      }
    } else if (m.t === "inv_sync") {
      // 採集 v1：連線後收到背包全量快照，重置本地存量。
      myInv.clear();
      for (const [bid, cnt] of (m.items || [])) {
        if (cnt > 0) myInv.set(bid, cnt);
      }
      updateInvHud();
      updateGiftBtn(); // 贈禮 v1：背包恢復後同步更新按鈕
    } else if (m.t === "inv_update") {
      // 採集 v1：單一材料增減後的新存量（伺服器回傳 total，非 delta）。
      if (m.count > 0) myInv.set(m.block_id, m.count);
      else myInv.delete(m.block_id);
      updateInvHud();
      updateGiftBtn(); // 贈禮 v1：材料變動後同步更新按鈕
    } else if (m.t === "inv_denied") {
      // 採集 v1：放置材料不足，短暫提示。
      const bname = BLOCK_NAME[m.block_id] || "方塊";
      showErr("材料不足：" + bname + "（先去挖一些吧）");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "craft_ok") {
      // 合成台 v1：合成成功，短暫提示並刷新合成面板。
      const oname = BLOCK_NAME[m.out_count > 0 ? RECIPES_JS.find(r => r.id === m.recipe_id)?.output_block : 0] || m.name_zh;
      showMsg("合成成功：" + m.name_zh + " ×" + m.out_count + "！");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2200);
      renderCraftPanel();
    } else if (m.t === "craft_fail") {
      // 合成台 v1：材料不足，短暫提示。
      showErr("材料不足，無法合成（先多採集一些）");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
      renderCraftPanel();
    } else if (m.t === "plant_ok") {
      // 種田 v1：種植成功，短暫提示。
      showMsg("已種下種子！等 90 秒小麥就成熟 🌾");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2500);
    } else if (m.t === "plant_fail") {
      // 種田 v1：種植失敗（非農田土 / 沒種子 / 太遠），短暫提示。
      showErr("種植失敗：" + (m.reason || "未知原因"));
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "gift_ok") {
      // 贈禮 v1：送禮成功——居民道謝訊息顯示在對話框；更新贈禮鈕顯示。
      const iname = BLOCK_NAME[m.item_id] || m.item_name || "物品";
      appendMsg("sys", "✨ 你送出了 " + iname + " 給 " + (m.resident_name || "居民"));
      updateGiftBtn(); // 背包已更新，重算鈕
    } else if (m.t === "gift_fail") {
      // 贈禮 v1：送禮失敗（太遠 / 沒材料）。
      showErr(m.reason || "無法送禮");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    }
  };
  ws.onclose = () => { wsReady = false; showErr("連線中斷，重新連線中…"); setTimeout(connect, 1500); };
  ws.onerror = () => { showErr("連線錯誤"); };
}
connect();

// 走到哪、補要哪：請求玩家周邊半徑內、尚未載入也沒要過的 column。
let reqTimer = 0;
function streamChunks(dt) {
  reqTimer -= dt;
  if (!wsReady || reqTimer > 0) return;
  reqTimer = 0.25;
  const pcx = Math.floor(player.x / CHUNK), pcz = Math.floor(player.z / CHUNK);
  const R = 3;
  let sent = 0;
  for (let r = 0; r <= R && sent < 3; r++) {
    for (let dx = -r; dx <= r && sent < 3; dx++) {
      for (let dz = -r; dz <= r && sent < 3; dz++) {
        if (Math.max(Math.abs(dx), Math.abs(dz)) !== r) continue; // 由近到遠的環
        const cx = pcx + dx, cz = pcz + dz, k = cx + "," + cz;
        if (requested.has(k)) continue;
        // 該 column 任一 cy 已載入就算有了
        if (chunks.has(ckey(cx, 0, cz)) || chunks.has(ckey(cx, 1, cz))) { requested.add(k); continue; }
        requested.add(k);
        ws.send(JSON.stringify({ t: "req", cx, cz }));
        sent++;
      }
    }
  }
}

// 位置上報（節流）
let sendTimer = 0;
function sendMove(dt) {
  sendTimer -= dt;
  if (!wsReady || sendTimer > 0) return;
  sendTimer = 0.1;
  ws.send(JSON.stringify({ t: "move", x: player.x, y: player.y, z: player.z, yaw: player.yaw }));
}

// ── 主迴圈 ─────────────────────────────────────────────────────────────────
const SPEED = 5.0, GRAVITY = 24.0;
let last = performance.now();
let frames = 0, fpsT = 0, fps = 0;
let dbgT = 0;

function update(dt) {
  // 方向（相對鏡頭 yaw）
  const fwd = new THREE.Vector3(-Math.sin(player.yaw), 0, -Math.cos(player.yaw));
  const right = new THREE.Vector3(Math.cos(player.yaw), 0, -Math.sin(player.yaw));
  let mx = 0, mz = 0;
  if (keys["KeyW"] || keys["ArrowUp"]) mz += 1;
  if (keys["KeyS"] || keys["ArrowDown"]) mz -= 1;
  if (keys["KeyD"] || keys["ArrowRight"]) mx += 1;
  if (keys["KeyA"] || keys["ArrowLeft"]) mx -= 1;
  // 觸控搖桿（y 往上＝前進）
  mz += -joyVec.y; mx += joyVec.x;
  if ((keys["Space"]) && player.grounded) tryJump();

  const dir = new THREE.Vector3();
  dir.addScaledVector(fwd, mz).addScaledVector(right, mx);
  if (dir.lengthSq() > 1e-4) {
    dir.normalize();
    moveAxis("x", dir.x * SPEED * dt);
    moveAxis("z", dir.z * SPEED * dt);
  }

  // 重力 + 垂直碰撞
  player.vy -= GRAVITY * dt;
  // 限制單幀垂直位移避免穿牆
  let dy = Math.max(-1.5, Math.min(1.5, player.vy * dt));
  const prevY = player.y;
  player.y += dy;
  if (overlaps()) {
    player.y = prevY;
    if (player.vy < 0) player.grounded = true;
    player.vy = 0;
  } else {
    if (player.vy < 0) player.grounded = false;
  }
  // 掉出世界保險：低於 -10 拉回出生高度
  if (player.y < -10) { player.y = 40; player.vy = 0; stepSmooth = 0; }

  // 踏階視覺補間衰減（frame-rate 無關的指數平滑）
  // stepSmooth > 0 → 視覺 Y 低於物理 Y；每幀靠近直到 < 0.005 格就吸附歸零。
  // 重力下落時 stepSmooth 保持 0，不影響往下的動態。
  if (stepSmooth > 0) {
    stepSmooth *= Math.exp(-STEP_SMOOTH_K * dt);
    if (stepSmooth < 0.005) stepSmooth = 0;
  }
  // visualY：bodyMesh 與鏡頭看向點用此值——踏階時從原地平滑升上去，消除瞬跳閃爍。
  const visualY = player.y - stepSmooth;

  // 玩家身體 + 朝向（用 visualY 避免角色瞬跳一格）
  bodyMesh.position.set(player.x, visualY + PH / 2, player.z);
  if (dir.lengthSq() > 1e-4) bodyMesh.rotation.y = Math.atan2(dir.x, dir.z);

  // 第三人稱鏡頭跟隨（用 visualY 讓鏡頭也跟著平滑升，不突然跳）
  const lookTarget = new THREE.Vector3(player.x, visualY + 1.3, player.z);
  const dist = 6.0, cp = Math.cos(camPitch), sp = Math.sin(camPitch);
  camera.position.set(
    lookTarget.x + Math.sin(player.yaw) * dist * cp,
    lookTarget.y + dist * sp,
    lookTarget.z + Math.cos(player.yaw) * dist * cp
  );
  camera.lookAt(lookTarget);

  // 準心對準的方塊（破壞/放置目標）+ 高亮外框。
  updateTarget();

  streamChunks(dt);
  sendMove(dt);

  // 每幀重建少量 dirty chunk（分攤成本）
  let built = 0;
  for (const key of dirty) {
    rebuildChunk(key);
    dirty.delete(key);
    if (++built >= 4) break;
  }
}

function safeRender() {
  renderer.render(scene, camera);
}

function loop() {
  const now = performance.now();
  let dt = (now - last) / 1000; last = now;
  if (dt > 0.1) dt = 0.1; // 分頁切回來別跳一大步
  try {
    update(dt);
    safeRender();
  } catch (e) {
    // render 一拋會永久停 rAF（畫面凍結）——抓住、印出、自我恢復（比照 3D safeRender 護網）。
    console.error("[voxel] 迴圈例外：", e);
    showErr("渲染例外（已自我恢復，見 console）：" + (e && e.message ? e.message : e));
  }

  // FPS / HUD
  frames++; fpsT += dt;
  if (fpsT >= 0.5) { fps = frames / fpsT; frames = 0; fpsT = 0; }
  dbgT += dt;
  if (dbgT >= 0.25) {
    dbgT = 0;
    // 觸控裝置顯示精簡文字，避免直式螢幕頂部 HUD 溢出
    hudEl.textContent = isTouch
      ? `乙太方界 · ${myName}\n輕點挖・放置鈕放\nchunk:${chunks.size} 線上:${others.size + 1} 居民:${residents.size}`
      : `乙太方界 · ${myName}\nWASD移動·拖曳轉視角·空白跳\n左鍵/輕點挖·右鍵/放置鈕放·1-6選方塊\nchunk: ${chunks.size}　線上: ${others.size + 1}　居民: ${residents.size}`;
    if (DEBUG) {
      dbgEl.style.display = "block";
      dbgEl.textContent =
        `FPS ${fps.toFixed(0)}\n` +
        `chunks ${chunks.size}  meshes ${meshes.size}\n` +
        `pos ${player.x.toFixed(1)},${player.y.toFixed(1)},${player.z.toFixed(1)}\n` +
        `grounded ${player.grounded}\n` +
        `build ${window.__BUILD__ || "?"}`;
    }
  }
  requestAnimationFrame(loop);
}
requestAnimationFrame(loop);

addEventListener("resize", () => {
  camera.aspect = window.innerWidth / window.innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(window.innerWidth, window.innerHeight);
});

// ── 合成台 v1（ROADMAP 658）──────────────────────────────────────────────────
// 前端配方表（對齊後端 voxel_craft::RECIPES，id/inputs/output 穩定契約）。
const RECIPES_JS = [
  { id: "plank",        name: "木板",   inputs: [[WOOD, 2]],  output_block: PLANK,       out_count: 4 },
  { id: "stone_brick",  name: "石磚",   inputs: [[STONE, 2]], output_block: STONE_BRICK, out_count: 2 },
  { id: "glass",        name: "玻璃",   inputs: [[SAND, 2]],  output_block: GLASS,       out_count: 1 },
  { id: "till",         name: "農田土", inputs: [[DIRT, 2]],  output_block: FARM_SOIL,   out_count: 2 },
];

const craftPanelEl = document.getElementById("craft");
const craftBodyEl  = document.getElementById("craftBody");

function openCraftPanel() {
  if (!craftPanelEl) return;
  craftPanelEl.style.display = "flex";
  renderCraftPanel();
}
function closeCraftPanel() {
  if (craftPanelEl) craftPanelEl.style.display = "none";
}

/** 渲染合成面板——列出每條配方、即時顯示材料是否充足，純函式組 DOM。 */
function renderCraftPanel() {
  if (!craftBodyEl) return;
  craftBodyEl.innerHTML = "";
  RECIPES_JS.forEach(r => {
    const canCraft = r.inputs.every(([bid, cnt]) => (myInv.get(bid) || 0) >= cnt);
    const row = document.createElement("div");
    row.className = "craft-row" + (canCraft ? "" : " craft-disabled");
    // 配料欄
    const inp = document.createElement("div");
    inp.className = "craft-inp";
    r.inputs.forEach(([bid, cnt]) => {
      const have = myInv.get(bid) || 0;
      const s = document.createElement("span");
      s.className = "craft-mat" + (have >= cnt ? " ok" : " short");
      s.textContent = (BLOCK_NAME[bid] || "?") + " ×" + cnt + " (" + have + ")";
      inp.appendChild(s);
    });
    // 箭頭
    const arr = document.createElement("span");
    arr.className = "craft-arr";
    arr.textContent = "→";
    // 產出欄
    const out = document.createElement("span");
    out.className = "craft-out";
    const oc = COLOR[r.output_block] || COLOR[STONE];
    out.innerHTML =
      '<span class="craft-swatch" style="background:rgb(' +
      ((oc[0] * 255) | 0) + "," + ((oc[1] * 255) | 0) + "," + ((oc[2] * 255) | 0) +
      ')"></span>' + r.name + " ×" + r.out_count;
    // 合成鈕
    const btn = document.createElement("button");
    btn.className = "craft-btn";
    btn.textContent = "合成";
    btn.disabled = !canCraft;
    btn.addEventListener("click", () => {
      if (!wsReady) return;
      ws.send(JSON.stringify({ t: "craft", recipe_id: r.id }));
    });
    row.appendChild(inp); row.appendChild(arr); row.appendChild(out); row.appendChild(btn);
    craftBodyEl.appendChild(row);
  });
}

// 合成鈕（🔨）開關合成面板。
const craftBtnEl = document.getElementById("craftBtn");
if (craftBtnEl) craftBtnEl.addEventListener("click", (e) => {
  if (!craftPanelEl) return;
  if (craftPanelEl.style.display === "none" || !craftPanelEl.style.display) {
    openCraftPanel();
  } else {
    closeCraftPanel();
  }
  e.stopPropagation();
});
// 點面板外關閉（與日記、Feed 面板保持一致）。
document.addEventListener("pointerdown", (e) => {
  if (craftPanelEl && craftPanelEl.style.display === "flex") {
    if (!craftPanelEl.contains(e.target) && e.target !== craftBtnEl) closeCraftPanel();
  }
});

// 合成台面板關閉鈕（✕）。
const craftCloseEl = document.getElementById("craftClose");
if (craftCloseEl) craftCloseEl.addEventListener("click", closeCraftPanel);

/** 簡短綠色提示（合成成功用；區別於 showErr 紅色錯誤）。 */
function showMsg(text) {
  const el = document.getElementById("msg");
  if (!el) return;
  el.textContent = text;
  el.style.display = "block";
  clearTimeout(el._hideTimer);
  el._hideTimer = setTimeout(() => { el.style.display = "none"; }, 3000);
}

// 對外暴露一點狀態，方便真瀏覽器 QA 讀數驗證。
window.__voxel = {
  get chunks() { return chunks.size; },
  get meshes() { return meshes.size; },
  get fps() { return fps; },
  get player() { return player; },
  // ── 踏階平滑 QA 用：讀視覺 Y（平滑後）與補間偏移 ──
  get stepSmooth() { return stepSmooth; },
  get visualY() { return player.y - stepSmooth; },
  // 乙太方界 AI 居民（QA 用）：數量 + 位置/名字/說的話快照。
  get residentCount() { return residents.size; },
  residentInfo() {
    return [...residents.entries()].map(([id, e]) => ({
      id, name: e.lastName, say: e.lastSay,
      x: e.group.position.x, y: e.group.position.y, z: e.group.position.z,
      visible: e.group.visible,
    }));
  },
  // ── 對話 QA 用：列居民 id、直接對某居民送一句、讀最近回覆 ──
  residentIds() { return [...residents.keys()]; },
  talkTo(rid, text) {
    const ent = residents.get(rid);
    openChat(rid, ent && ent.lastName);
    sendTalk(text);
    return chatRid;
  },
  get lastTalkReply() { return lastTalkReply; },
  closeChat() { closeChat(); },
  // ── 日記 QA 用（ROADMAP 650）──
  openDiary(rid) { return openDiary(rid); },
  closeDiary() { closeDiary(); },
  get diaryVisible() { return diaryEl ? diaryEl.style.display !== "none" : false; },
  // ── 動態 Feed QA 用（ROADMAP 655）──
  openFeed() { return openFeed(); },
  closeFeed() { closeFeed(); },
  get feedVisible() { return feedVisible; },
  renderFeed(ev) { renderFeed(ev); return feedBodyEl && feedBodyEl.innerHTML; },
  // ── 好感度 QA 用（ROADMAP 656）──
  affinityEmoji(count) { return affinityEmoji(count); },
  get myAffinity() { return Object.fromEntries(myAffinity); },
  refreshAffinity() { return refreshAffinity(); },
  // ── 採集背包 QA 用（ROADMAP 657）──
  get myInv() { return Object.fromEntries(myInv); },
  setInvForTest(bid, cnt) { if (cnt > 0) myInv.set(bid, cnt); else myInv.delete(bid); updateInvHud(); },
  updateInvHud() { updateInvHud(); },
  // ── 合成台 QA 用（ROADMAP 658）──
  get craftPanelVisible() { return craftPanelEl ? craftPanelEl.style.display === "flex" : false; },
  openCraftPanel() { openCraftPanel(); },
  closeCraftPanel() { closeCraftPanel(); },
  renderCraftPanel() { renderCraftPanel(); return craftBodyEl ? craftBodyEl.innerHTML : ""; },
  get RECIPES_JS() { return RECIPES_JS; },
  PLANK, STONE_BRICK, GLASS,
  // 種田 v1 常數 + QA 介面
  FARM_SOIL, FARM_SOIL_SEEDED, WHEAT_MATURE, SEEDS,
  // ── 贈禮 v1 QA 介面（ROADMAP 660）──
  giftPickItem(inv) { return giftPickItem(inv); },
  updateGiftBtn() { updateGiftBtn(); },
  get giftBtnText() { const e = document.getElementById("chatGift"); return e ? e.textContent : ""; },
  get giftBtnEmpty() { const e = document.getElementById("chatGift"); return e ? e.classList.contains("gift-empty") : false; },
  GIFT_EXCLUDED: [...GIFT_EXCLUDED],
  // ── 晝夜循環 v1 QA 用（ROADMAP 661）──
  get worldTime() { return worldTime; },
  updateSkyAndLight(t) { updateSkyAndLight(t); },
  get skyColor() { const c = scene.background; return { r: c.r, g: c.g, b: c.b }; },
  get sunIntensity() { return sun.intensity; },
  get hemiIntensity() { return hemi.intensity; },
  // ── 真瀏覽器 QA 用：讀準心目標、讀方塊、觸發破壞/放置、選方塊 ──
  get target() { return target; },
  getBlock(x, y, z) { return getRaw(x, y, z); },
  doBreak() { return breakAtTarget(); },
  doPlace() { return placeAtTarget(); },
  selectSlotByBlock(b) { const i = HOTBAR.indexOf(b); if (i >= 0) selectSlot(i); return selectedBlock(); },
};
