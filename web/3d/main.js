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
  g.add(status); g.add(care); g.add(thought);
  g.userData.statusSprite = status;
  g.userData.careSprite = care;
  g.userData.thoughtSprite = thought;
}

// 每幀更新所有 NPC 的內心生活呈現（在 updateRemoteEntities(npcs) 之後呼叫：
// 那裡的 updateFade 會把每個子 sprite 的 opacity 設成 AOI 淡入淡出值，這裡再乘上各自的
// 顯示強度覆寫上去，故 AOI 淡入淡出仍生效、又不被它壓掉內心生活的呈現）。
function updateResidentStatus(t) {
  for (const [, g] of npcs) {
    const status = g.userData.statusSprite;
    if (!status) continue; // 非居民類 NPC group 沒掛（理論上都掛了，保險）
    const item = g.userData.item;
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

    // ③ 思想泡泡 💭（thought）：有內心話才飄；柔和半透明
    if (item && item.thought) {
      thought.material.map = thoughtTexture(item.thought);
      thought.material.needsUpdate = true;
      thought.visible = true;
      thought.material.opacity = fade * 0.92;
    } else if (thought) {
      thought.visible = false;
    }
  }
}

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
  // 野生動物：記住身體 group，幼獸體型縮放要套在它身上（與 fade 的 g.scale 相乘）
  if (body && body.userData && body.userData.isCreature) g.userData.creature = body;
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

// 單格耕地的視覺：把一筆 TileView 算成「這格該怎麼畫」。純函式、確定性、壞值安全。
// 回傳 null＝這格不長作物（自然地／空土／壞值）——呼叫端就不替它生作物 mesh。
// 只讀權威 `state`/`dry`，不嵌任何種田規則（能不能種是伺服器的事，前端純呈現）。
function cropCellVisual(cell) {
  if (!cell || typeof cell !== "object") return null;
  const meta = CROP_STAGE[cell.state]; // state 0/1/未知 → undefined → 不長作物
  if (!meta) return null;
  return { state: cell.state, h: meta.h, color: meta.color, glow: meta.glow, dry: cell.dry === true };
}

// 視野內農地的 HUD 標籤：幾塊地、幾株作物待收成（state 4＝成熟）。純函式、壞值安全。
// 面向玩家字串集中前端、glyph 留 i18n 空間（後端只送穩定數值 state，文案由前端對照）。
function farmHudLabel(fieldList) {
  if (!Array.isArray(fieldList) || fieldList.length === 0) return "";
  let ripe = 0;
  for (const f of fieldList) {
    const cells = f && Array.isArray(f.cells) ? f.cells : [];
    for (const c of cells) if (c && c.state === 4) ripe++;
  }
  return `🌾 農地 ${fieldList.length}${ripe > 0 ? " · " + ripe + " 株待收" : ""}`;
}

// 一塊田的「視覺指紋」：把每格的 state/dry 串成字串＋稻草人位置。只有指紋變了才重建作物
// mesh（多數幀作物沒變、不必每幀重生 mesh＝近乎零增量開銷）。壞值安全（缺 cells → 空指紋）。
function fieldDigest(field) {
  if (!field || typeof field !== "object") return "x";
  let d = "";
  const cells = Array.isArray(field.cells) ? field.cells : [];
  for (const c of cells) d += (c && typeof c.state === "number" ? c.state : 0) + (c && c.dry ? "w" : "") + ",";
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
function reconcileFields(list, recvT) {
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

// 每幀更新所有田：作物隨風輕搖、成熟金果發光脈動、AOI 淡入淡出。皆尊重 reduceMotion。
function updateFields(dt, t) {
  for (const [key, g] of fields) {
    const layer = g.userData.cropLayer;
    if (layer && !reduceMotion) {
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
    if (updateFade(g, dt)) { scene.remove(g); fields.delete(key); }
  }
}

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
    case "snapshot": {
      snapshotCount++;
      // 日夜狀態：留存最新一筆權威 daynight，render 每幀據此讓世界的天色／光照流轉（ROADMAP 612）。
      if (msg.daynight && typeof msg.daynight === "object") latestDayNight = msg.daynight;
      // 天氣／彩虹：留存最新一筆權威 weather／rainbow，render 每幀據此讓粒子場與遠空彩虹流轉（ROADMAP 613）。
      if (msg.weather && typeof msg.weather === "object") latestWeather = msg.weather;
      if (msg.rainbow && typeof msg.rainbow === "object") latestRainbow = msg.rainbow;
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
      // 農地（每位玩家的耕地）：把翻好的土＋成長中的作物＋稻草人接進 3D（ROADMAP 614）。
      reconcileFields(msg.fields, recvT);

      // 人造地標（眾人一起蓋的篝火／協力瞭望塔／署名雪人）：接進 3D（ROADMAP 616）。
      // 都是固定地標、讀 wx/wy；瞭望塔在進度變動時即時把塔身升到對應高度。
      reconcileStatic(msg.campfires, campfires, "cf", makeCampfire, null, recvT);
      reconcileStatic(msg.watchtowers, watchtowers, "wt", makeWatchtower,
        (g, item) => applyTowerProgress(g, watchtowerVisual(item)), recvT);
      reconcileStatic(msg.snowmen, snowmen, "sm", makeSnowman, null, recvT);

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

      // HUD：線上人數／自己名字／日夜階段／操作提示
      const meItem = Array.isArray(msg.players) ? msg.players.find((p) => p.id === myId) : null;
      const myName = meItem ? (meItem.name || "玩家") : "（加入中…）";
      const phaseLabel = dayNightPhaseLabel(latestDayNight);
      const weatherLabel = weatherHudLabel(latestWeather, latestRainbow);
      const farmLabel = farmHudLabel(msg.fields); // 視野內農地數＋待收成作物株數（ROADMAP 614）
      const wildLabel = wildlifeHudLabel(msg.wildlife); // 野生動物數＋其中已馴養數（ROADMAP 615）
      const builtLabel = structuresHudLabel(msg.campfires, msg.watchtowers, msg.snowmen); // 視野內人造地標（ROADMAP 616）
      hudEl.innerHTML =
        `<b>${myName}</b> · 線上 ${players.size} 人${phaseLabel ? " · " + phaseLabel : ""}${weatherLabel ? " · " + weatherLabel : ""}\n` +
        `NPC ${npcs.size} · ${wildLabel || "野生 " + wildlife.size} · 敵人 ${enemies.size}${farmLabel ? " · " + farmLabel : ""}${builtLabel ? " · " + builtLabel : ""}\n` +
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
    // 日夜流轉：依最新權威 daynight 平滑更新天空／太陽／環境光（ROADMAP 612）
    applyDayNight(dt);
    // 天氣流轉：依最新權威 weather／rainbow 平滑更新粒子場／霧染／遠空彩虹（ROADMAP 613，疊在日夜之後）
    applyWeather(dt);

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
    // NPC 內心生活呈現：在 npcs 的 updateFade 之後覆寫狀態/關懷/思想 sprite 的顯示（ROADMAP 611）
    updateResidentStatus(t);
    updateRemoteEntities(wildlife, scene, renderTime, true, dt, t, undefined, k);
    // 野生動物呈現：在 wildlife 的 updateFade 之後覆寫頭頂狀態 sprite 與幼獸體型（ROADMAP 615）
    updateWildlifeStatus(t);
    updateRemoteEntities(enemies, scene, renderTime, true, dt, t, undefined, k);
    // 節點靜態：不轉身/起伏；位置吸最新目標（內插對靜態無差），仍走 AOI 淡入淡出
    updateRemoteEntities(nodes, scene, renderTime, false, dt, t, undefined, 1);
    // 農地：作物隨風輕搖、成熟金果發光脈動、AOI 淡入淡出（ROADMAP 614）
    updateFields(dt, t);
    // 人造地標：篝火火焰跳動／暖圈入夜更亮、塔頂燈入夜亮起、雪人愛心數（ROADMAP 616）
    updateStructures(dt, t);

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

// 測試掛鉤（scripts/qa/render-smoke-3d.mjs 用；瀏覽器中無副作用、只暴露純邏輯供斷言）。
if (typeof globalThis !== "undefined") {
  globalThis.__bf3dTest = { residentStatusEmoji, NPC_ACTIVITY_ICON, thoughtTexture, dayNightVisual, dayNightPhaseLabel, weatherVisual, weatherHudLabel, cropCellVisual, fieldDigest, farmHudLabel, wildlifeVisual, wildlifeStatusEmoji, wildlifeHudLabel, campfireVisual, watchtowerVisual, snowmanVisual, structuresHudLabel };
}

// 啟動
setStatus("連線中…");
connect();
safeRender();
