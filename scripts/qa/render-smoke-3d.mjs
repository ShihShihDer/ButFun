// Render-smoke（3D／ROADMAP 611）：用假 THREE / 假 DOM / 假 WebSocket 載入「真正的」
// web/3d/main.js，餵一份含各種內心生活狀態（活動／思想／關懷／危機／歡慶）的 NPC 快照，
// 實際逐幀跑 safeRender()，抓任何在繪製中拋出的例外——這正是「3D 角色突然不見／畫面凍結」
// 的根因型態（render 一拋就停 rAF 迴圈）。同時對 residentStatusEmoji 純邏輯驗真值表。
// 用法：node scripts/qa/render-smoke-3d.mjs
import { readFileSync } from "fs";
import vm from "vm";

let src = readFileSync(new URL("../../web/3d/main.js", import.meta.url), "utf8");
// main.js 以 ESM `import * as THREE from "three"` 取得 THREE；vm 不解析 ESM，
// 故把該行抽掉、改由 sandbox 注入全域 THREE（行為等價）。
src = src.replace(/^import \* as THREE from "three";\s*$/m, "/* THREE 由 sandbox 注入 */");

// ── 假 THREE：所有用到的類別都給最小可用實作（位置/縮放/旋轉是真的可變物件，
//    才能讓內插、轉身、淡入淡出等真的跑起來、踩到真實程式路徑）──────────────────
function v3() {
  return {
    x: 0, y: 0, z: 0,
    set(a, b, c) { this.x = a; this.y = b; this.z = c; return this; },
    copy(o) { this.x = o.x; this.y = o.y; this.z = o.z; return this; },
    lerp(o, a) { this.x += (o.x - this.x) * a; this.y += (o.y - this.y) * a; this.z += (o.z - this.z) * a; return this; },
  };
}
function scaleObj() {
  return { x: 1, y: 1, z: 1, set(a, b, c) { this.x = a; this.y = b; this.z = c; return this; }, setScalar(s) { this.x = this.y = this.z = s; return this; } };
}
class Obj3D {
  constructor() {
    this.position = v3();
    this.rotation = { x: 0, y: 0, z: 0 };
    this.scale = scaleObj();
    this.children = [];
    this.userData = {};
    this.material = null;
    this.visible = true;
  }
  add(c) { this.children.push(c); return this; }
  remove(c) { const i = this.children.indexOf(c); if (i >= 0) this.children.splice(i, 1); return this; }
  // 忠實鏡像 THREE.Object3D.traverse：先訪自己、再遞迴所有子節點（#767 起火柴人是巢狀群組，
  // updateFade 用 traverse 才能調到藏在深處的材質 opacity）。
  traverse(cb) { cb(this); for (const c of this.children) c.traverse(cb); }
  lookAt() {}
  updateProjectionMatrix() {}
}
class Mesh extends Obj3D { constructor(geo, mat) { super(); this.geometry = geo; this.material = mat; } }
class Sprite extends Obj3D { constructor(mat) { super(); this.material = mat || {}; } }
class Geo { constructor() {} }
const geo = Geo;
// 假 Color：忠實提供 setRGB（main.js 的日夜系統每幀對背景／霧／太陽光色 setRGB）。
class FColor { constructor(c) { this.c = c; this.r = 0; this.g = 0; this.b = 0; } setRGB(r, g, b) { this.r = r; this.g = g; this.b = b; return this; } }
const THREE = {
  Scene: class extends Obj3D { constructor() { super(); this.background = null; this.fog = null; } },
  Color: FColor,
  // 真 THREE 的 Fog 會把傳入色值包成 Color；鏡像之，才能讓 fog.color.setRGB 跑真實路徑。
  Fog: class { constructor(c, n, f) { this.color = new FColor(c); this.near = n; this.far = f; } },
  FogExp2: class { constructor() {} },
  PerspectiveCamera: class extends Obj3D { constructor(fov, aspect) { super(); this.fov = fov; this.aspect = aspect; } },
  WebGLRenderer: class {
    constructor() { this.domElement = makeEl("<canvas>"); }
    setSize() {} setPixelRatio() {} render() {}
  },
  HemisphereLight: class extends Obj3D {}, DirectionalLight: class extends Obj3D {}, AmbientLight: class extends Obj3D {},
  Mesh, Sprite, Group: class extends Obj3D {},
  PlaneGeometry: geo, BoxGeometry: geo, CapsuleGeometry: geo, ConeGeometry: geo,
  OctahedronGeometry: geo, DodecahedronGeometry: geo, SphereGeometry: geo, CylinderGeometry: geo,
  MeshLambertMaterial: class { constructor(o) { Object.assign(this, o || {}); } },
  MeshBasicMaterial: class { constructor(o) { Object.assign(this, o || {}); } },
  SpriteMaterial: class { constructor(o) { Object.assign(this, o || {}); this.needsUpdate = false; } },
  // 天氣粒子場（ROADMAP 613）：PointsMaterial 給可變 size/opacity/color（color 是真的 FColor 才能 setRGB）。
  PointsMaterial: class { constructor(o) { Object.assign(this, o || {}); if (!this.color) this.color = new FColor(); } },
  TorusGeometry: geo,
  // BufferGeometry／BufferAttribute：忠實提供 setAttribute／setDrawRange／attributes.position.needsUpdate
  //（applyWeather 每幀寫位置陣列後標 needsUpdate，要踩到真實路徑）。
  BufferGeometry: class { constructor() { this.attributes = {}; this.drawRange = { start: 0, count: Infinity }; } setAttribute(n, a) { this.attributes[n] = a; return this; } setDrawRange(s, c) { this.drawRange.start = s; this.drawRange.count = c; } dispose() {} },
  BufferAttribute: class { constructor(arr, itemSize) { this.array = arr; this.itemSize = itemSize; this.needsUpdate = false; } },
  Points: class extends Obj3D { constructor(geom, mat) { super(); this.geometry = geom; this.material = mat; this.frustumCulled = true; } },
  CanvasTexture: class { constructor(cv) { this.image = cv; this.anisotropy = 1; } dispose() {} },
  GridHelper: class extends Obj3D {},
  Clock: class { constructor() { this._t = 0; } getDelta() { return 0.016; } get elapsedTime() { return (this._t += 0.016); } },
  Vector3: class { constructor(x = 0, y = 0, z = 0) { this.x = x; this.y = y; this.z = z; } },
};

// ── 假 2D ctx（給 makeLabel／emojiTexture／thoughtTexture 的離屏 canvas 用）──
function makeCtx() {
  const noop = () => {};
  return {
    font: "", textAlign: "", textBaseline: "", fillStyle: "", strokeStyle: "", lineWidth: 0,
    fillText: noop, strokeText: noop, beginPath: noop, moveTo: noop, lineTo: noop, arcTo: noop,
    closePath: noop, fill: noop, stroke: noop, fillRect: noop, clearRect: noop,
    measureText: (s) => ({ width: s == null ? 0 : String(s).length * 12 }),
  };
}
const elCache = new Map();
function makeEl(id) {
  if (elCache.has(id)) return elCache.get(id);
  const el = {
    id, width: 256, height: 64, style: new Proxy({}, { get: () => "", set: () => true }),
    textContent: "", innerHTML: "",
    classList: { add: () => {}, remove: () => {}, toggle: () => {}, contains: () => false },
    appendChild: () => {}, addEventListener: () => {}, removeEventListener: () => {},
    getContext: (t) => (t === "2d" ? makeCtx() : null),
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 100, height: 100 }),
  };
  elCache.set(id, el);
  return el;
}

// ── 假 WebSocket：擷取實例，手動驅動 onopen/onmessage ──
let lastWS = null;
class FakeWS {
  constructor(url) { this.url = url; this.readyState = 1; lastWS = this; this.onopen = null; this.onmessage = null; this.onclose = null; this.onerror = null; }
  send() {} close() { this.readyState = 3; }
}
FakeWS.CONNECTING = 0; FakeWS.OPEN = 1; FakeWS.CLOSING = 2; FakeWS.CLOSED = 3;

// ── requestAnimationFrame：擷取 callback，手動逐幀呼叫 ──
let rafCb = null;
let perfNow = 0;

const documentStub = {
  getElementById: (id) => makeEl(id),
  createElement: (tag) => makeEl("<" + tag + ">"),
  elementFromPoint: () => null,
  addEventListener: () => {}, removeEventListener: () => {},
  body: makeEl("body"),
};

// 攔截 console.error/warn：safeRender 把繪製例外印成 console.error；reconcile/handleServerMsg
// 把單筆失敗印成 console.warn。任一被攔到都代表 3D 渲染踩到真 bug，測試要紅。
const caught = [];
const consoleProxy = {
  ...console,
  error: (...a) => { caught.push("ERROR " + a.map(x => (x && x.stack) ? x.stack : String(x)).join(" ")); console.error(...a); },
  warn: (...a) => { caught.push("WARN " + a.map(x => (x && x.stack) ? x.stack : String(x)).join(" ")); console.warn(...a); },
};

const windowStub = {
  THREE,
  requestAnimationFrame: (cb) => { rafCb = cb; return 1; },
  cancelAnimationFrame: () => {},
  performance: { now: () => perfNow },
  navigator: { maxTouchPoints: 0, userAgent: "node-render-smoke-3d" },
  location: { host: "localhost:3000", protocol: "http:" },
  devicePixelRatio: 1, innerWidth: 800, innerHeight: 600,
  addEventListener: () => {}, removeEventListener: () => {},
  matchMedia: () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {} }),
  setTimeout: () => 0, clearTimeout: () => {},
  WebSocket: FakeWS,
};

const sandbox = { ...windowStub, document: documentStub, console: consoleProxy };
sandbox.window = sandbox; sandbox.self = sandbox; sandbox.globalThis = sandbox;
vm.createContext(sandbox);

function fail(msg) { console.error("❌ " + msg); process.exit(1); }

// ── 載入真正的 3D 客戶端 ──
try { vm.runInContext(src, sandbox, { filename: "web/3d/main.js" }); }
catch (e) { fail("載入 web/3d/main.js 即拋例外：" + (e && e.stack ? e.stack : e)); }
if (!lastWS) fail("connect() 未建立 WebSocket");
console.log("✅ 3D 客戶端載入、WebSocket 已建立:", lastWS.url);

// ── ① 純邏輯真值表：residentStatusEmoji 優先序與壞值安全 ──
const T = sandbox.__bf3dTest;
if (!T || typeof T.residentStatusEmoji !== "function") fail("__bf3dTest.residentStatusEmoji 未暴露");
const rse = T.residentStatusEmoji;
const cases = [
  [null, null, "null item → 無 emoji"],
  [{}, null, "空 NPC → 無 emoji"],
  [{ activity: "hammering" }, "🔨", "活動 hammering → 🔨"],
  [{ activity: "patrolling" }, "👀", "活動 patrolling → 👀"],
  [{ activity: "no_such_code" }, null, "未知活動代碼 → 安全 null"],
  [{ celebrating: true, activity: "hammering" }, "🎉", "歡慶蓋過活動 → 🎉"],
  [{ alarmed: true, celebrating: true, activity: "hammering" }, "😰", "危機優先序最高 → 😰"],
];
for (const [item, want, desc] of cases) {
  const got = rse(item);
  if (got !== want) fail(`真值表失誤：${desc}（得 ${JSON.stringify(got)}，期望 ${JSON.stringify(want)}）`);
}
// thoughtTexture 對壞值不拋（null/長字串都該安全產出貼圖）
try { T.thoughtTexture(null); T.thoughtTexture("這是一句很長很長的內心話用來測試截斷不會爆掉喔喔喔喔"); }
catch (e) { fail("thoughtTexture 壞值拋例外：" + e); }
console.log("✅ residentStatusEmoji 真值表 7 例全綠、thoughtTexture 壞值安全");

// ── ①b 日夜視覺純邏輯：天空/太陽/環境光參數的合理性、極端對比、壞值安全（ROADMAP 612）──
if (typeof T.dayNightVisual !== "function" || typeof T.dayNightPhaseLabel !== "function") fail("__bf3dTest 未暴露 dayNightVisual/dayNightPhaseLabel");
const dnv = T.dayNightVisual;
function okVisual(v, desc) {
  const rgbOk = (a) => Array.isArray(a) && a.length === 3 && a.every((x) => Number.isFinite(x) && x >= 0 && x <= 1);
  if (!rgbOk(v.sky)) fail(`${desc}：sky RGB 非法 ${JSON.stringify(v.sky)}`);
  if (!rgbOk(v.fog)) fail(`${desc}：fog RGB 非法 ${JSON.stringify(v.fog)}`);
  if (!rgbOk(v.sun)) fail(`${desc}：sun 光色 RGB 非法 ${JSON.stringify(v.sun)}`);
  if (!(Number.isFinite(v.sunIntensity) && v.sunIntensity > 0)) fail(`${desc}：sunIntensity 非法 ${v.sunIntensity}`);
  if (!(Number.isFinite(v.hemiIntensity) && v.hemiIntensity > 0)) fail(`${desc}：hemiIntensity 非法 ${v.hemiIntensity}`);
  for (const k of ["x", "y", "z"]) if (!Number.isFinite(v.sunPos[k])) fail(`${desc}：sunPos.${k} 非有限`);
}
const dayV = dnv({ phase: "day", day_fraction: 0.33, light: 1.0 });
const nightV = dnv({ phase: "night", day_fraction: 0.8, light: 0.2 });
const dangerV = dnv({ phase: "night", day_fraction: 0.8, light: 0.2, night_danger: true });
okVisual(dayV, "白天"); okVisual(nightV, "夜晚"); okVisual(dangerV, "夜間危機");
okVisual(dnv(null), "預設(null)"); okVisual(dnv({ day_fraction: NaN, light: "x" }), "壞值");
// 對比：白天比夜晚亮（太陽強度、環境光、天空總亮度都該更高）
const sum = (a) => a[0] + a[1] + a[2];
if (!(dayV.sunIntensity > nightV.sunIntensity)) fail("白天太陽強度應大於夜晚");
if (!(dayV.hemiIntensity > nightV.hemiIntensity)) fail("白天環境光應大於夜晚");
if (!(sum(dayV.sky) > sum(nightV.sky))) fail("白天天空應比夜晚明亮");
// 太陽仰角：正午應高於夜晚（位置 y）
if (!(dayV.sunPos.y > nightV.sunPos.y)) fail("正午太陽應高於夜晚");
// 夜間危機讓霧色更偏紅（fog 紅通道升高）
if (!(dangerV.fog[0] > nightV.fog[0])) fail("夜間危機霧色紅通道應升高");
// 確定性：同輸入同輸出
if (JSON.stringify(dnv({ day_fraction: 0.5, light: 0.6 })) !== JSON.stringify(dnv({ day_fraction: 0.5, light: 0.6 }))) fail("dayNightVisual 非確定性");
// 階段標籤：已知碼有值、未知/缺欄位回空字串
if (!T.dayNightPhaseLabel({ phase: "dusk" })) fail("dusk 應有階段標籤");
if (T.dayNightPhaseLabel({ phase: "no_such" }) !== "") fail("未知階段碼應回空字串");
if (T.dayNightPhaseLabel(null) !== "") fail("null 應回空字串");
console.log("✅ dayNightVisual 合理性／晝夜對比／危機紅化／壞值安全＋階段標籤全綠");

// ── ①c 天氣視覺純邏輯：粒子場參數合理、雨/沙/晶/霧區隔、晴天/壞值安全、HUD 標籤（ROADMAP 613）──
if (typeof T.weatherVisual !== "function" || typeof T.weatherHudLabel !== "function") fail("__bf3dTest 未暴露 weatherVisual/weatherHudLabel");
const wv = T.weatherVisual;
function okWeather(v, desc) {
  const rgbOk = (a) => Array.isArray(a) && a.length === 3 && a.every((x) => Number.isFinite(x) && x >= 0 && x <= 1);
  if (!rgbOk(v.color)) fail(`${desc}：粒子 color RGB 非法 ${JSON.stringify(v.color)}`);
  if (!(Number.isFinite(v.density) && v.density >= 0 && v.density <= 1)) fail(`${desc}：density 非法 ${v.density}`);
  if (!(Number.isFinite(v.size) && v.size > 0)) fail(`${desc}：size 非法 ${v.size}`);
  for (const k of ["fall", "windX", "windZ", "fogMix", "fogFar"]) if (!Number.isFinite(v[k])) fail(`${desc}：${k} 非有限 ${v[k]}`);
}
const wind = { dir_x: 1, dir_y: 0, strength: 1 };
const rainV = wv({ weather_type: "grassland_rain", intensity: 1.0, wind });
const sandV = wv({ weather_type: "desert_sandstorm", intensity: 1.0, wind });
const dustV = wv({ weather_type: "rocky_crystal_dust", intensity: 0.6, wind });
const mistV = wv({ weather_type: "water_sea_mist", intensity: 0.8, wind });
const clearV = wv({ weather_type: "clear", intensity: 0 });
okWeather(rainV, "細雨"); okWeather(sandV, "風沙"); okWeather(dustV, "晶塵"); okWeather(mistV, "海霧");
okWeather(clearV, "晴天"); okWeather(wv(null), "預設(null)"); okWeather(wv({ weather_type: "no_such", intensity: "x", wind: null }), "壞值");
// 晴天：不掉粒子（density 0）、不染霧（fogMix 0）、視野最遠（fogFar 600）
if (clearV.density !== 0 || clearV.fogMix !== 0) fail("晴天不該掉粒子／染霧");
if (!(clearV.fogFar >= 600)) fail("晴天視野應最遠");
// 有天氣：density>0、起霧染色 fogMix>0、視野被拉近（fogFar<600）
if (!(rainV.density > 0 && rainV.fogMix > 0 && rainV.fogFar < 600)) fail("下雨應掉粒子＋染霧＋視野拉近");
// 海霧上飄（fall<0）、其餘下落（fall>0）——3D 把 2D「海霧往上漂」的差異忠實帶過來
if (!(mistV.fall < 0)) fail("海霧粒子應上飄（fall<0）");
if (!(rainV.fall > 0 && sandV.fall > 0 && dustV.fall > 0)) fail("雨/沙/晶塵應下落（fall>0）");
// 風沙最隨風（windX 最大）、沙暴視野最短——鏡像 2D 易感度/能見度差異
if (!(sandV.windX > rainV.windX)) fail("風沙應比細雨更隨風橫掃");
if (!(sandV.fogFar < rainV.fogFar)) fail("風沙視野應比細雨更短");
// 無風時粒子不橫移
const calm = wv({ weather_type: "grassland_rain", intensity: 1.0, wind: { dir_x: 1, dir_y: 0, strength: 0 } });
if (calm.windX !== 0 || calm.windZ !== 0) fail("無風時粒子不該橫移");
// 確定性：同輸入同輸出
if (JSON.stringify(wv({ weather_type: "grassland_rain", intensity: 0.5, wind })) !== JSON.stringify(wv({ weather_type: "grassland_rain", intensity: 0.5, wind }))) fail("weatherVisual 非確定性");
// HUD 標籤：有天氣有字、晴天無字、彩虹追加、晴天+彩虹只剩彩虹
if (!T.weatherHudLabel({ weather_type: "grassland_rain", intensity: 0.8 }, null)) fail("下雨應有 HUD 標籤");
if (T.weatherHudLabel({ weather_type: "clear", intensity: 0 }, null) !== "") fail("晴天無彩虹應回空字串");
if (!/🌈/.test(T.weatherHudLabel({ weather_type: "clear", intensity: 0 }, { active: true }))) fail("彩虹時應顯示 🌈");
if (!/🌧️/.test(T.weatherHudLabel({ weather_type: "grassland_rain", intensity: 0.8 }, { active: true })) || !/🌈/.test(T.weatherHudLabel({ weather_type: "grassland_rain", intensity: 0.8 }, { active: true }))) fail("下雨+彩虹應同時顯示");
console.log("✅ weatherVisual 合理性／雨沙晶霧區隔／晴天壞值安全／風向／HUD 標籤全綠");

// ── ② 逐幀跑：先 welcome，再餵含各種內心生活的 NPC 快照，跑多幀抓例外 ──
function drive(msg) { lastWS.onmessage({ data: JSON.stringify(msg) }); }
function frames(n) { for (let i = 0; i < n; i++) { perfNow += 33; if (rafCb) { const cb = rafCb; rafCb = null; cb(); } } }

lastWS.onopen && lastWS.onopen();
drive({ type: "welcome", id: "me", world: { width: 6000, height: 6000 } });

const npcsA = [
  { id: "n1", name: "鐵匠", x: 3000, y: 3000, activity: "hammering" },
  { id: "n2", name: "里長", x: 3100, y: 3000, activity: "patrolling", thought: "今晚別出事就好" },
  { id: "n3", name: "獵手", x: 3200, y: 3000, alarmed: true },
  { id: "n4", name: "商人", x: 3300, y: 3000, celebrating: true },
  { id: "n5", name: "農婦", x: 3400, y: 3000, needs_care: true, thought: "有點累了" },
  { id: "n6", name: "旅人", x: 3500, y: 3000 }, // 無任何內心生活欄位（其他 NPC）
];
drive({ type: "snapshot", players: [{ id: "me", name: "我", x: 3000, y: 3000 }], npcs: npcsA, wildlife: [], enemies: [], nodes: [],
  daynight: { phase: "day", day_fraction: 0.33, light: 1.0, night_danger: false },
  // 細雨＋橫風＋彩虹：踩 applyWeather 的粒子推進／回收、霧染、彩虹淡入路徑（ROADMAP 613）
  weather: { weather_type: "grassland_rain", intensity: 0.9, wind: { dir_x: 0.8, dir_y: 0.6, strength: 0.7 }, fish_phase: 0 },
  rainbow: { active: true, remaining_secs: 30 } });
frames(8);

// 狀態轉移：活動結束（→null）、思想消失、危機解除轉歡慶、需求被撫平——踩 setSpriteEmoji 換貼圖/熄滅路徑
const npcsB = [
  { id: "n1", name: "鐵匠", x: 3000, y: 3000 }, // 活動結束
  { id: "n2", name: "里長", x: 3100, y: 3000, activity: "patrolling" }, // 思想消失
  { id: "n3", name: "獵手", x: 3200, y: 3000, celebrating: true }, // 危機→歡慶
  { id: "n5", name: "農婦", x: 3400, y: 3000 }, // 需求撫平
  // n4、n6 從快照消失 → 走 AOI 淡出移除
];
// 同時把日夜推進到「夜間危機」：踩 applyDayNight 的天色／太陽流轉 + 危機紅化路徑
drive({ type: "snapshot", players: [{ id: "me", name: "我", x: 3000, y: 3000 }], npcs: npcsB, wildlife: [], enemies: [], nodes: [],
  daynight: { phase: "night", day_fraction: 0.82, light: 0.2, night_danger: true },
  // 天氣切到海霧（上飄粒子）＋彩虹消失：踩 fall<0 上飄回收、霧染轉色、彩虹淡出路徑
  weather: { weather_type: "water_sea_mist", intensity: 0.7, wind: { dir_x: -0.5, dir_y: 0.3, strength: 0.4 }, fish_phase: 1 },
  rainbow: { active: false, remaining_secs: 0 } });
frames(12);

if (caught.length) {
  console.error("❌ 跑了多幀後攔到 " + caught.length + " 筆繪製例外/警告：");
  for (const c of caught.slice(0, 10)) console.error("   · " + c);
  process.exit(1);
}
console.log("✅ NPC 內心生活（活動／思想／關懷／危機／歡慶＋狀態轉移＋AOI 淡出）跑 20 幀零例外");
console.log("✅ render-smoke-3d 全綠");
