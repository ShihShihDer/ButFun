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
// 工作台 v1（ROADMAP 665）——玩家合成+放置，互動開 3×3 合成格
const WORKBENCH = 15;
// 熔爐 v1（ROADMAP 666）——工作台合成+放置，互動開冶煉面板
const FURNACE = 16;
// 拋光石 v1（ROADMAP 666）——熔爐冶煉所得，精緻灰石建材
const SMOOTH_STONE = 17;
// 麵包 v1（ROADMAP 668）——純物品，收割小麥→合麵包；18=小麥顆粒,19=麵包
const WHEAT = 18, BREAD = 19;
// 深層礦石 v1（ROADMAP 682）——地底石層採掘所得
const COAL_ORE = 20; // 煤礦——最淺的礦石，y ≤ 3 的石層有機率生成
const IRON_ORE = 21; // 鐵礦——更深更稀少，y ≤ 1 的石層有機率生成
// 鐵錠 v1（ROADMAP 683）——熔爐冶煉所得（1 鐵礦 + 1 煤礦 → 2 鐵錠）
const IRON_INGOT = 22;
// 鐵磚 v1（ROADMAP 684）——工作台合成（4 鐵錠 → 1 鐵磚）
const IRON_BLOCK = 23;
// 流動水（水流動模擬）——來源水 WATER=7 是 level 0/無限；24..=30 是流動 level 1..=7。
// 非實心（可穿越、水面渲染同來源水），玩家不可放置（伺服器模擬維護的狀態方塊）。
const WATER_FLOW_BASE = 24, WATER_FLOW_MAX_LVL = 7;
// 任一方塊 id 是否為「水」（來源或流動）——渲染與碰撞都把兩者當水看待。
function isWaterId(b) { return b === WATER || (b >= WATER_FLOW_BASE && b < WATER_FLOW_BASE + WATER_FLOW_MAX_LVL); }
// 火把 v1（ROADMAP 685）——背包合成（1 木頭 + 1 煤礦 → 4 火把）；橘黃光源，礦坑標記用
const TORCH = 31;
// 鎬具 v1（ROADMAP 687）——純物品，不可放置；提升石/礦採集速度
const PICKAXE_WOOD = 32, PICKAXE_STONE = 33, PICKAXE_IRON = 34;
// 梯子 v1（ROADMAP 688）——可放置；玩家進入方格後自動抓握、取消重力可垂直攀爬
const LADDER = 35;
// 斧頭 v1（ROADMAP 689）——純物品，不可放置；持斧砍木頭/葉片/木板大幅加速
const AXE_WOOD = 36, AXE_STONE = 37, AXE_IRON = 38;
// 鏟子 v1（ROADMAP 690）——純物品，不可放置；持鏟挖草地/泥土/沙地/農田大幅加速
const SHOVEL_WOOD = 39, SHOVEL_STONE = 40, SHOVEL_IRON = 41;
// 箱子 v1（ROADMAP 692）——工作台合成（8 木板 → 1 箱子）；放置後右鍵互動開儲物面板
const CHEST = 42;
// 木門 v1（ROADMAP 693）——背包 2×2 合成（4 木板 → 2 門）；右鍵切換開/關，DoorOpen 非實心可穿越
const DOOR_CLOSED = 43, DOOR_OPEN = 44;
// 床 v1——背包 2×2 合成（3 木板 + 3 葉片 → 1 床）；右鍵夜晚睡覺跳過黑夜到隔天黎明
const BED = 45;
// 第二種作物 v1——種田系統第一次有兩種作物可選：胡蘿蔔比小麥快熟（60s/水耕30s vs 90s/45s）。
// 胡蘿蔔幼苗/成熟胡蘿蔔為伺服器狀態方塊；種子/收成為純物品，從草地(GRASS)破壞額外掉落種子。
const CARROT_SEEDED = 46, CARROT_MATURE = 47;
const CARROT_SEEDS = 48, CARROT = 49;
// 第三種作物 v1——馬鈴薯比小麥慢熟但收成量大（120s/水耕60s，收成×2）。
// 馬鈴薯幼苗/成熟馬鈴薯為伺服器狀態方塊；種子/收成為純物品，從泥土(DIRT)破壞額外掉落種子（與胡蘿蔔取自草地區隔）。
const POTATO_SEEDED = 50, POTATO_MATURE = 51;
const POTATO_SEEDS = 52, POTATO = 53;
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
  // 工作台 v1
  [WORKBENCH]:         [0.62, 0.40, 0.18], // 工作台——深琥珀棕，木製工作台感
  // 熔爐 v1
  [FURNACE]:           [0.36, 0.26, 0.20], // 熔爐——暗灰棕，燻黑石材爐
  [SMOOTH_STONE]:      [0.72, 0.72, 0.74], // 拋光石——明亮冷灰，精煉石材感
  // 深層礦石 v1（ROADMAP 682）——石灰底+礦石紋理感
  [COAL_ORE]:          [0.35, 0.33, 0.32], // 煤礦——深灰帶黑，石中夾黑炭
  [IRON_ORE]:          [0.66, 0.44, 0.28], // 鐵礦——帶鏽橙的石，鐵質感
  // 鐵錠 v1（ROADMAP 683）——閃亮銀灰，精煉金屬感
  [IRON_INGOT]:        [0.76, 0.76, 0.82], // 鐵錠——明亮銀灰，冶煉後的光澤金屬
  // 鐵磚 v1（ROADMAP 684）——壓縮精煉，比鐵錠更亮更飽和
  [IRON_BLOCK]:        [0.88, 0.88, 0.94], // 鐵磚——高亮銀白帶藍，光潔金屬塊感
  // 火把 v1（ROADMAP 685）——橘黃火焰感，點亮礦坑隧道
  [TORCH]:             [1.00, 0.61, 0.05], // 火把——暖橘黃，燃燒火焰的光感
  // 梯子 v1（ROADMAP 688）——暖木棕，比木板略深；放置後可垂直攀爬
  [LADDER]:            [0.62, 0.42, 0.20], // 梯子——深暖棕，木製梯架感
  // 鎬具 v1（ROADMAP 687）——工具物品，不渲染為世界方塊（只在 UI 顯示）
  [PICKAXE_WOOD]:  [0.60, 0.44, 0.26], // 木鎬——深棕木柄感
  [PICKAXE_STONE]: [0.58, 0.58, 0.60], // 石鎬——冷灰石質感
  [PICKAXE_IRON]:  [0.80, 0.82, 0.90], // 鐵鎬——明亮銀藍，精煉金屬感
  // 斧頭 v1（ROADMAP 689）——暖木棕/冷石灰/亮金屬，與鎬具色系相近但偏暖
  [AXE_WOOD]:  [0.70, 0.48, 0.22], // 木斧——暖棕，比木鎬淺一階（新磨的木刃）
  [AXE_STONE]: [0.55, 0.52, 0.48], // 石斧——微暖灰，比石鎬偏赭（石刃較粗礦）
  [AXE_IRON]:  [0.82, 0.78, 0.74], // 鐵斧——偏暖銀，比鐵鎬少一分冷藍（寬刃感）
  // 鏟子 v1（ROADMAP 690）——泥土色系，土木感；比斧頭更飽和偏赭（鏟面是鏟土的顏色）
  [SHOVEL_WOOD]:  [0.65, 0.45, 0.18], // 木鏟——赭棕，新刻木鏟頭感
  [SHOVEL_STONE]: [0.50, 0.46, 0.42], // 石鏟——灰赭，石材鏟頭（比石斧偏紅）
  [SHOVEL_IRON]:  [0.78, 0.80, 0.84], // 鐵鏟——冷銀，鐵製鏟面反光
  // 箱子 v1（ROADMAP 692）——暖棕木箱感，比工作台淺、比梯子亮；金屬鉚釘感
  [CHEST]:        [0.72, 0.52, 0.28], // 箱子——中暖棕，木箱+鐵釘視覺
  // 木門 v1（ROADMAP 693）——關閉=深暖棕厚實；開啟=淡杏白，一眼分辨可穿越
  [DOOR_CLOSED]:  [0.58, 0.36, 0.14], // 木門（關）——深暖棕，實心大門感
  [DOOR_OPEN]:    [0.85, 0.72, 0.55], // 木門（開）——淡杏白，半透感，可穿越
  // 床 v1——暖紅被褥感，一眼認出是家具而非建材
  [BED]:          [0.72, 0.30, 0.28], // 床——暖磚紅，被褥的溫暖感
  // 第二種作物 v1（胡蘿蔔）——橘色調對比小麥的金黃，一眼分辨兩種作物
  [CARROT_SEEDED]: [0.30, 0.44, 0.22], // 胡蘿蔔幼苗——帶綠的深色，種子萌芽中
  [CARROT_MATURE]:  [0.90, 0.52, 0.16], // 成熟胡蘿蔔——飽和橘色，可收割
  // 第三種作物 v1（馬鈴薯）——土黃偏棕，與小麥金黃、胡蘿蔔橘都能一眼分辨
  [POTATO_SEEDED]: [0.32, 0.30, 0.18], // 馬鈴薯幼苗——暗土黃，埋在土裡的感覺
  [POTATO_MATURE]:  [0.78, 0.64, 0.38], // 成熟馬鈴薯——土黃偏棕，可收割
};

const DEBUG = location.search.includes("debug");
// 觸控裝置偵測（用於顯示精簡 HUD 文字 + 啟用搖桿/跳鈕/放置鈕）
const isTouch = "ontouchstart" in window || navigator.maxTouchPoints > 0;
const hudEl = document.getElementById("hud");
const dbgEl = document.getElementById("dbg");

// 後端版本戳記：?debug=1 時 fetch /version，把「後端 commit / build 時間」顯示在 dbg HUD，
// 與前端內容雜湊（window.__BUILD__）並列 → 前後端版本都對得上 origin/main = 全上線了，一眼看出。
// 堵死「舊 binary 靜默上線、沒人發現」。失敗（端點不存在/離線）一律安全靜默，不影響遊戲。
let backendVersion = null;
if (DEBUG) {
  fetch("/version", { cache: "no-store" })
    .then((r) => (r.ok ? r.json() : null))
    .then((v) => { if (v) backendVersion = v; })
    .catch(() => {});
}
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
  let sr = sr0 + (sr1 - sr0) * f;
  let sg = sg0 + (sg1 - sg0) * f;
  let sb = sb0 + (sb1 - sb0) * f;
  // 下雨天氣 v1（ROADMAP 700）：天空/霧色往灰藍調混，讓下雨天一眼可辨。
  if (isRaining) {
    const [gr, gg, gb] = [0.42, 0.46, 0.52];
    const rw = 0.55;
    sr = sr + (gr - sr) * rw;
    sg = sg + (gg - sg) * rw;
    sb = sb + (gb - sb) * rw;
  }
  scene.background.setRGB(sr, sg, sb);
  scene.fog.color.setRGB(sr, sg, sb);

  // 插值太陽色與強度。
  const [ur0, ug0, ub0] = _hc(sun0);
  const [ur1, ug1, ub1] = _hc(sun1);
  sun.color.setRGB(ur0 + (ur1 - ur0) * f, ug0 + (ug1 - ug0) * f, ub0 + (ub1 - ub0) * f);
  sun.intensity = (si0 + (si1 - si0) * f) * (isRaining ? 0.6 : 1); // 下雨天陽光轉弱

  // 太陽軌跡：t=0.25 日出（東）、t=0.5 正午（頂）、t=0.75 日落（西）。
  const ang = (t - 0.25) * Math.PI * 2;
  sun.position.set(-Math.cos(ang) * 80, Math.sin(ang) * 80, 25);

  // 半球光強度。
  hemi.intensity = hi0 + (hi1 - hi0) * f;
}

// ── 下雨天氣 v1（ROADMAP 700）─────────────────────────────────────────────
// 伺服器機率式演變晴/雨並隨玩家快照廣播 raining:bool；前端只負責視覺：天空灰藍調 + 雨滴粒子。
// 宣告需在初始 updateSkyAndLight() 呼叫之前，避免其讀取 isRaining 時尚未初始化。
let isRaining = false;

// 初始套用，讓進場就是白天而非等第一幀快照。
updateSkyAndLight(worldTime);

// 雨滴粒子：單一 THREE.Points（一次 draw call，效能鐵律——別用逐滴 mesh）。
// 座標系相對鏡頭：每幀把整片粒子雲平移到鏡頭上方，粒子本身只在小範圍內落下+重置高度循環。
const RAIN_COUNT = 400;
const RAIN_SPREAD = 30;   // 粒子雲水平範圍（格）
const RAIN_HEIGHT = 20;   // 粒子雲垂直範圍（格），落到底部就重置回頂部
const RAIN_FALL_SPEED = 24; // 格/秒
const rainPositions = new Float32Array(RAIN_COUNT * 3);
for (let i = 0; i < RAIN_COUNT; i++) {
  rainPositions[i * 3 + 0] = (Math.random() - 0.5) * RAIN_SPREAD;
  rainPositions[i * 3 + 1] = Math.random() * RAIN_HEIGHT;
  rainPositions[i * 3 + 2] = (Math.random() - 0.5) * RAIN_SPREAD;
}
const rainGeom = new THREE.BufferGeometry();
rainGeom.setAttribute("position", new THREE.BufferAttribute(rainPositions, 3));
const rainMat = new THREE.PointsMaterial({
  color: 0xaac4e0, size: 0.12, transparent: true, opacity: 0.55, depthWrite: false,
});
const rainPoints = new THREE.Points(rainGeom, rainMat);
rainPoints.visible = false;
scene.add(rainPoints);

// 每幀推進雨滴下落（純視覺，無碰撞）；不下雨時整組隱藏、零成本早退。
function updateRain(dt) {
  if (!isRaining) { rainPoints.visible = false; return; }
  rainPoints.visible = true;
  // 粒子雲整體跟著鏡頭水平移動，讓雨看起來覆蓋玩家周遭而非固定世界座標。
  rainPoints.position.set(camera.position.x, camera.position.y + RAIN_HEIGHT / 2, camera.position.z);
  const pos = rainGeom.attributes.position;
  for (let i = 0; i < RAIN_COUNT; i++) {
    let y = pos.getY(i) - RAIN_FALL_SPEED * dt;
    if (y < -RAIN_HEIGHT / 2) y += RAIN_HEIGHT; // 落到底部循環回頂部
    pos.setY(i, y);
  }
  pos.needsUpdate = true;
}

// 方塊用 Lambert + 頂點色（每方塊上色），對光反應但靠半球光保底不黑。
// DoubleSide：切片① 求穩，避免任一面纏繞方向算錯被背面剔除成破洞/黑屏（perf 微讓步，之後可收回 FrontSide）。
const opaqueMat = new THREE.MeshLambertMaterial({ vertexColors: true, side: THREE.DoubleSide });

// ── 水體視覺升級 v1 ──────────────────────────────────────────────────────────
// 水 mesh 走頂點色（vertexColors: true），以便依流動等級染不同深淺。
// 來源水（WATER）：深藍不透明感；流動水 level 越高越淺色、更透明——一眼看出流向。
// 水面微動感：onBeforeCompile 注入 GLSL，頂面頂點依 time uniform 做輕微正弦波動。
// 水下氛圍：相機進入水方塊時，#underwaterOverlay 淡藍覆蓋層出現（CSS 即可、零 draw call）。

// 全域 time uniform——每幀在 update() 更新，共用給水面 shader。
const waterTime = { value: 0.0 };

// 水面動感 shader 注入：頂面頂點（normal.y > 0.9）沿 Y 軸輕微正弦偏移。
// 振幅 0.04 格，週期約 2.5 秒，水平位置也做微小 XZ offset 讓波紋更自然。
function makeWaterMat() {
  const mat = new THREE.MeshLambertMaterial({
    vertexColors: true,
    transparent: true,
    opacity: 0.60,
    side: THREE.DoubleSide,
  });
  // 避免 Three.js 快取到沒有 uTime uniform 的 program（每個水材質唯一 cache key）。
  mat.customProgramCacheKey = () => "butfun-water-wave-v1";
  mat.onBeforeCompile = (shader) => {
    shader.uniforms.uTime = waterTime;
    // 在 vertex shader 最前面宣告 uniform
    shader.vertexShader = "uniform float uTime;\n" + shader.vertexShader;
    // 在 #include <begin_vertex> 之後插入波動邏輯。
    // Three.js Lambert vertex shader 在 begin_vertex 時已有 position（attribute）
    // 和 objectNormal（已 decode 的法線），transformed 就是 begin_vertex 所建的工作座標。
    shader.vertexShader = shader.vertexShader.replace(
      "#include <begin_vertex>",
      `#include <begin_vertex>
      // 水面微動感：只對頂面（objectNormal.y > 0.9）的頂點做 Y 軸正弦偏移
      if (objectNormal.y > 0.9) {
        float wave = sin(position.x * 1.8 + uTime * 2.5) * 0.030
                   + sin(position.z * 1.6 + uTime * 1.9 + 1.2) * 0.025;
        transformed.y += wave;
      }
      `
    );
    // 記住 shader reference（uniforms 已含 uTime 指標，自動跟著 waterTime.value 更新）
    mat.userData.shader = shader;
  };
  return mat;
}
const waterMat = makeWaterMat();

// ── 火把發光 v1（ROADMAP 691）─────────────────────────────────────────────────
// 火把（block 31）放置後向周遭散發暖橘光；手持火把時鏡頭附近同樣有光。
// 純前端、零後端、零協議、零 migration、零 LLM。

// 追蹤世界中所有已知火把座標（key="wx,wy,wz" → {wx,wy,wz}）。
const torchPositions = new Map();
function torchKey(wx, wy, wz) { return wx + "," + wy + "," + wz; }
function registerTorchBlock(wx, wy, wz) {
  torchPositions.set(torchKey(wx, wy, wz), { wx, wy, wz });
}
function unregisterTorchBlock(wx, wy, wz) {
  torchPositions.delete(torchKey(wx, wy, wz));
}
/** 掃描整個 chunk 找火把並登記（chunk 載入時呼叫，讓重連後既有火把也有光）。 */
function scanChunkForTorches(cx, cy, cz) {
  const ch = chunks.get(ckey(cx, cy, cz));
  if (!ch) return;
  const bx = cx * CHUNK, by = cy * CHUNK, bz = cz * CHUNK;
  for (let ly = 0; ly < CHUNK; ly++)
    for (let lz = 0; lz < CHUNK; lz++)
      for (let lx = 0; lx < CHUNK; lx++)
        if (ch[lx + lz * CHUNK + ly * CHUNK * CHUNK] === TORCH)
          registerTorchBlock(bx + lx, by + ly, bz + lz);
}

const TORCH_LIGHT_COLOR = 0xff8820;     // 暖橘黃（比火把顏色稍橘，光感更暖）
const TORCH_LIGHT_INTENSITY = 2.5;      // 亮度（影響照亮半徑內的方塊面）
const TORCH_LIGHT_DIST = 10;            // 光照衰減半徑（方塊單位）
const MAX_TORCH_POOL = 6;               // 效能上限：同時啟用的近旁火把光數量

// 放置火把光源池——預建好 PointLight 陣列、只移位、不動態 add/remove（效能穩定）。
const torchLightPool = [];
for (let _i = 0; _i < MAX_TORCH_POOL; _i++) {
  const pl = new THREE.PointLight(TORCH_LIGHT_COLOR, 0, TORCH_LIGHT_DIST);
  pl.visible = false;
  scene.add(pl);
  torchLightPool.push(pl);
}

// 手持火把光源——手拿火把時從鏡頭附近散出較暗的暖光（8 格半徑）。
const heldTorchLight = new THREE.PointLight(TORCH_LIGHT_COLOR, 0, 8);
scene.add(heldTorchLight);

let _torchRefreshTimer = 0; // 每 2 秒刷新一次近旁火把光（非每幀掃，降 CPU）

/**
 * 更新「放置火把」光源池：
 * 選距鏡頭最近的 MAX_TORCH_POOL 個登記火把 → 移過去點亮；其餘熄滅。
 * 每 2 秒呼叫一次（中頻，非每幀）。
 */
function updateNearbyTorchLights() {
  if (torchPositions.size === 0) {
    for (const pl of torchLightPool) { pl.visible = false; pl.intensity = 0; }
    return;
  }
  const cx = camera.position.x, cy = camera.position.y, cz = camera.position.z;
  const sorted = [];
  for (const { wx, wy, wz } of torchPositions.values()) {
    const dx = wx + 0.5 - cx, dy = wy + 0.5 - cy, dz = wz + 0.5 - cz;
    sorted.push({ wx, wy, wz, d2: dx * dx + dy * dy + dz * dz });
  }
  sorted.sort((a, b) => a.d2 - b.d2);
  for (let i = 0; i < torchLightPool.length; i++) {
    const pl = torchLightPool[i];
    if (i < sorted.length) {
      const t = sorted[i];
      pl.position.set(t.wx + 0.5, t.wy + 0.5, t.wz + 0.5);
      pl.intensity = TORCH_LIGHT_INTENSITY;
      pl.visible = true;
    } else {
      pl.intensity = 0;
      pl.visible = false;
    }
  }
}
// ── end 火把發光 v1 ───────────────────────────────────────────────────────────

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

// 碰撞用：未載入(-1)視為空（不擋路、不卡人）；來源水/流動水與空氣皆不實心。
function solidCollide(wx, wy, wz) {
  const r = getRaw(wx, wy, wz);
  // 梯子（LADDER=35）、木門（開）（DOOR_OPEN=44）非實心——玩家可穿入；水與 AIR 同理不碰撞
  return r > 0 && !isWaterId(r) && r !== LADDER && r !== DOOR_OPEN;
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

// 不透明面是否該畫：相鄰是空氣、水或梯子（可視穿）才畫；未載入(-1)當作實心 → 不畫
//（避免世界邊緣冒出一面牆，等鄰塊串到再補）。
function faceVisibleOpaque(nx, ny, nz) {
  const r = getRaw(nx, ny, nz);
  if (r === -1) return false;
  // 梯子（LADDER=35）、木門（開）（DOOR_OPEN=44）是可穿越方塊，視覺上等同空氣
  return r === AIR || isWaterId(r) || r === LADDER || r === DOOR_OPEN;
}
// 水面可見性改由 rebuildChunk 內的水流分支就地判斷（含階梯落差牆），見 waterTopH/emitWaterFace。

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
  const wpos = [], wnorm = [], wcol = [], widx = [];
  const baseX = cx * CHUNK, baseY = cy * CHUNK, baseZ = cz * CHUNK;

  for (let ly = 0; ly < CHUNK; ly++) {
    for (let lz = 0; lz < CHUNK; lz++) {
      for (let lx = 0; lx < CHUNK; lx++) {
        const b = ch[lx + lz * CHUNK + ly * CHUNK * CHUNK];
        if (b === AIR) continue;
        const wx = baseX + lx, wy = baseY + ly, wz = baseZ + lz;
        if (isWaterId(b)) {
          // 流動水視覺（麥塊做法）：依等級渲染成遞減高度，形成往低處流的階梯水面。
          // 來源水（WATER）維持滿格；流動水 level 1..7 越遠越矮。純視覺，不動後端水流邏輯。
          const topH = waterTopH(b);
          for (const f of FACES) {
            const nb = getRaw(wx + f.d[0], wy + f.d[1], wz + f.d[2]);
            if (f.n[1] === 1) {
              // 頂面：上方空氣才露出水面，畫在 topH（矮水面一眼看得出在漫）。
              if (nb === AIR) emitWaterFace(wpos, wnorm, wcol, widx, lx, ly, lz, f, topH, topH, b);
            } else if (f.n[1] === -1) {
              // 底面：下方空氣才畫（避免內面）。
              if (nb === AIR) emitWaterFace(wpos, wnorm, wcol, widx, lx, ly, lz, f, 0, 0, b);
            } else {
              // 側面：鄰空氣→整片側牆(0..topH)；鄰為較矮的水→畫階梯落差牆(鄰topH..topH)，
              // 讓「越流越低」的水階在側面也看得出來，不是兩塊水之間破洞。
              if (nb === AIR) {
                emitWaterFace(wpos, wnorm, wcol, widx, lx, ly, lz, f, topH, 0, b);
              } else if (isWaterId(nb)) {
                const nH = waterTopH(nb);
                if (nH < topH - 1e-3) emitWaterFace(wpos, wnorm, wcol, widx, lx, ly, lz, f, topH, nH, b);
              }
            }
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
    g.setAttribute("color", new THREE.Float32BufferAttribute(wcol, 3));
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

// 水面高度（0..1）：來源水滿格；流動水依 level 遞減，形成往低處的階梯。純視覺、不動後端。
function waterTopH(b) {
  if (b === WATER) return 1.0;              // 來源水滿格
  const lvl = b - WATER_FLOW_BASE + 1;      // 1..7（越大＝離源越遠＝越矮）
  return Math.max(0.12, 1.0 - lvl * 0.11);  // level1≈0.89 … level7≈0.23
}

// 水體顏色：依流動等級深淺——來源水深藍，level 越高越淺越透明（一眼看出流向）。
// 回傳 [r, g, b]（0..1 線性），由 emitWaterFace 注入頂點色。
function waterColor(b) {
  if (b === WATER) return [0.13, 0.38, 0.80];   // 來源水：飽和深藍
  const lvl = b - WATER_FLOW_BASE + 1;            // 1..7
  const t = lvl / WATER_FLOW_MAX_LVL;             // 0..1（越大=越遠=越淡）
  // 從深藍（0.13,0.40,0.82）漸變到淺藍白（0.50,0.72,0.95）
  const r = 0.13 + t * (0.50 - 0.13);
  const g = 0.40 + t * (0.72 - 0.40);
  const bv = 0.82 + t * (0.95 - 0.82);
  return [r, g, bv];
}

// 推一個水面（4 頂點、2 三角）：頂邊在 yTop、底邊在 yBot（側面藉此畫出階梯落差牆）。
// wcol：水頂點色陣列；blockId：水方塊 id（決定深淺色）。
function emitWaterFace(pos, norm, col, idx, lx, ly, lz, f, yTop, yBot, blockId) {
  const start = pos.length / 3;
  const c = waterColor(blockId);
  for (const v of f.v) {
    const y = v[1] === 1 ? yTop : yBot;
    pos.push(lx + v[0], ly + y, lz + v[2]);
    norm.push(f.n[0], f.n[1], f.n[2]);
    col.push(c[0], c[1], c[2]);
  }
  idx.push(start, start + 1, start + 2, start, start + 2, start + 3);
}

// ── 水下氛圍 v1 ──────────────────────────────────────────────────────────────
// 相機進入水方塊 → #underwaterOverlay（淡藍色覆蓋層）淡入；離水即淡出。
// 零 Three.js draw call，只改 DOM 元素 opacity。
const _underwaterEl = document.getElementById("underwaterOverlay");
let _isUnderwater = false;
function updateUnderwaterAtmosphere() {
  const cx = Math.floor(camera.position.x);
  const cy = Math.floor(camera.position.y);
  const cz = Math.floor(camera.position.z);
  const blockAtCamera = getRaw(cx, cy, cz);
  const underwater = isWaterId(blockAtCamera);
  if (underwater !== _isUnderwater) {
    _isUnderwater = underwater;
    if (_underwaterEl) {
      _underwaterEl.style.opacity = underwater ? "1" : "0";
    }
    // 水下微霧：略縮短 Three.js fog，回到地面立即還原（與 overlay 獨立，確保霧效無論如何生效）
    scene.fog.near = underwater ? 6 : 40;
    scene.fog.far  = underwater ? 18 : 120;
  }
}
// ── end 水下氛圍 v1 ───────────────────────────────────────────────────────────

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

// 登入綁定（比照 3D #821）：開頁查 /auth/me，登入者拿到帳號身分 → 進場以帳號名綁定
// 記憶/好感度/背包（換訪客名也認得你）。訪客仍可逛。身份真相由後端 cookie 決定，
// 這兩個變數只供 UI 與「入場名先正確」用。
let isLoggedIn = false;
let myAccountName = null;

// 好感度 v1（ROADMAP 656）：我與各居民的互動記憶筆數（連線後從 /voxel/affinity 拉取）。
// key = resident_id, value = count (0=陌生人, 1-2=相識, 3+=友人)
const myAffinity = new Map();

// 玩家身體（第三人稱可見的小方塊角色）
const bodyGeo = new THREE.BoxGeometry(0.6, PH, 0.6);
const bodyMat = new THREE.MeshLambertMaterial({ color: 0xffcf6b });
const bodyMesh = new THREE.Mesh(bodyGeo, bodyMat);
scene.add(bodyMesh);

// 其他玩家：id -> { mesh, bubble, lastSay }（bubble = 頭上對話泡泡，embodied 靠近說話 v1）
const others = new Map();
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

// ── embodied 靠近說話 v1：自己頭上的對話泡泡（本地驅動，說話立即冒、計時消失）─────
// 不蓋住畫面、跟著角色在 3D 世界裡（「話活在世界裡」）。別人看到的版本走 players 廣播的 say。
const MY_BUBBLE_SECS = 6;
const myBubble = makeTextSprite("", true);
myBubble.visible = false;
scene.add(myBubble);
let myBubbleTimer = 0;
let myBubbleText = "";
function showMyBubble(text) {
  const t = (text || "").trim();
  if (!t) return;
  myBubbleText = t.slice(0, 60);
  setSpriteText(myBubble, myBubbleText, true);
  myBubble.visible = true;
  myBubbleTimer = MY_BUBBLE_SECS;
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

/** 製作心情指示燈 sprite（心情 emoji，居名牌左側，ROADMAP 676）。 */
function makeMoodSprite(emoji) {
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

function setMoodEmoji(sprite, emoji) {
  const fresh = makeMoodSprite(emoji);
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
  // 心情指示燈（ROADMAP 676）：伺服器廣播 mood emoji，偏置在名牌左側與好感度左右對稱。
  const moodIndicator = makeMoodSprite("");
  moodIndicator.position.set(-0.85, 2.05, 0);
  moodIndicator.visible = false;
  group.add(moodIndicator);
  scene.add(group);
  return { group, label, desireLabel, bubble, affinityIndicator, moodIndicator, lastName: name, lastSay: "", lastDesire: "", lastAffinity: "", lastMood: "" };
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
    // 心情指示燈（ROADMAP 676）：伺服器動態計算並廣播 mood emoji，sig 保護不重建貼圖。
    const moodEmoji = r.mood || "";
    if (moodEmoji !== ent.lastMood) {
      ent.lastMood = moodEmoji;
      if (moodEmoji) { setMoodEmoji(ent.moodIndicator, moodEmoji); ent.moodIndicator.visible = true; }
      else { ent.moodIndicator.visible = false; }
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
  releaseMouse(); // 桌機：開對話要放開滑鼠鎖定，游標才能打字
  chatEl.style.display = "flex";
  // 開定向對話 modal 時收起常駐說話列，避免兩者重疊。
  const sb = document.getElementById("speakBar");
  if (sb) sb.style.display = "none";
  updateGiftBtn(); // 贈禮 v1：更新按鈕顯示哪件物品
  hideTradeOffer(); // 換居民時清掉舊交易提案（不同居民的提案不共用）
}
function closeChat() {
  if (chatEl) chatEl.style.display = "none";
  const sb = document.getElementById("speakBar");
  if (sb) sb.style.display = "flex"; // 恢復常駐說話列
}

// 送一句話給目前對話的居民（指定對象＝點居民 / 走近面對）。
function sendTalk(text) {
  const t = (text || "").trim();
  if (!t || !chatRid || !wsReady) return;
  ws.send(JSON.stringify({ t: "talk", resident_id: chatRid, text: t.slice(0, 200) }));
  appendMsg("me", "你：" + t);
  showMyBubble(t); // embodied：自己頭上也冒泡（話活在世界裡）
}

// embodied 靠近說話 v1：範圍「說話」——不指定居民，伺服器挑半徑內最近/面對者回話，
// 其餘附近居民旁聽（進記憶、偶爾搭話）。不開 modal，回覆走世界裡的頭上泡泡。
function sendSpeak(text) {
  const t = (text || "").trim();
  if (!t || !wsReady) return;
  ws.send(JSON.stringify({ t: "talk", text: t.slice(0, 200) })); // 無 resident_id = 範圍說話
  showMyBubble(t); // 自己頭上立即冒泡（零延遲、不等伺服器來回）
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
  // 交易鈕：向當前居民請求以物易物（ROADMAP 670）。
  const tradeBtnEl = document.getElementById("chatTrade");
  if (tradeBtnEl) tradeBtnEl.addEventListener("click", () => { if (chatRid) tryRequestTrade(); });
  // 接受交易按鈕。
  const tradeAcceptBtnEl = document.getElementById("tradeAcceptBtn");
  if (tradeAcceptBtnEl) tradeAcceptBtnEl.addEventListener("click", () => { if (chatRid) sendTradeAccept(chatRid); });
}

// ── 常駐說話輸入列（embodied 靠近說話 v1）─────────────────────────────────────
// 底部常駐輸入列：打字 → 範圍「說話」（不必先點居民）。手機/直式友善、不開 modal。
const speakInputEl = document.getElementById("speakInput");
const speakSendEl = document.getElementById("speakSend");
if (speakInputEl && speakSendEl) {
  const fireSpeak = () => {
    sendSpeak(speakInputEl.value);
    speakInputEl.value = "";
    speakInputEl.blur(); // 送完收鍵盤焦點，讓 WASD 等遊戲鍵恢復作用
  };
  speakSendEl.addEventListener("click", fireSpeak);
  speakInputEl.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { fireSpeak(); e.preventDefault(); }
  });
}

// ── 居民贈禮 v1（ROADMAP 660）────────────────────────────────────────────────
// 把採來的材料化作一份心意送給居民；居民記得你的照料，好感度 +2。

/// 不可作為禮物的 block_id（純 inventory 物品 / 不合語意送出）。
const GIFT_EXCLUDED = new Set([0, 7, 12, DOOR_OPEN, CARROT_SEEDED, POTATO_SEEDED]); // Air / Water / FarmSoilSeeded / DoorOpen / CarrotSeeded / PotatoSeeded（伺服器狀態，不可贈）

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

// ── 居民以物易物（ROADMAP 670）───────────────────────────────────────────────
// 玩家點「⇌ 交易」→ 伺服器回 trade_offer → 前端顯示提案橫幅 → 玩家點接受 → 伺服器執行交易。

let lastTradeMs = 0;     // 交易請求本地冷卻（防連按）
let pendingTradeRid = null; // 目前有開放提案的居民 id

/** 請求與當前居民交易（發 TradeRequest，等 trade_offer 回應）。 */
function tryRequestTrade() {
  if (!wsReady || !chatRid) return;
  const now = Date.now();
  if (now - lastTradeMs < 2000) return; // 2 秒冷卻
  lastTradeMs = now;
  hideTradeOffer(); // 清掉舊提案
  ws.send(JSON.stringify({ t: "trade_request", resident_id: chatRid }));
}

/** 接受指定居民的交易提案（發 TradeAccept）。 */
function sendTradeAccept(rid) {
  if (!wsReady) return;
  ws.send(JSON.stringify({ t: "trade_accept", resident_id: rid }));
  hideTradeOffer();
}

/** 顯示交易提案橫幅（trade_offer 到來時呼叫）。 */
function showTradeOffer(m) {
  const el = document.getElementById("tradeOffer");
  const textEl = document.getElementById("tradeOfferText");
  if (!el || !textEl) return;
  pendingTradeRid = m.resident_id;
  const offerLine = m.offer_count > 1
    ? `${m.offer_name}×${m.offer_count}`
    : m.offer_name;
  const wantLine = m.want_count > 1
    ? `${m.want_name}×${m.want_count}`
    : m.want_name;
  textEl.textContent = `${m.resident_name || "居民"} 提議：給你 ${offerLine}，換你的 ${wantLine}`;
  el.style.display = "flex";
}

/** 隱藏交易提案橫幅。 */
function hideTradeOffer() {
  const el = document.getElementById("tradeOffer");
  if (el) el.style.display = "none";
  pendingTradeRid = null;
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
  // 日記＝瞥見居民沒說出口的內心（第一人稱反思，不是聊天謄本）。
  secTitle.textContent = entries.length ? "📝 內心的迴響（最新在前）" : "📝 內心的迴響";
  memSection.appendChild(secTitle);
  if (entries.length === 0) {
    const empty = document.createElement("div");
    empty.className = "diary-empty";
    empty.textContent = name + " 的心湖還很平靜……來跟她說說話，留下些漣漪吧。";
    memSection.appendChild(empty);
  } else {
    for (const e of entries) {
      const row = document.createElement("div");
      row.className = "diary-entry";
      // 只渲染反思文字（無玩家名 / 無原話）——隱私邊界已在後端把關。
      row.innerHTML = '<span class="diary-entry-text">' + escHtml(e.text) + '</span>';
      memSection.appendChild(row);
    }
  }
  diaryBodyEl.appendChild(memSection);

  // 模糊印象區塊（記憶 v2 最小可行版）：更早以前被記憶 cap 淘汰的舊記憶，
  // 沒有直接消失、留下一句去識別化的殘影——只在真的有淡忘過才顯示。
  if (page && page.faint_impression) {
    const impSection = document.createElement("div");
    impSection.className = "diary-impression";
    impSection.textContent = page.faint_impression;
    diaryBodyEl.appendChild(impSection);
  }
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

// ── 居民日記牆（ROADMAP 674）────────────────────────────────────────────────────
// 玩家從任何地方都可翻閱所有居民的日記——不需走近、不需開聊天——一覽 AI 居民的內心世界。
const diaryWallEl = document.getElementById("diaryWall");
const diaryWallBodyEl = document.getElementById("diaryWallBody");
const diaryWallBtnEl = document.getElementById("diaryWallBtn");

/**
 * 把所有居民的 DiaryPage 渲染成日記牆卡片列表（純函式，操作 DOM）。
 * 每張卡片顯示居民名、當前心願摘要、內心反思條數，並附「📖 詳細日記」按鈕。
 * @param {Array<{resident_id:string,resident_name:string,desire?:string,entries:Array}>} pages
 */
export function renderDiaryWall(pages) {
  if (!diaryWallBodyEl) return;
  if (!pages || pages.length === 0) {
    diaryWallBodyEl.innerHTML = '<div class="dw-empty">還沒有居民日記。</div>';
    return;
  }
  diaryWallBodyEl.innerHTML = "";
  for (const page of pages) {
    const card = document.createElement("div");
    card.className = "dw-card";
    const name = escHtml(page.resident_name || page.resident_id);
    const desireHtml = page.desire
      ? '<div class="dw-desire-label">💭 當前心願</div>' +
        '<div class="dw-desire">「' + escHtml(page.desire) + '」</div>'
      : '<div class="dw-meta">還沒有心願……等旅人的話語種下第一顆夢想。</div>';
    const entryCount = Array.isArray(page.entries) ? page.entries.length : 0;
    const metaHtml = entryCount > 0
      ? '<div class="dw-meta">共 ' + entryCount + ' 段內心反思</div>'
      : '<div class="dw-meta">心湖還很平靜……</div>';
    card.innerHTML =
      '<div class="dw-name">' + name + '</div>' +
      desireHtml +
      metaHtml;
    // 「詳細日記」按鈕：點擊後關日記牆、開個別居民完整日記。
    const openBtn = document.createElement("button");
    openBtn.className = "dw-open";
    openBtn.textContent = "📖 詳細日記";
    const rid = page.resident_id;
    openBtn.addEventListener("click", () => { closeDiaryWall(); openDiary(rid); });
    card.appendChild(openBtn);
    diaryWallBodyEl.appendChild(card);
  }
}

let diaryWallVisible = false;

/** 開啟日記牆面板，從 /voxel/diary 抓取所有居民日記並渲染卡片列表。 */
async function openDiaryWall() {
  if (!diaryWallEl) return;
  diaryWallVisible = true;
  diaryWallEl.style.display = "flex";
  if (diaryWallBodyEl) diaryWallBodyEl.innerHTML = '<div class="dw-empty">載入中…</div>';
  try {
    const resp = await fetch("/voxel/diary");
    if (!resp.ok) throw new Error("diary fetch: " + resp.status);
    const pages = await resp.json();
    renderDiaryWall(Array.isArray(pages) ? pages : []);
  } catch (_e) {
    if (diaryWallBodyEl) diaryWallBodyEl.innerHTML = '<div class="dw-empty">無法讀取日記。</div>';
  }
}

/** 關閉日記牆面板。 */
function closeDiaryWall() {
  diaryWallVisible = false;
  if (diaryWallEl) diaryWallEl.style.display = "none";
}

// 綁定日記牆按鈕與關閉事件。
if (diaryWallBtnEl) diaryWallBtnEl.addEventListener("click", () => {
  diaryWallVisible ? closeDiaryWall() : openDiaryWall();
});
if (diaryWallEl) {
  const closeBtn = document.getElementById("diaryWallClose");
  if (closeBtn) closeBtn.addEventListener("click", closeDiaryWall);
  // 點面板外（背景遮罩）也關閉。
  diaryWallEl.addEventListener("click", (e) => { if (e.target === diaryWallEl) closeDiaryWall(); });
}

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
  "居民易物": "⇌",
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

// ── 居民羅盤（ROADMAP 705）──────────────────────────────────────────────────
// 居民散居世界四方（653：露娜在原點、諾娃/賽勒/奧瑞各在南/西/東 75 格）之後，
// 玩家只能憑印象亂走才找得到人。本面板讀伺服器早已廣播的居民即時座標（零新協議），
// 純前端算出「往哪個方向、還有多遠」，讓散居系統第一次真的方便被使用。
const compassEl = document.getElementById("compassPanel");
const compassBodyEl = document.getElementById("compassBody");
const compassBtnEl = document.getElementById("compassBtn");

/** 世界座標下，從 (px,pz) 望向 (rx,rz) 的方位角（弧度）。
 * 與本引擎鏡頭朝向慣例同一套定義（`fwd(yaw) = (-sin(yaw), 0, -cos(yaw))`）：
 * 當 `worldBearing === yaw` 時，目標正好落在玩家正前方。
 * @returns {number} 方位角（弧度，未正規化）
 */
export function worldBearing(px, pz, rx, rz) {
  return Math.atan2(-(rx - px), -(rz - pz));
}

/** 目標相對玩家目前朝向（yaw）的螢幕旋轉角度（度，0~359）：
 * 0 = 正前方（箭頭朝上）、90 = 右方、180 = 正後方、270 = 左方。
 * 純函式、確定性，供羅盤箭頭 CSS `rotate()` 直接使用。
 * @returns {number} 0~359 的相對角度
 */
export function compassRelativeDeg(px, pz, rx, rz, yaw) {
  const rel = yaw - worldBearing(px, pz, rx, rz);
  const deg = (rel * 180 / Math.PI) % 360;
  return deg < 0 ? deg + 360 : deg;
}

let compassVisible = false;
let compassTimer = null;

/** 重新計算並渲染羅盤列表：依所有居民的即時座標算方位＋距離，離玩家近的排前面。 */
function renderCompassPanel() {
  if (!compassBodyEl) return;
  const rows = [...residents.values()].map((ent) => {
    const p = ent.group.position;
    const dx = p.x - player.x, dz = p.z - player.z;
    return {
      name: ent.lastName,
      dist: Math.hypot(dx, dz),
      deg: compassRelativeDeg(player.x, player.z, p.x, p.z, player.yaw),
    };
  }).sort((a, b) => a.dist - b.dist);
  if (rows.length === 0) {
    compassBodyEl.innerHTML = '<div class="compass-empty">目前沒有居民座標可指引。</div>';
    return;
  }
  compassBodyEl.innerHTML = "";
  for (const row of rows) {
    const div = document.createElement("div");
    div.className = "compass-row";
    div.innerHTML =
      '<span class="compass-arrow" style="transform: rotate(' + row.deg.toFixed(0) + 'deg)">↑</span>' +
      '<span class="compass-name">' + escHtml(row.name) + '</span>' +
      '<span class="compass-dist">' + Math.round(row.dist) + ' 格</span>';
    compassBodyEl.appendChild(div);
  }
}

/** 開啟居民羅盤面板，開始每 0.3 秒刷新一次方位（面板關閉時停止，不空耗）。 */
function openCompass() {
  if (!compassEl) return;
  compassVisible = true;
  compassEl.style.display = "flex";
  renderCompassPanel();
  if (compassTimer) clearInterval(compassTimer);
  compassTimer = setInterval(() => { if (compassVisible) renderCompassPanel(); }, 300);
}

/** 關閉居民羅盤面板。 */
function closeCompass() {
  compassVisible = false;
  if (compassEl) compassEl.style.display = "none";
  if (compassTimer) { clearInterval(compassTimer); compassTimer = null; }
}

if (compassBtnEl) compassBtnEl.addEventListener("click", () => {
  compassVisible ? closeCompass() : openCompass();
});
if (compassEl) {
  const closeBtn = document.getElementById("compassClose");
  if (closeBtn) closeBtn.addEventListener("click", closeCompass);
}

// ── 居民交情網（ROADMAP 708）────────────────────────────────────────────────
// 居民彼此拜訪（671）很久前就悄悄累積情誼（672：陌生→相識→老朋友），驅動問候語
// /八卦轉述（694）/互助蓋家（696），但這份資料只活在伺服器內部，玩家完全看不見
// 「這座小社會到底誰跟誰要好」。本面板讀新後端唯讀端點 `/voxel/relations`，把這
// 份隱形的社交網絡攤開給玩家看——跟羅盤（705）異曲同工：讓早已存在的系統第一次
// 被看見，而不是新造一套關係系統。
const relationsEl = document.getElementById("relationsPanel");
const relationsBodyEl = document.getElementById("relationsBody");
const relationsBtnEl = document.getElementById("relationsBtn");

const RELATION_TIER_ICON = { friend: "🤝", acquaintance: "🙂", stranger: "·" };
const RELATION_TIER_LABEL = { friend: "老朋友", acquaintance: "相識", stranger: "陌生" };
const RELATION_TIER_RANK = { friend: 2, acquaintance: 1, stranger: 0 };

/** 依情誼層級排序（老朋友優先），同層級依拜訪次數多到少排列。純函式、確定性、可測。
 * @param {Array<{a:string,b:string,tier:string,visits:number}>} rows
 * @returns {Array} 排序後的新陣列（不改動原陣列）
 */
export function sortRelationRows(rows) {
  return [...rows].sort((x, y) => {
    const r = (RELATION_TIER_RANK[y.tier] ?? 0) - (RELATION_TIER_RANK[x.tier] ?? 0);
    return r !== 0 ? r : (y.visits ?? 0) - (x.visits ?? 0);
  });
}

let relationsVisible = false;
let relationsRefreshTimer = null;

/** 重新渲染交情網列表。 */
function renderRelationsPanel(rows) {
  if (!relationsBodyEl) return;
  if (!rows || rows.length === 0) {
    relationsBodyEl.innerHTML = '<div class="relations-empty">目前沒有交情資料。</div>';
    return;
  }
  const sorted = sortRelationRows(rows);
  relationsBodyEl.innerHTML = "";
  for (const row of sorted) {
    const div = document.createElement("div");
    div.className = "relations-row tier-" + (row.tier || "stranger");
    div.innerHTML =
      '<span class="relations-icon">' + (RELATION_TIER_ICON[row.tier] || "·") + '</span>' +
      '<span class="relations-names">' + escHtml(row.a) + ' ↔ ' + escHtml(row.b) + '</span>' +
      '<span class="relations-tier">' + (RELATION_TIER_LABEL[row.tier] || "陌生") + '</span>';
    relationsBodyEl.appendChild(div);
  }
}

/** 向後端抓最新交情資料並重新渲染。 */
async function refreshRelations() {
  if (!relationsBodyEl) return;
  try {
    const resp = await fetch("/voxel/relations");
    if (!resp.ok) throw new Error("relations fetch failed: " + resp.status);
    const rows = await resp.json();
    renderRelationsPanel(rows);
  } catch (err) {
    relationsBodyEl.innerHTML = '<div class="relations-empty">無法讀取交情資料。</div>';
  }
}

/** 開啟居民交情網面板（情誼靠拜訪慢慢累積、變化很慢，30 秒刷新一次足夠，
 * 面板關閉時停止刷新，不背景空耗）。 */
function openRelations() {
  if (!relationsEl) return;
  relationsVisible = true;
  relationsEl.style.display = "flex";
  refreshRelations();
  if (relationsRefreshTimer) clearInterval(relationsRefreshTimer);
  relationsRefreshTimer = setInterval(() => { if (relationsVisible) refreshRelations(); }, 30_000);
}

/** 關閉居民交情網面板。 */
function closeRelations() {
  relationsVisible = false;
  if (relationsEl) relationsEl.style.display = "none";
  if (relationsRefreshTimer) { clearInterval(relationsRefreshTimer); relationsRefreshTimer = null; }
}

if (relationsBtnEl) relationsBtnEl.addEventListener("click", () => {
  relationsVisible ? closeRelations() : openRelations();
});
if (relationsEl) {
  const closeBtn = document.getElementById("relationsClose");
  if (closeBtn) closeBtn.addEventListener("click", closeRelations);
}

// ── 居民技能簿（ROADMAP 719）────────────────────────────────────────────────
// 技能發明（716）+ 傳授（717）至今只透過稍縱即逝的 Feed 文字曝光（「露娜教了我
// 『燒玻璃』這招！」播報一過就沒了），玩家從沒有任何管道能回頭查「這座小社會
// 現在誰會什麼」——這份資料只活在伺服器 `InventedSkillStore` 裡。跟 708 交情網
// 同一手法：讓早已存在的系統第一次被看見，而不是新造一套技能系統。
const skillsEl = document.getElementById("skillsPanel");
const skillsBodyEl = document.getElementById("skillsBody");
const skillsBtnEl = document.getElementById("skillsBtn");

/** 重新渲染技能簿列表。
 * @param {Array<{name:string, skills:string[]}>} rows
 */
function renderSkillsPanel(rows) {
  if (!skillsBodyEl) return;
  if (!rows || rows.length === 0) {
    skillsBodyEl.innerHTML = '<div class="skills-empty">目前沒有技能資料。</div>';
    return;
  }
  skillsBodyEl.innerHTML = "";
  for (const row of rows) {
    const div = document.createElement("div");
    div.className = "skills-row";
    const skills = row.skills || [];
    const chips = skills.length > 0
      ? '<div class="skills-chips">' +
        skills.map((s) => '<span class="skills-chip">' + escHtml(s) + '</span>').join("") +
        '</div>'
      : '<div class="skills-none">尚未發明任何技能</div>';
    div.innerHTML = '<span class="skills-name">' + escHtml(row.name) + '</span>' + chips;
    skillsBodyEl.appendChild(div);
  }
}

/** 向後端抓最新技能資料並重新渲染。 */
async function refreshSkills() {
  if (!skillsBodyEl) return;
  try {
    const resp = await fetch("/voxel/skills");
    if (!resp.ok) throw new Error("skills fetch failed: " + resp.status);
    const rows = await resp.json();
    renderSkillsPanel(rows);
  } catch (err) {
    skillsBodyEl.innerHTML = '<div class="skills-empty">無法讀取技能資料。</div>';
  }
}

let skillsVisible = false;
let skillsRefreshTimer = null;

/** 開啟居民技能簿面板（技能發明頻率低，30 秒刷新一次足夠，面板關閉時停止刷新）。 */
function openSkills() {
  if (!skillsEl) return;
  skillsVisible = true;
  skillsEl.style.display = "flex";
  refreshSkills();
  if (skillsRefreshTimer) clearInterval(skillsRefreshTimer);
  skillsRefreshTimer = setInterval(() => { if (skillsVisible) refreshSkills(); }, 30_000);
}

/** 關閉居民技能簿面板。 */
function closeSkills() {
  skillsVisible = false;
  if (skillsEl) skillsEl.style.display = "none";
  if (skillsRefreshTimer) { clearInterval(skillsRefreshTimer); skillsRefreshTimer = null; }
}

if (skillsBtnEl) skillsBtnEl.addEventListener("click", () => {
  skillsVisible ? closeSkills() : openSkills();
});
if (skillsEl) {
  const closeBtn = document.getElementById("skillsClose");
  if (closeBtn) closeBtn.addEventListener("click", closeSkills);
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
// 快捷欄麥塊化：固定 9 格（麥塊就是 9 格），數字鍵 1-9 選格、手機點選。
// 完整物品清單移到「背包」面板，從背包點物品即可指派進當前選中的快捷欄格。
// 空格 = AIR(0)；每格內容持久化到 localStorage，重整後保留。
const HOTBAR_SIZE = 9;
const HOTBAR_LS_KEY = "butfun.voxel.hotbar.v1";
// 預設起手：草/土/石/木/木板 + 木鎬，其餘留空；開局不空白、也不再洗版。
const HOTBAR_DEFAULT = [GRASS, DIRT, STONE, WOOD, PLANK, PICKAXE_WOOD, AIR, AIR, AIR];
// 從 localStorage 還原上次的快捷欄指派；資料壞掉或不可用就 fallback 預設。
function loadHotbar() {
  try {
    const raw = localStorage.getItem(HOTBAR_LS_KEY);
    if (raw) {
      const arr = JSON.parse(raw);
      if (Array.isArray(arr) && arr.length === HOTBAR_SIZE &&
          arr.every((n) => Number.isInteger(n) && n >= 0)) {
        return arr;
      }
    }
  } catch (_) { /* localStorage 不可用或壞資料：用預設 */ }
  return HOTBAR_DEFAULT.slice();
}
const HOTBAR = loadHotbar();
// 把當前快捷欄指派寫回 localStorage（每次指派/變更後呼叫）。
function saveHotbar() {
  try { localStorage.setItem(HOTBAR_LS_KEY, JSON.stringify(HOTBAR)); } catch (_) { /* 忽略：無痕/禁 storage */ }
}
// 指派一個 block 到指定快捷欄格（麥塊互動：從背包點物品→放進當前格），存檔並重建 UI。
function assignToHotbar(slot, blockId) {
  if (slot < 0 || slot >= HOTBAR_SIZE) return;
  HOTBAR[slot] = blockId;
  saveHotbar();
  buildHotbar();
  updateInvHud();
}
const BLOCK_NAME = {
  [GRASS]: "草", [DIRT]: "土", [STONE]: "石", [WOOD]: "木", [SAND]: "沙", [LEAVES]: "葉",
  [PLANK]: "木板", [STONE_BRICK]: "石磚", [GLASS]: "玻璃",
  // 種田 v1
  [FARM_SOIL]: "農田土", [FARM_SOIL_SEEDED]: "幼苗", [WHEAT_MATURE]: "成熟小麥",
  [SEEDS]: "種子",
  // 工作台 v1
  [WORKBENCH]: "工作台",
  // 熔爐 v1
  [FURNACE]: "熔爐", [SMOOTH_STONE]: "拋光石",
  // 麵包 v1（ROADMAP 668）
  [WHEAT]: "小麥", [BREAD]: "麵包",
  // 深層礦石 v1（ROADMAP 682）
  [COAL_ORE]: "煤礦", [IRON_ORE]: "鐵礦",
  // 鐵錠 v1（ROADMAP 683）
  [IRON_INGOT]: "鐵錠",
  // 鐵磚 v1（ROADMAP 684）
  [IRON_BLOCK]: "鐵磚",
  // 火把 v1（ROADMAP 685）
  [TORCH]: "火把",
  // 鎬具 v1（ROADMAP 687）
  [PICKAXE_WOOD]: "木鎬", [PICKAXE_STONE]: "石鎬", [PICKAXE_IRON]: "鐵鎬",
  [AXE_WOOD]: "木斧", [AXE_STONE]: "石斧", [AXE_IRON]: "鐵斧",
  // 梯子 v1（ROADMAP 688）
  [LADDER]: "梯子",
  // 鏟子 v1（ROADMAP 690）
  [SHOVEL_WOOD]: "木鏟", [SHOVEL_STONE]: "石鏟", [SHOVEL_IRON]: "鐵鏟",
  // 箱子 v1（ROADMAP 692）
  [CHEST]: "箱子",
  // 木門 v1（ROADMAP 693）
  [DOOR_CLOSED]: "木門（關）", [DOOR_OPEN]: "木門（開）",
  // 床 v1
  [BED]: "床",
  // 第二種作物 v1
  [CARROT_SEEDED]: "胡蘿蔔幼苗", [CARROT_MATURE]: "成熟胡蘿蔔",
  [CARROT_SEEDS]: "胡蘿蔔種子", [CARROT]: "胡蘿蔔",
  // 第三種作物 v1
  [POTATO_SEEDED]: "馬鈴薯幼苗", [POTATO_MATURE]: "成熟馬鈴薯",
  [POTATO_SEEDS]: "馬鈴薯種子", [POTATO]: "馬鈴薯",
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
    if (b === AIR) { if (cnt) cnt.textContent = ""; slot.classList.add("empty"); return; }
    const n = myInv.get(b) || 0;
    if (cnt) cnt.textContent = n > 0 ? "×" + n : "";
    slot.classList.toggle("empty", n === 0);
  });
}

function buildHotbar() {
  if (!hotbarEl) return;
  hotbarEl.innerHTML = "";
  HOTBAR.forEach((b, i) => {
    const isEmpty = (b === AIR); // 空格：只顯示格號，不放色塊/名稱
    const slot = document.createElement("div");
    slot.className = "slot" + (i === selectedSlot ? " sel" : "") + " empty";
    const sw = document.createElement("div");
    sw.className = "sw";
    if (isEmpty) {
      sw.style.background = "transparent";
    } else {
      const c = COLOR[b] || COLOR[STONE];
      sw.style.background = `rgb(${(c[0] * 255) | 0},${(c[1] * 255) | 0},${(c[2] * 255) | 0})`;
    }
    const lbl = document.createElement("div");
    lbl.textContent = isEmpty ? String(i + 1) : ((i + 1) + " " + (BLOCK_NAME[b] || "?"));
    const cnt = document.createElement("div");
    cnt.className = "cnt";
    slot.appendChild(sw); slot.appendChild(lbl); slot.appendChild(cnt);
    slot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      // 麥塊互動：背包開著且手上拿著物品（bagPick）→ 指派到這一格；否則單純選格。
      if (bagPanelVisible() && bagPick !== 0) {
        assignToHotbar(i, bagPick);
        bagPick = 0;
        selectSlot(i);
        renderBagPanel();
      } else {
        selectSlot(i);
      }
    });
    hotbarEl.appendChild(slot);
  });
  updateInvHud(); // 重建後補上數量/空格樣式
}
function selectSlot(i) {
  selectedSlot = ((i % HOTBAR.length) + HOTBAR.length) % HOTBAR.length;
  for (let k = 0; k < hotbarEl.children.length; k++) {
    hotbarEl.children[k].classList.toggle("sel", k === selectedSlot);
  }
}
function selectedBlock() { return HOTBAR[selectedSlot]; }
buildHotbar();
// 數字鍵 1-9 切快捷欄（麥塊固定 9 格）
addEventListener("keydown", (e) => {
  if (e.target && e.target.tagName === "INPUT") return; // 對話輸入中不搶鍵
  const n = parseInt(e.key, 10);
  if (n >= 1 && n <= HOTBAR.length) selectSlot(n - 1);
});

// 純函式：玩家 AABB 是否與任一梯子方塊重疊（攀爬判定）。
// 不依賴全域，可測；getBlock(bx,by,bz) 回傳方塊 id（未載入回 -1）。
function aabbHitsLadder(x, y, z, getBlock, pw, ph) {
  const x0 = Math.floor(x - pw), x1 = Math.floor(x + pw);
  const y0 = Math.floor(y), y1 = Math.floor(y + ph - 0.01);
  const z0 = Math.floor(z - pw), z1 = Math.floor(z + pw);
  for (let bx = x0; bx <= x1; bx++)
    for (let by = y0; by <= y1; by++)
      for (let bz = z0; bz <= z1; bz++)
        if (getBlock(bx, by, bz) === LADDER) return true;
  return false;
}

// 純函式：以「腳底在 y、半寬 pw、身高 ph」的 AABB，問 isSolid(bx,by,bz) 是否與任一實心格重疊。
// 不依賴全域（player/solidCollide 由呼叫端帶入），方便真瀏覽器 QA 直接餵假地形驗證。
function aabbHitsSolid(x, y, z, isSolid, pw, ph) {
  const x0 = Math.floor(x - pw), x1 = Math.floor(x + pw);
  const y0 = Math.floor(y), y1 = Math.floor(y + ph - 0.01);
  const z0 = Math.floor(z - pw), z1 = Math.floor(z + pw);
  for (let bx = x0; bx <= x1; bx++)
    for (let by = y0; by <= y1; by++)
      for (let bz = z0; bz <= z1; bz++)
        if (isSolid(bx, by, bz)) return true;
  return false;
}

// 純函式：脫困（depenetration）。若 (x,y,z) 的 AABB 卡在實心方塊內，沿 +Y 逐格上抬，
// 回傳第一個「不再重疊」的整數腳底高度（站到方塊頂上）。本來就沒卡 → 原值返回（不動）。
// 收斂穩定：找到第一個 clear 高度就停，不會持續往上彈；maxRise 防呆（理論上地表之上必有空氣）。
function unstuckY(x, y, z, isSolid, pw, ph, maxRise = 96) {
  if (!aabbHitsSolid(x, y, z, isSolid, pw, ph)) return y; // 沒卡：完全不干擾正常走路/重力/踏階
  let ny = Math.floor(y);
  for (let i = 0; i <= maxRise; i++) {
    if (!aabbHitsSolid(x, ny, z, isSolid, pw, ph)) return ny;
    ny += 1;
  }
  return y; // 極端情況找不到 clear：維持原值，至少不亂跳
}

// AABB 是否與任一實心方塊重疊（碰撞核心）；用上面的純函式，餵入玩家當前位置。
function overlaps() {
  return aabbHitsSolid(player.x, player.y, player.z, solidCollide, PW, PH);
}

// 脫困保險：玩家被實心方塊埋住時（出生瞬間 / 新 chunk 載入 / 從未載入區走入後補載），
// 把腳底上抬到第一個不重疊高度頂出來，避免永久卡死。只在「真的重疊實心」時作用，
// 平常走路/重力/踏階完全不觸發（unstuckY 開頭早退）。回傳是否有脫困。
function unstuckIfNeeded() {
  if (!overlaps()) return false;
  const ny = unstuckY(player.x, player.y, player.z, solidCollide, PW, PH);
  if (ny !== player.y) {
    player.y = ny;
    player.vy = 0;       // 清掉下墜速度，別把人又壓回方塊裡
    stepSmooth = 0;      // 視覺別殘留踏階補間
    player.grounded = false; // 讓重力把人輕輕放到地表，穩定收斂
    return true;
  }
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
    if (r > 0 && !isWaterId(r)) return { bx, by, bz, nx, ny, nz }; // 穿過來源/流動水，命中實心才停
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
  const old = ch[lx + lz * CHUNK + ly * CHUNK * CHUNK];
  // 火把發光 v1：放火把→登記；破壞火把→移除登記（讓光源池即時更新）。
  if (old === TORCH) unregisterTorchBlock(wx, wy, wz);
  ch[lx + lz * CHUNK + ly * CHUNK * CHUNK] = b;
  if (b === TORCH) registerTorchBlock(wx, wy, wz);
  markDirty(cx, cy, cz); // markDirty 只標該 chunk + 6 鄰塊
}

// ── 採礦手感 v1（ROADMAP 687）─────────────────────────────────────────────────
// 桌機：按住左鍵持續挖，挖滿進度條後才真正送 break（與手感一致）。
// 行動裝置：輕點保持即時挖（MCPE v1；採礦進度條不影響手機操作體感）。

// 方塊硬度（秒）——土/草/沙最快，礦石最慢。
const BLOCK_HARDNESS = {
  [GRASS]: 0.35, [DIRT]: 0.35, [SAND]: 0.35, [LEAVES]: 0.25,
  [WOOD]: 0.75,
  [PLANK]: 0.9, [STONE_BRICK]: 1.8,
  [STONE]: 1.8, [SMOOTH_STONE]: 2.0,
  [COAL_ORE]: 2.5, [IRON_ORE]: 2.5,
  [IRON_INGOT]: 1.5, [IRON_BLOCK]: 2.0,
  [FARM_SOIL]: 0.4, [FARM_SOIL_SEEDED]: 0.4, [WHEAT_MATURE]: 0.2,
  [CARROT_SEEDED]: 0.4, [CARROT_MATURE]: 0.2,
  [POTATO_SEEDED]: 0.4, [POTATO_MATURE]: 0.2,
  [WORKBENCH]: 1.2, [FURNACE]: 1.5,
  [TORCH]: 0.1,
  [LADDER]: 0.4,  // 梯子——木製，輕鬆打破
  [CHEST]: 1.0,   // 箱子——木箱，中等硬度（含存量，需謹慎破壞）
  [DOOR_CLOSED]: 0.8,  // 木門（關）——木製，輕鬆打破
  [DOOR_OPEN]:   0.8,  // 木門（開）——同材質，可破壞
  [BED]: 0.6,  // 床——布料+木架，輕鬆打破
};
function blockHardness(bid) { return BLOCK_HARDNESS[bid] ?? 1.0; }

// 鎬具加速倍率（持特定鎬對石/礦類方塊的速度倍數）。
function pickaxeBonus(bid) {
  const stoneTypes = [STONE, STONE_BRICK, SMOOTH_STONE, COAL_ORE, IRON_ORE, IRON_BLOCK, IRON_INGOT, WORKBENCH, FURNACE];
  if (!stoneTypes.includes(bid)) return 1.0;
  if ((myInv.get(PICKAXE_IRON) || 0) > 0) return 6.0;   // 鐵鎬：最快
  if ((myInv.get(PICKAXE_STONE) || 0) > 0) return 4.0;  // 石鎬：快
  if ((myInv.get(PICKAXE_WOOD) || 0) > 0) return 2.5;   // 木鎬：普通加速
  return 1.0; // 空手：基礎速度
}

// 斧頭加速倍率（持特定斧對木材類方塊的速度倍數；ROADMAP 689）。
// 只對木頭/葉片/木板有效；鎬具類方塊回 1.0（斧頭不補石礦）。
export function axeBonus(bid) {
  const woodTypes = [WOOD, LEAVES, PLANK];
  if (!woodTypes.includes(bid)) return 1.0;
  if ((myInv.get(AXE_IRON) || 0) > 0) return 6.0;   // 鐵斧：最快
  if ((myInv.get(AXE_STONE) || 0) > 0) return 4.0;  // 石斧：快
  if ((myInv.get(AXE_WOOD) || 0) > 0) return 2.5;   // 木斧：普通加速
  return 1.0; // 空手：基礎速度
}

// 鏟子加速倍率（持特定鏟對軟土類方塊的速度倍數；ROADMAP 690）。
// 只對草地/泥土/沙地/農田有效；石/木類方塊回 1.0（鏟子不補硬材）。
export function shovelBonus(bid) {
  const softTypes = [GRASS, DIRT, SAND, FARM_SOIL, FARM_SOIL_SEEDED];
  if (!softTypes.includes(bid)) return 1.0;
  if ((myInv.get(SHOVEL_IRON) || 0) > 0) return 6.0;   // 鐵鏟：最快
  if ((myInv.get(SHOVEL_STONE) || 0) > 0) return 4.0;  // 石鏟：快
  if ((myInv.get(SHOVEL_WOOD) || 0) > 0) return 2.5;   // 木鏟：普通加速
  return 1.0; // 空手：基礎速度
}

// 採礦狀態（桌機按住左鍵期間維持）。
let mining = null; // { x, y, z, bid, progress, total } 或 null

// 進度條 DOM（在 crosshair 正下方渲染進度）。
const miningBarEl = document.getElementById("miningBar");
const miningBarFillEl = document.getElementById("miningBarFill");

function updateMiningBar(frac) {
  if (!miningBarEl) return;
  if (frac === null) {
    miningBarEl.style.display = "none";
    return;
  }
  miningBarEl.style.display = "block";
  if (miningBarFillEl) miningBarFillEl.style.width = Math.min(1, frac) * 100 + "%";
}

/** 開始對準心對準的方塊計時挖掘（桌機模式）。*/
function startMining() {
  if (!target || !wsReady) return;
  const bid = getRaw(target.bx, target.by, target.bz);
  const hardness = blockHardness(bid);
  // 鎬具補石/礦，斧頭補木材，鏟子補土/沙，三者互補不疊加（各自對其他類回 1.0）
  const bonus = pickaxeBonus(bid) * axeBonus(bid) * shovelBonus(bid);
  const total = hardness / bonus; // 實際需要幾秒
  mining = { x: target.bx, y: target.by, z: target.bz, bid, progress: 0, total };
  updateMiningBar(0);
}

/** 取消當前採礦計時（鬆開左鍵、切換目標時）。*/
function cancelMining() {
  mining = null;
  updateMiningBar(null);
}

/** 每幀推進採礦進度（dt 秒），完成時送 break 訊息。應在 requestAnimationFrame 迴圈呼叫。*/
function tickMining(dt) {
  if (!mining) return;
  // 若準心目標改變（轉頭對準另一格），重置進度。
  if (!target || target.bx !== mining.x || target.by !== mining.y || target.bz !== mining.z) {
    cancelMining();
    if (target && isMouseDown) startMining();
    return;
  }
  mining.progress += dt;
  if (mining.progress >= mining.total) {
    // 進度滿：送 break，立刻開始下一塊（如果按著）。
    ws.send(JSON.stringify({ t: "break", x: mining.x, y: mining.y, z: mining.z }));
    cancelMining();
    if (isMouseDown) startMining();
  } else {
    updateMiningBar(mining.progress / mining.total);
  }
}

// 破壞準心對準的方塊：送 break（伺服器驗證後廣播 → setLocalBlock 套用）。回傳被挖座標或 null。
// 注意：行動裝置仍走即時送 break（MCPE 快感體驗）；桌機走 startMining / tickMining。
function breakAtTarget() {
  if (!target || !wsReady) return null;
  const c = { x: target.bx, y: target.by, z: target.bz };
  ws.send(JSON.stringify({ t: "break", x: c.x, y: c.y, z: c.z }));
  return c;
}

// 桌機是否按住左鍵（追蹤採礦狀態用）。
let isMouseDown = false;
// 在準心方塊的「面外側」放一個方塊：座標 = 命中方塊 + 命中面法線。回傳放置座標或 null。
// 種田 v1 + 工作台 v1：特殊互動邏輯，再 fallback 到一般放置。
function placeAtTarget() {
  if (!target || !wsReady) return null;
  // 空的快捷欄格（AIR）：沒選任何方塊，靜默忽略（避免送出 place AIR 誤刪）。
  if (selectedBlock() === AIR) return null;
  // 工作台互動：右鍵對準工作台方塊 → 開啟 3×3 合成面板（不放置新方塊）。
  if (getRaw(target.bx, target.by, target.bz) === WORKBENCH) {
    openWbPanel();
    return null;
  }
  // 熔爐互動：右鍵對準熔爐方塊 → 開啟冶煉面板（不放置新方塊）。
  if (getRaw(target.bx, target.by, target.bz) === FURNACE) {
    openFurnacePanel();
    return null;
  }
  // 箱子互動：右鍵對準箱子方塊 → 傳送 open_chest，伺服器回 chest_view 後開面板。
  if (getRaw(target.bx, target.by, target.bz) === CHEST) {
    openChestPanel(target.bx, target.by, target.bz);
    return null;
  }
  // 木門互動（ROADMAP 693）：右鍵對準門（開或關）→ 傳送 toggle_door，伺服器廣播新狀態。
  const _doorRaw = getRaw(target.bx, target.by, target.bz);
  if (_doorRaw === DOOR_CLOSED || _doorRaw === DOOR_OPEN) {
    ws.send(JSON.stringify({ t: "toggle_door", x: target.bx, y: target.by, z: target.bz }));
    return null;
  }
  // 床互動：右鍵對準床 → 傳送 sleep_in_bed，夜晚時伺服器把時鐘撥到隔天黎明。
  if (getRaw(target.bx, target.by, target.bz) === BED) {
    ws.send(JSON.stringify({ t: "sleep_in_bed", x: target.bx, y: target.by, z: target.bz }));
    return null;
  }
  // 種子的特殊種植動作：目標是農田土本身（不偏移到面外側）。
  // 第二種作物 v1：胡蘿蔔種子選中時種下胡蘿蔔；第三種作物 v1：馬鈴薯種子選中時種下馬鈴薯，
  // 皆附帶 seed 欄位讓伺服器分辨作物種類。
  if (selectedBlock() === SEEDS || selectedBlock() === CARROT_SEEDS || selectedBlock() === POTATO_SEEDS) {
    const hitRaw = getRaw(target.bx, target.by, target.bz);
    if (hitRaw === FARM_SOIL) {
      const sel = selectedBlock();
      const seed = sel === CARROT_SEEDS ? CARROT_SEEDS : sel === POTATO_SEEDS ? POTATO_SEEDS : undefined;
      ws.send(JSON.stringify({ t: "plant", x: target.bx, y: target.by, z: target.bz, seed }));
      return { x: target.bx, y: target.by, z: target.bz };
    }
    // 種子只能種在農田土上——其他方塊靜默忽略。
    return null;
  }
  // 麵包 v1（ROADMAP 668）+ 胡蘿蔔（第二種作物 v1）+ 馬鈴薯（第三種作物 v1）：純物品，不可放置——靜默忽略。
  if (selectedBlock() === WHEAT || selectedBlock() === BREAD || selectedBlock() === CARROT || selectedBlock() === POTATO) return null;
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
  // 麥塊式快捷鍵：F5 切第一/三人稱、E 開/關背包（開背包會釋放滑鼠鎖定）。
  if (e.code === "F5") { e.preventDefault(); toggleViewMode(); }
  if (e.code === "KeyE") {
    e.preventDefault();
    if (bagPanelVisible()) closeBagPanel(); else openBagPanel();
  }
});
addEventListener("keyup", (e) => { keys[e.code] = false; });

// ── 視角模式 + 俯仰 ─────────────────────────────────────────────────────────
// camPitch：視線俯仰（0=水平、正=往下看、負=往上看），yaw+pitch 純前端相機視角，
// 後端玩家只有 yaw（pitch 不上傳、不影響移動/面向）。
let camPitch = 0.35;
// 視角模式：'first'=第一人稱（相機在眼睛、藏身體）、'third'=第三人稱（後上方跟隨、看得到身體）。
// 預設：桌機第一人稱、手機第三人稱（麥塊桌機/MCPE 觸控各自的成熟手感）。
let viewMode = isTouch ? "third" : "first";
// 俯仰上下限：第一人稱可近乎正負直視（±83°）；第三人稱維持較窄，避免鏡頭鑽進地面。
function pitchLimits() { return viewMode === "first" ? [-1.45, 1.45] : [-0.2, 1.3]; }
function clampPitch() { const [lo, hi] = pitchLimits(); camPitch = Math.max(lo, Math.min(hi, camPitch)); }

// 切換第一/三人稱（F5 或 👁 鈕）：重新夾俯仰到該模式範圍、切身體可見性、更新鈕字。
function toggleViewMode() {
  viewMode = (viewMode === "first") ? "third" : "first";
  clampPitch();
  bodyMesh.visible = (viewMode !== "first"); // 第一人稱藏自己身體（每幀也會再設一次，冪等）
  const vb = document.getElementById("viewBtn");
  if (vb) vb.textContent = (viewMode === "first") ? "👁 第一人稱" : "🧍 第三人稱";
}

// 桌機滑鼠鎖定（pointer lock）：點畫面進入、Esc 離開（麥塊桌機手感）。
let pointerLocked = false;
const MOUSE_SENS = 0.0022;
// 有面板/對話開著時不進滑鼠鎖定（那些需要游標操作）。
function anyPanelOpen() {
  return bagPanelVisible() || wbPanelVisible() || furnacePanelVisible() || chestPanelVisible() ||
         (chatEl && chatEl.style.display === "flex");
}
// 釋放滑鼠鎖定（開面板/對話時呼叫，讓游標回來能點格子/打字）。
function releaseMouse() { if (pointerLocked) { try { document.exitPointerLock(); } catch (e) {} } }
document.addEventListener("pointerlockchange", () => {
  pointerLocked = (document.pointerLockElement === renderer.domElement);
});

if (!isTouch) {
  // 桌機：麥塊式滑鼠鎖定視角。沒鎖定時點畫面＝進入視角（此下不當破壞）；
  // 鎖定中：mousemove 轉 yaw+pitch、左鍵挖/點居民對話、右鍵放置，準心固定螢幕中心。
  renderer.domElement.addEventListener("mousedown", (e) => {
    if (!pointerLocked) {
      if (!anyPanelOpen()) renderer.domElement.requestPointerLock();
      return;
    }
    if (e.button === 2) { placeAtTarget(); return; }
    if (e.button !== 0) return;
    isMouseDown = true;
    // 鎖定中游標藏在螢幕中心 → 用中心點做居民 raycast / 破壞。
    const cx = window.innerWidth / 2, cy = window.innerHeight / 2;
    const rid = pickResident(cx, cy);
    if (rid) {
      const ent = residents.get(rid);
      openChat(rid, ent && ent.lastName);
    } else {
      startMining(); // 採礦手感 v1：開始計時挖掘，而非立即 break
    }
  });
  document.addEventListener("mouseup", (e) => {
    if (e.button === 0) { isMouseDown = false; cancelMining(); }
  });
  document.addEventListener("mousemove", (e) => {
    if (!pointerLocked) return;
    player.yaw -= e.movementX * MOUSE_SENS;
    camPitch += e.movementY * MOUSE_SENS; // 滑鼠往下＝往下看（往下 movementY>0 → pitch 增）
    clampPitch();
  });
  // 右鍵放置：擋掉瀏覽器選單。
  renderer.domElement.addEventListener("contextmenu", (e) => e.preventDefault());
}

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
      camPitch += (t.clientY - cy0) * 0.006; clampPitch(); // 夾到目前模式的俯仰範圍
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
    // 入場名：登入者用帳號名（伺服器仍以 cookie 為準覆蓋，這裡只是讓 HUD 先正確），
    // 訪客用 localStorage 暫存名。身份綁定的真相在後端 resolve_identity。
    let nm = "旅人";
    try { nm = (myAccountName || localStorage.getItem("butfun_name") || "旅人"); } catch (e) { nm = myAccountName || "旅人"; }
    ws.send(JSON.stringify({ t: "join", name: nm }));
  };
  ws.onmessage = (ev) => {
    let m; try { m = JSON.parse(ev.data); } catch (e) { return; }
    if (m.t === "welcome") {
      myId = m.id; myName = m.name || "旅人";
      player.x = m.spawn.x; player.y = m.spawn.y; player.z = m.spawn.z;
      // 出生瞬間先脫困一次（若出生 chunk 已到、地表把人埋住，立刻頂出來）。
      unstuckIfNeeded();
      // 好感度 v1：連線後立即拉取與各居民的好感度，讓指示燈盡快亮起。
      refreshAffinity();
    } else if (m.t === "chunks") {
      for (const c of m.chunks) {
        const key = ckey(c.cx, c.cy, c.cz);
        chunks.set(key, b64ToBytes(c.data));
        markDirty(c.cx, c.cy, c.cz);
        scanChunkForTorches(c.cx, c.cy, c.cz); // 火把發光 v1：掃描新 chunk 的既有火把
      }
      // chunk 載入後立刻脫困：若新載入的方塊把玩家埋住（出生／chunk 邊緣／下落最常見），
      // 同一則訊息就把人頂出來，不必等下一幀。沒卡則零成本早退。
      unstuckIfNeeded();
    } else if (m.t === "block") {
      // 伺服器權威方塊更新（破壞/放置）：本地套用 + 只重建受影響 chunk。
      setLocalBlock(m.x, m.y, m.z, m.b);
    } else if (m.t === "players") {
      const seen = new Set();
      for (const p of m.players) {
        if (p.id === myId) continue;
        seen.add(p.id);
        let ent = others.get(p.id);
        if (!ent) {
          const mesh = new THREE.Mesh(bodyGeo, otherMat); scene.add(mesh);
          // 頭上對話泡泡（child of mesh，sprite 永遠面向鏡頭、不受 mesh 旋轉影響）。
          const bubble = makeTextSprite("", true);
          bubble.position.y = PH / 2 + 1.7; // mesh 原點在身體中心，泡泡浮到頭頂上方
          bubble.visible = false;
          mesh.add(bubble);
          ent = { mesh, bubble, lastSay: "" };
          others.set(p.id, ent);
        }
        ent.mesh.position.set(p.x, p.y + PH / 2, p.z);
        ent.mesh.rotation.y = p.yaw || 0;
        // embodied：別人說話 → 頭上冒泡（你走過會「聽到」別人在聊，世界有人聲）。
        const say = p.say || "";
        if (say !== ent.lastSay) {
          ent.lastSay = say;
          if (say) { setSpriteText(ent.bubble, say, true); ent.bubble.visible = true; }
          else { ent.bubble.visible = false; }
        }
      }
      for (const [id, ent] of others) if (!seen.has(id)) { scene.remove(ent.mesh); others.delete(id); }
      // 乙太方界 AI 居民（與玩家分開的陣列）：位置/名字/說的話。
      if (m.residents) updateResidents(m.residents);
      // 晝夜循環 v1：伺服器每幀帶 time_of_day(0.0–1.0)，前端據此更新天空/光照。
      // 下雨天氣 v1（ROADMAP 700）：raining 隨同一份快照送達，一併觸發天空重繪。
      let skyDirty = false;
      if (typeof m.time_of_day === "number") { worldTime = m.time_of_day; skyDirty = true; }
      if (typeof m.raining === "boolean" && m.raining !== isRaining) { isRaining = m.raining; skyDirty = true; }
      if (skyDirty) updateSkyAndLight(worldTime);
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
      if (chestPanelVisible()) renderChestPanel(); // 箱子 v1：背包變動後同步更新箱子面板背包區
    } else if (m.t === "inv_denied") {
      // 採集 v1：放置材料不足，短暫提示。
      const bname = BLOCK_NAME[m.block_id] || "方塊";
      showErr("材料不足：" + bname + "（先去挖一些吧）");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "craft_ok") {
      // 合成成功（背包 2×2 或工作台 3×3）→ 清空對應格子 + 重繪面板。
      showMsg("合成成功：" + m.name_zh + " ×" + m.out_count + "！");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2200);
      bagGrid.fill(0); bagPick = 0;
      wbGrid.fill(0); wbPick = 0;
      if (bagPanelVisible()) renderBagPanel();
      if (wbPanelVisible()) renderWbPanel();
      if (furnacePanelVisible()) renderFurnacePanel();
    } else if (m.t === "craft_fail") {
      showErr("材料不足，無法合成（先多採集一些）");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
      if (bagPanelVisible()) renderBagPanel();
      if (wbPanelVisible()) renderWbPanel();
      if (furnacePanelVisible()) renderFurnacePanel();
    } else if (m.t === "plant_ok") {
      // 種田 v1 / 水耕農業 v1（ROADMAP 686）/ 第二/三種作物 v1：依作物種類 + 是否鄰近水源給不同提示。
      const plantMsg = m.carrot
        ? (m.irrigated ? "💧 水耕！胡蘿蔔將在 30 秒後成熟 🥕" : "已種下胡蘿蔔種子！等 60 秒就成熟 🥕")
        : m.potato
        ? (m.irrigated ? "💧 水耕！馬鈴薯將在 60 秒後成熟 🥔" : "已種下馬鈴薯種子！等 120 秒就成熟，收成量大 🥔")
        : (m.irrigated ? "💧 水耕！種子將在 45 秒後成熟 🌾" : "已種下種子！等 90 秒小麥就成熟 🌾");
      showMsg(plantMsg);
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
    } else if (m.t === "return_gift") {
      // 居民回禮 v1（ROADMAP 667）：只有當事玩家才顯示提示並更新背包。
      if (m.player === myName) {
        const iname = BLOCK_NAME[m.item_id] || m.item_name || "物品";
        // 更新本地庫存（伺服器已累計新數量）。
        if (m.new_count > 0) myInv.set(m.item_id, m.new_count);
        updateInvHud();
        updateGiftBtn();
        // 顯示溫馨提示（比一般系統訊息更暖、附愛心）。
        appendMsg("sys", "💛 " + (m.resident_name || "居民") + " 把 " + iname + " ×" + m.qty + " 送給你了！");
      }
    } else if (m.t === "fetch_delivered") {
      // 跑腿採集 v1（指令→任務第三刀）：只有下單的當事玩家才顯示提示並更新背包。
      if (m.player === myName) {
        const iname = BLOCK_NAME[m.item_id] || m.item_name || "物品";
        if (m.new_count > 0) myInv.set(m.item_id, m.new_count);
        updateInvHud();
        updateGiftBtn();
        appendMsg("sys", "📦 " + (m.resident_name || "居民") + "：" + (m.line || ("這是你要的" + iname + "！")));
      }
    } else if (m.t === "welcome_back") {
      // 久別重逢摘要 v1（ROADMAP 721）：只有自己看得到，離線期間世界發生的重要事件摘要。
      if (m.text) appendMsg("sys", m.text);
    } else if (m.t === "build_complete") {
      // 建物完工廣播 v1（ROADMAP 669）：全員可見，世界在長大。
      const who = m.resident || "居民";
      const what = m.kind || "建物";
      appendMsg("sys", "🏗️ " + who + " 完成了「" + what + "」的建造！走近去看看吧。");
    } else if (m.t === "wish_come_true") {
      // 心願真的成真 v1（ROADMAP 720）：這座建物是某位玩家的話種下的心願，全員可見；
      // 啟發者本人看到會格外有感（「我隨口說的話，真的被記住、被實現了」）。
      const who = m.resident || "居民";
      const what = m.kind || "建物";
      const player = m.player || "旅人";
      const mine = player === myName;
      appendMsg("sys", "🌟 " + who + " 因為" + (mine ? "你" : player) + "的一句話，把「" + what + "」蓋好了！");
    } else if (m.t === "item_wish_fulfilled") {
      // 送對禮物 v1（ROADMAP 722）：玩家送來的禮物正好是居民一直想要的東西，全員可見；
      // 送禮的當事人本人看到會格外有感（「我隨口送的東西，真的是她想要的」）。
      const who = m.resident || "居民";
      const item = m.item || "物品";
      const player = m.player || "旅人";
      const mine = player === myName;
      appendMsg("sys", "🎁 " + (mine ? "你" : player) + "送來的" + item + "，正是" + who + "一直想要的！");
    } else if (m.t === "trade_offer") {
      // 居民交易 v1（ROADMAP 670）：收到交易提案，顯示橫幅讓玩家確認。
      showTradeOffer(m);
      appendMsg("sys", "⇌ " + (m.resident_name || "居民") + " 想和你交易");
    } else if (m.t === "trade_done") {
      // 交易完成：顯示成功訊息，收起提案。
      hideTradeOffer();
      const got = m.got_count > 1 ? `${m.got_name}×${m.got_count}` : m.got_name;
      const gave = m.gave_count > 1 ? `${m.gave_name}×${m.gave_count}` : m.gave_name;
      appendMsg("sys", "✅ 交易成功！你給出了 " + gave + "，得到了 " + got);
      updateGiftBtn();
    } else if (m.t === "trade_fail") {
      // 交易失敗（太遠 / 沒材料 / 提案過期）。
      hideTradeOffer();
      showErr(m.reason || "交易失敗");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2200);
    } else if (m.t === "chest_view") {
      // 箱子 v1（ROADMAP 692）：伺服器回傳箱子內容，更新面板。
      _chestPos = { x: m.x, y: m.y, z: m.z };
      _chestItems = (m.items || []).slice();
      if (chestPanelEl) {
        chestPanelEl.style.display = "flex";
        renderChestPanel();
      }
    } else if (m.t === "chest_fail") {
      // 箱子操作失敗（數量不足等）。
      showErr(m.reason || "箱子操作失敗");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    } else if (m.t === "sleep_ok") {
      // 床 v1：睡覺成功——時鐘已跳到黎明（time_of_day 隨下一份 players 快照自動更新天色）。
      showMsg("😴 睡了一覺，天亮了！");
      setTimeout(() => { const e = document.getElementById("msg"); if (e) e.style.display = "none"; }, 2200);
    } else if (m.t === "sleep_fail") {
      // 床 v1：白天/黎明/黃昏睡不著。
      showErr(m.reason || "睡不著");
      setTimeout(() => { const e = document.getElementById("err"); if (e) e.style.display = "none"; }, 2000);
    }
  };
  ws.onclose = () => { wsReady = false; showErr("連線中斷，重新連線中…"); setTimeout(connect, 1500); };
  ws.onerror = () => { showErr("連線錯誤"); };
}
connect();

// ── Google 登入入口（比照 3D #821；沿用 2D/3D 同一套同源 cookie session）─────────────
// 右上角帳號 chip：訪客→「🔑 登入」（點了走既有 /auth/google/start）；登入→「帳號名 · 登出」。
// 登入後伺服器以 cookie 解出帳號 → join 自動綁帳號（記憶/好感度/背包跨 session 認得你）。
// 手機/直式友善：chip 小、靠右上、pointer-events 自理，不擋遊戲操作。
const acctEl = document.getElementById("acct");

function renderAccountChip() {
  if (!acctEl) return;
  acctEl.innerHTML = "";
  if (isLoggedIn) {
    const name = document.createElement("span");
    name.className = "acct-name";
    name.textContent = myAccountName || myName || "帳號";
    name.title = "已登入：" + (myAccountName || "");
    const out = document.createElement("button");
    out.type = "button";
    out.className = "acct-btn";
    out.textContent = "登出";
    out.title = "登出回到訪客";
    out.addEventListener("click", (e) => {
      if (e && e.preventDefault) e.preventDefault();
      // 清 session cookie 後重整：回訪客態（與 2D/3D 登出一致）。
      fetch("/auth/logout", { method: "POST", credentials: "same-origin" })
        .catch(() => {})
        .then(() => { try { location.reload(); } catch (_) { /* 測試 DOM 無 location */ } });
    });
    acctEl.appendChild(name);
    acctEl.appendChild(out);
  } else {
    const login = document.createElement("button");
    login.type = "button";
    login.className = "acct-btn acct-login";
    login.textContent = "🔑 登入";
    login.title = "用 Google 登入，讓居民記得你（記憶/好感度綁帳號）";
    login.addEventListener("click", (e) => {
      if (e && e.preventDefault) e.preventDefault();
      try { location.href = "/auth/google/start"; } catch (_) { /* 測試 DOM 無 location */ }
    });
    acctEl.appendChild(login);
  }
}

// 開頁查 /auth/me：已登入就點亮帳號 chip、記下帳號名（進場時帶上、伺服器仍以 cookie 為準）。
// OAuth 未設定／未登入回非 2xx → 維持訪客態（照常逛）。fetch 不可用（沙箱）就跳過。
renderAccountChip(); // 先以訪客態畫一次（/auth/me 命中再重畫）
if (typeof fetch === "function") {
  fetch("/auth/me", { credentials: "same-origin" })
    .then((r) => (r && r.ok ? r.json() : null))
    .then((me) => {
      if (me && me.id) {
        isLoggedIn = true;
        if (me.name) myAccountName = me.name;
        renderAccountChip();
      }
    })
    .catch(() => { /* 查不到就當訪客，不影響逛 */ });
}

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
// 梯子攀爬速度（方塊/秒）；比走路略慢，強調「謹慎攀降」感。
const CLIMB_SPEED = 3.5;
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

  // ── 梯子攀爬 v1（ROADMAP 688）：進入梯子方格後取消重力、Space/跳鈕上爬、S/搖桿下降 ──
  const climbing = aabbHitsLadder(player.x, player.y, player.z, getRaw, PW, PH);
  if (climbing) {
    player.vy = 0;       // 取消重力累積
    player.grounded = false;
    // 上爬：Space（桌機）或搖桿向上（-joyVec.y > 0.2；y 軸向上為負）
    const climbUp = keys["Space"] || (-joyVec.y > 0.2);
    // 下降：S 鍵（桌機）或搖桿向下（joyVec.y > 0.2）
    const climbDown = keys["KeyS"] || keys["ArrowDown"] || (joyVec.y > 0.2);
    if (climbUp)        player.y += CLIMB_SPEED * dt;
    else if (climbDown) player.y -= CLIMB_SPEED * dt;
    // 水平仍可移動（側步可脫離梯子）
  } else {
    if ((keys["Space"]) && player.grounded) tryJump();
  }

  const dir = new THREE.Vector3();
  dir.addScaledVector(fwd, mz).addScaledVector(right, mx);
  if (dir.lengthSq() > 1e-4) {
    dir.normalize();
    moveAxis("x", dir.x * SPEED * dt);
    moveAxis("z", dir.z * SPEED * dt);
  }

  if (!climbing) {
    // 重力 + 垂直碰撞（只在非攀爬模式下套用）
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
  }

  // 脫困保險（每幀）：若這幀結束後仍與實心方塊重疊（最常見：新 chunk 載入把人埋住、
  // 出生瞬間、走進未載入區後補載），把人頂出方塊外，避免永久卡死。沒卡就零成本早退。
  unstuckIfNeeded();

  // 踏階視覺補間衰減（frame-rate 無關的指數平滑）
  // stepSmooth > 0 → 視覺 Y 低於物理 Y；每幀靠近直到 < 0.005 格就吸附歸零。
  // 重力下落時 stepSmooth 保持 0，不影響往下的動態。
  if (stepSmooth > 0) {
    stepSmooth *= Math.exp(-STEP_SMOOTH_K * dt);
    if (stepSmooth < 0.005) stepSmooth = 0;
  }
  // visualY：bodyMesh 與鏡頭看向點用此值——踏階時從原地平滑升上去，消除瞬跳閃爍。
  const visualY = player.y - stepSmooth;

  // 玩家身體 + 朝向（用 visualY 避免角色瞬跳一格）。第一人稱藏自己身體（相機在眼睛裡）。
  bodyMesh.visible = (viewMode !== "first");
  bodyMesh.position.set(player.x, visualY + PH / 2, player.z);
  if (dir.lengthSq() > 1e-4) bodyMesh.rotation.y = Math.atan2(dir.x, dir.z);

  // embodied 靠近說話 v1：自己頭上的對話泡泡跟隨角色 + 倒數消失（話活在世界裡）。
  if (myBubbleTimer > 0) {
    myBubbleTimer -= dt;
    if (myBubbleTimer <= 0) { myBubble.visible = false; myBubbleText = ""; }
  }
  if (myBubble.visible) myBubble.position.set(player.x, visualY + PH + 0.85, player.z);

  if (viewMode === "first") {
    // 第一人稱：相機在眼睛高度，朝視線方向（yaw+pitch）看出去。
    const eye = new THREE.Vector3(player.x, visualY + 1.5, player.z);
    camera.position.copy(eye);
    const d = viewDir();
    camera.lookAt(eye.x + d.x, eye.y + d.y, eye.z + d.z);
  } else {
    // 第三人稱鏡頭跟隨（用 visualY 讓鏡頭也跟著平滑升，不突然跳）
    const lookTarget = new THREE.Vector3(player.x, visualY + 1.3, player.z);
    const dist = 6.0, cp = Math.cos(camPitch), sp = Math.sin(camPitch);
    camera.position.set(
      lookTarget.x + Math.sin(player.yaw) * dist * cp,
      lookTarget.y + dist * sp,
      lookTarget.z + Math.cos(player.yaw) * dist * cp
    );
    camera.lookAt(lookTarget);
  }

  // 準心對準的方塊（破壞/放置目標）+ 高亮外框。
  updateTarget();

  // 採礦手感 v1（ROADMAP 687）：桌機按住左鍵期間每幀推進挖掘進度。
  if (!isTouch) tickMining(dt);

  // 火把發光 v1（ROADMAP 691）：手持火把時在鏡頭附近亮暖橘光。
  const holdingTorch = selectedBlock() === TORCH && (myInv.get(TORCH) || 0) > 0;
  if (holdingTorch) {
    heldTorchLight.position.copy(camera.position);
    heldTorchLight.intensity = 1.8;
    heldTorchLight.visible = true;
  } else {
    heldTorchLight.intensity = 0;
    heldTorchLight.visible = false;
  }
  // 每 2 秒刷新近旁放置火把的光源池位置（非每幀掃，節省 CPU）。
  _torchRefreshTimer -= dt;
  if (_torchRefreshTimer <= 0) {
    _torchRefreshTimer = 2.0;
    updateNearbyTorchLights();
  }

  // 水體視覺升級 v1：累積 time uniform 驅動水面波動 shader。
  waterTime.value += dt;

  // 水下氛圍：相機所在方塊是否為水（每幀一次 getRaw 查詢，成本極低）。
  updateUnderwaterAtmosphere();

  updateRain(dt);
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
      // 後端版本：直式/手機友善，commit 與 build 時間各自一行。後端離線/未起時顯示「?」。
      const beCommit = backendVersion ? backendVersion.commit : "?";
      const beBuilt = backendVersion ? (backendVersion.built_at || "?") : "?";
      dbgEl.textContent =
        `FPS ${fps.toFixed(0)}\n` +
        `chunks ${chunks.size}  meshes ${meshes.size}\n` +
        `pos ${player.x.toFixed(1)},${player.y.toFixed(1)},${player.z.toFixed(1)}\n` +
        `grounded ${player.grounded}\n` +
        `build ${window.__BUILD__ || "?"}\n` +
        `後端 ${beCommit}\n` +
        `built ${beBuilt}`;
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

// ── 背包 + 2×2 合成格 v1（ROADMAP 664）──────────────────────────────────────
// 前端配方表（對齊後端 voxel_craft::RECIPES，id/inputs/output 穩定契約）。
// 無順序合成（shapeless）：格子裡只要湊齊材料種類+數量即可，位置不限。
const RECIPES_JS = [
  { id: "plank",        name: "木板",   inputs: [[WOOD, 2]],  output_block: PLANK,       out_count: 4 },
  { id: "stone_brick",  name: "石磚",   inputs: [[STONE, 2]], output_block: STONE_BRICK, out_count: 2 },
  { id: "glass",        name: "玻璃",   inputs: [[SAND, 2]],  output_block: GLASS,       out_count: 1 },
  { id: "till",         name: "農田土", inputs: [[DIRT, 2]],  output_block: FARM_SOIL,   out_count: 2 },
  // 工作台 v1（ROADMAP 665）：4 木板 → 1 工作台（放滿 2×2 四格）。
  // 先前缺這條 → 玩家放 4 木板卻合不出工作台（前端配方表和後端 voxel_craft 脫節）。
  { id: "workbench",    name: "工作台", inputs: [[PLANK, 4]], output_block: WORKBENCH,   out_count: 1 },
  // 麵包 v1（ROADMAP 668）：3 小麥顆粒 → 1 麵包
  { id: "bread",        name: "麵包",   inputs: [[WHEAT, 3]], output_block: BREAD,       out_count: 1 },
  // 火把 v1（ROADMAP 685）：1 木頭 + 1 煤礦 → 4 火把
  { id: "torch",        name: "火把",   inputs: [[WOOD, 1], [COAL_ORE, 1]], output_block: TORCH, out_count: 4 },
  // 鎬具 v1（ROADMAP 687）：採石/採礦手感加速；剛好 2×2 四格
  { id: "wood_pickaxe",  name: "木鎬", inputs: [[WOOD, 3], [PLANK, 1]],  output_block: PICKAXE_WOOD,  out_count: 1 },
  { id: "stone_pickaxe", name: "石鎬", inputs: [[STONE, 3], [PLANK, 1]], output_block: PICKAXE_STONE, out_count: 1 },
  // 梯子 v1（ROADMAP 688）：3 木板 → 3 梯子（垂直攀爬，深礦上下自如）
  { id: "ladder", name: "梯子", inputs: [[PLANK, 3]], output_block: LADDER, out_count: 3 },
  // 斧頭 v1（ROADMAP 689）：砍木加速，和鎬具互補的工具線；剛好 2×2 四格
  { id: "wood_axe",  name: "木斧", inputs: [[WOOD, 3], [PLANK, 1]],  output_block: AXE_WOOD,  out_count: 1 },
  { id: "stone_axe", name: "石斧", inputs: [[STONE, 3], [PLANK, 1]], output_block: AXE_STONE, out_count: 1 },
  // 鏟子 v1（ROADMAP 690）：挖土/沙/草地加速，完成採集三件套（鎬+斧+鏟）
  { id: "wood_shovel",  name: "木鏟", inputs: [[WOOD, 1], [PLANK, 1]],  output_block: SHOVEL_WOOD,  out_count: 1 },
  { id: "stone_shovel", name: "石鏟", inputs: [[STONE, 1], [PLANK, 1]], output_block: SHOVEL_STONE, out_count: 1 },
  // 木門 v1（ROADMAP 693）：4 木板 → 2 門（填滿 2×2 格）
  { id: "door", name: "木門", inputs: [[PLANK, 4]], output_block: DOOR_CLOSED, out_count: 2 },
  // 床 v1：3 木板 + 3 葉片（當被褥）→ 1 床
  { id: "bed", name: "床", inputs: [[PLANK, 3], [LEAVES, 3]], output_block: BED, out_count: 1 },
];

// ── 背包面板狀態 ──────────────────────────────────────────────────────────────
// bagGrid[0..3]：2×2 格子，0 代表空格，非零代表 block_id。
const bagGrid = [0, 0, 0, 0];
// 目前被「拿起」的 block_id（0 = 沒拿任何東西）。
let bagPick = 0;

const bagPanelEl = document.getElementById("bagPanel");
const bagInvGridEl = document.getElementById("bagInvGrid");
const bagGrid2x2El = document.getElementById("bagGrid2x2");
const bagResultEl  = document.getElementById("bagResultSlot");

function openBagPanel() {
  if (!bagPanelEl) return;
  releaseMouse(); // 桌機：開面板要放開滑鼠鎖定，游標才能點格子
  bagPanelEl.style.display = "flex";
  renderBagPanel();
}
function closeBagPanel() {
  if (!bagPanelEl) return;
  bagPanelEl.style.display = "none";
  bagPick = 0; // 關面板時清除選取
}
function bagPanelVisible() {
  return bagPanelEl ? bagPanelEl.style.display === "flex" : false;
}

/**
 * matchBagRecipe——無順序配方比對（純函式，確定性）。
 * 統計格子裡的 block_id 出現次數，比對 RECIPES_JS，回傳 {recipe, canCraft} 或 null。
 * canCraft = 玩家實際背包材料足夠（格子放入是「預覽意圖」，不實際扣除）。
 */
function matchBagRecipe() {
  const gridCounts = new Map();
  for (const bid of bagGrid) {
    if (bid !== 0) gridCounts.set(bid, (gridCounts.get(bid) || 0) + 1);
  }
  if (gridCounts.size === 0) return null;
  for (const r of RECIPES_JS) {
    const needed = new Map(r.inputs.map(([b, c]) => [b, c]));
    if (needed.size !== gridCounts.size) continue;
    let match = true;
    for (const [b, c] of needed) {
      if ((gridCounts.get(b) || 0) !== c) { match = false; break; }
    }
    if (!match) continue;
    const canCraft = r.inputs.every(([b, c]) => (myInv.get(b) || 0) >= c);
    return { recipe: r, canCraft };
  }
  return null;
}

/** 顏色方塊 DOM（inline background swatch）。 */
function makeSwatchEl(blockId, cls) {
  const el = document.createElement("div");
  el.className = cls;
  const c = COLOR[blockId] || COLOR[STONE];
  el.style.background = `rgb(${(c[0]*255)|0},${(c[1]*255)|0},${(c[2]*255)|0})`;
  return el;
}

/** 渲染物品欄區域——列出背包內所有有數量的方塊，可點選「拿起」。 */
function renderBagInvGrid() {
  if (!bagInvGridEl) return;
  bagInvGridEl.innerHTML = "";
  const items = [...myInv.entries()].filter(([, cnt]) => cnt > 0);
  if (items.length === 0) {
    const emp = document.createElement("div");
    emp.style.cssText = "color:#506070;font-size:12px;font-style:italic;padding:6px 0";
    emp.textContent = "背包是空的，去挖一些方塊吧";
    bagInvGridEl.appendChild(emp);
    return;
  }
  items.forEach(([bid, cnt]) => {
    const slot = document.createElement("div");
    slot.className = "bag-inv-slot" + (bagPick === bid ? " picked" : "");
    slot.appendChild(makeSwatchEl(bid, "bag-inv-sw"));
    const name = document.createElement("div");
    name.className = "bag-inv-name";
    name.textContent = BLOCK_NAME[bid] || "?";
    const cntEl = document.createElement("div");
    cntEl.className = "bag-inv-cnt";
    cntEl.textContent = "×" + cnt;
    slot.appendChild(name); slot.appendChild(cntEl);
    slot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      if (bagPick === bid) {
        bagPick = 0; // 再點同一格 → 放下（取消選取），不動快捷欄
      } else {
        bagPick = bid;
        // 麥塊互動：點背包物品 → 指派進當前選中的快捷欄格（也可再點某格改放）。
        assignToHotbar(selectedSlot, bid);
      }
      renderBagPanel();
    });
    bagInvGridEl.appendChild(slot);
  });
}

/** 渲染 2×2 合成格 + 結果格。 */
function renderBagCraftArea() {
  if (!bagGrid2x2El || !bagResultEl) return;
  bagGrid2x2El.innerHTML = "";
  for (let i = 0; i < 4; i++) {
    const bid = bagGrid[i];
    const slot = document.createElement("div");
    slot.className = "bag-grid-slot" + (bid !== 0 ? " filled" : "");
    if (bid !== 0) {
      slot.appendChild(makeSwatchEl(bid, "bag-grid-sw"));
      const lbl = document.createElement("div");
      lbl.className = "bag-grid-lbl";
      lbl.textContent = BLOCK_NAME[bid] || "?";
      slot.appendChild(lbl);
    }
    slot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      if (bagPick !== 0 && bid === 0) {
        // 拿著東西 + 格子空 → 放入
        bagGrid[i] = bagPick;
        bagPick = 0;
      } else if (bagPick !== 0 && bid !== 0) {
        // 拿著東西 + 格子已有 → 交換
        bagGrid[i] = bagPick;
        bagPick = bid;
      } else if (bagPick === 0 && bid !== 0) {
        // 沒拿東西 + 格子有東西 → 拿起（格子清空）
        bagPick = bid;
        bagGrid[i] = 0;
      }
      renderBagPanel();
    });
    bagGrid2x2El.appendChild(slot);
  }
  // 結果格
  const match = matchBagRecipe();
  bagResultEl.className = ""; // 重設 class
  bagResultEl.innerHTML = "";
  if (match) {
    const r = match.recipe;
    bagResultEl.classList.add(match.canCraft ? "has-result" : "no-material");
    bagResultEl.appendChild(makeSwatchEl(r.output_block, "bag-res-sw"));
    const cnt = document.createElement("div");
    cnt.className = "bag-res-cnt";
    cnt.textContent = "×" + r.out_count;
    const name = document.createElement("div");
    name.className = "bag-res-name";
    name.textContent = r.name;
    bagResultEl.appendChild(cnt); bagResultEl.appendChild(name);
    if (!match.canCraft) {
      const warn = document.createElement("div");
      warn.style.cssText = "font-size:9px;color:#ff8060;margin-top:2px";
      warn.textContent = "材料不足";
      bagResultEl.appendChild(warn);
    }
  } else {
    // 沒有配方吻合：不洩題（維護者「合成應該自己摸索」）——只給「還沒湊對」的模糊訊號，
    // 讓玩家知道格子有反應、不是壞掉，但確切配方留給自己試 / 問居民（古代智慧口耳相傳）。
    const hasItems = bagGrid.some((b) => b !== 0);
    if (hasItems) {
      const hint = document.createElement("div");
      hint.style.cssText = "font-size:10px;color:#c0b090;text-align:center;line-height:1.3";
      hint.textContent = "……還湊不出什麼。";
      bagResultEl.appendChild(hint);
    }
  }
}

/** 渲染整個背包面板（物品欄 + 合成格）。 */
function renderBagPanel() {
  renderBagInvGrid();
  renderBagCraftArea();
}

// 結果格點擊：送出合成請求。
if (bagResultEl) bagResultEl.addEventListener("pointerdown", (e) => {
  e.stopPropagation();
  const match = matchBagRecipe();
  if (!match || !match.canCraft || !wsReady) return;
  ws.send(JSON.stringify({ t: "craft", recipe_id: match.recipe.id }));
});

// 清除合成格按鈕。
const bagClearBtnEl = document.getElementById("bagClearBtn");
if (bagClearBtnEl) bagClearBtnEl.addEventListener("pointerdown", (e) => {
  e.stopPropagation();
  bagGrid.fill(0);
  bagPick = 0;
  renderBagPanel();
});

// 視角切換鈕（👁）：第一/三人稱互換（等同 F5）。初始文字依預設模式設定。
const viewBtnEl = document.getElementById("viewBtn");
if (viewBtnEl) {
  viewBtnEl.textContent = (viewMode === "first") ? "👁 第一人稱" : "🧍 第三人稱";
  viewBtnEl.addEventListener("click", (e) => { toggleViewMode(); e.stopPropagation(); });
}

// 背包按鈕（🎒）開關面板。
const bagBtnEl = document.getElementById("bagBtn");
if (bagBtnEl) bagBtnEl.addEventListener("click", (e) => {
  if (bagPanelVisible()) { closeBagPanel(); } else { openBagPanel(); }
  e.stopPropagation();
});
// 點面板外關閉。
document.addEventListener("pointerdown", (e) => {
  if (bagPanelVisible()) {
    if (!bagPanelEl.contains(e.target) && e.target !== bagBtnEl) closeBagPanel();
  }
});
// 關閉鈕（✕）。
const bagCloseEl = document.getElementById("bagClose");
if (bagCloseEl) bagCloseEl.addEventListener("click", closeBagPanel);

// ── 工作台 3×3 合成面板 v1（ROADMAP 665）─────────────────────────────────────
// 工作台配方表（對齊後端 voxel_craft::WORKBENCH_RECIPES）。
// 需要 5-6 個格子，超出 2×2，必須在工作台才能完成。
const WORKBENCH_RECIPES_JS = [
  { id: "plank_wb",       name: "木板（大量）",   inputs: [[WOOD, 6]],                output_block: PLANK,       out_count: 16 },
  { id: "stone_brick_wb", name: "石磚（大量）",   inputs: [[STONE, 6]],               output_block: STONE_BRICK, out_count: 10 },
  { id: "glass_wb",       name: "玻璃（大量）",   inputs: [[SAND, 6]],                output_block: GLASS,       out_count: 8  },
  { id: "stone_wood_mix", name: "混合石磚",       inputs: [[STONE, 3], [PLANK, 3]],   output_block: STONE_BRICK, out_count: 6  },
  { id: "farm_kit",       name: "農耕大包",       inputs: [[DIRT, 4], [WOOD, 2]],     output_block: FARM_SOIL,   out_count: 8  },
  { id: "iron_block",     name: "鐵磚",           inputs: [[IRON_INGOT, 6]],           output_block: IRON_BLOCK,  out_count: 2  },
  // 鐵鎬（ROADMAP 687）：3 鐵錠 + 2 木板 → 1 鐵鎬（5 格，需工作台）
  { id: "iron_pickaxe",   name: "鐵鎬",           inputs: [[IRON_INGOT, 3], [PLANK, 2]], output_block: PICKAXE_IRON, out_count: 1  },
  // 鐵斧（ROADMAP 689）：3 鐵錠 + 2 木板 → 1 鐵斧（5 格，需工作台；砍木材 6×）
  { id: "iron_axe",       name: "鐵斧",           inputs: [[IRON_INGOT, 3], [PLANK, 2]], output_block: AXE_IRON,     out_count: 1  },
  // 鐵鏟（ROADMAP 690）：2 鐵錠 + 3 木板 → 1 鐵鏟（5 格，需工作台；挖軟土 6×）
  { id: "iron_shovel",    name: "鐵鏟",           inputs: [[IRON_INGOT, 2], [PLANK, 3]], output_block: SHOVEL_IRON,  out_count: 1  },
  // 箱子 v1（ROADMAP 692）：8 木板 → 1 箱子（8 格，需工作台；放置後儲物）
  { id: "chest",          name: "箱子",           inputs: [[PLANK, 8]],                  output_block: CHEST,        out_count: 1  },
];

// wbGrid[0..8]：3×3 共 9 格，0 代表空格，非零代表 block_id。
const wbGrid = new Array(9).fill(0);
// 目前被「拿起」的 block_id（0 = 沒拿）。
let wbPick = 0;

const wbPanelEl  = document.getElementById("wbPanel");
const wbInvGridEl = document.getElementById("wbInvGrid");
const wbGrid3x3El = document.getElementById("wbGrid3x3");
const wbResultEl  = document.getElementById("wbResultSlot");
const wbBtnEl     = document.getElementById("wbBtn");

function openWbPanel() {
  if (!wbPanelEl) return;
  releaseMouse(); // 桌機：開面板要放開滑鼠鎖定，游標才能拖放材料
  wbPanelEl.style.display = "flex";
  renderWbPanel();
}
function closeWbPanel() {
  if (!wbPanelEl) return;
  wbPanelEl.style.display = "none";
  wbPick = 0;
}
function wbPanelVisible() {
  return wbPanelEl ? wbPanelEl.style.display === "flex" : false;
}

/**
 * matchWbRecipe——無順序配方比對（純函式，確定性）。
 * 統計 wbGrid 裡的材料次數，比對 WORKBENCH_RECIPES_JS，回傳 {recipe, canCraft} 或 null。
 */
function matchWbRecipe() {
  const gridCounts = new Map();
  for (const bid of wbGrid) {
    if (bid !== 0) gridCounts.set(bid, (gridCounts.get(bid) || 0) + 1);
  }
  if (gridCounts.size === 0) return null;
  for (const r of WORKBENCH_RECIPES_JS) {
    const needed = new Map(r.inputs.map(([b, c]) => [b, c]));
    if (needed.size !== gridCounts.size) continue;
    let match = true;
    for (const [b, c] of needed) {
      if ((gridCounts.get(b) || 0) !== c) { match = false; break; }
    }
    if (!match) continue;
    const canCraft = r.inputs.every(([b, c]) => (myInv.get(b) || 0) >= c);
    return { recipe: r, canCraft };
  }
  return null;
}

/** 渲染工作台物品欄（共用 bag-inv-* CSS）。 */
function renderWbInvGrid() {
  if (!wbInvGridEl) return;
  wbInvGridEl.innerHTML = "";
  const items = [...myInv.entries()].filter(([, cnt]) => cnt > 0);
  if (items.length === 0) {
    const emp = document.createElement("div");
    emp.style.cssText = "color:#605040;font-size:12px;font-style:italic;padding:6px 0";
    emp.textContent = "背包是空的，去挖一些方塊吧";
    wbInvGridEl.appendChild(emp);
    return;
  }
  for (const [bid, cnt] of items) {
    const slot = document.createElement("div");
    slot.className = "bag-inv-slot" + (wbPick === bid ? " picked" : "");
    slot.appendChild(makeSwatchEl(bid, "bag-inv-sw"));
    const name = document.createElement("div");
    name.className = "bag-inv-name";
    name.textContent = BLOCK_NAME[bid] || "?";
    const cntEl = document.createElement("div");
    cntEl.className = "bag-inv-cnt";
    cntEl.textContent = "×" + cnt;
    slot.appendChild(name); slot.appendChild(cntEl);
    slot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      wbPick = (wbPick === bid) ? 0 : bid;
      renderWbPanel();
    });
    wbInvGridEl.appendChild(slot);
  }
}

/** 渲染 3×3 合成格 + 結果格。 */
function renderWbCraftArea() {
  if (!wbGrid3x3El || !wbResultEl) return;
  wbGrid3x3El.innerHTML = "";
  for (let i = 0; i < 9; i++) {
    const bid = wbGrid[i];
    const slot = document.createElement("div");
    slot.className = "wb-grid-slot" + (bid !== 0 ? " filled" : "");
    if (bid !== 0) {
      slot.appendChild(makeSwatchEl(bid, "wb-grid-sw"));
      const lbl = document.createElement("div");
      lbl.className = "wb-grid-lbl";
      lbl.textContent = BLOCK_NAME[bid] || "?";
      slot.appendChild(lbl);
    }
    slot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      if (wbPick !== 0 && bid === 0) {
        wbGrid[i] = wbPick; wbPick = 0;
      } else if (wbPick !== 0 && bid !== 0) {
        wbGrid[i] = wbPick; wbPick = bid;
      } else if (wbPick === 0 && bid !== 0) {
        wbPick = bid; wbGrid[i] = 0;
      }
      renderWbPanel();
    });
    wbGrid3x3El.appendChild(slot);
  }
  // 結果格
  wbResultEl.className = "";
  wbResultEl.innerHTML = "";
  const match = matchWbRecipe();
  if (match) {
    const r = match.recipe;
    wbResultEl.classList.add(match.canCraft ? "has-result" : "no-material");
    wbResultEl.appendChild(makeSwatchEl(r.output_block, "bag-res-sw"));
    const nm = document.createElement("div"); nm.className = "bag-res-name"; nm.textContent = r.name;
    const ct = document.createElement("div"); ct.className = "bag-res-cnt"; ct.textContent = "×" + r.out_count;
    wbResultEl.appendChild(nm); wbResultEl.appendChild(ct);
    if (!match.canCraft) {
      const warn = document.createElement("div");
      warn.style.cssText = "font-size:9px;color:#ff8060;margin-top:2px";
      warn.textContent = "材料不足";
      wbResultEl.appendChild(warn);
    }
  }
}

/** 渲染整個工作台面板（物品欄 + 合成格）。 */
function renderWbPanel() {
  renderWbInvGrid();
  renderWbCraftArea();
}

// 結果格點擊：送出工作台合成請求。
if (wbResultEl) wbResultEl.addEventListener("pointerdown", (e) => {
  e.stopPropagation();
  const match = matchWbRecipe();
  if (!match || !match.canCraft || !wsReady) return;
  ws.send(JSON.stringify({ t: "craft", recipe_id: match.recipe.id }));
});

// 清除合成格。
const wbClearBtnEl = document.getElementById("wbClearBtn");
if (wbClearBtnEl) wbClearBtnEl.addEventListener("pointerdown", (e) => {
  e.stopPropagation();
  wbGrid.fill(0); wbPick = 0;
  renderWbPanel();
});

// 工作台按鈕（🔨）點擊開啟面板（備用：也可以右鍵對準工作台方塊開啟）。
if (wbBtnEl) wbBtnEl.addEventListener("click", (e) => {
  if (wbPanelVisible()) { closeWbPanel(); } else { openWbPanel(); }
  e.stopPropagation();
});

// 關閉鈕（✕）。
const wbCloseEl = document.getElementById("wbClose");
if (wbCloseEl) wbCloseEl.addEventListener("click", closeWbPanel);

// 點面板外關閉。
document.addEventListener("pointerdown", (e) => {
  if (wbPanelVisible()) {
    if (wbPanelEl && !wbPanelEl.contains(e.target) && e.target !== wbBtnEl) closeWbPanel();
  }
});

// ── inv_update / inv_sync 後也刷新工作台面板（若已開啟）────────────────────
// 注意：inv_sync 和 inv_update handler 在 WS onmessage 裡，
//       craft_ok / craft_fail 的刷新邏輯已在 WS handler 那段處理過。

// ── 熔爐面板 v2（ROADMAP 712）──────────────────────────────────────────────────
// 改成跟背包 2×2 / 工作台 3×3 一致的「拿起→放入格子」格子式互動（原 v1 是按鈕清單，
// 跟另外兩層合成介面手感不一致；格數取 3——熔爐配方裡最耗格的 smelt_stone
// 需要「同一種材料×3」湊數量，比清單更貼近玩家已經上手的操作邏輯）。
const FURNACE_RECIPES_JS = [
  { id: "smelt_stone", name: "拋光石",       inputs: [[STONE, 3]],               output_block: SMOOTH_STONE, out_count: 3 },
  { id: "smelt_glass", name: "玻璃（冶煉）", inputs: [[SAND,  2]],               output_block: GLASS,        out_count: 3 },
  { id: "smelt_brick", name: "石磚（冶煉）", inputs: [[STONE, 2]],               output_block: STONE_BRICK,  out_count: 4 },
  // 鐵錠 v1（ROADMAP 683）：1 鐵礦 + 1 煤礦（燃料）→ 2 鐵錠
  { id: "smelt_iron",  name: "鐵錠",         inputs: [[IRON_ORE, 1], [COAL_ORE, 1]], output_block: IRON_INGOT, out_count: 2 },
];

// furnaceGrid[0..2]：3 格輸入，0 代表空格，非零代表 block_id。
// 3 格而非 2 格：smelt_stone 單一配方就需要 3 顆石頭（同一材料佔滿 3 格才湊得出數量），
// 熔爐所有配方裡最多材料格數的就是它，故取 3 為格數上限（沿用背包/工作台「格數=最大配方所需」的設計）。
const furnaceGrid = [0, 0, 0];
// 目前被「拿起」的 block_id（0 = 沒拿）。
let furnacePick = 0;

const furnacePanelEl   = document.getElementById("furnacePanel");
const furnaceBtnEl     = document.getElementById("furnaceBtn");
const furnaceInvGridEl = document.getElementById("furnaceInvGrid");
const furnaceGridEl    = document.getElementById("furnaceGrid2");
const furnaceResultEl  = document.getElementById("furnaceResultSlot");

function openFurnacePanel() {
  if (!furnacePanelEl) return;
  releaseMouse(); // 桌機：開面板要放開滑鼠鎖定，游標才能點冶煉
  furnacePanelEl.style.display = "flex";
  renderFurnacePanel();
}
function closeFurnacePanel() {
  if (!furnacePanelEl) return;
  furnacePanelEl.style.display = "none";
  furnacePick = 0; // 關面板時清除選取
}
function furnacePanelVisible() {
  return furnacePanelEl ? furnacePanelEl.style.display === "flex" : false;
}

/**
 * matchFurnaceRecipe——無順序配方比對（純函式，確定性，與 matchBagRecipe/matchWbRecipe 同手法）。
 * 統計 furnaceGrid 裡的材料次數，比對 FURNACE_RECIPES_JS，回傳 {recipe, canCraft} 或 null。
 */
function matchFurnaceRecipe() {
  const gridCounts = new Map();
  for (const bid of furnaceGrid) {
    if (bid !== 0) gridCounts.set(bid, (gridCounts.get(bid) || 0) + 1);
  }
  if (gridCounts.size === 0) return null;
  for (const r of FURNACE_RECIPES_JS) {
    const needed = new Map(r.inputs.map(([b, c]) => [b, c]));
    if (needed.size !== gridCounts.size) continue;
    let match = true;
    for (const [b, c] of needed) {
      if ((gridCounts.get(b) || 0) !== c) { match = false; break; }
    }
    if (!match) continue;
    const canCraft = r.inputs.every(([b, c]) => (myInv.get(b) || 0) >= c);
    return { recipe: r, canCraft };
  }
  return null;
}

/** 渲染熔爐物品欄（點選拿起，與背包/工作台一致）。 */
function renderFurnaceInvGrid() {
  if (!furnaceInvGridEl) return;
  furnaceInvGridEl.innerHTML = "";
  const items = [...myInv.entries()].filter(([, cnt]) => cnt > 0);
  if (items.length === 0) {
    const emp = document.createElement("div");
    emp.style.cssText = "color:#605040;font-size:12px;font-style:italic;padding:6px 0";
    emp.textContent = "背包是空的，去挖一些方塊吧";
    furnaceInvGridEl.appendChild(emp);
    return;
  }
  for (const [bid, cnt] of items) {
    const slot = document.createElement("div");
    slot.className = "bag-inv-slot" + (furnacePick === bid ? " picked" : "");
    slot.appendChild(makeSwatchEl(bid, "bag-inv-sw"));
    const name = document.createElement("div");
    name.className = "bag-inv-name";
    name.textContent = BLOCK_NAME[bid] || "?";
    const cntEl = document.createElement("div");
    cntEl.className = "bag-inv-cnt";
    cntEl.textContent = "×" + cnt;
    slot.appendChild(name); slot.appendChild(cntEl);
    slot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      furnacePick = (furnacePick === bid) ? 0 : bid;
      renderFurnacePanel();
    });
    furnaceInvGridEl.appendChild(slot);
  }
}

/** 渲染 3 格冶煉格 + 結果格。 */
function renderFurnaceCraftArea() {
  if (!furnaceGridEl || !furnaceResultEl) return;
  furnaceGridEl.innerHTML = "";
  for (let i = 0; i < 3; i++) {
    const bid = furnaceGrid[i];
    const slot = document.createElement("div");
    slot.className = "furnace-grid-slot" + (bid !== 0 ? " filled" : "");
    if (bid !== 0) {
      slot.appendChild(makeSwatchEl(bid, "furnace-grid-sw"));
      const lbl = document.createElement("div");
      lbl.className = "furnace-grid-lbl";
      lbl.textContent = BLOCK_NAME[bid] || "?";
      slot.appendChild(lbl);
    }
    slot.addEventListener("pointerdown", (e) => {
      e.stopPropagation();
      if (furnacePick !== 0 && bid === 0) {
        furnaceGrid[i] = furnacePick; furnacePick = 0;
      } else if (furnacePick !== 0 && bid !== 0) {
        furnaceGrid[i] = furnacePick; furnacePick = bid;
      } else if (furnacePick === 0 && bid !== 0) {
        furnacePick = bid; furnaceGrid[i] = 0;
      }
      renderFurnacePanel();
    });
    furnaceGridEl.appendChild(slot);
  }
  // 結果格
  furnaceResultEl.className = "";
  furnaceResultEl.innerHTML = "";
  const match = matchFurnaceRecipe();
  if (match) {
    const r = match.recipe;
    furnaceResultEl.classList.add(match.canCraft ? "has-result" : "no-material");
    furnaceResultEl.appendChild(makeSwatchEl(r.output_block, "bag-res-sw"));
    const nm = document.createElement("div"); nm.className = "bag-res-name"; nm.textContent = r.name;
    const ct = document.createElement("div"); ct.className = "bag-res-cnt"; ct.textContent = "×" + r.out_count;
    furnaceResultEl.appendChild(nm); furnaceResultEl.appendChild(ct);
    if (!match.canCraft) {
      const warn = document.createElement("div");
      warn.style.cssText = "font-size:9px;color:#ff8060;margin-top:2px";
      warn.textContent = "材料不足";
      furnaceResultEl.appendChild(warn);
    }
  }
}

/** 渲染整個熔爐面板（物品欄 + 冶煉格）。 */
function renderFurnacePanel() {
  renderFurnaceInvGrid();
  renderFurnaceCraftArea();
}

// 結果格點擊：送出冶煉請求。
if (furnaceResultEl) furnaceResultEl.addEventListener("pointerdown", (e) => {
  e.stopPropagation();
  const match = matchFurnaceRecipe();
  if (!match || !match.canCraft || !wsReady) return;
  ws.send(JSON.stringify({ t: "craft", recipe_id: match.recipe.id }));
});

// 清除冶煉格按鈕。
const furnaceClearBtnEl = document.getElementById("furnaceClearBtn");
if (furnaceClearBtnEl) furnaceClearBtnEl.addEventListener("pointerdown", (e) => {
  e.stopPropagation();
  furnaceGrid.fill(0);
  furnacePick = 0;
  renderFurnacePanel();
});

// 熔爐 HUD 按鈕（🔥）開閉面板。
if (furnaceBtnEl) furnaceBtnEl.addEventListener("click", (e) => {
  if (furnacePanelVisible()) { closeFurnacePanel(); } else { openFurnacePanel(); }
  e.stopPropagation();
});

// 關閉鈕（✕）。
const furnaceCloseEl = document.getElementById("furnaceClose");
if (furnaceCloseEl) furnaceCloseEl.addEventListener("click", closeFurnacePanel);

// 點面板外關閉。
document.addEventListener("pointerdown", (e) => {
  if (furnacePanelVisible()) {
    if (furnacePanelEl && !furnacePanelEl.contains(e.target) && e.target !== furnaceBtnEl) closeFurnacePanel();
  }
});

// ── 箱子面板 v1（ROADMAP 692）──────────────────────────────────────────────────
// 箱子世界座標（開啟中的箱子）；null = 沒有打開的箱子。
let _chestPos = null;
// 箱子當前內容（從 chest_view 伺服器訊息更新）：[{id, count}, ...]。
let _chestItems = [];

const chestPanelEl = document.getElementById("chestPanel");
const chestInvGridEl = document.getElementById("chestInvGrid");
const chestBoxGridEl = document.getElementById("chestBoxGrid");

function chestPanelVisible() {
  return chestPanelEl ? chestPanelEl.style.display === "flex" : false;
}

/** 開啟箱子面板：傳送 open_chest 請求，伺服器回 chest_view 後才真正渲染內容。 */
function openChestPanel(bx, by, bz) {
  if (!chestPanelEl) return;
  releaseMouse();
  _chestPos = { x: bx, y: by, z: bz };
  _chestItems = [];
  chestPanelEl.style.display = "flex";
  renderChestPanel();
  if (wsReady) ws.send(JSON.stringify({ t: "open_chest", x: bx, y: by, z: bz }));
}

function closeChestPanel() {
  if (!chestPanelEl) return;
  chestPanelEl.style.display = "none";
  _chestPos = null;
}

/**
 * 渲染箱子面板——分上下兩區：
 * - 上：箱子內容（_chestItems）；點某格→取出 1 個。
 * - 下：玩家背包（myInv）；點某格→放入 1 個（排除工具純物品）。
 */
function renderChestPanel() {
  if (!chestPanelEl) return;
  // 箱子內容區。
  if (chestBoxGridEl) {
    chestBoxGridEl.innerHTML = "";
    if (_chestItems.length === 0) {
      const empty = document.createElement("span");
      empty.className = "chest-empty-hint";
      empty.textContent = "（空箱子——從下方背包點格子存入物品）";
      chestBoxGridEl.appendChild(empty);
    } else {
      for (const { id, count } of _chestItems) {
        const sl = makeSwatchEl(id, "chest-box-slot");
        const cnt = document.createElement("span");
        cnt.className = "inv-count";
        cnt.textContent = "×" + count;
        sl.appendChild(cnt);
        sl.title = (BLOCK_NAME[id] || "方塊") + " ×" + count + "\n（點擊取出 1 個）";
        sl.addEventListener("click", () => {
          if (!_chestPos || !wsReady) return;
          ws.send(JSON.stringify({ t: "chest_take", x: _chestPos.x, y: _chestPos.y, z: _chestPos.z, item_id: id, count: 1 }));
        });
        chestBoxGridEl.appendChild(sl);
      }
    }
  }
  // 玩家背包區（只顯示可存入箱子的物品：排除種子 14、麵包 19 等純物品也可存，工具只存 item_id≤41）。
  if (chestInvGridEl) {
    chestInvGridEl.innerHTML = "";
    const EXCLUDE_FROM_CHEST = new Set([0]); // 只排 Air
    for (const [bid, cnt] of myInv) {
      if (EXCLUDE_FROM_CHEST.has(bid) || cnt <= 0) continue;
      const sl = makeSwatchEl(bid, "chest-inv-slot");
      const c = document.createElement("span");
      c.className = "inv-count";
      c.textContent = "×" + cnt;
      sl.appendChild(c);
      sl.title = (BLOCK_NAME[bid] || "物品") + " ×" + cnt + "\n（點擊存入 1 個）";
      sl.addEventListener("click", () => {
        if (!_chestPos || !wsReady) return;
        ws.send(JSON.stringify({ t: "chest_put", x: _chestPos.x, y: _chestPos.y, z: _chestPos.z, item_id: bid, count: 1 }));
      });
      chestInvGridEl.appendChild(sl);
    }
    if (chestInvGridEl.children.length === 0) {
      const hint = document.createElement("span");
      hint.className = "chest-empty-hint";
      hint.textContent = "（背包空的）";
      chestInvGridEl.appendChild(hint);
    }
  }
}

const chestCloseEl = document.getElementById("chestClose");
if (chestCloseEl) chestCloseEl.addEventListener("click", closeChestPanel);

// 點面板外關閉箱子面板。
document.addEventListener("pointerdown", (e) => {
  if (chestPanelVisible()) {
    if (chestPanelEl && !chestPanelEl.contains(e.target)) closeChestPanel();
  }
});

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
  // ── 視角模式 QA 用（滑鼠鎖定視角 + 第一/三人稱切換）──
  get viewMode() { return viewMode; },
  get camPitch() { return camPitch; },
  get pointerLocked() { return pointerLocked; },
  get bodyVisible() { return bodyMesh.visible; },
  toggleViewMode() { toggleViewMode(); return viewMode; },
  setCamPitch(p) { camPitch = p; clampPitch(); return camPitch; },
  setYaw(y) { player.yaw = y; return player.yaw; },
  get camPos() { return { x: camera.position.x, y: camera.position.y, z: camera.position.z }; },
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
  // ── embodied 靠近說話 v1 QA 用：範圍說話 + 讀自己頭上泡泡狀態 ──
  speak(text) { sendSpeak(text); return myBubbleText; },
  get myBubbleText() { return myBubble.visible ? myBubbleText : ""; },
  get myBubbleVisible() { return myBubble.visible; },
  // ── 日記 QA 用（ROADMAP 650）──
  openDiary(rid) { return openDiary(rid); },
  closeDiary() { closeDiary(); },
  get diaryVisible() { return diaryEl ? diaryEl.style.display !== "none" : false; },
  // ── 日記牆 QA 用（ROADMAP 674）──
  openDiaryWall() { return openDiaryWall(); },
  closeDiaryWall() { closeDiaryWall(); },
  get diaryWallVisible() { return diaryWallVisible; },
  renderDiaryWall(p) { renderDiaryWall(p); return diaryWallBodyEl && diaryWallBodyEl.innerHTML; },
  // ── 動態 Feed QA 用（ROADMAP 655）──
  openFeed() { return openFeed(); },
  closeFeed() { closeFeed(); },
  get feedVisible() { return feedVisible; },
  renderFeed(ev) { renderFeed(ev); return feedBodyEl && feedBodyEl.innerHTML; },
  // ── 居民羅盤 QA 用（ROADMAP 705）──
  openCompass() { return openCompass(); },
  closeCompass() { closeCompass(); },
  get compassVisible() { return compassVisible; },
  renderCompassPanel() { renderCompassPanel(); return compassBodyEl && compassBodyEl.innerHTML; },
  worldBearing(px, pz, rx, rz) { return worldBearing(px, pz, rx, rz); },
  compassRelativeDeg(px, pz, rx, rz, yaw) { return compassRelativeDeg(px, pz, rx, rz, yaw); },
  // ── 居民交情網 QA 用（ROADMAP 708）──
  openRelations() { return openRelations(); },
  closeRelations() { closeRelations(); },
  get relationsVisible() { return relationsVisible; },
  refreshRelations() { return refreshRelations(); },
  renderRelationsPanel(rows) { renderRelationsPanel(rows); return relationsBodyEl && relationsBodyEl.innerHTML; },
  sortRelationRows(rows) { return sortRelationRows(rows); },
  // ── 居民技能簿 QA 用（ROADMAP 719）──
  openSkills() { return openSkills(); },
  closeSkills() { closeSkills(); },
  get skillsVisible() { return skillsVisible; },
  refreshSkills() { return refreshSkills(); },
  renderSkillsPanel(rows) { renderSkillsPanel(rows); return skillsBodyEl && skillsBodyEl.innerHTML; },
  // ── 好感度 QA 用（ROADMAP 656）──
  affinityEmoji(count) { return affinityEmoji(count); },
  get myAffinity() { return Object.fromEntries(myAffinity); },
  refreshAffinity() { return refreshAffinity(); },
  // ── 採集背包 QA 用（ROADMAP 657）──
  get myInv() { return Object.fromEntries(myInv); },
  setInvForTest(bid, cnt) { if (cnt > 0) myInv.set(bid, cnt); else myInv.delete(bid); updateInvHud(); },
  updateInvHud() { updateInvHud(); },
  // ── 背包合成格 QA 用（ROADMAP 664）──
  get bagPanelVisible() { return bagPanelVisible(); },
  openBagPanel() { openBagPanel(); },
  closeBagPanel() { closeBagPanel(); },
  renderBagPanel() { renderBagPanel(); },
  get RECIPES_JS() { return RECIPES_JS; },
  get bagGrid() { return [...bagGrid]; },
  get bagPick() { return bagPick; },
  matchBagRecipe() { return matchBagRecipe(); },
  setBagGrid(slots) { slots.forEach((v, i) => { if (i < 4) bagGrid[i] = v; }); renderBagPanel(); },
  PLANK, STONE_BRICK, GLASS,
  // 種田 v1 常數 + QA 介面
  FARM_SOIL, FARM_SOIL_SEEDED, WHEAT_MATURE, SEEDS,
  // 第二種作物 v1 常數 QA 用
  CARROT_SEEDED, CARROT_MATURE, CARROT_SEEDS, CARROT,
  // 第三種作物 v1 常數 QA 用
  POTATO_SEEDED, POTATO_MATURE, POTATO_SEEDS, POTATO,
  // ── 工作台 3×3 QA 用（ROADMAP 665）──
  WORKBENCH,
  get wbPanelVisible() { return wbPanelVisible(); },
  openWbPanel() { openWbPanel(); },
  closeWbPanel() { closeWbPanel(); },
  renderWbPanel() { renderWbPanel(); },
  get WORKBENCH_RECIPES_JS() { return WORKBENCH_RECIPES_JS; },
  get wbGrid() { return [...wbGrid]; },
  get wbPick() { return wbPick; },
  matchWbRecipe() { return matchWbRecipe(); },
  setWbGrid(slots) { slots.forEach((v, i) => { if (i < 9) wbGrid[i] = v; }); renderWbPanel(); },
  // ── 熔爐 v1 QA 用（ROADMAP 666）──
  FURNACE, SMOOTH_STONE,
  get furnacePanelVisible() { return furnacePanelVisible(); },
  openFurnacePanel() { openFurnacePanel(); },
  closeFurnacePanel() { closeFurnacePanel(); },
  renderFurnacePanel() { renderFurnacePanel(); },
  get FURNACE_RECIPES_JS() { return FURNACE_RECIPES_JS; },
  // ── 熔爐冶煉格子化 QA 用（ROADMAP 712）──
  get furnaceGrid() { return [...furnaceGrid]; },
  get furnacePick() { return furnacePick; },
  matchFurnaceRecipe() { return matchFurnaceRecipe(); },
  setFurnaceGrid(slots) { slots.forEach((v, i) => { if (i < 3) furnaceGrid[i] = v; }); renderFurnacePanel(); },
  // ── 贈禮 v1 QA 介面（ROADMAP 660）──
  giftPickItem(inv) { return giftPickItem(inv); },
  updateGiftBtn() { updateGiftBtn(); },
  get giftBtnText() { const e = document.getElementById("chatGift"); return e ? e.textContent : ""; },
  get giftBtnEmpty() { const e = document.getElementById("chatGift"); return e ? e.classList.contains("gift-empty") : false; },
  GIFT_EXCLUDED: [...GIFT_EXCLUDED],
  // ── 麵包 v1 QA 用（ROADMAP 668）──
  WHEAT, BREAD,
  // ── 晝夜循環 v1 QA 用（ROADMAP 661）──
  get worldTime() { return worldTime; },
  updateSkyAndLight(t) { updateSkyAndLight(t); },
  get skyColor() { const c = scene.background; return { r: c.r, g: c.g, b: c.b }; },
  get sunIntensity() { return sun.intensity; },
  get hemiIntensity() { return hemi.intensity; },
  // ── 下雨天氣 v1 QA 用（ROADMAP 700）──
  get isRaining() { return isRaining; },
  set isRaining(v) { isRaining = !!v; updateSkyAndLight(worldTime); },
  get rainVisible() { return rainPoints.visible; },
  updateRain(dt) { updateRain(dt); },
  // ── 真瀏覽器 QA 用：讀準心目標、讀方塊、觸發破壞/放置、選方塊 ──
  get target() { return target; },
  getBlock(x, y, z) { return getRaw(x, y, z); },
  doBreak() { return breakAtTarget(); },
  doPlace() { return placeAtTarget(); },
  selectSlotByBlock(b) {
    // 麥塊 9 格快捷欄：若該方塊已在欄上就選那格；不在就指派進當前格（QA/測試可選任意方塊）。
    let i = HOTBAR.indexOf(b);
    if (i < 0) { i = selectedSlot; assignToHotbar(i, b); }
    selectSlot(i);
    return selectedBlock();
  },
  // ── 快捷欄麥塊化 QA 用 ──
  get HOTBAR() { return [...HOTBAR]; },
  get HOTBAR_SIZE() { return HOTBAR_SIZE; },
  get selectedSlot() { return selectedSlot; },
  assignToHotbar(slot, blockId) { assignToHotbar(slot, blockId); return [...HOTBAR]; },
  // ── 流動水階梯視覺 QA 用：注入一塊測試 chunk + 讀水面高度，驗證階梯遞減 ──
  waterTopH(b) { return waterTopH(b); },
  injectChunkForTest(cx, cy, cz, bytes) {
    chunks.set(ckey(cx, cy, cz), Uint8Array.from(bytes));
    markDirty(cx, cy, cz);
    return chunks.get(ckey(cx, cy, cz)).length;
  },
  WATER, WATER_FLOW_BASE, CHUNK,
  // ── 脫困 / 碰撞 QA 用（修玩家卡地裡）──
  // 純函式：餵假地形（isSolid 回呼）即可驗證 AABB 重疊偵測與脫困上抬，不依賴真世界。
  aabbHitsSolid(x, y, z, isSolid, pw, ph) { return aabbHitsSolid(x, y, z, isSolid, pw == null ? PW : pw, ph == null ? PH : ph); },
  unstuckY(x, y, z, isSolid, pw, ph) { return unstuckY(x, y, z, isSolid, pw == null ? PW : pw, ph == null ? PH : ph); },
  PW, PH,
  get overlapping() { return overlaps(); },     // 當前玩家是否卡在實心方塊內
  unstuckNow() { return unstuckIfNeeded(); },    // 手動觸發一次脫困（回傳是否有動）
  // 直接設玩家位置（QA 模擬出生卡住／走進未載入區後補載的情境）。
  setPlayerPos(x, y, z) { player.x = x; player.y = y; player.z = z; player.vy = 0; },
  // ── 登入綁定 QA 用（比照 3D #821）──
  get isLoggedIn() { return isLoggedIn; },
  get myAccountName() { return myAccountName; },
  get myName() { return myName; },
  // ── 深層礦石 v1 QA 用（ROADMAP 682）──
  COAL_ORE, IRON_ORE,
  // ── 鐵錠 v1 QA 用（ROADMAP 683）──
  IRON_INGOT,
  // ── 鐵磚 v1 QA 用（ROADMAP 684）──
  IRON_BLOCK,
  // ── 火把 v1 QA 用（ROADMAP 685）──
  TORCH,
  // ── 鎬具 v1 QA 用（ROADMAP 687）──
  PICKAXE_WOOD, PICKAXE_STONE, PICKAXE_IRON,
  blockHardness, pickaxeBonus,
  get mining() { return mining; },
  get isMouseDown() { return isMouseDown; },
  // ── 梯子 v1 QA 用（ROADMAP 688）──
  LADDER, CLIMB_SPEED,
  aabbHitsLadder(x, y, z, getBlock, pw, ph) {
    return aabbHitsLadder(x, y, z, getBlock, pw == null ? PW : pw, ph == null ? PH : ph);
  },
  // ── 斧頭 v1 QA 用（ROADMAP 689）──
  AXE_WOOD, AXE_STONE, AXE_IRON,
  axeBonus,
  // ── 鏟子 v1 QA 用（ROADMAP 690）──
  SHOVEL_WOOD, SHOVEL_STONE, SHOVEL_IRON,
  shovelBonus,
  // ── 火把發光 v1 QA 用（ROADMAP 691）──
  get torchCount() { return torchPositions.size; },
  get heldTorchActive() { return heldTorchLight.intensity > 0; },
  get activeTorchLights() { return torchLightPool.filter(pl => pl.visible).length; },
  registerTorchBlock,
  unregisterTorchBlock,
  updateNearbyTorchLights,
  // ── 箱子 v1 QA 用（ROADMAP 692）──
  CHEST,
  get chestPanelVisible() { return chestPanelVisible(); },
  openChestPanel(bx, by, bz) { openChestPanel(bx, by, bz); },
  closeChestPanel() { closeChestPanel(); },
  get chestPos() { return _chestPos; },
  get chestItems() { return [..._chestItems]; },
  renderChestPanel() { renderChestPanel(); },
  // ── 木門 v1 QA 用（ROADMAP 693）──
  DOOR_CLOSED, DOOR_OPEN,
  // ── 床 v1 QA 用 ──
  BED,
};
