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
// 天時盤 widget 句柄（ROADMAP 620）：可能不存在（舊頁／極簡測試 DOM）→ updateDayClock 全程 guard。
const dcDialEl = document.getElementById("dcDial");
const dcOrbitEl = document.getElementById("dcOrbit");
const dcSunEl = document.getElementById("dcSun");
const dcPhaseEl = document.getElementById("dcPhase");
const dcNextEl = document.getElementById("dcNext");
let dcLastPhase = null, dcLastNext = null; // 省 DOM 寫入：只在文字真的變動時才改寫
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

// 對帳「每幀平滑連續收斂」核心調校（單位＝世界 px，與 step_player 同座標）。
// 病史：
//  · 舊舊版用大死區（250px）放任預測自由往前衝 → 角色永遠飄在真實位置前方 ~250px（≈1.4 秒移動量）。
//  · 上一版（#799）改「每收到一筆權威快照，就把 predBase 往權威拉 0.5」→ 誤差是降下來了（✓），
//    但快照 15Hz＝基準每秒被離散猛拉 15 次、每次跳掉一半誤差 ＝ 渲染位置一格一格猛跳 ＝ 抖爆。
//    「每快照離散校準」就是抖動來源。
// 正解：把「每快照離散校準」換成「每一幀平滑連續收斂」（frame-rate 無關）：
//   每幀都讓預測基準 predBase（＝predWorld）平滑滑向「最近收到的自身權威座標」authority：
//     predBase += (1 - exp(-dt*K)) * (authority - predBase)
//   authority 每快照更新、但收斂是「每幀連續」做的，不再一格一格跳 → 根治抖動。
//   每幀仍照舊用 wasm step_player 從 predBase 外插「當前這一幀輸入」得到要畫的位置 → 即時跟手。
//   移動中平衡誤差 ≈ speed/K（180/9 ≈ 20px，落在目標 ~20-40 且視覺上看不到）；停下用較大 K 快對齊。
// 超大跳躍（傳送／回城／復活）仍直接 snap 到權威。
const RECONCILE_JUMP_PX = 800;        // 超大誤差（傳送／回城／復活）→ 直接 snap 到權威（世界 px）
const CONVERGE_K = 9;                 // 移動中：predBase 每幀平滑收斂到「對齊目標」的速率（每秒；≈1/K 秒時間常數，平衡誤差≈speed/K）
const CONVERGE_STOP_K = 18;           // 停下（無方向輸入）時用較大速率快對齊（每秒，靜止悄悄歸零誤差）

// 權威外插（QA #803 實測挑出的「對齊目標平滑化」做法，殺 #802 殘留卡頓）。
// 病根（QA 量到）：prod 快照「叢發抵達」（間隔 1.4–283ms 抖動）→ 收斂的「目標」＝原始權威
//   每收到一筆就往前跳一段＝階梯狀 → 收斂目標本身不平滑 → 殘留單幀 lurch（殘卡）。
// 解：移動中不再朝「原始權威」收斂，而是用權威的估計速度把對齊目標往前外插成「平滑連續移動的點」：
//   target = 最近權威 + 權威估計速度 × (此幀距該權威到達經過的時間)
//   predBase 再每幀朝這個平滑 target 收斂（K 不變）→ 目標本身平滑 → 殘留 lurch 殺掉。
// QA 多候選離線重放（同一份 prod 權威序列）實測：外插把現行 A 的最大單幀 lurch 砍掉 ~55-79%，
//   誤差(領先量) 仍落在健康的 ~7-12px（目標 10-30、不延遲）。K 沿用 9（保留既調好的跟手/領先量；
//   K6 雖再順一點但領先量變大、且 K6↔K12 的 lurch 差落在 run 間雜訊內）。詳見 scripts/qa/move-qa.mjs。
// 靜止時不外插（權威速度已歸零、且避免衝過停點），改朝原始權威收斂（CONVERGE_STOP_K）。
const AUTH_VEL_WINDOW_MS = 180;       // 估計權威速度的回看時間窗（ms）：夠平滑掉快照叢發雜訊、又夠跟手
const AUTH_EXTRAP_MAX_MS = 250;       // 外插上限（ms）：快照久久不來時，對齊目標別無限往前飛

// 偵錯讀數（FPS／自身預測誤差／線上人數）：預設關，?debug=1 或點畫面左上角切換。
let DEBUG_HUD = false;
try { DEBUG_HUD = new URLSearchParams(location.search).get("debug") === "1"; } catch (e) { /* 無 location（測試 DOM）→ 維持關 */ }
let selfPredErrPx = 0;  // 自身預測位置與最近一筆權威位置的距離（世界 px），每幀更新供 debug 讀數

// 3) AOI 進出淡入淡出 — 實體第一次出現淡入、離開快照淡出再移除，不啪一下彈出/消失。
const FADE_RATE = 6;           // 淡入淡出速率（每秒，1-exp 收斂）

// ============================================================
// world-core wasm（自身移動預測的「碰撞感知」來源，與伺服器同源）
// ============================================================
// /3d/ 原本的自身預測（#764）只用固定速度往前推、完全不懂地形碰撞：玩家一靠近
// 牆／水／障礙，預測就「走進去」→ 伺服器（2D 權威、有碰撞）把你擋住 → 對帳又把你
// 拉回 → 反覆＝卡卡＋空氣牆。2D web/game.js 沒這問題，因為它的預測走 world-core
// wasm 的 step_player（與伺服器 Player::step 同一份 Rust 物理、含地形碰撞）。
// 這裡照樣把 /3d/ 的自身預測接上同一支 .wasm：該停的地方就停、預測與權威幾乎一致，
// 不再穿牆被拉回。載入失敗（檔案不在／舊瀏覽器）時自動退回 #764 的簡單推（後備，不白屏）。
// .wasm 不入 repo——由 scripts/build-wasm.sh 建到 web/wasm/（deploy.sh 會自動跑）。
let wasmTerrain = null;          // 載入成功 = wasm instance.exports（具 step_player 才算可預測）
// wasm 是否可做碰撞感知預測（出口齊全才算）。
function wasmPredictReady() {
  return !!(wasmTerrain && typeof wasmTerrain.step_player === "function" &&
    typeof wasmTerrain.step_out_x === "function" && typeof wasmTerrain.step_out_y === "function");
}
// 跑步倍率：優先讀 wasm（與 world_core::RUN_MULT 同源，避免前後端漂移）；舊 .wasm 退回 1.6。
function wasmRunMult() {
  return (wasmTerrain && typeof wasmTerrain.run_mult === "function") ? wasmTerrain.run_mult() : 1.6;
}
(async function loadWorldCoreWasm() {
  try {
    // main.js 是 ES module，import.meta.url 指向 /3d/main.js；../wasm/ ＝站台根的 /wasm/，
    // 與 game.js 的 fetch("wasm/world_core.wasm") 指向同一支 .wasm（同一份實作）。
    const url = new URL("../wasm/world_core.wasm", import.meta.url);
    const resp = await fetch(url);
    if (!resp.ok) throw new Error("HTTP " + resp.status);
    const { instance } = await WebAssembly.instantiate(await resp.arrayBuffer(), {});
    const ex = instance.exports;
    if (typeof ex.step_player !== "function" ||
        typeof ex.step_out_x !== "function" || typeof ex.step_out_y !== "function") {
      throw new Error("缺 step_player/step_out_x/step_out_y 出口");
    }
    wasmTerrain = ex;
    console.log("[world-core] 3D 自身預測已接 wasm（碰撞感知，與伺服器同源）");
  } catch (err) {
    console.warn("[world-core] wasm 載入失敗，3D 自身預測退回簡單推（#764 後備）:",
      err && err.message ? err.message : err);
  }
})();

// ============================================================
// 手機 / 弱 GPU 渲染削減（perf/3d-mobile）
// ============================================================
// 病徵：桌機 Intel HD530 量到 60fps，但玩家 iPhone 實測走路只有 ~30fps —— 瓶頸是手機 GPU。
// 場景已是 draw-call bound（#816 砍過田畦洩漏 884→245），但弱 GPU 的「每像素填充率」也是
// 大殺手：DPR=3 的手機，每幀要算的像素數是 DPR=1 桌機的 9 倍。本區塊偵測手機／弱機，套用
// 一組積極但「仍可玩、不太醜」的削減（桌機維持原樣或也受益）。
//
// 削減清單（挑對 FPS 影響大的）：
//   1) 降 pixelRatio 上限（手機最大單一殺手）——手機 cap 1.5、桌機維持 2。
//   2) 關抗鋸齒（antialias 對弱 GPU 偏貴；邊緣略糊可接受）。
//   3) powerPreference:'high-performance'（請系統挑獨顯／高效能 GPU）。
//   4) 明確關陰影（本場景本就沒開，明寫 false 防未來誤開、零成本）。
//   5) 拉近霧 + 對遠端實體做距離剔除（遠到沒入霧就整個 visible=false，省 draw call＋省動畫 CPU）。
//
// 偵測：?lowfx=1 強制開／?lowfx=0 強制關（給 QA／桌機對照）；否則自動判斷——
//   userAgent 含行動裝置關鍵字、或（觸控 且 高 DPR）、或（觸控 且 少核）視為弱機。
function detectLowFx() {
  try {
    const q = new URLSearchParams(location.search).get("lowfx");
    if (q === "1") return true;
    if (q === "0") return false;
  } catch (e) { /* 無 location（測試 DOM）→ 往下走自動偵測 */ }
  try {
    const ua = (typeof navigator !== "undefined" && navigator.userAgent) || "";
    const mobileUA = /Android|iPhone|iPad|iPod|Mobile|Silk|Kindle|Opera Mini|IEMobile/i.test(ua);
    const touch = typeof navigator !== "undefined" &&
      ((navigator.maxTouchPoints || 0) > 0 || (typeof window !== "undefined" && "ontouchstart" in window));
    const hiDpr = (typeof window !== "undefined" ? (window.devicePixelRatio || 1) : 1) >= 2;
    const fewCores = typeof navigator !== "undefined" &&
      Number.isFinite(navigator.hardwareConcurrency) && navigator.hardwareConcurrency <= 4;
    // 行動裝置一律算弱機；觸控筆電要「高 DPR」或「少核」才算（避免誤殺高階觸控桌機）。
    return mobileUA || (touch && hiDpr) || (touch && fewCores);
  } catch (e) { return false; }
}
const LOWFX = detectLowFx();
const PIXEL_RATIO_CAP = LOWFX ? 1.5 : 2;     // 手機降到 1.5（像素數 ~4倍→~2.25倍），桌機維持 2
// 距離常數量級參考：世界 6000px×WORLD_SCALE(0.05)=300 場景單位寬、第三人稱 camDist=48。
// 故「遠」落在 ~120-250 這個量級（既有 STICK_FAR_DIST=90、CROP_LOD~70 也是這量級）。
const FOG_NEAR = LOWFX ? 120 : 250;          // 霧起點（場景單位）：手機把視野拉近一截
const FOG_FAR = LOWFX ? 230 : 600;           // 霧終點＝看得見距離；手機拉近，遠物提早沒入霧
const ENTITY_CULL_DIST = LOWFX ? 250 : Infinity; // 遠端實體距鏡頭超過此距離（已沒入霧）就不畫；桌機不剔
const ENTITY_CULL_DIST2 = ENTITY_CULL_DIST * ENTITY_CULL_DIST;
try { console.log(`[3d] lowfx=${LOWFX} dpr=${typeof window !== "undefined" ? window.devicePixelRatio : "?"} cap=${PIXEL_RATIO_CAP} aa=${!LOWFX}`); } catch (e) { /* 無 console 無妨 */ }

// ---- Three.js 基礎場景 ----
const app = document.getElementById("app");
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x0d1117);
scene.fog = new THREE.Fog(0x0d1117, FOG_NEAR, FOG_FAR);

const camera = new THREE.PerspectiveCamera(58, window.innerWidth / window.innerHeight, 0.5, 2000);
camera.position.set(0, 60, 80);

const renderer = new THREE.WebGLRenderer({
  antialias: !LOWFX,                    // 手機關抗鋸齒（弱 GPU 偏貴；邊緣略糊換 FPS）
  powerPreference: "high-performance",  // 請系統挑高效能／獨顯 GPU
});
renderer.setSize(window.innerWidth, window.innerHeight);
// 手機友善：pixelRatio 上限——手機 1.5、桌機 2（高 DPI 手機算爆 GPU 的最大單一殺手）
renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, PIXEL_RATIO_CAP));
// 明確關陰影：本場景燈本就不投影、mesh 也沒 castShadow，明寫 false 防未來誤開（零成本）。
// guard：測試替身的 fake renderer 沒有 shadowMap；真 Three.js 一定有，故 if 不影響正式行為。
if (renderer.shadowMap) renderer.shadowMap.enabled = false;
app.appendChild(renderer.domElement);

// 燈光：半球光給柔和環境色 + 一盞方向光（太陽／月亮）給立體感。
// 兩盞的強度／顏色／太陽位置都會隨「日夜循環」每幀流轉（見下方 day/night 區塊）。
const hemi = new THREE.HemisphereLight(0xbfd4ff, 0x20303a, 1.1);
scene.add(hemi);
const sun = new THREE.DirectionalLight(0xffffff, 1.2);
sun.position.set(120, 200, 80);
scene.add(sun);

// ============================================================
// 日夜循環在 3D 裡流轉（ROADMAP 612）：快照早就帶著權威的 daynight
// （phase／light／day_fraction／night_danger，見 protocol.rs DayNightView，2D 一直在用），
// 3D 卻一直忽略、世界永遠是同一個灰藍午後。本區塊把它接上——天空色、霧色、太陽弧線與
// 顏色、環境光強度全隨 live 日夜緩緩流轉：破曉天邊泛紅、正午湛藍高陽、黃昏橙金、入夜墨藍月光。
// 純讀快照、零後端／協議改動（資料本來就在快照裡）。
// ============================================================
let latestDayNight = null; // 最新一筆快照的 daynight（沒有就用預設白天）
let dayNightAnchorMs = (typeof performance !== "undefined" ? performance.now() : 0); // 收到最新 daynight 的本地時刻：天時盤倒數/繞盤錨點（ROADMAP 620）
const DN_RATE = 1.6;       // 視覺平滑趨近速率（每秒，1-exp；緩緩流轉、不跳變、不被 15Hz 快照打頓）

function dnClamp01(x) { return x < 0 ? 0 : x > 1 ? 1 : x; }
function dnLerp(a, b, t) { return a + (b - a) * t; }
function dnSmoothstep(e0, e1, x) { const t = dnClamp01((x - e0) / (e1 - e0 || 1e-6)); return t * t * (3 - 2 * t); }

// 沿「日循環比例 f∈[0,1)」環狀內插一張 {f,r,g,b} 關鍵幀表（會跨 1.0→0.0 邊界 wrap）。
// f 壞值（非有限）退回 0；永不 throw。
function dnKeyLerp(table, f) {
  f = Number.isFinite(f) ? ((f % 1) + 1) % 1 : 0;
  const n = table.length;
  for (let i = 0; i < n; i++) {
    const a = table[i], b = table[(i + 1) % n];
    const af = a.f;
    let bf = b.f; if (bf <= af) bf += 1;       // 末段跨 1.0 回到第一幀
    let ff = f; if (ff < af) ff += 1;          // 讓 f 落進 [af, bf)
    if (ff >= af && ff < bf) {
      const t = (ff - af) / (bf - af);
      return [dnLerp(a.r, b.r, t), dnLerp(a.g, b.g, t), dnLerp(a.b, b.b, t)];
    }
  }
  return [table[0].r, table[0].g, table[0].b];
}

// 天空／霧色關鍵幀（與 daynight.rs 的階段邊界對齊：破曉 0–0.15、白天 0.15–0.5、黃昏 0.5–0.65、夜晚 0.65–1）。
const DN_SKY_KEYS = [
  { f: 0.00, r: 0.10, g: 0.11, b: 0.21 }, // 將明未明的深藍
  { f: 0.07, r: 0.78, g: 0.50, b: 0.45 }, // 破曉天邊泛紅
  { f: 0.15, r: 0.50, g: 0.68, b: 0.92 }, // 晨光轉藍
  { f: 0.33, r: 0.38, g: 0.64, b: 0.96 }, // 正午湛藍（最亮）
  { f: 0.50, r: 0.55, g: 0.62, b: 0.82 }, // 午後偏白
  { f: 0.57, r: 0.90, g: 0.48, b: 0.30 }, // 黃昏橙金
  { f: 0.65, r: 0.20, g: 0.16, b: 0.30 }, // 暮色轉紫
  { f: 0.80, r: 0.05, g: 0.06, b: 0.13 }, // 入夜墨藍
];
// 太陽／月亮的光色關鍵幀：晨昏暖橙、正午純白、入夜轉冷月光。
const DN_SUN_KEYS = [
  { f: 0.00, r: 0.55, g: 0.62, b: 0.95 }, // 殘月冷光
  { f: 0.07, r: 1.00, g: 0.62, b: 0.42 }, // 破曉暖橙
  { f: 0.20, r: 1.00, g: 0.96, b: 0.88 }, // 晨光近白
  { f: 0.40, r: 1.00, g: 0.98, b: 0.92 }, // 正午純白
  { f: 0.57, r: 1.00, g: 0.55, b: 0.32 }, // 黃昏暖橙
  { f: 0.66, r: 0.60, g: 0.62, b: 0.96 }, // 入夜月光
  { f: 0.85, r: 0.52, g: 0.60, b: 1.00 }, // 深夜冷月
];

// 日夜階段 → 面向玩家的標籤（後端只送穩定的 snake_case 階段碼，文案／emoji 由前端對照＝留 i18n 空間）。
const DN_PHASE_LABEL = { dawn: "🌅 破曉", day: "☀️ 白天", dusk: "🌆 黃昏", night: "🌙 夜晚" };
// 純函式：取一筆 daynight 的階段標籤；缺欄位／未知碼 → 空字串（HUD 就不顯示，向後相容）。
function dayNightPhaseLabel(dn) {
  if (!dn || typeof dn.phase !== "string") return "";
  return DN_PHASE_LABEL[dn.phase] || "";
}

// ── 天時盤 HUD（ROADMAP 620）：3D 世界至此只在 HUD 文字行埋一個小小時段 emoji；
// AI 居民反覆許願「醒目的時間狀態指示器／時間流速節奏指標」（data/suggestions.jsonl 居民-5/11/12/4）。
// 快照早就帶著權威天時盤欄位（day_fraction／secs_to_next／next_phase／night_danger，見 419 天時盤），
// 這裡據此在右上角畫一個醒目的日晷：太陽/月亮繞盤、時段大字、下一時段倒數、夜間危機暈輪。
// 純讀快照、零後端／協議改動——資料本來就在 DayNightView 裡。
const DAY_LENGTH_SECS = 600; // 一整輪日夜＝600 秒（與 2D game.js 天時盤錨點同步，見 419/497）
// 純函式：把一筆權威 daynight + 自收到該快照起算的 elapsedSecs，算成天時盤顯示值。
// 平滑推進 day_fraction 與倒數（不必等下一份快照），確定性、壞值安全，供 render-smoke 斷言。
function dayClockReadout(dn, elapsedSecs) {
  const e = Number.isFinite(elapsedSecs) ? Math.max(0, elapsedSecs) : 0;
  const baseFrac = (dn && Number.isFinite(dn.day_fraction)) ? dn.day_fraction : 0;
  // 平滑推進並夾進 [0,1)（負值也安全）
  const frac = ((((baseFrac + e / DAY_LENGTH_SECS) % 1) + 1) % 1);
  // 太陽/月亮繞盤角度（deg，0=盤頂、順時針為正）：正午(0.5)在頂、午夜(0)在底、
  // 破曉(0.25)在右(東昇)、黃昏(0.75)在左(西落)——直覺的日行軌跡。
  const sunDeg = (((180 - frac * 360) % 360) + 360) % 360;
  // 倒數：權威 secs_to_next 減去已過秒數、夾非負；缺欄位（舊伺服器）→ null＝不顯示倒數。
  const rawSecs = (dn && Number.isFinite(dn.secs_to_next)) ? dn.secs_to_next : null;
  const secsLeft = (rawSecs == null) ? null : Math.max(0, Math.round(rawSecs - e));
  const phaseLabel = dayNightPhaseLabel(dn);
  const nextLabel = (dn && typeof dn.next_phase === "string") ? (DN_PHASE_LABEL[dn.next_phase] || "") : "";
  const isNight = !!(dn && dn.phase === "night");
  const danger = !!(dn && dn.night_danger);
  const gameHour = Math.floor(frac * 24); // 0..23 遊戲整點
  return { frac, sunDeg, secsLeft, phaseLabel, nextLabel, isNight, danger, gameHour };
}
// 純函式：把秒數格式成 m:ss 倒數字串；缺值／壞值 → 空字串（不顯示倒數）。
function fmtCountdown(secs) {
  if (secs == null || !Number.isFinite(secs) || secs < 0) return "";
  const m = Math.floor(secs / 60), s = Math.floor(secs % 60);
  return m + ":" + String(s).padStart(2, "0");
}

// 純函式：把一筆 daynight 算成這一刻的視覺參數（天空／霧色 RGB、太陽光色／強度／位置、環境光強度）。
// 確定性、壞值安全（缺欄位／非有限 → 當成晴朗白天），供每幀平滑趨近，亦供 render-smoke 斷言。
function dayNightVisual(dn) {
  const f = dn && Number.isFinite(dn.day_fraction) ? dn.day_fraction : 0.33;
  const light = dnClamp01(dn && Number.isFinite(dn.light) ? dn.light : 1.0); // 後端保證 [0.2,1]，仍夾界
  const danger = !!(dn && dn.night_danger);

  let sky = dnKeyLerp(DN_SKY_KEYS, f);
  const sun = dnKeyLerp(DN_SUN_KEYS, f);
  // 夜間危機：天色／霧色微微壓向不祥的暗紅（純氛圍、輕微、不刺眼）
  if (danger) sky = [dnLerp(sky[0], 0.22, 0.14), dnLerp(sky[1], 0.05, 0.14), dnLerp(sky[2], 0.07, 0.14)];

  const lit = dnSmoothstep(0.2, 1.0, light);
  const sunIntensity = 0.08 + lit * 1.35;   // 夜裡僅微光、正午最盛
  const hemiIntensity = 0.35 + light * 0.9; // 環境光隨亮度起落
  // 太陽位置：方位角隨日循環掃過天際、仰角隨亮度（正午最高、夜裡貼近地平）
  const ang = (Number.isFinite(f) ? f : 0.33) * Math.PI * 2;
  const sunPos = { x: Math.cos(ang) * 160, y: 35 + light * 190, z: Math.sin(ang) * 160 };
  return { sky, fog: sky.slice(), sun, sunIntensity, hemiIntensity, sunPos };
}

// 背景／霧／太陽光色用持久 Color 實例，每幀 setRGB 寫入（不每幀 new、省 GC）。
const dnSkyColor = new THREE.Color();
const dnSunColor = new THREE.Color();
scene.background = dnSkyColor;
sun.color = dnSunColor;

// 平滑趨近的目前視覺狀態（初始＝預設白天，第一幀不從黑畫面淡入）。
const _dnInit = dayNightVisual(null);
const dnSky = _dnInit.sky.slice();
const dnFog = _dnInit.fog.slice();
const dnSun = _dnInit.sun.slice();
let dnSunI = _dnInit.sunIntensity;
let dnHemiI = _dnInit.hemiIntensity;
const dnSunPos = { ..._dnInit.sunPos };

// 每幀：算出目標視覺、平滑趨近、寫進場景（天空／霧／太陽／環境光）。
function applyDayNight(dt) {
  const target = dayNightVisual(latestDayNight);
  const a = 1 - Math.exp(-Math.max(0, dt) * DN_RATE); // 1-exp → 與幀率無關
  for (let i = 0; i < 3; i++) {
    dnSky[i] = dnLerp(dnSky[i], target.sky[i], a);
    dnFog[i] = dnLerp(dnFog[i], target.fog[i], a);
    dnSun[i] = dnLerp(dnSun[i], target.sun[i], a);
  }
  dnSunI = dnLerp(dnSunI, target.sunIntensity, a);
  dnHemiI = dnLerp(dnHemiI, target.hemiIntensity, a);
  dnSunPos.x = dnLerp(dnSunPos.x, target.sunPos.x, a);
  dnSunPos.y = dnLerp(dnSunPos.y, target.sunPos.y, a);
  dnSunPos.z = dnLerp(dnSunPos.z, target.sunPos.z, a);
  dnSkyColor.setRGB(dnSky[0], dnSky[1], dnSky[2]);
  if (scene.fog && scene.fog.color) scene.fog.color.setRGB(dnFog[0], dnFog[1], dnFog[2]);
  dnSunColor.setRGB(dnSun[0], dnSun[1], dnSun[2]);
  sun.intensity = dnSunI;
  sun.position.set(dnSunPos.x, dnSunPos.y, dnSunPos.z);
  hemi.intensity = dnHemiI;
}

// ============================================================
// 天上的日月星辰在 3D 裡現身（ROADMAP 628）：日夜系統（612）早把天色、霧色、隱形太陽光
// 都接上了，HUD 角落也有日晷（620）——但抬頭看天，天上空空如也：沒有一輪會東昇西落的太陽、
// 沒有與之相對的月亮、入夜也不見半顆星。AI 居民反覆許願「醒目的時間狀態指示器／時間流速節奏
// 指標」（data/suggestions.jsonl 居民-5/11/12/4）——620 在 HUD 上回應了，這一塊把它做進「世界本身」：
// 一輪太陽沿天弧東昇、登頂、西落，一輪月亮恆與它相對地起落，夜深時滿天星斗淡入、隨日輪緩緩斗轉。
// 從此抬頭一眼便知此刻何時——時間第一次不只在儀表板上，而在天上。
// 純讀既有 daynight 快照（day_fraction／light）、零後端／協議／成本；沿用日夜同一套手法：
// 純函式算擺位明暗、每幀 1-exp 平滑趨近、壞值安全降級不拋。
// ============================================================
// 純函式：把一筆 daynight 算成天上日月星辰的方位、仰角與明暗。
// 日月在同一條天弧的兩端：太陽正午(0.5)登頂、午夜沉到地平下；月亮恆與太陽相對地起落，
// 破曉/黃昏兩者都貼近地平、雙雙轉淡（自然的晝夜接力交班）。星辰隨夜色轉濃、白天全隱。
// 確定性、壞值安全（缺欄位／非有限 → 當成晴朗白天），供每幀平滑趨近，亦供 render-smoke 斷言。
function celestialSky(dn) {
  const f = dn && Number.isFinite(dn.day_fraction) ? (((dn.day_fraction % 1) + 1) % 1) : 0.33;
  const light = dnClamp01(dn && Number.isFinite(dn.light) ? dn.light : 1.0); // 後端保證 [0.2,1]，仍夾界
  // 天弧角：破曉(0.25)在東地平、正午(0.5)登頂、黃昏(0.75)在西地平、午夜(0)沉到地平下。
  const theta = (f - 0.25) * Math.PI * 2;
  const sunElev = Math.sin(theta);  // 太陽仰角分量 ∈[-1,1]（>0 在地平上）
  const sunEW = Math.cos(theta);    // 東(+)西(-)分量
  // 仰角→明暗：升出地平才現身、貼地平轉淡（smoothstep 讓日月在晨昏交班時柔和淡入淡出）
  const sunOpacity = dnSmoothstep(-0.06, 0.18, sunElev);
  const moonOpacity = dnSmoothstep(-0.06, 0.18, -sunElev) * 0.9; // 月色比日略淡
  // 星辰：天色越暗越濃（white 白天全隱、深夜最盛）
  const starOpacity = dnClamp01(1 - dnSmoothstep(0.30, 0.66, light));
  return {
    sun:  { ew: sunEW,  elev: sunElev,  opacity: sunOpacity },
    moon: { ew: -sunEW, elev: -sunElev, opacity: moonOpacity }, // 恆與太陽相對
    starOpacity,
    spin: f * Math.PI * 2, // 夜空隨日輪緩緩斗轉星移
  };
}

// 天球：圍著鏡頭、半徑固定（material.fog=false 不被霧吃掉），故日月星彷彿在無限遠的天上。
// 整個天弧向前傾，讓日月落在前上方的可見天空、而非橫穿鏡頭兩側。
const SKY_R = 560;
const CELE_TILT = -0.55;
const celestialGroup = new THREE.Group();
celestialGroup.rotation.x = CELE_TILT;
scene.add(celestialGroup);

// 太陽：亮暖核 + 一圈柔光暈（MeshBasicMaterial 自發光、不吃場景燈，恰合自體發光的天體）。
const sunCoreMat = new THREE.MeshBasicMaterial({ color: 0xffe6a6, transparent: true, opacity: 0, fog: false, depthWrite: false });
const sunMesh = new THREE.Mesh(new THREE.SphereGeometry(26, 16, 12), sunCoreMat);
const sunHaloMat = new THREE.MeshBasicMaterial({ color: 0xffd27a, transparent: true, opacity: 0, fog: false, depthWrite: false });
const sunHalo = new THREE.Mesh(new THREE.SphereGeometry(46, 16, 12), sunHaloMat);
celestialGroup.add(sunHalo);
celestialGroup.add(sunMesh);
// 月亮：清冷淡白。
const moonMat = new THREE.MeshBasicMaterial({ color: 0xdde6ff, transparent: true, opacity: 0, fog: false, depthWrite: false });
const moonMesh = new THREE.Mesh(new THREE.SphereGeometry(19, 16, 12), moonMat);
celestialGroup.add(moonMesh);

// 星辰：散在上半天球的點陣，入夜淡入、隨日輪緩緩斗轉（手機減量、省 GPU）。
const _celeTouch = ("ontouchstart" in window) || (navigator.maxTouchPoints > 0);
const STAR_CAP = _celeTouch ? 140 : 320;
const starGroup = new THREE.Group();
celestialGroup.add(starGroup);
const starPos = new Float32Array(STAR_CAP * 3);
for (let i = 0; i < STAR_CAP; i++) {
  // 均勻散在半徑略小於天球的上半殼（仰角恆非負，免得埋到地平下）
  const phi = Math.acos(Math.max(0.05, Math.random()));   // 0=天頂、越大越貼地平
  const ang = Math.random() * Math.PI * 2;
  const r = SKY_R * 0.96;
  const sinp = Math.sin(phi);
  starPos[i * 3]     = Math.cos(ang) * sinp * r;
  starPos[i * 3 + 1] = Math.cos(phi) * r; // ≥0：上半天球
  starPos[i * 3 + 2] = Math.sin(ang) * sinp * r;
}
const starGeo = new THREE.BufferGeometry();
starGeo.setAttribute("position", new THREE.BufferAttribute(starPos, 3));
const starMat = new THREE.PointsMaterial({ color: 0xeaf0ff, size: 3.2, transparent: true, opacity: 0, fog: false, depthWrite: false, sizeAttenuation: true });
const starField = new THREE.Points(starGeo, starMat);
starField.frustumCulled = false;
starGroup.add(starField);

// 平滑趨近的目前狀態（初始＝預設白天：日高懸、月星俱隱）。
const _celeInit = celestialSky(null);
let celeSunO = _celeInit.sun.opacity, celeMoonO = _celeInit.moon.opacity, celeStarO = _celeInit.starOpacity;
const celeSunP = { ew: _celeInit.sun.ew, elev: _celeInit.sun.elev };
const celeMoonP = { ew: _celeInit.moon.ew, elev: _celeInit.moon.elev };

// 每幀：算出目標擺位明暗、平滑趨近、寫進場景；天球圍著鏡頭平移（不隨鏡頭旋轉→方位固定）。
function applyCelestial(dt) {
  const target = celestialSky(latestDayNight);
  const a = 1 - Math.exp(-Math.max(0, dt) * DN_RATE);
  celeSunP.ew = dnLerp(celeSunP.ew, target.sun.ew, a);
  celeSunP.elev = dnLerp(celeSunP.elev, target.sun.elev, a);
  celeMoonP.ew = dnLerp(celeMoonP.ew, target.moon.ew, a);
  celeMoonP.elev = dnLerp(celeMoonP.elev, target.moon.elev, a);
  celeSunO = dnLerp(celeSunO, target.sun.opacity, a);
  celeMoonO = dnLerp(celeMoonO, target.moon.opacity, a);
  celeStarO = dnLerp(celeStarO, target.starOpacity, a);
  // 圍著鏡頭：日月星永遠在「無限遠」的天上（隨鏡頭平移、不隨鏡頭旋轉→世界方位固定，可繞看）
  celestialGroup.position.set(camera.position.x, camera.position.y, camera.position.z);
  sunMesh.position.set(celeSunP.ew * SKY_R, celeSunP.elev * SKY_R, 0);
  sunHalo.position.copy(sunMesh.position);
  moonMesh.position.set(celeMoonP.ew * SKY_R, celeMoonP.elev * SKY_R, 0);
  sunCoreMat.opacity = celeSunO;
  sunHaloMat.opacity = celeSunO * 0.35;
  moonMat.opacity = celeMoonO;
  starMat.opacity = celeStarO;
  // 全隱時關掉可見性（省繪製；隱形物件無謂送 GPU）
  sunMesh.visible = sunHalo.visible = celeSunO > 0.01;
  moonMesh.visible = celeMoonO > 0.01;
  starField.visible = celeStarO > 0.01;
  starGroup.rotation.y = target.spin;
}

// ============================================================
// 天氣在 3D 裡落下來（ROADMAP 613）：快照早就帶著伺服器權威的 weather
// （weather_type／intensity／wind，見 protocol.rs WeatherView，2D game.js 一直在用它畫雨絲沙塵），
// 還有 rainbow（雨後彩虹），3D 卻一直忽略——不管伺服器走到細雨、風沙還是海霧，3D 裡永遠晴空無物。
// 本區塊把它接上：一片圍著鏡頭的低多邊形粒子場（雨／沙／晶塵／海霧各有顏色、落速、隨風漂向），
// 配上霧色微染與遠空的彩虹弧——天色（612）之後，世界第一次有了「天氣」。純讀快照、零後端／協議改動。
// 沿用日夜系統同一套手法：純函式算視覺、每幀 1-exp 平滑趨近、壞值安全降級不拋。
// ============================================================
let latestWeather = null; // 最新一筆快照的 weather（沒有就當晴天）
let latestRainbow = null; // 最新一筆快照的 rainbow（沒有就無彩虹）
const WX_RATE = 1.3;      // 天氣視覺平滑趨近速率（每秒，1-exp；淡入淡出、不跟著 15Hz 快照跳變）

// 各天氣的視覺規格（顏色鏡像 2D game.js：雨#88ccff／沙#c9a24b／晶#aaddff／霧淡白）。
//   color  ── 粒子顏色 [r,g,b]∈[0,1]
//   size   ── 粒子大小（場景單位，sizeAttenuation 會隨遠近縮放）
//   fall   ── 垂直落速（場景單位/秒；負值＝上飄，海霧用）
//   drift  ── 對世界風的易感度（鏡像 2D WEATHER_WIND_SUSCEPT 的相對關係）
//   fog    ── 此天氣把霧／天空往哪個色調微染 [r,g,b]
//   fogMix ── 染色強度上限（再乘當下 intensity）
//   fogFar ── 此天氣把霧的「看得見的距離」拉近到多遠（場景單位；起霧／風沙視野變短）
//   sparkle── 是否忽明忽暗閃爍（晶塵專用）
const WEATHER_SPEC = {
  grassland_rain:     { color: [0.53, 0.80, 1.00], size: 0.7, fall: 46, drift: 0.55, fog: [0.60, 0.66, 0.76], fogMix: 0.32, fogFar: 460, sparkle: false },
  desert_sandstorm:   { color: [0.79, 0.63, 0.29], size: 1.5, fall: 7,  drift: 1.40, fog: [0.78, 0.64, 0.40], fogMix: 0.48, fogFar: 300, sparkle: false },
  rocky_crystal_dust: { color: [0.67, 0.87, 1.00], size: 1.2, fall: 16, drift: 0.90, fog: [0.70, 0.78, 0.90], fogMix: 0.22, fogFar: 520, sparkle: true  },
  water_sea_mist:     { color: [0.85, 0.90, 0.93], size: 2.1, fall: -6, drift: 0.70, fog: [0.80, 0.85, 0.89], fogMix: 0.42, fogFar: 360, sparkle: false },
};

// 純函式：把一筆 weather 算成這一刻的視覺參數。確定性、壞值安全（缺欄位／晴天／非有限 → density 0、
// 不染色、不掉粒子），供每幀平滑趨近，亦供 render-smoke 斷言。
function weatherVisual(weather) {
  const type = weather && typeof weather.weather_type === "string" ? weather.weather_type : "clear";
  const spec = WEATHER_SPEC[type];
  const intensity = weather && Number.isFinite(weather.intensity) ? dnClamp01(weather.intensity) : 0;
  if (!spec || intensity <= 0.001) {
    return { density: 0, color: [0.8, 0.85, 0.9], size: 1, fall: 0, windX: 0, windZ: 0,
             fogTint: null, fogMix: 0, fogFar: 600, sparkle: false };
  }
  // 世界風：水平分量推動粒子橫向漂移（與 2D、與會吹彎樹梢的同一陣風一致）
  const w = weather.wind;
  const wx = w && Number.isFinite(w.dir_x) ? w.dir_x : 0;
  const wz = w && Number.isFinite(w.dir_y) ? w.dir_y : 0;
  const ws = w && Number.isFinite(w.strength) ? dnClamp01(w.strength) : 0;
  const windMag = ws * spec.drift * 22; // 風速 → 粒子橫移速度（場景單位/秒）
  return {
    density: intensity,
    color: spec.color,
    size: spec.size,
    fall: spec.fall,
    windX: wx * windMag,
    windZ: wz * windMag,
    fogTint: spec.fog,
    fogMix: spec.fogMix * intensity,
    // 晴天視野 600 ↔ 此天氣 fogFar，按強度內插（強度越大視野越短）
    fogFar: dnLerp(600, spec.fogFar, intensity),
    sparkle: spec.sparkle,
  };
}

// 天氣 → 面向玩家的 HUD 標籤（glyph 鏡像 2D WEATHER_INFO；後端只送穩定 snake_case 碼＝留 i18n 空間）。
const WX_LABEL = { grassland_rain: "🌧️ 細雨", desert_sandstorm: "🌪️ 風沙", rocky_crystal_dust: "✨ 晶塵", water_sea_mist: "🌊 海霧" };
// 純函式：取天氣＋彩虹的 HUD 標籤；晴天且無彩虹 → 空字串（HUD 不顯示，向後相容）。
function weatherHudLabel(weather, rainbow) {
  let s = "";
  if (weather && typeof weather.weather_type === "string") {
    const intensity = Number.isFinite(weather.intensity) ? weather.intensity : 0;
    if (intensity > 0.05) s = WX_LABEL[weather.weather_type] || "";
  }
  if (rainbow && rainbow.active) s = s ? s + " · 🌈 彩虹" : "🌈 彩虹";
  return s;
}

// ── 粒子場：一片圍著鏡頭的盒子，粒子在盒內以本地座標循環（永遠跟著玩家，落出底就從頂回收）──
const _wxTouch = ("ontouchstart" in window) || (navigator.maxTouchPoints > 0);
const WX_CAP = _wxTouch ? 300 : 620;   // 粒子上限（手機減量，省 GPU）
const WX_BOX_XZ = 140, WX_BOX_Y = 96;  // 取樣盒水平／垂直尺寸（場景單位）
const wxPositions = new Float32Array(WX_CAP * 3); // 本地偏移（相對 wxPoints.position，每幀吸到鏡頭）
const wxJitter = new Float32Array(WX_CAP * 3);    // 每顆的個別落速倍率＋橫向抖動（破除整齊感）
for (let i = 0; i < WX_CAP; i++) {
  wxPositions[i * 3]     = (Math.random() - 0.5) * WX_BOX_XZ;
  wxPositions[i * 3 + 1] = Math.random() * WX_BOX_Y;
  wxPositions[i * 3 + 2] = (Math.random() - 0.5) * WX_BOX_XZ;
  wxJitter[i * 3]        = (Math.random() - 0.5) * 6;  // x 抖動速度
  wxJitter[i * 3 + 1]    = 0.6 + Math.random() * 0.8;  // 落速倍率（雨絲快慢不一）
  wxJitter[i * 3 + 2]    = (Math.random() - 0.5) * 6;  // z 抖動速度
}
const wxGeo = new THREE.BufferGeometry();
wxGeo.setAttribute("position", new THREE.BufferAttribute(wxPositions, 3));
const wxMat = new THREE.PointsMaterial({ size: 1, transparent: true, opacity: 0, depthWrite: false, sizeAttenuation: true });
const wxPoints = new THREE.Points(wxGeo, wxMat);
wxPoints.frustumCulled = false; // 永遠繞著鏡頭，不要被視錐剔除
wxPoints.visible = false;
scene.add(wxPoints);

// ── 彩虹弧：七條同心半環疊出彩虹，掛在遠空、鏡頭走它也守在天邊（雨後彩虹祝福，鏡像 2D 361）──
const WX_RAINBOW_BANDS = [
  [0.86, 0.27, 0.24], [0.93, 0.55, 0.22], [0.95, 0.85, 0.32],
  [0.36, 0.78, 0.40], [0.30, 0.56, 0.92], [0.36, 0.40, 0.82], [0.52, 0.32, 0.70],
];
const rainbowGroup = new THREE.Group();
for (let i = 0; i < WX_RAINBOW_BANDS.length; i++) {
  const c = WX_RAINBOW_BANDS[i];
  const band = new THREE.Mesh(
    new THREE.TorusGeometry(150 + i * 6, 3.0, 6, 48, Math.PI), // arc=π → 上半環
    new THREE.MeshBasicMaterial({ color: new THREE.Color(c[0], c[1], c[2]), transparent: true, opacity: 0, depthWrite: false })
  );
  rainbowGroup.add(band);
}
rainbowGroup.visible = false;
scene.add(rainbowGroup);
let rainbowFade = 0; // 彩虹淡入淡出進度 [0,1]

// 平滑趨近的目前天氣視覺狀態（初始＝晴天）。
const _wxInit = weatherVisual(null);
let wxDensity = _wxInit.density;
const wxColor = _wxInit.color.slice();
let wxSize = _wxInit.size;
let wxFall = _wxInit.fall;
let wxWindX = 0, wxWindZ = 0;
let wxFogMix = 0;
let wxFogFar = 600;
const wxFogTint = [0.8, 0.85, 0.9]; // 持有最後一次有效染色（淡出時續用、不閃白）

// 每幀：算目標、平滑趨近、推進粒子、把霧／天空往天氣色微染、流轉彩虹。
// 在 applyDayNight 之後呼叫——日夜先把 dnFog/dnSky 寫好，天氣再疊染在其上。
function applyWeather(dt) {
  const target = weatherVisual(latestWeather);
  const a = 1 - Math.exp(-Math.max(0, dt) * WX_RATE);
  wxDensity = dnLerp(wxDensity, target.density, a);
  for (let i = 0; i < 3; i++) wxColor[i] = dnLerp(wxColor[i], target.color[i], a);
  wxSize = dnLerp(wxSize, target.size, a);
  wxFall = dnLerp(wxFall, target.fall, a);
  wxWindX = dnLerp(wxWindX, target.windX, a);
  wxWindZ = dnLerp(wxWindZ, target.windZ, a);
  wxFogMix = dnLerp(wxFogMix, target.fogMix, a);
  wxFogFar = dnLerp(wxFogFar, target.fogFar, a);
  if (target.fogTint) { wxFogTint[0] = target.fogTint[0]; wxFogTint[1] = target.fogTint[1]; wxFogTint[2] = target.fogTint[2]; }

  // 霧色／天空往天氣色微染（疊在日夜已寫好的 dnFog/dnSky 之上），並把霧拉近製造起霧感。
  if (scene.fog) {
    const m = dnClamp01(wxFogMix);
    if (m > 0.001) {
      scene.fog.color.setRGB(dnLerp(dnFog[0], wxFogTint[0], m), dnLerp(dnFog[1], wxFogTint[1], m), dnLerp(dnFog[2], wxFogTint[2], m));
      dnSkyColor.setRGB(dnLerp(dnSky[0], wxFogTint[0], m * 0.7), dnLerp(dnSky[1], wxFogTint[1], m * 0.7), dnLerp(dnSky[2], wxFogTint[2], m * 0.7));
    }
    scene.fog.far = wxFogFar;
  }

  // 粒子推進：偏好減少動態時不顯示粒子（鏡像 2D drawWeatherParticles 在 reduceMotion 直接早退），
  // 但保留霧染／彩虹（皆靜態、無刺眼動態）。
  if (!reduceMotion && wxDensity > 0.01) {
    wxPoints.visible = true;
    let op = wxDensity * 0.72;
    if (target.sparkle) op *= 0.7 + 0.3 * Math.sin(clock.elapsedTime * 4); // 晶塵忽明忽暗
    wxMat.opacity = op;
    wxMat.size = wxSize;
    wxMat.color.setRGB(wxColor[0], wxColor[1], wxColor[2]);
    // 盒子吸到鏡頭（略往下偏，讓粒子多半在視線高度以上落下）
    wxPoints.position.set(camera.position.x, camera.position.y - WX_BOX_Y * 0.35, camera.position.z);
    const halfXZ = WX_BOX_XZ / 2;
    const active = Math.max(1, Math.round(WX_CAP * dnClamp01(wxDensity)));
    for (let i = 0; i < active; i++) {
      const ix = i * 3;
      wxPositions[ix]     += (wxWindX + wxJitter[ix]) * dt;
      wxPositions[ix + 1] += (-wxFall * wxJitter[ix + 1]) * dt;
      wxPositions[ix + 2] += (wxWindZ + wxJitter[ix + 2]) * dt;
      // 在本地盒內循環回收（落出底→回頂，上飄出頂→回底，水平出界→繞回另一側）
      let y = wxPositions[ix + 1];
      if (y < 0) y += WX_BOX_Y; else if (y > WX_BOX_Y) y -= WX_BOX_Y;
      wxPositions[ix + 1] = y;
      let x = wxPositions[ix];
      if (x < -halfXZ) x += WX_BOX_XZ; else if (x > halfXZ) x -= WX_BOX_XZ;
      wxPositions[ix] = x;
      let z = wxPositions[ix + 2];
      if (z < -halfXZ) z += WX_BOX_XZ; else if (z > halfXZ) z -= WX_BOX_XZ;
      wxPositions[ix + 2] = z;
    }
    wxGeo.setDrawRange(0, active);
    wxGeo.attributes.position.needsUpdate = true;
  } else {
    wxPoints.visible = false;
  }

  // 彩虹弧：依權威 active 旗標淡入淡出，掛在遠空、每幀守在鏡頭天邊（固定世界朝向、不隨鏡頭轉）。
  const rbTarget = (latestRainbow && latestRainbow.active) ? 1 : 0;
  rainbowFade = dnLerp(rainbowFade, rbTarget, a);
  if (rainbowFade > 0.01) {
    rainbowGroup.visible = true;
    rainbowGroup.position.set(camera.position.x - 80, camera.position.y - 30, camera.position.z - 300);
    for (const band of rainbowGroup.children) band.material.opacity = rainbowFade * 0.45;
  } else {
    rainbowGroup.visible = false;
  }
}

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
const WILDLIFE_COLOR = 0x7fd87f; // 野生動物：未知種類的安全後備色（綠）
const ENEMY_COLOR = 0xff5a5a;    // 敵人：紅色盒子（盒子，跟「人」一眼區分）

// ============================================================
// 野生動物在 3D 裡有了模樣（ROADMAP 615）：把快照裡早就有、2D 一直畫得活靈活現、
// 3D 卻全擠成同一個綠盒子的 `wildlife`（種類 wild_bird／wild_deer／small_critter／
// wild_wolf／wild_fox、行為 state、馴養 tamed／親近 familiarity、幼獸 juvenile／scale）
// 在 3D 呈現出來——鳥／鹿／小獸／野狼／野狐各有低多邊形身形，馴養的頂著 💛、
// 幼獸縮小一號頂著 ✨、歇息的飄 💤。純讀快照、零後端改動、零協議改動。
// ============================================================
// 各種野生動物的視覺規格：顏色鏡像 2D game.js（SPECIES rgb），身形分四型——
// bird（小鳥）／quadruped（鹿，帶角）／critter（圓滾小獸）／predator（狼狐，尖耳長尾）。
// 後端只送穩定的 snake_case 種類碼，身形／顏色由前端對照＝留 i18n／美術一致空間。
const WILDLIFE_SPEC = {
  wild_bird:     { type: "bird",      color: 0x87cefa }, // 野鳥：天藍（2D 135,206,250）
  wild_deer:     { type: "quadruped", color: 0x6cb45a }, // 野鹿：草綠（2D 60,180,80）
  small_critter: { type: "critter",   color: 0xd2aa64 }, // 小動物：土黃（2D 210,170,100）
  wild_wolf:     { type: "predator",  color: 0x9098a0 }, // 野狼：灰
  wild_fox:      { type: "predator",  color: 0xe0883c }, // 野狐：橘
};
const WILDLIFE_DEFAULT_SPEC = { type: "box", color: WILDLIFE_COLOR }; // 未知種類 → 退回綠盒（向後相容）

// 把一筆 WildlifeView 算成「這隻該怎麼呈現」。純函式、確定性、壞值安全（null／非物件 →
// 安全的綠盒後備，永不 throw）。只讀權威欄位、不嵌任何生態規則（行為是伺服器的事，前端純呈現）。
function wildlifeVisual(item) {
  const spec = (item && typeof item === "object" && WILDLIFE_SPEC[item.kind]) || WILDLIFE_DEFAULT_SPEC;
  // 幼獸縮一號：優先用伺服器送的相對體型 scale，缺漏／非有限時退回 juvenile 旗標推估。
  let scale = item && Number.isFinite(item.scale) ? item.scale : (item && item.juvenile ? 0.62 : 1.0);
  if (!(scale > 0.2)) scale = 0.2; // 夾住下限，別縮成看不見
  if (scale > 1.6) scale = 1.6;     // 也夾上限，壞值不爆大
  const state = item && typeof item.state === "string" ? item.state : "";
  return {
    type: spec.type,
    color: spec.color,
    scale,
    tamed: !!(item && item.tamed),
    familiarity: item && Number.isFinite(item.familiarity) ? Math.max(0, Math.min(1, item.familiarity)) : 0,
    juvenile: !!(item && item.juvenile),
    // 歇息（resting／sleeping／napping）：穩定靜止，前端飄 💤、不上下彈
    resting: state === "resting" || state === "sleeping" || state === "napping",
  };
}

// 野生動物頭頂狀態 emoji（優先序：馴養 💛 ＞ 親近過半 💗 ＞ 幼獸 ✨ ＞ 歇息 💤 ＞ 無）。
// 鏡像 2D「馴養顯示愛心、親近度漸滿、幼獸新生微光」。壞值安全（回 null＝不顯示）。
function wildlifeStatusEmoji(item) {
  const v = wildlifeVisual(item);
  if (v.tamed) return "💛";
  if (v.familiarity > 0.5) return "💗";
  if (v.juvenile) return "✨";
  if (v.resting) return "💤";
  return null;
}

// 視野內野生動物的 HUD 標籤：幾隻、其中幾隻已馴養。純函式、壞值安全（空陣列回空字串）。
// 面向玩家字串集中前端、glyph 留 i18n 空間（後端只送穩定欄位，文案由前端對照）。
function wildlifeHudLabel(list) {
  if (!Array.isArray(list) || list.length === 0) return "";
  let tamed = 0;
  for (const w of list) if (w && w.tamed) tamed++;
  return `🦌 野生 ${list.length}${tamed > 0 ? " · 馴養 " + tamed : ""}`;
}

// ============================================================
// 玩家的寵物夥伴在 3D 裡現身（ROADMAP 627）：快照早就帶著每位玩家的權威寵物欄位
// （種類 pet_kind／世界座標 pet_x/pet_y／正與別的寵物玩耍 pet_playing／正叼玩具 pet_fetching／
// 性格 pet_personality／羈絆 pet_bond，見 protocol.rs；2D game.js 一直在主人腳邊畫出跟隨的小夥伴、
// 玩耍愛心、性格心情泡泡、羈絆默契愛心條），3D 卻整個忽略——於是養了寵物的玩家站在 3D 世界裡，
// 身邊空無一物。本切片把寵物接進 3D：每種寵物一具辨識得出的低多邊形小身形＋頭頂玩耍／性格狀態＋
// 腳邊羈絆愛心條，自有世界座標跟著主人在世界裡跑（走與玩家同套快照內插）。
// 純前端、純讀快照——零後端改動、零協議改動、零 migration、零 LLM、零經濟。
// ============================================================

// 寵物視覺規格：低多邊形身型／顏色，鏡像 2D game.js 的 PET_EMOJI 五種寵物。
// 後端只送穩定的 snake_case 種類碼，身形／顏色由前端對照＝留 i18n／美術一致空間。
const PET_SPEC = {
  flutter_sprite:  { type: "sprite",   color: 0xe2a8f0 }, // 翩翩精靈（🧚）：粉紫，圓身雙翼
  crystal_golem:   { type: "crystal",  color: 0x7ad0e8 }, // 晶石魔像（💠）：青藍，八面晶簇
  coral_crab:      { type: "crab",     color: 0xf08a55 }, // 珊瑚蟹（🦀）：珊瑚橘，扁殼雙螯
  jade_wraith:     { type: "wraith",   color: 0xa6e6c4 }, // 翡翠幽靈（👻）：淡碧，飄浮圓身
  origin_guardian: { type: "guardian", color: 0xffd860 }, // 初源守護（🌟）：金，星核斜環
};
const PET_DEFAULT_SPEC = { type: "box", color: 0xc8b8e0 }; // 未知種類 → 退回小盒（向後相容）

// 寵物性格心情泡泡（鏡像 2D PET_PERSONALITY mood）：活潑🎵／慵懶💤／好奇❓／黏人💕。
const PET_PERSONALITY_MOOD = { playful: "🎵", lazy: "💤", curious: "❓", clingy: "💕" };
const PET_BOND_MAX = 5; // 與 2D petBondHearts 同上限

// 把一筆玩家快照的寵物欄位算成「這隻寵物該怎麼呈現」。純函式、確定性、壞值安全
// （無 pet_kind／非物件 → 回 null＝這位玩家沒寵物、不畫）。只讀權威欄位、不嵌行為規則。
function petVisual(p) {
  if (!p || typeof p !== "object" || typeof p.pet_kind !== "string" || !p.pet_kind) return null;
  const spec = PET_SPEC[p.pet_kind] || PET_DEFAULT_SPEC;
  const bond = Number.isFinite(p.pet_bond) ? Math.max(0, Math.min(PET_BOND_MAX, p.pet_bond | 0)) : 0;
  return {
    kind: p.pet_kind,
    type: spec.type,
    color: spec.color,
    playing: !!p.pet_playing,   // 正與別的寵物玩耍：蹦得最歡、頭頂飄愛心
    fetching: !!p.pet_fetching, // 正叼玩具衝刺：蹦得最急、頭頂掛 🎾
    personality: typeof p.pet_personality === "string" ? p.pet_personality : "",
    bond,
  };
}

// 寵物頭頂狀態 emoji（優先序：玩耍 💞 ＞ 接物 🎾 ＞ 性格心情泡泡 ＞ 無）。
// 鏡像 2D「玩耍飄愛心、歇腳依性格飄心情泡泡」。壞值安全（回 null＝不顯示）。
function petStatusEmoji(p) {
  const v = petVisual(p);
  if (!v) return null;
  if (v.playing) return "💞";
  if (v.fetching) return "🎾";
  return PET_PERSONALITY_MOOD[v.personality] || null;
}

// 寵物羈絆默契愛心條（鏡像 2D petBondHearts）：實心♥＝已養階數、空心♡＝未滿。
// bond<=0＝還沒養出默契→回空字串（不畫、零干擾）。純函式、好測。
function petBondHearts(bond) {
  const b = Math.max(0, Math.min(PET_BOND_MAX, bond | 0));
  if (b <= 0) return "";
  return "♥".repeat(b) + "♡".repeat(PET_BOND_MAX - b);
}

// 視野內寵物夥伴的 HUD 標籤：幾隻。純函式、壞值安全（無寵物回空字串）。
function petHudLabel(players) {
  if (!Array.isArray(players)) return "";
  let n = 0;
  for (const p of players) if (p && typeof p.pet_kind === "string" && p.pet_kind) n++;
  return n > 0 ? `🐾 夥伴 ${n}` : "";
}

// ============================================================
// 夜間的威脅在 3D 裡現形（ROADMAP 626）：把快照裡早就有、2D 一直畫得有血有肉、3D 卻全擠成
// 同一個無差別紅盒子的 `enemies`（種類 kind／等級 level／血量 hp/max_hp／兇名 notorious／
// 夜歇 resting／潰逃 routing／破綻 weak）在 3D 呈現出來——機械／靈體／巨像三型身形各異、
// 受傷的頭頂浮血條（殘血轉深紅）、兇名精英體型微大頂著 💢、破綻時刻閃 ✨、夜歇 💤／潰逃 💨。
// 回應 AI 居民反覆許願的「夜間缺乏目標／看不出危險」，也讓 623 篝火「逼退的是什麼」終於看得見。
// 純讀快照、零後端改動、零協議改動——資料本來就在 EnemyView 裡（2D game.js 早在用）。
// ============================================================
// 各敵種的身形原型＋配色（後端只送穩定 snake_case kind，身形/顏色由前端對照＝留 i18n／美術一致）。
// 三大身形原型，對應「機械／靈體／巨像」三類威脅感：drone（稜角硬殼＋頂上轉子）／
// wisp（懸浮八面體幽靈）／golem（龐然晶體巨像）。各 kind 配色刻意拉開色相，一眼分得出種類。
const ENEMY_SPEC = {
  scrap_drone:      { type: "drone", color: 0xb86b3a }, // 廢鐵無人機：銹橘
  rune_guardian:    { type: "drone", color: 0xd9b34a }, // 符文守衛：沙金（沙漠機械）
  steam_construct:  { type: "drone", color: 0xff5a3c }, // 蒸汽構裝：熔岩橙紅
  ether_wisp:       { type: "wisp",  color: 0xb060ff }, // 乙太鬼火：乙太紫
  flutter_sprite:   { type: "wisp",  color: 0xf4a6d6 }, // 飄舞精靈：花粉粉
  jade_wraith:      { type: "wisp",  color: 0x4fd99a }, // 翠幽魅影：翠綠
  void_phantom:     { type: "wisp",  color: 0x7a52a8 }, // 虛空幽靈：紫黑
  aether_specter:   { type: "wisp",  color: 0xbfe6e6 }, // 霧醚幻靈：青白
  crystal_golem:    { type: "golem", color: 0x6fd6e0 }, // 晶石傀儡：晶青
  mushroom_stalker: { type: "golem", color: 0xc0573c }, // 蕈菇潛行者：菇紅褐
  coral_crab:       { type: "golem", color: 0xff7f6a }, // 珊瑚蟹：珊瑚橙
  origin_guardian:  { type: "golem", color: 0xffd24a }, // 源晶守護者：黃金
  rift_guardian:    { type: "golem", color: 0xc060ff }, // 裂縫守護者：次元紫
  ether_overlord:   { type: "golem", color: 0xcc3030 }, // 乙太霸主：黑紅（入侵頭目）
};
const ENEMY_DEFAULT_SPEC = { type: "drone", color: ENEMY_COLOR }; // 未知種類 → 紅機械盒（向後相容）

// 把一筆 EnemyView 算成「這隻該怎麼呈現」。純函式、確定性、壞值安全（null／非物件 → 安全後備、
// 永不 throw）。只讀權威欄位、不嵌任何戰鬥規則（危險度／行為是伺服器的事，前端純呈現）。
function enemyVisual(item) {
  const spec = (item && typeof item === "object" && ENEMY_SPEC[item.kind]) || ENEMY_DEFAULT_SPEC;
  const hp = item && Number.isFinite(item.hp) ? Math.max(0, item.hp) : 0;
  const maxHp = item && Number.isFinite(item.max_hp) && item.max_hp > 0 ? item.max_hp : 0;
  const ratio = maxHp > 0 ? Math.max(0, Math.min(1, hp / maxHp)) : 1; // 缺血量資訊 → 視為滿（不畫血條）
  const notorious = !!(item && item.notorious);
  return {
    type: spec.type,
    color: spec.color,
    // 兇名精英體型微大（鏡像 2D：notorious 體型放大、全服通告過）；其餘 1.0。
    scale: notorious ? 1.32 : 1.0,
    hpRatio: ratio,
    damaged: maxHp > 0 && ratio < 1, // 受過傷才畫血條（滿血不畫，減雜訊）
    level: item && Number.isFinite(item.level) ? Math.max(0, Math.floor(item.level)) : 0,
    notorious,
    resting: !!(item && item.resting),
    routing: !!(item && item.routing),
    weak: !!(item && item.weak),
  };
}

// 敵人頭頂狀態 emoji（優先序：破綻 ✨＞潰逃 💨＞夜歇 💤＞兇名 💢＞無）。
// 鏡像 2D 的破綻光環／潰逃 💨／夜歇 💤／兇名標記。壞值安全（回 null＝不顯示）。
function enemyStatusEmoji(item) {
  const v = enemyVisual(item);
  if (v.weak) return "✨";      // 兇名精英露出破綻：現在砍傷害最高
  if (v.routing) return "💨";   // 潰逃中（強制逃離玩家）
  if (v.resting) return "💤";   // 夜間回巢靜止
  if (v.notorious) return "💢"; // 兇名精英（無上述狀態時的持續標記）
  return null;
}

// 血條填充規格（ROADMAP 626，鏡像 2D 敵人血條）：純函式、確定性、壞值安全。
// 回填充比例 ratio∈[0,1] 與是否殘血 critical（≤30%＝快倒了、轉深紅）。缺血量資訊 → 視為滿、不殘血。
function enemyHpFill(hp, maxHp) {
  const h = Number(hp), m = Number(maxHp);
  if (!Number.isFinite(h) || !Number.isFinite(m) || m <= 0) return { ratio: 1, critical: false };
  const ratio = Math.max(0, Math.min(1, h / m));
  return { ratio, critical: ratio <= 0.3 };
}

// 視野內敵人的 HUD 標籤：幾隻威脅、其中幾隻兇名精英。只數活著的（alive!==false）。
// 純函式、壞值安全。面向玩家字串集中前端、glyph 留 i18n 空間（後端只送穩定欄位，文案前端對照）。
function enemyHudLabel(list) {
  if (!Array.isArray(list)) return "";
  let n = 0, noto = 0;
  for (const e of list) {
    if (!e || e.alive === false) continue;
    n++;
    if (e.notorious) noto++;
  }
  if (n === 0) return "";
  return `⚔️ 威脅 ${n}${noto > 0 ? " · 兇名 " + noto : ""}`;
}

// ============================================================
// AI 居民的內心生活（ROADMAP 611）：把快照裡早就有、2D 看得到、3D 卻一直忽略的
// activity／thought／needs_care／alarmed／celebrating 在 3D 頭頂呈現出來，
// 讓住在這個世界裡的 AI 居民「看得出在做什麼、在想什麼、心情如何」。
// 純讀快照、零後端改動、零協議改動——資料本來就在 NpcView 裡。
// ============================================================
// 偏好減少動態：尊重系統設定，關掉跳動／脈動（鏡像 2D game.js 的 reduceMotion）。
const reduceMotion =
  typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

// 故鄉七大居民的「當前活動 → 頭頂符號」對照（鏡像 web/game.js 的 NPC_ACTIVITY_ICON；
// 後端只送穩定的 activity 代碼，文案／符號由前端對照＝留 i18n 空間，別把字面意義寫死進後端）。
const NPC_ACTIVITY_ICON = {
  commuting: "🚶",   // 趕路中
  resting: "💤",     // 夜間休憩
  lunching: "🍲",    // 正午聚到廣場用餐
  tallying: "🪙",    // 商人點算貨銀
  hammering: "🔨",   // 工匠敲打鍛造
  sharpening: "🏹",  // 獵手擦拭上弦
  mapping: "🗺️",     // 探勘員看地圖
  stocktaking: "📦", // 採購清點備貨
  judging: "📋",     // 評審打分
  patrolling: "👀",  // 里長巡視
  visiting: "🤝",    // 黃昏串門子
};

// 純函式：依一筆 NpcView 算出「頭頂主狀態 emoji」。優先序與 2D 一致——
// 危機避難 😰 ＞ 凱旋歡慶 🎉 ＞ 當前活動符號；都沒有則回 null（不顯示）。
// 壞資料（item 為 null／activity 對不到）一律安全回 null，不讓渲染掛掉。
function residentStatusEmoji(item) {
  if (!item) return null;
  if (item.alarmed) return "😰";
  if (item.celebrating) return "🎉";
  if (item.activity) return NPC_ACTIVITY_ICON[item.activity] || null;
  return null;
}

// 共用 emoji 貼圖快取：同一顆 emoji 全場共用一張 CanvasTexture（distinct emoji 數很小、自然有界）。
const emojiTexCache = new Map();
function emojiTexture(emoji) {
  let tex = emojiTexCache.get(emoji);
  if (tex) return tex;
  const canvas = document.createElement("canvas");
  canvas.width = 64; canvas.height = 64;
  const ctx = canvas.getContext("2d");
  ctx.font = "48px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  ctx.fillText(emoji, 32, 36);
  tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  emojiTexCache.set(emoji, tex);
  return tex;
}

// 思想泡泡 💭 貼圖：淡紫圓角底 + 白字（鏡像 2D drawResidentThought 的柔和調性）。
// 文案是後端罐頭短句、種類有界，仍加 FIFO 上限保險，避免長跑累積貼圖。
const thoughtTexCache = new Map();
const THOUGHT_CACHE_MAX = 48;
function thoughtTexture(text) {
  const key = String(text == null ? "" : text);
  let tex = thoughtTexCache.get(key);
  if (tex) return tex;
  const canvas = document.createElement("canvas");
  canvas.width = 512; canvas.height = 96;
  const ctx = canvas.getContext("2d");
  // 內心話本就短：過長截斷（與 2D 一致，泡泡不撐爆）
  let label = "💭 " + key;
  if (label.length > 22) label = label.slice(0, 21) + "…";
  ctx.font = "30px system-ui, sans-serif";
  const tw = Math.min(500, ctx.measureText(label).width + 36);
  const bx = (512 - tw) / 2, by = 20, bw = tw, bh = 56, r = 16;
  // 圓角底（淡紫、半透明，比對話泡泡更柔）
  ctx.fillStyle = "rgba(70,55,110,0.78)";
  ctx.beginPath();
  ctx.moveTo(bx + r, by);
  ctx.arcTo(bx + bw, by, bx + bw, by + bh, r);
  ctx.arcTo(bx + bw, by + bh, bx, by + bh, r);
  ctx.arcTo(bx, by + bh, bx, by, r);
  ctx.arcTo(bx, by, bx + bw, by, r);
  ctx.closePath();
  ctx.fill();
  ctx.fillStyle = "#f3ecff";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  ctx.fillText(label, 256, by + bh / 2 + 1);
  tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  // FIFO 汰換：超量就丟最舊一張並釋放 GPU 資源
  if (thoughtTexCache.size >= THOUGHT_CACHE_MAX) {
    const oldestKey = thoughtTexCache.keys().next().value;
    const old = thoughtTexCache.get(oldestKey);
    if (old) old.dispose();
    thoughtTexCache.delete(oldestKey);
  }
  thoughtTexCache.set(key, tex);
  return tex;
}

// 對話泡泡貼圖（ROADMAP 622）：白底圓角＋深字（鏡像 2D drawNpcSpeechBubbles 的「說出口的話」調性，
// 刻意比 💭 思想泡泡的淡紫底更明亮、更像真的在講話——一個是內心獨白、一個是對人說的話）。
// 對白由後端 AI/罐頭生成、種類可多，仍加 FIFO 上限保險，避免長跑累積貼圖。
const speechTexCache = new Map();
const SPEECH_CACHE_MAX = 64;
function speechTexture(text) {
  const key = String(text == null ? "" : text);
  let tex = speechTexCache.get(key);
  if (tex) return tex;
  const canvas = document.createElement("canvas");
  canvas.width = 512; canvas.height = 96;
  const ctx = canvas.getContext("2d");
  // 對白可能偏長：過長截斷（與 2D 一致，泡泡不撐爆）
  let label = key;
  if (label.length > 24) label = label.slice(0, 23) + "…";
  ctx.font = "30px system-ui, sans-serif";
  const tw = Math.min(500, ctx.measureText(label).width + 40);
  const bx = (512 - tw) / 2, by = 18, bw = tw, bh = 60, r = 18;
  // 圓角底（近白、明亮——說出口的話比內心話醒目）
  ctx.fillStyle = "rgba(252,250,245,0.94)";
  ctx.beginPath();
  ctx.moveTo(bx + r, by);
  ctx.arcTo(bx + bw, by, bx + bw, by + bh, r);
  ctx.arcTo(bx + bw, by + bh, bx, by + bh, r);
  ctx.arcTo(bx, by + bh, bx, by, r);
  ctx.arcTo(bx, by, bx + bw, by, r);
  ctx.closePath();
  ctx.fill();
  ctx.fillStyle = "#2a2326";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  ctx.fillText(label, 256, by + bh / 2 + 1);
  tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  // FIFO 汰換：超量就丟最舊一張並釋放 GPU 資源
  if (speechTexCache.size >= SPEECH_CACHE_MAX) {
    const oldestKey = speechTexCache.keys().next().value;
    const old = speechTexCache.get(oldestKey);
    if (old) old.dispose();
    speechTexCache.delete(oldestKey);
  }
  speechTexCache.set(key, tex);
  return tex;
}

// 純函式：一枚對話泡泡在 elapsedMs 這一刻的顯示參數（快速淡入→穩定停留→末段淡出）。
// 對白泡泡比表情泡泡沉穩——不彈跳、不上浮，只是浮在頭頂幾秒後淡去（鏡像 2D 的對話泡泡）。
// displaySecs＝後端廣播的顯示秒數（預設 8，與 2D `display_secs || 8` 對齊）。確定性、壞值安全
// （非有限 elapsed／超出存活期 → visible:false，永不 throw）。
function npcSpeechVisual(elapsedMs, displaySecs) {
  const dur = (Number.isFinite(displaySecs) && displaySecs > 0 ? displaySecs : 8) * 1000;
  const e = Number.isFinite(elapsedMs) ? elapsedMs : -1;
  if (e < 0 || e >= dur) return { visible: false, opacity: 0 };
  const t = e / dur; // 0..1 存活進度
  // 起手 12% 快速淡入、末 28% 淡出、中間維持全亮
  let opacity = 1;
  if (t < 0.12) opacity = t / 0.12;
  else if (t > 0.72) opacity = Math.max(0, 1 - (t - 0.72) / 0.28);
  return { visible: true, opacity };
}

// 建一個透明的小 emoji sprite（一開始隱形；setSpriteEmoji 換上貼圖時才現身）。
function makeEmojiSprite(size) {
  const s = new THREE.Sprite(new THREE.SpriteMaterial({ transparent: true, depthTest: false, opacity: 0 }));
  s.scale.set(size, size, 1);
  s.visible = false;
  return s;
}
// 設定 sprite 顯示的 emoji（只在改變時換貼圖，省每幀重建）；回傳是否有東西可顯示。
function setSpriteEmoji(sprite, emoji) {
  if (sprite.userData.emoji !== emoji) {
    sprite.userData.emoji = emoji;
    if (emoji) { sprite.material.map = emojiTexture(emoji); sprite.material.needsUpdate = true; }
  }
  return !!emoji;
}

// ============================================================
// 玩家表情在 3D 裡比出來（ROADMAP 621）：把 2D 早有的「表情輪」接進 3D——
// 玩家↔玩家的即時情緒表態。按表情鈕送既有權威 `ClientMsg::Emote`，伺服器查白名單後廣播
// `ServerMsg::PlayerEmote` 回來，前端就在比表情的玩家頭頂彈跳浮起一枚大 emoji 後淡出。
// 純前端、純送既有訊息＋讀既有廣播——零後端改動、零協議改動、零 migration、零 LLM、零經濟。
// ============================================================

// 表情白名單（wire key, glyph, 中文標籤）——次序與後端 `player_emote::EMOTES` 一致。
// wire key 是穩定協議契約（不重排、只可尾端追加）；glyph 僅供按鈕顯示，真正畫在頭頂的 glyph
// 由伺服器廣播帶回＝單一真實來源；面向玩家中文標籤集中此處（i18n 友善）。
const EMOTE_CHOICES = [
  ["wave", "👋", "揮手"],
  ["cheer", "🎉", "歡呼"],
  ["heart", "❤️", "愛心"],
  ["laugh", "😆", "大笑"],
  ["thumbsup", "👍", "比讚"],
  ["cry", "😢", "哭哭"],
  ["angry", "😠", "生氣"],
  ["sleep", "💤", "想睡"],
];
const EMOTE_KEYS = new Set(EMOTE_CHOICES.map((c) => c[0]));

// 純函式：把一個 wire key 包成送伺服器的權威表情意圖。未知 key 回 null（前端不送偽造表情，
// 與後端白名單同一道防線）。確定性、壞值安全。
function emoteWireMsg(kind) {
  return EMOTE_KEYS.has(kind) ? { type: "emote", kind } : null;
}

// 純函式：一枚表情泡泡在 elapsedMs 這一刻的動畫參數（起手彈跳→緩緩上浮→末段淡出）。
// displaySecs＝後端廣播的顯示秒數（預設 4，與後端 EMOTE_DISPLAY_SECS 對齊）。確定性、壞值安全
// （非有限 elapsed／超出存活期 → visible:false，永不 throw）。
function emoteBubbleVisual(elapsedMs, displaySecs) {
  const dur = (Number.isFinite(displaySecs) && displaySecs > 0 ? displaySecs : 4) * 1000;
  const e = Number.isFinite(elapsedMs) ? elapsedMs : -1;
  if (e < 0 || e >= dur) return { visible: false, opacity: 0, rise: 0, scale: 1 };
  const t = e / dur; // 0..1 存活進度
  // 起手 18% 一個彈跳（easeOutCubic 彈起＋一個正弦過衝），之後回穩
  const popT = Math.min(1, t / 0.18);
  const pop = 1 - Math.pow(1 - popT, 3);
  const scale = 0.55 + 0.45 * pop + 0.14 * Math.sin(popT * Math.PI);
  const rise = 1.5 + 6 * t; // 緩緩上浮（場景單位）
  const opacity = t > 0.7 ? Math.max(0, 1 - (t - 0.7) / 0.3) : 1; // 末 30% 淡出
  return { visible: true, opacity, rise, scale };
}

// 給玩家 group 掛一枚表情泡泡 sprite（初始隱形，由 updatePlayerEmotes 每幀依 playerEmotes 決定）。
function attachEmoteBubble(g) {
  const s = makeEmojiSprite(7); // 比頭頂狀態大一號、醒目
  s.position.set(0, 15, 0);     // 浮在名字標籤（y=12）之上
  g.add(s);
  g.userData.emoteSprite = s;
}

// 給一個 NPC group 掛上三層內心生活的呈現 sprite（主狀態 emoji／💚 關懷側標／💭 思想泡泡）。
// 都疊在名字標籤之上，depthTest:false 浮在最前；初始隱形，由 updateResidentStatus 每幀依快照決定。
function attachResidentStatus(g) {
  const status = makeEmojiSprite(5);
  status.position.set(0, 13.5, 0);
  const care = makeEmojiSprite(3.6);
  care.position.set(3.6, 11.5, 0);
  const thought = new THREE.Sprite(new THREE.SpriteMaterial({ transparent: true, depthTest: false, opacity: 0 }));
  thought.scale.set(16, 3, 1);
  thought.position.set(0, 18, 0);
  thought.visible = false;
  // 對話泡泡（ROADMAP 622）：居民「說出口的話」。與思想泡泡同位（兩者互斥、不同時露出），
  // 但明亮白底、略寬，一眼區分「對人說的話」與「內心獨白」。由 updateNpcSpeech 每幀依 npcSpeech 決定。
  const speech = new THREE.Sprite(new THREE.SpriteMaterial({ transparent: true, depthTest: false, opacity: 0 }));
  speech.scale.set(18, 3.4, 1);
  speech.position.set(0, 18, 0);
  speech.visible = false;
  g.add(status); g.add(care); g.add(thought); g.add(speech);
  g.userData.statusSprite = status;
  g.userData.careSprite = care;
  g.userData.thoughtSprite = thought;
  g.userData.speechSprite = speech;
}

// 每幀更新所有 NPC 的內心生活呈現（在 updateRemoteEntities(npcs) 之後呼叫：
// 那裡的 updateFade 會把每個子 sprite 的 opacity 設成 AOI 淡入淡出值，這裡再乘上各自的
// 顯示強度覆寫上去，故 AOI 淡入淡出仍生效、又不被它壓掉內心生活的呈現）。
function updateResidentStatus(t) {
  for (const [id, g] of npcs) {
    const status = g.userData.statusSprite;
    if (!status) continue; // 非居民類 NPC group 沒掛（理論上都掛了，保險）
    const item = g.userData.item;
    const speaking = npcSpeech.has(id); // 正在說出口的話 → 讓位給對話泡泡，不同時露出思想（鏡像 2D）
    const fade = g.userData.fade ?? 1; // AOI 淡入淡出基底
    const care = g.userData.careSprite, thought = g.userData.thoughtSprite;

    // ① 主狀態 emoji（危機／歡慶／活動）
    const emoji = residentStatusEmoji(item);
    if (setSpriteEmoji(status, emoji)) {
      status.visible = true;
      let bob = 0;
      if (!reduceMotion && item) {
        // 危機輕跳、歡慶更快更高跳（鏡像 2D 的節奏暗示）
        if (item.alarmed) bob = Math.sin(t * 6 + g.position.x * 0.3) * 0.5;
        else if (item.celebrating) bob = Math.abs(Math.sin(t * 9 + g.position.x * 0.3)) * 1.1;
      }
      status.position.y = 13.5 + bob;
      status.material.opacity = fade;
    } else {
      status.visible = false;
    }

    // ② 關懷側標 💚（needs_care）：輕輕脈動，呼應 2D 的「這位需要被關心」
    if (item && item.needs_care) {
      setSpriteEmoji(care, "💚");
      care.visible = true;
      const pulse = reduceMotion ? 1 : 0.72 + 0.28 * Math.abs(Math.sin(t * 3));
      care.material.opacity = fade * pulse;
    } else if (care) {
      care.visible = false;
    }

    // ③ 思想泡泡 💭（thought）：有內心話、且當下沒在說出口的話時才飄；柔和半透明（鏡像 2D：
    // 對話泡泡與內心話互斥，說話時讓位給對話泡泡，不互疊）
    if (item && item.thought && !speaking) {
      thought.material.map = thoughtTexture(item.thought);
      thought.material.needsUpdate = true;
      thought.visible = true;
      thought.material.opacity = fade * 0.92;
    } else if (thought) {
      thought.visible = false;
    }
  }
}

// 每幀更新所有居民頭頂的對話泡泡（ROADMAP 622；在 npcs 的 updateFade／updateResidentStatus 之後呼叫：
// updateFade 已把 sprite opacity 設成 AOI 淡入淡出值，這裡再乘上對白自己的存活淡出覆寫上去）。
// 由 npcSpeech Map（npc_id → { text, startMs, displaySecs }）驅動；過期自清，說話者離開視野自然收掉。
function updateNpcSpeech(nowMs) {
  for (const [id, sp] of npcSpeech) {
    const vis = npcSpeechVisual(nowMs - sp.startMs, sp.displaySecs);
    if (!vis.visible) { npcSpeech.delete(id); } // 存活期已過 → 移除（對應 sprite 下面會被隱藏）
    const g = npcs.get(id);
    const s = g && g.userData.speechSprite;
    if (!s) continue; // 說話者不在視野內（或非居民類 group）→ 無從定位，安全跳過
    if (!vis.visible) { s.visible = false; continue; }
    s.material.map = speechTexture(sp.text);
    s.material.needsUpdate = true;
    s.visible = true;
    const fade = g.userData.fade ?? 1; // AOI 淡入淡出基底
    s.material.opacity = fade * vis.opacity;
  }
}

// ── 居民派系關係在 3D 裡看得見（ROADMAP 625）──────────────────────────────
// 後端早已在快照送 town_factions：七大居民「此刻」自然湧現的結盟／敵對配對（355）。
// 2D 把它畫成右下角面板；3D 更進一步——把這份「看不見的社會結構」**畫成兩人之間的關係連線**：
// 結盟＝一道暖金弧舒展相連、敵對＝一道警紅弧低伏緊繃，AI 社會的政治第一次「在世界裡看得見」。
// 後端只送 wire key（alliance/rivalry），配色/圖示/文案由前端鏡像（鏡像 2D 的 FACTION_BOND_STYLE，
// 保留 i18n 空間）。純前端讀既有快照、零後端改動、零持久化。
const FACTION_BOND_STYLE = {
  alliance: { color: 0xffd966, icon: "🤝", label: "結盟", arc: 5.0 }, // 暖金、弧高舒展（和睦）
  rivalry:  { color: 0xff6b6b, icon: "⚔️", label: "敵對", arc: 1.2 }, // 警紅、弧低緊繃（張力）
};
const FACTION_LINK_Y = 11;   // 連線端點高度（略高於頭頂 9.3、低於名牌），一眼看見「兩人之間有關係」
const FACTION_ARC_SEG = 14;  // 弧線取樣分段數

// 一條關係連線此刻的視覺（顏色固定、透明度隨關係類型輕脈動）：結盟徐徐呼吸（和睦），
// 敵對較快閃動且更明（張力）。未知 bond 回 null（呼叫端略過）。決定性、好測。
function factionLinkVisual(bond, t) {
  const st = FACTION_BOND_STYLE[bond];
  if (!st) return null;
  const speed = bond === "rivalry" ? 3.2 : 1.4;
  const base = bond === "rivalry" ? 0.6 : 0.66;
  const amp = bond === "rivalry" ? 0.3 : 0.18;
  const opacity = base + amp * (0.5 + 0.5 * Math.sin(t * speed));
  return { color: st.color, opacity, arc: st.arc };
}

// 兩名居民之間關係連線的取樣點（場景座標）：一道在兩人頭頂之上隆起的拋物弧。
// 端點落在兩人位置（高 FACTION_LINK_Y）、中點抬最高（arc 由 bond 決定：結盟高舒展、敵對低緊繃）。
// 回傳扁平 [x,y,z, x,y,z, …]（segments+1 個點）。決定性、好測。
function factionArcPoints(ax, az, bx, bz, arc, segments) {
  const seg = Math.max(2, segments | 0);
  const out = [];
  for (let i = 0; i <= seg; i++) {
    const f = i / seg;
    const x = ax + (bx - ax) * f;
    const z = az + (bz - az) * f;
    // 拋物線抬升：端點 0、中點最高（4·f·(1−f) 在 f=0.5 時 =1）
    const y = FACTION_LINK_Y + arc * 4 * f * (1 - f);
    out.push(x, y, z);
  }
  return out;
}

// 鎮民派系 HUD 一行：數出此刻幾組結盟、幾組敵對；和平相處（無明顯派系）回空字串。
// 未知 wire key 略過（與 2D 一致、不算進任何一類）。鏡像 farmHudLabel 風格、好測。
function factionHudLabel(factions) {
  if (!Array.isArray(factions) || !factions.length) return "";
  let ally = 0, rival = 0;
  for (const f of factions) {
    if (!f || !FACTION_BOND_STYLE[f.bond]) continue;
    if (f.bond === "alliance") ally++;
    else if (f.bond === "rivalry") rival++;
  }
  if (!ally && !rival) return "";
  const parts = [];
  if (ally) parts.push(`🤝 ${ally} 結盟`);
  if (rival) parts.push(`⚔️ ${rival} 敵對`);
  return parts.join(" · ");
}

// 最新一筆快照的鎮民派系配對（ROADMAP 355）；空陣列＝目前相處平和、無明顯派系。
let latestTownFactions = [];
// 關係連線實體池：pairKey → { line, geom, posAttr, mat }；隨關係解除／一方離線自然回收。
const factionLinks = new Map();
const factionLinkGroup = new THREE.Group();
scene.add(factionLinkGroup);
function factionPairKey(a, b, bond) { return bond + "|" + a + "|" + b; }

// 每幀更新所有居民之間的關係連線（在 npcs 的 updateRemoteEntities／updateResidentStatus 之後呼叫：
// 那時 npc group 的位置已內插到本幀、fade 也已更新）。連線位置吸兩人當下位置、隨關係類型脈動，
// 透明度再乘上兩端 AOI 淡入淡出的較小者（任一方淡出，連線一起淡掉，不殘留空中）。
function updateFactionLinks(t) {
  const seen = new Set();
  for (const f of latestTownFactions) {
    if (!f || !FACTION_BOND_STYLE[f.bond]) continue;
    const ga = npcs.get(f.npc_a), gb = npcs.get(f.npc_b);
    if (!ga || !gb) continue; // 兩位都在線且在視野內才畫得出關係
    const fadeA = ga.userData.fade ?? 1, fadeB = gb.userData.fade ?? 1;
    const fade = Math.min(fadeA, fadeB);
    const key = factionPairKey(f.npc_a, f.npc_b, f.bond);
    seen.add(key);
    const vis = factionLinkVisual(f.bond, t);
    const pts = factionArcPoints(ga.position.x, ga.position.z, gb.position.x, gb.position.z, vis.arc, FACTION_ARC_SEG);
    let entry = factionLinks.get(key);
    if (!entry) {
      const geom = new THREE.BufferGeometry();
      const posAttr = new THREE.Float32BufferAttribute(new Float32Array((FACTION_ARC_SEG + 1) * 3), 3);
      geom.setAttribute("position", posAttr);
      const mat = new THREE.LineBasicMaterial({ transparent: true, depthWrite: false });
      const line = new THREE.Line(geom, mat);
      factionLinkGroup.add(line);
      entry = { line, geom, posAttr, mat };
      factionLinks.set(key, entry);
    }
    const arr = entry.posAttr.array;
    for (let i = 0; i < pts.length; i++) arr[i] = pts[i];
    entry.posAttr.needsUpdate = true;
    if (entry.geom.computeBoundingSphere) entry.geom.computeBoundingSphere();
    entry.mat.color.setHex(vis.color);
    entry.mat.opacity = vis.opacity * fade;
    entry.line.visible = fade > 0.04; // 兩端幾近淡出時整條收掉，不殘影
  }
  // 清掉本幀不再存在的連線（關係解除、或有一方離線/離開視野）
  for (const [key, entry] of factionLinks) {
    if (seen.has(key)) continue;
    factionLinkGroup.remove(entry.line);
    if (entry.geom.dispose) entry.geom.dispose();
    if (entry.mat.dispose) entry.mat.dispose();
    factionLinks.delete(key);
  }
}

// ── 居民互助送禮（ROADMAP 369／town_share）：後端每幀已送 `town_share`＝「此刻一位寬裕的居民
//    正把心意勻給一位拮据的居民」{giver, receiver, t}（手勢進度 0→1）。2D 把它畫成一枚 🎁 暖光
//    沿弧飄越兩人之間；3D 一直空白。本切片把這份「居民彼此互助」的湧現畫進 3D：一枚暖金光禮
//    從送禮者頭頂浮起、循一道上隆的拋物弧飄向受禮者、頭尾淡入淡出——AI 社會的「善意流動」第一次
//    在 3D 世界裡看得見。純前端讀既有快照、零後端改動、零持久化、零新協議欄位。──
const TOWN_SHARE_ARC = 3.6;    // 光禮在兩人之間隆起的拋物抬升上限（場景單位；中段最高）
const TOWN_SHARE_BASE_Y = 12;  // 光禮飄送基準高度（略高於頭頂 9.3、與派系連線同層級，一眼看見「心意在飄送」）

// 光禮此刻的視覺規格（純函式、決定性、好測）。給手勢進度 t∈[0,1]：
// frac＝夾鉗後的行程比例；lift＝沿弧隆起的抬升（端點 0、中點最高，sin 峰在 t=0.5）；
// alpha＝頭尾各 12% 行程漸顯/漸隱、中途全亮（鏡像 2D drawTownShare）。
// 壞 t（NaN／越界）一律夾鉗到 [0,1]，render 不爆。
function townShareGiftSpec(t) {
  const f = Number.isFinite(+t) ? Math.max(0, Math.min(1, +t)) : 0;
  const lift = Math.sin(Math.PI * f) * TOWN_SHARE_ARC;
  const alpha = Math.max(0, Math.min(1, f / 0.12, (1 - f) / 0.12));
  return { frac: f, lift, alpha };
}

// 居民互助 HUD 一行：此刻有人正在互相分享心意則回提示，否則空字串（鏡像 factionHudLabel 風格、好測）。
function townShareHudLabel(share) {
  if (!share || !share.giver || !share.receiver) return "";
  return "🎁 互助分享";
}

// 光禮貼圖（單張快取）：暖金柔光暈＋🎁 本體烘在同一張透明畫布上（療癒暖調，鏡像 2D 的 radial halo）。
let townShareGiftTex = null;
function townShareGiftTexture() {
  if (townShareGiftTex) return townShareGiftTex;
  const canvas = document.createElement("canvas");
  canvas.width = 128; canvas.height = 128;
  const c = canvas.getContext("2d");
  // 暖金柔光暈
  const grad = c.createRadialGradient(64, 64, 0, 64, 64, 60);
  grad.addColorStop(0, "rgba(255,226,150,0.6)");
  grad.addColorStop(1, "rgba(255,226,150,0)");
  c.fillStyle = grad;
  c.beginPath(); c.arc(64, 64, 60, 0, Math.PI * 2); c.fill();
  // 光禮本體
  c.font = "64px system-ui, sans-serif";
  c.textAlign = "center"; c.textBaseline = "middle";
  c.fillText("🎁", 64, 70);
  townShareGiftTex = new THREE.CanvasTexture(canvas);
  townShareGiftTex.anisotropy = 4;
  return townShareGiftTex;
}

// 最新一筆快照的居民互助（ROADMAP 369）；null＝此刻無人正在分享。
let latestTownShare = null;
const townShareGroup = new THREE.Group();
scene.add(townShareGroup);
let townShareSprite = null; // 單一可複用的光禮 billboard（懶建）

// 每幀更新光禮飄送（在 npcs 位置/fade 更新後、與 updateFactionLinks 同段呼叫）：
// 吸送禮者→受禮者當下位置內插＋沿弧抬升，透明度＝行程淡入淡出 × 兩端 AOI 淡出較小者。
// 無互助／任一方不在視野／壞值一律安靜收起 sprite，不殘留、不拋（守 render-loop 韌性）。
function updateTownShare(t) {
  if (!latestTownShare || !latestTownShare.giver || !latestTownShare.receiver) {
    if (townShareSprite) townShareSprite.visible = false;
    return;
  }
  const g = npcs.get(latestTownShare.giver), r = npcs.get(latestTownShare.receiver);
  if (!g || !r) { if (townShareSprite) townShareSprite.visible = false; return; } // 兩位都在視野內才畫得出心意流動
  const spec = townShareGiftSpec(latestTownShare.t);
  const fadeA = g.userData.fade ?? 1, fadeB = r.userData.fade ?? 1;
  const alpha = spec.alpha * Math.min(fadeA, fadeB);
  if (!townShareSprite) {
    townShareSprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: townShareGiftTexture(), transparent: true, depthTest: false, opacity: 0 }));
    townShareGroup.add(townShareSprite);
  }
  const f = spec.frac;
  townShareSprite.position.set(
    g.position.x + (r.position.x - g.position.x) * f,
    TOWN_SHARE_BASE_Y + spec.lift,
    g.position.z + (r.position.z - g.position.z) * f,
  );
  const pulse = 1 + Math.sin(t * 5) * 0.1; // 輕脈動（與派系連線同調）
  townShareSprite.scale.set(6 * pulse, 6 * pulse, 1);
  townShareSprite.material.opacity = alpha;
  townShareSprite.visible = alpha > 0.04; // 頭尾幾近淡出時整枚收掉，不殘影
}

// 每幀更新所有玩家頭頂的表情泡泡（ROADMAP 621；在 players 的 updateRemoteEntities 之後呼叫：
// 那裡 updateFade 已把每個子 sprite 的 opacity 設成 AOI 淡入淡出值，這裡再乘上表情自己的存活淡出
// 覆寫上去，故 AOI 淡入淡出仍生效、又不被它壓掉表情的顯示）。過期或玩家已離開即收掉。
function updatePlayerEmotes(nowMs) {
  for (const [fromId, em] of playerEmotes) {
    const vis = emoteBubbleVisual(nowMs - em.startMs, em.displaySecs);
    const g = players.get(fromId);
    if (!vis.visible || !g) {
      // 過期、或該玩家已離開視野/離線 → 隱藏其泡泡 sprite；存活期已過就從 Map 移除
      if (g && g.userData.emoteSprite) g.userData.emoteSprite.visible = false;
      if (!vis.visible) playerEmotes.delete(fromId);
      continue;
    }
    const s = g.userData.emoteSprite;
    if (!s) continue;
    setSpriteEmoji(s, em.glyph);
    s.visible = true;
    const fade = g.userData.fade ?? 1; // AOI 淡入淡出基底
    if (reduceMotion) {
      // 尊重 reduceMotion：固定高度與大小、不彈跳不上浮，只保留柔和淡出
      s.position.y = 16; s.scale.set(7, 7, 1);
    } else {
      s.position.y = 15 + vis.rise;
      const sc = 7 * vis.scale;
      s.scale.set(sc, sc, 1);
    }
    s.material.opacity = fade * vis.opacity;
  }
}

// ---- 程序化火柴人（stickman）----
// 人形＝純幾何組裝：球當頭、膠囊當軀幹、細圓柱當四肢，零美術資產。
// 套用對象＝玩家（自己＋別人）＋ NPC／居民；敵人／野生動物／節點維持盒子等好區分。
//
// 比例（場景單位，可調）。腳底約落在 y=0（站在地面），整體高度與舊膠囊相近。
// 效能改版（perf/3d-fps）：四肢由「大腿+小腿／上臂+前臂」兩節各自一個 mesh，
// 合併成「整條腿／整條臂」單一錐形圓柱。每隻火柴人 mesh 數 10→6（draw call 砍 4 成），
// 走路動畫關節 8→4（每幀 CPU 砍半）；腿臂仍從髖／肩擺動＝走路視覺保留，只是不再彎膝彎肘。
const SK = {
  HIP_Y: 4.6, LEG_LEN: 4.6, HIP_HALF_W: 0.7,          // LEG_LEN＝舊 THIGH_LEN(2.4)+SHIN_LEN(2.2)
  SHOULDER_Y: 7.8, SHOULDER_HALF_W: 1.35, ARM_LEN: 3.9, // ARM_LEN＝舊 UPPER(2.0)+FORE(1.9)
  TORSO_Y: 6.2, HEAD_Y: 9.3, HEAD_R: 1.3,
};

// 共用幾何（全模組只建一次 → 幾十個火柴人也不重建頂點、不爆記憶體）。
// 圓柱預設沿 +Y、以原點為中心；做四肢時讓 mesh 往下沉半截，關節樞紐就落在「上端」。
// 腿／臂各只剩一節：半徑由上端（粗）漸縮到下端（細），保留人形錐度；面數仍取 6（夠看、省頂點）。
const SK_GEO = {
  leg:   new THREE.CylinderGeometry(0.50, 0.34, SK.LEG_LEN, 6), // 整條腿（髖→腳）
  arm:   new THREE.CylinderGeometry(0.42, 0.30, SK.ARM_LEN, 6), // 整條臂（肩→手）
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

  // 腿：單節，樞紐在髖部、整條腿往下掛，rotation.x 前後擺即走路
  function leg(sign) {
    const l = skSegment(SK_GEO.leg, mat, SK.LEG_LEN);
    l.position.set(sign * SK.HIP_HALF_W, SK.HIP_Y, 0);
    g.add(l);
    return l;
  }
  // 臂：單節，樞紐在肩、整條臂往下掛，rotation.x 與同側腿反相擺
  function arm(sign) {
    const a = skSegment(SK_GEO.arm, mat, SK.ARM_LEN);
    a.position.set(sign * SK.SHOULDER_HALF_W, SK.SHOULDER_Y, 0);
    g.add(a);
    return a;
  }
  const legL = leg(1), legR = leg(-1);
  const armL = arm(1), armR = arm(-1);

  g.userData.isStickman = true;
  g.userData.joints = {
    torso, head,
    legL, legR, armL, armR,
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
  // 野生動物：記住身體 group，幼獸體型縮放要套在它身上（與 fade 的 g.scale 相乘）
  if (body && body.userData && body.userData.isCreature) g.userData.creature = body;
  // 敵人：記住身體 group，兇名精英體型放大要套在它身上（與 fade 的 g.scale 相乘，ROADMAP 626）
  if (body && body.userData && body.userData.isEnemy) g.userData.enemyBody = body;
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

// 夜採星晶礦脈（ROADMAP 629／50）：夜間限定、可採集的發光晶簇。比一般礦脈更大更亮（自發光晶藍），
// 讓「夜裡才有的東西」在 3D 裡一眼認得出來——回應居民反覆許願的「夜間值得出門的目標」。
// 白天伺服器不送＝reconcile 自動清掉，故只在夜空下浮現。靜態地形物，走 AOI 淡入淡出。
function makeStarCrystal() {
  const mat = new THREE.MeshLambertMaterial({ color: 0x8fd6ff });
  mat.emissive = new THREE.Color(0x2f5fb0); // 自發光晶藍，夜裡也亮（呼應 makeNode 成熟金果的發光手法）
  mat.emissiveIntensity = 0.7;
  const mesh = new THREE.Mesh(new THREE.OctahedronGeometry(3.6), mat);
  mesh.position.y = 3.8;
  const g = new THREE.Group();
  g.add(mesh);
  g.userData.tx = g.position.x;
  g.userData.tz = g.position.z;
  initNetState(g);
  scene.add(g);
  return g;
}

// ---- 程序化野生動物（ROADMAP 615）----
// 四型低多邊形身形：bird（小鳥）／quadruped（鹿）／critter（圓滾小獸）／predator（狼狐）。
// 共用幾何（全模組只建一次，幾十隻也不重建頂點）；材質「每隻一份」（單色）——這樣 AOI
// 淡入淡出能各自獨立調 opacity，不會牽連同種類的別隻。
const WL_GEO = {
  birdBody:  new THREE.SphereGeometry(1.4, 8, 6),       // 鳥身（小橢球，拉長當身體）
  birdWing:  new THREE.ConeGeometry(0.5, 1.8, 4),       // 翅膀（扁錐）
  birdBeak:  new THREE.ConeGeometry(0.3, 0.9, 4),       // 喙
  quadBody:  new THREE.BoxGeometry(2.0, 1.5, 3.6),      // 四足身軀
  quadLeg:   new THREE.CylinderGeometry(0.28, 0.22, 2.4, 5),
  quadNeck:  new THREE.CylinderGeometry(0.42, 0.5, 1.8, 5),
  quadHead:  new THREE.SphereGeometry(0.8, 8, 6),
  antler:    new THREE.ConeGeometry(0.18, 1.2, 4),      // 鹿角
  ear:       new THREE.ConeGeometry(0.32, 0.9, 4),      // 尖耳（小獸／掠食者）
  critBody:  new THREE.SphereGeometry(1.2, 8, 6),       // 小獸圓身
  tail:      new THREE.ConeGeometry(0.4, 2.0, 5),       // 長尾（掠食者）
};

// 建一隻指定種類的低多邊形動物：回傳一個 group（userData.isCreature／bodyType）。
// 未知種類退回原本的綠盒，安全且向後相容。材質每隻一份（fade 可獨立）。
function makeCreature(kind) {
  const spec = WILDLIFE_SPEC[kind] || WILDLIFE_DEFAULT_SPEC;
  const g = new THREE.Group();
  const mat = new THREE.MeshLambertMaterial({ color: spec.color });
  const add = (geo, x, y, z, rx, ry, rz) => {
    const m = new THREE.Mesh(geo, mat);
    m.position.set(x, y, z);
    if (rx || ry || rz) m.rotation.set(rx || 0, ry || 0, rz || 0);
    g.add(m);
    return m;
  };
  if (spec.type === "bird") {
    const body = add(WL_GEO.birdBody, 0, 2.2, 0); body.scale.set(0.8, 0.8, 1.4);
    add(WL_GEO.birdWing, -0.9, 2.4, -0.2, 0, 0, 0.9);  // 左翼
    add(WL_GEO.birdWing, 0.9, 2.4, -0.2, 0, 0, -0.9);  // 右翼
    add(WL_GEO.birdBeak, 0, 2.3, 1.4, Math.PI / 2, 0, 0); // 喙朝前
  } else if (spec.type === "quadruped") {
    add(WL_GEO.quadBody, 0, 2.6, 0);
    for (const sx2 of [-1, 1]) for (const sz2 of [-1, 1]) add(WL_GEO.quadLeg, sx2 * 0.8, 1.2, sz2 * 1.3); // 四腿
    add(WL_GEO.quadNeck, 0, 3.6, 1.5, Math.PI / 5, 0, 0); // 前傾的脖子
    add(WL_GEO.quadHead, 0, 4.4, 2.3);                     // 頭
    add(WL_GEO.antler, -0.35, 5.2, 2.3, 0, 0, 0.3);        // 鹿角左
    add(WL_GEO.antler, 0.35, 5.2, 2.3, 0, 0, -0.3);        // 鹿角右
  } else if (spec.type === "predator") {
    add(WL_GEO.quadBody, 0, 2.4, 0).scale.set(0.85, 0.8, 1.05); // 較精瘦的身軀
    for (const sx2 of [-1, 1]) for (const sz2 of [-1, 1]) add(WL_GEO.quadLeg, sx2 * 0.7, 1.2, sz2 * 1.2);
    add(WL_GEO.quadHead, 0, 3.2, 1.9).scale.set(0.9, 0.85, 1.1); // 略尖的頭
    add(WL_GEO.ear, -0.4, 3.9, 1.9);  // 尖耳左
    add(WL_GEO.ear, 0.4, 3.9, 1.9);   // 尖耳右
    add(WL_GEO.tail, 0, 2.8, -1.9, -Math.PI / 3, 0, 0); // 翹起的長尾
  } else if (spec.type === "critter") {
    add(WL_GEO.critBody, 0, 1.4, 0);
    add(WL_GEO.ear, -0.5, 2.4, 0.2).scale.set(0.7, 0.7, 0.7); // 小圓耳
    add(WL_GEO.ear, 0.5, 2.4, 0.2).scale.set(0.7, 0.7, 0.7);
    add(WL_GEO.tail, 0, 1.6, -1.0, -Math.PI / 2.4, 0, 0).scale.set(0.7, 0.7, 0.7); // 翹尾
  } else {
    // 未知種類：退回原本的綠盒（向後相容、永不空殼）
    const box = new THREE.Mesh(new THREE.BoxGeometry(3, 3, 5), mat);
    box.position.y = 1.5;
    g.add(box);
  }
  g.userData.isCreature = true;
  g.userData.bodyType = spec.type;
  return g;
}

// 寵物共用幾何（ROADMAP 627；全模組只建一次，幾十隻不重建頂點）。
const PET_GEO = {
  orb:      new THREE.SphereGeometry(0.9, 8, 6),          // 精靈／幽靈圓身
  wing:     new THREE.ConeGeometry(0.45, 1.1, 4),         // 精靈翅膀（扁錐）
  gem:      new THREE.OctahedronGeometry(0.8),            // 晶石（八面體）
  gemSmall: new THREE.OctahedronGeometry(0.42),           // 小晶簇
  shell:    new THREE.SphereGeometry(0.85, 8, 5),         // 蟹殼（壓扁的圓）
  claw:     new THREE.ConeGeometry(0.3, 0.8, 4),          // 蟹螯
  legSm:    new THREE.CylinderGeometry(0.1, 0.08, 0.7, 4),// 蟹腳
  star:     new THREE.OctahedronGeometry(0.7),            // 守護星核
  ring:     new THREE.TorusGeometry(0.95, 0.12, 6, 12),   // 守護光環
};

// 建一隻指定種類的低多邊形寵物：回傳一個 group（userData.isPet／petType／petBody＝身體子群，
// 待機浮動／玩耍蹦跳套在 petBody 上，與 g.position.y 的移動 bob、g.scale 的 fade 互不干擾）。
// 未知種類退回小盒，安全且向後相容。材質每隻一份（fade 可獨立淡入淡出）。
function makePet(kind) {
  const spec = PET_SPEC[kind] || PET_DEFAULT_SPEC;
  const g = new THREE.Group();
  const body = new THREE.Group();
  g.add(body);
  const mat = new THREE.MeshLambertMaterial({ color: spec.color });
  const add = (geo, x, y, z, rx, ry, rz) => {
    const m = new THREE.Mesh(geo, mat);
    m.position.set(x, y, z);
    if (rx || ry || rz) m.rotation.set(rx || 0, ry || 0, rz || 0);
    body.add(m);
    return m;
  };
  if (spec.type === "sprite") {
    add(PET_GEO.orb, 0, 1.5, 0).scale.set(0.8, 0.9, 0.8);   // 小圓身
    add(PET_GEO.wing, -0.7, 1.7, -0.1, 0, 0, 0.9);          // 左翼
    add(PET_GEO.wing, 0.7, 1.7, -0.1, 0, 0, -0.9);          // 右翼
  } else if (spec.type === "crystal") {
    add(PET_GEO.gem, 0, 1.5, 0);                            // 主晶
    add(PET_GEO.gemSmall, -0.6, 1.0, 0.2);                  // 小晶簇
    add(PET_GEO.gemSmall, 0.55, 1.1, -0.2).scale.setScalar(0.8);
  } else if (spec.type === "crab") {
    add(PET_GEO.shell, 0, 1.0, 0).scale.set(1.1, 0.6, 1.0); // 壓扁的蟹殼
    add(PET_GEO.claw, -0.9, 1.0, 0.5, 0, 0, -0.5);          // 左螯
    add(PET_GEO.claw, 0.9, 1.0, 0.5, 0, 0, 0.5);            // 右螯
    for (const sx2 of [-1, 1]) add(PET_GEO.legSm, sx2 * 0.7, 0.5, -0.4, 0, 0, sx2 * 0.6); // 後腳
  } else if (spec.type === "wraith") {
    add(PET_GEO.orb, 0, 1.6, 0).scale.set(0.9, 1.1, 0.9);   // 飄浮圓身
    add(PET_GEO.orb, 0, 0.9, 0).scale.set(0.7, 0.5, 0.7);   // 下襬（幽靈尾）
  } else if (spec.type === "guardian") {
    add(PET_GEO.star, 0, 1.5, 0);                           // 星核
    add(PET_GEO.ring, 0, 1.5, 0, Math.PI / 2.6, 0, 0);      // 斜環
  } else {
    // 未知種類：退回小盒（向後相容、永不空殼）
    const box = new THREE.Mesh(new THREE.BoxGeometry(1.2, 1.2, 1.2), mat);
    box.position.y = 1.0;
    body.add(box);
  }
  g.userData.isPet = true;
  g.userData.petType = spec.type;
  g.userData.petBody = body;
  return g;
}

// 給一隻寵物 group 掛上頭頂狀態 sprite（玩耍 💞／接物 🎾／性格心情泡泡）＋腳邊羈絆愛心條 sprite。
// 比居民精簡：兩層 sprite、無思想泡泡；初始隱形，由 updatePetStatus 每幀依快照決定。
function attachPetStatus(g) {
  const status = makeEmojiSprite(2.6);
  status.position.set(0, 3.6, 0); // 浮在寵物頭頂
  g.add(status);
  const hearts = new THREE.Sprite(new THREE.SpriteMaterial({ transparent: true, depthTest: false, opacity: 0 }));
  hearts.scale.set(3.4, 0.7, 1); // 橫向一排小愛心：寬扁
  hearts.position.set(0, 0.2, 0); // 墊在寵物腳邊
  hearts.visible = false;
  g.add(hearts);
  g.userData.petStatus = status;
  g.userData.petHearts = hearts;
}

// 寵物羈絆愛心條貼圖（鏡像 2D 腳邊一排小愛心；♥/♡ 非單一 emoji，故另用寬畫布渲染，不走 emojiTexture）。
// bond 只有 0..5＝至多 6 種字串，快取無上限之虞。空字串＝不畫。
const petHeartsTexCache = new Map();
function petHeartsTexture(hearts) {
  const key = String(hearts || "");
  let tex = petHeartsTexCache.get(key);
  if (tex) return tex;
  const canvas = document.createElement("canvas");
  canvas.width = 160; canvas.height = 32;
  const ctx = canvas.getContext("2d");
  ctx.font = "26px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  ctx.fillStyle = "rgba(232,120,140,0.95)"; // 暖玫瑰紅，鏡像 2D 羈絆條
  ctx.fillText(key, 80, 18);
  tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  petHeartsTexCache.set(key, tex);
  return tex;
}

// 給一隻野生動物 group 掛上頭頂狀態 sprite（馴養 💛／親近 💗／幼獸 ✨／歇息 💤）。
// 比居民的精簡：只一層 emoji、無思想泡泡；初始隱形，由 updateWildlifeStatus 每幀依快照決定。
function attachWildlifeStatus(g) {
  const status = makeEmojiSprite(3.4);
  status.position.set(0, 7.5, 0); // 浮在動物頭頂之上
  g.add(status);
  g.userData.wlStatus = status;
}

// 每幀更新所有野生動物的呈現（在 updateRemoteEntities(wildlife) 之後呼叫：那裡 updateFade 已把
// 子 sprite 的 opacity 設成 AOI 淡入淡出值，這裡再依快照覆寫狀態 sprite 與幼獸體型）。
function updateWildlifeStatus(t) {
  for (const [, g] of wildlife) {
    const item = g.userData.item;
    const v = wildlifeVisual(item);
    // 幼獸縮一號：套在身體 group（creature）上，與 g.scale（fade）相乘，互不干擾。
    const body = g.userData.creature;
    if (body) {
      const cur = body.userData.shownScale ?? 1;
      const ns = cur + (v.scale - cur) * 0.2; // 平滑趨近，餵食長大不突跳
      body.userData.shownScale = ns;
      body.scale.setScalar(ns);
    }
    // 頭頂狀態 emoji
    const status = g.userData.wlStatus;
    if (!status) continue;
    const fade = g.userData.fade ?? 1;
    const emoji = wildlifeStatusEmoji(item);
    if (setSpriteEmoji(status, emoji)) {
      status.visible = true;
      // 馴養／親近的愛心輕輕脈動（呼應 2D 漸滿的愛心）；其餘穩定顯示。皆尊重 reduceMotion。
      const pulsing = !reduceMotion && (v.tamed || v.familiarity > 0.5);
      const pulse = pulsing ? 0.72 + 0.28 * Math.abs(Math.sin(t * 3)) : 1;
      status.material.opacity = fade * pulse;
    } else {
      status.visible = false;
    }
  }
}

// 每幀更新所有寵物的呈現（ROADMAP 627；在 updateRemoteEntities(pets) 之後呼叫：那裡 updateFade 已把
// 子 sprite/mesh 的 opacity 設成 AOI 淡入淡出值，這裡再依快照覆寫頭頂狀態／腳邊羈絆條，並讓身體
// 待機輕浮、玩耍／接物時蹦得更歡）。皆尊重 reduceMotion。
function updatePetStatus(t) {
  for (const [, g] of pets) {
    const item = g.userData.item;
    const v = petVisual(item);
    const fade = g.userData.fade ?? 1;
    // 身體待機浮動／玩耍蹦跳：套在 petBody 子群（與 g.position.y 的移動 bob、g.scale 的 fade 互不干擾）。
    const body = g.userData.petBody;
    if (body) {
      if (reduceMotion) {
        body.position.y = 0; // 偏好減少動態：身體不浮動（鏡像 2D reduceMotion 關閉彈跳）
      } else {
        const amp = v && v.fetching ? 0.5 : (v && v.playing ? 0.42 : 0.16);  // 接物蹦最高、玩耍次之、待機輕浮
        const rate = v && v.fetching ? 9 : (v && v.playing ? 7.5 : 3.2);     // 接物蹦最急
        if (g.userData.petPhase === undefined) g.userData.petPhase = (g.position.x + g.position.z) % 6.28; // 各自相位、不齊步
        body.position.y = Math.abs(Math.sin(t * rate + g.userData.petPhase)) * amp;
      }
    }
    // 頭頂狀態 emoji（玩耍 💞／接物 🎾／性格心情泡泡）
    const status = g.userData.petStatus;
    if (status) {
      const emoji = petStatusEmoji(item);
      if (setSpriteEmoji(status, emoji)) {
        status.visible = true;
        // 玩耍／黏人愛心輕輕脈動（呼應 2D 玩耍上飄愛心）；其餘穩定顯示。皆尊重 reduceMotion。
        const pulsing = !reduceMotion && v && (v.playing || emoji === "💕");
        const pulse = pulsing ? 0.72 + 0.28 * Math.abs(Math.sin(t * 3)) : 1;
        status.material.opacity = fade * pulse;
      } else {
        status.visible = false;
      }
    }
    // 腳邊羈絆默契愛心條（養出默契才畫；沒默契＝不畫，零干擾）
    const hearts = g.userData.petHearts;
    if (hearts) {
      const str = v ? petBondHearts(v.bond) : "";
      if (str) {
        if (hearts.userData.hearts !== str) { // 只在愛心數變動時換貼圖，省每幀重建
          hearts.userData.hearts = str;
          hearts.material.map = petHeartsTexture(str);
          hearts.material.needsUpdate = true;
        }
        hearts.visible = true;
        hearts.material.opacity = fade * 0.85;
      } else {
        hearts.visible = false;
      }
    }
  }
}

// ---- 程序化敵人身形（ROADMAP 626）----
// 三型低多邊形威脅體：drone（機械稜角＋頂上轉子）／wisp（懸浮八面體幽靈）／golem（龐然晶體巨像）。
// 共用幾何（全模組只建一次，幾十隻也不重建頂點）；材質「每隻一份」（單色）——AOI 淡入淡出能各自
// 獨立調 opacity，不牽連同種類的別隻。
const EN_GEO = {
  droneBody:     new THREE.BoxGeometry(4, 3.4, 4),
  droneCore:     new THREE.OctahedronGeometry(1.3),
  droneRotor:    new THREE.BoxGeometry(6, 0.4, 0.8),
  droneLeg:      new THREE.CylinderGeometry(0.22, 0.22, 2.2, 4),
  wispBody:      new THREE.OctahedronGeometry(2.6),
  wispSpike:     new THREE.ConeGeometry(0.7, 2.6, 4),
  golemCore:     new THREE.DodecahedronGeometry(3.0),
  golemShoulder: new THREE.BoxGeometry(2.2, 2.2, 2.2),
};

// 建一隻指定種類的低多邊形敵人：回傳 group（userData.isEnemy／bodyType）。未知種類退回紅機械盒，
// 安全且向後相容。材質每隻一份（fade 可獨立）。
function makeEnemy(kind) {
  const spec = ENEMY_SPEC[kind] || ENEMY_DEFAULT_SPEC;
  const g = new THREE.Group();
  const mat = new THREE.MeshLambertMaterial({ color: spec.color });
  const add = (geo, x, y, z, rx, ry, rz) => {
    const m = new THREE.Mesh(geo, mat);
    m.position.set(x, y, z);
    if (rx || ry || rz) m.rotation.set(rx || 0, ry || 0, rz || 0);
    g.add(m);
    return m;
  };
  if (spec.type === "wisp") {
    add(EN_GEO.wispBody, 0, 4.4, 0);                 // 懸浮的八面體靈體
    add(EN_GEO.wispSpike, 0, 6.6, 0);                // 頂上尖刺
    add(EN_GEO.wispSpike, 0, 2.2, 0, Math.PI, 0, 0); // 底下倒刺（幽靈尾）
  } else if (spec.type === "golem") {
    add(EN_GEO.golemCore, 0, 4.2, 0);                   // 龐然晶體核心
    add(EN_GEO.golemShoulder, -2.4, 3.0, 0, 0, 0, 0.3); // 左肩塊
    add(EN_GEO.golemShoulder, 2.4, 3.0, 0, 0, 0, -0.3); // 右肩塊
  } else {
    // drone（含未知後備）：機械稜角硬殼＋前方紅核＋頂上轉子＋短腿
    add(EN_GEO.droneBody, 0, 4.0, 0);
    add(EN_GEO.droneCore, 0, 4.0, 2.0).scale.set(0.9, 0.9, 0.9); // 前方核（像個眼）
    add(EN_GEO.droneRotor, 0, 6.0, 0);                           // 頂上轉子
    for (const sx2 of [-1, 1]) add(EN_GEO.droneLeg, sx2 * 1.2, 1.3, 0); // 短腿
  }
  g.userData.isEnemy = true;
  g.userData.bodyType = spec.type;
  return g;
}

// 血條／狀態的視覺常數（ROADMAP 626）。
const _enemyBarGeo = new THREE.BoxGeometry(1, 0.5, 0.18); // 血條（單位寬，用 scale.x 調長）
const ENEMY_BAR_BG_COLOR = 0x20242a;  // 血條底槽（深灰）
const ENEMY_BAR_HP_COLOR = 0xff6a5a;  // 血條（紅）
const ENEMY_BAR_LOW_COLOR = 0x8a1f1f; // 殘血（深紅，≤30%＝快倒了）
const ENEMY_BAR_W = 6;   // 血條全長（場景單位）
const ENEMY_BAR_Y = 8.6; // 血條浮在敵人頭頂之上

// 給一隻敵人 group 掛上頭頂血條（底槽＋填充）＋狀態 emoji sprite。初始隱形，由 updateEnemyStatus
// 每幀依快照決定。填充預建兩份材質（常態紅／殘血深紅），每幀換 `fill.material` 參照切色——
// 不在每幀 mutate 材質顏色（既省、又對測試替身的 MeshBasicMaterial 安全）。
function attachEnemyStatus(g) {
  const bg = new THREE.Mesh(_enemyBarGeo, new THREE.MeshBasicMaterial({ color: ENEMY_BAR_BG_COLOR, transparent: true, opacity: 0 }));
  bg.position.set(0, ENEMY_BAR_Y, 0);
  bg.scale.set(ENEMY_BAR_W, 1, 1);
  bg.visible = false;
  const hpMat = new THREE.MeshBasicMaterial({ color: ENEMY_BAR_HP_COLOR, transparent: true, opacity: 0 });
  const lowMat = new THREE.MeshBasicMaterial({ color: ENEMY_BAR_LOW_COLOR, transparent: true, opacity: 0 });
  const fill = new THREE.Mesh(_enemyBarGeo, hpMat);
  fill.position.set(0, ENEMY_BAR_Y + 0.04, 0);
  fill.visible = false;
  g.add(bg);
  g.add(fill);
  g.userData.enHpBg = bg;
  g.userData.enHpFill = fill;
  g.userData.enHpMat = hpMat;
  g.userData.enHpLowMat = lowMat;
  const status = makeEmojiSprite(3.4);
  status.position.set(0, ENEMY_BAR_Y + 2.2, 0); // 浮在血條之上
  g.add(status);
  g.userData.enStatus = status;
}

// 每幀更新所有敵人的呈現（在 updateRemoteEntities(enemies) 之後呼叫：updateFade 已把子 mesh opacity
// 設成 AOI 淡入淡出值＝fade，這裡再依快照覆寫兇名體型／血條／狀態 emoji）。
// 守 [[render-loop-resilience]]：壞值全程安全、永不 throw。
function updateEnemyStatus(t) {
  for (const [, g] of enemies) {
    const item = g.userData.item;
    const v = enemyVisual(item);
    const fade = g.userData.fade ?? 1;
    // 兇名精英體型微大：套在身體 group（enemyBody）上，與 g.scale（fade）相乘，平滑趨近不突跳。
    const body = g.userData.enemyBody;
    if (body) {
      const cur = body.userData.shownScale ?? 1;
      const ns = cur + (v.scale - cur) * 0.2;
      body.userData.shownScale = ns;
      body.scale.setScalar(ns);
    }
    // 血條：只在受過傷（damaged）時顯示；填充寬隨剩餘血量、殘血(≤30%)換深紅材質。
    const bg = g.userData.enHpBg, fill = g.userData.enHpFill;
    if (bg && fill) {
      if (v.damaged) {
        const fillSpec = enemyHpFill(item && item.hp, item && item.max_hp);
        bg.visible = true;
        bg.material.opacity = fade * 0.55;
        const fw = ENEMY_BAR_W * fillSpec.ratio;
        if (fw > 0.001) {
          fill.visible = true;
          fill.material = fillSpec.critical ? g.userData.enHpLowMat : g.userData.enHpMat;
          fill.material.opacity = fade;
          // 左對齊：box 中心預設在原點，向左挪半條寬、再加回填充的一半。
          fill.position.x = -ENEMY_BAR_W / 2 + fw / 2;
          fill.scale.x = fw;
        } else {
          fill.visible = false; // 血量歸 0（即將消失）：不畫填充
        }
      } else {
        bg.visible = false;
        fill.visible = false;
      }
    }
    // 頭頂狀態 emoji（破綻 ✨／潰逃 💨／夜歇 💤／兇名 💢）。
    const status = g.userData.enStatus;
    if (!status) continue;
    const emoji = enemyStatusEmoji(item);
    if (setSpriteEmoji(status, emoji)) {
      status.visible = true;
      // 破綻 ✨「現在砍！」輕輕脈動提示時機；其餘穩定顯示。尊重 reduceMotion。
      const pulsing = !reduceMotion && v.weak;
      const pulse = pulsing ? 0.7 + 0.3 * Math.abs(Math.sin(t * 6)) : 1;
      status.material.opacity = fade * pulse;
    } else {
      status.visible = false;
    }
  }
}

// ============================================================
// 農地在 3D 裡長出來（ROADMAP 614）：把快照裡早就有、2D 一直看得到、3D 卻整個忽略的
// `fields`（每位玩家的耕地：翻好的土＋一格格成長中的作物＋稻草人）在 3D 呈現出來，
// 讓「這個世界裡有人在種田」一眼看得見——翻好的土褐色一畦、種子冒土點、發芽抽綠莖、
// 成熟結金果發光、缺水的格子透藍、立了稻草人守望。
// 純讀快照、零後端改動、零協議改動——資料本來就在 FieldView/TileView 裡（2D game.js 早在用）。
// ============================================================

// 作物三階段的視覺規格（鏡像 2D game.js 的程式繪製 fallback 配色，留 i18n／美術一致）：
// state 2=種子（土裡的小點，深棕褐）3=發芽（鮮綠抽莖）4=成熟（亮金發光果）。
// 高度／顏色刻意拉開明度＋色相，跟 2D 的 stage pips 一樣讓三階段一眼分得開。
const CROP_STAGE = {
  2: { h: 0.5, color: 0xa8662e, glow: false }, // 種子：壓暗的棕，貼著土
  3: { h: 1.4, color: 0x5ad94f, glow: false }, // 發芽：鮮綠抽高
  4: { h: 2.0, color: 0xffd24a, glow: true },  // 成熟：亮金、會發光
};
const FIELD_SOIL_COLOR = 0x5b4636;      // 翻好的潮土（深咖啡，鏡像 2D state>=1 底色）
const FIELD_SOIL_OWN_COLOR = 0x6b513c;  // 自己的地：暖一階，跟別人的地一眼分得開
const FIELD_NATURAL_COLOR = 0x7a5f3c;   // 未翻的自然地（暖土黃，鏡像 2D state 0 底色）
const CROP_DRY_COLOR = 0x5aaaff;        // 缺水：藍色提示（鏡像 2D 缺水藍虛線框）
const SCARECROW_COLOR = 0xc9a24b;       // 稻草人：稻草棕黃
const CROP_BAR_BG_COLOR = 0x20242a;     // 熟成進度條底槽（深灰，ROADMAP 624）
const CROP_BAR_GROW_COLOR = 0x66dd55;   // 成長中填充：青綠（鏡像 2D "grow"）
const CROP_BAR_SOON_COLOR = 0xffc24a;   // ≥80% 填充：暖金「就快可收」（鏡像 2D "soon"）

// 單格耕地的視覺：把一筆 TileView 算成「這格該怎麼畫」。純函式、確定性、壞值安全。
// 回傳 null＝這格不長作物（自然地／空土／壞值）——呼叫端就不替它生作物 mesh。
// 只讀權威 `state`/`dry`，不嵌任何種田規則（能不能種是伺服器的事，前端純呈現）。
function cropCellVisual(cell) {
  if (!cell || typeof cell !== "object") return null;
  const meta = CROP_STAGE[cell.state]; // state 0/1/未知 → undefined → 不長作物
  if (!meta) return null;
  // 熟成進度（ROADMAP 624，讀後端權威 `grow` 0~100，ROADMAP 421）：成長中（2/3）才有意義，
  // 讓離散三階段之間也看得出「離收成還差多久」；成熟（4）視為滿、壞值保守歸 0。
  const rawGrow = Number(cell.grow);
  const grow = cell.state === 4 ? 100 : (Number.isFinite(rawGrow) ? Math.max(0, Math.min(100, rawGrow)) : 0);
  return { state: cell.state, h: meta.h, color: meta.color, glow: meta.glow, dry: cell.dry === true, grow };
}

// 熟成進度條的填充規格（ROADMAP 624，鏡像 2D `cropBarFillKind`／公田 421）：純函式、確定性、
// 壞值安全。回傳填充比例 ratio∈[0,1] 與是否「就快可收」soon（≥80% 轉暖金，給接近收成的期待感）。
function cropBarFill(grow) {
  const v = Number(grow);
  const g = Number.isFinite(v) ? Math.max(0, Math.min(100, v)) : 0;
  return { ratio: g / 100, soon: g >= 80 };
}

// 視野內農地的 HUD 標籤：幾塊地、幾株作物待收成（state 4＝成熟）、幾株就快熟（成長中且
// grow≥80，ROADMAP 624 給「接近目標」的動能）。純函式、壞值安全。
// 面向玩家字串集中前端、glyph 留 i18n 空間（後端只送穩定數值 state/grow，文案由前端對照）。
function farmHudLabel(fieldList) {
  if (!Array.isArray(fieldList) || fieldList.length === 0) return "";
  let ripe = 0, soon = 0;
  for (const f of fieldList) {
    const cells = f && Array.isArray(f.cells) ? f.cells : [];
    for (const c of cells) {
      if (!c) continue;
      if (c.state === 4) ripe++;
      else if ((c.state === 2 || c.state === 3) && Number(c.grow) >= 80) soon++;
    }
  }
  let s = `🌾 農地 ${fieldList.length}`;
  if (ripe > 0) s += " · " + ripe + " 株待收";
  if (soon > 0) s += " · " + soon + " 株將熟";
  return s;
}

// 一塊田的「視覺指紋」：把每格的 state/dry 串成字串＋稻草人位置。只有指紋變了才重建作物
// mesh（多數幀作物沒變、不必每幀重生 mesh＝近乎零增量開銷）。壞值安全（缺 cells → 空指紋）。
function fieldDigest(field) {
  if (!field || typeof field !== "object") return "x";
  let d = "";
  const cells = Array.isArray(field.cells) ? field.cells : [];
  for (const c of cells) {
    const st = c && typeof c.state === "number" ? c.state : 0;
    // 成長中（2/3）才把熟成進度納入指紋，量化成每 10% 一檔——進度條跨檔才重建作物層，
    // 不必每長 1% 都重生 mesh（ROADMAP 624）。其餘狀態的 grow 無意義，不影響指紋。
    const gq = (st === 2 || st === 3) && Number.isFinite(Number(c.grow))
      ? Math.floor(Math.max(0, Math.min(100, Number(c.grow))) / 10) : "";
    d += st + (c && c.dry ? "w" : "") + (gq === "" ? "" : "g" + gq) + ",";
  }
  const sc = field.scarecrow;
  if (Array.isArray(sc) && sc.length === 2) d += "sc" + sc[0] + "_" + sc[1];
  return d;
}

// 場景單位下的每格邊長（壞值退回預設 48px×scale）。
function fieldTileScene(field) {
  const ts = field && Number.isFinite(field.tile_size) && field.tile_size > 0 ? field.tile_size : 48;
  return ts * WORLD_SCALE;
}

// 共用幾何體（所有田、所有格共用一份，省記憶體；顏色由各自材質帶）。
const _cropSeedGeo = new THREE.SphereGeometry(0.5, 6, 4);    // 種子：小球貼土
const _cropSproutGeo = new THREE.ConeGeometry(0.45, 1, 5);   // 發芽：細綠錐
const _cropMatureGeo = new THREE.ConeGeometry(0.6, 1, 6);    // 成熟：飽滿金錐
const _scarePoleGeo = new THREE.CylinderGeometry(0.12, 0.12, 2.2, 5);
const _scareArmGeo = new THREE.BoxGeometry(1.6, 0.18, 0.18);
const _cropBarGeo = new THREE.BoxGeometry(1, 0.14, 0.42);    // 熟成進度條（單位寬，用 scale.x 調長）

// 替一塊田（FieldView）建/重建它的作物層：清掉舊作物群、依最新各格 state 長出新作物。
// 土底（base plane）只建一次、之後重用；只有作物層隨 digest 變更重建。
function rebuildFieldCrops(g, field) {
  // 移除上一批作物 mesh（含稻草人），釋放它們的材質（共用幾何體不釋放）。
  const old = g.userData.cropLayer;
  if (old) {
    old.traverse((o) => { if (o.material && o.material.dispose) o.material.dispose(); });
    g.remove(old);
  }
  const layer = new THREE.Group();
  g.userData.cropLayer = layer;
  g.add(layer);

  const cols = Number.isFinite(field.cols) && field.cols > 0 ? Math.min(field.cols, 64) : 0;
  const rows = Number.isFinite(field.rows) && field.rows > 0 ? Math.min(field.rows, 64) : 0;
  const tileS = fieldTileScene(field);
  const cells = Array.isArray(field.cells) ? field.cells : [];
  // 格子在群組本地座標的偏移（群組原點＝田中心，故減去半幅置中）。
  const offOf = (col, row) => ({
    x: (col + 0.5 - cols / 2) * tileS,
    z: (row + 0.5 - rows / 2) * tileS,
  });

  for (let row = 0; row < rows; row++) {
    for (let col = 0; col < cols; col++) {
      const cell = cells[row * cols + col];
      const vis = cropCellVisual(cell);
      if (!vis) continue;
      const off = offOf(col, row);
      let mesh;
      if (vis.state === 2) {
        mesh = new THREE.Mesh(_cropSeedGeo, new THREE.MeshLambertMaterial({ color: vis.color }));
        mesh.position.set(off.x, vis.h, off.z);
      } else {
        const geo = vis.state === 4 ? _cropMatureGeo : _cropSproutGeo;
        const mat = new THREE.MeshLambertMaterial({ color: vis.color });
        if (vis.glow) { mat.emissive = new THREE.Color(0xffe9a0); mat.emissiveIntensity = 0.5; } // 成熟金果發光
        mesh = new THREE.Mesh(geo, mat);
        mesh.position.set(off.x, vis.h, off.z);
        mesh.scale.y = vis.h; // 錐高隨階段拉伸（幾何體預設高 1）
      }
      mesh.userData.cropState = vis.state;
      mesh.userData.glow = vis.glow;
      mesh.userData.sway = vis.state >= 3; // 發芽／成熟會隨風輕搖
      layer.add(mesh);
      // 缺水：在這格上方插一根藍針提示「該澆水了」（鏡像 2D 缺水藍框）。
      if (vis.dry) {
        const dryMark = new THREE.Mesh(_cropSproutGeo, new THREE.MeshBasicMaterial({ color: CROP_DRY_COLOR, transparent: true, opacity: 0.75 }));
        dryMark.position.set(off.x, vis.h + 0.9, off.z);
        dryMark.scale.set(0.4, 0.6, 0.4);
        layer.add(dryMark);
      }
      // 熟成進度條（ROADMAP 624，回應 AI 居民反覆許願「作物週期進度條，讓玩家看到接近目標」）：
      // 成長中（種子/發芽）的作物頭頂浮一條躺平的進度條——底槽＋隨 grow 填充的條，讓離散三階段
      // 之間也看得出「離收成還差多久」。成熟（state 4）不畫（已是發光金果）。鏡像 2D drawCropSlotBar。
      if (vis.state === 2 || vis.state === 3) {
        const fill = cropBarFill(vis.grow);
        const barW = tileS * 0.66;
        const barY = vis.h + (vis.state === 2 ? 0.55 : 0.9); // 浮在作物上方一點
        const bg = new THREE.Mesh(_cropBarGeo, new THREE.MeshBasicMaterial({ color: CROP_BAR_BG_COLOR, transparent: true, opacity: 0.55 }));
        bg.position.set(off.x, barY, off.z);
        bg.scale.set(barW, 1, 1);
        layer.add(bg);
        if (fill.ratio > 0) {
          const fw = barW * fill.ratio;
          const fm = new THREE.Mesh(_cropBarGeo, new THREE.MeshBasicMaterial({ color: fill.soon ? CROP_BAR_SOON_COLOR : CROP_BAR_GROW_COLOR }));
          // 左對齊：box 中心預設在原點，向左挪半條寬、再加回填充的一半。
          fm.position.set(off.x - barW / 2 + fw / 2, barY + 0.04, off.z);
          fm.scale.set(fw, 1, 1);
          layer.add(fm);
        }
      }
    }
  }

  // 稻草人守望（ROADMAP 476）：在指定格立一座十字稻草人，讓「這塊地有人看著」一眼看得見。
  const sc = field.scarecrow;
  if (Array.isArray(sc) && sc.length === 2 && Number.isFinite(sc[0]) && Number.isFinite(sc[1])) {
    const off = offOf(sc[0], sc[1]);
    const crow = new THREE.Group();
    const mat = new THREE.MeshLambertMaterial({ color: SCARECROW_COLOR });
    const pole = new THREE.Mesh(_scarePoleGeo, mat);
    pole.position.y = 1.1;
    const arms = new THREE.Mesh(_scareArmGeo, mat);
    arms.position.y = 1.5;
    crow.add(pole); crow.add(arms);
    crow.position.set(off.x, 0, off.z);
    layer.add(crow);
  }
}

// 替一塊田建立 group：一塊翻好的土底（單一平面，省 draw call）＋作物層。
// 田不移動（per-player 固定地塊），故不做內插/轉身；走 AOI 淡入淡出（離開視野柔和消失）。
function makeFieldPlot(field) {
  const g = new THREE.Group();
  const cols = Number.isFinite(field.cols) && field.cols > 0 ? field.cols : 1;
  const rows = Number.isFinite(field.rows) && field.rows > 0 ? field.rows : 1;
  const tileS = fieldTileScene(field);
  const own = field.owner && myId && field.owner === myId;
  // 土底：整塊田一張平面（翻好的潮土；自己的地暖一階）。鋪在地面之上、作物之下。
  const base = new THREE.Mesh(
    new THREE.PlaneGeometry(cols * tileS, rows * tileS),
    new THREE.MeshLambertMaterial({ color: own ? FIELD_SOIL_OWN_COLOR : FIELD_SOIL_COLOR })
  );
  base.rotation.x = -Math.PI / 2;
  base.position.y = -0.4; // 略高於草地（地面在 -0.45/-0.5），讓田畦浮出來
  g.add(base);
  g.userData.base = base;
  g.userData.isOwn = own;
  rebuildFieldCrops(g, field);
  g.userData.digest = fieldDigest(field);
  initNetState(g); // 田也走 AOI 淡入淡出
  scene.add(g);
  return g;
}

// 田的 reconcile：以 owner 當 key（per-player 一塊地）。位置不動（固定地塊），故不進
// updateRemoteEntities（那是給會走動的實體內插用）；只在 digest 變更時重建作物層。
const fields = new Map();
function reconcileFieldsLegacy(list, recvT) {
  const seen = new Set();
  if (Array.isArray(list)) {
    for (const field of list) {
      try {
        if (!field || typeof field !== "object") continue;
        if (!Number.isFinite(field.origin_x) || !Number.isFinite(field.origin_y)) continue;
        const key = String(field.owner || (field.origin_x + "_" + field.origin_y));
        seen.add(key);
        let g = fields.get(key);
        const cols = Number.isFinite(field.cols) && field.cols > 0 ? field.cols : 1;
        const rows = Number.isFinite(field.rows) && field.rows > 0 ? field.rows : 1;
        const tileS = fieldTileScene(field);
        // 田中心的場景座標（群組原點落在田中心）。
        const cx = sx(field.origin_x + (cols * field.tile_size) / 2);
        const cz = sz(field.origin_y + (rows * field.tile_size) / 2);
        if (!g) {
          g = makeFieldPlot(field);
          g.position.set(cx, 0, cz);
          g.userData.fade = 0; g.userData.fadeTarget = 1; g.userData.removing = false;
          // 關鍵修補（perf/3d-fps）：把新建的田存進 fields 映射！漏了這行 → 每份快照都
          // fields.get(key) 落空、又 makeFieldPlot 重建一份加進場景，舊的永不去重也永不移除
          // （下方「離開 AOI 淡出」迴圈跑的是空 map），田畦 mesh 無限堆積＝幾百個殭屍 draw call，
          // 正是真 QA 抓到的走路掉幀元兇。補上後：同一塊地重用、離場淡出移除、距離 LOD 才生效。
          fields.set(key, g);
        } else {
          g.position.set(cx, 0, cz); // origin 理論上不變，仍每次對齊（防擴地等改動）
          if (g.userData.removing) { g.userData.removing = false; g.userData.fadeTarget = 1; }
          const dg = fieldDigest(field);
          if (dg !== g.userData.digest) { rebuildFieldCrops(g, field); g.userData.digest = dg; }
        }
      } catch (e) {
        console.warn("reconcileFields 單筆失敗，已略過", e);
      }
    }
  }
  // 沒在這份快照出現的田 → 淡出移除（AOI 邊緣不啪一下消失）。
  for (const [key, g] of fields) {
    if (!seen.has(key) && !g.userData.removing) { g.userData.removing = true; g.userData.fadeTarget = 0; }
  }
}

// 作物細節距離 LOD（perf/3d-fps）：每塊田的作物層是「每格一顆錐／一條進度條」的一堆獨立 mesh，
// 一塊地就可能幾十個 draw call。遠到看不清單株時，整個作物層 visible=false（那堆 mesh 不畫、
// 也省每幀逐株 sway/glow 的 CPU），只留翻好的土底平面 → 田畦仍在＝「這裡有人耕種」的視覺保留；
// 近處照舊每株可見＋隨風搖。用遲滯帶（IN/OUT）避免剛好站在界線上時反覆開關。土底本身不收（便宜、
// 又是田的識別），離開 AOI 仍走既有淡出移除。
const CROP_LOD_IN = 66, CROP_LOD_OUT = 74;        // 作物細節：距鏡頭 <IN 展開、>OUT 收起（場景單位）
const CROP_LOD_IN2 = CROP_LOD_IN * CROP_LOD_IN, CROP_LOD_OUT2 = CROP_LOD_OUT * CROP_LOD_OUT;

// 每幀更新所有田：作物細節距離 LOD＋作物隨風輕搖、成熟金果發光脈動、AOI 淡入淡出。皆尊重 reduceMotion。
// （legacy：每格一 mesh 路徑，僅在不支援 InstancedMesh 的環境＝render-smoke 假 THREE 走到。）
function updateFieldsLegacy(dt, t) {
  for (const [key, g] of fields) {
    const layer = g.userData.cropLayer;
    if (layer) {
      // 與鏡頭的水平距離平方（田不動，只看 XZ）。無 camera（極早期）時當作很近＝照常展開。
      let near = true;
      if (camera) {
        const dx = g.position.x - camera.position.x;
        const dz = g.position.z - camera.position.z;
        const d2 = dx * dx + dz * dz;
        let hide = g.userData.lodCropsHidden || false; // 遲滯：保留上次狀態，只在跨過外/內界才翻轉
        if (d2 > CROP_LOD_OUT2) hide = true;
        else if (d2 < CROP_LOD_IN2) hide = false;
        g.userData.lodCropsHidden = hide;
        near = !hide;
      }
      if (layer.visible !== near) layer.visible = near; // 跨界才改，省無謂寫入
      if (near && !reduceMotion) {
        for (const m of layer.children) {
          if (m.userData && m.userData.sway) {
            // 輕微搖擺：用世界位置當相位，整片田不會同手同腳。
            m.rotation.z = Math.sin(t * 1.6 + (g.position.x + m.position.x) * 0.5) * 0.12;
          }
          if (m.userData && m.userData.glow && m.material) {
            m.material.emissiveIntensity = 0.4 + 0.2 * (0.5 + 0.5 * Math.sin(t * 2.2 + g.position.z));
          }
        }
      }
    }
    if (updateFade(g, dt)) { scene.remove(g); fields.delete(key); }
  }
}

// ============================================================
// 作物實例化（perf/3d-crop-instancing，修 #614 走路 FPS 懸崖）
// ============================================================
// 病灶——#614 把「每一格作物」做成獨立 THREE.Mesh＋獨立 MeshLambertMaterial，再加 #624 進度條
// （每株成長中作物 +2 mesh）、枯萎藍針、稻草人……一塊地幾十顆、滿世界幾百顆獨立場景物件。
// Three.js 每幀都要遍歷所有物件做視錐剔除＋送出幾百個 draw call → 既 draw-call-bound 又
// CPU-bound（即使作物沒變、每幀 JS 照漲）。節流真 QA 量到：走路 ~350 draw call、JS/幀 ~34ms、
// throttle 3x 下走路 FPS 卡在 ~15。
//
// 對策——把所有田、所有格的同類東西，合進「每類一個 THREE.InstancedMesh」：種子/發芽/成熟/
// 枯萎針/進度條底槽/進度條填充/稻草人桿/稻草人臂/土底，共 9 個 InstancedMesh。draw call 從幾百
// 崩成 9、場景圖物件剩個位數、每幀遍歷成本近乎歸零。顏色固定的階段共用一份材質；需要逐顆變色的
// （土底「自己的地」暖色、進度條 grow/soon）用 per-instance setColorAt；變換用 setMatrixAt。
//
// 「資料變才更新 buffer」：作物每快照才會變（生長/收成/種植/澆水）。維護 cropsDirty 旗標——只有
// 田增減、digest 變（fieldDigest 量化過、跨檔才算變）、距離 LOD 翻轉、或 AOI 淡入淡出進行中時，
// 才在該幀重建一次 instance buffer；其餘幀完全不碰 buffer（零增量開銷）。絕不每幀重建整批。
//
// 取捨——作物隨風輕搖（sway）原是「每幀逐顆改 rotation」＝在實例化下等於每幀重寫整批矩陣 buffer，
// 正是要消滅的 CPU 成本，故捨去（reduceMotion 本來就關它）。成熟金果發光改為「整批共用材質的
// emissiveIntensity 每幀脈動一次」（單一 uniform 更新，極省），視覺保留。階段生長、顏色、枯萎針、
// 進度條、收成/種植即時更新全數保留。距離 LOD（遠田只留土底）與 AOI 淡入淡出（以縮放烘進矩陣）亦保留。
//
// 相容——render-smoke-3d 的假 THREE 無 InstancedMesh/Object3D → FIELD_INSTANCING=false，自動退回
// 上方 *Legacy 路徑（邏輯原封不動），閘門測試照綠；真瀏覽器走實例化路徑拿效能。
// ============================================================
const FIELD_INSTANCING = typeof THREE.InstancedMesh === "function" && typeof THREE.Object3D === "function";

let _fieldSoilGeo = null;    // 土底單位平面（攤平到 XZ），延後到 init 建（假 THREE 的 geo stub 無 .rotateX）
let cropBatches = null;      // { seed, sprout, mature, dry, barbg, barfill, soil, pole, arm } 九個 InstancedMesh
let _instDummy = null;       // 組合 per-instance 矩陣用的暫存 Object3D（重複使用，不每顆 new）
let _instColor = null;       // setColorAt 用的暫存 Color
let _matureMat = null;       // 成熟金果共用材質（每幀脈動 emissiveIntensity）
let cropsDirty = false;      // 只有資料/可見性變了才在該幀重建 instance buffer

// 各 instance 批次初始容量（不夠會在重建前自動倍增）；土/稻草人少、作物與進度條較多。
const INST_CAP0 = { seed: 128, sprout: 256, mature: 128, dry: 64, barbg: 256, barfill: 256, soil: 32, pole: 16, arm: 16 };

// 建一個 InstancedMesh 批次：實例散佈全世界→整批 bounding sphere 失準（Three 預設用單一幾何體
// 的 bound 算剔除，會誤剔），故關視錐剔除——反正只 9 批，永遠提交也便宜。
function makeInstBatch(geo, mat, cap, withColor) {
  const m = new THREE.InstancedMesh(geo, mat, cap);
  m.frustumCulled = false;
  m.count = 0;
  try { if (THREE.DynamicDrawUsage && m.instanceMatrix && m.instanceMatrix.setUsage) m.instanceMatrix.setUsage(THREE.DynamicDrawUsage); } catch (e) { /* 無此 API 無妨 */ }
  m.userData.cap = cap;
  m.userData.withColor = !!withColor;
  scene.add(m);
  return m;
}

// 首次需要時建好 9 個批次與共用材質/幾何（只在支援 InstancedMesh 的真瀏覽器走到）。
function initCropInstances() {
  if (cropBatches) return;
  _instDummy = new THREE.Object3D();
  _instColor = new THREE.Color();
  _fieldSoilGeo = new THREE.PlaneGeometry(1, 1);
  _fieldSoilGeo.rotateX(-Math.PI / 2); // 攤平到地面（XZ），per-instance 矩陣縮放成各田尺寸

  _matureMat = new THREE.MeshLambertMaterial({ color: CROP_STAGE[4].color });
  _matureMat.emissive = new THREE.Color(0xffe9a0); _matureMat.emissiveIntensity = 0.5; // 成熟金果發光（鏡像 legacy）
  const seedMat = new THREE.MeshLambertMaterial({ color: CROP_STAGE[2].color });
  const sproutMat = new THREE.MeshLambertMaterial({ color: CROP_STAGE[3].color });
  const dryMat = new THREE.MeshBasicMaterial({ color: CROP_DRY_COLOR, transparent: true, opacity: 0.75 });
  const barBgMat = new THREE.MeshBasicMaterial({ color: CROP_BAR_BG_COLOR, transparent: true, opacity: 0.55 });
  const barFillMat = new THREE.MeshBasicMaterial({ color: 0xffffff }); // 底色白，grow/soon 由 per-instance 色相乘
  const soilMat = new THREE.MeshLambertMaterial({ color: 0xffffff });  // 「自己的地」暖色差由 per-instance 色帶
  const scareMat = new THREE.MeshLambertMaterial({ color: SCARECROW_COLOR }); // 稻草人桿/臂共用一份材質

  cropBatches = {
    seed: makeInstBatch(_cropSeedGeo, seedMat, INST_CAP0.seed, false),
    sprout: makeInstBatch(_cropSproutGeo, sproutMat, INST_CAP0.sprout, false),
    mature: makeInstBatch(_cropMatureGeo, _matureMat, INST_CAP0.mature, false),
    dry: makeInstBatch(_cropSproutGeo, dryMat, INST_CAP0.dry, false),
    barbg: makeInstBatch(_cropBarGeo, barBgMat, INST_CAP0.barbg, false),
    barfill: makeInstBatch(_cropBarGeo, barFillMat, INST_CAP0.barfill, true),
    soil: makeInstBatch(_fieldSoilGeo, soilMat, INST_CAP0.soil, true),
    pole: makeInstBatch(_scarePoleGeo, scareMat, INST_CAP0.pole, false),
    arm: makeInstBatch(_scareArmGeo, scareMat, INST_CAP0.arm, false),
  };
}

// 容量不夠就倍增重建該批次（保留 geo/material，只換更大的 instance buffer）；很罕見（容量是上限）。
function ensureCropCap(kind, need) {
  const b = cropBatches[kind];
  if (need <= b.userData.cap) return;
  let cap = b.userData.cap;
  while (cap < need) cap *= 2;
  scene.remove(b);
  try { b.dispose && b.dispose(); } catch (e) { /* 無妨 */ }
  cropBatches[kind] = makeInstBatch(b.geometry, b.material, cap, b.userData.withColor);
}

// 走訪一塊田該長出的所有 instance（土底＋各格作物/枯萎針/進度條＋稻草人），對每顆呼叫 cb(kind,x,y,z,sx,sy,sz,color?)。
// 計數與填充兩階段共用同一支走訪，杜絕兩邊邏輯漂移。fade 以縮放烘進矩陣（鏡像 legacy updateFade 的
// g.scale.setScalar(0.55+0.45*fade)＝淡入長大、淡出縮回）；不做 per-instance 透明（實例共用材質，改縮放等價且省）。
function forEachFieldInstance(r, cb) {
  const sc = 0.55 + 0.45 * (r.fade != null ? r.fade : 1);
  cb("soil", r.cx, -0.4 * sc, r.cz, r.cols * r.tileS * sc, sc, r.rows * r.tileS * sc, r.own ? FIELD_SOIL_OWN_COLOR : FIELD_SOIL_COLOR);
  if (r.lodHidden) return; // 距離 LOD：遠到看不清單株，只留土底（鏡像 legacy 作物層 visible=false）
  const cols = r.cols, rows = r.rows, tileS = r.tileS, cells = r.cells;
  for (let row = 0; row < rows; row++) {
    for (let col = 0; col < cols; col++) {
      const vis = cropCellVisual(cells[row * cols + col]);
      if (!vis) continue;
      const lx = (col + 0.5 - cols / 2) * tileS, lz = (row + 0.5 - rows / 2) * tileS;
      const wx = r.cx + lx * sc, wz = r.cz + lz * sc;
      if (vis.state === 2) cb("seed", wx, vis.h * sc, wz, sc, sc, sc);                  // 種子：小球
      else if (vis.state === 4) cb("mature", wx, vis.h * sc, wz, sc, vis.h * sc, sc);   // 成熟：飽滿金錐（高隨階段）
      else cb("sprout", wx, vis.h * sc, wz, sc, vis.h * sc, sc);                        // 發芽：細綠錐（高隨階段）
      if (vis.dry) cb("dry", wx, (vis.h + 0.9) * sc, wz, 0.4 * sc, 0.6 * sc, 0.4 * sc); // 缺水藍針
      if (vis.state === 2 || vis.state === 3) {                                          // 熟成進度條（ROADMAP 624）
        const barW = tileS * 0.66, barY = vis.h + (vis.state === 2 ? 0.55 : 0.9);
        cb("barbg", wx, barY * sc, wz, barW * sc, sc, sc);
        const fill = cropBarFill(vis.grow);
        if (fill.ratio > 0) {
          const fw = barW * fill.ratio; // 左對齊：box 中心向左挪半條寬、再加回填充半寬
          cb("barfill", r.cx + (lx - barW / 2 + fw / 2) * sc, (barY + 0.04) * sc, wz, fw * sc, sc, sc, fill.soon ? CROP_BAR_SOON_COLOR : CROP_BAR_GROW_COLOR);
        }
      }
    }
  }
  const s = r.scarecrow; // 稻草人守望（ROADMAP 476）：十字＝桿＋臂
  if (Array.isArray(s) && s.length === 2 && Number.isFinite(s[0]) && Number.isFinite(s[1])) {
    const lx = (s[0] + 0.5 - cols / 2) * tileS, lz = (s[1] + 0.5 - rows / 2) * tileS;
    cb("pole", r.cx + lx * sc, 1.1 * sc, r.cz + lz * sc, sc, sc, sc);
    cb("arm", r.cx + lx * sc, 1.5 * sc, r.cz + lz * sc, sc, sc, sc);
  }
}

// 重建所有 instance buffer（只在 cropsDirty 那幀呼叫）：(1) 計數定容量 (2) 填矩陣/顏色 (3) 設 count＋needsUpdate。
function rebuildCropInstances() {
  if (!cropBatches) initCropInstances();
  const need = { seed: 0, sprout: 0, mature: 0, dry: 0, barbg: 0, barfill: 0, soil: 0, pole: 0, arm: 0 };
  for (const [, r] of fields) {
    if (r.fade <= 0.005) continue; // 幾近全透明（剛生/將滅）不畫
    forEachFieldInstance(r, (kind) => { need[kind]++; });
  }
  for (const k in need) ensureCropCap(k, need[k]);
  const idx = { seed: 0, sprout: 0, mature: 0, dry: 0, barbg: 0, barfill: 0, soil: 0, pole: 0, arm: 0 };
  const dummy = _instDummy;
  for (const [, r] of fields) {
    if (r.fade <= 0.005) continue;
    forEachFieldInstance(r, (kind, x, y, z, sxx, syy, szz, color) => {
      const b = cropBatches[kind], i = idx[kind]++;
      dummy.position.set(x, y, z);
      dummy.scale.set(sxx, syy, szz);
      dummy.updateMatrix();
      b.setMatrixAt(i, dummy.matrix);
      if (color !== undefined) { _instColor.setHex(color); b.setColorAt(i, _instColor); }
    });
  }
  for (const k in cropBatches) {
    const b = cropBatches[k];
    b.count = idx[k]; // count 之外的舊矩陣自動不畫（收成/淡出即時生效）
    if (b.instanceMatrix) b.instanceMatrix.needsUpdate = true;
    if (b.instanceColor) b.instanceColor.needsUpdate = true;
  }
}

// 田的 reconcile（instancing 版）：以 owner 為 key（per-player 一塊地）。把每塊田存成輕量 record（資料＋
// 中心座標＋fade/lod/digest），只在「田增減／digest 變／中心移動／自他切換」時標 cropsDirty。
function reconcileFieldsInst(list, recvT) {
  const seen = new Set();
  if (Array.isArray(list)) {
    for (const field of list) {
      try {
        if (!field || typeof field !== "object") continue;
        if (!Number.isFinite(field.origin_x) || !Number.isFinite(field.origin_y)) continue;
        const key = String(field.owner || (field.origin_x + "_" + field.origin_y));
        seen.add(key);
        const cols = Number.isFinite(field.cols) && field.cols > 0 ? Math.min(field.cols, 64) : 1;
        const rows = Number.isFinite(field.rows) && field.rows > 0 ? Math.min(field.rows, 64) : 1;
        const tileS = fieldTileScene(field);
        const tsPx = Number.isFinite(field.tile_size) && field.tile_size > 0 ? field.tile_size : 48;
        const cx = sx(field.origin_x + (cols * tsPx) / 2); // 田中心場景座標（record 原點＝田中心）
        const cz = sz(field.origin_y + (rows * tsPx) / 2);
        const own = !!(field.owner && myId && field.owner === myId);
        let r = fields.get(key);
        if (!r) {
          r = { fade: 0, fadeTarget: 1, removing: false, lodHidden: false, digest: "", cx, cz, own };
          fields.set(key, r);
          cropsDirty = true;
        }
        r.cells = Array.isArray(field.cells) ? field.cells : [];
        r.scarecrow = field.scarecrow;
        r.cols = cols; r.rows = rows; r.tileS = tileS;
        if (cx !== r.cx || cz !== r.cz) { r.cx = cx; r.cz = cz; cropsDirty = true; }
        if (own !== r.own) { r.own = own; cropsDirty = true; }
        if (r.removing) { r.removing = false; r.fadeTarget = 1; cropsDirty = true; }
        const dg = fieldDigest(field);
        if (dg !== r.digest) { r.digest = dg; cropsDirty = true; }
      } catch (e) {
        console.warn("reconcileFields 單筆失敗，已略過", e);
      }
    }
  }
  // 沒在這份快照出現的田 → 淡出移除（AOI 邊緣不啪一下消失）。
  for (const [key, r] of fields) {
    if (!seen.has(key) && !r.removing) { r.removing = true; r.fadeTarget = 0; cropsDirty = true; }
  }
}

// 田的每幀更新（instancing 版）：手動推進 AOI 淡入淡出＋距離 LOD（皆鏡像 legacy 收斂率/遲滯帶），
// 任一改變即標 cropsDirty；成熟金果發光整批脈動一次；最後若 dirty 才重建一次 instance buffer。
function updateFieldsInst(dt, t) {
  for (const [key, r] of fields) {
    const tgt = r.fadeTarget != null ? r.fadeTarget : 1;
    const nf = r.fade + (tgt - r.fade) * Math.min(1, dt * FADE_RATE);
    if (Math.abs(nf - r.fade) > 0.0008) cropsDirty = true; // 仍在淡入淡出 → 縮放在動 → 該幀重建
    r.fade = nf;
    if (r.removing && nf < 0.02) { fields.delete(key); cropsDirty = true; continue; }
    if (camera) { // 距離 LOD（遲滯帶；鏡像 legacy）：遠到看不清就只留土底
      const dx = r.cx - camera.position.x, dz = r.cz - camera.position.z;
      const d2 = dx * dx + dz * dz;
      let hide = r.lodHidden;
      if (d2 > CROP_LOD_OUT2) hide = true; else if (d2 < CROP_LOD_IN2) hide = false;
      if (hide !== r.lodHidden) { r.lodHidden = hide; cropsDirty = true; }
    }
  }
  // 成熟金果發光脈動：整批共用材質一次更新（極省；取代 legacy 的每顆脈動，視覺保留）。尊重 reduceMotion。
  if (_matureMat) _matureMat.emissiveIntensity = reduceMotion ? 0.5 : (0.4 + 0.2 * (0.5 + 0.5 * Math.sin(t * 2.2)));
  if (cropsDirty) { rebuildCropInstances(); cropsDirty = false; }
}

// 派發：真瀏覽器走實例化、假 THREE（render-smoke）退回 legacy 每格一 mesh 路徑。
function reconcileFields(list, recvT) { return FIELD_INSTANCING ? reconcileFieldsInst(list, recvT) : reconcileFieldsLegacy(list, recvT); }
function updateFields(dt, t) { return FIELD_INSTANCING ? updateFieldsInst(dt, t) : updateFieldsLegacy(dt, t); }

// ============================================================
// 人造地標在 3D 裡立起來（ROADMAP 616）：把快照裡早就有、2D 一直看得到、3D 卻整個忽略的
// 三種「眾人一起蓋出來」的世界地標接進 3D，讓「這個世界裡住著會動手蓋東西的人」一眼看得見——
//   · 篝火 `campfires`（CampfireView）：交叉柴堆＋跳動火焰＋地面暖意圈，圍爐越多火越旺、入夜更亮、將熄漸弱。
//   · 協力瞭望塔 `watchtowers`（WatchtowerView）：依進度從工地一節節升起，落成後入夜亮起塔頂燈。
//   · 署名雪人 `snowmen`（SnowmanView）：三球疊起＋圍巾＋表情，頭頂署名與愛心數（天回暖即消融）。
// 純讀快照、零後端改動、零協議改動——資料本來就在 CampfireView/WatchtowerView/SnowmanView 裡（2D game.js 早在用）。
// 這三者都是「不會走動的固定地標」，故不進 updateRemoteEntities（那是給會移動的實體內插用），
// 走跟農地一樣的 reconcileStatic：以 id 為 key、位置一次定位、只走 AOI 淡入淡出。
// ============================================================

// 雪人圍巾配色（鏡像 2D game.js SNOWMAN_STYLES，與後端 snowman::SNOWMAN_STYLES(4) 對齊）。
// 同 style 永遠同圍巾色＋表情，每座雪人各有個性。未知 style 取模保守落在合法款。
const SNOWMAN_STYLES_3D = [
  { scarf: 0xd4533f, face: "🙂" }, // 紅圍巾
  { scarf: 0x3f7bd4, face: "😊" }, // 藍圍巾
  { scarf: 0x3fae72, face: "😌" }, // 綠圍巾
  { scarf: 0xc46fd0, face: "🤗" }, // 紫圍巾
];

const CAMPFIRE_LOG_COLOR = 0x6b4a2f;   // 柴薪：深木褐
const CAMPFIRE_FLAME_COLOR = 0xff8a2a; // 火焰：暖橙（會發光）
const CAMPFIRE_GLOW_COLOR = 0xffb060;  // 暖意圈：柔橙
const TOWER_WOOD_COLOR = 0x8a6a44;     // 塔身：木黃褐
const TOWER_BEACON_COLOR = 0xffe08a;   // 塔頂燈：暖黃（落成後入夜亮起）
const SNOW_COLOR = 0xeef4ff;           // 雪：帶藍的白
const WELL_STONE_COLOR = 0x8f8d86;     // 古井石身：暖灰石
const WELL_RIM_COLOR = 0x6f6d66;       // 井緣石環：深一階
const WELL_WOOD_COLOR = 0x8a6a44;      // 木頂柱：木黃褐（對齊瞭望塔木色）
const WELL_ROOF_COLOR = 0x9a5a3a;      // 井頂小屋頂：磚紅褐
const WELL_WATER_COLOR = 0x4aa3f5;     // 井水：清透藍（與作物缺水藍同色系——這水正是來治那片乾旱）
const WELL_RIPPLE_COLOR = 0x9fd4ff;    // 汲水水波：淺亮藍
// 故鄉茶棚（ROADMAP 641，禱告驅動·應露娜之禱）：市集一角的熱茶攤，暖色調與冷色的古井相對。
const TEA_POST_COLOR = 0x8a6a44;       // 棚柱：木黃褐（對齊古井/瞭望塔木色，一望即知同個世界）
const TEA_COUNTER_COLOR = 0xa9794e;    // 攤台：淺木褐
const TEA_AWNING_COLOR = 0xc7553f;     // 棚頂遮陽布：暖磚紅（市集攤位的招牌色）
const TEA_AWNING_STRIPE_COLOR = 0xe8c9a0; // 遮陽布淺色條紋
const TEA_POT_COLOR = 0x4a4640;        // 茶壺：深鐵灰
const TEA_LANTERN_COLOR = 0xffcf6e;    // 暖燈：琥珀黃（呼應露娜「街角熱茶暖身」的暖意）
const TEA_STEAM_COLOR = 0xf3ead8;      // 出爐蒸汽：暖白
// 田邊清泉（ROADMAP 647，禱告驅動·應諾娃之禱）：農田北坡天然湧泉，色系清冷帶青——水從大地自然湧出。
const SPRING_BASIN_COLOR  = 0x8a9090; // 圍石水盆：青灰石（帶海水礦物感）
const SPRING_WATER_COLOR  = 0x68d4f0; // 清泉水面：淡青綠（比古井藍更清澈透亮——泉水比井水新鮮）
const SPRING_ROCK_COLOR   = 0x707878; // 天然岩石：深青灰（與古井石色稍異，強調「大地原石」而非人工砌石）
const SPRING_MOSS_COLOR   = 0x4a8060; // 苔蘚：沉鬱綠（濕潤石頭上特有的深苔色）
const SPRING_BUBBLE_COLOR = 0xc0eef8; // 湧泉水泡：近白青（清澈見底、若隱若現）
const SPRING_RIPPLE_COLOR = 0x8cd8f0; // 漣漪環：清水藍（比水面稍亮，讓玩家一眼看出「水在動」）

// 故鄉街燈（ROADMAP 648，禱告驅動·應露娜之禱）：入夜自動亮起，照亮露娜回家的路。色系暖琥珀。
const LAMP_POST_COLOR  = 0x3d3a36; // 燈柱：深鐵灰（低調沉穩的金屬色）
const LAMP_ARM_COLOR   = 0x4a4640; // 弧臂：略淡鐵灰
const LAMP_GLOBE_COLOR = 0xfff0c0; // 燈球：暖乳白（類蠟燭暖光，日間淡顯、夜間發亮）
const LAMP_GLOW_COLOR  = 0xffdd80; // 光暈：琥珀黃（尊重露娜「街角熱茶暖身」暖意，與茶棚燈籠同色系）
const LAMP_BASE_COLOR  = 0x7a6e62; // 燈座：暖灰棕（底座磚石感，與古井石色相近）

// 篝火視覺（純函式）：圍爐人數→火旺（鏡像 2D campfireBlazeScale）、剩餘秒數→將熄漸弱、warmth_radius→暖意圈大小。
// 只讀權威欄位、確定性、壞值安全；前端據此調火焰縮放／暖圈半徑／將熄透明度。
function campfireVisual(item) {
  if (!item || typeof item !== "object") return { blaze: 1, warmthRadius: 60, gather: 0, dying: false };
  const n = Number.isFinite(item.gather_count) ? Math.max(0, Math.floor(item.gather_count)) : 0;
  // 圍爐越多火越旺（鏡像 2D：1 + min((n-1)*0.10, 0.40)）
  const blaze = 1 + Math.min(Math.max(0, n - 1) * 0.10, 0.40);
  const wr = Number.isFinite(item.warmth_radius) && item.warmth_radius > 0 ? item.warmth_radius : 60;
  // 剩餘秒數低（將熄）→ dying，前端讓火光漸弱
  const dying = Number.isFinite(item.remaining_secs) && item.remaining_secs <= 10;
  return { blaze, warmthRadius: wr, gather: n, dying };
}

// 瞭望塔視覺（純函式）：進度 0..1（夾）、是否落成、本拍協力工人數。前端據此把塔身升到對應高度、落成後亮燈。
function watchtowerVisual(item) {
  if (!item || typeof item !== "object") return { progress: 0, done: false, builders: 0 };
  let p = Number.isFinite(item.progress) ? item.progress / 100 : 0;
  p = p < 0 ? 0 : p > 1 ? 1 : p;
  const done = item.done === true || p >= 1;
  const builders = Number.isFinite(item.builders) ? Math.max(0, Math.floor(item.builders)) : 0;
  return { progress: done ? 1 : p, done, builders };
}

// 雪人視覺（純函式）：style→圍巾色＋表情、累積愛心數、署名。確定性、壞值安全。
function snowmanVisual(item) {
  const n = SNOWMAN_STYLES_3D.length;
  if (!item || typeof item !== "object") return { ...SNOWMAN_STYLES_3D[0], cheers: 0, builder: "" };
  const raw = Number.isFinite(item.style) ? Math.floor(item.style) : 0;
  const spec = SNOWMAN_STYLES_3D[((raw % n) + n) % n];
  const cheers = Number.isFinite(item.cheers) ? Math.max(0, Math.floor(item.cheers)) : 0;
  return { scarf: spec.scarf, face: spec.face, cheers, builder: typeof item.builder === "string" ? item.builder : "" };
}

// 視野內人造地標的 HUD 標籤：幾座篝火／瞭望塔／雪人。純函式、壞值安全；全無則回空字串。
function structuresHudLabel(campfires, watchtowers, snowmen) {
  const c = Array.isArray(campfires) ? campfires.length : 0;
  const w = Array.isArray(watchtowers) ? watchtowers.length : 0;
  const s = Array.isArray(snowmen) ? snowmen.length : 0;
  const parts = [];
  if (c) parts.push("🔥" + c);
  if (w) parts.push("🗼" + w);
  if (s) parts.push("⛄" + s);
  return parts.join(" ");
}

// 入夜程度（0＝大白天，1＝全黑）：篝火／塔頂燈據此入夜更亮。讀最新權威 daynight.light。
function nightFactor() {
  const lt = latestDayNight && Number.isFinite(latestDayNight.light) ? latestDayNight.light : 1;
  const c = lt < 0 ? 0 : lt > 1 ? 1 : lt;
  return 1 - c;
}

// 共用幾何（全模組只建一次，幾十座地標也不重建頂點）。材質一律「每座一份」——AOI 淡入淡出
// 才能各自獨立調 opacity，不牽連同型的別座。
const ST_GEO = {
  log:       new THREE.CylinderGeometry(0.4, 0.4, 5, 5),    // 篝火柴薪（細圓木）
  flame:     new THREE.ConeGeometry(1.6, 4.2, 7),           // 火焰（尖錐）
  ember:     new THREE.ConeGeometry(0.9, 2.4, 6),           // 內焰（亮心）
  glowDisc:  new THREE.CylinderGeometry(1, 1, 0.2, 24),     // 暖意圈（扁圓盤，鋪地，半徑由 scale 撐）
  towerPost: new THREE.BoxGeometry(0.9, 1, 0.9),            // 塔柱（高度由 scale.y 撐）
  towerDeck: new THREE.BoxGeometry(6, 0.8, 6),              // 瞭望平台
  towerRoof: new THREE.ConeGeometry(4.6, 3.2, 4),           // 塔頂尖頂
  beacon:    new THREE.SphereGeometry(1.1, 8, 6),           // 塔頂燈
  snowBall:  new THREE.SphereGeometry(1, 10, 8),            // 雪球（三球疊起，半徑由 scale 撐）
  scarf:     new THREE.TorusGeometry(2.2, 0.5, 6, 12),      // 圍巾
  nose:      new THREE.ConeGeometry(0.35, 1.6, 5),          // 紅蘿蔔鼻
  // 故鄉古井（ROADMAP 640）：石井身（中空圓筒，敞口）＋井緣環＋木頂柱＋小斜頂＋水面＋地面水波。
  wellWall:  new THREE.CylinderGeometry(1.5, 1.7, 1.8, 10, 1, true), // 石井身（敞口圓筒）
  wellRim:   new THREE.TorusGeometry(1.55, 0.22, 6, 12),    // 井緣石環
  wellWater: new THREE.CircleGeometry(1.4, 12),             // 井裡的水面
  wellPost:  new THREE.CylinderGeometry(0.13, 0.13, 2.6, 5),// 撐小屋頂的木柱
  wellRoof:  new THREE.ConeGeometry(2.1, 1.3, 4),           // 井上小斜頂
  wellRipple:new THREE.RingGeometry(0.85, 1.0, 18),         // 汲水時井口盪開的水波環
  // 故鄉茶棚（ROADMAP 641）：攤台＋兩根棚柱＋斜遮陽棚頂＋茶壺＋暖燈＋出爐蒸汽。
  teaCounter: new THREE.BoxGeometry(3.0, 1.0, 1.4),         // 攤台（櫃身）
  teaPost:    new THREE.CylinderGeometry(0.12, 0.12, 3.0, 5), // 撐遮陽棚的木柱
  teaAwning:  new THREE.ConeGeometry(2.6, 0.9, 4),          // 斜遮陽棚頂（四角錐＝市集攤布）
  teaPot:     new THREE.CylinderGeometry(0.42, 0.5, 0.7, 8),// 茶壺身
  teaSpout:   new THREE.CylinderGeometry(0.07, 0.1, 0.6, 4),// 茶壺嘴
  teaLantern: new THREE.SphereGeometry(0.26, 8, 6),         // 棚角暖燈
  teaSteam:   new THREE.SphereGeometry(0.3, 6, 5),          // 出爐蒸汽團（嫋嫋上升）
  // 居民木屋（ROADMAP 642）：木屋主體＋四角錐屋頂＋門＋窗格＋煙囪＋窗光球。
  homeBody:    new THREE.BoxGeometry(4.5, 3.0, 3.5),        // 木屋主體（木板牆，寬×高×深）
  homeRoof:    new THREE.ConeGeometry(3.2, 2.0, 4),         // 屋頂（四角錐尖頂）
  homeDoor:    new THREE.BoxGeometry(0.9, 1.5, 0.15),       // 木門（前面正中）
  homeWindow:  new THREE.BoxGeometry(0.9, 0.8, 0.15),       // 窗格（兩扇，正面及側面各一）
  homeChimney: new THREE.BoxGeometry(0.5, 1.6, 0.5),        // 煙囪（磚石，斜屋頂後側）
  homeGlow:    new THREE.SphereGeometry(0.55, 6, 4),        // 窗光球（自發光暖黃，夜晚更亮）
  // 林野小屋（ROADMAP 644 cabin）：比木屋更矮小，單坡屋頂（不同角度錐），無煙囪，一扇小窗。
  cabinBody:   new THREE.BoxGeometry(3.5, 2.2, 2.8),        // 小屋主體（較矮小）
  cabinRoof:   new THREE.ConeGeometry(2.6, 2.8, 4),         // 單坡式高錐頂（比 house 更陡峭）
  cabinDoor:   new THREE.BoxGeometry(0.7, 1.2, 0.15),       // 較矮木門
  cabinWindow: new THREE.BoxGeometry(0.7, 0.6, 0.15),       // 一扇小窗
  cabinGlow:   new THREE.SphereGeometry(0.42, 6, 4),        // 窗光球（小屋光源更暗）
  // 遊牧帳篷（ROADMAP 644 tent）：圓錐形布面＋支撐木桿＋帳門暗口。
  tentCone:    new THREE.ConeGeometry(3.0, 6.0, 8),         // 帳篷主體（八邊形圓錐）
  tentPole:    new THREE.CylinderGeometry(0.07, 0.07, 7.5, 4), // 中央支撐木桿（略高出頂端）
  tentFlag:    new THREE.BoxGeometry(0.5, 0.3, 0.05),       // 頂端小旗幟
  tentDoor:    new THREE.BoxGeometry(1.0, 1.6, 0.1),        // 帳門暗口（正面黑色長方形）
  // 田邊清泉（ROADMAP 647）：低多邊形圍石水盆＋天然岩石＋苔蘚球＋漣漪環＋水泡。
  springBasin:  new THREE.CylinderGeometry(2.2, 2.0, 0.55, 12), // 圍石水盆（敞口淺盤）
  springWater:  new THREE.CircleGeometry(1.85, 14),             // 清泉水面（盤底平放）
  springRock1:  new THREE.SphereGeometry(0.9, 7, 5),           // 後側大岩石（主視覺）
  springRock2:  new THREE.SphereGeometry(0.55, 6, 4),          // 左側中岩石
  springRock3:  new THREE.SphereGeometry(0.38, 5, 4),          // 右側小岩石
  springBubble: new THREE.SphereGeometry(0.11, 5, 4),          // 湧泉水泡（緩緩浮起）
  springRipple: new THREE.RingGeometry(0.35, 0.5, 12),         // 漣漪環（自泉眼向外擴散）
  springMoss:   new THREE.SphereGeometry(0.17, 5, 4),          // 苔蘚球（點綴石縫）
};

// 一座篝火：交叉柴堆＋雙層火焰（外焰/亮心，會跳動發光）＋地面暖意圈。火焰／暖圈引用留在 userData 供每幀調。
function makeCampfire(item) {
  const v = campfireVisual(item);
  const g = new THREE.Group();
  // 交叉柴薪：四根斜放圍成井字火堆
  for (let i = 0; i < 4; i++) {
    const log = new THREE.Mesh(ST_GEO.log, new THREE.MeshLambertMaterial({ color: CAMPFIRE_LOG_COLOR }));
    const ang = (i / 4) * Math.PI;
    log.position.set(0, 0.7, 0);
    log.rotation.set(Math.PI / 2.4, ang, 0);
    g.add(log);
  }
  // 外焰（橙、發光）
  const flame = new THREE.Mesh(ST_GEO.flame, new THREE.MeshBasicMaterial({ color: CAMPFIRE_FLAME_COLOR, transparent: true }));
  flame.position.y = 2.6;
  g.add(flame);
  // 內焰亮心（亮黃、更小）
  const ember = new THREE.Mesh(ST_GEO.ember, new THREE.MeshBasicMaterial({ color: 0xfff0a0, transparent: true }));
  ember.position.y = 2.2;
  g.add(ember);
  // 地面暖意圈：扁圓盤，半徑＝warmth_radius（世界像素）×WORLD_SCALE
  const glow = new THREE.Mesh(ST_GEO.glowDisc, new THREE.MeshBasicMaterial({ color: CAMPFIRE_GLOW_COLOR, transparent: true, opacity: 0.16, depthWrite: false }));
  const r = Math.max(2, v.warmthRadius * WORLD_SCALE);
  glow.scale.set(r, 1, r);
  glow.position.y = 0.12;
  g.add(glow);
  g.userData.flame = flame;
  g.userData.ember = ember;
  g.userData.glow = glow;
  return g;
}

// 一口故鄉古井（ROADMAP 640，禱告驅動）：石井身＋井緣＋井裡的水面＋兩根木柱撐一片小斜頂，
// 汲水時井口盪開一圈擴散水波。應 AI 居民諾娃之禱立在公田旁——立在這裡本身就是「世界因居民的
// 願望而長大」一眼看得見的證據。純讀快照、零後端耦合。水波引用留 userData 供每幀調。
function makeVillageWell(_item) {
  const g = new THREE.Group();
  // 石井身（敞口圓筒，立在地面上）
  const wall = new THREE.Mesh(ST_GEO.wellWall, new THREE.MeshLambertMaterial({ color: WELL_STONE_COLOR, side: THREE.DoubleSide }));
  wall.position.y = 0.9;
  g.add(wall);
  // 井緣石環
  const rim = new THREE.Mesh(ST_GEO.wellRim, new THREE.MeshLambertMaterial({ color: WELL_RIM_COLOR }));
  rim.rotation.x = Math.PI / 2;
  rim.position.y = 1.8;
  g.add(rim);
  // 井裡的水面（朝上平放，略低於井緣）
  const water = new THREE.Mesh(ST_GEO.wellWater, new THREE.MeshBasicMaterial({ color: WELL_WATER_COLOR, transparent: true, opacity: 0.85 }));
  water.rotation.x = -Math.PI / 2;
  water.position.y = 1.5;
  g.add(water);
  // 兩根木柱＋小斜頂（像個井亭，讓它一眼是「井」不是「桶」）
  for (const s of [-1, 1]) {
    const post = new THREE.Mesh(ST_GEO.wellPost, new THREE.MeshLambertMaterial({ color: WELL_WOOD_COLOR }));
    post.position.set(s * 1.4, 2.3, 0);
    g.add(post);
  }
  const roof = new THREE.Mesh(ST_GEO.wellRoof, new THREE.MeshLambertMaterial({ color: WELL_ROOF_COLOR }));
  roof.rotation.y = Math.PI / 4;
  roof.position.y = 4.0;
  g.add(roof);
  // 地面水波環：汲水時才浮現、向外擴散（平放貼地，初始隱形）。
  const ripple = new THREE.Mesh(ST_GEO.wellRipple, new THREE.MeshBasicMaterial({ color: WELL_RIPPLE_COLOR, transparent: true, opacity: 0, depthWrite: false }));
  ripple.rotation.x = -Math.PI / 2;
  ripple.position.y = 0.14;
  g.add(ripple);
  g.userData.water = water;
  g.userData.ripple = ripple;
  return g;
}

// 一座故鄉茶棚（ROADMAP 641，禱告驅動·應露娜之禱）：攤台＋兩根棚柱撐起斜遮陽棚頂＋台上茶壺與暖燈，
// 出爐時棚上嫋嫋蒸汽升騰。蒸汽團與暖燈引用留在 userData 供每幀調（出爐脈動／靜時微亮）。
function makeVillageTeaStall(_item) {
  const g = new THREE.Group();
  // 攤台（櫃身）
  const counter = new THREE.Mesh(ST_GEO.teaCounter, new THREE.MeshLambertMaterial({ color: TEA_COUNTER_COLOR }));
  counter.position.y = 0.5;
  g.add(counter);
  // 兩根棚柱
  for (const s of [-1, 1]) {
    const post = new THREE.Mesh(ST_GEO.teaPost, new THREE.MeshLambertMaterial({ color: TEA_POST_COLOR }));
    post.position.set(s * 1.3, 1.5, -0.5);
    g.add(post);
  }
  // 斜遮陽棚頂（暖磚紅四角錐，像市集攤布）＋一圈淺色條紋環點綴
  const awning = new THREE.Mesh(ST_GEO.teaAwning, new THREE.MeshLambertMaterial({ color: TEA_AWNING_COLOR }));
  awning.rotation.y = Math.PI / 4;
  awning.position.set(0, 3.4, -0.5);
  g.add(awning);
  const stripe = new THREE.Mesh(ST_GEO.wellRim, new THREE.MeshLambertMaterial({ color: TEA_AWNING_STRIPE_COLOR }));
  stripe.rotation.x = Math.PI / 2;
  stripe.scale.set(0.95, 0.95, 0.6);
  stripe.position.set(0, 3.05, -0.5);
  g.add(stripe);
  // 台上茶壺（壺身＋壺嘴）
  const pot = new THREE.Mesh(ST_GEO.teaPot, new THREE.MeshLambertMaterial({ color: TEA_POT_COLOR }));
  pot.position.set(-0.7, 1.35, 0.1);
  g.add(pot);
  const spout = new THREE.Mesh(ST_GEO.teaSpout, new THREE.MeshLambertMaterial({ color: TEA_POT_COLOR }));
  spout.rotation.z = Math.PI / 3;
  spout.position.set(-1.05, 1.45, 0.1);
  g.add(spout);
  // 棚角暖燈（自發光暖黃，呼應露娜「街角熱茶暖身」）
  const lantern = new THREE.Mesh(ST_GEO.teaLantern, new THREE.MeshBasicMaterial({ color: TEA_LANTERN_COLOR, transparent: true, opacity: 0.9 }));
  lantern.position.set(1.3, 2.7, -0.5);
  g.add(lantern);
  // 出爐蒸汽團：出爐時自茶壺嘴上方升起＋淡出（初始隱形）
  const steam = new THREE.Mesh(ST_GEO.teaSteam, new THREE.MeshBasicMaterial({ color: TEA_STEAM_COLOR, transparent: true, opacity: 0, depthWrite: false }));
  steam.position.set(-0.9, 1.9, 0.1);
  g.add(steam);
  g.userData.lantern = lantern;
  g.userData.steam = steam;
  return g;
}

// ── 豐收節慶典裝飾（ROADMAP 646，禱告驅動·應露娜之禱） ─────────────────────────
// 露娜反覆禱告「盼望有個豐收節好熱鬧一下」——廣場升起彩旗柱＋彩燈籠。
// active=true 時旗幟舞動、燈籠搖曳發光；非活躍時裝飾靜立（提示慶典將至）。
const FEST_POLE_COLOR    = 0x7a5a3a; // 旗竿（深木棕）
const FEST_FLAG_COLORS   = [0xe06030, 0xd4a820, 0x48a860, 0x3878c0]; // 彩旗（橙/金/翠/藍）
const FEST_LANTERN_COLOR = 0xff8830; // 燈籠（暖橙）
const FEST_BANNER_COLOR  = 0xfae080; // 橫幅（米黃）

function makeHarvestFestival(_item) {
  const g = new THREE.Group();
  // 兩根旗竿（左右對稱，間距 4 個世界單位）
  const poleGeo = new THREE.CylinderGeometry(0.12, 0.15, 7.0, 6);
  for (let s = -1; s <= 1; s += 2) {
    const pole = new THREE.Mesh(poleGeo, new THREE.MeshLambertMaterial({ color: FEST_POLE_COLOR }));
    pole.position.set(s * 2.0, 3.5, 0);
    g.add(pole);
    // 竿頂尖球（裝飾用）
    const tip = new THREE.Mesh(new THREE.SphereGeometry(0.22, 6, 6),
      new THREE.MeshLambertMaterial({ color: FEST_FLAG_COLORS[1] }));
    tip.position.set(s * 2.0, 7.2, 0);
    g.add(tip);
    // 三角彩旗（上中下各一面，交錯顏色）
    for (let fi = 0; fi < 3; fi++) {
      const flagGeo = new THREE.ConeGeometry(0.5, 1.2, 3);
      const flag = new THREE.Mesh(flagGeo,
        new THREE.MeshLambertMaterial({ color: FEST_FLAG_COLORS[(fi + (s > 0 ? 2 : 0)) % 4], side: THREE.DoubleSide }));
      flag.rotation.z = s > 0 ? -Math.PI / 2 : Math.PI / 2;
      flag.position.set(s > 0 ? s * 2.5 + 0.3 : s * 2.5 - 0.3, 5.8 - fi * 1.6, 0);
      g.add(flag);
      // 旗子節點留在 userData.flags 供動畫搖擺
      if (!g.userData.flags) g.userData.flags = [];
      g.userData.flags.push({ mesh: flag, baseY: 5.8 - fi * 1.6, side: s });
    }
  }
  // 兩竿間橫幅（薄平板，暖米黃色，掛在兩竿中間偏上）
  const bannerGeo = new THREE.BoxGeometry(3.6, 0.85, 0.08);
  const banner = new THREE.Mesh(bannerGeo, new THREE.MeshLambertMaterial({ color: FEST_BANNER_COLOR }));
  banner.position.set(0, 6.0, 0);
  g.add(banner);
  // 三顆彩燈籠（掛在橫幅下方，間距均勻；初始半透）
  const lanternGeo = new THREE.SphereGeometry(0.38, 7, 5);
  const lanterns = [];
  for (let i = 0; i < 3; i++) {
    const lan = new THREE.Mesh(lanternGeo,
      new THREE.MeshBasicMaterial({ color: FEST_LANTERN_COLOR, transparent: true, opacity: 0.25, depthWrite: false }));
    lan.position.set((i - 1) * 1.4, 5.0, 0);
    g.add(lan);
    lanterns.push(lan);
  }
  g.userData.lanterns = lanterns;
  g.userData.banner = banner;
  return g;
}

// ── 田邊清泉（ROADMAP 647，禱告驅動·應諾娃之禱） ──────────────────────────────
// 諾娃反覆禱告「願農田旁能有清泉常流，灌溉我的汗水與希望」——農田北坡天然湧出的清泉，
// 與古井（西側人工水源）、茶棚（東側市集設施）並立，讓農田三方都有水土生機的回應。
// 圍石水盆＋後側主岩＋左右小岩石＋苔蘚點綴＋漣漪環向外擴散＋湧泉水泡緩緩浮起。
// 純讀快照座標、零後端耦合；漣漪／水泡動畫完全基於本機時間，尊重 reduceMotion。
function makeFieldSpring(_item) {
  const g = new THREE.Group();
  // 圍石水盆（淺盤敞口，稍高於地面——大地的裂縫聚水成泉）
  const basin = new THREE.Mesh(ST_GEO.springBasin,
    new THREE.MeshLambertMaterial({ color: SPRING_BASIN_COLOR, side: THREE.DoubleSide }));
  basin.position.y = 0.28;
  g.add(basin);
  // 清泉水面（盤底平鋪，透明感，色比古井水更清澈）
  const water = new THREE.Mesh(ST_GEO.springWater,
    new THREE.MeshBasicMaterial({ color: SPRING_WATER_COLOR, transparent: true, opacity: 0.82 }));
  water.rotation.x = -Math.PI / 2;
  water.position.y = 0.55;
  g.add(water);
  // 後側主岩石（天然湧泉感——水從石縫中湧出）
  const rock1 = new THREE.Mesh(ST_GEO.springRock1,
    new THREE.MeshLambertMaterial({ color: SPRING_ROCK_COLOR }));
  rock1.scale.y = 0.75; // 壓扁一點，更像自然岩石
  rock1.position.set(0, 0.68, -1.7);
  g.add(rock1);
  // 左側中岩石
  const rock2 = new THREE.Mesh(ST_GEO.springRock2,
    new THREE.MeshLambertMaterial({ color: SPRING_ROCK_COLOR }));
  rock2.position.set(-1.55, 0.38, -0.75);
  g.add(rock2);
  // 右側小岩石
  const rock3 = new THREE.Mesh(ST_GEO.springRock3,
    new THREE.MeshLambertMaterial({ color: SPRING_ROCK_COLOR }));
  rock3.position.set(1.4, 0.25, -0.55);
  g.add(rock3);
  // 苔蘚球（石縫潮濕地帶的典型覆蓋）
  for (const [px, py, pz] of [[-0.45, 0.62, -2.1], [0.38, 0.42, -1.95], [-1.75, 0.48, -0.3]]) {
    const moss = new THREE.Mesh(ST_GEO.springMoss,
      new THREE.MeshLambertMaterial({ color: SPRING_MOSS_COLOR }));
    moss.position.set(px, py, pz);
    g.add(moss);
  }
  // 漣漪環：從泉眼中心向外循環擴散（平放貼水面，初始縮小隱形）
  const ripple = new THREE.Mesh(ST_GEO.springRipple,
    new THREE.MeshBasicMaterial({ color: SPRING_RIPPLE_COLOR, transparent: true, opacity: 0, depthWrite: false }));
  ripple.rotation.x = -Math.PI / 2;
  ripple.position.y = 0.58;
  g.add(ripple);
  // 湧泉水泡（清泉特有的微小泡泡，緩緩浮起消散）
  const bubble = new THREE.Mesh(ST_GEO.springBubble,
    new THREE.MeshBasicMaterial({ color: SPRING_BUBBLE_COLOR, transparent: true, opacity: 0.6, depthWrite: false }));
  bubble.position.set(0.25, 0.68, 0.15);
  g.add(bubble);
  g.userData.water = water;
  g.userData.ripple = ripple;
  g.userData.bubble = bubble;
  return g;
}

// ── 故鄉街燈（ROADMAP 648，禱告驅動·應露娜之禱） ─────────────────────────────────
// 露娜反覆禱告「願今晚的街燈亮起，照亮我回家的路」——城鎮沿途立起七盞街燈，
// 入夜後自動亮起暖琥珀光暈，白天低調靜立，不搶佔視線；僅夜間才搶眼。
// 燈柱：細挺鐵竿＋弧臂＋頂端燈球（暖乳白）＋地面光暈圓盤（入夜才亮）。
// 與古井、茶棚、木屋一樣走 reconcileStatic，純靜態座標不隨時間移動。
function makeStreetLamp(_item) {
  const g = new THREE.Group();
  // 底座磚塊（微小正方體，讓燈柱「插入地面」有點根基感）
  const base = new THREE.Mesh(
    new THREE.BoxGeometry(0.65, 0.35, 0.65),
    new THREE.MeshLambertMaterial({ color: LAMP_BASE_COLOR })
  );
  base.position.y = 0.175;
  g.add(base);
  // 燈柱（細長圓柱，深鐵灰）
  const post = new THREE.Mesh(
    new THREE.CylinderGeometry(0.1, 0.13, 5.5, 6),
    new THREE.MeshLambertMaterial({ color: LAMP_POST_COLOR })
  );
  post.position.y = 3.1;
  g.add(post);
  // 弧臂（短橫桿，往前伸出——低多邊形簡化為薄圓柱橫置）
  const arm = new THREE.Mesh(
    new THREE.CylinderGeometry(0.07, 0.07, 1.4, 5),
    new THREE.MeshLambertMaterial({ color: LAMP_ARM_COLOR })
  );
  arm.rotation.z = Math.PI / 2;     // 橫置
  arm.position.set(0.55, 5.65, 0);  // 柱頂偏前
  g.add(arm);
  // 燈球（小圓球，暖乳白；日間半透，夜間幾乎不透——讓白天也看得到輪廓）
  const globe = new THREE.Mesh(
    new THREE.SphereGeometry(0.38, 7, 5),
    new THREE.MeshBasicMaterial({ color: LAMP_GLOBE_COLOR, transparent: true, opacity: 0.3, depthWrite: false })
  );
  globe.position.set(1.25, 5.65, 0); // 弧臂末端
  g.add(globe);
  // 地面光暈圓盤（入夜才亮；白天 opacity=0；圓盤鋪地，半徑 1.8 世界單位）
  const glow = new THREE.Mesh(
    new THREE.CircleGeometry(1.8, 16),
    new THREE.MeshBasicMaterial({ color: LAMP_GLOW_COLOR, transparent: true, opacity: 0, depthWrite: false, side: THREE.DoubleSide })
  );
  glow.rotation.x = -Math.PI / 2;   // 平鋪地面
  glow.position.set(1.25, 0.05, 0);  // 燈球正下方
  g.add(glow);
  g.userData.globe = globe;
  g.userData.glow  = glow;
  return g;
}

// 居民住宅色盤（ROADMAP 642–643）：依居民風格分兩套，純物件方便按名字查取。
// 露娜（木屋）：暖棕木板牆＋磚紅瓦頂——靠市集、溫暖療癒感。
// 諾娃（農舍）：石灰牆＋草綠茅草頂——靠農田、樸實農家感。
const HOME_PALETTES = {
  露娜: {
    body:    0xc4956a, // 木板牆（暖棕）
    roof:    0x9a4a2a, // 瓦頂（磚紅陶土）
    door:    0x6b3d2a, // 木門（深棕）
    window:  0x8b6914, // 窗框（蜂蜜棕）
    chimney: 0x7a6a5a, // 煙囪（灰石）
    glow:    0xffd080, // 窗光（暖琥珀黃）
    label:   "的家",
  },
  諾娃: {
    body:    0xb0a898, // 石灰牆（田邊撿來的灰石塊，比露娜牆冷一階）
    roof:    0x5a7a3a, // 草綠茅草頂（農田風）
    door:    0x5a3a22, // 木門（與露娜同色系，皆是深棕舊木）
    window:  0x6a7a4a, // 窗框（橄欖綠，呼應茅草頂）
    chimney: 0x8a8070, // 煙囪（粗糙石灰，比露娜更樸素）
    glow:    0xffc060, // 窗光（稍冷琥珀，農舍燈少燻黃）
    label:   "的農舍",
  },
  // ROADMAP 644 散居三棲所色盤 ───────────────────────────────────────────
  賽勒: {
    // 漁人小屋：海岸木料＋藍灰屋頂＋漁網褐窗框＋冷白窗光（海邊燈色）
    body:    0x7a9aaa, // 藍灰舊木板牆（海風侵蝕）
    roof:    0x4a6a7a, // 深灰藍石板頂
    door:    0x4a3820, // 深棕木門（海水染深）
    window:  0x5a7a80, // 漁網藍窗框
    chimney: 0x505870, // 無用（cabin 無煙囪，傳入後不渲染，預設中性灰）
    glow:    0xd0f0ff, // 冷白藍窗光（海邊燈色，像漁燈）
    label:   "的小屋",
  },
  奧瑞: {
    // 隱士石寮：岩地深棕石牆＋深石板頂＋厚重石門＋冷琥珀窗光（隱士燈火）
    body:    0x6a5a4a, // 深灰棕石牆（岩石地帶）
    roof:    0x3a3028, // 近黑石板屋頂（沉重古老）
    door:    0x3a2a1a, // 極深棕木門
    window:  0x5a4a3a, // 石框窗（暗棕）
    chimney: 0x4a4040, // 同上（cabin 無煙囪，中性備用）
    glow:    0xffb840, // 冷琥珀窗光（隱士燈火，比露娜更孤寂）
    label:   "的石寮",
  },
  薇朵: {
    // 遊牧帳篷：溫暖草原土黃＋赭紅頂布＋深橄欖木桿＋溫白窗光（帳篷火光）
    body:    0xd4b870, // 暖土黃帳篷布（草原沙土色）
    roof:    0xb05a28, // 赭紅頂布（遊牧風染色）
    door:    0x2a2010, // 黑褐帳門
    window:  0x8a6a30, // 深金棕（旗幟色）
    chimney: 0x6a5020, // 同上（tent 無煙囪，備用中性色）
    glow:    0xffe8a0, // 溫白窗光（帳篷內火光，溫暖但模糊）
    label:   "的帳篷",
  },
};
// 未知居民用露娜的木屋色盤作為預設，保持與既有系統相容。
const HOME_DEFAULT_PALETTE = HOME_PALETTES["露娜"];

// 居民棲所工廠（ROADMAP 642–644，禱告驅動·散居擴展）。
// 依 item.dwelling_type 派發到對應形狀函式；未知類型退回 house（向後相容）。
function makeResidentHome(item) {
  const dtype = (item && item.dwelling_type) ? item.dwelling_type : "house";
  if (dtype === "cabin")  return _makeResidentCabin(item);
  if (dtype === "tent")   return _makeResidentTent(item);
  return _makeResidentHouse(item); // "house" 或未知類型
}

// 一般木屋（ROADMAP 642–643）：牆身＋四角錐屋頂＋煙囪＋兩窗＋窗光球。
function _makeResidentHouse(item) {
  const name = (item && item.name) ? item.name : "居民";
  const p = HOME_PALETTES[name] || HOME_DEFAULT_PALETTE;
  const g = new THREE.Group();
  const body = new THREE.Mesh(ST_GEO.homeBody, new THREE.MeshLambertMaterial({ color: p.body }));
  body.position.y = 1.5;
  g.add(body);
  const roof = new THREE.Mesh(ST_GEO.homeRoof, new THREE.MeshLambertMaterial({ color: p.roof }));
  roof.rotation.y = Math.PI / 4;
  roof.position.y = 4.0;
  g.add(roof);
  const chimney = new THREE.Mesh(ST_GEO.homeChimney, new THREE.MeshLambertMaterial({ color: p.chimney }));
  chimney.position.set(1.0, 4.8, -1.0);
  g.add(chimney);
  const door = new THREE.Mesh(ST_GEO.homeDoor, new THREE.MeshLambertMaterial({ color: p.door }));
  door.position.set(0, 0.75, 1.76);
  g.add(door);
  const winFront = new THREE.Mesh(ST_GEO.homeWindow, new THREE.MeshLambertMaterial({ color: p.window }));
  winFront.position.set(-1.4, 1.8, 1.76);
  g.add(winFront);
  const winSide = new THREE.Mesh(ST_GEO.homeWindow, new THREE.MeshLambertMaterial({ color: p.window }));
  winSide.rotation.y = Math.PI / 2;
  winSide.position.set(2.26, 1.8, 0);
  g.add(winSide);
  const winGlow = new THREE.Mesh(ST_GEO.homeGlow, new THREE.MeshBasicMaterial({
    color: p.glow, transparent: true, opacity: 0.0, depthWrite: false,
  }));
  winGlow.position.set(-1.4, 1.8, 1.6);
  g.add(winGlow);
  g.add(makeLabel(name + p.label));
  g.userData.winGlow = winGlow;
  return g;
}

// 林野小屋（ROADMAP 644 cabin）：矮小屋身＋陡峭錐頂＋單窗＋窗光球（無煙囪，更樸素）。
// 用於散居遠方的探索棲所（賽勒漁人小屋 / 奧瑞隱士石寮）。
function _makeResidentCabin(item) {
  const name = (item && item.name) ? item.name : "居民";
  const p = HOME_PALETTES[name] || HOME_DEFAULT_PALETTE;
  const g = new THREE.Group();
  // 矮小屋身（y=1.1 = 半高）
  const body = new THREE.Mesh(ST_GEO.cabinBody, new THREE.MeshLambertMaterial({ color: p.body }));
  body.position.y = 1.1;
  g.add(body);
  // 陡峭錐頂（比 house 更高/更尖，呼應林野風格）
  const roof = new THREE.Mesh(ST_GEO.cabinRoof, new THREE.MeshLambertMaterial({ color: p.roof }));
  roof.rotation.y = Math.PI / 4;
  roof.position.y = 3.5;
  g.add(roof);
  // 正面木門（比 house 矮）
  const door = new THREE.Mesh(ST_GEO.cabinDoor, new THREE.MeshLambertMaterial({ color: p.door }));
  door.position.set(0, 0.6, 1.51);
  g.add(door);
  // 正面單窗
  const win = new THREE.Mesh(ST_GEO.cabinWindow, new THREE.MeshLambertMaterial({ color: p.window }));
  win.position.set(-1.1, 1.4, 1.51);
  g.add(win);
  // 窗光球（比 house 更小更暗）
  const winGlow = new THREE.Mesh(ST_GEO.cabinGlow, new THREE.MeshBasicMaterial({
    color: p.glow, transparent: true, opacity: 0.0, depthWrite: false,
  }));
  winGlow.position.set(-1.1, 1.4, 1.4);
  g.add(winGlow);
  g.add(makeLabel(name + p.label));
  g.userData.winGlow = winGlow;
  return g;
}

// 遊牧帳篷（ROADMAP 644 tent）：八邊形圓錐＋中央木桿＋頂旗＋帳門暗口（無窗、無煙囪）。
// 用於薇朵的草原遊牧棲所。
function _makeResidentTent(item) {
  const name = (item && item.name) ? item.name : "居民";
  const p = HOME_PALETTES[name] || HOME_DEFAULT_PALETTE;
  const g = new THREE.Group();
  // 帳篷主體（八邊形圓錐，底面朝下，y=3 = 半高落地）
  const cone = new THREE.Mesh(ST_GEO.tentCone, new THREE.MeshLambertMaterial({ color: p.body }));
  cone.position.y = 3.0;
  g.add(cone);
  // 裝飾橫帶（用兩圈扁平圓環模擬布料花紋，圍在帳身腰部）
  for (const fy of [1.6, 3.0]) {
    const band = new THREE.Mesh(
      new THREE.CylinderGeometry(3.0 - fy * 0.3, 3.0 - fy * 0.3 + 0.1, 0.2, 8),
      new THREE.MeshLambertMaterial({ color: p.roof }),
    );
    band.position.y = fy;
    g.add(band);
  }
  // 中央支撐木桿（略高出錐頂，帶野外感）
  const pole = new THREE.Mesh(ST_GEO.tentPole, new THREE.MeshLambertMaterial({ color: 0x5a3a18 }));
  pole.position.y = 3.75; // 桿子中心在此高度
  g.add(pole);
  // 頂端小旗幟（帶顏色，隨風旗標感）
  const flag = new THREE.Mesh(ST_GEO.tentFlag, new THREE.MeshLambertMaterial({ color: p.roof }));
  flag.position.set(0.25, 7.6, 0);
  g.add(flag);
  // 帳門暗口（正面底部）
  const door = new THREE.Mesh(ST_GEO.tentDoor, new THREE.MeshLambertMaterial({ color: 0x1a1208 }));
  door.position.set(0, 0.8, 2.8);
  g.add(door);
  // 帳篷無窗，winGlow 用帳內火光（半透明暖色，入夜隱約透出布面）
  const glow = new THREE.Mesh(
    new THREE.SphereGeometry(1.8, 6, 4),
    new THREE.MeshBasicMaterial({ color: p.glow, transparent: true, opacity: 0.0, depthWrite: false }),
  );
  glow.position.y = 2.0;
  g.add(glow);
  g.add(makeLabel(name + p.label));
  g.userData.winGlow = glow; // 複用 winGlow 鍵讓 updateStructures 夜晚亮起
  return g;
}

// 一座瞭望塔：四根升起的塔柱＋平台＋尖頂＋塔頂燈。塔身高度由進度撐（工地→落成）；燈在落成入夜時亮。
function makeWatchtower(item) {
  const v = watchtowerVisual(item);
  const g = new THREE.Group();
  const posts = [];
  for (const sx2 of [-1, 1]) for (const sz2 of [-1, 1]) {
    const post = new THREE.Mesh(ST_GEO.towerPost, new THREE.MeshLambertMaterial({ color: TOWER_WOOD_COLOR }));
    post.position.set(sx2 * 2.2, 0, sz2 * 2.2); // y/縮放由 applyTowerProgress 設
    g.add(post);
    posts.push(post);
  }
  const deck = new THREE.Mesh(ST_GEO.towerDeck, new THREE.MeshLambertMaterial({ color: TOWER_WOOD_COLOR }));
  g.add(deck);
  const roof = new THREE.Mesh(ST_GEO.towerRoof, new THREE.MeshLambertMaterial({ color: 0x9a5a3a }));
  roof.rotation.y = Math.PI / 4;
  g.add(roof);
  const beacon = new THREE.Mesh(ST_GEO.beacon, new THREE.MeshBasicMaterial({ color: TOWER_BEACON_COLOR, transparent: true, opacity: 0 }));
  g.add(beacon);
  g.userData.posts = posts;
  g.userData.deck = deck;
  g.userData.roof = roof;
  g.userData.beacon = beacon;
  applyTowerProgress(g, v);
  return g;
}

// 依瞭望塔進度把塔柱升到對應高度、平台/尖頂/燈座落到塔頂。落成的塔比施工中更高、更挺。
const TOWER_MAX_H = 18; // 落成塔身高（場景單位）
function applyTowerProgress(g, v) {
  const h = Math.max(1.5, TOWER_MAX_H * (0.25 + 0.75 * v.progress)); // 即使剛起也有一截工地
  for (const post of g.userData.posts) {
    post.scale.y = h;
    post.position.y = h / 2;
  }
  g.userData.deck.position.y = h;
  g.userData.deck.visible = v.progress > 0.5; // 過半才架平台
  g.userData.roof.position.y = h + 2;
  g.userData.roof.visible = v.done; // 落成才封頂
  g.userData.beacon.position.y = h + 1;
  g.userData.builtH = h;
}

// 一座雪人：三球疊起（下大上小）＋圍巾＋紅蘿蔔鼻＋頭頂表情/署名/愛心。圍巾色由 style 決定。
function makeSnowman(item) {
  const v = snowmanVisual(item);
  const g = new THREE.Group();
  const snowMat = () => new THREE.MeshLambertMaterial({ color: SNOW_COLOR });
  const base = new THREE.Mesh(ST_GEO.snowBall, snowMat()); base.scale.setScalar(2.4); base.position.y = 2.4; g.add(base);
  const mid  = new THREE.Mesh(ST_GEO.snowBall, snowMat()); mid.scale.setScalar(1.8); mid.position.y = 6.0; g.add(mid);
  const head = new THREE.Mesh(ST_GEO.snowBall, snowMat()); head.scale.setScalar(1.3); head.position.y = 8.8; g.add(head);
  // 圍巾（套在中球與頭球之間）
  const scarf = new THREE.Mesh(ST_GEO.scarf, new THREE.MeshLambertMaterial({ color: v.scarf }));
  scarf.position.y = 7.4; scarf.rotation.x = Math.PI / 2; scarf.scale.set(0.9, 0.9, 0.6);
  g.add(scarf);
  // 紅蘿蔔鼻（朝前）
  const nose = new THREE.Mesh(ST_GEO.nose, new THREE.MeshLambertMaterial({ color: 0xe8862f }));
  nose.position.set(0, 8.8, 1.3); nose.rotation.x = Math.PI / 2;
  g.add(nose);
  // 頭頂表情 sprite（固定一張，依 style）
  const faceSprite = makeEmojiSprite(3.4);
  faceSprite.position.set(0, 12.5, 0);
  g.add(faceSprite);
  setSpriteEmoji(faceSprite, v.face);
  faceSprite.visible = true;
  // 署名標籤（堆雪人的人）
  if (v.builder) g.add(makeLabel(v.builder));
  // 愛心數 sprite（有讚才現身）
  const cheerSprite = makeEmojiSprite(3.0);
  cheerSprite.position.set(3.4, 11, 0);
  g.add(cheerSprite);
  g.userData.faceSprite = faceSprite;
  g.userData.cheerSprite = cheerSprite;
  return g;
}

// 固定地標的 reconcile：以 id 為 key、位置一次定位（讀 wx/wy，非 x/y）、只走 AOI 淡入淡出。
// update(g, item) 在每次收到快照時呼叫，讓地標即時反映進度／圍爐人數等變化。
function reconcileStatic(list, map, prefix, create, update, recvT) {
  const seen = new Set();
  if (Array.isArray(list)) {
    for (const item of list) {
      try {
        if (!item || typeof item !== "object") continue;
        if (!Number.isFinite(item.wx) || !Number.isFinite(item.wy)) continue;
        const key = prefix + (item.id != null ? item.id : Math.round(item.wx) + "," + Math.round(item.wy));
        seen.add(key);
        let g = map.get(key);
        if (!g) {
          g = create(item); map.set(key, g);
          g.position.set(sx(item.wx), 0, sz(item.wy));
          g.userData.fade = 0; g.userData.fadeTarget = 1; g.userData.removing = false;
        } else if (g.userData.removing) {
          g.userData.removing = false; g.userData.fadeTarget = 1;
        }
        g.userData.item = item;
        if (update) update(g, item);
      } catch (e) {
        console.warn("reconcileStatic 單筆失敗，已略過", e);
      }
    }
  }
  // 沒在這份快照出現的 → 淡出移除（AOI 邊緣不啪一下消失）。
  for (const [key, g] of map) {
    if (!seen.has(key) && !g.userData.removing) { g.userData.removing = true; g.userData.fadeTarget = 0; }
  }
}

const campfires = new Map();
const watchtowers = new Map();
const snowmen = new Map();
const villageWells = new Map(); // 故鄉古井（ROADMAP 640）：單一固定設施，仍走 reconcileStatic 的 AOI 淡入淡出
const villageTeaStalls = new Map(); // 故鄉茶棚（ROADMAP 641）：單一固定設施，仍走 reconcileStatic 的 AOI 淡入淡出
const residentHomes = new Map();    // 居民木屋（ROADMAP 642）：居民之禱而生的溫暖木屋列表
const harvestFestivals = new Map(); // 豐收節慶典（ROADMAP 646）：廣場彩旗慶典裝飾
const fieldSprings = new Map();     // 田邊清泉（ROADMAP 647）：應諾娃之禱，農田北坡天然湧泉
const streetLamps  = new Map();     // 故鄉街燈（ROADMAP 648）：應露娜之禱，城鎮街道七盞夜燈

// 每幀更新所有人造地標：篝火火焰跳動／暖圈脈動（入夜更亮、將熄漸弱）、塔頂燈入夜亮起、雪人愛心數。
// 在各自 Map 的 updateFade 之後跑：updateFade 把所有材質 opacity 壓成 AOI 淡入淡出值，這裡再
// 乘上各自的呈現強度覆寫回去（火焰透明度、暖圈淡度、燈亮度、愛心顯示），故 AOI 仍生效又不被壓掉。
function updateStructures(dt, t) {
  const night = nightFactor();
  // ── 篝火 ──
  for (const [key, g] of campfires) {
    const fade = g.userData.fade ?? 1;
    if (updateFade(g, dt)) { scene.remove(g); campfires.delete(key); continue; }
    const v = campfireVisual(g.userData.item);
    const flame = g.userData.flame, ember = g.userData.ember, glow = g.userData.glow;
    // 將熄的火整體變淡
    const dim = v.dying ? 0.45 : 1;
    // 火焰跳動：尊重 reduceMotion（靜止時不抖）。圍爐越多火越高。
    const flick = reduceMotion ? 1 : 0.85 + 0.15 * Math.sin(t * 11 + g.position.x);
    if (flame) {
      flame.scale.y = v.blaze * flick;
      flame.material.opacity = fade * dim * (reduceMotion ? 0.92 : 0.78 + 0.22 * Math.abs(Math.sin(t * 9)));
    }
    if (ember) {
      ember.scale.y = v.blaze * (reduceMotion ? 1 : 0.9 + 0.2 * Math.sin(t * 14 + 1.3));
      ember.material.opacity = fade * dim;
    }
    if (glow) {
      // 暖意圈：入夜更顯（夜裡的火最暖）、將熄漸弱、輕緩脈動
      const pulse = reduceMotion ? 1 : 0.85 + 0.15 * Math.sin(t * 3 + g.position.z);
      glow.material.opacity = fade * dim * (0.10 + 0.22 * night) * pulse;
    }
  }
  // ── 瞭望塔 ──
  for (const [key, g] of watchtowers) {
    const fade = g.userData.fade ?? 1;
    if (updateFade(g, dt)) { scene.remove(g); watchtowers.delete(key); continue; }
    const v = watchtowerVisual(g.userData.item);
    const beacon = g.userData.beacon;
    if (beacon) {
      // 塔頂燈：只在落成後、且入夜才亮（白天落成的塔不點燈），輕緩脈動如守夜的火光
      const lit = v.done ? night : 0;
      const pulse = reduceMotion ? 1 : 0.8 + 0.2 * Math.sin(t * 2.5 + g.position.x);
      beacon.material.opacity = fade * lit * pulse;
      beacon.visible = lit > 0.02;
    }
  }
  // ── 雪人 ──
  for (const [key, g] of snowmen) {
    const fade = g.userData.fade ?? 1;
    if (updateFade(g, dt)) { scene.remove(g); snowmen.delete(key); continue; }
    const v = snowmanVisual(g.userData.item);
    const face = g.userData.faceSprite;
    if (face) face.material.opacity = fade; // updateFade 已設成 fade，這裡確保表情一直看得見
    const cheer = g.userData.cheerSprite;
    if (cheer) {
      if (v.cheers > 0) {
        // 有人讚過 → 顯示 ♥（emoji sprite 只在改變時換貼圖）
        if (setSpriteEmoji(cheer, "❤️")) {
          cheer.visible = true;
          // 愛心輕輕脈動，呼應 2D 雪人讚賞
          const pulse = reduceMotion ? 1 : 0.72 + 0.28 * Math.abs(Math.sin(t * 3));
          cheer.material.opacity = fade * pulse;
        }
      } else {
        cheer.visible = false;
      }
    }
  }
  // ── 故鄉古井（ROADMAP 640）：水面微微盪漾、汲水時井口擴散水波 ──
  for (const [key, g] of villageWells) {
    const fade = g.userData.fade ?? 1;
    if (updateFade(g, dt)) { scene.remove(g); villageWells.delete(key); continue; }
    const item = g.userData.item || {};
    const water = g.userData.water, ripple = g.userData.ripple;
    // 水面：靜時穩定半透，不抖時也一直看得見（updateFade 已壓成 fade，這裡覆寫回呈現值）。
    if (water) water.material.opacity = fade * (reduceMotion ? 0.85 : 0.78 + 0.12 * Math.abs(Math.sin(t * 1.5)));
    // 汲水水波：watering 時環向外擴散＋淡出（循環），平時隱形。尊重 reduceMotion（靜止時只定格淡顯）。
    if (ripple) {
      if (item.watering === true) {
        if (reduceMotion) {
          ripple.scale.set(2.4, 2.4, 1);
          ripple.material.opacity = fade * 0.5;
        } else {
          const phase = (t * 0.8) % 1;          // 0→1 循環
          const scl = 1 + phase * 2.6;           // 由小擴大
          ripple.scale.set(scl, scl, 1);
          ripple.material.opacity = fade * 0.6 * (1 - phase); // 越擴越淡
        }
        ripple.visible = true;
      } else {
        ripple.visible = false;
      }
    }
  }
  // ── 居民木屋（ROADMAP 642）：夜晚窗光緩緩亮起——「有人在家」的溫暖感 ──
  for (const [key, g] of residentHomes) {
    if (updateFade(g, dt)) { scene.remove(g); residentHomes.delete(key); continue; }
    const fade = g.userData.fade ?? 1;
    const winGlow = g.userData.winGlow;
    if (winGlow) {
      // 白天窗光很淡（0.1），入夜漸亮到 0.7；reduceMotion 時直接取夜晚值不呼吸
      const baseGlow = 0.1 + night * 0.6;
      winGlow.material.opacity = fade * (reduceMotion
        ? baseGlow
        : baseGlow * (0.9 + 0.1 * Math.abs(Math.sin(t * 0.7)))); // 極輕微的呼吸感
    }
  }
  // ── 故鄉茶棚（ROADMAP 641）：暖燈微亮、出爐時棚上嫋嫋蒸汽升起＋淡出 ──
  for (const [key, g] of villageTeaStalls) {
    const fade = g.userData.fade ?? 1;
    if (updateFade(g, dt)) { scene.remove(g); villageTeaStalls.delete(key); continue; }
    const item = g.userData.item || {};
    const lantern = g.userData.lantern, steam = g.userData.steam;
    // 暖燈：靜時穩定亮，不抖時輕微呼吸（updateFade 已壓成 fade，這裡覆寫回呈現值）。
    if (lantern) lantern.material.opacity = fade * (reduceMotion ? 0.9 : 0.82 + 0.12 * Math.abs(Math.sin(t * 1.3)));
    // 出爐蒸汽：brewing 時自茶壺嘴上方升起＋淡出（循環），平時隱形。尊重 reduceMotion（靜止時只定格淡顯）。
    if (steam) {
      if (item.brewing === true) {
        if (reduceMotion) {
          steam.position.y = 2.3;
          steam.scale.set(1.1, 1.1, 1.1);
          steam.material.opacity = fade * 0.45;
        } else {
          const phase = (t * 0.6) % 1;            // 0→1 循環
          steam.position.y = 1.9 + phase * 1.2;    // 自壺嘴上方緩緩升起
          const scl = 0.7 + phase * 0.9;           // 越升越散
          steam.scale.set(scl, scl, scl);
          steam.material.opacity = fade * 0.55 * (1 - phase); // 越升越淡
        }
        steam.visible = true;
      } else {
        steam.visible = false;
      }
    }
  }
  // ── 豐收節慶典（ROADMAP 646）：active 時旗幟搖擺、燈籠發光搖曳 ──
  for (const [key, g] of harvestFestivals) {
    const fade = g.userData.fade ?? 1;
    if (updateFade(g, dt)) { scene.remove(g); harvestFestivals.delete(key); continue; }
    const item = g.userData.item || {};
    const active = item.active === true;
    const lanterns = g.userData.lanterns || [];
    // 燈籠：慶典中發光搖曳；平時暗淡靜止（提示有慶典裝置，但未啟動）。
    for (let i = 0; i < lanterns.length; i++) {
      const lan = lanterns[i];
      if (active) {
        const swing = reduceMotion ? 1 : 0.85 + 0.15 * Math.sin(t * 1.8 + i * 1.3);
        lan.material.opacity = fade * 0.82 * swing;
        if (!reduceMotion) {
          lan.position.y = 5.0 + 0.12 * Math.sin(t * 1.5 + i * 0.9); // 輕微搖曳
        }
      } else {
        lan.material.opacity = fade * 0.18; // 平時只有一點暖光，告訴玩家「這裡偶爾有慶典」
        lan.position.y = 5.0;
      }
    }
    // 旗幟：慶典中依時間位移製造迎風飄揚感；尊重 reduceMotion。
    const flags = g.userData.flags || [];
    for (const { mesh, baseY, side } of flags) {
      if (active && !reduceMotion) {
        mesh.rotation.z = (side > 0 ? -1 : 1) * (Math.PI / 2 + 0.18 * Math.sin(t * 2.2 + baseY));
      }
      // 旗子本身不透明，不需要 opacity 控制
    }
    // 橫幅：慶典中略微閃亮（輕微材質 emissive 感，但 Lambert 無 emissive，改做透明層閃動）
    const banner = g.userData.banner;
    if (banner) {
      // 橫幅顏色不透明，只讓 fade 帶入（入場/退場的淡入淡出）
      banner.material.opacity = fade; // 實際 Lambert 不需 opacity，保持不透明
    }
  }
  // ── 田邊清泉（ROADMAP 647）：漣漪環自泉眼持續向外擴散＋湧泉水泡緩緩浮起消散 ──
  for (const [key, g] of fieldSprings) {
    const fade = g.userData.fade ?? 1;
    if (updateFade(g, dt)) { scene.remove(g); fieldSprings.delete(key); continue; }
    const water = g.userData.water, ripple = g.userData.ripple, bubble = g.userData.bubble;
    // 水面：微弱呼吸感（清泉水面比井水更活潑）；reduceMotion 靜態。
    if (water) {
      water.material.opacity = fade * (reduceMotion ? 0.82 : 0.72 + 0.10 * Math.abs(Math.sin(t * 1.1)));
    }
    // 漣漪環：從泉眼中心向外循環擴散，越外越淡（phase 0→1 一圈）。
    if (ripple) {
      if (!reduceMotion) {
        const phase = (t * 0.45) % 1;                     // 擴散速度略比古井慢，更「天然常流」
        const s = 1 + phase * 3.0;                        // 從 1× 擴到 4× 倍（清泉漣漪更廣）
        ripple.scale.set(s, s, 1);
        ripple.material.opacity = fade * 0.65 * (1 - phase); // 越外越淡
      } else {
        ripple.scale.set(2.2, 2.2, 1);
        ripple.material.opacity = fade * 0.18;
      }
    }
    // 湧泉水泡：從水面緩緩浮起再消散（清泉特有的鮮活感，尊重 reduceMotion）。
    if (bubble) {
      if (!reduceMotion) {
        const bp = (t * 0.28 + 0.3) % 1;                 // 比 phase 慢一點，和漣漪不同步
        bubble.position.y = 0.57 + bp * 0.55;            // 緩緩上浮
        bubble.material.opacity = fade * 0.60 * (1 - bp * 0.75); // 越升越淡
      } else {
        bubble.position.y = 0.68;
        bubble.material.opacity = fade * 0.28;
      }
    }
  }
  // ── 故鄉街燈（ROADMAP 648）：入夜亮起——燈球從半透變亮、地面琥珀光暈從無到顯 ──
  for (const [key, g] of streetLamps) {
    const fade = g.userData.fade ?? 1;
    if (updateFade(g, dt)) { scene.remove(g); streetLamps.delete(key); continue; }
    const globe = g.userData.globe, glow = g.userData.glow;
    if (globe) {
      // 燈球：白天淡顯（0.3），入夜漸亮到 0.95；夜間加極輕微脈動，如蠟燭微顫。
      const baseOpacity = 0.3 + night * 0.65;
      globe.material.opacity = fade * (reduceMotion
        ? baseOpacity
        : baseOpacity * (0.92 + 0.08 * Math.abs(Math.sin(t * 1.6 + g.position.x * 0.3))));
    }
    if (glow) {
      // 地面光暈：白天不顯（0），入夜漸亮（最亮 0.45）；夜間輕緩脈動——入夜最溫暖。
      const baseGlow = night * 0.45;
      glow.material.opacity = fade * (reduceMotion
        ? baseGlow
        : baseGlow * (0.85 + 0.15 * Math.abs(Math.sin(t * 1.1 + g.position.z * 0.2))));
    }
  }
}

// ============================================================
// 玩家親手種下的世界樹群在 3D 裡長大（ROADMAP 617）：把快照裡早就有、2D 一直看得到、3D 卻整個忽略的
// `world_groves`（TreeView）接進 3D——全服共享、隨真實時間一階階長大的樹。讓「這個世界是大家一起種出來的」
// 一眼看得見：今天一株嫩芽 🌱，過些時候回來，它已抽成幼樹、長成松、最後撐成能遮蔭的成樹 🌳。
//   · 階段（與後端 world_grove::GrowStage.wire 對齊、別重排）：0🌱嫩芽 / 1🌿幼苗 / 2🌲幼樹 / 3🌳成樹。
//   · 成樹（🌳）在腳邊投下一汪清涼樹蔭——呼應林蔭小憩（ROADMAP 467，後端在樹蔭下加速脫戰回血），
//     讓玩家一眼看出「這棵社群種大的樹能遮蔭、能在底下歇腳」。半徑鏡像 2D GROVE_SHADE_RADIUS。
// 純讀快照、零後端改動、零協議改動——資料本來就在 TreeView 裡（2D game.js drawWorldGroves 早在用）。
// 樹是「不會走動的固定地景」、但**會隨時間換階段**，故以座標當 key、stage 變了才重塑樹身（rebuild），
// 平時只走 AOI 淡入淡出 + 成樹的微風輕擺（皆尊重 reduceMotion）。
// ============================================================

const GROVE_TRUNK_COLOR = 0x6b4a2f;        // 樹幹：深木褐
// 各階段樹冠配色：嫩芽鮮嫩 → 成樹沉綠（越長越深，成長一眼可讀）。
const GROVE_LEAF_SPROUT = 0x8ed27a;        // 嫩芽：鮮嫩黃綠
const GROVE_LEAF_SAPLING = 0x6cbf5a;       // 幼苗：嫩綠
const GROVE_LEAF_PINE = 0x2f7d32;          // 幼樹松：標準林綠（對齊採集樹節點 makeNode）
const GROVE_LEAF_MATURE = 0x276b2b;        // 成樹：沉穩深綠
const GROVE_SHADE_COLOR = 0x3f6b3a;        // 樹蔭：清涼草綠（鏡像 2D）
// 成樹樹蔭半徑（世界像素，鏡像 2D GROVE_SHADE_RADIUS=44，與後端 world_grove::SHADE_RADIUS 同契約）。
const GROVE_SHADE_RADIUS_PX = 44;

// 世界樹視覺（純函式）：成長階段 → 樹幹高、樹冠尺寸、松型/圓冠、是否成樹遮蔭、是否隨風擺。
// 只讀權威 stage（夾 0..3）、確定性、壞值安全；前端據此把同一棵樹塑成對應階段身形。
function groveVisual(item) {
  const raw = item && typeof item === "object" && Number.isFinite(item.stage) ? Math.floor(item.stage) : 0;
  const stage = raw < 0 ? 0 : raw > 3 ? 3 : raw;
  // 樹幹高（場景單位）：嫩芽幾乎無幹、越長越高（單調遞增）。
  const trunkH = [0.0, 2.4, 4.8, 7.6][stage];
  // 樹冠尺寸倍率：成樹樹冠最闊。
  const crownScale = [0.5, 0.95, 1.15, 1.65][stage];
  return {
    stage,
    trunkH,
    crownScale,
    sprout: stage === 0,   // 嫩芽：貼地小葉簇，無幹無冠
    pine: stage === 2,     // 幼樹：松型尖冠；其餘有冠階段用圓冠
    shade: stage === 3,    // 只有成樹投樹蔭（與林蔭小憩契約一致）
    sway: stage >= 2,      // 幼樹／成樹隨風輕擺；嫩芽幼苗太小不擺
  };
}

// 共用幾何（全模組只建一次，整片林子也不重建頂點）。材質一律「每棵一份」——AOI 淡入淡出
// 才能各自獨立調 opacity，不牽連同階段的別棵。
const GROVE_GEO = {
  trunk:  new THREE.CylinderGeometry(0.42, 0.6, 1, 6),   // 樹幹（高度由 scale.y 撐）
  round:  new THREE.SphereGeometry(2.6, 9, 7),           // 圓冠（幼苗／成樹，尺寸由 scale 撐）
  pine:   new THREE.ConeGeometry(2.4, 4.4, 7),           // 松型尖冠（幼樹，雙層）
  leaf:   new THREE.ConeGeometry(0.5, 1.7, 5),           // 嫩芽小葉
  shade:  new THREE.CylinderGeometry(1, 1, 0.16, 22),    // 樹蔭圓盤（鋪地，半徑由 scale 撐）
};

// 一棵世界樹：含「全階段」零件（樹幹／圓冠／松冠×2／嫩芽小葉×3／樹蔭圓盤），由 applyGroveStage
// 依當前階段切換可見性與尺寸——這樣樹長大時不必 dispose／重建幾何，只切顯隱＋縮放。
function makeGroveTree(item) {
  const g = new THREE.Group();
  // 樹幹（高度由 applyGroveStage 撐）。
  const trunk = new THREE.Mesh(GROVE_GEO.trunk, new THREE.MeshLambertMaterial({ color: GROVE_TRUNK_COLOR }));
  g.add(trunk);
  // 圓冠：幼苗（嫩綠）與成樹（深綠）各一顆，依階段切顯隱（不在執行期改色，貼齊測試 mock 行為）。
  const roundSap = new THREE.Mesh(GROVE_GEO.round, new THREE.MeshLambertMaterial({ color: GROVE_LEAF_SAPLING }));
  const roundMat = new THREE.Mesh(GROVE_GEO.round, new THREE.MeshLambertMaterial({ color: GROVE_LEAF_MATURE }));
  g.add(roundSap); g.add(roundMat);
  // 松型尖冠（幼樹，雙層尖塔）。
  const pineLo = new THREE.Mesh(GROVE_GEO.pine, new THREE.MeshLambertMaterial({ color: GROVE_LEAF_PINE }));
  const pineHi = new THREE.Mesh(GROVE_GEO.pine, new THREE.MeshLambertMaterial({ color: GROVE_LEAF_PINE }));
  pineHi.scale.setScalar(0.66);
  g.add(pineLo); g.add(pineHi);
  // 嫩芽小葉簇（三片小葉外張，貼地）。
  const sprout = new THREE.Group();
  for (let i = 0; i < 3; i++) {
    const leaf = new THREE.Mesh(GROVE_GEO.leaf, new THREE.MeshLambertMaterial({ color: GROVE_LEAF_SPROUT }));
    const ang = (i / 3) * Math.PI * 2;
    leaf.position.set(Math.cos(ang) * 0.35, 0.9, Math.sin(ang) * 0.35);
    leaf.rotation.z = Math.cos(ang) * 0.4; leaf.rotation.x = -Math.sin(ang) * 0.4;
    sprout.add(leaf);
  }
  g.add(sprout);
  // 樹蔭圓盤（只有成樹現身；半徑＝GROVE_SHADE_RADIUS_PX×WORLD_SCALE）。
  const shade = new THREE.Mesh(GROVE_GEO.shade, new THREE.MeshBasicMaterial({ color: GROVE_SHADE_COLOR, transparent: true, opacity: 0.16, depthWrite: false }));
  const sr = Math.max(1.2, GROVE_SHADE_RADIUS_PX * WORLD_SCALE);
  shade.scale.set(sr, 1, sr);
  shade.position.y = 0.1;
  g.add(shade);
  g.userData.trunk = trunk;
  g.userData.roundSap = roundSap;
  g.userData.roundMat = roundMat;
  g.userData.pineLo = pineLo;
  g.userData.pineHi = pineHi;
  g.userData.sprout = sprout;
  g.userData.shade = shade;
  // 整棵樹隨機朝向，整片林子才不會同手同腳（用座標當相位，確定性、不每幀亂跳）。
  g.userData.phase = (Number.isFinite(item && item.x) ? item.x : 0) + (Number.isFinite(item && item.y) ? item.y : 0);
  applyGroveStage(g, groveVisual(item));
  return g;
}

// 依成長階段把一棵樹塑成對應身形：撐樹幹高、切換圓冠／松冠／嫩芽可見性與尺寸、成樹才現樹蔭。
function applyGroveStage(g, v) {
  const u = g.userData;
  const h = v.trunkH;
  // 樹幹：嫩芽階段近乎無幹（藏起）；有幹階段把高度撐到 h、底部貼地。
  if (h > 0.01) {
    u.trunk.visible = true;
    u.trunk.scale.y = h; u.trunk.position.y = h / 2;
  } else {
    u.trunk.visible = false;
  }
  // 嫩芽小葉：只在 stage 0 現身（貼地）。
  u.sprout.visible = v.sprout;
  // 圓冠：幼苗（stage1·嫩綠）與成樹（stage3·深綠）各用一顆，縮放＋座落樹幹頂；幼樹／嫩芽不現圓冠。
  const wantRound = !v.sprout && !v.pine;
  const mature = v.stage >= 3;
  const round = mature ? u.roundMat : u.roundSap;
  const other = mature ? u.roundSap : u.roundMat;
  other.visible = false;
  if (wantRound) {
    round.visible = true;
    round.scale.setScalar(v.crownScale);
    round.position.y = h + 2.4 * v.crownScale * 0.5;
  } else {
    round.visible = false;
  }
  // 松冠：只在幼樹（stage2）現身，雙層尖塔疊在樹幹頂。
  if (v.pine) {
    u.pineLo.visible = true; u.pineHi.visible = true;
    u.pineLo.position.y = h + 2.0; u.pineHi.position.y = h + 4.0;
  } else {
    u.pineLo.visible = false; u.pineHi.visible = false;
  }
  // 樹蔭：只有成樹現身。
  u.shade.visible = v.shade;
  u.userData_h = h;
}

const worldGroves = new Map(); // key 用座標字串（樹無穩定 id）

// 世界樹群 reconcile：以座標當 key（樹位置固定）、stage 變了才重塑樹身、AOI 淡入淡出。
// 走跟農地一樣的固定地景模式（位置一次定位、不做內插/轉身）。
function reconcileGroves(list, recvT) {
  const seen = new Set();
  if (Array.isArray(list)) {
    for (const item of list) {
      try {
        if (!item || typeof item !== "object") continue;
        if (!Number.isFinite(item.x) || !Number.isFinite(item.y)) continue;
        const key = "g" + Math.round(item.x) + "," + Math.round(item.y);
        seen.add(key);
        let g = worldGroves.get(key);
        if (!g) {
          g = makeGroveTree(item);
          worldGroves.set(key, g);
          g.position.set(sx(item.x), 0, sz(item.y));
          g.userData.fade = 0; g.userData.fadeTarget = 1; g.userData.removing = false;
          g.userData.stage = groveVisual(item).stage;
          scene.add(g);
        } else {
          if (g.userData.removing) { g.userData.removing = false; g.userData.fadeTarget = 1; }
          // 長大了（階段變動）→ 重塑樹身（只切顯隱＋縮放，不重建幾何）。
          const st = groveVisual(item).stage;
          if (st !== g.userData.stage) { applyGroveStage(g, groveVisual(item)); g.userData.stage = st; }
        }
        g.userData.item = item;
      } catch (e) {
        console.warn("reconcileGroves 單筆失敗，已略過", e);
      }
    }
  }
  // 沒在這份快照出現的樹 → 淡出移除（AOI 邊緣不啪一下消失）。
  for (const [key, g] of worldGroves) {
    if (!seen.has(key) && !g.userData.removing) { g.userData.removing = true; g.userData.fadeTarget = 0; }
  }
}

// 每幀更新世界樹群：幼樹／成樹隨風輕擺（reduceMotion 下不擺）、AOI 淡入淡出。
function updateGroves(dt, t) {
  for (const [key, g] of worldGroves) {
    if (!reduceMotion) {
      const v = groveVisual(g.userData.item || { stage: g.userData.stage });
      g.rotation.z = v.sway ? Math.sin(t * 1.2 + g.userData.phase * 0.05) * (v.stage >= 3 ? 0.05 : 0.035) : 0;
    } else {
      g.rotation.z = 0;
    }
    if (updateFade(g, dt)) { scene.remove(g); worldGroves.delete(key); }
  }
}

// 視野內世界樹群的 HUD 標籤：幾棵樹／其中幾棵已長成成樹（🌳）。純函式、壞值安全；無樹則回空字串。
function groveHudLabel(groves) {
  if (!Array.isArray(groves) || !groves.length) return "";
  let mature = 0;
  for (const g of groves) if (g && (g.stage | 0) === 3) mature++;
  return mature > 0 ? `🌳 樹 ${groves.length}（成樹 ${mature}）` : `🌱 樹 ${groves.length}`;
}

// ============================================================
// 重大世界事件在 3D 裡現形（ROADMAP 631）：世界導演每隔幾分鐘就上演的兩樁大事——
// 宇宙裂縫（world_event，ROADMAP 26：在城外某座標撕開裂縫、注入最強的裂縫守護者）與
// 獸潮攻城（horde_event，ROADMAP 44：生態壓力到臨界，挑一座城門廣播倒數→攻城）——
// 後端早就隨快照下傳權威座標／倒數／階段（2D 一直用小地圖標記＋橫幅演出），3D 卻把這
// 兩個 optional 欄位整個丟掉：裂縫開了看不見、獸潮逼近毫無預警。本切片把它們接成「一眼
// 望去就知道世界出大事了」的醒目地標：裂縫＝次元紫光柱＋旋轉能量環；獸潮＝城門警示光束
// ＋浮空地名牌（逼近琥珀、攻城赤紅）。純讀既有欄位，缺欄／NaN／非物件一律保守、永不拋。
// ============================================================
const RIFT_COLOR = 0xb060ff;                                    // 裂縫光柱：次元紫
const RIFT_CORE_COLOR = 0xe6c2ff;                               // 裂縫亮心：更亮的淡紫
const HORDE_COLOR = { announcing: 0xffb030, sieging: 0xff3020 }; // 逼近琥珀／攻城赤紅

// ── 純函式（供 render-smoke 斷言；確定性、壞值安全、永不拋）─────────────────
// 宇宙裂縫顯影參數：缺有限座標一律 active:false（優雅無事件）。remainingSecs clamp ≥0。
function riftVisual(ev) {
  if (!ev || typeof ev !== "object" || !Number.isFinite(ev.x) || !Number.isFinite(ev.y)) {
    return { active: false, remainingSecs: 0, x: 0, y: 0 };
  }
  const rem = Number.isFinite(ev.remaining_secs) && ev.remaining_secs > 0 ? ev.remaining_secs : 0;
  return { active: true, remainingSecs: rem, x: ev.x, y: ev.y };
}
// 裂縫 HUD 文字行（無事件回空字串；倒數無條件進位整秒，缺倒數只報已開啟）。
function riftHudLabel(ev) {
  const v = riftVisual(ev);
  if (!v.active) return "";
  const s = Math.ceil(v.remainingSecs);
  return s > 0 ? `🌀 宇宙裂縫已開啟 · 約 ${s}s` : "🌀 宇宙裂縫已開啟";
}
// 獸潮攻城顯影參數：phase 正規化（"sieging"→攻城、其餘→"announcing"逼近）；缺有限座標
// 一律 active:false；secsLeft clamp ≥0；label 壞值回空字串。
function hordeVisual(ev) {
  if (!ev || typeof ev !== "object" || !Number.isFinite(ev.site_x) || !Number.isFinite(ev.site_y)) {
    return { active: false, phase: "announcing", secsLeft: 0, x: 0, y: 0, label: "" };
  }
  const phase = ev.phase === "sieging" ? "sieging" : "announcing";
  const secs = Number.isFinite(ev.secs_left) && ev.secs_left > 0 ? ev.secs_left : 0;
  const label = typeof ev.site_label === "string" ? ev.site_label : "";
  return { active: true, phase, secsLeft: secs, x: ev.site_x, y: ev.site_y, label };
}
// 獸潮 HUD 文字行（無事件回空字串；逼近／攻城兩階段不同字樣與圖示）。
function hordeHudLabel(ev) {
  const v = hordeVisual(ev);
  if (!v.active) return "";
  const name = v.label || "城門";
  const s = Math.ceil(v.secsLeft);
  const tail = s > 0 ? ` · ${s}s` : "";
  return v.phase === "sieging" ? `⚔️ 獸潮攻城·${name}${tail}` : `⚠️ 獸潮逼近·${name}${tail}`;
}

// ── 程序化地標（零美術資產；單例，懶建一次、之後切 .visible）─────────────────
// 宇宙裂縫光柱：一道次元紫發光能量柱＋亮心＋基座兩道反向旋轉的能量環。MeshBasicMaterial
// 不受光＝永遠自發亮，夜裡也醒目；半透明疊出光霧感。userData 留引用供每幀脈動。
function makeRiftPortal() {
  const g = new THREE.Group();
  const column = new THREE.Mesh(
    new THREE.CylinderGeometry(3.2, 1.2, 64, 10),
    new THREE.MeshBasicMaterial({ color: RIFT_COLOR, transparent: true, opacity: 0.3, depthWrite: false })
  );
  column.position.y = 32;
  g.add(column);
  const core = new THREE.Mesh(
    new THREE.CylinderGeometry(0.9, 0.4, 66, 8),
    new THREE.MeshBasicMaterial({ color: RIFT_CORE_COLOR, transparent: true, opacity: 0.6, depthWrite: false })
  );
  core.position.y = 33;
  g.add(core);
  const ring = new THREE.Mesh(
    new THREE.TorusGeometry(5, 0.5, 8, 24),
    new THREE.MeshBasicMaterial({ color: RIFT_COLOR, transparent: true, opacity: 0.7, depthWrite: false })
  );
  ring.rotation.x = Math.PI / 2; ring.position.y = 1.5;
  g.add(ring);
  const ring2 = new THREE.Mesh(
    new THREE.TorusGeometry(3, 0.35, 8, 20),
    new THREE.MeshBasicMaterial({ color: RIFT_CORE_COLOR, transparent: true, opacity: 0.6, depthWrite: false })
  );
  ring2.rotation.x = Math.PI / 2; ring2.position.y = 13;
  g.add(ring2);
  g.userData.column = column; g.userData.core = core; g.userData.ring = ring; g.userData.ring2 = ring2;
  scene.add(g);
  g.visible = false;
  return g;
}

// 獸潮警示光束：一道警示色光柱＋地面警示圈＋浮空地名牌。顏色隨階段（逼近琥珀／攻城赤紅）
// 由 updateWorldEvents 每幀更新；地名牌變了才重建貼圖。色材質用 THREE.Color 包好（測試假
// THREE 才有 setHex）。
function makeHordeBeacon() {
  const g = new THREE.Group();
  const beam = new THREE.Mesh(
    new THREE.CylinderGeometry(2.6, 1.4, 48, 8),
    new THREE.MeshBasicMaterial({ color: new THREE.Color(HORDE_COLOR.announcing), transparent: true, opacity: 0.32, depthWrite: false })
  );
  beam.position.y = 24;
  g.add(beam);
  const disc = new THREE.Mesh(
    new THREE.CylinderGeometry(8, 8, 0.3, 28),
    new THREE.MeshBasicMaterial({ color: new THREE.Color(HORDE_COLOR.announcing), transparent: true, opacity: 0.25, depthWrite: false })
  );
  disc.position.y = 0.15;
  g.add(disc);
  g.userData.beam = beam; g.userData.disc = disc;
  g.userData.label = null; g.userData.labelText = null; // 地名牌懶建（首次有 label 才生）
  scene.add(g);
  g.visible = false;
  return g;
}

// 重建獸潮地名牌（地名變了才呼叫，避免每幀重生貼圖）。沿用 makeLabel 的白字描邊牌，
// 抬高到光束頂、放大成醒目橫牌。
function rebuildHordeLabel(marker, text) {
  const old = marker.userData.label;
  if (old) {
    marker.remove(old);
    if (old.material && old.material.map && typeof old.material.map.dispose === "function") old.material.map.dispose();
  }
  const label = makeLabel(text || "獸潮");
  label.position.y = 42;
  label.scale.set(30, 7.5, 1);
  marker.add(label);
  marker.userData.label = label;
  marker.userData.labelText = text;
}

// 最新一筆世界事件快照（非物件＝null＝無事件）。單例地標懶建，事件消失即隱藏。
let latestRift = null;
let latestHorde = null;
let riftMarker = null;
let hordeMarker = null;

// 每幀更新世界事件地標：吸到事件權威座標（sx/sz）、脈動光效、地名變了才重建牌面、
// 事件消失就隱藏。無事件時連 mesh 都不建（懶建），對壞值一律安全。
function updateWorldEvents(dt, t) {
  // 宇宙裂縫
  const rv = riftVisual(latestRift);
  if (rv.active) {
    if (!riftMarker) riftMarker = makeRiftPortal();
    riftMarker.visible = true;
    riftMarker.position.x = sx(rv.x);
    riftMarker.position.z = sz(rv.y);
    const pulse = 0.5 + 0.5 * Math.sin(t * 2.2);
    const ud = riftMarker.userData;
    if (ud.column && ud.column.material) ud.column.material.opacity = 0.24 + 0.14 * pulse;
    if (ud.core && ud.core.material) ud.core.material.opacity = 0.45 + 0.25 * pulse;
    if (ud.ring) ud.ring.rotation.y = t * 0.8;
    if (ud.ring2) ud.ring2.rotation.y = -t * 0.6;
  } else if (riftMarker) {
    riftMarker.visible = false;
  }
  // 獸潮攻城
  const hv = hordeVisual(latestHorde);
  if (hv.active) {
    if (!hordeMarker) hordeMarker = makeHordeBeacon();
    hordeMarker.visible = true;
    hordeMarker.position.x = sx(hv.x);
    hordeMarker.position.z = sz(hv.y);
    const col = HORDE_COLOR[hv.phase] || HORDE_COLOR.announcing;
    const ud = hordeMarker.userData;
    if (ud.beam && ud.beam.material && ud.beam.material.color) ud.beam.material.color.setHex(col);
    if (ud.disc && ud.disc.material && ud.disc.material.color) ud.disc.material.color.setHex(col);
    // 攻城期脈動更急更亮，傳達迫近感
    const rate = hv.phase === "sieging" ? 5.0 : 2.0;
    const amp = hv.phase === "sieging" ? 0.3 : 0.16;
    const pulse = 0.5 + 0.5 * Math.sin(t * rate);
    if (ud.beam && ud.beam.material) ud.beam.material.opacity = 0.28 + amp * pulse;
    if (ud.disc && ud.disc.material) ud.disc.material.opacity = 0.2 + 0.5 * amp * pulse;
    if (ud.labelText !== hv.label) rebuildHordeLabel(hordeMarker, hv.label);
  } else if (hordeMarker) {
    hordeMarker.visible = false;
  }
}

// ============================================================
// 探索羅盤雷達（ROADMAP 633）
// ============================================================
// 3D 世界一直沒有任何導航：玩家不知道家在哪、附近有沒有居民／敵人、世界大事（裂縫／獸潮）
// 在哪個方位該往哪奔。AI 居民反覆許願「探索沒方向感／找不到目標／不知世界大事在哪」——
// 本切片補上一面「玩家為心、北朝上」的圓形雷達 HUD：純讀既有快照座標（其他玩家／居民／
// 敵人／新手村家／重大世界事件），把每個目標換算成雷達上的方位點；超出範圍者夾到邊緣只指方向，
// 讓「世界出大事了」第一次「看得出在哪、該往哪奔」。零後端改動、零協議改動、純前端呈現。

// 新手村（家）中心，鏡像 world-core SAFE_ZONE_CX/CY（單一真相在 Rust，此處只讀不另算）。
const RADAR_HOME_WX = 2344, RADAR_HOME_WY = 2296;
// 雷達半徑（場景單位；1 場景單位 = 1/WORLD_SCALE = 20 世界像素）。約涵蓋自身周圍 ~1400px。
const RADAR_RANGE = 70;
// 各類目標的雷達配色與點半徑（畫得越晚越上層；ring=true 者加脈動外環更搶眼＝奔赴目標）。
const RADAR_KIND = {
  home:     { color: "#7fd6ff", r: 3.4, ring: false },
  player:   { color: "#4aa3ff", r: 2.6, ring: false },
  resident: { color: "#9be36b", r: 2.6, ring: false },
  enemy:    { color: "#ff5a5a", r: 2.8, ring: false },
  horde:    { color: "#ff8c1a", r: 4.2, ring: true },
  rift:     { color: "#c060ff", r: 4.2, ring: true },
};

// 純函式：把場景座標的目標清單換算成雷達正規座標 [-1,1]（north-up：+x=東/右、+z=南/下）。
// self={x,z}（場景座標）；entities=[{x,z,kind}]；range=雷達邊緣對應的場景半徑。
// 回傳每筆 {nx,ny,kind,edge}：edge=true 表超出範圍、已夾到單位圓邊緣（只剩方向意義）。
// 壞值（缺 self／非有限座標／非陣列）安全略過，永遠回得出陣列（守 render-loop-resilience）。
function radarBlips(self, entities, range) {
  const out = [];
  if (!self || !Number.isFinite(self.x) || !Number.isFinite(self.z)) return out;
  const R = Number.isFinite(range) && range > 0 ? range : 1;
  if (!Array.isArray(entities)) return out;
  for (const e of entities) {
    if (!e || !Number.isFinite(e.x) || !Number.isFinite(e.z)) continue;
    let nx = (e.x - self.x) / R;
    let ny = (e.z - self.z) / R;
    const d = Math.hypot(nx, ny);
    let edge = false;
    if (d > 1) { nx /= d; ny /= d; edge = true; } // 超出範圍 → 夾到單位圓邊緣只指方向
    out.push({ nx, ny, kind: e.kind, edge });
  }
  return out;
}

// 純函式：鏡頭朝向在雷達上的單位向量（north-up：+x=右、+y=下）。鏡頭在玩家後方 (sin,cos)*dist
// 看向玩家，故視線方向 = -(sin camYaw, cos camYaw)。確定性、壞值退回朝上 (0,-1)。
function radarHeading(camYawArg) {
  if (!Number.isFinite(camYawArg)) return { x: 0, y: -1 };
  return { x: -Math.sin(camYawArg), y: -Math.cos(camYawArg) };
}

// 蒐集這一幀要上雷達的目標（場景座標）。只讀既有 live maps／快照衍生，零後端改動。
function collectRadarEntities() {
  const list = [];
  // 其他玩家（排除自己）
  for (const [id, g] of players) { if (id !== myId) list.push({ x: g.position.x, z: g.position.z, kind: "player" }); }
  // 故鄉居民
  for (const [, g] of npcs) list.push({ x: g.position.x, z: g.position.z, kind: "resident" });
  // 敵人（已倒下的不標）
  for (const [, g] of enemies) {
    const it = g.userData && g.userData.item;
    if (it && it.alive === false) continue;
    list.push({ x: g.position.x, z: g.position.z, kind: "enemy" });
  }
  // 新手村家（恆在）
  list.push({ x: sx(RADAR_HOME_WX), z: sz(RADAR_HOME_WY), kind: "home" });
  // 重大世界事件（裂縫／獸潮）——召喚玩家奔赴的目標
  const rv = riftVisual(latestRift);
  if (rv.active) list.push({ x: sx(rv.x), z: sz(rv.y), kind: "rift" });
  const hv = hordeVisual(latestHorde);
  if (hv.active) list.push({ x: sx(hv.x), z: sz(hv.y), kind: "horde" });
  return list;
}

const radarCanvas = document.getElementById("radar");
const radarCtx = radarCanvas ? radarCanvas.getContext("2d") : null;

// 每幀繪製雷達。內部全 try/catch 包覆（呼叫端 safeRender 也有護網，雙保險不凍畫面）。
function drawRadar() {
  if (!radarCtx) return;
  const W = radarCanvas.width, H = radarCanvas.height;
  const cx = W / 2, cy = H / 2, R = W / 2 - 6; // 邊緣留一點內距
  radarCtx.clearRect(0, 0, W, H);
  const me = myId ? players.get(myId) : null;
  if (!me) return; // 還沒在快照裡找到自己 → 留空（不畫殘影）
  const self = { x: me.position.x, z: me.position.z };
  const blips = radarBlips(self, collectRadarEntities(), RADAR_RANGE);
  const now = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
  const pulse = reduceMotion ? 1 : 0.7 + 0.3 * Math.sin(now / 320); // 事件外環脈動

  radarCtx.save();
  // 圓形裁切 + 底盤
  radarCtx.beginPath(); radarCtx.arc(cx, cy, R, 0, Math.PI * 2); radarCtx.clip();
  radarCtx.fillStyle = "rgba(10,14,22,0.35)"; radarCtx.fillRect(0, 0, W, H);
  // 距離環（兩圈淡灰）＋十字方位線
  radarCtx.strokeStyle = "rgba(255,255,255,0.12)"; radarCtx.lineWidth = 1;
  for (const f of [0.5, 1]) { radarCtx.beginPath(); radarCtx.arc(cx, cy, R * f, 0, Math.PI * 2); radarCtx.stroke(); }
  radarCtx.beginPath(); radarCtx.moveTo(cx - R, cy); radarCtx.lineTo(cx + R, cy);
  radarCtx.moveTo(cx, cy - R); radarCtx.lineTo(cx, cy + R); radarCtx.stroke();

  // 鏡頭視野扇形（半透明金）：讓玩家知道「自己正看著哪個方位」
  const hd = radarHeading(camYaw);
  const ang = Math.atan2(hd.y, hd.x);
  const half = 0.42; // 視野扇半角（rad）
  radarCtx.beginPath(); radarCtx.moveTo(cx, cy);
  radarCtx.arc(cx, cy, R * 0.95, ang - half, ang + half);
  radarCtx.closePath();
  radarCtx.fillStyle = "rgba(255,213,74,0.10)"; radarCtx.fill();

  // 目標點
  for (const b of blips) {
    const px = cx + b.nx * R, py = cy + b.ny * R;
    const spec = RADAR_KIND[b.kind] || { color: "#cccccc", r: 2.4, ring: false };
    if (spec.ring) { // 事件：脈動外環，最搶眼
      radarCtx.beginPath(); radarCtx.arc(px, py, spec.r + 3 * pulse, 0, Math.PI * 2);
      radarCtx.strokeStyle = spec.color; radarCtx.globalAlpha = 0.6 * pulse; radarCtx.lineWidth = 1.6; radarCtx.stroke();
      radarCtx.globalAlpha = 1;
    }
    if (b.edge) {
      // 邊緣目標：畫一個指向外的小三角（只剩方向意義）
      const a = Math.atan2(b.ny, b.nx);
      radarCtx.save(); radarCtx.translate(px, py); radarCtx.rotate(a);
      radarCtx.beginPath(); radarCtx.moveTo(spec.r + 1, 0); radarCtx.lineTo(-spec.r, spec.r * 0.8); radarCtx.lineTo(-spec.r, -spec.r * 0.8); radarCtx.closePath();
      radarCtx.fillStyle = spec.color; radarCtx.fill(); radarCtx.restore();
    } else {
      radarCtx.beginPath(); radarCtx.arc(px, py, spec.r, 0, Math.PI * 2);
      radarCtx.fillStyle = spec.color; radarCtx.fill();
    }
  }

  // 自己：中心金點 + 朝向短箭
  radarCtx.fillStyle = "#ffd54a";
  radarCtx.beginPath(); radarCtx.arc(cx, cy, 3.2, 0, Math.PI * 2); radarCtx.fill();
  radarCtx.strokeStyle = "#ffd54a"; radarCtx.lineWidth = 2;
  radarCtx.beginPath(); radarCtx.moveTo(cx, cy); radarCtx.lineTo(cx + hd.x * 11, cy + hd.y * 11); radarCtx.stroke();
  radarCtx.restore();

  // 北標記（north-up：頂端）
  radarCtx.fillStyle = "rgba(255,255,255,0.6)";
  radarCtx.font = "bold 13px system-ui, sans-serif";
  radarCtx.textAlign = "center"; radarCtx.textBaseline = "top";
  radarCtx.fillText("北", cx, 3);
}

// 各類實體用各自的 Map 追蹤（id → group），快照進來時 reconcile。
const players = new Map();
const npcs = new Map();
const wildlife = new Map();
const enemies = new Map();
const nodes = new Map(); // key 用座標字串（節點無穩定 id）
// 夜採星晶礦脈（ROADMAP 629）：夜間限定可採集的發光晶脈，key 用座標字串（無穩定 id）。
// 天亮後伺服器停送 star_crystals → reconcile 自動把它們淡出清除（鏡像節點）。
const starCrystals = new Map();
// 採集判距用的「最新一筆權威世界座標／快照」（世界 px，非場景單位）：每份快照更新，
// render 每幀據此算「腳邊有沒有可採的東西」→ 點亮採集鈕（ROADMAP 629）。
let latestSelfWorld = null; // {x,y}＝自己最新權威世界座標；沒有就還沒收到含自己的快照
let latestNodes = [];       // 最新一筆採集節點（樹／石／乙太礦，世界 px）
let latestCrystals = [];    // 最新一筆星晶礦脈（夜間限定，世界 px）
// 在 3D 裡揮劍迎敵（ROADMAP 632）：留存最新一筆「活著的敵人」快照（世界 px，含 hp／max_hp），
// render 每幀據此算「腳邊有沒有可攻擊的敵人」→ 點亮攻擊鈕、按鍵即送既有權威攻擊意圖。
let latestEnemies = [];     // 最新一筆活著的敵人（世界 px）
// 園丁撫慰（ROADMAP 634）：留存最新一筆居民快照（世界 px，含 needs_care／name），render 每幀據此
// 算「腳邊有沒有正有心事、需要被關心的故鄉居民」→ 點亮關心鈕、按鍵即送既有權威撫慰意圖。
let latestNpcs = [];        // 最新一筆居民／NPC 快照（世界 px）
// 居民互助（ROADMAP 125）：把既有「協助求助居民」接進 3D。每份快照留存「目前正在求助的居民 id 集合」
// （active_help_requests），render 每幀據此算「腳邊有沒有正在求助的居民」→ 點亮幫忙鈕、按鍵即送既有權威協助意圖。
let latestHelpRequests = new Set(); // 最新一筆「正在求助的居民 id」集合（resident_N）
// 城鎮交易（ROADMAP 630）：把既有「商人商店」（#57）接進 3D。每份快照留存「帶買賣目錄的 NPC」
// 與「自己這筆 PlayerView」（含乙太／背包），render 每幀據此點亮交易鈕、開著面板時即時刷新行情。
let latestMerchants = [];   // 最新一筆商人清單（NpcView 中 buy_list／sell_list 非空者，世界 px）
let latestSelfItem = null;  // 自己這筆 PlayerView（含 ether／inventory）；沒有就還沒收到含自己的快照
// 情境感知澆水／收成鈕（手機優先 HUD）：留存最新一筆農地快照（含 owner／origin_x／origin_y／cols／rows／
// tile_size，世界 px），render 每幀據此算「自己有沒有站在自家田旁」→ 才顯示澆水／收成鈕（否則隱藏，不擾畫面）。
let latestFields = [];
// 寵物夥伴（ROADMAP 627）：key 用「主人 id:種類」——主人換了寵物時舊 key 自然淡出、新身形淡入。
// 寵物有自己的世界座標（pet_x/pet_y，伺服器權威跟隨主人），走與玩家同套快照內插。
const pets = new Map();
// 玩家表情泡泡（ROADMAP 621）：from_id → { glyph, startMs, displaySecs }。收到 player_emote 即覆寫，
// 每幀由 updatePlayerEmotes 依存活進度動畫＋過期自清，與玩家 group 鬆耦合（玩家離線/離開視野自然收掉）。
const playerEmotes = new Map();
// 居民對話泡泡（ROADMAP 622）：npc_id → { text, startMs, displaySecs }。收到 npc_speech 即覆寫，
// 每幀由 updateNpcSpeech 依存活進度淡入淡出＋過期自清，與 NPC group 鬆耦合（說話者離開視野自然收掉）。
const npcSpeech = new Map();

let myId = null;
let snapshotCount = 0;
let firstFollowDone = false;
let missingSelfWarned = false;

// 自身客戶端預測 + 對帳的狀態（場景座標）：權威座標來自快照，自己的 mesh 平滑拉回它。
let selfAuthX = 0, selfAuthZ = 0;         // 最新權威座標（場景單位）
let selfHasAuth = false;                  // 是否已收到過自己的權威座標
let lastSelfAuthT = 0;                     // 上一筆自己權威快照的到達時間（performance.now）
let selfMeasuredSpeed = PREDICT_SCENE_SPEED; // 實測自身速度（場景單位/秒），由快照自我校準
// wasm 碰撞感知預測用的「自身世界座標」（世界 px）。wasmPredictReady() 時取代上面的場景空間
// 簡單推：每幀以 step_player 推進（會被地形擋住），再轉 sx/sz 套到 mesh。!has = 尚未錨定。
const predWorld = { x: 0, y: 0, has: false };
// 註：對帳改成「每幀平滑連續收斂到最新權威」後，不再需要「逐快照序號」去 gate 離散校準
// （那正是抖動來源）；收斂每幀都做、自動吃進最新的 latestSelfWorld / selfAuth，無須序號。
// 權威外插用的「自身權威世界座標」近期歷史（世界 px）：每收一筆自己的快照就 push，
// 供 estimateSelfAuthVel() 用時間窗算權威移動速度，把對齊目標往前外插成平滑點。
const selfAuthHist = [];            // [{t, x, y}]（t＝performance.now、x/y＝世界 px），只留窗內幾筆
// 用 AUTH_VEL_WINDOW_MS 時間窗估計自身權威速度（世界 px/s）：取窗內最舊一筆到最新一筆的位移／時間。
// 時間窗夠長＝平滑掉「快照叢發（1–2ms 連到）」造成的雜訊；夠短＝跟得上真實轉向。樣本不足回 0（不外插）。
function estimateSelfAuthVel() {
  const h = selfAuthHist;
  if (h.length < 2) return { vx: 0, vy: 0 };
  const newest = h[h.length - 1];
  let oldest = h[h.length - 2];
  for (let i = h.length - 2; i >= 0; i--) {
    oldest = h[i];
    if (newest.t - h[i].t >= AUTH_VEL_WINDOW_MS) break; // 回看到時間窗邊界就停
  }
  const dt = (newest.t - oldest.t) / 1000;
  if (dt < 1e-3) return { vx: 0, vy: 0 };
  return { vx: (newest.x - oldest.x) / dt, vy: (newest.y - oldest.y) / dt };
}

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
        g.userData.item = item; // 留存最新一筆快照欄位（NPC 內心生活呈現要讀 activity/thought/…）
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
    case "chat": {
      // 只接「自己剛觸發的照料動作」的系統權威回報（ROADMAP 619）：在送出 water_all／harvest_all
      // 後開的 3 秒窗內、且來源是「系統」單播，才浮成 toast——誠實回饋（澆了幾株／太遠走近／作物剛好），
      // 不洩漏其他玩家的聊天、也不假裝結果。窗外或非系統訊息一律忽略（3D 暫不顯示一般聊天）。
      const now = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
      if (now < awaitTendReplyUntil && msg && msg.from === "系統" && typeof msg.text === "string") {
        awaitTendReplyUntil = -1e9; // 用掉這次窗，避免一次動作吃到後續無關系統訊息
        flashToast(msg.text);
      }
      break;
    }
    case "snapshot": {
      snapshotCount++;
      // 日夜狀態：留存最新一筆權威 daynight，render 每幀據此讓世界的天色／光照流轉（ROADMAP 612）。
      if (msg.daynight && typeof msg.daynight === "object") { latestDayNight = msg.daynight; dayNightAnchorMs = performance.now(); } // 重設天時盤倒數錨點（ROADMAP 620）
      // 天氣／彩虹：留存最新一筆權威 weather／rainbow，render 每幀據此讓粒子場與遠空彩虹流轉（ROADMAP 613）。
      if (msg.weather && typeof msg.weather === "object") latestWeather = msg.weather;
      if (msg.rainbow && typeof msg.rainbow === "object") latestRainbow = msg.rainbow;
      // 鎮民派系（ROADMAP 625）：留存最新一筆結盟／敵對配對，render 每幀據此在居民之間畫關係連線。
      if (Array.isArray(msg.town_factions)) latestTownFactions = msg.town_factions;
      // 居民互助送禮（ROADMAP 369）：留存最新一筆「誰正分享給誰」，render 每幀據此在兩人之間飄一枚光禮（非物件＝無人正在分享）。
      latestTownShare = (msg.town_share && typeof msg.town_share === "object") ? msg.town_share : null;
      // 重大世界事件（ROADMAP 631）：留存最新一筆宇宙裂縫／獸潮攻城（非物件＝無事件），
      // render 每幀據此在事件座標立起／隱藏醒目地標（光柱／警示光束）。
      latestRift = (msg.world_event && typeof msg.world_event === "object") ? msg.world_event : null;
      latestHorde = (msg.horde_event && typeof msg.horde_event === "object") ? msg.horde_event : null;
      // 這份快照的到達時間：全類共用一個時間戳，內插時間軸才一致。
      const recvT = performance.now();
      // 玩家：火柴人（自己金色、別人藍色），帶名字標籤
      reconcile(
        msg.players, players,
        (p) => p.id,
        (p) => {
          // 火柴人（自己金色／別人藍色）＋名字，再掛一枚頭頂表情泡泡（ROADMAP 621）
          const g = makeEntity(makeStickman(p.id === myId ? SELF_COLOR : PLAYER_COLOR), p.name || "玩家");
          attachEmoteBubble(g);
          return g;
        },
        recvT
      );
      // 寵物夥伴（ROADMAP 627）：從玩家快照派生「有養寵物且帶有效座標」的那些，接進 3D。
      // 寵物自有世界座標（pet_x/pet_y）＝獨立實體，故另成一列走 reconcile 的內插＋AOI 淡入淡出，
      // 而非掛成玩家子節點（座標不同步）。key 用「主人 id:種類」：換寵物 → 舊身形淡出、新身形淡入。
      const petList = Array.isArray(msg.players)
        ? msg.players.filter((p) => p && typeof p.pet_kind === "string" && p.pet_kind
            && Number.isFinite(p.pet_x) && Number.isFinite(p.pet_y))
            .map((p) => ({
              owner: p.id, x: p.pet_x, y: p.pet_y, pet_kind: p.pet_kind,
              pet_playing: p.pet_playing, pet_fetching: p.pet_fetching,
              pet_personality: p.pet_personality, pet_bond: p.pet_bond,
            }))
        : [];
      reconcile(
        petList, pets,
        (p) => p.owner + ":" + p.pet_kind,
        (p) => {
          const g = makePet(p.pet_kind);
          attachPetStatus(g);
          scene.add(g); // makePet 不經 makeEntity（寵物無名牌），故自行入景
          return g;
        },
        recvT
      );
      // NPC（含居民／商人）：暖棕火柴人，帶名字
      reconcile(
        msg.npcs, npcs,
        (n) => n.id,
        (n) => {
          // 暖棕火柴人 + 名字（沿用 #767 程序化 stickman），再掛上內心生活的三層呈現（狀態／關懷／思想）
          const g = makeEntity(makeStickman(NPC_COLOR), n.name || "NPC");
          attachResidentStatus(g);
          return g;
        },
        recvT
      );
      // 野生動物：低多邊形動物身形（鳥／鹿／小獸／狼狐），頭頂掛馴養／幼獸狀態（ROADMAP 615）
      reconcile(
        msg.wildlife, wildlife,
        (w) => "w" + w.id,
        (w) => {
          const g = makeEntity(makeCreature(w.kind));
          attachWildlifeStatus(g);
          return g;
        },
        recvT
      );
      // 敵人：機械／靈體／巨像三型低多邊形身形，頭頂掛血條＋狀態 emoji（ROADMAP 626）；
      // 被打倒（alive=false）就當作消失移除。
      reconcile(
        Array.isArray(msg.enemies) ? msg.enemies.filter((e) => e.alive !== false) : [],
        enemies,
        (e) => e.eid || (e.x + "_" + e.y),
        (e) => {
          const g = makeEntity(makeEnemy(e.kind));
          attachEnemyStatus(g);
          return g;
        },
        recvT
      );
      // 採集節點（樹／石／乙太礦）：以座標當 key（節點無穩定 id）
      reconcile(
        msg.nodes, nodes,
        (n) => n.kind + "@" + Math.round(n.x) + "," + Math.round(n.y),
        (n) => makeNode(n.kind),
        recvT
      );
      // 夜採星晶礦脈（ROADMAP 629）：夜間限定可採集晶脈，以座標當 key（無穩定 id）；
      // 天亮後伺服器停送 → reconcile 自動淡出清除（與一般節點同套靜態淡入淡出）。
      reconcile(
        Array.isArray(msg.star_crystals) ? msg.star_crystals : [], starCrystals,
        (c) => "sc@" + Math.round(c.x) + "," + Math.round(c.y),
        () => makeStarCrystal(),
        recvT
      );
      // 農地（每位玩家的耕地）：把翻好的土＋成長中的作物＋稻草人接進 3D（ROADMAP 614）。
      reconcileFields(msg.fields, recvT);
      // 情境感知澆水／收成鈕：留存本份農地快照，供 render 每幀算「自己有沒有站在自家田旁」。
      latestFields = Array.isArray(msg.fields) ? msg.fields : [];

      // 人造地標（眾人一起蓋的篝火／協力瞭望塔／署名雪人）：接進 3D（ROADMAP 616）。
      // 都是固定地標、讀 wx/wy；瞭望塔在進度變動時即時把塔身升到對應高度。
      reconcileStatic(msg.campfires, campfires, "cf", makeCampfire, null, recvT);
      reconcileStatic(msg.watchtowers, watchtowers, "wt", makeWatchtower,
        (g, item) => applyTowerProgress(g, watchtowerVisual(item)), recvT);
      reconcileStatic(msg.snowmen, snowmen, "sm", makeSnowman, null, recvT);
      // 故鄉古井（ROADMAP 640，禱告驅動）：把單一固定設施包成單元素清單走同一套 reconcile；
      // wx/wy 對齊 reconcileStatic 介面，watering 記進 userData.item 供每幀畫水波。
      reconcileStatic(
        msg.village_well && Number.isFinite(msg.village_well.x) && Number.isFinite(msg.village_well.y)
          ? [{ id: "well", wx: msg.village_well.x, wy: msg.village_well.y, watering: msg.village_well.watering === true }]
          : [],
        villageWells, "vw", makeVillageWell, null, recvT);
      // 故鄉茶棚（ROADMAP 641，禱告驅動）：把單一固定設施包成單元素清單走同一套 reconcile；
      // wx/wy 對齊 reconcileStatic 介面，brewing 記進 userData.item 供每幀畫蒸汽。
      reconcileStatic(
        msg.village_tea_stall && Number.isFinite(msg.village_tea_stall.x) && Number.isFinite(msg.village_tea_stall.y)
          ? [{ id: "tea", wx: msg.village_tea_stall.x, wy: msg.village_tea_stall.y, brewing: msg.village_tea_stall.brewing === true }]
          : [],
        villageTeaStalls, "vts", makeVillageTeaStall, null, recvT);
      // 居民木屋（ROADMAP 642，禱告驅動）：列表形式，每座木屋以 name 為 id 區分。
      // wx/wy 對齊 reconcileStatic 介面；name 記進 item 供 makeResidentHome 顯示標籤。
      reconcileStatic(
        Array.isArray(msg.resident_homes)
          ? msg.resident_homes
              .filter(h => h && Number.isFinite(h.x) && Number.isFinite(h.y) && typeof h.name === "string")
              .map(h => ({ id: "home_" + h.name, wx: h.x, wy: h.y, name: h.name }))
          : [],
        residentHomes, "rh", makeResidentHome, null, recvT);
      // 豐收節慶典（ROADMAP 646，禱告驅動）：單一固定設施；active 記進 userData.item 供每幀動畫。
      reconcileStatic(
        msg.harvest_festival && Number.isFinite(msg.harvest_festival.x) && Number.isFinite(msg.harvest_festival.y)
          ? [{ id: "hf", wx: msg.harvest_festival.x, wy: msg.harvest_festival.y, active: msg.harvest_festival.active === true }]
          : [],
        harvestFestivals, "hfst", makeHarvestFestival, null, recvT);
      // 田邊清泉（ROADMAP 647，禱告驅動·應諾娃之禱）：農田北坡的天然湧泉；前端畫圍石盆＋漣漪＋水泡動畫。
      reconcileStatic(
        msg.field_spring && Number.isFinite(msg.field_spring.x) && Number.isFinite(msg.field_spring.y)
          ? [{ id: "fs", wx: msg.field_spring.x, wy: msg.field_spring.y }]
          : [],
        fieldSprings, "fsp", makeFieldSpring, null, recvT);
      // 故鄉街燈（ROADMAP 648，禱告驅動·應露娜之禱）：城鎮街道七盞夜燈；入夜自動亮起光暈（讀 daynight.phase），純靜態座標。
      reconcileStatic(
        Array.isArray(msg.street_lamps)
          ? msg.street_lamps
              .filter(l => Number.isFinite(l.x) && Number.isFinite(l.y))
              .map((l, i) => ({ id: `sl${i}`, wx: l.x, wy: l.y }))
          : [],
        streetLamps, "sl", makeStreetLamp, null, recvT);

      // 玩家親手種下、隨真實時間長大的世界樹群（ROADMAP 617）：以座標當 key、長大了才重塑樹身。
      reconcileGroves(msg.world_groves, recvT);

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

      // 採集判距：留存自己的權威「世界座標」＋本份節點／星晶快照（世界 px），供 render 每幀算
      // 「腳邊有沒有可採的東西」→ 點亮採集鈕（ROADMAP 629）。注意是世界 px、非場景單位（sx/sz）。
      latestNodes = Array.isArray(msg.nodes) ? msg.nodes : [];
      latestCrystals = Array.isArray(msg.star_crystals) ? msg.star_crystals : [];
      latestSelfWorld = (meAuth && Number.isFinite(meAuth.x) && Number.isFinite(meAuth.y))
        ? { x: meAuth.x, y: meAuth.y } : null;
      // 權威外插：把這筆自身權威世界座標記進近期歷史（含到達時間 recvT），供收斂目標往前外插。
      // 只留時間窗 + 一點餘裕的幾筆（避免無限長）。
      if (latestSelfWorld) {
        selfAuthHist.push({ t: recvT, x: latestSelfWorld.x, y: latestSelfWorld.y });
        const cutoff = recvT - (AUTH_VEL_WINDOW_MS + 120);
        while (selfAuthHist.length > 2 && selfAuthHist[0].t < cutoff) selfAuthHist.shift();
      }

      // 揮劍迎敵判距（ROADMAP 632）：留存本份「活著的敵人」快照（世界 px），供 render 每幀算
      // 「腳邊有沒有可攻擊的敵人」→ 點亮攻擊鈕。死掉的（alive=false）排除＝不會誤點亮揮向屍體。
      latestEnemies = Array.isArray(msg.enemies) ? msg.enemies.filter((e) => e && e.alive !== false) : [];

      // 園丁撫慰判距（ROADMAP 634）：留存本份居民快照（世界 px），供 render 每幀算「腳邊有沒有
      // 正有心事、需要被關心的故鄉居民」→ 點亮關心鈕（needs_care 由後端依需求偏低判定）。
      latestNpcs = Array.isArray(msg.npcs) ? msg.npcs : [];

      // 居民互助判距（ROADMAP 125）：留存本份「正在求助的居民 id」集合，供 render 每幀算「腳邊有沒有
      // 正在求助的居民」→ 點亮幫忙鈕。壞值（缺欄位／非陣列）→ 空集合，按鈕一律保守鎖定，永不誤點亮。
      latestHelpRequests = new Set(Array.isArray(msg.active_help_requests) ? msg.active_help_requests : []);

      // 城鎮交易判距（ROADMAP 630）：留存「帶買賣目錄的 NPC」＋「自己這筆 PlayerView」（含乙太／背包），
      // 供 render 每幀算「腳邊有沒有商人」→ 點亮交易鈕；面板開著時據此即時刷新行情／庫存／餘額。
      latestMerchants = shopMerchantsFrom(msg.npcs);
      latestSelfItem = (meAuth && typeof meAuth === "object") ? meAuth : null;
      if (shopOpen) refreshShopPanel(); // 面板開著就跟著快照刷新（行情漲跌、庫存、我的乙太/背包）

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

      // HUD（手機優先極簡重做）：只留名字＋線上人數一行；危機級警示（宇宙裂縫／獸潮）才補第二行。
      // 移除了過去那行會與畫面重疊的操作提示，以及一整排次要數字（NPC／野生／敵人／農地…）——
      // 日夜看右上角天時盤、方位看雷達、可做的事由情境動作鈕自己冒出來，頂部不再喧賓奪主。
      const meItem = Array.isArray(msg.players) ? msg.players.find((p) => p.id === myId) : null;
      const myName = meItem ? (meItem.name || "玩家") : "（加入中…）";
      const riftLabel = riftHudLabel(latestRift);    // 宇宙裂縫已開啟＋倒數（ROADMAP 631）
      const hordeLabel = hordeHudLabel(latestHorde); // 獸潮逼近／攻城＋地名＋倒數（ROADMAP 631）
      const alerts = [riftLabel, hordeLabel].filter(Boolean).join(" · ");
      hudEl.innerHTML =
        `<b>${myName}</b> · 線上 ${players.size} 人` + (alerts ? `\n${alerts}` : "");

      setStatus(
        `真實世界已連上 · 快照 #${snapshotCount}` +
        (meGroup ? "（鏡頭跟著你）" : "")
      );
      break;
    }
    case "player_emote": {
      // 玩家表情（ROADMAP 621）：在比表情的玩家頭頂彈跳浮起一枚大 emoji 後淡出。
      // glyph 由伺服器權威帶回（白名單內合法表情）；同一玩家連發會覆蓋上一個未消的表情。
      if (msg && msg.from_id) {
        playerEmotes.set(msg.from_id, {
          glyph: msg.glyph || "❓",
          startMs: performance.now(),
          displaySecs: msg.display_secs || 4,
        });
      }
      break;
    }
    case "npc_reply": {
      // 搭話回話（ROADMAP 636）：自己向居民／城鎮大人物攀談後，伺服器單播這句回應（純模板／話題層、零 LLM）。
      // 浮成 toast 讓本人讀得到（同一句也會由 NpcSpeech 在對方頭頂顯成對話泡泡 622）。缺欄位安全降級、永不拋。
      if (msg && typeof msg.text === "string" && msg.text) {
        const who = (typeof msg.display === "string" && msg.display) ? msg.display : "💬";
        flashToast(`${who}：${msg.text}`);
      }
      break;
    }
    case "npc_speech": {
      // 居民對話泡泡（ROADMAP 92／622）：居民彼此互聊／向玩家打招呼／評論時，伺服器廣播這則事件——
      // 在說話者頭頂浮起一枚白底對話泡泡、幾秒後淡出。AI 社會的對話第一次在 3D 裡「看得見、看得懂」。
      // 同一位居民連說會覆蓋上一句（與 2D 一致）；缺欄位安全降級（空字串／預設秒數），永不拋。
      if (msg && msg.npc_id) {
        npcSpeech.set(msg.npc_id, {
          text: typeof msg.text === "string" ? msg.text : "",
          startMs: performance.now(),
          displaySecs: msg.display_secs || 8,
        });
      }
      break;
    }
    case "attack_hit": {
      // 命中傷害飄字（ROADMAP 387／632）：任何人打中敵人，伺服器即全服廣播命中——在命中處（ex,ey）
      // 浮起傷害數字＋淡出上飄。3D 世界第一次「看得見」戰鬥的力道（暴擊更大更橘、破綻直擊金、擊殺加 💀）。
      // 缺欄位安全降級、缺命中座標安全跳過（不 throw；守 render-loop-resilience）。
      spawnDamageFloat(msg);
      break;
    }
    case "loot_pickup": {
      // 戰利品入袋飄字（ROADMAP 635／509）：擊殺怪物後伺服器私信 LootPickup{ex,ey,item,qty}——
      // 在怪物原位飄出「🪵 木材 ×2」暖米色飄字＝你打贏了、戰利品入袋了。補上 632 戰鬥迴圈在 3D 缺的
      // 「獎勵回饋」那一半。缺座標／壞欄位由 spawnRewardFloat／lootFloatSpec 安全降級（不 throw）。
      spawnRewardFloat(lootFloatSpec(msg), msg && msg.ex, msg && msg.ey);
      break;
    }
    case "kill_streak": {
      // 連殺標語（ROADMAP 635／381）：8 秒內連殺達 2/4/8 隻時伺服器私信本人 KillStreak{player_id,streak,x,y}——
      // 在自己頭頂飄出熱度標語（段位越高越橘紅）。旁觀者／非本人由 killStreakFloatSpec 回 null 自動忽略。
      spawnRewardFloat(killStreakFloatSpec(msg, myId), msg && msg.x, msg && msg.y);
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
let runHeld = false;           // Shift 跑（桌機 hold-to-run）
let runToggle = false;         // 跑步切換鈕（手機）：tap 切走/跑。搖桿只控方向、不用推深控速
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
  // 在數量輸入框（交易面板）裡打字時，別讓 WASD／動作鍵被當成遊戲操作（ROADMAP 630）。
  if (e.target && e.target.tagName === "INPUT") return;
  if (e.code === "Space") { wantJump = true; e.preventDefault(); return; }
  if (e.code === "ShiftLeft" || e.code === "ShiftRight") { runHeld = true; return; }
  if (e.code === "KeyB" && !e.repeat) { if (globalThis.__bf3dToggleShop) globalThis.__bf3dToggleShop(); e.preventDefault(); return; } // 城鎮交易（ROADMAP 630）
  if (e.code === "KeyT" && !e.repeat) { tryPlantTree(); e.preventDefault(); return; } // 種樹（ROADMAP 618）
  if (e.code === "KeyF" && !e.repeat) { tryTend("water"); e.preventDefault(); return; }   // 一鍵澆水（ROADMAP 619）
  if (e.code === "KeyH" && !e.repeat) { tryTend("harvest"); e.preventDefault(); return; } // 一鍵收成（ROADMAP 619）
  if (e.code === "KeyC" && !e.repeat) { tryLightCampfire(); e.preventDefault(); return; } // 野營篝火（ROADMAP 623）
  if (e.code === "KeyG" && !e.repeat) { tryGather(); e.preventDefault(); return; } // 採集（ROADMAP 629）
  if (e.code === "KeyR" && !e.repeat) { tryAttack(); e.preventDefault(); return; } // 揮劍迎敵（ROADMAP 632）
  if (e.code === "KeyV" && !e.repeat) { tryComfort(); e.preventDefault(); return; } // 園丁關心有心事的居民（ROADMAP 634）
  if (e.code === "KeyX" && !e.repeat) { tryHelp(); e.preventDefault(); return; } // 幫忙正在求助的居民（ROADMAP 125）；F 已被一鍵澆水佔用，改 X（與 Z 搭話／V 撫慰同屬左下社交動作群）
  if (e.code === "KeyZ" && !e.repeat) { tryTalk(); e.preventDefault(); return; } // 跟居民／城鎮大人物搭話（ROADMAP 636）
  if (e.code === "KeyQ" && !e.repeat) { tryGift(); e.preventDefault(); return; } // 送禮給故鄉居民（ROADMAP 639；社交動作群，挑未佔用的 Q 鍵）
  if (e.code === "KeyE" && !e.repeat) { if (globalThis.__bf3dToggleEmoteWheel) globalThis.__bf3dToggleEmoteWheel(); e.preventDefault(); return; } // 表情輪（ROADMAP 621）
  const dir = keyToDir(e);
  if (dir) { heldKeys[dir] = true; e.preventDefault(); }
});
window.addEventListener("keyup", (e) => {
  if (e.code === "ShiftLeft" || e.code === "ShiftRight") { runHeld = false; return; }
  const dir = keyToDir(e);
  if (dir) heldKeys[dir] = false;
});

// ============================================================
// 照料世界·互動（ROADMAP 618）：3D 世界第一次能「動手」——登入後可在 3D 裡親手種下世界樹。
//   · 純前端、純送既有權威意圖 `plant_tree`（與 2D web/game.js 同協議），零後端／協議／world-core 改動。
//   · 伺服器才是權威：種樹需「已登入＋戶外」（3D 一律戶外），訪客送了不留痕跡，故 UI 誠實鎖定、不假裝能種。
//   · 登入態同源自動帶（cookie session）；開頁查 /auth/me 確認後點亮鈕。
//   · 種下後嫩芽會在下一份快照的 world_groves 冒出來、隨真實時間一階階長大（ROADMAP 617 已會畫）。
// ============================================================
let isLoggedIn = false;          // /auth/me 查到帳號才為 true（純供 UI 誠實；真正權限由後端 cookie 決定）
let myName3d = "3D玩家";          // 顯示名：登入後換成帳號名（authed 身分仍由 cookie 決定，與此無關）
let mySpecies3d = "terran";
let lastPlantAt = -1e9;           // 上次種樹的本地時戳，做點擊冷卻防手滑連點
const PLANT_COOLDOWN_MS = 600;    // 本地冷卻（後端另有每秒 3 次限流，這只是前端手感防呆）
let lastTendAt = -1e9;            // 上次澆水／收成的本地時戳（共用一個冷卻，避免連點洗系統訊息）
const TEND_COOLDOWN_MS = 600;     // 本地冷卻
let lastCampfireAt = -1e9;        // 上次生火的本地時戳（後端另有每人冷卻＋全服上限，這只是手感防呆）
const CAMPFIRE_COOLDOWN_MS = 800; // 本地冷卻
let awaitTendReplyUntil = -1e9;   // 在此時戳前收到的系統單播回報，浮成 toast（只接自己剛觸發的那則）
const TEND_REPLY_WINDOW_MS = 3000;// 等待窗：送出後 3 秒內的系統回覆視為本次照料的權威回報

// 要送給伺服器的「種樹」意圖（與 2D 同形 `{type:"plant_tree"}`）。抽成純函式供 smoke 斷言。
function plantTreeWireMsg() { return { type: "plant_tree" }; }
// 一鍵澆水／一鍵收成意圖（ROADMAP 619，與 2D web/game.js 同協議 `water_all`／`harvest_all`）。
// 都是無參數的權威意圖——伺服器自行判定「在自家農地可及範圍」才生效（鏡像 2D），3D 純送意圖。
function waterAllWireMsg() { return { type: "water_all" }; }
function harvestAllWireMsg() { return { type: "harvest_all" }; }
// 野營篝火意圖（ROADMAP 623，與 2D web/game.js 同協議 `light_campfire`）。
// 無參數的權威意圖——伺服器一律用升火者自己的權威座標在腳下升火（防隔空生火），
// 並由 CampfireField 的每人冷卻＋全服上限把關；3D 純送意圖，火會在下一份快照冒出來。
function campfireWireMsg() { return { type: "light_campfire" }; }

// 種樹鈕的顯示狀態（登入→可種、訪客→鎖定提示）。純函式、面向玩家字串集中可 i18n、供 smoke 真值表。
function plantButtonState(loggedIn) {
  return loggedIn
    ? { label: "🌱 種樹", locked: false, hint: "在你站的地方種下一株嫩芽" }
    : { label: "🌱 種樹（登入後）", locked: true, hint: "在 but-fun.com 登入後，就能在世界種下你的樹" };
}

// 照料農地鈕（澆水／收成）的顯示狀態。登入→可用（須走到自己農地旁才生效，由伺服器裁決）；
// 訪客→鎖定（沒地、送了不留痕跡，故誠實鎖定不假裝）。純函式、字串集中可 i18n、供 smoke 真值表。
// kind: "water"｜"harvest"。未知 kind 保守回種樹以外的安全鎖定態（不 throw）。
function tendButtonState(loggedIn, kind) {
  const spec = kind === "harvest"
    ? { emoji: "🌾", name: "收成", doneHint: "走到自己的農地旁，把成熟的作物一次收完", lockHint: "在 but-fun.com 登入並擁有農地後，就能在 3D 裡收成" }
    : { emoji: "💧", name: "澆水", doneHint: "走到自己的農地旁，把缺水的作物一次澆滿", lockHint: "在 but-fun.com 登入並擁有農地後，就能在 3D 裡澆水" };
  return loggedIn
    ? { label: `${spec.emoji} ${spec.name}`, locked: false, hint: spec.doneHint }
    : { label: `${spec.emoji} ${spec.name}（登入後）`, locked: true, hint: spec.lockHint };
}

// 篝火鈕的顯示狀態（ROADMAP 623）。與 2D 一致——升火不需帳號登入，連上線的訪客也能在腳下生火
// （伺服器只認連線玩家的權威座標＋戶外），故永不鎖定；純函式、字串集中可 i18n、供 smoke 真值表。
function campfireButtonState() {
  return { label: "🔥 生火", locked: false, hint: "在你站的地方升起一堆篝火，火光暖意逼退附近野獸（燒完即熄）" };
}

// ============================================================
// 在 3D 裡採集（ROADMAP 629）：3D 世界第一次能做「核心勞動」——走近樹／石／乙太礦按鍵採集，
// 夜裡更有星晶礦脈現身可採。直接回應 AI 居民反覆許願的「夜間缺乏值得出門的目標」：
// 過去 3D 的夜只有威脅（敵人），現在夜裡走出去，腳邊會冒出可採的星晶＝夜的第一個「正向理由」。
//   · 純前端、純送既有權威意圖：一般節點 {type:"gather"}、星晶 {type:"gather_star_crystal"}
//     （與 2D web/game.js 同協議）；零後端／協議／world-core 改動。
//   · 伺服器才是權威：兩種採集都「不帶座標」——伺服器一律用採集者自己的權威座標判距（防隔空採集），
//     星晶還會驗「現在是夜間」。前端只負責「走近了才點亮鈕、送出意圖」，採不採得到由後端裁決。
//   · 採集不需登入（連線玩家即可，鏡像生火的誠實態）；星晶白天本就不存在＝鈕自然不亮。
// ============================================================
const GATHER_NODE_REACH = 56;     // 一般節點伸手範圍（px）——對齊後端 gather_field::GATHER_REACH
const GATHER_CRYSTAL_REACH = 80;  // 星晶礦脈伸手範圍（px）——對齊後端 star_crystal::GATHER_REACH
let lastGatherAt = -1e9;          // 上次採集的本地時戳（手感防呆；後端另有權威判定）
const GATHER_COOLDOWN_MS = 320;   // 本地冷卻（連點上限；真正可否採由後端決定）

// 採集意圖 wire（與 2D 同協議，無座標——伺服器用採集者權威座標判距）。抽成純函式供 smoke 斷言。
function gatherWireMsg() { return { type: "gather" }; }
function gatherStarCrystalWireMsg() { return { type: "gather_star_crystal" }; }

// 採集目標判定（純函式、確定性、可測）：給自己的世界座標＋本份節點／星晶快照（世界 px），
// 回最近且落在伸手範圍內的可採目標。星晶（夜間限定、80px）優先於一般節點（56px）——
// 夜裡走到晶脈先採晶，否則採腳邊的樹／石／礦。壞座標／空快照／無自己座標一律安全回 null
// （不 throw、不誤點亮鈕；守 render-loop-resilience）。
function gatherTargetAt(self, nodes3, crystals) {
  if (!self || !Number.isFinite(self.x) || !Number.isFinite(self.y)) return null;
  const nearest = (list, reach, pick) => {
    let best = null, bestD = reach * reach;
    for (const it of (Array.isArray(list) ? list : [])) {
      if (!it) continue;
      const m = pick(it);
      if (!m || !Number.isFinite(m.x) || !Number.isFinite(m.y)) continue;
      const dx = m.x - self.x, dy = m.y - self.y, d = dx * dx + dy * dy;
      if (d <= bestD) { bestD = d; best = m; }
    }
    return best;
  };
  // 星晶優先（夜間限定的稀客，別被腳邊普通節點蓋過）。
  const crystal = nearest(crystals, GATHER_CRYSTAL_REACH, (c) => ({ x: c.x, y: c.y }));
  if (crystal) return { kind: "crystal", x: crystal.x, y: crystal.y };
  const node = nearest(nodes3, GATHER_NODE_REACH, (n) => ({ x: n.x, y: n.y, nodeKind: n.kind }));
  if (node) return { kind: "node", nodeKind: node.nodeKind, x: node.x, y: node.y };
  return null;
}

// 採集鈕的顯示狀態（純函式、面向玩家字串集中可 i18n、供 smoke 真值表）。
// 走近可採目標才亮（依種類換字樣／提示），否則鎖定提示「走近資源」。
function gatherButtonState(target) {
  if (!target) return { label: "⛏️ 採集", locked: true, hint: "走到樹／礦脈／星晶旁，就能採集" };
  if (target.kind === "crystal") return { label: "💎 採星晶", locked: false, hint: "採下這道夜間限定的星晶礦脈，得星晶碎片" };
  const k = target.nodeKind;
  if (k === "tree") return { label: "🪓 伐木", locked: false, hint: "採這棵樹得木材" };
  if (k === "ether_ore") return { label: "🔮 採乙太", locked: false, hint: "採這道乙太礦脈得乙太礦" };
  return { label: "⛏️ 採石", locked: false, hint: "採腳邊的礦脈得石材" };
}

// 只在 ws 開著時送，避免未連線時丟訊息拋例外。回傳是否真的送出。
function safeSend(obj) {
  if (ws && ws.readyState === WebSocket.OPEN) { ws.send(JSON.stringify(obj)); return true; }
  return false;
}

// 情境感知動作鈕的「顯示／隱藏」：只在狀態真的變動時改 DOM（守 render-loop-resilience，避免每幀寫樣式）。
// 用 dataset 記住上次顯示態；隱藏走 display:none（脫離排版，動作鈕欄自動收合＝乾淨不留空位）。
function setBtnShown(btn, show) {
  if (!btn) return;
  // 用元素上的私有屬性記住上次顯示態（比 dataset 穩——測試用的假 DOM 不一定有 dataset／style）。
  const want = show ? "1" : "0";
  if (btn._bfShown === want) return;
  btn._bfShown = want;
  if (btn.style) btn.style.display = show ? "" : "none";
}

// 純函式：自己有沒有站在「自家田」的可及範圍內（世界 px）。供澆水／收成鈕情境顯示。
// 田 bbox = origin..origin+格數×格寬，再往外放寬一格當「田邊也算可及」（伺服器才是真正權威判距）。
// 壞座標／空快照／無 myId 一律保守回 false（不誤顯示鈕；不 throw）。
function nearOwnField(self, fields, selfId) {
  if (!self || !selfId || !Number.isFinite(self.x) || !Number.isFinite(self.y)) return false;
  for (const f of (Array.isArray(fields) ? fields : [])) {
    if (!f || f.owner !== selfId) continue;
    if (!Number.isFinite(f.origin_x) || !Number.isFinite(f.origin_y)) continue;
    const cols = Number.isFinite(f.cols) && f.cols > 0 ? f.cols : 1;
    const rows = Number.isFinite(f.rows) && f.rows > 0 ? f.rows : 1;
    const ts = Number.isFinite(f.tile_size) && f.tile_size > 0 ? f.tile_size : 48;
    const pad = ts; // 田邊外放寬一格也算「可及」
    if (self.x >= f.origin_x - pad && self.x <= f.origin_x + cols * ts + pad &&
        self.y >= f.origin_y - pad && self.y <= f.origin_y + rows * ts + pad) {
      return true;
    }
  }
  return false;
}

// 短暫飄字回饋（種樹確認／訪客提示）。reduceMotion 下照常顯示（淡入淡出屬輕量、不擾人）。
let toastTimer = null;
function flashToast(text) {
  const el = document.getElementById("toast");
  if (!el) return;
  el.textContent = text;
  el.classList.add("show");
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { el.classList.remove("show"); }, 2200);
}

// 依登入態刷新「種樹」與常駐「生火」鈕（澆水／收成改走每幀情境顯示，見 updateFarmBtns）。
// 手機優先 HUD：種樹鈕只在登入後出現（訪客本就種不了，與其顯示鎖定鈕不如收起＝乾淨）；
// 生火人人可用＝常駐。在登入態變動時（開頁查 /auth/me）呼叫一次即可。
function updateActBtns() {
  const plantBtn = document.getElementById("plantBtn");
  if (plantBtn) {
    setBtnShown(plantBtn, isLoggedIn);
    if (isLoggedIn) {
      const st = plantButtonState(true);
      plantBtn.textContent = st.label;
      plantBtn.classList.toggle("locked", st.locked);
      plantBtn.title = st.hint;
    }
  }
  const campfireBtn = document.getElementById("campfireBtn");
  if (campfireBtn) {
    const st = campfireButtonState();
    campfireBtn.textContent = st.label;
    campfireBtn.classList.toggle("locked", st.locked);
    campfireBtn.title = st.hint;
  }
}

// 每幀刷新澆水／收成鈕：只在「已登入＋站在自家田旁」時顯示（情境感知），否則隱藏不擾畫面。
// 只在顯示態／文字變動時改 DOM（守 render-loop-resilience）。
function updateFarmBtns() {
  const near = nearOwnField(latestSelfWorld, latestFields, myId);
  for (const kind of ["water", "harvest"]) {
    const btn = document.getElementById(kind === "water" ? "waterBtn" : "harvestBtn");
    if (!btn) continue;
    const show = isLoggedIn && near;
    setBtnShown(btn, show);
    if (!show) continue;
    const st = tendButtonState(true, kind);
    if (btn.textContent !== st.label) btn.textContent = st.label;
    btn.classList.toggle("locked", st.locked);
    btn.title = st.hint;
  }
}

// 嘗試種樹：訪客給登入提示、登入者送權威意圖＋冷卻＋確認飄字。
function tryPlantTree() {
  if (!isLoggedIn) {
    flashToast("🌱 在 but-fun.com 登入後，就能在世界裡種下你的樹");
    return;
  }
  const now = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
  if (now - lastPlantAt < PLANT_COOLDOWN_MS) return; // 冷卻內忽略連點
  if (safeSend(plantTreeWireMsg())) {
    lastPlantAt = now;
    flashToast("🌱 種下一株嫩芽，它會隨時間慢慢長大");
  }
}

// 嘗試一鍵澆水／收成（ROADMAP 619）：訪客給登入提示；登入者送無參數權威意圖＋冷卻。
// 真正結果（澆了幾株／太遠走近）由伺服器單播 ServerMsg::Chat 回報，下面 handleServerMsg 在
// 短窗內把那則系統回覆浮成 toast＝誠實回饋（不過度宣稱、也不假裝站在田邊就一定有作物可照料）。
function tryTend(kind) {
  if (!isLoggedIn) {
    flashToast(kind === "harvest"
      ? "🌾 在 but-fun.com 登入並擁有農地後，就能在 3D 裡收成"
      : "💧 在 but-fun.com 登入並擁有農地後，就能在 3D 裡澆水");
    return;
  }
  const now = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
  if (now - lastTendAt < TEND_COOLDOWN_MS) return; // 冷卻內忽略連點
  const ok = safeSend(kind === "harvest" ? harvestAllWireMsg() : waterAllWireMsg());
  if (ok) {
    lastTendAt = now;
    awaitTendReplyUntil = now + TEND_REPLY_WINDOW_MS; // 開窗等伺服器權威回報，浮成 toast
  }
}

// 嘗試生篝火（ROADMAP 623）：無須登入（連線玩家即可），送無參數權威意圖＋本地冷卻＋樂觀飄字。
// 真正成敗（冷卻中／達全服上限）由伺服器決定；成功的火會在下一份快照的 campfires 冒出來、
// 由既有 campfireVisual 畫成跳動火堆＋暖意圈（ROADMAP 616 已會畫），故此處只送意圖。
function tryLightCampfire() {
  const now = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
  if (now - lastCampfireAt < CAMPFIRE_COOLDOWN_MS) return; // 冷卻內忽略連點
  if (safeSend(campfireWireMsg())) {
    lastCampfireAt = now;
    flashToast("🔥 升起一堆篝火——火光暖意把附近的野獸逼退，圍出一塊喘息的安全角落（燒完即熄）");
  }
}

// 嘗試採集（ROADMAP 629）：依自己最新權威世界座標算腳邊有沒有可採目標；有就送對應權威意圖
// ＋本地冷卻＋樂觀飄字，沒有就給一句「走近資源」的溫和提示（誠實、不假裝採到）。真正成敗
// （太遠／白天採星晶／節點剛被採光）由伺服器裁決，採到的資源會在下一份背包快照反映。
function tryGather() {
  const target = gatherTargetAt(latestSelfWorld, latestNodes, latestCrystals);
  if (!target) { flashToast("⛏️ 走到樹／礦脈／星晶旁，才能採集"); return; }
  const now = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
  if (now - lastGatherAt < GATHER_COOLDOWN_MS) return; // 冷卻內忽略連點
  const wire = target.kind === "crystal" ? gatherStarCrystalWireMsg() : gatherWireMsg();
  if (safeSend(wire)) {
    lastGatherAt = now;
    flashToast(target.kind === "crystal" ? "💎 採下一道星晶礦脈" : "⛏️ 採集了腳邊的資源");
  }
}

// 每幀依「腳邊有沒有可採目標」刷新採集鈕外觀（純讀 latest* 算 gatherTargetAt）。
// 只在 label／鎖定態真的變動時才改寫 DOM（省排版）；無 widget／壞值一律安全靜默
// （守 render-loop-resilience）。供 render 迴圈每幀呼叫。
let gatherBtnLastLabel = null, gatherBtnLastLocked = null;
function updateGatherBtn() {
  const btn = document.getElementById("gatherBtn");
  if (!btn) return; // 舊頁／測試 DOM 無此鈕 → 靜默跳過
  const target = gatherTargetAt(latestSelfWorld, latestNodes, latestCrystals);
  setBtnShown(btn, !!target); // 情境感知：腳邊有可採目標才顯示
  if (!target) return;
  const st = gatherButtonState(target);
  if (st.label !== gatherBtnLastLabel) { btn.textContent = st.label; gatherBtnLastLabel = st.label; }
  if (st.locked !== gatherBtnLastLocked) { btn.classList.toggle("locked", st.locked); gatherBtnLastLocked = st.locked; }
  btn.title = st.hint;
}

// ============================================================
// 在 3D 裡揮劍迎敵（ROADMAP 632）：3D 世界第一次能「主動戰鬥」——走近敵人按鍵／點鈕揮出一擊。
// 直接回應 3D 大弧裡早已存在卻打不到的張力：世界事件（631）把獸潮／裂縫守護者灌進世界、夜裡敵人
// 加速逼近，敵人有血條（626）卻只能挨打逃跑——現在終於能反擊、能自保、能替家園迎戰。
//   · 純前端、純送既有權威意圖：攻擊一律送 {type:"attack"}（與 2D web/game.js 同協議，無座標）；
//     零後端／協議／world-core 改動。
//   · 伺服器才是權威：攻擊不帶目標——伺服器一律用攻擊者自己的權威座標挑 ATTACK_REACH 內最近的
//     存活敵人結算傷害＋掉落（防隔空攻擊）。前端只負責「走近了才點亮鈕、送出意圖、揮出弧光」，
//     打不打得到、傷害多少由後端裁決，敵人血條／消失由下一份快照反映（626 已會畫）。
//   · 命中飄字：伺服器命中即全服廣播 AttackHit（387），在命中處浮起傷害數字＝戰鬥力道第一次「看得見」。
//   · 攻擊不需登入（連線玩家即可自保，鏡像 2D／生火的誠實態）。
// ============================================================
const ATTACK_REACH = 64;          // 近戰判定半徑（px）——對齊後端 enemy_field::ATTACK_REACH／2D web/game.js
const ATTACK_COOLDOWN_MS = 600;   // 本地冷卻（連點上限；真正可否出手由後端 ATTACK_COOLDOWN_SECS 裁決）
let lastAttackAt = -1e9;          // 上次攻擊的本地時戳（手感防呆；後端另有權威冷卻）

// 攻擊意圖 wire（與 2D 同協議，無座標——伺服器用攻擊者權威座標挑最近敵人）。抽純函式供 smoke 斷言。
function attackWireMsg() { return { type: "attack" }; }

// 攻擊目標判定（純函式、確定性、可測）：給自己的世界座標＋本份活著的敵人快照（世界 px），
// 回最近且落在 ATTACK_REACH 內的存活敵人。死掉的（alive=false）／壞座標一律跳過；無自己座標／
// 空快照安全回 null（不 throw、不誤點亮鈕；守 render-loop-resilience）。
function attackTargetAt(self, enemies) {
  if (!self || !Number.isFinite(self.x) || !Number.isFinite(self.y)) return null;
  let best = null, bestD = ATTACK_REACH * ATTACK_REACH;
  for (const e of (Array.isArray(enemies) ? enemies : [])) {
    if (!e || e.alive === false) continue;
    if (!Number.isFinite(e.x) || !Number.isFinite(e.y)) continue;
    const dx = e.x - self.x, dy = e.y - self.y, d = dx * dx + dy * dy;
    if (d <= bestD) { bestD = d; best = e; }
  }
  if (!best) return null;
  return { x: best.x, y: best.y, kind: best.kind, hp: best.hp, maxHp: best.max_hp, notorious: !!best.notorious };
}

// 攻擊鈕的顯示狀態（純函式、面向玩家字串集中可 i18n、供 smoke 真值表）。
// 走近敵人才亮（兇名精英給更急迫的字樣），否則鎖定提示「走近敵人」。
function attackButtonState(target) {
  if (!target) return { label: "⚔️ 攻擊", locked: true, hint: "走近敵人，就能揮劍迎戰" };
  if (target.notorious) return { label: "💢 迎戰兇敵", locked: false, hint: "向腳邊的兇名精英揮出一擊（武器在手傷害更高）" };
  return { label: "⚔️ 攻擊", locked: false, hint: "向腳邊的敵人揮出一擊（武器在手傷害更高）" };
}

// 嘗試攻擊（ROADMAP 632）：依自己最新權威世界座標算腳邊有沒有可攻擊的敵人；有就送 {type:"attack"}
// ＋本地冷卻＋揮出弧光（純表現），沒有就給一句「走近敵人」的溫和提示（誠實、不假裝揮到）。真正成敗
// （太遠／冷卻中／敵人剛被打倒）由伺服器裁決，傷害與敵人血條／消失在下一份快照／AttackHit 廣播反映。
function tryAttack() {
  const target = attackTargetAt(latestSelfWorld, latestEnemies);
  if (!target) { flashToast("⚔️ 走近敵人，才能揮劍迎戰"); return; }
  const now = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
  if (now - lastAttackAt < ATTACK_COOLDOWN_MS) return; // 冷卻內忽略連點
  if (safeSend(attackWireMsg())) {
    lastAttackAt = now;
    spawnMeleeSwing(latestSelfWorld, target); // 朝目標方向揮出一道弧光（純表現、樂觀；命中與否看後端）
  }
}

// 每幀依「腳邊有沒有可攻擊的敵人」刷新攻擊鈕外觀（純讀 latest* 算 attackTargetAt）。
// 只在 label／鎖定態真的變動時才改寫 DOM；無 widget／壞值一律安全靜默（守 render-loop-resilience）。
let attackBtnLastLabel = null, attackBtnLastLocked = null;
function updateAttackBtn() {
  const btn = document.getElementById("attackBtn");
  if (!btn) return; // 舊頁／測試 DOM 無此鈕 → 靜默跳過
  const target = attackTargetAt(latestSelfWorld, latestEnemies);
  setBtnShown(btn, !!target); // 情境感知：腳邊有敵人才顯示
  if (!target) return;
  const st = attackButtonState(target);
  if (st.label !== attackBtnLastLabel) { btn.textContent = st.label; attackBtnLastLabel = st.label; }
  if (st.locked !== attackBtnLastLocked) { btn.classList.toggle("locked", st.locked); attackBtnLastLocked = st.locked; }
  btn.title = st.hint;
}

// ── 揮砍弧光（純表現）：單一可重用 Torus 弧 mesh，攻擊時定位在自己腳下、朝目標方向掃過一個弧並淡出。
//    單一 mesh、零逐擊配置；reduceMotion 下不掃只給一道靜態淡出閃光（尊重偏好）。fake-THREE 安全。
let meleeSwing = null;            // 弧光 mesh（首次攻擊時惰性建立，避免測試 DOM 無 scene 時報錯）
let meleeSwingStartMs = -1e9;     // 本次揮砍起始本地時戳
let meleeSwingAngle = 0;          // 朝目標方向（場景 XZ 平面）
const MELEE_SWING_MS = 280;       // 弧光存活時間
function spawnMeleeSwing(self, target) {
  if (!self || !target) return;
  if (!meleeSwing) {
    meleeSwing = new THREE.Mesh(
      new THREE.TorusGeometry(3.2, 0.5, 6, 24, Math.PI * 0.9), // 約 160° 的弧（不是整環）＝像一道揮砍
      new THREE.MeshBasicMaterial({ color: 0xfff1a8, transparent: true, opacity: 0, depthWrite: false })
    );
    meleeSwing.visible = false;
    scene.add(meleeSwing);
  }
  meleeSwing.position.set(sx(self.x), 6, sz(self.y));
  meleeSwingAngle = Math.atan2(sx(target.x) - sx(self.x), sz(target.y) - sz(self.y));
  meleeSwingStartMs = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
  meleeSwing.visible = true;
}
function updateMeleeSwing(nowMs) {
  if (!meleeSwing || !meleeSwing.visible) return;
  const age = (nowMs - meleeSwingStartMs) / MELEE_SWING_MS;
  if (age >= 1) { meleeSwing.visible = false; if (meleeSwing.material) meleeSwing.material.opacity = 0; return; }
  meleeSwing.rotation.x = -Math.PI / 2; // 躺平貼在地面平面上
  if (reduceMotion) {
    meleeSwing.rotation.z = meleeSwingAngle;     // 不掃過、固定朝向
    meleeSwing.scale.set(1, 1, 1);
  } else {
    meleeSwing.rotation.z = meleeSwingAngle - 0.9 + age * 1.8; // 掃過一個弧
    const s = 0.7 + age * 0.6;
    meleeSwing.scale.set(s, s, 1);
  }
  if (meleeSwing.material) meleeSwing.material.opacity = (1 - age) * 0.85;
}

// ── 命中傷害飄字（ROADMAP 387／632）：AttackHit 全服廣播驅動，在命中處浮起傷害數字＋淡出上飄。
//    任何人打中敵人都看得到＝3D 世界第一次「看得見」戰鬥的力道。貼圖 FIFO 快取＋飄字數上限＝有界。
const dmgTexCache = new Map();
const DMG_TEX_MAX = 48;
function damageTexture(text, css) {
  const key = css + "|" + text;
  let tex = dmgTexCache.get(key);
  if (tex) return tex;
  const canvas = document.createElement("canvas");
  canvas.width = 256; canvas.height = 128;
  const ctx = canvas.getContext("2d");
  ctx.font = "bold 72px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  ctx.lineWidth = 8; ctx.strokeStyle = "rgba(20,12,0,0.85)"; // 深描邊，亮底/暗底都讀得到
  ctx.strokeText(text, 128, 64);
  ctx.fillStyle = css;
  ctx.fillText(text, 128, 64);
  tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  if (dmgTexCache.size >= DMG_TEX_MAX) { // FIFO 汰換、釋放 GPU 資源
    const k = dmgTexCache.keys().next().value;
    const o = dmgTexCache.get(k);
    if (o && o.dispose) o.dispose();
    dmgTexCache.delete(k);
  }
  dmgTexCache.set(key, tex);
  return tex;
}

// 傷害飄字的文字與配色（純函式、確定性、壞值安全、可測）：暴擊／滿蓄更大更橘紅、破綻直擊金、
// 半蓄橘、擊殺加 💀；缺欄位／壞 dmg 安全降級為「0」普通飄字（不 throw）。
function damageFloatSpec(ev) {
  const dmg = (ev && Number.isFinite(ev.dmg)) ? Math.max(0, Math.floor(ev.dmg)) : 0;
  const crit = !!(ev && ev.is_crit);
  const weak = !!(ev && ev.is_weak);
  const kill = !!(ev && ev.is_kill);
  const tier = (ev && Number.isFinite(ev.charge_tier)) ? ev.charge_tier : 0;
  let text = String(dmg);
  if (weak) text = "❉" + text;        // 破綻直擊：金色 ❉（鏡像 2D ROADMAP 489）
  if (crit) text = text + "!";         // 暴擊：驚嘆
  if (kill) text = text + " 💀";       // 擊殺
  let css = "#ffe9a8";                 // 普通：暖黃
  if (weak) css = "#ffd34d";           // 破綻直擊：金
  else if (crit || tier >= 2) css = "#ff7a5a"; // 暴擊／滿蓄：橘紅
  else if (tier >= 1) css = "#ffb070"; // 半蓄：橘
  const scale = (crit || tier >= 2) ? 1.5 : (weak ? 1.3 : 1.0);
  return { text, css, scale };
}

const damageFloats = [];            // 暫態傷害飄字 sprite 清單
const DAMAGE_FLOAT_MS = 750;        // 飄字存活時間
const DAMAGE_FLOAT_MAX = 24;        // 同時飄字上限（混戰不致無界累積）
function spawnDamageFloat(ev) {
  if (!ev || !Number.isFinite(ev.ex) || !Number.isFinite(ev.ey)) return; // 缺命中座標：安全跳過
  const spec = damageFloatSpec(ev);
  const tex = damageTexture(spec.text, spec.css);
  const sprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: false, opacity: 1 }));
  const h = 6 * spec.scale;
  sprite.scale.set(h * 2, h, 1);
  sprite.position.set(sx(ev.ex), 9, sz(ev.ey));
  scene.add(sprite);
  const nowMs = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
  damageFloats.push({ sprite, startMs: nowMs, baseY: 9 });
  while (damageFloats.length > DAMAGE_FLOAT_MAX) { // 超量丟最舊
    const old = damageFloats.shift();
    scene.remove(old.sprite);
    if (old.sprite.material && old.sprite.material.dispose) old.sprite.material.dispose();
  }
}
function updateDamageFloats(nowMs) {
  for (let i = damageFloats.length - 1; i >= 0; i--) {
    const f = damageFloats[i];
    const age = (nowMs - f.startMs) / DAMAGE_FLOAT_MS;
    if (age >= 1) {
      scene.remove(f.sprite);
      if (f.sprite.material && f.sprite.material.dispose) f.sprite.material.dispose();
      damageFloats.splice(i, 1);
      continue;
    }
    if (!reduceMotion) f.sprite.position.y = f.baseY + age * 8; // 緩緩上飄（reduceMotion 下定住）
    if (f.sprite.material) f.sprite.material.opacity = age < 0.6 ? 1 : (1 - (age - 0.6) / 0.4);
  }
}

// ============================================================
// 戰鬥的獎勵回饋（ROADMAP 635）：3D 世界第一次「看得見打贏了得到什麼」——632 把揮劍迎敵接進了 3D，
// 玩家能揮砍、看傷害數字、看敵人倒下，但戰鬥迴圈的「獎勵」那一半至今在 3D 完全沒聲息：殺了怪掉的
// 戰利品（後端擊殺即自動入袋＋私信 LootPickup）、短時間連殺的熱度（KillStreak 私信）——3D 都整個丟掉，
// 玩家打贏了卻不知道得到了什麼、也感受不到連戰連捷的爽快。本切片把這兩個既有廣播接成醒目的飄字：
//   · 戰利品入袋 → 在怪物原位飄出「🪵 木材 ×2」暖米色飄字（鏡像 2D ROADMAP 509）。
//   · 連殺標語 → 在自己頭頂飄出「🔥×N 連殺！／殺意漸濃！／戰意爆發！」，段位越高越橘紅（鏡像 2D 381）。
//   · 純前端、純讀既有私信廣播：零後端／協議／world-core 改動，舊客戶端／舊伺服器自然相容。
//   · 伺服器才是權威：掉落表／連殺判定全由後端裁決，前端只把「已發生的獎勵」顯影成飄字、不嵌任何規則。
// ============================================================

// 戰利品飄字的文字與配色（純函式、確定性、壞值安全、可測）：emoji＋中文名＋×數量、暖米色。
// 缺 item／壞 qty 安全降級（未知物品退 🎁／空名退「戰利品」／負數 clamp 0），永不拋。
function lootFloatSpec(ev) {
  const item = (ev && typeof ev.item === "string") ? ev.item : "";
  const qty = (ev && Number.isFinite(ev.qty)) ? Math.max(0, Math.floor(ev.qty)) : 0;
  const icon = ITEM_LOOK[item] || "🎁";
  const name = ITEM_NAME[item] || item || "戰利品";
  return { text: `${icon} ${name} ×${qty}`, css: "#dcd2b4" }; // 暖米色（鏡像 2D "220,210,180"）
}

// 連殺標語飄字的文字與配色（純函式、確定性、壞值安全、可測）：只給本人（旁觀者／非本人／streak<2 回 null
// ＝不飄），段位 2/4/8 越高越橘紅、文案越熱（鏡像 2D ROADMAP 381 的配色與分級）。
function killStreakFloatSpec(ev, selfId) {
  if (!ev || !ev.player_id || !selfId || ev.player_id !== selfId) return null; // 只給本人（私信語意）
  const s = (Number.isFinite(ev.streak)) ? Math.floor(ev.streak) : 0;
  if (s < 2) return null;                                            // 未達連殺門檻：不飄
  const css = s >= 8 ? "#ff7850" : s >= 4 ? "#ffbe3c" : "#dcff78";   // 8+橘紅／4+琥珀／2+黃綠
  const text = s >= 8 ? `🔥×${s} 戰意爆發！` : s >= 4 ? `🔥×${s} 殺意漸濃！` : `🔥×${s} 連殺！`;
  return { text, css };
}

// 獎勵飄字的文字貼圖（比傷害數字長、用較寬畫布＋較小字級容得下「emoji 名 ×N」整行）。FIFO 快取＋上限＝有界。
const rewardTexCache = new Map();
const REWARD_TEX_MAX = 32;
function rewardTexture(text, css) {
  const key = css + "|" + text;
  let tex = rewardTexCache.get(key);
  if (tex) return tex;
  const canvas = document.createElement("canvas");
  canvas.width = 512; canvas.height = 128;
  const ctx = canvas.getContext("2d");
  ctx.font = "bold 52px system-ui, sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  ctx.lineWidth = 7; ctx.strokeStyle = "rgba(20,12,0,0.85)"; // 深描邊，亮底/暗底都讀得到
  ctx.strokeText(text, 256, 64);
  ctx.fillStyle = css;
  ctx.fillText(text, 256, 64);
  tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  if (rewardTexCache.size >= REWARD_TEX_MAX) { // FIFO 汰換、釋放 GPU 資源
    const k = rewardTexCache.keys().next().value;
    const o = rewardTexCache.get(k);
    if (o && o.dispose) o.dispose();
    rewardTexCache.delete(k);
  }
  rewardTexCache.set(key, tex);
  return tex;
}

const rewardFloats = [];            // 暫態獎勵飄字（戰利品／連殺）sprite 清單
const REWARD_FLOAT_MS = 1100;       // 飄字存活時間（比傷害數字久一點，讓玩家讀得到整行字）
const REWARD_FLOAT_MAX = 16;        // 同時飄字上限（混戰不致無界累積）
// 在世界座標 (wx,wy) 噴一則獎勵飄字。spec 為 null（連殺旁觀者）或缺世界座標一律安全跳過、不拋
//（守 render-loop-resilience）；超量丟最舊、釋放材質。
function spawnRewardFloat(spec, wx, wy) {
  if (!spec || !spec.text || !Number.isFinite(wx) || !Number.isFinite(wy)) return;
  const tex = rewardTexture(spec.text, spec.css);
  const sprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: false, opacity: 1 }));
  const h = 7;
  sprite.scale.set(h * 4, h, 1); // 較寬（容得下整行「emoji 名 ×N」）
  sprite.position.set(sx(wx), 12, sz(wy));
  scene.add(sprite);
  const nowMs = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
  rewardFloats.push({ sprite, startMs: nowMs, baseY: 12 });
  while (rewardFloats.length > REWARD_FLOAT_MAX) {
    const old = rewardFloats.shift();
    scene.remove(old.sprite);
    if (old.sprite.material && old.sprite.material.dispose) old.sprite.material.dispose();
  }
}
function updateRewardFloats(nowMs) {
  for (let i = rewardFloats.length - 1; i >= 0; i--) {
    const f = rewardFloats[i];
    const age = (nowMs - f.startMs) / REWARD_FLOAT_MS;
    if (age >= 1) {
      scene.remove(f.sprite);
      if (f.sprite.material && f.sprite.material.dispose) f.sprite.material.dispose();
      rewardFloats.splice(i, 1);
      continue;
    }
    if (!reduceMotion) f.sprite.position.y = f.baseY + age * 10; // 緩緩上飄（reduceMotion 下定住）
    if (f.sprite.material) f.sprite.material.opacity = age < 0.7 ? 1 : (1 - (age - 0.7) / 0.3);
  }
}

// ============================================================
// 園丁撫慰居民（ROADMAP 634）：3D 世界第一次能「照料 AI 居民的內在需求」——走近一位正有心事的
// 故鄉居民按鍵／點鈕，上前關心、撫慰他低落的心事。這是北極星「人類是訪客／園丁」的核心動詞：
// 居民早有完整的需求系統（npc_needs.rs：安全感／歸屬感／繁榮感，#554 已在 2D 接成「💚 關心」），
// 3D 至今只把「這位需要被關心」的 💚 側標顯影出來（updateResidentStatus），卻沒有一把可以照料的鋤頭。
// 本切片補上那個缺席的動詞——把園丁照料居民的雙向互動接進 3D。
//   · 純前端、純送既有權威意圖：關心一律送 {type:"comfort_resident"}（與 2D web/game.js 同協議，無 payload）；
//     零後端／協議／world-core 改動。
//   · 伺服器才是權威：撫慰不帶目標——伺服器一律用玩家自己的權威座標挑 COMFORT_REACH 內最近、確有
//     偏低心事者，把那項需求往上推一點（防隔空關心）。前端只負責「走近了才點亮鈕、送出意圖」，
//     成不成、推多少由後端裁決；居民領情的回應由後端廣播 NpcSpeech，3D 既有對話泡泡會自動顯現（622）。
//   · 撫慰不需登入（連線玩家即可當園丁照料居民，鏡像 2D web/game.js／生火的誠實態；後端只用
//     玩家 id 取權威座標＋驗未倒地/冷卻到期，不另檢登入）。
// ============================================================
const COMFORT_REACH = 130;        // 撫慰判距半徑（px）——對齊後端 ws.rs COMFORT_REACH／2D web/game.js COMFORT_REACH_PX
const COMFORT_COOLDOWN_MS = 5000; // 本地冷卻（防洗泡泡；真正可否撫慰由後端 COMFORT_COOLDOWN_SECS 裁決）
let lastComfortAt = -1e9;         // 上次撫慰的本地時戳（手感防呆；後端另有權威冷卻）

// 撫慰意圖 wire（與 2D 同協議，無 payload——伺服器用玩家權威座標挑最近的有心事居民）。抽純函式供 smoke 斷言。
function comfortWireMsg() { return { type: "comfort_resident" }; }

// 撫慰目標判定（純函式、確定性、可測）：給自己的世界座標＋本份居民快照（世界 px），
// 回最近且落在 COMFORT_REACH 內、且正有心事（needs_care）的居民。沒心事的／壞座標一律跳過；
// 無自己座標／空快照安全回 null（不 throw、不誤點亮鈕；守 render-loop-resilience）。
function comfortTargetAt(self, npcList) {
  if (!self || !Number.isFinite(self.x) || !Number.isFinite(self.y)) return null;
  let best = null, bestD = COMFORT_REACH * COMFORT_REACH;
  for (const n of (Array.isArray(npcList) ? npcList : [])) {
    if (!n || !n.needs_care) continue;
    if (!Number.isFinite(n.x) || !Number.isFinite(n.y)) continue;
    const dx = n.x - self.x, dy = n.y - self.y, d = dx * dx + dy * dy;
    if (d <= bestD) { bestD = d; best = n; }
  }
  if (!best) return null;
  return { id: best.id, name: best.name, x: best.x, y: best.y };
}

// 關心鈕的顯示狀態（純函式、面向玩家字串集中可 i18n、供 smoke 真值表）。
// 走近一位有心事的居民才亮（提示帶上對方名字，園丁知道在關心誰），否則鎖定提示「走近有心事的居民」。
function comfortButtonState(target) {
  if (!target) return { label: "💚 關心", locked: true, hint: "走近一位正有心事的故鄉居民，就能上前關心" };
  const who = (target.name && typeof target.name === "string") ? target.name : "這位居民";
  return { label: "💚 關心", locked: false, hint: `上前撫慰 ${who} 低落的心事` };
}

// 嘗試撫慰（ROADMAP 634）：依自己最新權威世界座標算腳邊有沒有正有心事的居民；有就送
// {type:"comfort_resident"}＋本地冷卻，沒有就給一句「走近有心事的居民」的溫和提示（誠實、不假裝關心到）。
// 真正成敗（太遠／冷卻中／那位其實沒事了）由伺服器裁決；居民領情的回應由後端廣播 NpcSpeech、
// 3D 既有對話泡泡（622）會自動顯現＝撫慰被聽見了。
function tryComfort() {
  const target = comfortTargetAt(latestSelfWorld, latestNpcs);
  if (!target) { flashToast("💚 走近一位正有心事的居民，才能上前關心"); return; }
  const now = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
  if (now - lastComfortAt < COMFORT_COOLDOWN_MS) return; // 冷卻內忽略連點
  if (safeSend(comfortWireMsg())) {
    lastComfortAt = now;
    const who = (target.name && typeof target.name === "string") ? target.name : "居民";
    flashToast(`💚 你上前關心了 ${who}`);
  }
}

// 每幀依「腳邊有沒有有心事的居民」刷新關心鈕外觀（純讀 latest* 算 comfortTargetAt）。
// 只在 label／鎖定態真的變動時才改寫 DOM；無 widget／壞值一律安全靜默（守 render-loop-resilience）。
let comfortBtnLastLabel = null, comfortBtnLastLocked = null;
function updateComfortBtn() {
  const btn = document.getElementById("comfortBtn");
  if (!btn) return; // 舊頁／測試 DOM 無此鈕 → 靜默跳過
  const target = comfortTargetAt(latestSelfWorld, latestNpcs);
  setBtnShown(btn, !!target); // 情境感知：腳邊有有心事的居民才顯示
  if (!target) return;
  const st = comfortButtonState(target);
  if (st.label !== comfortBtnLastLabel) { btn.textContent = st.label; comfortBtnLastLabel = st.label; }
  if (st.locked !== comfortBtnLastLocked) { btn.classList.toggle("locked", st.locked); comfortBtnLastLocked = st.locked; }
  btn.title = st.hint;
}

// ============================================================
// 居民互助（ROADMAP 125）：把既有「協助正在求助的居民」接進 3D——3D 世界第一次能「伸手幫一位
// 開口求助的居民、領他一句道謝＋一點乙太」。撫慰（634）是回應「心事低落」的內在需求；互助則是
// 回應居民「主動開口的具體請求」（active_help_requests），是 3D 弧裡又一個「對 AI 居民做事」的動詞。
//   · 伺服器才是權威：協助送既有意圖 {type:"help_resident", resident_id}（與 2D web/game.js 同協議）；
//     伺服器用玩家自己的權威座標判距（RESIDENT_REACH=80）＋確認該居民確在求助，原子完成（只有第一個
//     點到的玩家成功），給 HELP_REWARD_ETHER 乙太、廣播居民道謝 NpcSpeech 泡泡（3D 既有對話泡泡會顯現）。
//   · 純前端、零後端改動、零協議改動——active_help_requests／help_resident／NpcView 座標本來就在快照／協議裡。
// ============================================================
const HELP_REACH = 80;            // 協助判距半徑（px）——對齊後端 resident_npc::RESIDENT_REACH／2D RESIDENT_REACH_PX
const HELP_REWARD_ETHER = 8;      // 協助報酬乙太（鏡像後端 resident_npc::HELP_REWARD_ETHER；僅供前端提示文案）
const HELP_COOLDOWN_MS = 1200;    // 本地冷卻（防連點；真正成敗由後端原子完成裁決）
let lastHelpAt = -1e9;            // 上次協助的本地時戳（手感防呆）

// 協助意圖 wire（送哪位居民的 id；伺服器仍用玩家權威座標複驗距離防隔空）。抽純函式供 smoke 斷言。
function helpWireMsg(residentId) { return { type: "help_resident", resident_id: residentId }; }

// 協助目標判定（純函式、確定性、可測）：給自己的世界座標＋本份居民快照（世界 px）＋「正在求助的 id 集合」，
// 回最近且落在 HELP_REACH 內、且 id 確在求助集合裡的居民。非求助中的／壞座標一律跳過；
// 無自己座標／空快照／空集合安全回 null（不 throw、不誤點亮鈕；守 render-loop-resilience）。
function helpTargetAt(self, npcList, helpSet) {
  if (!self || !Number.isFinite(self.x) || !Number.isFinite(self.y)) return null;
  // 鴨子型別判 Set（有 has() 即可）——比 instanceof 穩，跨 realm（如冒煙測試 vm 沙箱）也成立；
  // 非 Set 則當陣列轉成 Set。空集合（無人求助）→ 安全回 null，不點亮鈕。
  const reqs = (helpSet && typeof helpSet.has === "function") ? helpSet : new Set(Array.isArray(helpSet) ? helpSet : []);
  if (reqs.size === 0) return null;
  let best = null, bestD = HELP_REACH * HELP_REACH;
  for (const n of (Array.isArray(npcList) ? npcList : [])) {
    if (!n || !n.id || !reqs.has(n.id)) continue;
    if (!Number.isFinite(n.x) || !Number.isFinite(n.y)) continue;
    const dx = n.x - self.x, dy = n.y - self.y, d = dx * dx + dy * dy;
    if (d <= bestD) { bestD = d; best = n; }
  }
  if (!best) return null;
  return { id: best.id, name: best.name, x: best.x, y: best.y };
}

// 幫忙鈕的顯示狀態（純函式、面向玩家字串集中可 i18n、供 smoke 真值表）。
// 走近一位正在求助的居民才亮（提示帶上對方名字＋乙太報酬），否則鎖定提示「走近一位正在求助的居民」。
function helpButtonState(target) {
  if (!target) return { label: "🤝 幫忙", locked: true, hint: "走近一位正開口求助的居民，就能上前幫忙" };
  const who = (target.name && typeof target.name === "string") ? target.name : "這位居民";
  return { label: "🤝 幫忙", locked: false, hint: `幫助 ${who}（+${HELP_REWARD_ETHER} 乙太）` };
}

// 嘗試協助（ROADMAP 125）：依自己最新權威世界座標算腳邊有沒有正在求助的居民；有就送
// {type:"help_resident", resident_id}＋本地冷卻，沒有就給一句溫和提示（誠實、不假裝幫到）。
// 真正成敗（太遠／已被別人搶先完成）由伺服器裁決；居民領情的回應由後端廣播 NpcSpeech、3D 既有對話泡泡會顯現。
function tryHelp() {
  const target = helpTargetAt(latestSelfWorld, latestNpcs, latestHelpRequests);
  if (!target) { flashToast("🤝 走近一位正開口求助的居民，才能上前幫忙"); return; }
  const now = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
  if (now - lastHelpAt < HELP_COOLDOWN_MS) return; // 冷卻內忽略連點
  if (safeSend(helpWireMsg(target.id))) {
    lastHelpAt = now;
    const who = (target.name && typeof target.name === "string") ? target.name : "居民";
    flashToast(`🤝 你上前幫了 ${who} 一把`);
  }
}

// 每幀依「腳邊有沒有正在求助的居民」刷新幫忙鈕外觀（純讀 latest* 算 helpTargetAt）。
// 只在 label／鎖定態真的變動時才改寫 DOM；無 widget／壞值一律安全靜默（守 render-loop-resilience）。
let helpBtnLastLabel = null, helpBtnLastLocked = null;
function updateHelpBtn() {
  const btn = document.getElementById("helpResidentBtn");
  if (!btn) return; // 舊頁／測試 DOM 無此鈕 → 靜默跳過
  const target = helpTargetAt(latestSelfWorld, latestNpcs, latestHelpRequests);
  setBtnShown(btn, !!target); // 情境感知：腳邊有正在求助的居民才顯示
  if (!target) return;
  const st = helpButtonState(target);
  if (st.label !== helpBtnLastLabel) { btn.textContent = st.label; helpBtnLastLabel = st.label; }
  if (st.locked !== helpBtnLastLocked) { btn.classList.toggle("locked", st.locked); helpBtnLastLocked = st.locked; }
  btn.title = st.hint;
}

// ============================================================
// 跟 AI 居民／城鎮大人物搭話（ROADMAP 636）：3D 世界第一次能「主動開口、聽見 AI 居民回話」。
// 這正是北極星「人類是訪客／園丁、走進一個由 AI 居住的世界」的核心動詞——你能上前攀談，居民會
// 依此刻處境（季節／天氣／繁榮／生態警戒，後端 resident_chat／topic 層）回你一句。3D 至今只能
// 被動看居民彼此互聊、向你打招呼（NpcSpeech 泡泡 622），卻沒有一張可以開口的嘴；本切片補上它。
//   · 純前端、純送既有權威意圖：居民走 {type:"talk_to_resident", resident_id}、城鎮大人物走
//     {type:"talk_to_major_npc", npc_id}（皆與 2D web/game.js 同協議）；零後端／協議／world-core 改動。
//   · 伺服器才是權威：搭話帶對象 id，但成不成由後端用玩家權威座標判距（防隔空攀談）；回話由後端
//     單播 NpcReply 給本人（下面浮成 toast），同句再廣播 NpcSpeech、既有對話泡泡（622）自動在對方頭頂顯現。
//   · 成本紀律（鐵律）：零 LLM——talk_to_resident／talk_to_major_npc 都是純模板／話題層查表，不燒任何額度。
//   · 搭話不需登入（連線玩家即可當訪客攀談；後端只用玩家 id 取權威座標＋判距，不檢登入）。
// ============================================================
const TALK_RESIDENT_REACH = 80;   // 居民搭話判距（px）——對齊後端 resident_npc::RESIDENT_REACH／2D RESIDENT_REACH_PX
const TALK_MAJOR_REACH = 96;      // 城鎮大人物攀談判距（px）——對齊後端 npc::SHOP_REACH／2D SHOP_REACH_PX
const TALK_COOLDOWN_MS = 1200;    // 本地冷卻（防連點洗泡泡；真正可否由後端裁決）
let lastTalkAt = -1e9;            // 上次搭話的本地時戳（手感防呆）

// 城鎮大人物穩定 id（鏡像 2D web/game.js MAJOR_NPC_IDS）——這些走 talk_to_major_npc 攀談。
const MAJOR_NPC_IDS = new Set(["merchant", "workshop_npc", "bounty_npc", "expedition_npc", "procurement_npc", "farm_fair_npc", "village_chief", "traveler"]);

// 純函式：判斷一位 NPC 屬於哪種可搭話對象——"resident"（resident_N，走 talk_to_resident）／
// "major"（城鎮大人物，走 talk_to_major_npc）／null（他星商人／一般路人＝不可搭話，鏡像 2D）。
function talkKindOf(npc) {
  if (!npc || typeof npc.id !== "string") return null;
  if (npc.id.startsWith("resident_")) return "resident";
  if (MAJOR_NPC_IDS.has(npc.id)) return "major";
  return null;
}

// 純函式、確定性、可測：給自己的世界座標＋本份 npcs 快照，回「伸手範圍內最近、可搭話的對象」。
// 居民用 RESIDENT_REACH、大人物用 MAJOR_REACH（各自對齊後端判距）；先以「落在各自範圍內」為門檻，
// 再比實際距離取最近。不可搭話者／壞座標一律跳過；無自己座標／空快照安全回 null（不誤點亮鈕、不 throw）。
function talkTargetAt(self, npcList) {
  if (!self || !Number.isFinite(self.x) || !Number.isFinite(self.y)) return null;
  let best = null, bestD = Infinity;
  for (const n of (Array.isArray(npcList) ? npcList : [])) {
    const kind = talkKindOf(n);
    if (!kind) continue;
    if (!Number.isFinite(n.x) || !Number.isFinite(n.y)) continue;
    const reach = kind === "resident" ? TALK_RESIDENT_REACH : TALK_MAJOR_REACH;
    const dx = n.x - self.x, dy = n.y - self.y, d = dx * dx + dy * dy;
    if (d <= reach * reach && d < bestD) { bestD = d; best = { id: n.id, name: n.name, kind }; }
  }
  return best;
}

// 純函式：搭話意圖 wire——依對象種類送對應既有協議（居民 talk_to_resident、大人物 talk_to_major_npc）。
// 非法／未知對象回 null（呼叫端不送）。供 smoke 斷言。
function talkWireMsg(target) {
  if (!target || typeof target.id !== "string") return null;
  if (target.kind === "resident") return { type: "talk_to_resident", resident_id: target.id };
  if (target.kind === "major") return { type: "talk_to_major_npc", npc_id: target.id };
  return null;
}

// 純函式：搭話鈕顯示狀態（走近可搭話對象才亮、提示帶名字；否則鎖定提示）。字串集中可 i18n，供 smoke 真值表。
function talkButtonState(target) {
  if (!target) return { label: "💬 搭話", locked: true, hint: "走近一位居民或城鎮裡的人，就能上前攀談" };
  const who = (target.name && typeof target.name === "string") ? target.name : "這位";
  return { label: "💬 搭話", locked: false, hint: `上前和 ${who} 攀談，聽聽他此刻想說什麼` };
}

// 嘗試搭話（ROADMAP 636）：依自己最新權威世界座標算腳邊有沒有可搭話對象；有就送對應既有意圖＋本地冷卻
// ＋一句「你開口了」的飄字，沒有就給溫和提示（誠實、不假裝攀談到）。對方的回話由伺服器送 NpcReply
// （單播本人，下面 handleServerMsg 浮成 toast）＋廣播 NpcSpeech（既有對話泡泡 622 自動在他頭頂顯現）。
function tryTalk() {
  const target = talkTargetAt(latestSelfWorld, latestNpcs);
  if (!target) { flashToast("💬 走近一位居民或城鎮裡的人，才能攀談"); return; }
  const now = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
  if (now - lastTalkAt < TALK_COOLDOWN_MS) return; // 冷卻內忽略連點
  const msg = talkWireMsg(target);
  if (msg && safeSend(msg)) {
    lastTalkAt = now;
    const who = (target.name && typeof target.name === "string") ? target.name : "對方";
    flashToast(`💬 你向 ${who} 開口攀談…`);
  }
}

// 每幀依「腳邊有沒有可搭話對象」刷新搭話鈕外觀（純讀 latest* 算 talkTargetAt）。
// 只在 label／鎖定態真的變動時才改寫 DOM；無 widget／壞值一律安全靜默（守 render-loop-resilience）。
let talkBtnLastLabel = null, talkBtnLastLocked = null;
function updateTalkBtn() {
  const btn = document.getElementById("talkBtn");
  if (!btn) return; // 舊頁／測試 DOM 無此鈕 → 靜默跳過
  const target = talkTargetAt(latestSelfWorld, latestNpcs);
  setBtnShown(btn, !!target); // 情境感知：腳邊有可搭話對象才顯示
  if (!target) return;
  const st = talkButtonState(target);
  if (st.label !== talkBtnLastLabel) { btn.textContent = st.label; talkBtnLastLabel = st.label; }
  if (st.locked !== talkBtnLastLocked) { btn.classList.toggle("locked", st.locked); talkBtnLastLocked = st.locked; }
  btn.title = st.hint;
}

// ============================================================
// 送一份心意給故鄉居民（ROADMAP 639）：3D 弧「對 AI 居民做事」北極星軸再添一個動詞——
// 你能從背包拿一樣東西，親手送給走到跟前的故鄉七大居民。638 才讓你旁觀「居民彼此互助送禮」；
// 本片讓**你自己加入這份贈禮經濟**：送禮加深你倆的交情（複用撫慰同一份相熟度帳本＝「居民記得你的
// 照料」，你常帶禮來的居民日後會認得你、點名招呼），居民就地道謝。撫慰回應「沒說出口的低落」、
// 搭話是「閒聊」、幫忙回應「主動開口的請求」，送禮則是「實打實掏出一份心意」——又一個更主動的動詞。
//   · 伺服器才是權威：送禮**不帶目標 id**（與撫慰同模式），只帶要送的物品；伺服器用玩家權威座標
//     挑 GIFT_REACH 內最近的居民（防隔空）、扣 1 件、記交情、廣播道謝 NpcSpeech（既有泡泡 622 顯現）。
//   · 零 LLM（道謝語走後端 npc_agent 模板）、零經濟產出（純把材料化作交情，不發任何乙太/物品回報）。
// ============================================================
const GIFT_REACH = 130;        // 送禮判距（px）——對齊後端 ws.rs GIFT_REACH／撫慰 COMFORT_REACH
const GIFT_COOLDOWN_MS = 1200; // 本地冷卻（防連點；真正成敗由後端裁決）
let lastGiftAt = -1e9;         // 上次送禮的本地時戳（手感防呆）
// 故鄉七大居民穩定 id（鏡像後端 npc_schedule::VILLAGE_NPCS）——送禮對象就是這七位你朝夕相處的居民。
const GIFT_NPC_IDS = new Set(["merchant", "workshop_npc", "bounty_npc", "expedition_npc", "procurement_npc", "farm_fair_npc", "village_chief"]);

// 純函式、確定性、可測：從自己的背包挑「要送出的那件物品」——優先送庫存最多的（最不稀缺，避免誤送
// 唯一的武器/工具），同量時以 item key 字典序定序（確定性）。空背包／無正數庫存安全回 null。供 smoke 斷言。
function giftPickItem(inventory) {
  const rows = Array.isArray(inventory) ? inventory : [];
  let best = null;
  for (const r of rows) {
    if (!r || typeof r.item !== "string") continue;
    const q = Number.isFinite(r.qty) ? r.qty : 0;
    if (q <= 0) continue;
    if (!best || q > best.qty || (q === best.qty && r.item < best.item)) best = { item: r.item, qty: q };
  }
  return best ? best.item : null;
}

// 純函式、確定性、可測：給自己的世界座標＋本份 npcs 快照，回「GIFT_REACH 內最近的故鄉七大居民」。
// 非七大居民（他星商人／路人居民 resident_N／旅人）／壞座標一律跳過；無自己座標／空快照安全回 null
// （不誤點亮鈕、不 throw；守 render-loop-resilience）。
function giftTargetAt(self, npcList) {
  if (!self || !Number.isFinite(self.x) || !Number.isFinite(self.y)) return null;
  let best = null, bestD = GIFT_REACH * GIFT_REACH;
  for (const n of (Array.isArray(npcList) ? npcList : [])) {
    if (!n || typeof n.id !== "string" || !GIFT_NPC_IDS.has(n.id)) continue;
    if (!Number.isFinite(n.x) || !Number.isFinite(n.y)) continue;
    const dx = n.x - self.x, dy = n.y - self.y, d = dx * dx + dy * dy;
    if (d <= bestD) { bestD = d; best = { id: n.id, name: n.name, x: n.x, y: n.y }; }
  }
  return best;
}

// 純函式：送禮意圖 wire——不帶目標 id（伺服器用玩家權威座標挑最近居民，與撫慰同模式），只帶要送的物品。
// 缺物品回 null（呼叫端不送）。供 smoke 斷言。
function giftWireMsg(item) {
  if (typeof item !== "string" || !item) return null;
  return { type: "gift_resident", item };
}

// 純函式：送禮鈕的顯示狀態（字串集中可 i18n、供 smoke 真值表）。走近一位故鄉居民且背包有東西可送才亮
// （提示帶對方名＋要送的物品）；沒對象→鎖定提示「走近居民」；有對象但空背包→鎖定提示「先去採集」。
function giftButtonState(target, item) {
  if (!target) return { label: "🎁 送禮", locked: true, hint: "走近一位故鄉居民，就能送他一份心意" };
  const who = (target.name && typeof target.name === "string") ? target.name : "這位居民";
  if (!item) return { label: "🎁 送禮", locked: true, hint: `背包空空，先去採集些東西，再來送 ${who} 一份心意` };
  return { label: "🎁 送禮", locked: false, hint: `送 ${who} 一份 ${itemLabel(item)}，加深你倆的交情` };
}

// 嘗試送禮（ROADMAP 639）：依自己最新權威世界座標算腳邊有沒有故鄉居民、背包挑一件要送的東西；
// 兩者皆備就送 {type:"gift_resident", item}＋本地冷卻＋一句「你送了什麼給誰」的飄字，否則給溫和提示
// （誠實、不假裝送到）。居民的道謝由伺服器廣播 NpcSpeech、3D 既有對話泡泡（622）自動在他頭頂顯現。
function tryGift() {
  const target = giftTargetAt(latestSelfWorld, latestNpcs);
  if (!target) { flashToast("🎁 走近一位故鄉居民，才能送他一份心意"); return; }
  const item = giftPickItem(latestSelfItem && latestSelfItem.inventory);
  if (!item) { flashToast("🎁 背包空空的，先去採集些東西再來送禮吧"); return; }
  const now = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
  if (now - lastGiftAt < GIFT_COOLDOWN_MS) return; // 冷卻內忽略連點
  const msg = giftWireMsg(item);
  if (msg && safeSend(msg)) {
    lastGiftAt = now;
    const who = (target.name && typeof target.name === "string") ? target.name : "居民";
    flashToast(`🎁 你送了 ${who} 一份 ${itemLabel(item)}`);
  }
}

// 每幀依「腳邊有沒有故鄉居民＋背包有沒有可送物品」刷新送禮鈕外觀（純讀 latest* 算 giftTargetAt/giftPickItem）。
// 只在 label／鎖定態真的變動時才改寫 DOM；無 widget／壞值一律安全靜默（守 render-loop-resilience）。
let giftBtnLastLabel = null, giftBtnLastLocked = null;
function updateGiftBtn() {
  const btn = document.getElementById("giftBtn");
  if (!btn) return; // 舊頁／測試 DOM 無此鈕 → 靜默跳過
  const target = giftTargetAt(latestSelfWorld, latestNpcs);
  // 情境感知：腳邊有故鄉居民就顯示（即使背包空空也顯示成鎖定態，提示「先去採集再來送」）。
  setBtnShown(btn, !!target);
  if (!target) return;
  const item = giftPickItem(latestSelfItem && latestSelfItem.inventory);
  const st = giftButtonState(target, item);
  if (st.label !== giftBtnLastLabel) { btn.textContent = st.label; giftBtnLastLabel = st.label; }
  if (st.locked !== giftBtnLastLocked) { btn.classList.toggle("locked", st.locked); giftBtnLastLocked = st.locked; }
  btn.title = st.hint;
}

// ============================================================
// 城鎮交易（ROADMAP 630）：把既有「新手村商人商店」（#57）接進 3D——3D 世界第一次能「以物易乙太」。
//   · 伺服器才是權威：買賣一律送既有意圖 {type:"shop_buy"|"shop_sell", item, qty}（與 2D web/game.js 同協議），
//     伺服器用玩家權威座標判距（SHOP_REACH=96）、扣乙太/背包、不合法回滾，下一份快照反映真實結果。
//   · 交易需登入（動到乙太/背包＝帳號資產）；訪客可逛目錄、按鈕誠實鎖定不假裝能買賣（鏡像 2D isGuest）。
//   · 純前端、零後端改動、零協議改動——商人 buy_list/sell_list 與我的 ether/inventory 本來就在快照裡。
// ============================================================

// 物品中文名／emoji（鏡像 2D web/game.js 的 ITEM_NAME／ITEM_LOOK；面向玩家字串集中、留 i18n 空間）。
const ITEM_NAME = { wood: "木材", dirt: "土磚", stone: "石頭", ether: "乙太", pickaxe: "鎬子", reinforced_pickaxe: "強化鎬", weapon: "武器", axe: "斧頭", fishing_rod: "釣竿", crystal_shard: "晶石碎片", mushroom_spore: "蕈菇孢子", ancient_fragment: "古代碎片", deep_sea_pearl: "深海珍珠", wildflower_seed: "野花種子", healing_potion: "活力藥水", crystal_potion: "晶石強化液", mushroom_elixir: "蕈菇活化液", ether_pill: "古代乙太丸", pearl_potion: "珍珠復原藥", crystal_blade: "晶石之刃", coral_lance: "珊瑚矛", meadow_amulet: "草原護符", crystal_shield: "晶石護盾", star_chart: "星圖", mushroom_staff: "蕈菇杖", rune_blade: "符文刃", jade_shard: "翠幽碎片", jade_elixir: "翠幽精露", jade_blade: "翠幽刃", lava_crystal: "熔晶碎片", steam_elixir: "蒸汽精粹", crimson_blade: "赤焰刃", void_shard: "虛空碎片", void_elixir: "虛空精粹", void_blade: "虛空刃", aether_shard: "霧醚碎片", aether_essence: "霧醚精粹", aether_blade: "霧醚之刃", origin_shard: "源晶碎片", origin_essence: "源晶精粹", origin_blade: "源晶之刃", rift_shard: "裂縫碎片", cosmic_shield: "宇宙護盾", sprinkler: "灑水器", town_brew: "城鎮特釀", vibrant_elixir: "繁盛精露", wheat_grain: "小麥穗", star_dust: "星塵", star_amulet: "星光護符", rainbow_star_dust: "彩虹星塵", star_guardian_amulet: "星際守護符", star_crystal_shard: "星晶碎片", hardened_blade: "硬化刃", star_crystal_blade: "星晶之刃", rift_blade: "裂縫刃", coral_armor: "珊瑚鎧", rune_armor: "符文鎧", star_crystal_armor: "星晶鎧", ether_bow: "乙太弓", crystal_ballista: "晶石弩", void_cannon: "虛空炮", wild_flower: "野花", solar_shard: "太陽碎片", maple_leaf: "楓葉", ice_shard: "冰晶碎片", spring_sachet: "春日香囊", summer_elixir: "夏日精粹", autumn_tonic: "秋日補藥", winter_medicine: "冬日神藥", steam_bed: "蒸汽床", aether_chest: "乙太箱", ether_plant: "醚草盆栽", star_lantern: "星燈", ancient_deco: "古代裝飾", aquarium: "水族缸", ether_overlord_core: "霸主晶核", ether_overlord_blade: "守城戰刃", alpha_crystal: "Alpha晶石", alpha_force: "Alpha原力", legendary_core: "傳說晶核", legendary_blade: "傳說戰刃", fish_small: "小魚", fish_star: "星星魚", fish_deep: "深海魚", egg: "雞蛋", carrot: "胡蘿蔔", potato: "馬鈴薯", grilled_fish: "烤魚", star_sashimi: "星燦刺身", deep_broth: "深海濃湯", fried_egg: "煎蛋", honey: "蜂蜜", bread: "麵包", carrot_soup: "蔬菜湯", potato_gratin: "焗烤馬鈴薯", night_potion: "夜幻藥水", gold_ore: "黃金礦石" };
const ITEM_LOOK = { wood: "🪵", dirt: "🟫", stone: "🪨", ether: "✨", pickaxe: "⛏️", reinforced_pickaxe: "⚒️", weapon: "🗡️", axe: "🪓", fishing_rod: "🎣", crystal_shard: "💎", mushroom_spore: "🍄", ancient_fragment: "🏺", deep_sea_pearl: "🫧", wildflower_seed: "🌸", healing_potion: "🧪", crystal_potion: "🔮", mushroom_elixir: "🫗", ether_pill: "💊", pearl_potion: "💠", crystal_blade: "🔪", coral_lance: "🔱", meadow_amulet: "🍀", crystal_shield: "🛡️", star_chart: "🗺️", mushroom_staff: "🪄", rune_blade: "⚜️", jade_shard: "🟢", jade_elixir: "🍵", jade_blade: "🗡️", lava_crystal: "🔶", steam_elixir: "🔥", crimson_blade: "🗡️", void_shard: "🔮", void_elixir: "🌌", void_blade: "⚔️", aether_shard: "🌫️", aether_essence: "🔵", aether_blade: "🗡️", origin_shard: "🔮", origin_essence: "✨", origin_blade: "🗡️", rift_shard: "🌀", cosmic_shield: "🌌", sprinkler: "💧", town_brew: "🍺", vibrant_elixir: "🌟", wheat_grain: "🌾", star_dust: "☄️", star_amulet: "🌟", rainbow_star_dust: "🌈", star_guardian_amulet: "🌠", star_crystal_shard: "🔮", hardened_blade: "🗡️", star_crystal_blade: "⚔️", rift_blade: "🌀", coral_armor: "🦞", rune_armor: "🛡️", star_crystal_armor: "✨", ether_bow: "🏹", crystal_ballista: "🎯", void_cannon: "💥", wild_flower: "🌼", solar_shard: "🌞", maple_leaf: "🍁", ice_shard: "🧊", spring_sachet: "🌷", summer_elixir: "☀️", autumn_tonic: "🍂", winter_medicine: "❄️", steam_bed: "🛏️", aether_chest: "📦", ether_plant: "🪴", star_lantern: "🔮", ancient_deco: "🏺", aquarium: "🐠", ether_overlord_core: "💠", ether_overlord_blade: "⚔️", alpha_crystal: "💎", alpha_force: "⚡", legendary_core: "💫", legendary_blade: "🌟", fish_small: "🐟", fish_star: "⭐", fish_deep: "🦈", egg: "🥚", carrot: "🥕", potato: "🥔", grilled_fish: "🍢", star_sashimi: "🍣", deep_broth: "🍲", fried_egg: "🍳", honey: "🍯", bread: "🍞", carrot_soup: "🥣", potato_gratin: "🧀", night_potion: "🌙", gold_ore: "🪙" };
// 物品顯示（emoji + 中文名；未知 key 退回原始字串，留 i18n 空間）。
function itemLabel(key) { return `${ITEM_LOOK[key] || "📦"} ${ITEM_NAME[key] || key}`; }

const SHOP_REACH = 96;                       // 對齊後端 SHOP_REACH／2D web/game.js（走這距離內才能交易）
const SHOP_REACH_SQ = SHOP_REACH * SHOP_REACH;

let shopOpen = false;        // 交易面板是否開著
let lastShopSig = null;      // 面板內容簽章——只在行情/庫存/餘額/背包變動時才重建 DOM（守 panel-sig-stale）

// 純函式：從一份 npcs 快照挑出「商人」（buy_list 或 sell_list 非空者），精簡成交易要用的欄位。
// 確定性、壞值安全（非陣列回 []、缺座標者略過）。供 render 判距與 smoke 斷言。
function shopMerchantsFrom(npcs) {
  if (!Array.isArray(npcs)) return [];
  const out = [];
  for (const n of npcs) {
    if (!n || typeof n !== "object") continue;
    const buy = Array.isArray(n.buy_list) ? n.buy_list : [];
    const sell = Array.isArray(n.sell_list) ? n.sell_list : [];
    if (buy.length === 0 && sell.length === 0) continue;      // 非商人（一般居民／旅人）
    if (!Number.isFinite(n.x) || !Number.isFinite(n.y)) continue; // 壞座標略過
    out.push({ id: n.id, name: n.name || "商人", x: n.x, y: n.y, buy_list: buy, sell_list: sell });
  }
  return out;
}

// 純函式：給自己的世界座標＋商人清單，回「伸手範圍內最近的商人」（無則 null）。
// 確定性、壞值安全（無自己座標／空清單／壞座標一律安全回 null，不誤點亮鈕）。供 render 與 smoke。
function shopTargetAt(self, merchants, reachSq) {
  if (!self || !Number.isFinite(self.x) || !Number.isFinite(self.y)) return null;
  if (!Array.isArray(merchants)) return null;
  const rsq = Number.isFinite(reachSq) ? reachSq : SHOP_REACH_SQ;
  let best = null, bestD = Infinity;
  for (const m of merchants) {
    if (!m || !Number.isFinite(m.x) || !Number.isFinite(m.y)) continue;
    const dx = self.x - m.x, dy = self.y - m.y;
    const d = dx * dx + dy * dy;
    if (d <= rsq && d < bestD) { best = m; bestD = d; }
  }
  return best;
}

// 純函式：交易鈕的顯示狀態（走近商人→可開店；否則鎖定提示）。字串集中可 i18n，供 smoke 真值表。
function shopButtonState(target) {
  if (!target) return { label: "🛒 交易", locked: true, hint: "走近新手村的商人，就能買賣" };
  return { label: "🛒 交易", locked: false, hint: `和 ${target.name} 買賣——賣材料換乙太、花乙太買工具` };
}

// 純函式：面板內容簽章（近哪個商人＋登入態＋我的乙太＋背包＋收購價/趨勢＋販售價/庫存）。
// 只在這些真的變動時才重建 DOM（鏡像 2D lastShopSig，省排版、也避開「按了沒反應要重開」病）。
function shopPanelSig(merchant, selfItem, loggedIn) {
  if (!merchant) return "none";
  const ether = selfItem && Number.isFinite(selfItem.ether) ? selfItem.ether : 0;
  const inv = (selfItem && Array.isArray(selfItem.inventory) ? selfItem.inventory : [])
    .map((s) => `${s.item}:${s.qty}`).join(",");
  const buy = merchant.buy_list.map((e) => `${e.item}:${e.price_per}:${e.trend || ""}`).join(",");
  const sell = merchant.sell_list.map((e) => `${e.item}:${e.price_per}:${e.stock ?? ""}:${e.max_stock ?? ""}`).join(",");
  return `${merchant.id}|${loggedIn ? 1 : 0}|${ether}|${inv}|${buy}|${sell}`;
}

// 重建交易面板內容（賣給商人／向商人買兩區，逐項 emoji+名+價+數量+按鈕）。
// 訪客：按鈕鎖定＋登入提示。送出後不假裝成功——下一份快照的乙太/背包/庫存即真實回饋。
function renderShopPanel(merchant, selfItem, loggedIn) {
  const head = document.getElementById("shopHead");
  const body = document.getElementById("shopBody");
  if (!body) return;
  const ether = selfItem && Number.isFinite(selfItem.ether) ? selfItem.ether : 0;
  const invMap = new Map((selfItem && Array.isArray(selfItem.inventory) ? selfItem.inventory : []).map((s) => [s.item, s.qty]));
  if (head) head.textContent = `🛒 ${merchant.name}　餘額 ${ether}✨`;

  let html = "";
  // —— 賣給商人（換乙太）——
  html += `<div class="shop-sec">📤 賣給商人（換乙太）</div>`;
  if (merchant.buy_list.length === 0) html += `<div class="shop-empty">此商人暫不收購</div>`;
  for (const e of merchant.buy_list) {
    const have = invMap.get(e.item) || 0;
    const can = loggedIn && have > 0;
    const trend = e.trend === "down" ? ` <span class="shop-warn">↘供給過剩</span>` : "";
    html += `<div class="shop-row">
      <span class="shop-name">${itemLabel(e.item)} <span class="shop-have">持有 ${have}</span>${trend}</span>
      <span class="shop-price up">+${e.price_per}✨</span>
      <input class="shop-qty" id="shopSellQty_${e.item}" type="number" min="1" max="${Math.max(1, have)}" value="1" ${can ? "" : "disabled"}>
      <button class="shop-act" id="shopSellBtn_${e.item}" ${can ? "" : "disabled"}>賣出</button>
    </div>`;
  }
  // —— 向商人買（花乙太）——
  html += `<div class="shop-sec">📥 向商人購買（花乙太）</div>`;
  if (merchant.sell_list.length === 0) html += `<div class="shop-empty">此商人暫無販售</div>`;
  const buyback = new Map(merchant.buy_list.map((e) => [e.item, e.price_per]));
  for (const e of merchant.sell_list) {
    const out = e.stock != null && e.stock === 0;
    const can = loggedIn && ether >= e.price_per && !out;
    let stock = "";
    if (e.stock != null && e.max_stock != null) {
      stock = out ? ` <span class="shop-warn">🚫售罄</span>` : ` <span class="shop-stock">庫存 ${e.stock}/${e.max_stock}</span>`;
    }
    const back = buyback.get(e.item);
    const spread = back != null ? ` <span class="shop-dim">（賣回 ${back}✨）</span>` : "";
    html += `<div class="shop-row">
      <span class="shop-name">${itemLabel(e.item)}${stock}${spread}</span>
      <span class="shop-price down">-${e.price_per}✨</span>
      <input class="shop-qty" id="shopBuyQty_${e.item}" type="number" min="1" max="${Math.max(1, e.stock || 99)}" value="1" ${can ? "" : "disabled"}>
      <button class="shop-act" id="shopBuyBtn_${e.item}" ${can ? "" : "disabled"}>${out ? "缺貨" : "購買"}</button>
    </div>`;
  }
  if (!loggedIn) html += `<div class="shop-login">在 but-fun.com 登入後才能買賣（訪客可逛目錄）</div>`;
  body.innerHTML = html;

  // 綁定買賣按鈕（送既有權威意圖；伺服器裁決後下一份快照反映乙太/背包/庫存）。
  for (const e of merchant.buy_list) {
    const btn = document.getElementById(`shopSellBtn_${e.item}`);
    if (btn && btn.addEventListener) btn.addEventListener("click", () => doTrade("shop_sell", e.item, `shopSellQty_${e.item}`));
  }
  for (const e of merchant.sell_list) {
    const btn = document.getElementById(`shopBuyBtn_${e.item}`);
    if (btn && btn.addEventListener) btn.addEventListener("click", () => doTrade("shop_buy", e.item, `shopBuyQty_${e.item}`));
  }
}

// 送一筆買/賣意圖：讀數量框、夾正、送既有協議。未登入給登入提示（誠實鎖定）。
function doTrade(type, item, qtyId) {
  if (!isLoggedIn) { flashToast("🛒 在 but-fun.com 登入後才能買賣"); return; }
  const el = document.getElementById(qtyId);
  let qty = parseInt((el && el.value) || "1", 10);
  if (!Number.isFinite(qty) || qty < 1) qty = 1;
  if (safeSend({ type, item, qty })) {
    flashToast(`🛒 送出${type === "shop_buy" ? "買單" : "賣單"}（${ITEM_NAME[item] || item} ×${qty}）`);
  }
}

// 刷新面板（依簽章避免每幀重建；商人走遠/不見就自動收起，伺服器本就會擋遠距交易）。
function refreshShopPanel() {
  if (!shopOpen) return;
  const target = shopTargetAt(latestSelfWorld, latestMerchants);
  if (!target) { closeShop(); return; }
  const sig = shopPanelSig(target, latestSelfItem, isLoggedIn);
  if (sig === lastShopSig) return;
  lastShopSig = sig;
  renderShopPanel(target, latestSelfItem, isLoggedIn);
}

function openShop() {
  const target = shopTargetAt(latestSelfWorld, latestMerchants);
  if (!target) { flashToast("🛒 走近新手村的商人，就能買賣"); return; }
  const panel = document.getElementById("shopPanel");
  if (panel && panel.classList) panel.classList.remove("hidden");
  shopOpen = true;
  lastShopSig = null;  // 強制重建一次
  refreshShopPanel();
}
function closeShop() {
  const panel = document.getElementById("shopPanel");
  if (panel && panel.classList) panel.classList.add("hidden");
  shopOpen = false;
}
function toggleShop() { shopOpen ? closeShop() : openShop(); }

// 每幀刷新交易鈕外觀（走近商人才亮）。只在 label/鎖定態變動時改 DOM（守 render-loop-resilience）。
let shopBtnLastLabel = null, shopBtnLastLocked = null;
function updateShopBtn() {
  const btn = document.getElementById("shopBtn");
  if (!btn) return;
  const target = shopTargetAt(latestSelfWorld, latestMerchants);
  setBtnShown(btn, !!target); // 情境感知：走近商人才顯示
  if (!target) return;
  const st = shopButtonState(target);
  if (st.label !== shopBtnLastLabel) { btn.textContent = st.label; shopBtnLastLabel = st.label; }
  if (st.locked !== shopBtnLastLocked) { btn.classList.toggle("locked", st.locked); shopBtnLastLocked = st.locked; }
  btn.title = st.hint;
}

// 接線：點 🛒 鈕／按 B 鍵開關交易面板；面板上的 ✕／點面板外／Esc 收起（鏡像表情輪）。
(function wireShop() {
  const btn = document.getElementById("shopBtn");
  if (btn && btn.addEventListener) btn.addEventListener("click", (e) => { if (e && e.preventDefault) e.preventDefault(); toggleShop(); });
  const closeBtn = document.getElementById("shopClose");
  if (closeBtn && closeBtn.addEventListener) closeBtn.addEventListener("click", (e) => { if (e && e.preventDefault) e.preventDefault(); closeShop(); });
  const panel = document.getElementById("shopPanel");
  window.addEventListener("pointerdown", (e) => {
    if (!shopOpen || !panel) return;
    if (e.target === btn || (panel.contains && panel.contains(e.target))) return;
    closeShop();
  });
  window.addEventListener("keydown", (e) => { if (e.code === "Escape") closeShop(); });
  globalThis.__bf3dToggleShop = toggleShop; // 給 B 鍵共用
})();

// 接線：點 🌱／💧／🌾 鈕（桌機 + 手機）；T／F／H 鍵在上方 keydown 處理。
(function wireActButtons() {
  const bind = (id, fn) => {
    const btn = document.getElementById(id);
    if (btn && btn.addEventListener) {
      btn.addEventListener("click", (e) => { if (e && e.preventDefault) e.preventDefault(); fn(); });
    }
  };
  bind("plantBtn", tryPlantTree);
  bind("waterBtn", () => tryTend("water"));
  bind("harvestBtn", () => tryTend("harvest"));
  bind("campfireBtn", tryLightCampfire);
  bind("gatherBtn", tryGather);
  bind("attackBtn", tryAttack); // 揮劍迎敵（ROADMAP 632）；R 鍵在上方 keydown 處理
  bind("comfortBtn", tryComfort); // 園丁關心有心事的居民（ROADMAP 634）；V 鍵在上方 keydown 處理
  bind("helpResidentBtn", tryHelp); // 幫忙正在求助的居民（ROADMAP 125）；X 鍵在上方 keydown 處理
  bind("talkBtn", tryTalk); // 跟居民／城鎮大人物搭話（ROADMAP 636）；Z 鍵在上方 keydown 處理
  bind("giftBtn", tryGift); // 送禮給故鄉居民（ROADMAP 639）；Q 鍵在上方 keydown 處理
  updateActBtns();
})();

// ============================================================
// 表情輪（ROADMAP 621）：點 😊 鈕展開 8 顆表情，選一個就送既有權威 `emote` 意圖；
// 伺服器廣播回來後 updatePlayerEmotes 會在自己（與他人）頭頂浮起對應 emoji。
// 表情不需擁有農地、人人可比，故不做登入鎖定（與 2D 一致；伺服器以連線玩家身分為準）。
// ============================================================
let lastEmoteAt = -1e9;          // 本地冷卻時戳（後端另有每秒 3 次限流，這只是手感防呆）
const EMOTE_COOLDOWN_MS = 400;
(function wireEmoteWheel() {
  const btn = document.getElementById("emoteBtn");
  const popup = document.getElementById("emotePopup");
  if (!btn || !popup) return; // 舊頁／測試 DOM 無此 widget → 靜默跳過
  const hide = () => { popup.classList.add("hidden"); btn.setAttribute("aria-expanded", "false"); };
  const show = () => {
    // 表情鈕在情境動作鈕欄裡會隨其他鈕顯隱而上下移動，故每次展開都依鈕的實際位置把彈出層
    // 對齊到它正上方（不再寫死 bottom，避免與動態欄位錯位）。getBoundingClientRect 不可用就退回原樣式。
    try {
      const r = btn.getBoundingClientRect();
      popup.style.right = Math.max(8, window.innerWidth - r.right) + "px";
      popup.style.bottom = Math.max(8, window.innerHeight - r.top + 8) + "px";
    } catch (e) { /* 無 layout（測試 DOM）→ 用 CSS 預設位置 */ }
    popup.classList.remove("hidden"); btn.setAttribute("aria-expanded", "true");
  };
  const toggle = () => { popup.classList.contains("hidden") ? show() : hide(); };
  // 動態建表情鈕一次（次序與白名單一致）
  for (const [kind, glyph, label] of EMOTE_CHOICES) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "emote-choice";
    b.title = label;
    b.setAttribute("aria-label", label);
    b.textContent = glyph;
    b.addEventListener("click", (e) => {
      if (e && e.preventDefault) e.preventDefault();
      sendEmote(kind);
      hide();
    });
    popup.appendChild(b);
  }
  btn.addEventListener("click", (e) => { if (e && e.preventDefault) e.preventDefault(); toggle(); });
  // 點別處／按 Esc 收起表情輪（不擋遊戲操作）
  window.addEventListener("pointerdown", (e) => {
    if (popup.classList.contains("hidden")) return;
    if (e.target === btn || popup.contains(e.target)) return;
    hide();
  });
  window.addEventListener("keydown", (e) => { if (e.code === "Escape") hide(); });
  globalThis.__bf3dToggleEmoteWheel = toggle; // 給 E 鍵共用
})();

// 送一個表情意圖（白名單外靜默忽略；本地冷卻防手滑連點）。
function sendEmote(kind) {
  const wire = emoteWireMsg(kind);
  if (!wire) return;
  const now = (typeof performance !== "undefined" && performance.now) ? performance.now() : 0;
  if (now - lastEmoteAt < EMOTE_COOLDOWN_MS) return;
  if (safeSend(wire)) lastEmoteAt = now;
}

// ============================================================
// 登入入口（手機優先 HUD 重做）：3D 第一次有「登入」入口——過去一律訪客進場、收成等登入限定
// 功能永遠用不了。沿用 2D（web/game.js）同一套同源 cookie session：點「登入」走既有 Google
// OAuth（/auth/google/start），登入後同源 cookie 自動帶、進場以帳號身分；右上角…置左上角顯示
// 帳號名＋登出。訪客仍可逛（維持「訪客即可動」），但有明確入口拿回完整功能。
// ============================================================
// 依登入態重畫左上角帳號 chip：訪客→「🔑 登入」；已登入→「帳號名 · 登出」。
function renderAccountChip() {
  const bar = document.getElementById("topbar");
  // 需要完整 DOM API 才動手；舊頁／測試假 DOM 缺方法時靜默跳過（不 throw、不污染 smoke）。
  if (!bar || typeof document === "undefined" || typeof document.createElement !== "function" ||
      typeof bar.insertBefore !== "function") return;
  // 移除上一版帳號元素（保留 helpBtn）。
  if (typeof bar.querySelectorAll === "function") {
    bar.querySelectorAll(".acc-el").forEach((el) => { if (el.remove) el.remove(); });
  }
  if (isLoggedIn) {
    const name = document.createElement("span");
    name.className = "tb-name acc-el";
    name.textContent = myName3d;
    name.title = "已登入：" + myName3d;
    const out = document.createElement("button");
    out.type = "button";
    out.className = "tb-btn acc-el";
    out.textContent = "登出";
    out.addEventListener("click", (e) => {
      if (e && e.preventDefault) e.preventDefault();
      // 清 session cookie 後重整：回到訪客態（與 2D 登出一致）。
      fetch("/auth/logout", { method: "POST", credentials: "same-origin" })
        .catch(() => {})
        .then(() => { try { location.reload(); } catch (e) { /* 測試 DOM 無 location */ } });
    });
    // 插在 helpBtn 之前，讓「帳號 · 登出 · ?」由左到右排列。
    bar.insertBefore(name, bar.firstChild);
    bar.insertBefore(out, document.getElementById("helpBtn"));
  } else {
    const login = document.createElement("button");
    login.type = "button";
    login.id = "loginBtn";
    login.className = "tb-btn acc-el";
    login.textContent = "🔑 登入";
    login.title = "用 Google 登入，解鎖收成等登入限定功能";
    login.addEventListener("click", (e) => {
      if (e && e.preventDefault) e.preventDefault();
      // 走既有 Google OAuth（與 2D 同一條）；登入完成後伺服器導回首頁。
      try { location.href = "/auth/google/start"; } catch (e) { /* 測試 DOM 無 location */ }
    });
    bar.insertBefore(login, bar.firstChild);
  }
}

// 操作提示（手機優先：不再常駐長文字佔畫面，改成首次進場浮一次＋「?」鈕隨時叫回）。
function showControlsHint() {
  flashToast(isTouch
    ? "搖桿移動 · 拖曳畫面轉鏡頭 · 右下跳／跑 · 走近人或資源就會冒出可做的事"
    : "WASD 移動 · Shift 跑 · 拖曳轉鏡頭 · 空白鍵跳 · 走近人或資源就會冒出可做的事");
}
(function wireTopbar() {
  renderAccountChip(); // 先以訪客態畫一次（/auth/me 命中再重畫成已登入）
  const help = document.getElementById("helpBtn");
  if (help && help.addEventListener) {
    help.addEventListener("click", (e) => { if (e && e.preventDefault) e.preventDefault(); showControlsHint(); });
  }
  // 首次進場自動浮一次操作提示（記在 localStorage，之後不再自動打擾；查不到 storage 就略過）。
  try {
    if (!localStorage.getItem("bf3d_hint_seen")) {
      setTimeout(showControlsHint, 1200);
      localStorage.setItem("bf3d_hint_seen", "1");
    }
  } catch (e) { /* 隱私模式／無 storage → 不自動提示，仍可按 ? */ }
})();

// 開頁查 /auth/me：已登入就點亮種樹鈕、進場用帳號名、左上角換成「帳號名 · 登出」（authed 身分仍由 cookie 決定）。
// OAuth 未設定／未登入回非 2xx → 維持訪客態（照常觀賞）。fetch 不可用（如 smoke 沙箱）就跳過。
if (typeof fetch === "function") {
  fetch("/auth/me", { credentials: "same-origin" })
    .then((r) => (r && r.ok ? r.json() : null))
    .then((me) => {
      if (me && me.id) {
        isLoggedIn = true;
        if (me.name) myName3d = me.name;
        if (me.species) mySpecies3d = me.species;
        updateActBtns();
        renderAccountChip();
      }
    })
    .catch(() => { /* 查不到就當訪客，不影響觀賞 */ });
}

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

  // 跑步切換鈕：tap 切換走/跑（toggle）。on 時高亮；狀態反映到 runToggle → inputKeys.run。
  // preventDefault 抑制 ghost click，避免一次點擊切兩次。
  const runEl = document.getElementById("run");
  if (runEl) {
    runEl.addEventListener("touchstart", (e) => {
      e.preventDefault();
      runToggle = !runToggle;
      runEl.classList.toggle("on", runToggle);
    }, { passive: false });
  }

  // 右半邊拖曳轉鏡頭（避開搖桿/跳鈕/跑鈕；用 identifier 同時並存）
  let camId = null, camLX = 0, camLY = 0;
  function isOnUI(t) {
    const el = document.elementFromPoint(t.clientX, t.clientY);
    return el && (el.closest("#joy") || el.closest("#jump") || el.closest("#run"));
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
  // 跑步＝獨立旗標（桌機 Shift hold ／ 手機切換鈕），不再用搖桿推深控速——
  // 玩家回報類比推比例在手機難精準。本地預測（wasm run_mult）與送伺服器用的是同一個旗標。
  inputKeys.run = runHeld || runToggle;

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
    // 進場：訪客用預設名「3D玩家」，登入者用帳號名（authed 身分一律由 cookie 決定，與名字無關）。
    ws.send(JSON.stringify({ type: "join", name: myName3d, species: mySpecies3d }));
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
// 單節四肢版（perf/3d-fps）：膝／肘合進整條腿臂，少了彎曲就把擺幅略加大，
// 走路前後擺更明顯來補回「有在邁步」的辨識度。
const GAIT = {
  LEG: 1.0,     // 整條腿前後擺幅（弧度，較舊大腿 0.85 略大補回膝彎觀感）
  ARM: 0.8,     // 整條臂擺幅（與同側腿反相）
  LEAN: 0.08,   // 走路時整體略前傾（弧度）
  FREQ: 0.85,   // 相位推進係數（× 移動速度 → 走越快步頻越快）
  IDLE_FREQ: 0.6, // 站定時殘留的最小推進，讓關節平滑收回站姿
};

// 遠處火柴人動畫效能保險：每隻火柴人每幀 8 處關節擺動，視野內幾十隻會吃 CPU（掉 FPS＝卡頓）。
// 共用幾何已省記憶體，這裡再省 CPU：距鏡頭很遠的火柴人累積 dt、降頻更新關節（遠到看不清步態，
// 降頻無感）；近處（含自己）維持每幀更新＝順。可調開關／距離／間隔。
const STICK_FAR_THROTTLE = true;   // 遠處火柴人動畫節流開關
const STICK_FAR_DIST = 90;         // 超過此距離（場景單位，≈1800 世界 px）視為「遠處」→ 降頻
const STICK_FAR_DIST2 = STICK_FAR_DIST * STICK_FAR_DIST; // 比距離平方免開根號
const STICK_FAR_INTERVAL = 0.12;   // 遠處關節更新間隔（秒，約 8Hz；累積 dt 補進，步頻不失真）

// 程序化走路：用 sine 波擺動火柴人關節。speed＝場景單位/秒；停下時平滑回站姿。
// 盒子實體（無 stick）直接 return，不受影響。
function animateStickman(g, speed, dt) {
  const stick = g.userData.stick;
  if (!stick) return;
  // 遠處節流：距鏡頭很遠時累積 dt、未到間隔就跳過本幀關節更新（省 CPU）；到間隔再一次補進累積時間
  // （dt 變大 → 相位推進量等於跳過的那幾幀總和，步頻不失真）。近處不節流＝每幀順。
  if (STICK_FAR_THROTTLE && camera) {
    const dxc = g.position.x - camera.position.x;
    const dyc = g.position.y - camera.position.y;
    const dzc = g.position.z - camera.position.z;
    if (dxc * dxc + dyc * dyc + dzc * dzc > STICK_FAR_DIST2) {
      stick.userData.animAccum = (stick.userData.animAccum || 0) + dt;
      if (stick.userData.animAccum < STICK_FAR_INTERVAL) return; // 還沒到更新間隔→跳過
      dt = stick.userData.animAccum;
      stick.userData.animAccum = 0;
    } else {
      stick.userData.animAccum = 0;
    }
  }
  const j = stick.userData.joints;
  // 走路權重平滑進出（避免一停就僵、一動就跳）
  const moving = speed > 0.6 ? 1 : 0;
  stick.userData.walkW += (moving - stick.userData.walkW) * Math.min(1, dt * 6);
  const w = stick.userData.walkW;
  // 相位隨移動距離推進；殘留一點最小步頻讓站定也能把擺幅平滑歸零
  stick.userData.phase += (speed * GAIT.FREQ + GAIT.IDLE_FREQ) * dt;
  const ph = stick.userData.phase;
  const s = Math.sin(ph);
  // 腿：整條腿左右反相前後擺（單節，無膝彎）
  j.legL.rotation.x = s * GAIT.LEG * w;
  j.legR.rotation.x = -s * GAIT.LEG * w;
  // 手臂：整條臂與同側腿反相擺（單節，無肘彎）
  j.armL.rotation.x = -s * GAIT.ARM * w;
  j.armR.rotation.x = s * GAIT.ARM * w;
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
    // 手機弱 GPU：距鏡頭過遠（已沒入霧）的實體整個不畫——省 draw call，並跳過走動動畫省 CPU。
    // 仍跑 fade/移除邏輯（離場淡出要正確收掉）。桌機 ENTITY_CULL_DIST2=Infinity → 此分支不啟用、行為不變。
    if (LOWFX) {
      const dxc = g.position.x - camera.position.x;
      const dzc = g.position.z - camera.position.z;
      if (dxc * dxc + dzc * dzc > ENTITY_CULL_DIST2) {
        if (g.visible) g.visible = false; // 跨界才寫
        if (updateFade(g, dt)) { scene_.remove(g); map.delete(key); }
        continue;
      }
      if (!g.visible) g.visible = true; // 回到範圍內：恢復可見
    }
    if (animate) faceAndBob(g, g.position.x - ox, g.position.z - oz, dt, t);
    if (updateFade(g, dt)) { scene_.remove(g); map.delete(key); }
  }
}

// 自身客戶端預測 + 平滑對帳：輸入時本地立即往同方向推（零延遲），權威回來時柔和拉回。
// 不改協議、不改伺服器——這是伺服器權威下的純視覺預測。
// 優先用 world-core wasm 的 step_player（碰撞感知、與伺服器 Player::step 同源）；
// wasm 沒載到／還沒有權威座標時退回 #764 的場景空間簡單推（後備，不白屏）。
function updateSelfPrediction(g, dt) {
  if (wasmPredictReady() && latestSelfWorld) {
    // 第一次（或剛切換到 wasm 預測）：以最新權威世界座標錨定 predBase（latestSelfWorld 是世界 px）。
    if (!predWorld.has) {
      predWorld.x = latestSelfWorld.x; predWorld.y = latestSelfWorld.y;
      predWorld.has = true;
    }
    // 方向 mask：與 game.js／伺服器 Player::step 同形（上1 下2 左4 右8）。
    // inputKeys 已由鍵盤＋搖桿（只取方向、不取推深）映射好；run 旗標另外決定跑步。
    const mask = (inputKeys.up ? 1 : 0) | (inputKeys.down ? 2 : 0) | (inputKeys.left ? 4 : 0) | (inputKeys.right ? 8 : 0);
    const moving = mask !== 0;
    // 每幀：從 predBase 起，用 wasm step_player 往前外插「當前這一幀的輸入」（即時跟手；含碰撞，
    // 跑步倍率讀 wasm＝與後端 RUN_MULT 同源 → 跑步不落後被拉回）。
    if (mask) {
      const moveDt = dt * (inputKeys.run ? wasmRunMult() : 1);
      wasmTerrain.step_player(predWorld.x, predWorld.y, mask, moveDt);
      predWorld.x = wasmTerrain.step_out_x();
      predWorld.y = wasmTerrain.step_out_y();
    }
    // 對帳（reconcile）——「每幀平滑連續收斂到『權威外插』的平滑目標」（QA #803 實測挑出）：
    //  · 超大誤差（>JUMP_PX，傳送／回城／復活）：直接 snap 到原始權威（保留既有行為）。
    //  · 移動中：收斂目標 ≠ 原始權威（那是叢發快照造成的階梯狀目標＝殘卡來源），而是用權威估計
    //    速度把目標往前外插成「平滑連續移動的點」：target = 最近權威 + 速度 × (此幀距該權威到達的時間)。
    //    predBase 每幀朝這個平滑 target 收斂 CONVERGE_K → 目標本身平滑 → 殺掉殘留單幀 lurch。
    //  · 靜止：不外插（速度已歸零、且避免衝過停點），朝原始權威用較大的 CONVERGE_STOP_K 快對齊。
    //  收斂量 1-exp(-dt*K) ＝ 跟幀率無關。
    const err = Math.hypot(latestSelfWorld.x - predWorld.x, latestSelfWorld.y - predWorld.y);
    selfPredErrPx = err; // debug 讀數：自身預測與最近「原始權威」的距離（世界 px）＝領先量 → 應穩在 ~7-20
    if (err > RECONCILE_JUMP_PX) {
      predWorld.x = latestSelfWorld.x; predWorld.y = latestSelfWorld.y;
    } else {
      // 對齊目標：移動中外插（平滑），靜止用原始權威。
      let tx = latestSelfWorld.x, ty = latestSelfWorld.y;
      if (moving) {
        const v = estimateSelfAuthVel(); // 權威估計速度（世界 px/s），時間窗平滑過叢發雜訊
        const nowMs = (typeof performance !== "undefined" && performance.now) ? performance.now() : Date.now();
        const age = Math.min(Math.max(nowMs - lastSelfAuthT, 0), AUTH_EXTRAP_MAX_MS) / 1000; // 此幀距最近權威到達多久（s，夾上限）
        tx += v.vx * age; ty += v.vy * age;
      }
      const a = 1 - Math.exp(-dt * (moving ? CONVERGE_K : CONVERGE_STOP_K)); // 1-exp → 跟幀率無關
      predWorld.x += (tx - predWorld.x) * a; predWorld.y += (ty - predWorld.y) * a;
    }
    // 世界 px → 場景單位（沿用既有 sx/sz 比例常數），套到自己的 mesh。
    g.position.x = sx(predWorld.x);
    g.position.z = sz(predWorld.y);
    return;
  }

  // ── 後備（wasm 沒載到／還沒權威）：#764 的場景空間簡單推＋對帳 ──
  predWorld.has = false; // 之後 wasm／權威到齊時，重新從權威錨定預測
  // 預測位移：用「正要送出的世界四向意圖」立刻往前推（與伺服器一致地對角正規化）。
  let dx = (inputKeys.right ? 1 : 0) - (inputKeys.left ? 1 : 0);
  let dz = (inputKeys.down ? 1 : 0) - (inputKeys.up ? 1 : 0);
  const len = Math.hypot(dx, dz);
  const moving = len > 0;
  if (len > 0) {
    dx /= len; dz /= len;
    // selfMeasuredSpeed 已含跑步／加速／載具（由快照實測），不必再乘 run 倍率
    g.position.x += dx * selfMeasuredSpeed * dt;
    g.position.z += dz * selfMeasuredSpeed * dt;
  }
  // 對帳：同 wasm 路的「每幀平滑連續收斂」（這裡基準＝g.position 場景單位，閾值由世界 px × WORLD_SCALE 換算）。
  if (selfHasAuth) {
    const ex = selfAuthX - g.position.x;
    const ez = selfAuthZ - g.position.z;
    const err = Math.hypot(ex, ez);
    selfPredErrPx = err / WORLD_SCALE; // 場景單位 → 世界 px，供 debug 讀數與 wasm 路一致
    if (err > RECONCILE_JUMP_PX * WORLD_SCALE) {
      g.position.x += ex; g.position.z += ez; // 超大誤差直接 snap
    } else {
      const a = 1 - Math.exp(-dt * (moving ? CONVERGE_K : CONVERGE_STOP_K)); // 每幀平滑收斂（跟幀率無關，不再離散猛拉）
      g.position.x += ex * a; g.position.z += ez * a;
    }
  }
}

// ============================================================
// 偵錯讀數 HUD：FPS／自身預測誤差／線上人數（預設關，?debug=1 或點左上角切換）
// ============================================================
// 純前端唯讀小面板，下次一眼判斷：FPS 低＝效能問題；預測誤差大＝netcode 問題。
// 元素動態建立（不動 index.html 結構，只在 debug 開時掛上）。
let _dbgEl = null;
let _fpsEMA = 60;        // 平滑後 FPS（指數移動平均，避免逐幀跳動）
let _dbgLastText = "";
function ensureDebugEl() {
  if (_dbgEl || typeof document === "undefined" || !document.body) return _dbgEl;
  const el = document.createElement("div");
  el.id = "dbgHud";
  el.style.cssText =
    "position:fixed;right:12px;bottom:148px;z-index:60;pointer-events:none;" +
    "font:12px/1.45 ui-monospace,Menlo,Consolas,monospace;color:#9effa6;" +
    "background:rgba(13,17,23,0.74);padding:6px 10px;border-radius:8px;white-space:pre;" +
    "border:1px solid rgba(120,255,140,0.28);";
  document.body.appendChild(el);
  _dbgEl = el;
  return el;
}
// 左上角熱區切換 debug（避開右下跳鈕／左下搖桿等控制；?debug=1 則預設開）。
// 左上角現在擺了帳號 chip（登入／登出／?），故點到任何按鈕／連結時不觸發 debug 切換（否則點登入也會開 debug）。
if (typeof window !== "undefined") {
  window.addEventListener("pointerdown", (e) => {
    if (e.target && e.target.closest && e.target.closest("button, a, #topbar")) return;
    if (e.clientX < 56 && e.clientY < 56) {
      DEBUG_HUD = !DEBUG_HUD;
      if (!DEBUG_HUD && _dbgEl) _dbgEl.style.display = "none";
    }
  }, true);
}
// 每幀更新偵錯讀數：FPS 一律平滑累積（即使面板關著，開的瞬間就是準的）；開著才寫 DOM。
function updateDebugHud(dt) {
  if (dt > 1e-4) _fpsEMA += (1 / dt - _fpsEMA) * 0.1;
  if (!DEBUG_HUD) return;
  const el = ensureDebugEl();
  if (!el) return;
  if (el.style.display === "none") el.style.display = "";
  // window.__BUILD__ 由後端 serve_3d_index 注入（main.js 內容雜湊前 12 hex）；
  // 顯示在 debug HUD 讓維護者一眼確認瀏覽器拿到的是最新版前端。
  const build = (typeof window !== "undefined" && window.__BUILD__) ? `\nbuild ${window.__BUILD__}` : "";
  const txt = `FPS ${_fpsEMA.toFixed(0)}\n預測誤差 ${selfPredErrPx.toFixed(0)}px\n線上 ${players.size} 人${build}`;
  if (txt !== _dbgLastText) { el.textContent = txt; _dbgLastText = txt; }
}

// 每幀更新天時盤 HUD（ROADMAP 620）：太陽/月亮繞盤、時段大字、下一時段倒數、夜間危機暈輪。
// 純讀 latestDayNight + 本地錨點平滑推進；無 widget／壞值一律安全靜默（守 render-loop-resilience）。
function updateDayClock() {
  if (!dcOrbitEl) return; // 無此 widget（舊頁／測試 DOM）→ 靜默跳過
  const elapsed = (performance.now() - dayNightAnchorMs) / 1000;
  const r = dayClockReadout(latestDayNight, elapsed);
  // 太陽/月亮繞盤：轉 orbit 容器把圖示帶著繞圈，圖示本體反轉抵銷避免倒置。
  dcOrbitEl.style.transform = "rotate(" + r.sunDeg.toFixed(1) + "deg)";
  if (dcSunEl) {
    const icon = r.isNight ? "🌙" : "☀️";
    if (dcSunEl.textContent !== icon) dcSunEl.textContent = icon;
    dcSunEl.style.transform = "translateX(-50%) rotate(" + (-r.sunDeg).toFixed(1) + "deg)";
  }
  // 夜間危機暈輪：phase===night 時盤緣轉紅暈（呼應世界的危機紅化）。
  if (dcDialEl) dcDialEl.classList.toggle("danger", r.danger);
  // 時段大字（只在變動時改寫 DOM）。
  if (dcPhaseEl && r.phaseLabel !== dcLastPhase) { dcPhaseEl.textContent = r.phaseLabel || ""; dcLastPhase = r.phaseLabel; }
  // 下一時段倒數句：「再 m:ss → 🌙夜晚」——把抽象的「時間流速」具象成可讀的節奏感。
  const cd = fmtCountdown(r.secsLeft);
  const nextLine = (r.nextLabel && cd) ? ("再 " + cd + " → " + r.nextLabel) : (r.nextLabel || "");
  if (dcNextEl && nextLine !== dcLastNext) { dcNextEl.textContent = nextLine; dcLastNext = nextLine; }
}

function safeRender() {
  requestAnimationFrame(safeRender);
  try {
    const dt = Math.min(0.05, clock.getDelta());
    // 每幀把目前操控意圖換算並（在改變時）送出
    updateInput();
    // 日夜流轉：依最新權威 daynight 平滑更新天空／太陽／環境光（ROADMAP 612）
    applyDayNight(dt);
    // 天氣流轉：依最新權威 weather／rainbow 平滑更新粒子場／霧染／遠空彩虹（ROADMAP 613，疊在日夜之後）
    applyWeather(dt);
    // 手機弱 GPU：把霧的「看得見距離」夾在 FOG_FAR 內（晴天時 applyWeather 會把 fog.far 寫回 600，這裡夾回），
    // 讓被距離剔除的遠端實體是「沒入霧中消失」而非「憑空消失」，pop-in 不明顯。桌機 FOG_FAR=600 → 不影響。
    if (LOWFX && scene.fog && scene.fog.far > FOG_FAR) scene.fog.far = FOG_FAR;
    // 天上日月星辰：依最新權威 daynight 平滑更新太陽弧／月相對位／星空淡入斗轉（ROADMAP 628）
    applyCelestial(dt);

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
    // 玩家頭頂表情泡泡：在 players 的 updateFade 之後覆寫表情 sprite 的顯示（ROADMAP 621）
    updatePlayerEmotes(performance.now());
    // 寵物夥伴：正規實體內插（跟著主人跑）＋轉身起伏；之後覆寫頭頂狀態／腳邊羈絆條（ROADMAP 627）
    updateRemoteEntities(pets, scene, renderTime, true, dt, t, undefined, k);
    updatePetStatus(t);
    updateRemoteEntities(npcs, scene, renderTime, true, dt, t, undefined, k);
    // NPC 內心生活呈現：在 npcs 的 updateFade 之後覆寫狀態/關懷/思想 sprite 的顯示（ROADMAP 611）
    updateResidentStatus(t);
    // 居民對話泡泡：在 updateResidentStatus 之後覆寫對話 sprite 的顯示（ROADMAP 622）
    updateNpcSpeech(performance.now());
    // 居民派系關係連線：在 npcs 位置/fade 更新後，於結盟/敵對的兩位居民之間畫弧（ROADMAP 625）
    updateFactionLinks(t);
    // 居民互助送禮：在 npcs 位置/fade 更新後，於送禮者→受禮者之間飄一枚暖金光禮（ROADMAP 369）
    updateTownShare(t);
    updateRemoteEntities(wildlife, scene, renderTime, true, dt, t, undefined, k);
    // 野生動物呈現：在 wildlife 的 updateFade 之後覆寫頭頂狀態 sprite 與幼獸體型（ROADMAP 615）
    updateWildlifeStatus(t);
    updateRemoteEntities(enemies, scene, renderTime, true, dt, t, undefined, k);
    // 敵人呈現：在 enemies 的 updateFade 之後覆寫頭頂血條／狀態 emoji 與兇名體型（ROADMAP 626）
    updateEnemyStatus(t);
    // 節點靜態：不轉身/起伏；位置吸最新目標（內插對靜態無差），仍走 AOI 淡入淡出
    updateRemoteEntities(nodes, scene, renderTime, false, dt, t, undefined, 1);
    // 夜採星晶礦脈：同節點走靜態 AOI 淡入淡出（夜間限定，天亮自動淡出，ROADMAP 629）
    updateRemoteEntities(starCrystals, scene, renderTime, false, dt, t, undefined, 1);
    // 農地：作物隨風輕搖、成熟金果發光脈動、AOI 淡入淡出（ROADMAP 614）
    updateFields(dt, t);
    // 人造地標：篝火火焰跳動／暖圈入夜更亮、塔頂燈入夜亮起、雪人愛心數（ROADMAP 616）
    updateStructures(dt, t);
    // 世界樹群：幼樹／成樹隨風輕擺、AOI 淡入淡出（ROADMAP 617）
    updateGroves(dt, t);
    // 重大世界事件：宇宙裂縫光柱脈動旋轉、獸潮警示光束依階段變色脈動、事件消失即隱藏（ROADMAP 631）
    updateWorldEvents(dt, t);
    // 天時盤 HUD：太陽/月亮繞盤、時段、下一時段倒數、夜間危機暈輪（ROADMAP 620）
    updateDayClock();
    // 探索羅盤雷達 HUD：玩家為心標出家／居民／其他玩家／敵人／世界大事方位（ROADMAP 633）
    drawRadar();
    // 澆水／收成鈕：已登入＋站在自家田旁才顯示（情境感知，手機優先 HUD）
    updateFarmBtns();
    // 採集鈕：依腳邊有沒有可採目標（樹／石／乙太礦／夜間星晶）即時顯示／隱藏（ROADMAP 629）
    updateGatherBtn();
    // 交易鈕：走近新手村商人才亮（ROADMAP 630）
    updateShopBtn();
    // 攻擊鈕＋戰鬥特效：走近敵人才亮攻擊鈕，揮砍弧光掃過淡出、命中傷害飄字上飄淡出（ROADMAP 632）
    updateAttackBtn();
    // 關心鈕：走近一位正有心事（needs_care）的故鄉居民才亮（ROADMAP 634：園丁撫慰）
    updateComfortBtn();
    // 幫忙鈕：走近一位正開口求助（active_help_requests）的居民才亮（ROADMAP 125：居民互助）
    updateHelpBtn();
    // 搭話鈕：走近一位居民／城鎮大人物才亮（ROADMAP 636：主動開口攀談、聽見回話）
    updateTalkBtn();
    // 送禮鈕：走近一位故鄉居民＋背包有東西可送才亮（ROADMAP 639：送禮加深交情）
    updateGiftBtn();
    updateMeleeSwing(performance.now());
    updateDamageFloats(performance.now());
    updateRewardFloats(performance.now()); // 戰利品入袋／連殺標語飄字上飄淡出（ROADMAP 635）

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

    // 偵錯讀數（FPS／自身預測誤差／線上人數）：FPS 一律累積，?debug=1／點左上角才顯示
    updateDebugHud(dt);

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

// 測試掛鉤（scripts/qa/render-smoke-3d.mjs 用；瀏覽器中無副作用、只暴露純邏輯供斷言）。
if (typeof globalThis !== "undefined") {
  globalThis.__bf3dTest = { residentStatusEmoji, NPC_ACTIVITY_ICON, thoughtTexture, dayNightVisual, dayNightPhaseLabel, celestialSky, weatherVisual, weatherHudLabel, cropCellVisual, cropBarFill, fieldDigest, farmHudLabel, wildlifeVisual, wildlifeStatusEmoji, wildlifeHudLabel, enemyVisual, enemyStatusEmoji, enemyHpFill, enemyHudLabel, campfireVisual, watchtowerVisual, snowmanVisual, structuresHudLabel, groveVisual, groveHudLabel, plantTreeWireMsg, plantButtonState, waterAllWireMsg, harvestAllWireMsg, tendButtonState, campfireWireMsg, campfireButtonState, dayClockReadout, fmtCountdown, emoteWireMsg, emoteBubbleVisual, EMOTE_CHOICES, npcSpeechVisual, speechTexture, factionLinkVisual, factionArcPoints, factionHudLabel, FACTION_BOND_STYLE, townShareGiftSpec, townShareHudLabel, petVisual, petStatusEmoji, petBondHearts, petHudLabel, gatherWireMsg, gatherStarCrystalWireMsg, gatherTargetAt, gatherButtonState, attackWireMsg, attackTargetAt, attackButtonState, damageFloatSpec, spawnMeleeSwing, comfortWireMsg, comfortTargetAt, comfortButtonState, helpWireMsg, helpTargetAt, helpButtonState, talkKindOf, talkTargetAt, talkWireMsg, talkButtonState, giftPickItem, giftTargetAt, giftWireMsg, giftButtonState, lootFloatSpec, killStreakFloatSpec, shopMerchantsFrom, shopTargetAt, shopButtonState, shopPanelSig, itemLabel, riftVisual, riftHudLabel, hordeVisual, hordeHudLabel, radarBlips, radarHeading, nearOwnField };
}

// 啟動
setStatus("連線中…");
connect();
safeRender();
