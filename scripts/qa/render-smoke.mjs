// Render-smoke：用 Proxy 假 DOM / 假 canvas / 假 WebSocket 載入「真正的」web/game.js，
// 餵一份真實城鎮 snapshot，實際跑 render() 迴圈數幀，抓任何在繪製中拋出的例外
// ——這正是「進城人物突然不見」的根因型態（render 一拋就停 rAF 迴圈→畫面凍結）。
// 純 JS 例外(TypeError/RangeError…)在 Node 照樣會丟,不需要真 canvas。
// 用法：node scripts/qa/render-smoke.mjs [snapshotJsonPath]
import { readFileSync } from "fs";
import vm from "vm";

// 預設用 repo 內固定 fixture（自帶資料、可重現、不需先連線）；也可傳入即時撈的 snapshot 路徑。
const SNAP_PATH = process.argv[2] || new URL("./fixtures/town-snapshot.json", import.meta.url).pathname;
const snapshot = JSON.parse(readFileSync(SNAP_PATH, "utf8"));
const gameSrc = readFileSync(new URL("../../web/game.js", import.meta.url), "utf8");

// ── 假 canvas 2d context：全部繪製方法 no-op，回傳值方法給合理 stub ──────────────
function makeCtx(canvasEl) {
  const noop = () => {};
  const store = {};
  const base = {
    canvas: canvasEl,
    save: noop, restore: noop, beginPath: noop, closePath: noop, moveTo: noop, lineTo: noop,
    arc: noop, arcTo: noop, rect: noop, roundRect: noop, ellipse: noop, fill: noop, stroke: noop,
    fillRect: noop, strokeRect: noop, clearRect: noop, clip: noop, translate: noop, rotate: noop,
    scale: noop, transform: noop, setTransform: noop, resetTransform: noop, fillText: noop,
    strokeText: noop, drawImage: noop, putImageData: noop, setLineDash: noop, getLineDash: () => [],
    quadraticCurveTo: noop, bezierCurveTo: noop, createImageData: () => ({ data: new Uint8ClampedArray(4) }),
    getImageData: () => ({ data: new Uint8ClampedArray(4), width: 1, height: 1 }),
    measureText: (s) => ({ width: s == null ? 0 : String(s).length * 6 }),
    createLinearGradient: () => ({ addColorStop: noop }),
    createRadialGradient: () => ({ addColorStop: noop }),
    createConicGradient: () => ({ addColorStop: noop }),
    createPattern: () => ({}),
  };
  return new Proxy(base, {
    get(t, k) { if (k in t) return t[k]; if (k in store) return store[k]; return undefined; },
    set(t, k, v) { store[k] = v; return true; },
  });
}

// ── 假 DOM 元素：未知屬性回 no-op 函式 / 空字串，會記錄 addEventListener handler ────
const elCache = new Map();
function makeEl(id) {
  if (elCache.has(id)) return elCache.get(id);
  const handlers = {};
  const isCanvas = id === "game" || id === "<canvas>";
  const real = {
    id, tagName: "DIV", __handlers: handlers,
    width: 800, height: 600,
    value: "", textContent: "", innerHTML: "", checked: false, disabled: false,
    style: new Proxy({}, { get: () => "", set: () => true }),
    dataset: new Proxy({}, { get: () => undefined, set: () => true }),
    classList: { add: () => {}, remove: () => {}, toggle: () => {}, contains: () => false },
    options: [],
    addEventListener: (type, fn) => { (handlers[type] = handlers[type] || []).push(fn); },
    removeEventListener: () => {},
    getContext: isCanvas ? () => ctxSingleton : () => null,
    getBoundingClientRect: () => ({ left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600, x: 0, y: 0 }),
    appendChild: (c) => c, removeChild: () => {}, remove: () => {}, insertBefore: (c) => c,
    setAttribute: () => {}, removeAttribute: () => {}, getAttribute: () => null, hasAttribute: () => false,
    querySelector: () => makeEl(id + " *"), querySelectorAll: () => [],
    focus: () => {}, blur: () => {}, click: () => fire(handlers, "click"),
    scrollIntoView: () => {}, closest: () => null, matches: () => false,
    cloneNode: () => makeEl(id + "#clone"),
    __fire: (type, ev) => fire(handlers, type, ev),
    parentElement: null, parentNode: null, firstChild: null, children: [], childNodes: [],
  };
  const proxy = new Proxy(real, {
    get(t, k) {
      if (k in t) return t[k];
      if (typeof k === "string") return () => {}; // 未知方法 → no-op
      return undefined;
    },
    set(t, k, v) { t[k] = v; return true; },
  });
  elCache.set(id, proxy);
  return proxy;
}
function fire(handlers, type, ev = {}) {
  for (const fn of handlers[type] || []) { try { fn(ev); } catch (e) { throw e; } }
}

// ── canvas 單例 ctx ──
const canvasEl = makeEl("game");
const ctxSingleton = makeCtx(canvasEl);

// ── 假 WebSocket：擷取實例,手動驅動 onopen/onmessage ──
let lastWS = null;
class FakeWS {
  constructor(url) { this.url = url; this.readyState = 1; lastWS = this; this.onopen = null; this.onmessage = null; this.onclose = null; this.onerror = null; }
  send() {}
  close() { this.readyState = 3; }
}
FakeWS.CONNECTING = 0; FakeWS.OPEN = 1; FakeWS.CLOSING = 2; FakeWS.CLOSED = 3;

// ── requestAnimationFrame：擷取 callback,手動逐幀呼叫 ──
let rafCb = null;
let perfNow = 0;

// ── document / window 全域 stub ──
const documentStub = {
  getElementById: (id) => makeEl(id),
  createElement: (tag) => makeEl("<" + tag + ">"),
  createElementNS: (ns, tag) => makeEl("<" + tag + ">"),
  querySelector: () => makeEl("doc?"), querySelectorAll: () => [],
  addEventListener: () => {}, removeEventListener: () => {},
  body: makeEl("body"), documentElement: makeEl("html"), head: makeEl("head"),
  hidden: false, visibilityState: "visible", cookie: "",
  fonts: { ready: Promise.resolve(), add: () => {}, load: () => Promise.resolve() },
  createTextNode: () => ({}),
  activeElement: makeEl("active"),
};
class FakeImage { constructor() { this.onload = null; this.onerror = null; this._src = ""; } set src(v) { this._src = v; } get src() { return this._src; } addEventListener() {} }
class FakeAudio { constructor() { return new Proxy({}, { get: () => () => ({ connect: () => {}, start: () => {}, stop: () => {} }), set: () => true }); } }

const windowStub = {
  requestAnimationFrame: (cb) => { rafCb = cb; return 1; },
  cancelAnimationFrame: () => {},
  performance: { now: () => (perfNow += 16) },
  localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {}, clear: () => {} },
  sessionStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} },
  navigator: { getGamepads: () => [], userAgent: "node-render-smoke", language: "zh-TW", onLine: true, vibrate: () => {}, clipboard: { writeText: () => Promise.resolve() } },
  location: { host: "localhost:3000", hostname: "localhost", protocol: "http:", href: "http://localhost:3000/", origin: "http://localhost:3000", pathname: "/", search: "", reload: () => {} },
  devicePixelRatio: 1, innerWidth: 800, innerHeight: 600, scrollX: 0, scrollY: 0,
  addEventListener: () => {}, removeEventListener: () => {},
  matchMedia: () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {}, addListener: () => {}, removeListener: () => {} }),
  getComputedStyle: () => new Proxy({}, { get: () => "" }),
  setTimeout: (fn) => { return 0; }, clearTimeout: () => {}, setInterval: () => 0, clearInterval: () => {},
  fetch: () => Promise.resolve({ ok: true, json: () => Promise.resolve({}), text: () => Promise.resolve("") }),
  scrollTo: () => {}, alert: () => {}, confirm: () => true, prompt: () => null,
  AudioContext: FakeAudio, webkitAudioContext: FakeAudio, Image: FakeImage, WebSocket: FakeWS,
  MutationObserver: class { observe() {} disconnect() {} takeRecords() { return []; } },
  ResizeObserver: class { observe() {} unobserve() {} disconnect() {} },
  IntersectionObserver: class { observe() {} unobserve() {} disconnect() {} },
};

// 攔截 console.error：safeRender 會把「被攔下的繪製例外」印成 console.error("[render]…")。
// 這些例外不會往外傳(迴圈不凍結=好事),但代表底層仍有真 bug、某些東西沒畫出來,要記下來根治。
const caughtRenderErrors = [];
const consoleProxy = {
  ...console,
  error: (...args) => {
    const first = args[0];
    if (typeof first === "string" && first.includes("[render]")) {
      caughtRenderErrors.push(args.map((a) => (a && a.stack) ? a.stack : String(a)).join(" "));
    }
    console.error(...args);
  },
};

// 把 stub 放上 sandbox（game.js 的自由變數會解析到這裡）。vm context 本身已提供
// Object/Array/JSON/Math 等標準內建,這裡只需補瀏覽器全域 + console + 計時器。
const sandbox = { ...windowStub, document: documentStub, console: consoleProxy, Uint8ClampedArray, Float32Array };
sandbox.window = sandbox;   // window === global
sandbox.self = sandbox;
sandbox.globalThis = sandbox;

vm.createContext(sandbox);

// ── 載入真正的 game.js ──
let loadErr = null;
try {
  vm.runInContext(gameSrc, sandbox, { filename: "web/game.js" });
} catch (e) {
  loadErr = e;
}
if (loadErr) {
  console.error("❌ game.js 載入即拋例外（harness 缺 stub 或 game.js 真有問題）：");
  console.error(loadErr && loadErr.stack || loadErr);
  process.exit(2);
}
console.log("✅ game.js 載入成功（IIFE 已執行、handler 已註冊）");

// ── 觸發進場：點 joinBtn → connect() → new FakeWS ──
makeEl("joinBtn").__fire("click");
if (!lastWS) { console.error("❌ 點 joinBtn 後沒有建立 WebSocket（connect 未觸發）"); process.exit(2); }
console.log("✅ joinBtn → connect() → WebSocket 已建立:", lastWS.url);

// 模擬連線生命週期。
const myId = (snapshot.players && snapshot.players[0] && snapshot.players[0].id) || "qa";
if (lastWS.onopen) lastWS.onopen({});
// world 欄位名須與後端 WorldInfo 一致（width/height），否則 me===null 那幀 camX 會 NaN。
if (lastWS.onmessage) lastWS.onmessage({ data: JSON.stringify({ type: "welcome", id: myId, world: { width: 100000, height: 100000 } }) });
console.log("✅ welcome 已送（myId =", myId + "）");

// ── 跑 render 數幀：先空跑(無 snapshot)、再餵真實城鎮 snapshot 連跑多幀 ──
function pump(label, frames) {
  for (let i = 0; i < frames; i++) {
    const cb = rafCb; rafCb = null;
    if (!cb) { console.log(`  [${label}] 第 ${i} 幀沒有排定 rAF callback（迴圈可能已停）`); return false; }
    try {
      cb(perfNow);
    } catch (e) {
      console.error(`\n🔴 [${label}] 第 ${i} 幀 render 拋例外 —— 這就是「進城人物消失」的根因：`);
      console.error(e && e.stack || e);
      return e;
    }
  }
  return true;
}

console.log("\n── 階段 A：welcome 後、尚無 snapshot，空跑 3 幀 ──");
const a = pump("無snapshot", 3);
if (a instanceof Error) process.exit(1);

// 多情境：原始城鎮 snapshot ＋ 針對已知高風險狀態的合成變體（屍光 / 商人在場 / 態度越界 /
// 居民心情 / 互助請求）。每個變體連跑數幀，安全網攔下的例外也算 FAIL（代表底層真 bug）。
function variant(name, mutate) {
  const s = JSON.parse(JSON.stringify(snapshot));
  mutate(s);
  return { name, s };
}
const me0 = snapshot.players[0];
const scenarios = [
  { name: "原始城鎮", s: snapshot },
  variant("含屍光carion_orbs", (s) => { s.carion_orbs = [{ id: 1, x: me0.x + 40, y: me0.y }, { id: 2, x: me0.x - 30, y: me0.y + 20 }]; }),
  variant("旅行商人在場", (s) => { s.wandering_merchant_secs = 90; s.wandering_catalog = [{ item: "pickaxe", price_ether: 15, remaining: 3 }]; }),
  variant("態度越界(負/超100)", (s) => { if (s.species_attitudes?.length) { s.species_attitudes[0].attitude = -25; s.species_attitudes[0].tier = "hostile"; if (s.species_attitudes[1]) s.species_attitudes[1].attitude = 140; } }),
  variant("居民心情+互助請求", (s) => { s.resident_moods = { "r1": 20, "r2": 95 }; s.active_help_requests = ["r1"]; }),
  variant("野生動物含未知kind", (s) => { if (s.wildlife?.length) { s.wildlife[0] = { ...s.wildlife[0], kind: "mystery_beast", state: "hunting" }; } }),
  // 雨後彩虹（191）：先下草原雨（白天）→ 下一情境雨停，跨情境觸發彩虹繪製路徑。
  variant("草原降雨(白天)", (s) => { s.daynight = { phase: "day", light: 0.75, night_danger: false }; s.weather = { weather_type: "grassland_rain", intensity: 0.8 }; }),
  variant("雨停天青(彩虹)", (s) => { s.daynight = { phase: "day", light: 0.75, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 天邊流雲（193）：白天情境（上方 light 0.75）已跑白雲路徑；這裡加破曉金色時刻跑「染暖」分支。
  variant("破曉金色時刻流雲", (s) => { s.daynight = { phase: "dawn", light: 0.6, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 水域波光（195）：把玩家移到確定是水域的座標（biomeAtJS 在此片成片回 water），讓相機視野滿是
  // 水面 tile，實跑波光「撒點→明滅→繪製柔光斑」內層路徑（白天藍白波光）。
  variant("水域波光(白天)", (s) => { s.players[0].x = -4400; s.players[0].y = -3000; s.daynight = { phase: "day", light: 0.85, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 水域波光晨昏染金分支：同一片水域、破曉色溫，跑 shimmerTint 的 dawn/dusk 金橘路徑。
  variant("水域波光(破曉染金)", (s) => { s.players[0].x = -4400; s.players[0].y = -3000; s.daynight = { phase: "dawn", light: 0.6, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 水域波光夜間映月分支：同一片水域、入夜冷月白光，跑 shimmerTint 的夜/暗路徑與夜間 strength。
  variant("水域波光(夜映月)", (s) => { s.players[0].x = -4400; s.players[0].y = -3000; s.daynight = { phase: "night", light: 0.12, night_danger: true }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 水岸碎浪（196）：把玩家移到確定含水陸交界的座標（(960,-6000) 視野內 ~42 個臨陸水格），
  // 讓相機視野滿是岸線，實跑碎浪「找岸邊→漲退→雙描邊浪沫」內層路徑（白天亮白）。
  variant("水岸碎浪(白天)", (s) => { s.players[0].x = 960; s.players[0].y = -6000; s.daynight = { phase: "day", light: 0.85, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 水岸碎浪晨昏染暖分支：同一片岸線、破曉色溫，跑 foamTint 的 dawn/dusk 暖奶白路徑。
  variant("水岸碎浪(破曉染暖)", (s) => { s.players[0].x = 960; s.players[0].y = -6000; s.daynight = { phase: "dawn", light: 0.6, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 水岸碎浪夜映月分支：同一片岸線、入夜冷月白沫，跑 foamTint 的夜/暗路徑與夜間 strength。
  variant("水岸碎浪(夜映月)", (s) => { s.players[0].x = 960; s.players[0].y = -6000; s.daynight = { phase: "night", light: 0.12, night_danger: true }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 草原微風／草浪（197）：城鎮中心 (3000,3000) 是成片草原，視野滿是草地 tile，實跑草浪
  // 「投影風帶相位→陣風亮度→提亮草格」內層路徑（白天暖白草光，跑 windTint 白天分支）。
  variant("草原微風(白天)", (s) => { s.players[0].x = 3000; s.players[0].y = 3000; s.daynight = { phase: "day", light: 0.85, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 草浪晨昏染金分支：同一片草原、破曉色溫，跑 windTint 的 dawn/dusk 金橘路徑。
  variant("草原微風(破曉染金)", (s) => { s.players[0].x = 3000; s.players[0].y = 3000; s.daynight = { phase: "dawn", light: 0.6, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 草浪夜映月分支：同一片草原、入夜冷月白，跑 windTint 的夜/暗路徑與夜間 strength（風不歇、仍留微光）。
  variant("草原微風(夜映月)", (s) => { s.players[0].x = 3000; s.players[0].y = 3000; s.daynight = { phase: "night", light: 0.12, night_danger: true }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 沙漠流沙微光（198）：把玩家移到確定成片沙漠的座標（(2800,-1700) 視野內幾乎滿是 sand 格），
  // 讓相機視野滿是沙地，實跑流沙微光「撒點→順風飄移行進包絡→明滅→繪製柔光斑」內層路徑（白天暖金沙光）。
  variant("沙漠流沙(白天)", (s) => { s.players[0].x = 2800; s.players[0].y = -1700; s.daynight = { phase: "day", light: 0.85, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 流沙微光晨昏染金分支：同一片沙漠、破曉色溫，跑 sandGlintTint 的 dawn/dusk 金橘路徑。
  variant("沙漠流沙(破曉染金)", (s) => { s.players[0].x = 2800; s.players[0].y = -1700; s.daynight = { phase: "dawn", light: 0.6, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 流沙微光夜映月分支：同一片沙漠、入夜清冷月白，跑 sandGlintTint 的夜/暗路徑與夜間 strength（沙面映月微光）。
  variant("沙漠流沙(夜映月)", (s) => { s.players[0].x = 2800; s.players[0].y = -1700; s.daynight = { phase: "night", light: 0.12, night_danger: true }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 岩石礦脈微光（199）：把玩家移到確定成片岩石的座標（(-7000,5800) 視野內幾乎滿是 rocky 格），
  // 讓相機視野滿是岩地，實跑礦脈微光「撒點→明滅→繪製柔光斑」內層路徑（白天冷白銀光）。
  variant("岩石礦光(白天)", (s) => { s.players[0].x = -7000; s.players[0].y = 5800; s.daynight = { phase: "day", light: 0.85, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 礦脈微光晨昏染金分支：同一片岩地、黃昏色溫，跑 rockGlintTint 的 dawn/dusk 暖金路徑。
  variant("岩石礦光(黃昏染金)", (s) => { s.players[0].x = -7000; s.players[0].y = 5800; s.daynight = { phase: "dusk", light: 0.6, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 礦脈微光夜映月分支：同一片岩地、入夜清冷月白，跑 rockGlintTint 的夜/暗路徑與夜間 strength（岩面映月微光）。
  variant("岩石礦光(夜映月)", (s) => { s.players[0].x = -7000; s.players[0].y = 5800; s.daynight = { phase: "night", light: 0.12, night_danger: true }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
];

let failed = false;
for (const sc of scenarios) {
  const before = caughtRenderErrors.length;
  console.log(`── 情境：${sc.name}（連跑 6 幀）──`);
  lastWS.onmessage({ data: JSON.stringify({ ...sc.s, type: "snapshot" }) });
  const r = pump(sc.name, 6);
  if (r instanceof Error) { failed = true; console.error(`  ❌ ${sc.name}：未捕捉例外`); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ ${sc.name}：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log(`  ✅ ${sc.name}：乾淨`);
}

// 夜空流星（192）：入夜後偶發流星，首顆延遲 ~1.5s 才點燃，故需連跑較多幀（每幀 +16ms）
// 才會實跑「點燃→繪製漸層尾巴→熄滅→排下一顆」完整路徑（一般情境的 6 幀碰不到）。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：入夜（夜空流星，連跑 180 幀觸發流星劃過）──");
  const nightSnap = JSON.parse(JSON.stringify(snapshot));
  nightSnap.daynight = { phase: "night", light: 0.12, night_danger: true };
  nightSnap.weather = { weather_type: "clear", intensity: 0.0 };
  lastWS.onmessage({ data: JSON.stringify({ ...nightSnap, type: "snapshot" }) });
  const r = pump("入夜流星", 180);
  if (r instanceof Error) { failed = true; console.error("  ❌ 入夜流星：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 入夜流星：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 入夜流星：乾淨");
}

// 白晝飛鳥（194）：白天偶發鳥群，首群延遲 ~1.5s 才放飛、單群飛 9s，故連跑 ~720 幀（每幀 +16ms ≈ 11.5s）
// 才能實跑「放飛→拍翅 V 字編隊橫越→飛遠熄滅→排下一群」完整路徑（一般情境 6 幀碰不到）。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：白天（白晝飛鳥，連跑 720 幀觸發鳥群橫越）──");
  const daySnap = JSON.parse(JSON.stringify(snapshot));
  daySnap.daynight = { phase: "day", light: 0.85, night_danger: false };
  daySnap.weather = { weather_type: "clear", intensity: 0.0 };
  lastWS.onmessage({ data: JSON.stringify({ ...daySnap, type: "snapshot" }) });
  const r = pump("白晝飛鳥", 720);
  if (r instanceof Error) { failed = true; console.error("  ❌ 白晝飛鳥：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 白晝飛鳥：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 白晝飛鳥：乾淨");
}

// 天空的太陽與月亮（200）：太陽/月亮的「弧上水平位置」靠每幀追蹤 light 趨勢分辨（升 or 落），
// 故需「光度隨時間連續變化」才跑得到完整路徑——這裡實跑一輪日夜循環（夜→晨→正午→昏→夜），
// light 先升後降，依序觸發「月落→太陽東升→正午高懸→太陽西沉染紅→月升」的趨勢翻轉與交班。
// 日影晷（201）同口徑：此情境也連續跑到地面投影的 rising/falling 幾何（晨影朝西、昏影朝東）、
// 晨昏拉長與正午縮回、太陽/月亮交班混合，及白天深影→夜映冷淡影的濃淡色溫切換。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：日夜循環（太陽/月亮升落，light 連續升降跑趨勢翻轉與交班）──");
  const cycSnap = JSON.parse(JSON.stringify(snapshot));
  cycSnap.weather = { weather_type: "clear", intensity: 0.0 };
  // 一輪 light 軌跡：0.1（夜）→ 0.9（正午）→ 0.1（夜），對應 phase 由 night→dawn→day→dusk→night。
  const seq = [];
  for (let i = 0; i <= 16; i++) seq.push(0.1 + (0.8 * i) / 16);   // 升（月落→日升→正午）
  for (let i = 1; i <= 16; i++) seq.push(0.9 - (0.8 * i) / 16);   // 降（正午→日落→月升）
  let cycErr = false;
  for (const light of seq) {
    const phase = light < 0.35 ? "night" : (light < 0.62 ? "dawn" : "day");
    cycSnap.daynight = { phase, light, night_danger: light < 0.35 };
    lastWS.onmessage({ data: JSON.stringify({ ...cycSnap, type: "snapshot" }) });
    const r = pump(`日夜循環 light=${light.toFixed(2)}`, 3);
    if (r instanceof Error) { cycErr = true; break; }
  }
  if (cycErr) { failed = true; console.error("  ❌ 日夜循環：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 日夜循環：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!cycErr) console.log("  ✅ 日夜循環：乾淨");
}

console.log("");
if (failed) {
  console.error("🔴 render-smoke 發現繪製例外（見上）。safeRender 雖防止凍結，但應根治根因。");
  process.exit(1);
}
console.log("✅✅ render-smoke 全綠：所有情境（含屍光/商人/態度越界/未知物種）連跑多幀，render 零例外、safeRender 零攔截。");
process.exit(0);
