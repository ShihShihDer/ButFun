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
// fieldDigest：cells 變了指紋就變、沒變就同（重建作物層的依據）、壞值安全
const fd = T.fieldDigest;
const fA = { cells: [{ state: 2 }, { state: 4 }], scarecrow: null };
const fB = { cells: [{ state: 3 }, { state: 4 }], scarecrow: null }; // 第一格長大了
if (fd(fA) === fd(fB)) fail("作物階段變了，指紋應改變");
if (fd(fA) !== fd({ cells: [{ state: 2 }, { state: 4 }], scarecrow: null })) fail("同狀態指紋應一致");
if (fd({ cells: [{ state: 2 }] }) === fd({ cells: [{ state: 2 }], scarecrow: [1, 1] })) fail("立稻草人應改變指紋");
if (typeof fd(null) !== "string" || typeof fd({}) !== "string") fail("fieldDigest 壞值應回字串不拋");
// farmHudLabel：有田顯示塊數＋成熟株數、空陣列無字、壞值安全
if (T.farmHudLabel([]) !== "" || T.farmHudLabel(null) !== "") fail("無田應回空字串");
const hl = T.farmHudLabel([{ cells: [{ state: 4 }, { state: 4 }, { state: 2 }] }, { cells: [{ state: 1 }] }]);
if (!/農地 2/.test(hl) || !/2 株待收/.test(hl)) fail(`farmHudLabel 應數出 2 塊地 2 株待收，得「${hl}」`);
if (!/農地 1/.test(T.farmHudLabel([{ cells: [{ state: 1 }] }]))) fail("一塊無成熟的地仍應顯示塊數");
console.log("✅ cropCellVisual 階段遞增／發光／指紋／farmHudLabel／壞值安全全綠");

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
// 一塊 3×2 的田：含種子／發芽／成熟／缺水各種格＋稻草人——踩 makeFieldPlot/rebuildFieldCrops
// 全路徑（ROADMAP 614）。owner === myId 走「自己的地」暖色底。
const fieldsA = [{
  owner: "me", origin_x: 2900, origin_y: 2900, tile_size: 48, cols: 3, rows: 2,
  reach: 48,
  cells: [
    { state: 0, dry: false }, { state: 1, dry: false }, { state: 2, dry: true },
    { state: 3, dry: false }, { state: 4, dry: false, quality: 2 }, { state: 4, dry: true },
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
drive({ type: "snapshot", players: [{ id: "me", name: "我", x: 3000, y: 3000 }], npcs: npcsA, wildlife: wildlifeA, enemies: [], nodes: [],
  fields: fieldsA, campfires: campfiresA, watchtowers: watchtowersA, snowmen: snowmenA, world_groves: grovesA,
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
drive({ type: "snapshot", players: [{ id: "me", name: "我", x: 3000, y: 3000 }], npcs: npcsB, wildlife: wildlifeB, enemies: [], nodes: [],
  fields: fieldsB, campfires: campfiresB, watchtowers: watchtowersB, snowmen: snowmenB, world_groves: grovesB,
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
console.log("✅ NPC 內心生活（活動／思想／關懷／危機／歡慶＋狀態轉移＋AOI 淡出）＋野生動物（五種身形／幼獸縮放／馴養脈動／轉移）＋人造地標（篝火圍爐／將熄／塔施工→落成入夜亮燈／雪人讚賞＋AOI 淡出）＋世界樹群（嫩芽→幼苗→幼樹松→成樹遮蔭、長大重塑＋AOI 淡出）跑 20 幀零例外");
console.log("✅ render-smoke-3d 全綠");
