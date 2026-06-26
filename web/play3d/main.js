// ButFun — /play3d/ 可玩 3D 原型
// 目標：維護者開一個網址，真的操控一個角色在 3D 空間裡跑、跳、跳上平台、收集發光球。
// 求「會動、好玩、手機能玩」，不求完整或精緻。
//
// 核心原則：
//   - 這是「玩法」原型，不同於 /3d/ 那個唯讀觀察頁。要能操控 + 跳 + 立體碰撞。
//   - 客戶端物理：自己手寫重力 + 跳躍 + AABB 地面/平台碰撞（原型先求手感）。
//     伺服器權威 3D 是之後的事，本原型完全不碰後端 / world-core / game.js。
//   - 沒風格的風格：盒子／膠囊／低多邊形，零美術資產，實體全程式生成。

import * as THREE from "three";

// ---- 錯誤浮層：任何例外不白屏，把訊息顯示出來 ----
const errEl = document.getElementById("err");
function showErr(msg) {
  if (!errEl) return;
  errEl.style.display = "flex";
  errEl.textContent = "出錯了，但不白屏：\n" + msg;
}
window.addEventListener("error", (e) => showErr(e.message || String(e.error || e)));
window.addEventListener("unhandledrejection", (e) => showErr(String(e.reason)));

const hudEl = document.getElementById("hud");

// ============================================================
// Three.js 基礎場景
// ============================================================
const app = document.getElementById("app");
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x141a26);
scene.fog = new THREE.Fog(0x141a26, 60, 160);

const camera = new THREE.PerspectiveCamera(60, window.innerWidth / window.innerHeight, 0.1, 500);
camera.position.set(0, 8, 12);

const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setSize(window.innerWidth, window.innerHeight);
// 手機友善：pixelRatio 設上限，免得高 DPI 手機算爆 GPU
renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
app.appendChild(renderer.domElement);

// 燈光：半球光給柔和環境色 + 一盞方向光給立體感與影子方向
scene.add(new THREE.HemisphereLight(0xcfe0ff, 0x20262e, 1.0));
const sun = new THREE.DirectionalLight(0xffffff, 1.4);
sun.position.set(20, 40, 16);
scene.add(sun);

// ============================================================
// 世界：碰撞用的純資料（boxes / ramps）+ 對應的低多邊形 mesh
// 物理只認 boxes / ramps 陣列；mesh 只是畫面，兩者分開比較好除錯。
// ============================================================
const boxes = [];   // { min:Vector3, max:Vector3 }
const ramps = [];   // { xmin,xmax,zmin,zmax, lowY, highY, axis:'x'|'z', lowAt }

// 以中心 + 尺寸加一個盒子（同時建碰撞資料與 mesh）。
function addBox(cx, cy, cz, w, h, d, color) {
  const min = new THREE.Vector3(cx - w / 2, cy - h / 2, cz - d / 2);
  const max = new THREE.Vector3(cx + w / 2, cy + h / 2, cz + d / 2);
  boxes.push({ min, max });
  const mesh = new THREE.Mesh(
    new THREE.BoxGeometry(w, h, d),
    new THREE.MeshLambertMaterial({ color })
  );
  mesh.position.set(cx, cy, cz);
  scene.add(mesh);
  return { min, max };
}

// 以「頂面高度」加平台：top 是站上去的高度，h 是厚度（中心自動往下推）。
function addPlatform(cx, top, cz, w, d, color, h = 0.6) {
  return addBox(cx, top - h / 2, cz, w, h, d, color);
}

// 斜坡：在 footprint 內，沿某軸從 lowY 線性升到 highY，可走上去。
// axis='x' 表示沿 x 方向爬升；lowAt 是低邊的座標值。
function addRamp(xmin, xmax, zmin, zmax, lowY, highY, axis, color) {
  const lowAt = axis === "x" ? xmin : zmin;
  ramps.push({ xmin, xmax, zmin, zmax, lowY, highY, axis, lowAt });
  // 畫面：用一個被旋轉的薄盒當斜面（純視覺，碰撞走 height-field）
  const len = axis === "x" ? xmax - xmin : zmax - zmin;
  const width = axis === "x" ? zmax - zmin : xmax - xmin;
  const rise = highY - lowY;
  const slopeLen = Math.sqrt(len * len + rise * rise);
  const slab = new THREE.Mesh(
    new THREE.BoxGeometry(axis === "x" ? slopeLen : width, 0.3, axis === "x" ? width : slopeLen),
    new THREE.MeshLambertMaterial({ color })
  );
  slab.position.set((xmin + xmax) / 2, (lowY + highY) / 2, (zmin + zmax) / 2);
  const ang = Math.atan2(rise, len);
  if (axis === "x") slab.rotation.z = -ang; else slab.rotation.x = ang;
  scene.add(slab);
}

// 斜坡在某 (x,z) 的地面高度。
function rampSurfaceY(ramp, x, z) {
  const coord = ramp.axis === "x" ? x : z;
  const span = ramp.axis === "x" ? ramp.xmax - ramp.xmin : ramp.zmax - ramp.zmin;
  let t = (coord - ramp.lowAt) / span;
  t = Math.max(0, Math.min(1, t));
  return ramp.lowY + t * (ramp.highY - ramp.lowY);
}

// ---- 鋪設遊樂場 ----
// 大地面（也是一個 box collider，頂面在 y=0）
addBox(0, -1, 0, 90, 2, 90, 0x33452f);
{
  // 地面格線：給尺度感
  const grid = new THREE.GridHelper(90, 45, 0x29381f, 0x29381f);
  grid.position.y = 0.02;
  scene.add(grid);
}

// 起點附近幾個可跳的箱子
addBox(-4, 0.6, 3, 2.4, 1.2, 2.4, 0x8a6d4a);
addBox(-7, 1.2, 1, 2.4, 2.4, 2.4, 0x8a6d4a);

// 一段階梯往 -z 上升（每階都是獨立 box，直接複用盒子碰撞）
for (let i = 0; i < 4; i++) {
  const top = 0.6 * (i + 1);
  addPlatform(0, top, -8 - i * 2, 6, 2, 0x5a6b7a, 0.6);
}
// 階梯頂端的大平台
addPlatform(0, 2.6, -18, 9, 6, 0x4a7a8a);
// 從大平台跳上去的浮島（更高，需要一跳）
addPlatform(0, 4.2, -26, 6, 6, 0x6a4a8a);
// 浮島上更高的小台（連跳挑戰）
addPlatform(4, 5.6, -28, 3, 3, 0x8a4a6a);

// +x 側：斜坡 → 高台
addRamp(4, 12, -2.5, 2.5, 0, 2.4, "x", 0x7a6a4a);
addPlatform(15, 2.4, 0, 5, 5, 0x4a8a5a);

// +z 側：間隔的浮台，需要連續橫跳
addPlatform(10, 1.2, 12, 3.5, 3.5, 0x5a8a7a);
addPlatform(14, 2.0, 16, 3.5, 3.5, 0x5a8a7a);
addPlatform(18, 2.8, 20, 3.5, 3.5, 0x5a8a7a);

// -x 側：往上疊的塔（跳跳樂）
addPlatform(-14, 1.4, -2, 3, 3, 0x9a7a4a);
addPlatform(-14, 2.8, -6, 3, 3, 0x9a7a4a);
addPlatform(-14, 4.2, -10, 3, 3, 0x9a7a4a);

// ============================================================
// 可收集的發光球
// ============================================================
const orbs = []; // { mesh, pos:Vector3, taken:false, base:number }
function addOrb(x, y, z) {
  const mesh = new THREE.Mesh(
    new THREE.IcosahedronGeometry(0.45, 0),
    new THREE.MeshStandardMaterial({
      color: 0xffe27a, emissive: 0xffc94a, emissiveIntensity: 1.4, roughness: 0.3,
    })
  );
  mesh.position.set(x, y, z);
  scene.add(mesh);
  // 球外圍一圈淡光暈（sprite），讓「發光」更明顯
  const halo = new THREE.Sprite(new THREE.SpriteMaterial({
    color: 0xffd24a, transparent: true, opacity: 0.4, depthWrite: false,
  }));
  halo.scale.set(1.8, 1.8, 1.8);
  mesh.add(halo);
  orbs.push({ mesh, pos: new THREE.Vector3(x, y, z), taken: false, base: y });
}
// 把球擺在各平台 / 跳躍路徑上，當小目標
addOrb(-4, 1.8, 3);
addOrb(0, 1.7, -10);
addOrb(0, 3.6, -18);
addOrb(0, 5.4, -26);
addOrb(4, 6.8, -28);
addOrb(15, 3.6, 0);
addOrb(10, 2.4, 12);
addOrb(14, 3.2, 16);
addOrb(18, 4.0, 20);
addOrb(-14, 5.4, -10);
addOrb(8, 1.4, 0);   // 半空中（地面起跳就拿得到）
const ORB_TOTAL = orbs.length;
let collected = 0;

// ============================================================
// 玩家：低多邊形膠囊 + 客戶端物理狀態
// ============================================================
// 手感調校（參考 Mario / Celeste 的平台跳躍：有重量的加減速、可變跳躍、coyote/buffer）
const PR = 0.45;        // 碰撞半徑（XZ）
const PH = 1.7;         // 身高
const GRAVITY = 30;     // 基礎重力加速度
const FALL_MULT = 1.5;  // 下墜時的重力加成：少一點飄、落地更俐落（Celeste 式快落）
const LOW_JUMP_MULT = 3.0; // 上升中放開跳鍵的額外重力 → 可變跳躍高度（短按矮跳、長按高跳）
const MAX_FALL = 38;    // 最大下墜速度，避免失速與穿模
const JUMP_V = 12.0;    // 跳躍初速
const RUN_SPEED = 7.5;  // 水平最高速
const ACCEL_GROUND = 70;   // 地面加速度（要一點時間才到頂速 → 起步有重量）
const ACCEL_AIR = 32;      // 空中加速度（保留微操但不如地面靈活）
const FRICTION_GROUND = 60;// 放開後的地面減速（會再滑一點點才停）
const FRICTION_AIR = 10;   // 空中阻力很小 → 保留動量
const COYOTE = 0.10;       // 土狼時間：離開邊緣後仍可跳的寬限
const JUMP_BUFFER = 0.12;  // 跳躍緩衝：落地前一點點按跳會被記住，落地即跳

// 朝目標值逼近（用於加速/減速，給移動重量感）
function approach(cur, target, maxDelta) {
  if (cur < target) return Math.min(cur + maxDelta, target);
  if (cur > target) return Math.max(cur - maxDelta, target);
  return cur;
}

const spawn = new THREE.Vector3(0, 1.5, 6);
const p = spawn.clone();              // 玩家「腳底」位置
const vel = new THREE.Vector3(0, 0, 0);
let onGround = false;
let coyoteT = 99;       // 自上次離地起算的時間（土狼時間用）
let jumpBufferT = 99;   // 自上次按跳起算的時間（跳躍緩衝用）
let jumpHeld = false;   // 跳鍵是否仍按住（可變跳躍高度用）
let facing = 0;

// 膠囊視覺（含一個小「臉」方向標，看得出朝向）
const player = new THREE.Group();
const capsule = new THREE.Mesh(
  new THREE.CapsuleGeometry(PR, PH - PR * 2, 6, 12),
  new THREE.MeshStandardMaterial({ color: 0xffd54a, roughness: 0.5 })
);
capsule.position.y = PH / 2;
player.add(capsule);
const nose = new THREE.Mesh(
  new THREE.BoxGeometry(0.25, 0.25, 0.3),
  new THREE.MeshStandardMaterial({ color: 0x33312a })
);
nose.position.set(0, PH * 0.7, PR);
player.add(nose);
scene.add(player);

// ============================================================
// 輸入：鍵盤 + 滑鼠 +（手機）虛擬搖桿 / 拖曳轉鏡頭 / 跳鈕
// ============================================================
const keys = Object.create(null);
const joy = { active: false, x: 0, y: 0 }; // 搖桿輸出 [-1,1]

window.addEventListener("keydown", (e) => {
  keys[e.code] = true;
  // 按跳：記進 buffer（落地即跳）並標記按住（可變跳躍高度）
  if (e.code === "Space") { jumpBufferT = 0; jumpHeld = true; e.preventDefault(); }
});
window.addEventListener("keyup", (e) => {
  keys[e.code] = false;
  if (e.code === "Space") jumpHeld = false; // 放開→上升中改用大重力，矮跳
});

// ---- 鏡頭角度（第三人稱）----
let camYaw = 0;   // 一開始鏡頭在角色後方（+z），角色面朝 -z 進入遊樂場
let camPitch = 0.5;
const PITCH_MIN = 0.12, PITCH_MAX = 1.25;

// 滑鼠拖曳轉鏡頭（桌機；不鎖指標，原型夠用）
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

// ---- 觸控：偵測到觸控裝置就顯示搖桿/跳鈕 ----
const isTouch = ("ontouchstart" in window) || (navigator.maxTouchPoints > 0);
if (isTouch) {
  const touchUI = document.getElementById("touch");
  if (touchUI) touchUI.style.display = "block";

  // 左下虛擬搖桿
  const joyEl = document.getElementById("joy");
  const nubEl = document.getElementById("joyNub");
  let joyId = null;
  const JOY_R = 35; // 旋鈕可移動半徑（px）
  function joyStart(t) {
    joyId = t.identifier; joy.active = true; joyMove(t);
  }
  function joyMove(t) {
    const r = joyEl.getBoundingClientRect();
    let dx = t.clientX - (r.left + r.width / 2);
    let dy = t.clientY - (r.top + r.height / 2);
    const len = Math.hypot(dx, dy) || 1;
    const clamped = Math.min(len, JOY_R);
    const nx = (dx / len), ny = (dy / len);
    nubEl.style.left = (35 + nx * clamped) + "px";
    nubEl.style.top = (35 + ny * clamped) + "px";
    // deadzone + 類比 magnitude：小幅晃動不誤觸，推多少走多快（跟手）
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

  // 右下跳躍鈕：按下記 buffer + 標記按住；放開→矮跳（可變跳躍高度同鍵盤）
  const jumpEl = document.getElementById("jump");
  jumpEl.addEventListener("touchstart", (e) => { e.preventDefault(); jumpBufferT = 0; jumpHeld = true; }, { passive: false });
  jumpEl.addEventListener("touchend", (e) => { e.preventDefault(); jumpHeld = false; }, { passive: false });
  jumpEl.addEventListener("touchcancel", () => { jumpHeld = false; });

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

// ============================================================
// 物理：每軸分離的 AABB 碰撞解算（站得上、會被牆擋、跳得上邊緣）
// ============================================================
const EPS = 1e-3;
function overlaps(box) {
  return (p.x + PR > box.min.x + EPS) && (p.x - PR < box.max.x - EPS) &&
         (p.y + PH > box.min.y + EPS) && (p.y      < box.max.y - EPS) &&
         (p.z + PR > box.min.z + EPS) && (p.z - PR < box.max.z - EPS);
}

function physics(dt) {
  // ---- 算水平輸入（相對鏡頭）----
  let inF = 0, inR = 0;
  if (keys["KeyW"] || keys["ArrowUp"]) inF += 1;
  if (keys["KeyS"] || keys["ArrowDown"]) inF -= 1;
  if (keys["KeyD"] || keys["ArrowRight"]) inR += 1;
  if (keys["KeyA"] || keys["ArrowLeft"]) inR -= 1;
  if (joy.active) { inF += -joy.y; inR += joy.x; } // 搖桿上=前進
  // 夾長度避免斜向超速
  const inLen = Math.hypot(inF, inR);
  if (inLen > 1) { inF /= inLen; inR /= inLen; }

  // 鏡頭朝向投影到地面：forward 指向角色前方（鏡頭看出去的方向）
  const fwdX = -Math.sin(camYaw), fwdZ = -Math.cos(camYaw);
  const rightX = -fwdZ, rightZ = fwdX;
  const dirX = fwdX * inF + rightX * inR;
  const dirZ = fwdZ * inF + rightZ * inR;

  // 目標速度：類比量（搖桿推一半就半速）當油門 → 跟手。
  // 不直接設速度，而是朝目標加速/減速 → 起步、煞停都有重量感（非瞬間到頂速）。
  const targetVX = dirX * RUN_SPEED;
  const targetVZ = dirZ * RUN_SPEED;
  const hasInput = inLen > 0.01;
  const rate = hasInput
    ? (onGround ? ACCEL_GROUND : ACCEL_AIR)
    : (onGround ? FRICTION_GROUND : FRICTION_AIR);
  vel.x = approach(vel.x, targetVX, rate * dt);
  vel.z = approach(vel.z, targetVZ, rate * dt);
  // 角色朝「實際移動方向」轉身（用速度而非輸入 → 轉向帶一點慣性，較自然）
  if (vel.x * vel.x + vel.z * vel.z > 0.6) facing = Math.atan2(vel.x, vel.z);

  // ---- 跳躍：jump buffer + 土狼時間 ----
  jumpBufferT += dt;
  if (jumpBufferT < JUMP_BUFFER && (onGround || coyoteT < COYOTE)) {
    vel.y = JUMP_V;
    onGround = false;
    coyoteT = COYOTE;          // 用掉土狼，避免空中再跳
    jumpBufferT = JUMP_BUFFER; // 用掉緩衝
  }

  // ---- X 軸移動 + 解算 ----
  p.x += vel.x * dt;
  for (const b of boxes) {
    if (!overlaps(b)) continue;
    if (vel.x > 0) p.x = b.min.x - PR;
    else if (vel.x < 0) p.x = b.max.x + PR;
    else { // 無水平速度卻重疊：往最近面推出
      const dl = (p.x + PR) - b.min.x, dr = b.max.x - (p.x - PR);
      p.x = dl < dr ? b.min.x - PR : b.max.x + PR;
    }
    vel.x = 0;
  }

  // ---- Z 軸移動 + 解算 ----
  p.z += vel.z * dt;
  for (const b of boxes) {
    if (!overlaps(b)) continue;
    if (vel.z > 0) p.z = b.min.z - PR;
    else if (vel.z < 0) p.z = b.max.z + PR;
    else {
      const dl = (p.z + PR) - b.min.z, dr = b.max.z - (p.z - PR);
      p.z = dl < dr ? b.min.z - PR : b.max.z + PR;
    }
    vel.z = 0;
  }

  // ---- Y 軸：可變重力 + 整合 + 落地/撞頭解算 ----
  // 上升中放開跳鍵→大重力（矮跳）；下墜→加成重力（快落不飄）。兩者合起來＝可變跳躍高度。
  let g = GRAVITY;
  if (vel.y > 0 && !jumpHeld) g *= LOW_JUMP_MULT;
  else if (vel.y < 0) g *= FALL_MULT;
  vel.y -= g * dt;
  if (vel.y < -MAX_FALL) vel.y = -MAX_FALL;
  p.y += vel.y * dt;
  onGround = false;
  for (const b of boxes) {
    if (!overlaps(b)) continue;
    if (vel.y > 0) { p.y = b.min.y - PH; vel.y = 0; }        // 撞到平台底面
    else { p.y = b.max.y; vel.y = 0; onGround = true; }       // 站上平台頂面
  }

  // ---- 斜坡：height-field，貼著斜面走/爬 ----
  for (const r of ramps) {
    if (p.x + PR <= r.xmin || p.x - PR >= r.xmax) continue;
    if (p.z + PR <= r.zmin || p.z - PR >= r.zmax) continue;
    const surf = rampSurfaceY(r, p.x, p.z);
    if (vel.y <= 0 && p.y < surf + 0.06) {
      p.y = surf; vel.y = 0; onGround = true;
    }
  }

  // ---- 土狼時間計時：在地面歸零，離地後累加（離地一小段時間內仍可跳）----
  if (onGround) coyoteT = 0;
  else coyoteT += dt;

  // ---- 掉出世界：拉回起點（不穿模、不掉光）----
  if (p.y < -25) {
    p.copy(spawn); vel.set(0, 0, 0); onGround = false;
  }
}

// ============================================================
// 收集判定
// ============================================================
function checkOrbs() {
  const cx = p.x, cy = p.y + PH * 0.5, cz = p.z;
  for (const o of orbs) {
    if (o.taken) continue;
    const dx = cx - o.pos.x, dy = cy - o.pos.y, dz = cz - o.pos.z;
    if (dx * dx + dy * dy + dz * dz < 1.4 * 1.4) {
      o.taken = true; o.mesh.visible = false; collected++;
      updateHud();
    }
  }
}

function updateHud() {
  if (!hudEl) return;
  const done = collected >= ORB_TOTAL;
  hudEl.innerHTML =
    `發光球 <b>${collected} / ${ORB_TOTAL}</b>` +
    (done ? " · 全收集完成！" : "") +
    `\n${isTouch ? "搖桿移動 · 右側拖曳轉鏡頭 · 跳鈕跳" : "WASD 移動 · 拖曳轉鏡頭 · 空白鍵跳"}`;
}
updateHud();

// ============================================================
// （加分）連既有 WS：把線上其他玩家當淡色膠囊畫在地面上。
// 單人就能玩——這段全包 try/catch，連不上/出錯都不影響本機遊玩。
// ============================================================
const ghosts = new Map(); // id -> mesh
let myWsId = null;
const WS_SCALE = 0.02; // 6000px 世界 → 約 120 單位，攤在遊樂場周圍
function ghostPos(x, y) {
  return [(x - 3000) * WS_SCALE, (y - 3000) * WS_SCALE];
}
(function connectWS() {
  try {
    const proto = location.protocol === "https:" ? "wss" : "ws";
    const ws = new WebSocket(`${proto}://${location.host}/ws`);
    ws.onopen = () => {
      try { ws.send(JSON.stringify({ type: "join", name: "3D玩家", species: "terran" })); } catch {}
    };
    ws.onmessage = (ev) => {
      let msg; try { msg = JSON.parse(ev.data); } catch { return; }
      try {
        if (msg.type === "welcome") myWsId = msg.id;
        else if (msg.type === "snapshot" && Array.isArray(msg.players)) {
          const seen = new Set();
          for (const pl of msg.players) {
            if (pl.id === myWsId) continue; // 自己用本機操控的膠囊
            if (typeof pl.x !== "number" || typeof pl.y !== "number") continue;
            seen.add(pl.id);
            let m = ghosts.get(pl.id);
            if (!m) {
              m = new THREE.Mesh(
                new THREE.CapsuleGeometry(PR, PH - PR * 2, 4, 8),
                new THREE.MeshStandardMaterial({ color: 0x6aa3ff, transparent: true, opacity: 0.5 })
              );
              scene.add(m); ghosts.set(pl.id, m);
            }
            const [gx, gz] = ghostPos(pl.x, pl.y);
            m.position.set(gx, PH / 2, gz);
          }
          for (const [id, m] of ghosts) {
            if (!seen.has(id)) { scene.remove(m); ghosts.delete(id); }
          }
        }
      } catch (e) { /* 韌性：單則訊息出錯不影響遊玩 */ }
    };
    ws.onerror = () => { try { ws.close(); } catch {} };
  } catch (e) { /* 連不上就純單人 */ }
})();

// ============================================================
// Render loop
// ============================================================
const clock = new THREE.Clock();
function frame() {
  requestAnimationFrame(frame);
  try {
    // dt 夾上限：分頁切回來/卡頓時別讓物理一步爆衝穿模
    const dt = Math.min(0.033, clock.getDelta());

    physics(dt);
    checkOrbs();

    // 玩家 mesh 跟物理位置；平滑轉向移動方向
    player.position.set(p.x, p.y, p.z);
    let d = facing - player.rotation.y;
    while (d > Math.PI) d -= Math.PI * 2;
    while (d < -Math.PI) d += Math.PI * 2;
    player.rotation.y += d * Math.min(1, dt * 12);

    // 第三人稱跟隨鏡頭：在角色後方、平滑跟隨（damping 用 1-exp → 跟幀率無關，順）
    const dist = 8, lookH = 1.4;
    const ox = Math.sin(camYaw) * Math.cos(camPitch) * dist;
    const oz = Math.cos(camYaw) * Math.cos(camPitch) * dist;
    const oy = Math.sin(camPitch) * dist + lookH;
    const tgt = new THREE.Vector3(p.x, p.y + lookH, p.z);
    const desired = new THREE.Vector3(p.x + ox, p.y + oy, p.z + oz);
    // 避免鏡頭穿到地面下：壓在地面與角色腳底之上
    if (desired.y < p.y + 1.2) desired.y = p.y + 1.2;
    if (desired.y < 0.6) desired.y = 0.6;
    camera.position.lerp(desired, 1 - Math.exp(-dt * 9));
    camera.lookAt(tgt);

    // 發光球：自轉 + 上下浮動 + 光暈呼吸，給回饋感
    const t = clock.elapsedTime;
    for (const o of orbs) {
      if (o.taken) continue;
      o.mesh.rotation.y += dt * 1.5;
      o.mesh.position.y = o.base + Math.sin(t * 2 + o.pos.x) * 0.15;
    }

    renderer.render(scene, camera);
  } catch (e) {
    // 護網：render 任一例外不該永久凍住畫面（rAF 已先排好下一幀）
    console.error("frame 例外，已跳過本幀", e);
    showErr(String(e && e.message ? e.message : e));
  }
}

window.addEventListener("resize", () => {
  camera.aspect = window.innerWidth / window.innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(window.innerWidth, window.innerHeight);
});

frame();
