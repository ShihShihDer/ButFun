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
    // 真 THREE 的 rotation 是 Euler，帶 .set(x,y,z)（makeCreature 對部位 rotation.set 擺姿勢）。
    this.rotation = { x: 0, y: 0, z: 0, set(a, b, c) { this.x = a; this.y = b; this.z = c; return this; } };
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
class FColor { constructor(c) { this.c = c; this.r = 0; this.g = 0; this.b = 0; } setRGB(r, g, b) { this.r = r; this.g = g; this.b = b; return this; } setHex(h) { this.hex = h; return this; } }
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
  // Float32BufferAttribute／Line／LineBasicMaterial：派系關係連線（ROADMAP 625）每幀寫位置陣列、
  // 標 needsUpdate、setHex 改色，要踩到真實路徑。
  Float32BufferAttribute: class { constructor(arr, itemSize) { this.array = arr; this.itemSize = itemSize; this.needsUpdate = false; } },
  LineBasicMaterial: class { constructor(o) { Object.assign(this, o || {}); this.color = new FColor(); this.opacity = 1; } dispose() {} },
  Line: class extends Obj3D { constructor(geom, mat) { super(); this.geometry = geom; this.material = mat; } },
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
    setAttribute: () => {}, removeAttribute: () => {}, contains: () => false,
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

// ── ①b2 天上日月星辰純邏輯：日月相對起落、晨昏交班、星空隨夜淡入、壞值安全（ROADMAP 628）──
if (typeof T.celestialSky !== "function") fail("__bf3dTest 未暴露 celestialSky");
const cs = T.celestialSky;
function okCele(v, desc) {
  for (const body of ["sun", "moon"]) {
    const b = v[body];
    if (!b || !Number.isFinite(b.ew) || !Number.isFinite(b.elev)) fail(`${desc}：${body} 方位/仰角非有限`);
    if (!(Number.isFinite(b.opacity) && b.opacity >= 0 && b.opacity <= 1)) fail(`${desc}：${body} opacity 非法 ${b.opacity}`);
  }
  if (!(Number.isFinite(v.starOpacity) && v.starOpacity >= 0 && v.starOpacity <= 1)) fail(`${desc}：starOpacity 非法 ${v.starOpacity}`);
  if (!Number.isFinite(v.spin)) fail(`${desc}：spin 非有限`);
}
const noon = cs({ phase: "day", day_fraction: 0.5, light: 1.0 });
const midnight = cs({ phase: "night", day_fraction: 0.0, light: 0.2 });
const dawnC = cs({ phase: "dawn", day_fraction: 0.25, light: 0.6 });
const duskC = cs({ phase: "dusk", day_fraction: 0.75, light: 0.6 });
okCele(noon, "正午"); okCele(midnight, "午夜"); okCele(dawnC, "破曉"); okCele(duskC, "黃昏");
okCele(cs(null), "預設(null)"); okCele(cs({ day_fraction: NaN, light: "x" }), "壞值");
// 正午：太陽登頂且明亮、月亮沉底且全隱、星辰白天全隱
if (!(noon.sun.elev > 0.9 && noon.sun.opacity > 0.9)) fail("正午太陽應登頂且明亮");
if (!(noon.moon.elev < -0.9 && noon.moon.opacity < 0.05)) fail("正午月亮應沉底且隱沒");
if (!(noon.starOpacity < 0.05)) fail("正午星辰應全隱");
// 午夜：月亮登頂且現身、太陽沉底全隱、星辰最盛
if (!(midnight.moon.elev > 0.9 && midnight.moon.opacity > 0.7)) fail("午夜月亮應登頂且現身");
if (!(midnight.sun.elev < -0.9 && midnight.sun.opacity < 0.05)) fail("午夜太陽應沉底且隱沒");
if (!(midnight.starOpacity > 0.8)) fail("午夜星辰應最盛");
// 日月恆相對：仰角符號相反、方位相反
if (!(Math.abs(noon.sun.elev + noon.moon.elev) < 1e-9 && Math.abs(noon.sun.ew + noon.moon.ew) < 1e-9)) fail("日月應恆相對（仰角/方位相反）");
// 晨昏交班：破曉與黃昏時日月雙雙貼地平、雙雙轉淡
if (!(dawnC.sun.opacity < 0.5 && dawnC.moon.opacity < 0.5)) fail("破曉時日月應雙雙轉淡");
if (!(duskC.sun.opacity < 0.5 && duskC.moon.opacity < 0.5)) fail("黃昏時日月應雙雙轉淡");
// 東昇西落：破曉太陽偏東(ew>0)、黃昏偏西(ew<0)
if (!(dawnC.sun.ew > 0.5)) fail("破曉太陽應偏東（ew>0）");
if (!(duskC.sun.ew < -0.5)) fail("黃昏太陽應偏西（ew<0）");
// 越暗星越濃：暗夜星空濃於黃昏
if (!(midnight.starOpacity > duskC.starOpacity)) fail("越暗星辰應越濃");
// 確定性：同輸入同輸出
if (JSON.stringify(cs({ day_fraction: 0.4, light: 0.7 })) !== JSON.stringify(cs({ day_fraction: 0.4, light: 0.7 }))) fail("celestialSky 非確定性");
console.log("✅ celestialSky 日月相對起落／晨昏交班／東昇西落／星空隨夜淡入／壞值安全全綠");

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

// ── ①d 農地視覺純邏輯：作物階段／指紋／HUD 標籤、壞值安全（ROADMAP 614）──
if (typeof T.cropCellVisual !== "function" || typeof T.fieldDigest !== "function" || typeof T.farmHudLabel !== "function") fail("__bf3dTest 未暴露 cropCellVisual/fieldDigest/farmHudLabel");
const ccv = T.cropCellVisual;
// 自然地/空土/未知 state → 不長作物（null）
for (const st of [0, 1, 5, 99]) if (ccv({ state: st }) !== null) fail(`state ${st} 不該長作物`);
// 壞值安全：null / 非物件 → null，不拋
if (ccv(null) !== null || ccv(undefined) !== null || ccv("x") !== null) fail("cropCellVisual 壞值應回 null");
// 種子/發芽/成熟 → 有視覺，且高度遞增（三階段一眼分得開）、只有成熟發光
const seed = ccv({ state: 2 }), sprout = ccv({ state: 3 }), mature = ccv({ state: 4 });
for (const [v, d] of [[seed, "種子"], [sprout, "發芽"], [mature, "成熟"]]) {
  if (!v || !Number.isFinite(v.h) || v.h <= 0) fail(`${d} 高度非法 ${v && v.h}`);
  if (!Number.isFinite(v.color)) fail(`${d} 顏色非法`);
}
if (!(seed.h < sprout.h && sprout.h < mature.h)) fail("作物高度應隨階段遞增（種子<發芽<成熟）");
if (seed.glow || sprout.glow || !mature.glow) fail("只有成熟作物該發光");
// 缺水旗標忠實帶過
if (ccv({ state: 3, dry: true }).dry !== true || ccv({ state: 3, dry: false }).dry !== false) fail("缺水旗標應忠實帶過");
// 熟成進度 grow（ROADMAP 624）：成長中讀後端 grow（0~100、clamp、壞值→0）、成熟視為滿
if (typeof T.cropBarFill !== "function") fail("__bf3dTest 未暴露 cropBarFill");
if (ccv({ state: 3, grow: 40 }).grow !== 40) fail("成長中應忠實帶過 grow");
if (ccv({ state: 2 }).grow !== 0 || ccv({ state: 3, grow: "x" }).grow !== 0) fail("缺/壞 grow 應歸 0");
if (ccv({ state: 3, grow: 150 }).grow !== 100 || ccv({ state: 3, grow: -5 }).grow !== 0) fail("grow 應夾在 [0,100]");
if (ccv({ state: 4, grow: 0 }).grow !== 100) fail("成熟作物 grow 應視為滿 100");
// cropBarFill：填充比例＋「就快可收」≥80 轉暖金（鏡像 2D 0.8 閾值）、壞值安全
const cbf = T.cropBarFill;
if (cbf(0).ratio !== 0 || cbf(100).ratio !== 1 || Math.abs(cbf(50).ratio - 0.5) > 1e-9) fail("cropBarFill ratio 應正比 grow");
if (cbf(79).soon !== false || cbf(80).soon !== true || cbf(100).soon !== true) fail("cropBarFill ≥80 才 soon");
if (cbf("x").ratio !== 0 || cbf(NaN).ratio !== 0 || cbf(999).ratio !== 1 || cbf(-9).ratio !== 0) fail("cropBarFill 壞值/越界應夾安全");
// fieldDigest：cells 變了指紋就變、沒變就同（重建作物層的依據）、壞值安全
const fd = T.fieldDigest;
const fA = { cells: [{ state: 2 }, { state: 4 }], scarecrow: null };
const fB = { cells: [{ state: 3 }, { state: 4 }], scarecrow: null }; // 第一格長大了
if (fd(fA) === fd(fB)) fail("作物階段變了，指紋應改變");
if (fd(fA) !== fd({ cells: [{ state: 2 }, { state: 4 }], scarecrow: null })) fail("同狀態指紋應一致");
if (fd({ cells: [{ state: 2 }] }) === fd({ cells: [{ state: 2 }], scarecrow: [1, 1] })) fail("立稻草人應改變指紋");
if (typeof fd(null) !== "string" || typeof fd({}) !== "string") fail("fieldDigest 壞值應回字串不拋");
// 熟成進度跨 10% 檔位才改變指紋（進度條跨檔重建）、同檔位一致、成熟/空地 grow 不影響指紋
if (fd({ cells: [{ state: 3, grow: 20 }] }) === fd({ cells: [{ state: 3, grow: 35 }] })) fail("grow 跨檔（20→35）指紋應改變");
if (fd({ cells: [{ state: 3, grow: 31 }] }) !== fd({ cells: [{ state: 3, grow: 38 }] })) fail("grow 同檔（31/38 皆第3檔）指紋應一致");
if (fd({ cells: [{ state: 4, grow: 0 }] }) !== fd({ cells: [{ state: 4, grow: 50 }] })) fail("成熟格 grow 不該影響指紋");
// farmHudLabel：有田顯示塊數＋成熟株數、空陣列無字、壞值安全
if (T.farmHudLabel([]) !== "" || T.farmHudLabel(null) !== "") fail("無田應回空字串");
const hl = T.farmHudLabel([{ cells: [{ state: 4 }, { state: 4 }, { state: 2 }] }, { cells: [{ state: 1 }] }]);
if (!/農地 2/.test(hl) || !/2 株待收/.test(hl)) fail(`farmHudLabel 應數出 2 塊地 2 株待收，得「${hl}」`);
if (!/農地 1/.test(T.farmHudLabel([{ cells: [{ state: 1 }] }]))) fail("一塊無成熟的地仍應顯示塊數");
// 將熟（成長中 grow≥80）也數進 HUD，給「接近收成」的動能（ROADMAP 624）
const hl2 = T.farmHudLabel([{ cells: [{ state: 3, grow: 88 }, { state: 2, grow: 90 }, { state: 3, grow: 50 }, { state: 4 }] }]);
if (!/1 株待收/.test(hl2) || !/2 株將熟/.test(hl2)) fail(`farmHudLabel 應數出 1 株待收 2 株將熟，得「${hl2}」`);
if (/將熟/.test(T.farmHudLabel([{ cells: [{ state: 3, grow: 50 }] }]))) fail("grow<80 不該標將熟");
console.log("✅ cropCellVisual 階段遞增／發光／熟成進度條／指紋／farmHudLabel／壞值安全全綠");

// ── ①e 純邏輯真值表：wildlifeVisual／wildlifeStatusEmoji／wildlifeHudLabel（ROADMAP 615）──
if (typeof T.wildlifeVisual !== "function" || typeof T.wildlifeStatusEmoji !== "function" || typeof T.wildlifeHudLabel !== "function") fail("__bf3dTest 未暴露 wildlifeVisual/wildlifeStatusEmoji/wildlifeHudLabel");
const wlv = T.wildlifeVisual;
// 五種種類各對到正確身形＋顏色（鏡像 2D 配色）
if (wlv({ kind: "wild_bird" }).type !== "bird") fail("wild_bird 應為 bird 身形");
if (wlv({ kind: "wild_deer" }).type !== "quadruped") fail("wild_deer 應為 quadruped 身形");
if (wlv({ kind: "small_critter" }).type !== "critter") fail("small_critter 應為 critter 身形");
if (wlv({ kind: "wild_wolf" }).type !== "predator" || wlv({ kind: "wild_fox" }).type !== "predator") fail("狼／狐應為 predator 身形");
if (wlv({ kind: "wild_bird" }).color !== 0x87cefa) fail("野鳥應為天藍");
// 未知種類 / 壞值 → 安全綠盒後備，永不 throw
if (wlv({ kind: "???" }).type !== "box" || wlv(null).type !== "box" || wlv(undefined).type !== "box" || wlv("x").type !== "box") fail("未知種類／壞值應退回 box 後備");
// 幼獸縮一號（scale<1），成體 1.0；scale 一律夾在 [0.2,1.6]
if (!(wlv({ kind: "wild_deer", juvenile: true }).scale < 1)) fail("幼獸應縮小");
if (wlv({ kind: "wild_deer", scale: 1 }).scale !== 1) fail("成體 scale 應為 1");
if (wlv({ kind: "wild_deer", scale: 99 }).scale !== 1.6 || wlv({ kind: "wild_deer", scale: -5 }).scale !== 0.2) fail("scale 應夾在 [0.2,1.6]");
if (wlv({ kind: "wild_deer", scale: NaN }).scale !== 1.0) fail("scale 非有限應退回成體 1.0");
// 馴養／親近／歇息旗標忠實帶過
if (!wlv({ kind: "wild_deer", tamed: true }).tamed) fail("tamed 應帶過");
if (wlv({ kind: "wild_deer", familiarity: 0.8 }).familiarity !== 0.8) fail("familiarity 應帶過");
if (!wlv({ kind: "wild_deer", state: "resting" }).resting || wlv({ kind: "wild_deer", state: "wandering" }).resting) fail("resting 狀態判斷錯");
// 狀態 emoji 優先序：馴養 💛 ＞ 親近 💗 ＞ 幼獸 ✨ ＞ 歇息 💤 ＞ 無
if (T.wildlifeStatusEmoji({ kind: "wild_deer", tamed: true, juvenile: true }) !== "💛") fail("馴養應優先 💛");
if (T.wildlifeStatusEmoji({ kind: "wild_deer", familiarity: 0.7 }) !== "💗") fail("親近過半應 💗");
if (T.wildlifeStatusEmoji({ kind: "wild_deer", juvenile: true }) !== "✨") fail("幼獸應 ✨");
if (T.wildlifeStatusEmoji({ kind: "wild_deer", state: "resting" }) !== "💤") fail("歇息應 💤");
if (T.wildlifeStatusEmoji({ kind: "wild_deer" }) !== null) fail("平凡成體應無 emoji");
if (T.wildlifeStatusEmoji(null) !== null) fail("壞值狀態 emoji 應為 null");
// HUD 標籤：數隻數＋馴養數；空陣列空字串
const whl = T.wildlifeHudLabel([{ kind: "wild_deer", tamed: true }, { kind: "wild_bird" }, { kind: "wild_fox", tamed: true }]);
if (!/野生 3/.test(whl) || !/馴養 2/.test(whl)) fail(`wildlifeHudLabel 應數出 3 隻 2 馴養，得「${whl}」`);
if (T.wildlifeHudLabel([]) !== "" || T.wildlifeHudLabel(null) !== "") fail("空野生 HUD 應為空字串");
if (/馴養/.test(T.wildlifeHudLabel([{ kind: "wild_bird" }]))) fail("無馴養時不應顯示馴養數");
// 確定性
if (JSON.stringify(wlv({ kind: "wild_deer", scale: 0.6, tamed: true })) !== JSON.stringify(wlv({ kind: "wild_deer", scale: 0.6, tamed: true }))) fail("wildlifeVisual 非確定性");
console.log("✅ wildlifeVisual 身形／配色／幼獸縮放／馴養／狀態 emoji 優先序／HUD／壞值安全全綠");

// ── ①f 人造地標純邏輯：篝火／瞭望塔／雪人視覺＋HUD 標籤、壞值安全（ROADMAP 616）──
if (typeof T.campfireVisual !== "function" || typeof T.watchtowerVisual !== "function" || typeof T.snowmanVisual !== "function" || typeof T.structuresHudLabel !== "function") fail("__bf3dTest 未暴露 campfireVisual/watchtowerVisual/snowmanVisual/structuresHudLabel");
// 篝火：圍爐越多火越旺（單調遞增、夾上限 1.40），warmth_radius 帶過、將熄判定、壞值安全
const cfv = T.campfireVisual;
if (!(cfv({ gather_count: 1 }).blaze < cfv({ gather_count: 4 }).blaze)) fail("圍爐越多篝火應越旺");
if (cfv({ gather_count: 99 }).blaze > 1.4001) fail("篝火旺度應夾上限 1.40");
if (cfv({ warmth_radius: 120 }).warmthRadius !== 120) fail("warmth_radius 應帶過");
if (cfv({ warmth_radius: -5 }).warmthRadius !== 60) fail("壞 warmth_radius 應退回預設 60");
if (!cfv({ remaining_secs: 5 }).dying || cfv({ remaining_secs: 60 }).dying) fail("剩餘秒數低應判將熄");
if (cfv(null).blaze !== 1 || cfv("x").warmthRadius !== 60) fail("campfireVisual 壞值應安全");
// 瞭望塔：進度 0..1 夾、落成判定（done 或 progress 滿）、builders 帶過、壞值安全
const wtv = T.watchtowerVisual;
if (wtv({ progress: 50 }).progress !== 0.5) fail("塔進度 50 應為 0.5");
if (wtv({ progress: 999 }).progress !== 1 || wtv({ progress: -10 }).progress !== 0) fail("塔進度應夾在 [0,1]");
if (!wtv({ progress: 100 }).done || !wtv({ done: true, progress: 10 }).done) fail("滿進度或 done 旗標應判落成");
if (wtv({ progress: 40 }).done) fail("未滿且未標 done 不該算落成");
if (wtv({ builders: 3 }).builders !== 3) fail("builders 應帶過");
if (wtv(null).progress !== 0 || wtv("x").done !== false) fail("watchtowerVisual 壞值應安全");
// 雪人：style→圍巾色取模、表情、cheers 帶過、署名、壞值安全
const smv = T.snowmanVisual;
if (smv({ style: 1 }).scarf !== 0x3f7bd4) fail("style 1 應藍圍巾");
if (smv({ style: 4 }).scarf !== smv({ style: 0 }).scarf) fail("style 應對 4 取模");
if (smv({ style: -1 }).scarf !== smv({ style: 3 }).scarf) fail("負 style 取模應落合法款");
if (typeof smv({ style: 0 }).face !== "string" || !smv({ style: 0 }).face) fail("雪人應有表情");
if (smv({ cheers: 7 }).cheers !== 7 || smv({ cheers: -3 }).cheers !== 0) fail("cheers 應帶過且夾非負");
if (smv({ builder: "阿明" }).builder !== "阿明" || smv({}).builder !== "") fail("署名應帶過、缺時回空字串");
if (smv(null).cheers !== 0) fail("snowmanVisual 壞值應安全");
// HUD 標籤：分別計數＋只列有的、空時回空字串、壞值安全
const shl = T.structuresHudLabel([{ id: 1 }], [{ id: 1 }, { id: 2 }], [{ id: 1 }, { id: 2 }, { id: 3 }]);
if (!/🔥1/.test(shl) || !/🗼2/.test(shl) || !/⛄3/.test(shl)) fail(`structuresHudLabel 應數出 🔥1 🗼2 ⛄3，得「${shl}」`);
if (T.structuresHudLabel([], [], []) !== "" || T.structuresHudLabel(null, null, null) !== "") fail("無地標應回空字串");
if (/🗼/.test(T.structuresHudLabel([{ id: 1 }], [], []))) fail("無塔時不應顯示 🗼");
console.log("✅ campfireVisual 旺度遞增／瞭望塔進度落成／雪人圍巾取模／HUD 標籤／壞值安全全綠");

// ── ①g 世界樹群純邏輯：groveVisual 階段身形遞增／成樹遮蔭／groveHudLabel、壞值安全（ROADMAP 617）──
if (typeof T.groveVisual !== "function" || typeof T.groveHudLabel !== "function") fail("__bf3dTest 未暴露 groveVisual/groveHudLabel");
const gv = T.groveVisual;
// 階段夾 0..3；樹幹高隨階段單調遞增（嫩芽近乎無幹）；樹冠尺寸隨階段遞增
if (gv({ stage: 0 }).stage !== 0 || gv({ stage: 3 }).stage !== 3) fail("groveVisual stage 應原樣帶過");
if (gv({ stage: 9 }).stage !== 3 || gv({ stage: -2 }).stage !== 0) fail("groveVisual stage 應夾在 [0,3]");
if (!(gv({ stage: 0 }).trunkH < gv({ stage: 1 }).trunkH && gv({ stage: 1 }).trunkH < gv({ stage: 2 }).trunkH && gv({ stage: 2 }).trunkH < gv({ stage: 3 }).trunkH)) fail("樹幹高應隨階段單調遞增");
if (gv({ stage: 0 }).trunkH > 0.01) fail("嫩芽階段應近乎無樹幹");
if (!(gv({ stage: 1 }).crownScale < gv({ stage: 3 }).crownScale)) fail("樹冠尺寸應隨階段變闊");
// 身形旗標：只有嫩芽 sprout、只有幼樹 pine、只有成樹 shade/sway 在 stage>=2
if (!gv({ stage: 0 }).sprout || gv({ stage: 1 }).sprout || gv({ stage: 2 }).sprout) fail("只有嫩芽應 sprout");
if (gv({ stage: 1 }).pine || !gv({ stage: 2 }).pine || gv({ stage: 3 }).pine) fail("只有幼樹應 pine 松冠");
if (gv({ stage: 2 }).shade || !gv({ stage: 3 }).shade) fail("只有成樹應投樹蔭");
if (gv({ stage: 1 }).sway || !gv({ stage: 2 }).sway || !gv({ stage: 3 }).sway) fail("幼樹／成樹才隨風擺、幼苗不擺");
// 壞值安全：null/非物件/壞 stage → 退回嫩芽（stage 0），不爆
if (gv(null).stage !== 0 || gv("x").stage !== 0 || gv({ stage: NaN }).stage !== 0) fail("groveVisual 壞值應退回 stage 0");
// 確定性
if (JSON.stringify(gv({ stage: 2 })) !== JSON.stringify(gv({ stage: 2 }))) fail("groveVisual 非確定性");
// HUD 標籤：數出總棵數＋成樹數；無樹回空字串；只有未成樹時顯 🌱、有成樹顯 🌳
const ghl = T.groveHudLabel([{ stage: 0 }, { stage: 2 }, { stage: 3 }, { stage: 3 }]);
if (!/🌳 樹 4/.test(ghl) || !/成樹 2/.test(ghl)) fail(`groveHudLabel 應數出 4 棵其中成樹 2，得「${ghl}」`);
if (!/^🌱 樹 2$/.test(T.groveHudLabel([{ stage: 0 }, { stage: 1 }]))) fail("全未成樹應顯 🌱 不顯成樹數");
if (T.groveHudLabel([]) !== "" || T.groveHudLabel(null) !== "") fail("無樹應回空字串");
console.log("✅ groveVisual 階段身形遞增／嫩芽無幹／松冠成樹遮蔭／隨風擺旗標／HUD 標籤／壞值安全全綠");

// ── ①h 種樹互動純邏輯（ROADMAP 618）：wire 訊息固定、登入/訪客鈕態正確區隔 ──
if (typeof T.plantTreeWireMsg !== "function" || typeof T.plantButtonState !== "function") fail("__bf3dTest 未暴露 plantTreeWireMsg/plantButtonState");
const ptMsg = T.plantTreeWireMsg();
if (!ptMsg || ptMsg.type !== "plant_tree") fail("plantTreeWireMsg 應送 {type:'plant_tree'}（與 2D web/game.js 同協議），得 " + JSON.stringify(ptMsg));
const authedBtn = T.plantButtonState(true);
const guestBtn = T.plantButtonState(false);
if (authedBtn.locked !== false) fail("登入態種樹鈕不該鎖定");
if (guestBtn.locked !== true) fail("訪客態種樹鈕應鎖定（誠實：訪客送了不留痕跡）");
if (!/種樹/.test(authedBtn.label) || !/種樹/.test(guestBtn.label)) fail("種樹鈕標籤應含「種樹」字樣");
if (authedBtn.label === guestBtn.label) fail("登入/訪客鈕標籤應有區別（訪客標明需登入）");
if (!authedBtn.hint || !guestBtn.hint) fail("種樹鈕兩態都應有提示文案");
console.log("✅ 種樹互動 wire 訊息固定（plant_tree）／登入·訪客鈕態區隔／提示文案全綠");

// ── ①i 照料農地互動純邏輯（ROADMAP 619）：澆水／收成 wire 訊息固定、登入/訪客鈕態正確區隔 ──
if (typeof T.waterAllWireMsg !== "function" || typeof T.harvestAllWireMsg !== "function" || typeof T.tendButtonState !== "function") {
  fail("__bf3dTest 未暴露 waterAllWireMsg/harvestAllWireMsg/tendButtonState");
}
const waterMsg = T.waterAllWireMsg();
const harvestMsg = T.harvestAllWireMsg();
if (!waterMsg || waterMsg.type !== "water_all") fail("waterAllWireMsg 應送 {type:'water_all'}（與 2D 同協議），得 " + JSON.stringify(waterMsg));
if (!harvestMsg || harvestMsg.type !== "harvest_all") fail("harvestAllWireMsg 應送 {type:'harvest_all'}（與 2D 同協議），得 " + JSON.stringify(harvestMsg));
for (const kind of ["water", "harvest"]) {
  const onAuthed = T.tendButtonState(true, kind);
  const onGuest = T.tendButtonState(false, kind);
  if (onAuthed.locked !== false) fail(`登入態 ${kind} 鈕不該鎖定`);
  if (onGuest.locked !== true) fail(`訪客態 ${kind} 鈕應鎖定（沒地·送了不留痕跡）`);
  if (onAuthed.label === onGuest.label) fail(`登入/訪客 ${kind} 鈕標籤應有區別（訪客標明需登入）`);
  if (!onAuthed.hint || !onGuest.hint) fail(`${kind} 鈕兩態都應有提示文案`);
}
if (!/澆水/.test(T.tendButtonState(true, "water").label)) fail("澆水鈕標籤應含「澆水」字樣");
if (!/收成/.test(T.tendButtonState(true, "harvest").label)) fail("收成鈕標籤應含「收成」字樣");
// 未知 kind 保守降級不 throw、且仍是合法鈕態（預設走澆水路徑）
const tendUnknown = T.tendButtonState(true, "bogus");
if (!tendUnknown || typeof tendUnknown.label !== "string" || typeof tendUnknown.locked !== "boolean") fail("tendButtonState 未知 kind 應安全回合法鈕態，不 throw");
console.log("✅ 照料農地 wire 訊息固定（water_all／harvest_all）／登入·訪客鈕態區隔／未知 kind 安全降級全綠");

// ── ①i2 野營篝火互動純邏輯（ROADMAP 623）：wire 訊息固定、鈕態永不鎖定（訪客也能升火） ──
if (typeof T.campfireWireMsg !== "function" || typeof T.campfireButtonState !== "function") {
  fail("__bf3dTest 未暴露 campfireWireMsg/campfireButtonState");
}
const cfMsg = T.campfireWireMsg();
if (!cfMsg || cfMsg.type !== "light_campfire") fail("campfireWireMsg 應送 {type:'light_campfire'}（與 2D web/game.js 同協議），得 " + JSON.stringify(cfMsg));
const cfBtn = T.campfireButtonState();
if (!cfBtn || cfBtn.locked !== false) fail("篝火鈕應永不鎖定（升火不需登入，連線玩家即可）");
if (!/生火|篝火|🔥/.test(cfBtn.label)) fail("篝火鈕標籤應點明生火");
if (!cfBtn.hint) fail("篝火鈕應有提示文案");
console.log("✅ 野營篝火 wire 訊息固定（light_campfire）／鈕態永不鎖定·訪客也能升火全綠");

// ── ①i3 在 3D 裡採集純邏輯（ROADMAP 629）：wire 固定／目標判定（星晶優先·伸手範圍·壞值安全）／鈕態 ──
if (typeof T.gatherWireMsg !== "function" || typeof T.gatherStarCrystalWireMsg !== "function"
    || typeof T.gatherTargetAt !== "function" || typeof T.gatherButtonState !== "function") {
  fail("__bf3dTest 未暴露 gatherWireMsg/gatherStarCrystalWireMsg/gatherTargetAt/gatherButtonState");
}
// wire 訊息固定（與 2D 同協議、伺服器靠權威座標判距，故無座標）。
const gMsg = T.gatherWireMsg();
if (!gMsg || gMsg.type !== "gather") fail("gatherWireMsg 應送 {type:'gather'}，得 " + JSON.stringify(gMsg));
const gscMsg = T.gatherStarCrystalWireMsg();
if (!gscMsg || gscMsg.type !== "gather_star_crystal") fail("gatherStarCrystalWireMsg 應送 {type:'gather_star_crystal'}，得 " + JSON.stringify(gscMsg));
// 目標判定：腳邊一棵樹（圈內）→ 採到該節點。
const self0 = { x: 1000, y: 1000 };
const tgtNode = T.gatherTargetAt(self0, [{ kind: "tree", x: 1030, y: 1000 }], []);
if (!tgtNode || tgtNode.kind !== "node" || tgtNode.nodeKind !== "tree") fail("圈內樹節點應回 node/tree，得 " + JSON.stringify(tgtNode));
// 範圍外（>56px）→ 不採（回 null）。
const tgtFar = T.gatherTargetAt(self0, [{ kind: "rock", x: 1200, y: 1000 }], []);
if (tgtFar !== null) fail("範圍外節點不應可採，得 " + JSON.stringify(tgtFar));
// 星晶（夜間限定、80px）與普通節點同時在圈內 → 星晶優先。
const tgtPrefer = T.gatherTargetAt(self0, [{ kind: "rock", x: 1010, y: 1000 }], [{ x: 1060, y: 1000 }]);
if (!tgtPrefer || tgtPrefer.kind !== "crystal") fail("圈內星晶應優先於普通節點，得 " + JSON.stringify(tgtPrefer));
// 星晶在普通節點 56px 之外、但在星晶 80px 之內 → 仍採得到星晶。
const tgtCrystalOnly = T.gatherTargetAt(self0, [], [{ x: 1070, y: 1000 }]);
if (!tgtCrystalOnly || tgtCrystalOnly.kind !== "crystal") fail("星晶 80px 內應可採，得 " + JSON.stringify(tgtCrystalOnly));
// 壞座標／無自己座標／空快照 → 一律安全回 null（不誤點亮鈕）。
if (T.gatherTargetAt(null, [{ kind: "rock", x: 1000, y: 1000 }], []) !== null) fail("無自己座標應回 null");
if (T.gatherTargetAt({ x: NaN, y: 1 }, [{ kind: "rock", x: 1, y: 1 }], []) !== null) fail("壞自己座標應回 null");
if (T.gatherTargetAt(self0, [{ kind: "rock", x: NaN, y: 1000 }], []) !== null) fail("壞節點座標應安全跳過回 null");
if (T.gatherTargetAt(self0, null, null) !== null) fail("空快照應安全回 null");
// 鈕態：無目標 → 鎖定提示；各種類 → 對應字樣且不鎖定。
const gbNull = T.gatherButtonState(null);
if (!gbNull || gbNull.locked !== true || !gbNull.hint) fail("無目標時採集鈕應鎖定且有提示");
const gbCrystal = T.gatherButtonState({ kind: "crystal" });
if (!gbCrystal || gbCrystal.locked !== false || !/星晶|💎/.test(gbCrystal.label)) fail("星晶鈕態應點明採星晶且不鎖定");
const gbTree = T.gatherButtonState({ kind: "node", nodeKind: "tree" });
if (!gbTree || gbTree.locked !== false || !/伐木|🪓/.test(gbTree.label)) fail("樹鈕態應點明伐木");
const gbOre = T.gatherButtonState({ kind: "node", nodeKind: "ether_ore" });
if (!gbOre || !/乙太|🔮/.test(gbOre.label)) fail("乙太礦鈕態應點明採乙太");
const gbRock = T.gatherButtonState({ kind: "node", nodeKind: "rock" });
if (!gbRock || gbRock.locked !== false || !/採石|⛏️/.test(gbRock.label)) fail("礦脈鈕態應點明採石");
console.log("✅ 在 3D 採集 wire 固定（gather／gather_star_crystal）／目標判定·星晶優先·伸手範圍·壞值安全／鈕態種類字樣全綠");

// ── 城鎮交易純邏輯（ROADMAP 630）：商人挑選／走近判距／鈕態／面板簽章／物品標籤·壞值安全 ──
if (typeof T.shopMerchantsFrom !== "function" || typeof T.shopTargetAt !== "function"
    || typeof T.shopButtonState !== "function" || typeof T.shopPanelSig !== "function" || typeof T.itemLabel !== "function") {
  fail("__bf3dTest 未暴露 shopMerchantsFrom/shopTargetAt/shopButtonState/shopPanelSig/itemLabel");
}
// 商人挑選：只收「buy_list 或 sell_list 非空」者，缺座標／非物件／一般居民一律剔除。
const npcsForShop = [
  { id: "m1", name: "商人甲", x: 100, y: 100, sell_list: [{ item: "pickaxe", price_per: 15 }] }, // 只賣
  { id: "m2", name: "商人乙", x: 200, y: 100, buy_list: [{ item: "wood", price_per: 1 }] },       // 只收購
  { id: "r1", name: "居民", x: 150, y: 100 },                                                      // 非商人（無目錄）→ 剔除
  { id: "bad", name: "壞座標商人", x: NaN, y: 100, sell_list: [{ item: "axe", price_per: 8 }] },   // 壞座標 → 剔除
  null, "字串",                                                                                   // 非物件 → 安全跳過
];
const merchants = T.shopMerchantsFrom(npcsForShop);
if (merchants.length !== 2) fail("shopMerchantsFrom 應只收 2 名商人，得 " + JSON.stringify(merchants));
if (T.shopMerchantsFrom(null).length !== 0) fail("shopMerchantsFrom(null) 應回 []");
if (T.shopMerchantsFrom([{ id: "x", name: "空目錄", x: 0, y: 0, buy_list: [], sell_list: [] }]).length !== 0) fail("空目錄不算商人");
// 走近判距：圈內最近的商人；圈外回 null；壞值安全。
const nearShop = T.shopTargetAt({ x: 110, y: 100 }, merchants); // 距 m1=10、m2=90 → 最近 m1
if (!nearShop || nearShop.id !== "m1") fail("shopTargetAt 應回最近的商人 m1，得 " + JSON.stringify(nearShop));
if (T.shopTargetAt({ x: 1000, y: 1000 }, merchants) !== null) fail("圈外應回 null");
if (T.shopTargetAt(null, merchants) !== null) fail("無自己座標應回 null");
if (T.shopTargetAt({ x: NaN, y: 0 }, merchants) !== null) fail("壞自己座標應回 null");
if (T.shopTargetAt({ x: 0, y: 0 }, null) !== null) fail("空商人清單應回 null");
// 鈕態：無商人 → 鎖定提示；有商人 → 不鎖定且帶店名提示。
const sbNull = T.shopButtonState(null);
if (!sbNull || sbNull.locked !== true || !sbNull.hint) fail("無商人時交易鈕應鎖定且有提示");
const sbNear = T.shopButtonState({ name: "商人甲" });
if (!sbNear || sbNear.locked !== false || !/交易|🛒/.test(sbNear.label) || !/商人甲/.test(sbNear.hint)) fail("走近商人時交易鈕應不鎖定且提示帶店名");
// 面板簽章：行情／庫存／餘額／背包變動 → 簽章變；無變化 → 不變（驅動 DOM 只在必要時重建）。
const mSig = merchants[0];
const sigA = T.shopPanelSig(mSig, { ether: 50, inventory: [{ item: "wood", qty: 3 }] }, true);
const sigSame = T.shopPanelSig(mSig, { ether: 50, inventory: [{ item: "wood", qty: 3 }] }, true);
if (sigA !== sigSame) fail("同狀態 shopPanelSig 應一致");
const sigEther = T.shopPanelSig(mSig, { ether: 49, inventory: [{ item: "wood", qty: 3 }] }, true);
if (sigA === sigEther) fail("乙太變動 shopPanelSig 應改變");
const sigGuest = T.shopPanelSig(mSig, { ether: 50, inventory: [{ item: "wood", qty: 3 }] }, false);
if (sigA === sigGuest) fail("登入態變動 shopPanelSig 應改變");
if (T.shopPanelSig(null, null, false) !== "none") fail("無商人時簽章應為 none");
// 物品標籤：已知物品帶 emoji+中文名；未知物品退回原始字串＋📦 後備（留 i18n）。
if (!/木材/.test(T.itemLabel("wood"))) fail("itemLabel(wood) 應含中文名");
if (!/📦/.test(T.itemLabel("unknown_item_xyz"))) fail("未知物品應有 📦 後備");
console.log("✅ 城鎮交易純邏輯（商人挑選·壞值剔除／走近最近判距·圈外安全／鈕態·店名提示／面板簽章·必要才重建／物品標籤·i18n 後備）全綠");

// ── ①j 天時盤純邏輯（ROADMAP 620）：繞盤角度／倒數平滑遞減／時段·下一時段標籤／壞值安全 ──
if (typeof T.dayClockReadout !== "function" || typeof T.fmtCountdown !== "function") fail("__bf3dTest 未暴露 dayClockReadout/fmtCountdown");
const dcr = T.dayClockReadout;
// 繞盤角度：正午(0.5)在盤頂(~0°)、午夜(0)在底(180°)、破曉(0.25)在右(90°，東昇)、黃昏(0.75)在左(270°，西落)
const near = (a, b) => Math.abs(((a - b) % 360 + 540) % 360 - 180) < 0.5;
if (!near(dcr({ day_fraction: 0.5 }, 0).sunDeg, 0)) fail("正午太陽應在盤頂(0°)");
if (!near(dcr({ day_fraction: 0 }, 0).sunDeg, 180)) fail("午夜應在盤底(180°)");
if (!near(dcr({ day_fraction: 0.25 }, 0).sunDeg, 90)) fail("破曉應在右(90°，東昇)");
if (!near(dcr({ day_fraction: 0.75 }, 0).sunDeg, 270)) fail("黃昏應在左(270°，西落)");
// 角度恆落在 [0,360)
for (const f of [-0.3, 0, 0.1, 0.999, 1.4, NaN]) { const d = dcr({ day_fraction: f }, 0).sunDeg; if (!(d >= 0 && d < 360)) fail(`sunDeg 應落 [0,360)，f=${f} 得 ${d}`); }
// 倒數平滑遞減（錨點起算 elapsed），夾非負；day_fraction 隨 elapsed 平滑推進
const r0 = dcr({ phase: "day", day_fraction: 0.4, next_phase: "dusk", secs_to_next: 120, night_danger: false }, 0);
const r30 = dcr({ phase: "day", day_fraction: 0.4, next_phase: "dusk", secs_to_next: 120, night_danger: false }, 30);
if (r0.secsLeft !== 120) fail(`elapsed 0 倒數應為 120，得 ${r0.secsLeft}`);
if (r30.secsLeft !== 90) fail(`elapsed 30 倒數應遞減到 90，得 ${r30.secsLeft}`);
if (!(r30.frac > r0.frac)) fail("day_fraction 應隨 elapsed 平滑推進");
if (dcr({ secs_to_next: 10 }, 999).secsLeft !== 0) fail("倒數應夾非負（不變負數）");
// 缺 secs_to_next（舊伺服器）→ null＝不顯示倒數；缺 next_phase → 空字串
if (dcr({ phase: "day", day_fraction: 0.4 }, 0).secsLeft !== null) fail("缺 secs_to_next 倒數應回 null");
if (dcr({ phase: "day", day_fraction: 0.4 }, 0).nextLabel !== "") fail("缺 next_phase 下一時段標籤應回空字串");
// 時段標籤／夜間旗標／危機旗標
const rNight = dcr({ phase: "night", day_fraction: 0.85, next_phase: "dawn", secs_to_next: 40, night_danger: true }, 0);
if (rNight.isNight !== true || rNight.danger !== true) fail("夜間危機應 isNight＋danger 皆 true");
if (!/夜晚/.test(rNight.phaseLabel) || !/破曉/.test(rNight.nextLabel)) fail("夜間時段／下一時段標籤錯");
if (dcr({ phase: "day", day_fraction: 0.4 }, 0).isNight !== false) fail("白天 isNight 應為 false");
// 壞值安全：null dn / 非物件 / NaN elapsed → 不拋、回合法結構
for (const [dn, el, d] of [[null, 0, "null dn"], ["x", 5, "非物件 dn"], [{ day_fraction: NaN }, NaN, "NaN 全壞"]]) {
  const r = dcr(dn, el);
  if (!Number.isFinite(r.sunDeg) || !Number.isFinite(r.frac)) fail(`${d}：sunDeg/frac 應有限`);
  if (r.secsLeft !== null) fail(`${d}：缺欄位倒數應回 null`);
}
// 確定性：同輸入同輸出
if (JSON.stringify(dcr({ phase: "day", day_fraction: 0.3, secs_to_next: 50 }, 12)) !== JSON.stringify(dcr({ phase: "day", day_fraction: 0.3, secs_to_next: 50 }, 12))) fail("dayClockReadout 非確定性");
// fmtCountdown：m:ss 格式、補零、壞值空字串
if (T.fmtCountdown(200) !== "3:20") fail(`fmtCountdown(200) 應為 3:20，得 ${T.fmtCountdown(200)}`);
if (T.fmtCountdown(5) !== "0:05") fail(`fmtCountdown(5) 應補零為 0:05，得 ${T.fmtCountdown(5)}`);
if (T.fmtCountdown(0) !== "0:00") fail("fmtCountdown(0) 應為 0:00");
if (T.fmtCountdown(null) !== "" || T.fmtCountdown(-3) !== "" || T.fmtCountdown(NaN) !== "") fail("fmtCountdown 缺值/負值/壞值應回空字串");
console.log("✅ 天時盤 dayClockReadout 繞盤角度／倒數平滑遞減／時段·下一時段標籤／夜間危機旗標／壞值安全＋fmtCountdown 格式全綠");

// ── 表情輪純函式（ROADMAP 621）：白名單意圖／頭頂泡泡動畫／壞值安全 ──
if (typeof T.emoteWireMsg !== "function" || typeof T.emoteBubbleVisual !== "function" || !Array.isArray(T.EMOTE_CHOICES)) {
  fail("__bf3dTest 未暴露 emoteWireMsg/emoteBubbleVisual/EMOTE_CHOICES");
}
// EMOTE_CHOICES：8 顆、每顆 [wire, glyph, label] 皆非空
if (T.EMOTE_CHOICES.length !== 8) fail(`EMOTE_CHOICES 應有 8 顆，得 ${T.EMOTE_CHOICES.length}`);
for (const c of T.EMOTE_CHOICES) {
  if (!Array.isArray(c) || c.length !== 3 || !c[0] || !c[1] || !c[2]) fail(`EMOTE_CHOICES 元素格式錯：${JSON.stringify(c)}`);
}
// emoteWireMsg：白名單內回 {type:"emote",kind}，白名單外回 null（不送偽造表情）
const wm = T.emoteWireMsg("wave");
if (!wm || wm.type !== "emote" || wm.kind !== "wave") fail(`emoteWireMsg("wave") 應為 {type:emote,kind:wave}，得 ${JSON.stringify(wm)}`);
if (T.emoteWireMsg("explode") !== null) fail("emoteWireMsg 白名單外應回 null");
if (T.emoteWireMsg("WAVE") !== null) fail("emoteWireMsg 大小寫敏感，WAVE 應回 null");
if (T.emoteWireMsg("") !== null || T.emoteWireMsg(undefined) !== null) fail("emoteWireMsg 空值/缺值應回 null");
// emoteBubbleVisual：起手可見、會上浮、末段淡出、過期不可見、壞值安全
const ev0 = T.emoteBubbleVisual(0, 4);      // 剛開始
const evMid = T.emoteBubbleVisual(2000, 4); // 中段
const evLate = T.emoteBubbleVisual(3900, 4);// 末段（>70% 淡出區）
const evDone = T.emoteBubbleVisual(4000, 4);// 恰好到期
const evOver = T.emoteBubbleVisual(9999, 4);// 早已過期
if (!ev0.visible || !evMid.visible || !evLate.visible) fail("emoteBubbleVisual 存活期內應 visible");
if (evDone.visible || evOver.visible) fail("emoteBubbleVisual 到期/過期應不可見");
if (!(evMid.rise > ev0.rise)) fail("emoteBubbleVisual 泡泡應隨時間上浮（rise 遞增）");
if (!(evLate.opacity < 1) || !(evLate.opacity >= 0)) fail(`emoteBubbleVisual 末段應淡出於 [0,1)，得 ${evLate.opacity}`);
if (Math.abs(ev0.opacity - 1) > 1e-9) fail("emoteBubbleVisual 起手應全不透明");
// 確定性
if (JSON.stringify(T.emoteBubbleVisual(1234, 4)) !== JSON.stringify(T.emoteBubbleVisual(1234, 4))) fail("emoteBubbleVisual 非確定性");
// 壞值安全（NaN elapsed／非法 displaySecs 皆不可見、不 throw）
for (const bad of [NaN, -10, undefined, Infinity]) {
  const r = T.emoteBubbleVisual(bad, 4);
  if (r.visible) fail(`emoteBubbleVisual 壞 elapsed=${bad} 不應可見`);
}
const rBadDur = T.emoteBubbleVisual(100, 0); // displaySecs 非正 → 退回預設 4 秒，仍可見
if (!rBadDur.visible) fail("emoteBubbleVisual displaySecs 非正應退回預設、仍可見");
console.log("✅ 表情輪 emoteWireMsg 白名單意圖／emoteBubbleVisual 彈跳上浮淡出·確定性·壞值安全／EMOTE_CHOICES 對齊後端全綠");

// ── ①d 居民對話泡泡純邏輯：npcSpeechVisual 淡入停留淡出·確定性·壞值安全／speechTexture 壞值不拋（ROADMAP 622）──
if (typeof T.npcSpeechVisual !== "function" || typeof T.speechTexture !== "function") {
  fail("__bf3dTest 未暴露 npcSpeechVisual/speechTexture");
}
const sv0 = T.npcSpeechVisual(0, 8);        // 剛開始（淡入起點）
const svIn = T.npcSpeechVisual(600, 8);     // 淡入區間中（<12%＝960ms）
const svMid = T.npcSpeechVisual(4000, 8);   // 中段全亮
const svLate = T.npcSpeechVisual(7600, 8);  // 末段（>72% 淡出區）
const svDone = T.npcSpeechVisual(8000, 8);  // 恰好到期
const svOver = T.npcSpeechVisual(99999, 8); // 早已過期
if (!sv0.visible || !svIn.visible || !svMid.visible || !svLate.visible) fail("npcSpeechVisual 存活期內應 visible");
if (svDone.visible || svOver.visible) fail("npcSpeechVisual 到期/過期應不可見");
if (!(svIn.opacity > sv0.opacity)) fail("npcSpeechVisual 起手應淡入（opacity 遞增）");
if (Math.abs(svMid.opacity - 1) > 1e-9) fail(`npcSpeechVisual 中段應全亮，得 ${svMid.opacity}`);
if (!(svLate.opacity < 1) || !(svLate.opacity >= 0)) fail(`npcSpeechVisual 末段應淡出於 [0,1)，得 ${svLate.opacity}`);
// 確定性
if (JSON.stringify(T.npcSpeechVisual(2345, 8)) !== JSON.stringify(T.npcSpeechVisual(2345, 8))) fail("npcSpeechVisual 非確定性");
// 壞值安全（NaN elapsed／非法 displaySecs 皆不可見、不 throw）
for (const bad of [NaN, -10, undefined, Infinity]) {
  if (T.npcSpeechVisual(bad, 8).visible) fail(`npcSpeechVisual 壞 elapsed=${bad} 不應可見`);
}
if (!T.npcSpeechVisual(100, 0).visible) fail("npcSpeechVisual displaySecs 非正應退回預設、仍可見");
// speechTexture 對壞值不拋（null／超長對白都該安全產出貼圖）
try { T.speechTexture(null); T.speechTexture("這是一句非常非常長的對白用來測試截斷處理不會把泡泡撐爆也不會拋出例外喔喔喔"); }
catch (e) { fail("speechTexture 壞值拋例外：" + e); }
console.log("✅ 對話泡泡 npcSpeechVisual 淡入停留淡出·確定性·壞值安全／speechTexture 壞值不拋全綠");

// ── ①e 居民派系關係連線純邏輯（ROADMAP 625）：factionLinkVisual 配色脈動·factionArcPoints 弧形·
//        factionHudLabel 計數·確定性·壞值安全 ──
if (typeof T.factionLinkVisual !== "function" || typeof T.factionArcPoints !== "function" || typeof T.factionHudLabel !== "function") {
  fail("__bf3dTest 未暴露 factionLinkVisual/factionArcPoints/factionHudLabel");
}
// factionLinkVisual：已知 bond 回固定色＋[0,1] 透明度；未知 bond 回 null
const flAlly = T.factionLinkVisual("alliance", 1.0);
const flRival = T.factionLinkVisual("rivalry", 1.0);
if (!flAlly || flAlly.color !== T.FACTION_BOND_STYLE.alliance.color) fail("factionLinkVisual(alliance) 顏色應＝結盟色");
if (!flRival || flRival.color !== T.FACTION_BOND_STYLE.rivalry.color) fail("factionLinkVisual(rivalry) 顏色應＝敵對色");
for (const tt of [0, 0.7, 1.3, 5.0]) {
  for (const b of ["alliance", "rivalry"]) {
    const v = T.factionLinkVisual(b, tt);
    if (!(v.opacity >= 0 && v.opacity <= 1)) fail(`factionLinkVisual(${b},${tt}) 透明度應落在 [0,1]，得 ${v.opacity}`);
  }
}
if (T.factionLinkVisual("frenemy", 1.0) !== null) fail("factionLinkVisual 未知 bond 應回 null");
if (!(flRival.arc < flAlly.arc)) fail("敵對弧高應低於結盟弧高（緊繃 vs 舒展）");
if (JSON.stringify(T.factionLinkVisual("alliance", 2.5)) !== JSON.stringify(T.factionLinkVisual("alliance", 2.5))) fail("factionLinkVisual 非確定性");
// factionArcPoints：端點落在兩人位置、中點抬最高、弧高越大中點越高
const ptsAlly = T.factionArcPoints(0, 0, 10, 0, flAlly.arc, 8);
const ptsRival = T.factionArcPoints(0, 0, 10, 0, flRival.arc, 8);
if (ptsAlly.length !== (8 + 1) * 3) fail(`factionArcPoints 應回 (seg+1)*3 個分量，得 ${ptsAlly.length}`);
if (ptsAlly[0] !== 0 || ptsAlly[2] !== 0) fail("factionArcPoints 起點應＝第一位居民位置");
if (ptsAlly[ptsAlly.length - 3] !== 10 || ptsAlly[ptsAlly.length - 1] !== 0) fail("factionArcPoints 終點應＝第二位居民位置");
const midAlly = ptsAlly[Math.floor((8 / 2)) * 3 + 1]; // 中點 y
const endAlly = ptsAlly[1]; // 起點 y
if (!(midAlly > endAlly)) fail("factionArcPoints 中點 y 應高於端點 y（弧形隆起）");
const midRival = ptsRival[Math.floor((8 / 2)) * 3 + 1];
if (!(midAlly > midRival)) fail("結盟弧中點應高於敵對弧中點（弧高更大）");
if (JSON.stringify(T.factionArcPoints(1, 2, 3, 4, 5, 6)) !== JSON.stringify(T.factionArcPoints(1, 2, 3, 4, 5, 6))) fail("factionArcPoints 非確定性");
// factionHudLabel：數結盟／敵對組數；和平／空／未知 bond 的處理
if (T.factionHudLabel([]) !== "") fail("factionHudLabel 空陣列應回空字串");
if (T.factionHudLabel(null) !== "") fail("factionHudLabel null 應回空字串");
if (T.factionHudLabel([{ bond: "frenemy" }]) !== "") fail("factionHudLabel 全未知 bond 應回空字串");
const fhl = T.factionHudLabel([{ bond: "alliance" }, { bond: "alliance" }, { bond: "rivalry" }, { bond: "frenemy" }]);
if (!/🤝 2 結盟/.test(fhl) || !/⚔️ 1 敵對/.test(fhl)) fail(`factionHudLabel 應數出 🤝2 ⚔️1，得「${fhl}」`);
if (/敵對/.test(T.factionHudLabel([{ bond: "alliance" }]))) fail("factionHudLabel 無敵對時不應顯示敵對段");
if (/結盟/.test(T.factionHudLabel([{ bond: "rivalry" }]))) fail("factionHudLabel 無結盟時不應顯示結盟段");
console.log("✅ 派系關係連線 factionLinkVisual／factionArcPoints／factionHudLabel·確定性·壞值安全全綠");

// ── ①g 寵物夥伴呈現純邏輯（ROADMAP 627）：petVisual 種類/配色/羈絆夾值·petStatusEmoji 優先序·
//        petBondHearts 愛心條·petHudLabel 計數——確定性·壞值安全 ──
if (typeof T.petVisual !== "function" || typeof T.petStatusEmoji !== "function" || typeof T.petBondHearts !== "function" || typeof T.petHudLabel !== "function") {
  fail("__bf3dTest 未暴露 petVisual/petStatusEmoji/petBondHearts/petHudLabel");
}
const pv = T.petVisual;
// 無 pet_kind／壞值 → null（這位玩家沒寵物、不畫）
if (pv({ id: "me" }) !== null) fail("無 pet_kind 應回 null");
if (pv(null) !== null || pv("x") !== null || pv({ pet_kind: "" }) !== null) fail("壞值/空種類 petVisual 應回 null");
// 已知五種：型別/配色由對照表決定
const pvSprite = pv({ pet_kind: "flutter_sprite", pet_bond: 3 });
if (!pvSprite || pvSprite.type !== "sprite") fail("flutter_sprite 應為 sprite 身形");
if (pv({ pet_kind: "crystal_golem" }).type !== "crystal") fail("crystal_golem 應為 crystal");
if (pv({ pet_kind: "coral_crab" }).type !== "crab") fail("coral_crab 應為 crab");
if (pv({ pet_kind: "jade_wraith" }).type !== "wraith") fail("jade_wraith 應為 wraith");
if (pv({ pet_kind: "origin_guardian" }).type !== "guardian") fail("origin_guardian 應為 guardian");
// 未知種類 → box 後備（向後相容、永不空殼）
if (pv({ pet_kind: "mystery_pet" }).type !== "box") fail("未知種類應退回 box 後備");
// 羈絆夾在 [0,5]、壞值歸 0
if (pv({ pet_kind: "coral_crab", pet_bond: 9 }).bond !== 5) fail("羈絆應夾上限 5");
if (pv({ pet_kind: "coral_crab", pet_bond: -3 }).bond !== 0) fail("羈絆應夾下限 0");
if (pv({ pet_kind: "coral_crab", pet_bond: NaN }).bond !== 0) fail("壞羈絆應歸 0");
// 旗標讀取
if (!pv({ pet_kind: "coral_crab", pet_playing: true }).playing) fail("pet_playing 應讀成 playing");
if (!pv({ pet_kind: "coral_crab", pet_fetching: true }).fetching) fail("pet_fetching 應讀成 fetching");
// petStatusEmoji 優先序：玩耍 💞 ＞ 接物 🎾 ＞ 性格心情泡泡 ＞ 無
if (T.petStatusEmoji({ pet_kind: "coral_crab", pet_playing: true, pet_fetching: true, pet_personality: "clingy" }) !== "💞") fail("玩耍應優先 💞");
if (T.petStatusEmoji({ pet_kind: "coral_crab", pet_fetching: true, pet_personality: "clingy" }) !== "🎾") fail("接物應 🎾（蓋過性格）");
if (T.petStatusEmoji({ pet_kind: "coral_crab", pet_personality: "playful" }) !== "🎵") fail("活潑性格應 🎵");
if (T.petStatusEmoji({ pet_kind: "coral_crab", pet_personality: "lazy" }) !== "💤") fail("慵懶性格應 💤");
if (T.petStatusEmoji({ pet_kind: "coral_crab", pet_personality: "curious" }) !== "❓") fail("好奇性格應 ❓");
if (T.petStatusEmoji({ pet_kind: "coral_crab", pet_personality: "clingy" }) !== "💕") fail("黏人性格應 💕");
if (T.petStatusEmoji({ pet_kind: "coral_crab" }) !== null) fail("無玩耍/接物/性格應無 emoji");
if (T.petStatusEmoji(null) !== null) fail("壞值狀態 emoji 應為 null");
// petBondHearts：實心♥＝已養階、空心♡＝未滿；0 → 空字串
if (T.petBondHearts(0) !== "") fail("羈絆 0 應為空字串");
if (T.petBondHearts(2) !== "♥♥♡♡♡") fail(`羈絆 2 應為 ♥♥♡♡♡，得「${T.petBondHearts(2)}」`);
if (T.petBondHearts(5) !== "♥♥♥♥♥") fail("羈絆 5 應全滿");
if (T.petBondHearts(99) !== "♥♥♥♥♥") fail("羈絆超上限應夾成全滿");
// petHudLabel：數出視野內有寵物的玩家
const phl = T.petHudLabel([{ id: "a", pet_kind: "coral_crab" }, { id: "b" }, { id: "c", pet_kind: "jade_wraith" }]);
if (!/夥伴 2/.test(phl)) fail(`petHudLabel 應數出 2 隻夥伴，得「${phl}」`);
if (T.petHudLabel([{ id: "b" }]) !== "" || T.petHudLabel(null) !== "") fail("無寵物 HUD 應為空字串");
// 確定性
if (JSON.stringify(pv({ pet_kind: "coral_crab", pet_bond: 3 })) !== JSON.stringify(pv({ pet_kind: "coral_crab", pet_bond: 3 }))) fail("petVisual 非確定性");
console.log("✅ 寵物夥伴 petVisual 種類/配色/羈絆夾值／狀態 emoji 優先序／愛心條／HUD／壞值安全全綠");

// ── ①f 敵人威脅呈現純邏輯（ROADMAP 626）：enemyVisual 身形/配色/血量/兇名·enemyStatusEmoji 優先序·
//        enemyHpFill 比例與殘血·enemyHudLabel 計數——確定性·壞值安全 ──
if (typeof T.enemyVisual !== "function" || typeof T.enemyStatusEmoji !== "function" || typeof T.enemyHpFill !== "function" || typeof T.enemyHudLabel !== "function") {
  fail("__bf3dTest 未暴露 enemyVisual/enemyStatusEmoji/enemyHpFill/enemyHudLabel");
}
const ev = T.enemyVisual;
// 身形原型：機械 drone／靈體 wisp／巨像 golem；未知/壞值退回 drone 後備
if (ev({ kind: "scrap_drone" }).type !== "drone") fail("scrap_drone 應 drone 身形");
if (ev({ kind: "ether_wisp" }).type !== "wisp") fail("ether_wisp 應 wisp 身形");
if (ev({ kind: "crystal_golem" }).type !== "golem") fail("crystal_golem 應 golem 身形");
if (ev({ kind: "??" }).type !== "drone") fail("未知種類應退回 drone 後備");
if (ev(null).type !== "drone" || ev(undefined).type !== "drone") fail("壞值應安全後備 drone");
// 血量：受傷→damaged 畫血條；滿血/缺血量資訊→不畫；夾 [0,1]
if (ev({ kind: "scrap_drone", hp: 30, max_hp: 60 }).damaged !== true) fail("受傷應 damaged");
if (Math.abs(ev({ kind: "scrap_drone", hp: 30, max_hp: 60 }).hpRatio - 0.5) > 1e-9) fail("hpRatio 應＝hp/max");
if (ev({ kind: "scrap_drone", hp: 60, max_hp: 60 }).damaged !== false) fail("滿血不畫血條");
if (ev({ kind: "scrap_drone" }).damaged !== false || ev({ kind: "scrap_drone" }).hpRatio !== 1) fail("缺血量資訊應視為滿、不畫血條");
if (ev({ kind: "scrap_drone", hp: -5, max_hp: 60 }).hpRatio !== 0 || ev({ kind: "scrap_drone", hp: 999, max_hp: 60 }).hpRatio !== 1) fail("hpRatio 越界應夾 [0,1]");
// 兇名精英體型微大
if (!(ev({ kind: "crystal_golem", notorious: true }).scale > 1)) fail("兇名精英 scale 應 >1");
if (ev({ kind: "scrap_drone" }).scale !== 1) fail("非兇名 scale 應＝1");
// 狀態 emoji 優先序：破綻 ✨＞潰逃 💨＞夜歇 💤＞兇名 💢＞無
if (T.enemyStatusEmoji({ kind: "ether_wisp", weak: true, routing: true, resting: true, notorious: true }) !== "✨") fail("破綻應優先 ✨");
if (T.enemyStatusEmoji({ kind: "coral_crab", routing: true, resting: true, notorious: true }) !== "💨") fail("潰逃應 💨");
if (T.enemyStatusEmoji({ kind: "rune_guardian", resting: true, notorious: true }) !== "💤") fail("夜歇應 💤");
if (T.enemyStatusEmoji({ kind: "crystal_golem", notorious: true }) !== "💢") fail("兇名應 💢");
if (T.enemyStatusEmoji({ kind: "scrap_drone" }) !== null) fail("平凡敵人應無 emoji");
if (T.enemyStatusEmoji(null) !== null) fail("壞值狀態 emoji 應為 null");
// 血條填充：比例正比、≤30% critical、缺資訊視為滿、壞值安全、越界夾
const ehf = T.enemyHpFill;
if (ehf(30, 60).ratio !== 0.5 || ehf(30, 60).critical !== false) fail("enemyHpFill 50% 不殘血");
if (ehf(20, 60).critical !== false || ehf(18, 60).critical !== true) fail("enemyHpFill ≤30% 才 critical");
if (ehf(0, 60).ratio !== 0 || ehf(0, 60).critical !== true) fail("enemyHpFill 0 血應殘血");
if (ehf(5, 0).ratio !== 1 || ehf("x", 60).ratio !== 1 || ehf(NaN, NaN).ratio !== 1) fail("enemyHpFill 缺/壞資訊應視為滿");
if (ehf(120, 60).ratio !== 1) fail("enemyHpFill 越界應夾 1");
// HUD：只數活著的、兇名分開計、空/全死/壞值回空字串
const ehl = T.enemyHudLabel;
if (ehl([{ alive: true }, { alive: true, notorious: true }, { alive: false }]) !== "⚔️ 威脅 2 · 兇名 1") fail("enemyHudLabel 應只數活著、兇名分開計");
if (ehl([{ alive: true }, { alive: true }]) !== "⚔️ 威脅 2") fail("enemyHudLabel 無兇名不顯示兇名段");
if (ehl([]) !== "" || ehl([{ alive: false }]) !== "" || ehl(null) !== "") fail("enemyHudLabel 空/全死/壞值應回空字串");
// 確定性
if (JSON.stringify(ev({ kind: "ether_wisp", hp: 8, max_hp: 40, weak: true })) !== JSON.stringify(ev({ kind: "ether_wisp", hp: 8, max_hp: 40, weak: true }))) fail("enemyVisual 非確定性");
console.log("✅ enemyVisual 身形／配色／血量／兇名體型／狀態 emoji 優先序／血條／HUD／壞值安全全綠");

// ── ② 逐幀跑：先 welcome，再餵含各種內心生活的 NPC 快照，跑多幀抓例外 ──
function drive(msg) { lastWS.onmessage({ data: JSON.stringify(msg) }); }
function frames(n) { for (let i = 0; i < n; i++) { perfNow += 33; if (rafCb) { const cb = rafCb; rafCb = null; cb(); } } }

lastWS.onopen && lastWS.onopen();
drive({ type: "welcome", id: "me", world: { width: 6000, height: 6000 } });

const npcsA = [
  { id: "n1", name: "鐵匠", x: 3000, y: 3000, activity: "hammering" },
  { id: "n2", name: "里長", x: 3100, y: 3000, activity: "patrolling", thought: "今晚別出事就好" },
  { id: "n3", name: "獵手", x: 3200, y: 3000, alarmed: true },
  // 商人（ROADMAP 630）：帶 buy_list／sell_list（玩家走近可開店），且擺在「我」(3000,3000) 伸手範圍內
  // → 踩 shopMerchantsFrom 收錄＋走近點亮交易鈕＋（下方）開面板渲染賣/買兩區全路徑。
  { id: "n4", name: "商人", x: 3030, y: 3000, celebrating: true,
    buy_list: [{ item: "wood", price_per: 1, trend: "stable" }, { item: "stone", price_per: 2, trend: "down" }],
    sell_list: [{ item: "pickaxe", price_per: 15, trend: "stable", stock: 8, max_stock: 8 }, { item: "weapon", price_per: 40, trend: "stable", stock: 0, max_stock: 5 }] },
  { id: "n5", name: "農婦", x: 3400, y: 3000, needs_care: true, thought: "有點累了" },
  { id: "n6", name: "旅人", x: 3500, y: 3000 }, // 無任何內心生活欄位（其他 NPC）
];
// 一塊 3×2 的田：含種子／發芽／成熟／缺水各種格＋稻草人——踩 makeFieldPlot/rebuildFieldCrops
// 全路徑（ROADMAP 614）。owner === myId 走「自己的地」暖色底。
const fieldsA = [{
  owner: "me", origin_x: 2900, origin_y: 2900, tile_size: 48, cols: 3, rows: 2,
  reach: 48,
  cells: [
    { state: 0, dry: false }, { state: 1, dry: false }, { state: 2, dry: true, grow: 25 },
    { state: 3, dry: false, grow: 88 }, { state: 4, dry: false, quality: 2 }, { state: 4, dry: true },
  ],
  scarecrow: [1, 1],
}];
// 一群各種野生動物：鳥／鹿／小獸／狼／狐＋幼獸＋馴養＋歇息＋未知種類後備——踩 makeCreature
// 四型身形建構＋attachWildlifeStatus＋updateWildlifeStatus 全路徑（ROADMAP 615）。
const wildlifeA = [
  { id: 1, kind: "wild_bird", name: "野鳥", x: 2700, y: 3000, state: "wandering" },
  { id: 2, kind: "wild_deer", name: "野鹿", x: 2750, y: 3050, state: "wandering", familiarity: 0.3 },
  { id: 3, kind: "small_critter", name: "小獸", x: 2680, y: 2950, state: "resting" },
  { id: 4, kind: "wild_wolf", name: "野狼", x: 2600, y: 3100, state: "wandering" },
  { id: 5, kind: "wild_fox", name: "野狐", x: 2620, y: 3060, state: "wandering", tamed: true, familiarity: 1 },
  { id: 6, kind: "wild_deer", name: "小鹿", x: 2770, y: 3010, state: "wandering", juvenile: true, scale: 0.62 },
  { id: 7, kind: "mystery_beast", name: "謎獸", x: 2640, y: 2980, state: "wandering" }, // 未知種類 → 綠盒後備
];
// 人造地標：篝火（含圍爐／將熄）、瞭望塔（施工中／已落成）、雪人（各 style／含愛心）——
// 踩 makeCampfire/makeWatchtower/makeSnowman 建構＋applyTowerProgress＋updateStructures 全路徑（ROADMAP 616）。
const campfiresA = [
  { id: 1, wx: 3050, wy: 2900, remaining_secs: 120, gather_count: 3, warmth_radius: 140 }, // 圍爐 3 人、火旺
  { id: 2, wx: 2950, wy: 3100, remaining_secs: 6, gather_count: 0, warmth_radius: 60 },    // 將熄（剩 6 秒）
];
const watchtowersA = [
  { id: 1, wx: 3150, wy: 2850, progress: 40, builders: 2, done: false }, // 施工中
  { id: 2, wx: 2850, wy: 3150, progress: 100, builders: 0, done: true }, // 已落成（入夜亮燈）
];
const snowmenA = [
  { id: 1, wx: 3050, wy: 3150, builder: "阿明", style: 0, cheers: 0 },
  { id: 2, wx: 2950, wy: 2850, builder: "小華", style: 2, cheers: 5 }, // 有愛心
  { id: 3, wx: 3000, wy: 2800, builder: "旅人", style: 7, cheers: 0 }, // style 取模
];
// 世界樹群：各階段一棵（嫩芽／幼苗／幼樹松／成樹遮蔭）＋壞 stage 後備——踩 makeGroveTree 建構
// ＋applyGroveStage 各身形＋updateGroves 隨風擺全路徑（ROADMAP 617）。
const grovesA = [
  { x: 3050, y: 2950, stage: 0 }, // 嫩芽（無幹小葉）
  { x: 2980, y: 3050, stage: 1 }, // 幼苗（圓冠）
  { x: 3120, y: 3020, stage: 2 }, // 幼樹（松冠）
  { x: 2920, y: 2980, stage: 3 }, // 成樹（投樹蔭）
  { x: 3080, y: 2880, stage: 9 }, // 壞 stage → 夾成成樹
];
// 鎮民派系（ROADMAP 625）：n1↔n2 結盟、n3↔n4 敵對（兩位都在視野 → 應畫出連線）、
// n2↔n9 引用不存在的居民（應安全跳過）、n5↔n6 未知 bond（應略過、不畫）——踩 updateFactionLinks
// 的建線／改色／脈動／缺一方跳過／未知 bond 跳過全路徑。
const townFactionsA = [
  { npc_a: "n1", npc_b: "n2", npc_a_name: "鐵匠", npc_b_name: "里長", bond: "alliance", affinity: 82 },
  { npc_a: "n3", npc_b: "n4", npc_a_name: "獵手", npc_b_name: "商人", bond: "rivalry", affinity: 18 },
  { npc_a: "n2", npc_b: "n9", npc_a_name: "里長", npc_b_name: "幽靈", bond: "alliance", affinity: 60 }, // n9 不在 npcs → 跳過
  { npc_a: "n5", npc_b: "n6", npc_a_name: "農婦", npc_b_name: "旅人", bond: "frenemy", affinity: 50 },   // 未知 bond → 略過
];
// 一群各種敵人：機械／靈體／巨像三型＋受傷（畫血條）／殘血（深紅）／滿血（不畫）／兇名（體型大 💢）／
// 夜歇 💤／潰逃 💨／破綻 ✨／未知種類後備／缺血量資訊／alive=false（應過濾）——踩 makeEnemy 三型身形
// 建構＋attachEnemyStatus＋updateEnemyStatus 全路徑（ROADMAP 626）。
const enemiesA = [
  { eid: "e1", kind: "scrap_drone", x: 3060, y: 3050, level: 3, hp: 30, max_hp: 60, alive: true },                   // drone、受傷 50% → 紅血條
  { eid: "e2", kind: "ether_wisp", x: 3080, y: 2960, level: 4, hp: 8, max_hp: 40, alive: true, weak: true },          // wisp、殘血<30% → 深紅＋破綻 ✨
  { eid: "e3", kind: "crystal_golem", x: 2940, y: 3080, level: 9, hp: 200, max_hp: 200, alive: true, notorious: true },// golem、滿血(無血條)、兇名 💢、體型大
  { eid: "e4", kind: "rune_guardian", x: 2960, y: 2940, level: 5, hp: 50, max_hp: 80, alive: true, resting: true },    // 夜歇 💤
  { eid: "e5", kind: "coral_crab", x: 3120, y: 3000, level: 6, hp: 70, max_hp: 100, alive: true, routing: true },      // 潰逃 💨
  { eid: "e6", kind: "mystery_kind", x: 3000, y: 3120, level: 2, hp: 10, max_hp: 20, alive: true },                    // 未知種類 → drone 紅盒後備
  { eid: "e7", kind: "void_phantom", x: 2900, y: 3000, alive: true },                                                   // 缺 hp/max_hp/level → 視為滿、不畫血條、安全
  { eid: "e8", kind: "steam_construct", x: 9999, y: 9999, level: 7, hp: 5, max_hp: 120, alive: false },                 // alive=false → 過濾掉
];
// 寵物夥伴（ROADMAP 627）：五種寵物各一隻＋玩耍／接物／性格／羈絆／未知種類後備／缺座標過濾——
// 踩 makePet 五型身形建構＋attachPetStatus＋updatePetStatus（待機浮動/玩耍蹦跳/狀態 emoji/愛心條）全路徑。
const playersA = [
  { id: "me", name: "我", x: 3000, y: 3000, pet_kind: "flutter_sprite", pet_x: 3012, pet_y: 3000, pet_playing: true, pet_bond: 3, pet_personality: "clingy" }, // 精靈、玩耍中💞、羈絆3
  { id: "p2", name: "夥伴", x: 3050, y: 3000, pet_kind: "coral_crab", pet_x: 3062, pet_y: 3000, pet_fetching: true, pet_toy_x: 3070, pet_toy_y: 3000, pet_bond: 0 }, // 蟹、接物中🎾、無羈絆（不畫愛心條）
  { id: "p3", name: "晶友", x: 3100, y: 3000, pet_kind: "crystal_golem", pet_x: 3112, pet_y: 3000, pet_personality: "lazy", pet_bond: 5 }, // 晶石、慵懶💤、羈絆滿
  { id: "p4", name: "幽客", x: 2950, y: 3000, pet_kind: "jade_wraith", pet_x: 2938, pet_y: 3000, pet_personality: "curious", pet_bond: 2 }, // 幽靈、好奇❓
  { id: "p5", name: "守者", x: 2900, y: 3050, pet_kind: "origin_guardian", pet_x: 2912, pet_y: 3050, pet_bond: 4 }, // 守護星、無狀態 emoji、羈絆4
  { id: "p6", name: "謎友", x: 3150, y: 3050, pet_kind: "mystery_pet", pet_x: 3162, pet_y: 3050, pet_bond: 1 }, // 未知種類 → box 後備
  { id: "p7", name: "無座標", x: 3050, y: 3100, pet_kind: "coral_crab", pet_bond: 1 }, // 缺 pet_x/pet_y → 過濾、不畫寵物
];
drive({ type: "snapshot", players: playersA, npcs: npcsA, wildlife: wildlifeA, enemies: enemiesA, nodes: [],
  town_factions: townFactionsA,
  // 夜採星晶礦脈（ROADMAP 629）：餵兩道晶脈，踩 makeStarCrystal + reconcile + 靜態 AOI 淡入淡出路徑。
  star_crystals: [{ x: 1500, y: 1500 }, { x: 1560, y: 1520 }],
  fields: fieldsA, campfires: campfiresA, watchtowers: watchtowersA, snowmen: snowmenA, world_groves: grovesA,
  daynight: { phase: "day", day_fraction: 0.33, light: 1.0, night_danger: false, next_phase: "dusk", secs_to_next: 180 },
  // 細雨＋橫風＋彩虹：踩 applyWeather 的粒子推進／回收、霧染、彩虹淡入路徑（ROADMAP 613）
  weather: { weather_type: "grassland_rain", intensity: 0.9, wind: { dir_x: 0.8, dir_y: 0.6, strength: 0.7 }, fish_phase: 0 },
  rainbow: { active: true, remaining_secs: 30 } });
frames(8);

// 城鎮交易面板（ROADMAP 630）：「我」就站在商人 n4 伸手範圍內 → 開面板，踩 renderShopPanel／refreshShopPanel
// 的賣給商人（含 ↘供給過剩）／向商人買（含 🚫售罄）／訪客鎖定登入提示渲染全路徑（不該拋例外）。
if (typeof sandbox.__bf3dToggleShop === "function") {
  sandbox.__bf3dToggleShop();   // 走近商人 → 開店
  frames(2);
  sandbox.__bf3dToggleShop();   // 再按一次 → 收起
  frames(1);
}

// 玩家表情（ROADMAP 621）：自己與夥伴各比一個表情＋一筆未知 from_id（已離開的玩家）——
// 踩 player_emote 事件處理＋attachEmoteBubble＋updatePlayerEmotes 的點亮／上浮／淡出／自清路徑。
drive({ type: "player_emote", from_id: "me", from_name: "我", glyph: "👋", wx: 3000, wy: 3000, display_secs: 4 });
drive({ type: "player_emote", from_id: "p2", from_name: "夥伴", glyph: "🎉", wx: 3050, wy: 3000, display_secs: 4 });
drive({ type: "player_emote", from_id: "ghost", from_name: "幽靈", glyph: "❤️", wx: 9999, wy: 9999, display_secs: 4 }); // from_id 不在 players → 應安全跳過
frames(6);   // 彈跳＋上浮階段
frames(130); // 跑過 4 秒顯示窗 → 踩泡泡淡出與過期自清路徑（reduceMotion 與一般路徑都不該 throw）

// 居民對話泡泡（ROADMAP 622）：在視野內居民（n1 鐵匠）、有內心話的居民（n2 里長，驗對話蓋過思想）、
// 不在視野的說話者（ghostNpc）、缺欄位（n5 無 text）各送一筆——踩 npc_speech 事件處理＋attachResidentStatus
// 的對話 sprite＋updateNpcSpeech 的淡入／停留／淡出／過期自清＋思想讓位路徑。
drive({ type: "npc_speech", npc_id: "n1", npc_name: "鐵匠", text: "今天的鐵礦不錯！", display_secs: 8, wx: 3000, wy: 3000 });
drive({ type: "npc_speech", npc_id: "n2", npc_name: "里長", text: "大家辛苦了", display_secs: 8, wx: 3100, wy: 3000 }); // 此居民有 thought → 對話期間思想應讓位
drive({ type: "npc_speech", npc_id: "ghostNpc", npc_name: "幽靈居民", text: "在視野外說話", display_secs: 8, wx: 9999, wy: 9999 }); // 不在 npcs → 安全跳過
drive({ type: "npc_speech", npc_id: "n5", npc_name: "農婦", display_secs: 8, wx: 3400, wy: 3000 }); // 缺 text → 安全降級空字串
frames(4);   // 淡入＋停留階段
frames(280); // 跑過 8 秒顯示窗 → 踩對話泡泡淡出與過期自清路徑

// 狀態轉移：活動結束（→null）、思想消失、危機解除轉歡慶、需求被撫平——踩 setSpriteEmoji 換貼圖/熄滅路徑
const npcsB = [
  { id: "n1", name: "鐵匠", x: 3000, y: 3000 }, // 活動結束
  { id: "n2", name: "里長", x: 3100, y: 3000, activity: "patrolling" }, // 思想消失
  { id: "n3", name: "獵手", x: 3200, y: 3000, celebrating: true }, // 危機→歡慶
  { id: "n5", name: "農婦", x: 3400, y: 3000 }, // 需求撫平
  // n4、n6 從快照消失 → 走 AOI 淡出移除
];
// 田長大了（種子→發芽、發芽→成熟、稻草人移位）：digest 變更 → 踩 rebuildFieldCrops 重建作物層（ROADMAP 614）
const fieldsB = [{
  owner: "me", origin_x: 2900, origin_y: 2900, tile_size: 48, cols: 3, rows: 2,
  reach: 48,
  cells: [
    { state: 1, dry: false }, { state: 2, dry: false }, { state: 3, dry: false },
    { state: 4, dry: false, quality: 2 }, { state: 4, dry: false }, { state: 1, dry: false },
  ],
  scarecrow: [0, 0],
}];
// 同時把日夜推進到「夜間危機」：踩 applyDayNight 的天色／太陽流轉 + 危機紅化路徑
// 野生動物轉移：鹿被馴養（familiarity→1+tamed＝狀態 emoji 改、愛心脈動）、幼獸長大（scale↑）、
// 歇息小獸醒來移動、野狼從快照消失（AOI 淡出移除）——踩 updateWildlifeStatus 換貼圖／縮放平滑路徑。
const wildlifeB = [
  { id: 1, kind: "wild_bird", name: "野鳥", x: 2710, y: 3010, state: "wandering" },
  { id: 2, kind: "wild_deer", name: "野鹿", x: 2755, y: 3055, state: "wandering", tamed: true, familiarity: 1 },
  { id: 3, kind: "small_critter", name: "小獸", x: 2690, y: 2960, state: "wandering" }, // 醒來走動
  { id: 5, kind: "wild_fox", name: "野狐", x: 2625, y: 3065, state: "resting", tamed: true, familiarity: 1 },
  { id: 6, kind: "wild_deer", name: "小鹿", x: 2775, y: 3015, state: "wandering", juvenile: false, scale: 1.0 }, // 長成
  // id 4（野狼）、7（謎獸）從快照消失 → 走 AOI 淡出移除
];
// 地標轉移：圍爐塔升起（40→80%）、剛落成的塔入夜亮燈、一座篝火熄滅（消失→淡出）、雪人被讚（cheers↑）——
// 入夜後踩 updateStructures 的塔頂燈亮起、暖意圈更亮、AOI 淡出路徑（ROADMAP 616）。
const campfiresB = [
  { id: 1, wx: 3050, wy: 2900, remaining_secs: 90, gather_count: 5, warmth_radius: 180 }, // 更多人圍爐
  // id 2 熄滅 → 從快照消失 → AOI 淡出移除
];
const watchtowersB = [
  { id: 1, wx: 3150, wy: 2850, progress: 80, builders: 3, done: false }, // 升到 80%
  { id: 2, wx: 2850, wy: 3150, progress: 100, builders: 0, done: true }, // 落成的塔入夜亮燈
];
const snowmenB = [
  { id: 1, wx: 3050, wy: 3150, builder: "阿明", style: 0, cheers: 3 }, // 被讚了
  { id: 2, wx: 2950, wy: 2850, builder: "小華", style: 2, cheers: 5 },
  // id 3 融化 → 從快照消失 → AOI 淡出移除
];
// 樹長大了（嫩芽→幼苗、幼樹→成樹）：stage 變動 → 踩 applyGroveStage 重塑樹身路徑；一棵被砍/消失走 AOI 淡出。
const grovesB = [
  { x: 3050, y: 2950, stage: 1 }, // 嫩芽長成幼苗
  { x: 2980, y: 3050, stage: 1 }, // 不變
  { x: 3120, y: 3020, stage: 3 }, // 幼樹長成成樹（投樹蔭）
  { x: 2920, y: 2980, stage: 3 }, // 不變
  // x3080,y2880 那棵從快照消失 → AOI 淡出移除
];
// 敵人第二份（ROADMAP 626）：e1 回滿血（血條應消失）、e3 兇名受傷（畫血條）、e9 新生瀕死（深紅）；
// 其餘 e2/e4~e7 從快照消失 → 踩 AOI 淡出移除路徑。
const enemiesB = [
  { eid: "e1", kind: "scrap_drone", x: 3060, y: 3050, level: 3, hp: 60, max_hp: 60, alive: true },                     // 回滿 → 血條消失
  { eid: "e3", kind: "crystal_golem", x: 2940, y: 3080, level: 9, hp: 120, max_hp: 200, alive: true, notorious: true },// 兇名受傷 → 畫血條
  { eid: "e9", kind: "jade_wraith", x: 3030, y: 2980, level: 11, hp: 1, max_hp: 90, alive: true },                     // 新生、瀕死深紅
];
// 寵物轉移（ROADMAP 627）：me 換了寵物種類（精靈→守護星：舊 key 淡出、新身形淡入）＋玩耍轉接物、
// p2 收起玩具改歇腳（接物 emoji 熄滅）＋養出羈絆（愛心條從無到有）；p3~p7 從快照消失 → 寵物走 AOI 淡出。
const playersB = [
  { id: "me", name: "我", x: 3000, y: 3000, pet_kind: "origin_guardian", pet_x: 3012, pet_y: 3000, pet_fetching: true, pet_toy_x: 3020, pet_toy_y: 3000, pet_bond: 4 }, // 換成守護星、改接物
  { id: "p2", name: "夥伴", x: 3050, y: 3000, pet_kind: "coral_crab", pet_x: 3056, pet_y: 3000, pet_personality: "playful", pet_bond: 2 }, // 收玩具歇腳→性格🎵、養出羈絆2
];
drive({ type: "snapshot", players: playersB, npcs: npcsB, wildlife: wildlifeB, enemies: enemiesB, nodes: [],
  town_factions: [], // 派系全數解除（ROADMAP 625）→ 踩 updateFactionLinks 的連線回收／dispose 全清路徑
  star_crystals: [], // 星晶全數消失（天亮／採光）→ 踩 starCrystals reconcile 的移除淡出全清路徑（ROADMAP 629）
  fields: fieldsB, campfires: campfiresB, watchtowers: watchtowersB, snowmen: snowmenB, world_groves: grovesB,
  daynight: { phase: "night", day_fraction: 0.82, light: 0.2, night_danger: true, next_phase: "dawn", secs_to_next: 60 },
  // 天氣切到海霧（上飄粒子）＋彩虹消失：踩 fall<0 上飄回收、霧染轉色、彩虹淡出路徑
  weather: { weather_type: "water_sea_mist", intensity: 0.7, wind: { dir_x: -0.5, dir_y: 0.3, strength: 0.4 }, fish_phase: 1 },
  rainbow: { active: false, remaining_secs: 0 } });
frames(12);

// 照料回饋 chat 路徑（ROADMAP 619）：系統單播與一般聊天都不該讓 onmessage 拋例外（窗外/非系統靜默忽略）。
drive({ type: "chat", from: "系統", text: "💧 一鍵澆水：替 3 株缺水作物補滿了水！" });
drive({ type: "chat", from: "某玩家", text: "嗨大家" });
frames(2);

if (caught.length) {
  console.error("❌ 跑了多幀後攔到 " + caught.length + " 筆繪製例外/警告：");
  for (const c of caught.slice(0, 10)) console.error("   · " + c);
  process.exit(1);
}
console.log("✅ NPC 內心生活（活動／思想／關懷／危機／歡慶＋狀態轉移＋AOI 淡出）＋野生動物（五種身形／幼獸縮放／馴養脈動／轉移）＋人造地標（篝火圍爐／將熄／塔施工→落成入夜亮燈／雪人讚賞＋AOI 淡出）＋世界樹群（嫩芽→幼苗→幼樹松→成樹遮蔭、長大重塑＋AOI 淡出）＋玩家表情泡泡（揮手／歡呼點亮→彈跳上浮→4 秒淡出自清＋未知 from_id 安全跳過）＋居民對話泡泡（說出口的話淡入停留→8 秒淡出自清＋對話蓋過思想＋缺欄位降級＋視野外說話者安全跳過）＋寵物夥伴（五種低多邊形身形／玩耍蹦跳/接物/性格心情泡泡/羈絆愛心條／換寵物舊身形淡出新身形淡入／離開視野 AOI 淡出／未知種類 box 後備／缺座標安全過濾）跑多幀零例外");
console.log("✅ render-smoke-3d 全綠");
