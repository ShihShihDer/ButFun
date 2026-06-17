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
// FakeImage 報告「已載入完成」：complete + naturalWidth/Height 非零，讓 game.js 的 artOk/clayOk
// 判定為真，實際走精靈圖/黏土圖的 drawImage / createPattern 繪製分支（否則永遠 fallback、測不到）。
class FakeImage { constructor() { this.onload = null; this.onerror = null; this._src = ""; this.complete = true; this.naturalWidth = 512; this.naturalHeight = 512; } set src(v) { this._src = v; } get src() { return this._src; } addEventListener() {} }
class FakeAudio { constructor() { return new Proxy({}, { get: () => () => ({ connect: () => {}, start: () => {}, stop: () => {} }), set: () => true }); } }

const windowStub = {
  requestAnimationFrame: (cb) => { rafCb = cb; return 1; },
  cancelAnimationFrame: () => {},
  performance: { now: () => (perfNow += 16) },
  localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {}, clear: () => {} },
  sessionStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} },
  navigator: { getGamepads: () => [], userAgent: "node-render-smoke", language: "zh-TW", onLine: true, vibrate: () => {}, clipboard: { writeText: () => Promise.resolve() } },
  // 渲染風格情境：預設空（＝pixel）；BUTFUN_SMOKE_STYLE=clay 時模擬 ?style=clay，讓 game.js
  // IIFE 啟動即進黏土模式，連跑全黏土地面/sprite 路徑也抓零 render 例外。
  location: { host: "localhost:3000", hostname: "localhost", protocol: "http:", href: "http://localhost:3000/", origin: "http://localhost:3000", pathname: "/", search: process.env.BUTFUN_SMOKE_STYLE === "clay" ? "?style=clay" : "", reload: () => {} },
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

// ── ROADMAP 352：🌍 世界頻道 raw-fallback ──
// 後端大量「全服世界事件」走 tx_chat 直送純字串（NPC 宣告、里程碑同慶、生態見聞…），
// 非 JSON。舊版前端 ws.onmessage 的 JSON.parse 失敗即 return，把這些事件默默吞掉。
// 驗證：非 JSON 純文字會被當世界訊息送進 #chatLog；空白訊息忽略；合法 JSON 不誤入此路。
{
  let worldLines = 0;
  const chatLog = makeEl("chatLog");
  chatLog.appendChild = (c) => { worldLines++; return c; };
  lastWS.onmessage({ data: "🏅 阿狼 完成了生態清剿委託！獲得 120 乙太" }); // 非 JSON → 應顯示
  lastWS.onmessage({ data: "   " });                                       // 空白 → 應忽略
  if (worldLines !== 1) {
    console.error(`❌ 世界頻道 raw-fallback：預期 1 行進 chatLog，實得 ${worldLines}`);
    process.exit(2);
  }
  console.log("✅ 世界頻道 raw-fallback：非 JSON 全服事件已顯示、空白訊息已忽略");
}

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
  // 餵食馴養（205）：把幾隻野生動物設成已馴養（tamed→頭頂 💛）與餵食進行中（familiarity 部分→進度心 🤍），
  // 實跑 drawWildlife 的馴養愛心兩條新繪製分支（含 globalAlpha 進度染色）。
  // 野外守護者（357）：把一隻掠食者設成「正在追獵」且緊鄰玩家（≤90px）→ 跑 updateScarePredatorBtn
  // 的「建立並顯示驅趕按鈕」分支；另放一隻在遠處追獵的不應顯示（距離過濾）。零例外即通過。
  variant("驅趕掠食者按鈕(357)", (s) => {
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "wild_wolf", x: me0.x + 40, y: me0.y, state: "hunting", familiarity: 0, tamed: false, alive: true };
      if (s.wildlife[1]) s.wildlife[1] = { ...s.wildlife[1], kind: "wild_fox", x: me0.x + 2000, y: me0.y, state: "stalking", familiarity: 0, tamed: false, alive: true };
    }
  }),
  variant("野生動物馴養(愛心)", (s) => {
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], x: me0.x + 30, y: me0.y, state: "wandering", familiarity: 1.0, tamed: true };
      if (s.wildlife[1]) s.wildlife[1] = { ...s.wildlife[1], x: me0.x - 30, y: me0.y + 20, state: "resting", familiarity: 0.5, tamed: false };
    }
  }),
  // 晝夜作息（210）：夜間把獵物設成 resting → 跑 drawWildlife 的安睡 💤 繪製分支（含 globalAlpha 呼吸明滅）；
  // 同場放一隻 resting 的掠食者，驗證夜行者不畫 💤（!isPredator 守衛）。
  variant("野生動物夜眠(💤)", (s) => {
    s.daynight = { phase: "night", light: 0.12, night_danger: true };
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "wild_deer", x: me0.x + 30, y: me0.y, state: "resting", familiarity: 0, tamed: false };
      if (s.wildlife[1]) s.wildlife[1] = { ...s.wildlife[1], kind: "wild_wolf", x: me0.x - 30, y: me0.y + 20, state: "resting" };
    }
  }),
  // 白晝吃草（211）：白天把獵物設成 grazing → 跑 drawWildlife 的吃草 🌿 繪製分支（含 rotate 搖曳）。
  variant("野生動物白晝吃草(🌿)", (s) => {
    s.daynight = { phase: "day", light: 0.85, night_danger: false };
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "wild_deer", x: me0.x + 30, y: me0.y, state: "grazing", familiarity: 0, tamed: false };
    }
  }),
  // 群體警戒哨（212）：白天把一隻獵物設成 watching → 跑 drawWildlife 的放哨 👀 繪製分支（含張望平移）。
  variant("野生動物放哨(👀)", (s) => {
    s.daynight = { phase: "day", light: 0.85, night_danger: false };
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "wild_deer", x: me0.x + 30, y: me0.y, state: "watching", familiarity: 0, tamed: false };
    }
  }),
  // 孤獵潛行（213）：把掠食者設成 stalking → 跑 drawWildlife 的潛行 🐾 繪製分支（含起伏 alpha）。
  variant("掠食者潛行(🐾)", (s) => {
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "wild_wolf", x: me0.x - 40, y: me0.y, state: "stalking", familiarity: 0, tamed: false };
    }
  }),
  // 母獸護幼（214）：把成體獵物設成 defending → 跑 drawWildlife 的護幼 🛡 繪製分支（含抖動平移與警戒黃標籤）。
  variant("母獸護幼(🛡)", (s) => {
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "wild_deer", x: me0.x + 30, y: me0.y, state: "defending", familiarity: 0, tamed: false };
    }
  }),
  // 幼獸嬉戲（215）：把一隻幼獸設成 frolicking → 跑 drawWildlife 的嬉戲 💫 繪製分支（含上下彈跳平移）。
  variant("幼獸嬉戲(💫)", (s) => {
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "wild_deer", x: me0.x + 30, y: me0.y, state: "frolicking", juvenile: true, familiarity: 0, tamed: false };
    }
  }),
  // 幼獸學吃草（289）：把一隻幼獸設成 mimic_graze → 跑 drawWildlife 的學吃草 🌱 繪製分支（含一頓一頓平移）。
  variant("幼獸學吃草(🌱)", (s) => {
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "wild_deer", x: me0.x + 30, y: me0.y, state: "mimic_graze", juvenile: true, familiarity: 0, tamed: false };
    }
  }),
  // 掠食者夜嚎（217）：夜間把掠食者設成 howling → 跑 drawWildlife 的長嚎 🌙 繪製分支（含明滅與仰首上揚平移）。
  variant("掠食者夜嚎(🌙)", (s) => {
    s.daynight = { phase: "night", light: 0.2, night_danger: true };
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "wild_wolf", x: me0.x - 40, y: me0.y, state: "howling", familiarity: 0, tamed: false };
    }
  }),
  // 鳥群振翅升空盤旋（220）：白天把野鳥設成 flying → 跑 drawWildlife 的升空繪製分支（地面投影＋抬起鳥身）。
  variant("野鳥升空盤旋", (s) => {
    s.daynight = { phase: "day", light: 0.8, night_danger: false };
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "wild_bird", x: me0.x + 30, y: me0.y, state: "flying", familiarity: 0, tamed: false };
    }
  }),
  // 晝日鳥鳴呼應（221）：白天把野鳥設成 chirping → 跑 drawWildlife 的啁啾 🎵 繪製分支（含明滅與上下跳動平移）。
  variant("野鳥啁啾", (s) => {
    s.daynight = { phase: "day", light: 0.8, night_danger: false };
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "wild_bird", x: me0.x + 30, y: me0.y, state: "chirping", familiarity: 0, tamed: false };
    }
  }),
  // 小動物捧食啃咬（222）：白天把小動物設成 nibbling → 跑 drawWildlife 的啃咬 🌰 繪製分支（含明滅與微微起伏平移）。
  variant("小動物啃咬", (s) => {
    s.daynight = { phase: "day", light: 0.8, night_danger: false };
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "small_critter", x: me0.x + 30, y: me0.y, state: "nibbling", familiarity: 0, tamed: false };
    }
  }),
  // 野狐撲鼠（223）：白天把野狐設成 pouncing → 跑 drawWildlife 的撲跳繪製分支（抬起狐身成躍弧＋地面投影＋頭頂 💨）。
  variant("野狐撲鼠", (s) => {
    s.daynight = { phase: "day", light: 0.8, night_danger: false };
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "wild_fox", x: me0.x + 30, y: me0.y, state: "pouncing", familiarity: 0, tamed: false };
    }
  }),
  // 野鹿頂角較勁（224）：白天把野鹿設成 sparring → 跑 drawWildlife 的較勁繪製分支（身體一推一退地頂撞＋頭頂一脹一縮的 💥）。
  variant("野鹿頂角較勁", (s) => {
    s.daynight = { phase: "day", light: 0.8, night_danger: false };
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "wild_deer", x: me0.x + 30, y: me0.y, state: "sparring", familiarity: 0, tamed: false };
    }
  }),
  // 野生動物接地柔影（241）：破曉低光 → _shadowCast 拉長偏移，跑各物種腳下影的偏移＋拉長路徑。
  variant("野生動物晨昏接地長影", (s) => {
    s.daynight = { phase: "dawn", light: 0.5, night_danger: false };
    if (s.wildlife?.length) {
      s.wildlife[0] = { ...s.wildlife[0], kind: "wild_deer", x: me0.x + 30, y: me0.y, state: "wandering" };
      if (s.wildlife[1]) s.wildlife[1] = { ...s.wildlife[1], kind: "wild_wolf", x: me0.x - 30, y: me0.y + 20, state: "wandering" };
      if (s.wildlife[2]) s.wildlife[2] = { ...s.wildlife[2], kind: "small_critter", x: me0.x, y: me0.y + 40, state: "wandering" };
    }
  }),
  // 雨後彩虹（191）：先下草原雨（白天）→ 下一情境雨停，跨情境觸發彩虹繪製路徑。
  variant("草原降雨(白天)", (s) => { s.daynight = { phase: "day", light: 0.75, night_danger: false }; s.weather = { weather_type: "grassland_rain", intensity: 0.8 }; }),
  variant("雨停天青(彩虹)", (s) => { s.daynight = { phase: "day", light: 0.75, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 天邊流雲（193）：白天情境（上方 light 0.75）已跑白雲路徑；這裡加破曉金色時刻跑「染暖」分支。
  variant("破曉金色時刻流雲", (s) => { s.daynight = { phase: "dawn", light: 0.6, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 水域波光（195）：把玩家移到確定是水域的座標（biomeAtJS 在此片成片回 water），讓相機視野滿是
  // 水面 tile，實跑波光「撒點→明滅→繪製柔光斑」內層路徑（白天藍白波光）。
  // 水面映日／映月（202）一併在這三片水域情境內被跑到（同片水域，白天暖白／破曉金橘／夜映月銀三色溫
  // 分支與 drawSunGlint 撒方位倒影內層路徑）；其方位倒影強度的 rising/falling 幾何另由下方「日夜循環」
  // 情境（light 連續升降一輪）跑到晨昏強放／正午弱早退的完整切換。
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
  // 雲影掠地（203）：白天雲遮日在地表拖過的大片緩移柔暗斑，與地形無關（任何地表都畫），
  // 故沿用城鎮中心座標。正午（light 0.85）跑 strength 達滿的滿濃路徑（initCloudShadows→窗格回捲鋪滿→偏扁橢圓柔斑漸層）。
  variant("雲影掠地(正午滿濃)", (s) => { s.players[0].x = 3000; s.players[0].y = 3000; s.daynight = { phase: "day", light: 0.85, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 雲影晨昏轉淡分支：破曉 light 0.5 落在 MIN(0.42)→DAY(0.6) 淡入帶內，跑 strength 部分濃度的繪製路徑。
  variant("雲影掠地(破曉轉淡)", (s) => { s.players[0].x = 3000; s.players[0].y = 3000; s.daynight = { phase: "dawn", light: 0.5, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 晨昏霞光天幕（204）：破曉 light 0.5 落在霞光 MID(0.5) 峰值，phase=dawn 跑「東側玫瑰金、強度滿、
  // 主斑 radialGradient＋天邊 linearGradient」完整路徑（rising=true → nx 偏左）。
  variant("晨昏霞光(破曉玫瑰金)", (s) => { s.players[0].x = 3000; s.players[0].y = 3000; s.daynight = { phase: "dawn", light: 0.5, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 霞光黃昏橙紅分支：phase=dusk、light 0.42（偏低、lowness 大）跑 twilightGlowTint 的黃昏橙紅色與
  // 「光源貼地平→霞光更濃更沉」路徑（rising=false → nx 偏右）。
  variant("晨昏霞光(黃昏橙紅)", (s) => { s.players[0].x = 3000; s.players[0].y = 3000; s.daynight = { phase: "dusk", light: 0.42, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 冬日飄雪（226）：current_season=winter → 跑 drawSnow 的飄雪繪製分支（雪勢淡入→撒雪花→
  // 緩降搖擺→薄霜白幕）。6 幀已足以讓 _snowFade 越過 0.01 門檻進入繪製主路徑（完整落下/despawn 見專屬長跑情境）。
  variant("冬日飄雪", (s) => { s.current_season = "winter"; s.daynight = { phase: "day", light: 0.82, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 秋日落葉（227）：current_season=autumn → 跑 drawLeaves 的落葉繪製分支（葉勢淡入→撒葉→
  // 打旋搖擺緩降→中肋描線→暖金薄幕）。6 幀已足以讓 _leafFade 越過 0.01 門檻進入繪製主路徑。
  variant("秋日落葉", (s) => { s.current_season = "autumn"; s.daynight = { phase: "day", light: 0.82, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 春日花飛（228）：current_season=spring → 跑 drawBlossom 的花瓣繪製分支（花瓣勢淡入→撒瓣→
  // 橫飄翻轉緩降→淡粉薄幕）。6 幀已足以讓 _petalFade 越過 0.01 門檻進入繪製主路徑。
  variant("春日花飛", (s) => { s.current_season = "spring"; s.daynight = { phase: "day", light: 0.82, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 夏日蟬夏（229）：current_season=summer → 跑 drawSummerMotes 的浮塵繪製分支（夏絮勢淡入→撒絮→
  // 上飄閃爍橫盪→暖金薄幕）。6 幀已足以讓 _moteFade 越過 0.01 門檻進入繪製主路徑。
  variant("夏日蟬夏", (s) => { s.current_season = "summer"; s.daynight = { phase: "day", light: 0.82, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 春夏彩蝶（236）：current_season=spring/summer ＋ 白天（light 0.82 > 0.42）→ 跑 drawButterflies 的
  // 彩蝶繪製分支（蝶勢淡入→繞圈翩飛→開合拍翅）。6 幀已足以讓 _butterflyFade 越過 0.01 門檻進入繪製主路徑
  //（春夜螢火 233 的白天對偶）。spring 與 summer 各跑一次確認兩季皆觸發。
  variant("春日彩蝶", (s) => { s.current_season = "spring"; s.daynight = { phase: "day", light: 0.82, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  variant("夏日彩蝶", (s) => { s.current_season = "summer"; s.daynight = { phase: "day", light: 0.82, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 秋日紅蜻蜓（237）：current_season=autumn ＋ 白天（light 0.82 > 0.42）→ 跑 drawDragonflies 的蜻蜓繪製分支
  //（蜻蜓勢淡入→停—衝疾射→翅膀嗡振）。6 幀已足以讓 _dragonflyFade 越過 0.01 門檻進入繪製主路徑（春夏彩蝶的秋日對偶）。
  variant("秋日紅蜻蜓", (s) => { s.current_season = "autumn"; s.daynight = { phase: "day", light: 0.82, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 冬日寒雀（238）：current_season=winter ＋ 白天（light 0.82 > 0.42）→ 跑 drawSparrows 的寒雀繪製分支
  //（雀勢淡入→原地啄食點頭→停—衝短距蹦躍→拋物線弧＋展翅）。6 幀已足以讓 _sparrowFade 越過 0.01 門檻進入繪製主路徑（彩蝶 236／蜻蜓 237 的冬日對偶）。
  variant("冬日寒雀", (s) => { s.current_season = "winter"; s.daynight = { phase: "day", light: 0.82, night_danger: false }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 夜空月相（239）：入夜（light 0.12）→ 跑 drawCelestialBody 的月亮分支，月相陰影依真實朔望月
  // （moonPhase(Date.now)）罩出暗面、畫出陰晴圓缺（暗側月緣半圓＋終止線橢圓圍成的弧月）。
  // 取代 200 過去那抹固定假陰影；任一真實時刻的月相都應零例外繪過暗面合圍路徑。
  variant("夜空月相", (s) => { s.daynight = { phase: "night", light: 0.12, night_danger: true }; s.weather = { weather_type: "clear", intensity: 0.0 }; }),
  // 雨打水面漣漪（247）：玩家在成片水域（同波光那片 (-4400,-3000)）＋草原降雨（grassland_rain，
  // intensity 0.7 > 0.2 門檻）→ 跑 drawRainRipples 的繪製分支（水格上撒漣漪源→濺起→擴散環→淡盡換落點）。
  // 三種色溫（白天藍白／晨昏金／夜冷月白）各跑一片，確認 shimmerTint 三分支與漣漪環繪製零例外。
  variant("雨打水面漣漪(白天)", (s) => { s.players[0].x = -4400; s.players[0].y = -3000; s.daynight = { phase: "day", light: 0.7, night_danger: false }; s.weather = { weather_type: "grassland_rain", intensity: 0.7 }; }),
  variant("雨打水面漣漪(破曉金)", (s) => { s.players[0].x = -4400; s.players[0].y = -3000; s.daynight = { phase: "dawn", light: 0.55, night_danger: false }; s.weather = { weather_type: "grassland_rain", intensity: 0.7 }; }),
  variant("雨打水面漣漪(夜冷月)", (s) => { s.players[0].x = -4400; s.players[0].y = -3000; s.daynight = { phase: "night", light: 0.15, night_danger: true }; s.weather = { weather_type: "grassland_rain", intensity: 0.5 }; }),
  // 寵物現身相伴（ROADMAP 343）：玩家帶寵物 → 跑 drawPlayer 的寵物繪製分支（座標內插＋待機輕浮／
  // 追趕走動彈跳＋腳下小陰影＋登場蹦跳）。歇腳（寵物貼近主人）與追趕（寵物離主人 > 34px）兩條
  // 彈跳分支各跑一次，兩種代表寵物（飄浮系 🧚／守護系 🌟）各畫一回，確認寵物渲染零繪製例外。
  variant("寵物相伴(歇腳)", (s) => { const m = s.players[0]; m.pet_kind = "flutter_sprite"; m.pet_x = m.x + 10; m.pet_y = m.y + 6; }),
  variant("寵物相伴(追趕主人)", (s) => { const m = s.players[0]; m.pet_kind = "origin_guardian"; m.pet_x = m.x - 90; m.pet_y = m.y - 20; }),
  // 寵物玩伴嬉戲（ROADMAP 344）：寵物正在跟別的寵物玩耍 → 跑 drawPlayer 的玩耍分支（更歡快的
  // 蹦跳＋頭頂上飄循環淡出的愛心／音符）。pet_playing=true，覆蓋愛心（💕）與音符（🎵）兩種頭頂
  // glyph 分支（依 pet_x 奇偶挑選）各畫一回，確認玩耍特效渲染零繪製例外。
  variant("寵物玩伴嬉戲(愛心)", (s) => { const m = s.players[0]; m.pet_kind = "coral_crab"; m.pet_x = m.x + 4; m.pet_y = m.y + 2; m.pet_playing = true; }),
  variant("寵物玩伴嬉戲(音符)", (s) => { const m = s.players[0]; m.pet_kind = "jade_wraith"; m.pet_x = m.x + 12; m.pet_y = m.y + 2; m.pet_playing = true; }),
  // 寵物逗玩接物（ROADMAP 345）：玩家丟出玩具、寵物正在接物 → 跑 drawPlayer 的接物分支（興奮衝刺
  // 彈跳＋玩具 🎾 繪製）。覆蓋兩種玩具狀態：① 落地待叼（玩具離寵物遠＝追逐階段，畫落地玩具＋影子）；
  // ② 被叼著走（玩具貼著寵物＜16px＝叼回階段，畫在寵物嘴邊）。確認接物渲染零繪製例外。
  variant("寵物逗玩接物(衝去叼)", (s) => { const m = s.players[0]; m.pet_kind = "flutter_sprite"; m.pet_x = m.x + 20; m.pet_y = m.y; m.pet_fetching = true; m.pet_toy_x = m.x + 120; m.pet_toy_y = m.y; }),
  variant("寵物逗玩接物(叼回來)", (s) => { const m = s.players[0]; m.pet_kind = "crystal_golem"; m.pet_x = m.x + 60; m.pet_y = m.y + 10; m.pet_fetching = true; m.pet_toy_x = m.x + 60; m.pet_toy_y = m.y + 10; }),
  // 寵物性格（ROADMAP 358）：歇腳發呆的寵物帶 pet_personality → 跑 drawPlayer 的性格心情泡泡分支
  // （依性格每隔幾秒頭頂飄一枚 🎵/💤/❓/💕）。寵物貼近主人（歇腳、非追趕、非玩耍／接物）才會進這分支；
  // 覆蓋全部四種性格各畫一回，確認待機心情泡泡渲染零繪製例外。
  variant("寵物性格(活潑)", (s) => { const m = s.players[0]; m.pet_kind = "flutter_sprite"; m.pet_x = m.x + 10; m.pet_y = m.y + 6; m.pet_personality = "playful"; }),
  variant("寵物性格(慵懶)", (s) => { const m = s.players[0]; m.pet_kind = "coral_crab"; m.pet_x = m.x + 12; m.pet_y = m.y + 4; m.pet_personality = "lazy"; }),
  variant("寵物性格(好奇)", (s) => { const m = s.players[0]; m.pet_kind = "jade_wraith"; m.pet_x = m.x - 12; m.pet_y = m.y + 4; m.pet_personality = "curious"; }),
  variant("寵物性格(黏人)", (s) => { const m = s.players[0]; m.pet_kind = "origin_guardian"; m.pet_x = m.x + 8; m.pet_y = m.y + 6; m.pet_personality = "clingy"; }),
  // 未知性格 wire key（向後相容／前端保底）：帶寵物但 personality 是前端不認得的字串 → 性格分支
  // 應靜默略過（PET_PERSONALITY[key] 為 undefined），不畫泡泡也不丟例外。
  variant("寵物性格(未知key保底)", (s) => { const m = s.players[0]; m.pet_kind = "flutter_sprite"; m.pet_x = m.x + 10; m.pet_y = m.y + 6; m.pet_personality = "mystery_mood"; }),
  // 釣魚上鉤小遊戲（ROADMAP 346）：玩家拋竿後 → 跑 drawPlayer 的釣魚浮標分支（釣線＋浮標＋
  // 咬鉤漣漪＋頭頂「❗」）。覆蓋兩階段：① 等咬鉤（waiting，平靜浮標）；② 魚上鉤（biting，
  // 抖動浮標＋漣漪＋「❗」彈出，附 _fishBiteAt 起算彈出動畫）。確認浮標渲染零繪製例外。
  variant("釣魚上鉤(等咬鉤)", (s) => { const m = s.players[0]; m.fishing_phase = "waiting"; }),
  variant("釣魚上鉤(魚咬鉤)", (s) => { const m = s.players[0]; m.fishing_phase = "biting"; m._fishBiteAt = 0; }),
  // 季節漁汛（ROADMAP 363）：垂釣中時自己浮標旁多畫一行「🎣 秋汛·🦈當季」本季當紅魚提示——
  // 換成秋季（深海魚當季）走非預設季節的映射分支。確認季節漁汛提示渲染零繪製例外。
  variant("季節漁汛(秋汛·垂釣中)", (s) => { s.current_season = "autumn"; const m = s.players[0]; m.fishing_phase = "waiting"; }),
  // 礦脈深掘（ROADMAP 348）：玩家挖礦中 → 跑 drawPlayer 的「⛏️ 深度」頭頂標記分支（震動晃動＋
  // 依等級換色）。覆蓋三檔震動：① 淺層穩定（calm）；② 細微落石（faint，輕晃）；③ 劇烈搖晃
  // （severe，大晃＋紅字）。確認採礦世界訊號渲染零繪製例外。
  variant("採礦深掘(穩定)",   (s) => { const m = s.players[0]; m.mining_depth = 1; m.mining_tremor = "calm"; }),
  variant("採礦深掘(細微落石)", (s) => { const m = s.players[0]; m.mining_depth = 3; m.mining_tremor = "faint"; }),
  variant("採礦深掘(劇烈搖晃)", (s) => { const m = s.players[0]; m.mining_depth = 5; m.mining_tremor = "severe"; }),
  // 夜泉汲取（ROADMAP 350 汲泉聚精）：玩家汲取中 → 跑 drawPlayer 的頭頂「擺盪準星量表」分支
  // （軌道＋豐盈帶＋峰湧甜蜜區＋準星＋自己才顯示的提示字）。covers 準星掃到甜蜜區（中心）與
  // 量表左端兩種準星位置，確認汲取量表渲染零繪製例外。
  variant("夜泉汲取(準星峰湧)", (s) => { const m = s.players[0]; m.aether_draw_secs = 0.4167; m._drawRecvAt = performance.now(); }),
  variant("夜泉汲取(準星左端)", (s) => { const m = s.players[0]; m.aether_draw_secs = 0; m._drawRecvAt = performance.now(); }),
  // 住家窗景（ROADMAP 326）：玩家在室內 → 跑 drawIndoorScene 的北牆開窗分支（homeWindowScene
  // ＋drawHomeWindow＋drawWindowParticles＋窗光）。遍歷時辰／天氣／季節／居家風格，覆蓋
  // 全部天體（sun/lowsun/moon＋星點）與飄落物（rain/sand/dust/mist/snow/none）分支零繪製例外。
  ...(() => {
    const indoor = (extra) => (s) => {
      const me = s.players[0];
      me.indoor_plot_id = 7; me.indoor_x = 128; me.indoor_y = 200;
      me.home_furniture = [{ kind: "bed", emoji: "🛏️", col: 3, row: 3 }];
      Object.assign(me, extra.player || {});
      if (extra.daynight) s.daynight = extra.daynight;
      if (extra.weather) s.weather = extra.weather;
      if (extra.season) s.current_season = extra.season;
    };
    const day = { phase: "day", light: 0.85, night_danger: false };
    const night = { phase: "night", light: 0.12, night_danger: true };
    const dawn = { phase: "dawn", light: 0.5, night_danger: false };
    const dusk = { phase: "dusk", light: 0.45, night_danger: false };
    const clear = { weather_type: "clear", intensity: 0.0 };
    const rain = { weather_type: "grassland_rain", intensity: 0.7 };
    const sand = { weather_type: "desert_sandstorm", intensity: 0.6 };
    const dust = { weather_type: "rocky_crystal_dust", intensity: 0.6 };
    const mist = { weather_type: "water_sea_mist", intensity: 0.6 };
    return [
      variant("室內窗景:白天晴(夏·木屋)", indoor({ daynight: day, weather: clear, season: "summer", player: { home_style: "wood_cabin" } })),
      variant("室內窗景:入夜晴(月+星·星空)", indoor({ daynight: night, weather: clear, season: "summer", player: { home_style: "starlit" } })),
      variant("室內窗景:破曉(低陽·石砌)", indoor({ daynight: dawn, weather: clear, season: "spring", player: { home_style: "stone_hall" } })),
      variant("室內窗景:黃昏降雨(低陽+雨)", indoor({ daynight: dusk, weather: rain, season: "autumn", player: { home_style: "cozy_pastoral" } })),
      variant("室內窗景:白天草原降雨(陽+雨)", indoor({ daynight: day, weather: rain, season: "spring", player: { home_style: "aether_crystal" } })),
      variant("室內窗景:沙漠風沙(飛沙霾)", indoor({ daynight: day, weather: sand, season: "summer", player: { home_style: "wood_cabin" } })),
      variant("室內窗景:岩地晶塵(微光屑)", indoor({ daynight: day, weather: dust, season: "summer", player: { home_style: "aether_crystal" } })),
      variant("室內窗景:水域海霧(白濛)", indoor({ daynight: day, weather: mist, season: "summer", player: { home_style: "stone_hall" } })),
      variant("室內窗景:冬季天晴(季節飄雪)", indoor({ daynight: day, weather: clear, season: "winter", player: { home_style: "cozy_pastoral" } })),
      variant("室內窗景:冬夜(月+星+雪)", indoor({ daynight: night, weather: clear, season: "winter", player: { home_style: "starlit" } })),
    ];
  })(),
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

// ── 探索者路標（ROADMAP 353）──
// 路標走獨立 `wayposts` ServerMsg（非快照）。驗證：收到後 drawWayposts 能渲染「立牌＋近處紙條」
// 兩條路徑零繪製例外（一塊近處＝會浮紙條＋觸發發現、一塊遠處＝只見立牌、一塊即將消失＝漸淡）。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：探索者路標（近處紙條＋遠處立牌＋即將消失，連跑 6 幀）──");
  const wpSnap = JSON.parse(JSON.stringify(snapshot));
  lastWS.onmessage({ data: JSON.stringify({ ...wpSnap, type: "snapshot" }) });
  lastWS.onmessage({ data: JSON.stringify({ type: "wayposts", posts: [
    { id: 1, x: me0.x + 30, y: me0.y + 10, owner_name: "阿光", message_key: "good_view", remaining_secs: 540 },
    { id: 2, x: me0.x + 900, y: me0.y, owner_name: "小美", message_key: "watch_out", remaining_secs: 300 },
    { id: 3, x: me0.x + 80, y: me0.y - 40, owner_name: "旅人", message_key: "bogus_unknown_key", remaining_secs: 12 },
  ] }) });
  const r = pump("探索者路標", 6);
  if (r instanceof Error) { failed = true; console.error("  ❌ 探索者路標：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 探索者路標：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 探索者路標：近處紙條＋遠處立牌＋未知 key 保底＋即將消失漸淡皆乾淨");
}

// ── 星海寄語 / 漂流瓶（ROADMAP 354）──
// 漂流瓶是純面板互動（無世界座標渲染），驗證三條 ServerMsg（海上數量／撈瓶結果／回贈信箱）
// 進來時 message handler ＋ updateBottlePanel 重建 DOM 不拋例外（含撈到瓶顯示回贈鈕、空海保底、
// 未知 key label 保底、信箱列出），且 render 迴圈照樣乾淨。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：星海寄語 / 漂流瓶（海上數量＋撈到瓶＋空海＋回贈信箱，連跑 6 幀）──");
  const bSnap = JSON.parse(JSON.stringify(snapshot));
  lastWS.onmessage({ data: JSON.stringify({ ...bSnap, type: "snapshot" }) });
  let threw = null;
  try {
    // 海上數量。
    lastWS.onmessage({ data: JSON.stringify({ type: "bottle_sea_count", count: 7 }) });
    // 撈到一只瓶（合法 key → 應出現回贈鈕）。
    lastWS.onmessage({ data: JSON.stringify({ type: "bottle_drawn", from_name: "遠方旅人", message_key: "keep_going" }) });
    // 撈到未知 key（label 保底回原字串、不拋）。
    lastWS.onmessage({ data: JSON.stringify({ type: "bottle_drawn", from_name: "旅人", message_key: "bogus_unknown_key" }) });
    // 空海。
    lastWS.onmessage({ data: JSON.stringify({ type: "bottle_drawn", from_name: null, message_key: null }) });
    // 回贈信箱（含一筆缺名、一筆未知 key 的保底）。
    lastWS.onmessage({ data: JSON.stringify({ type: "bottle_inbox", replies: [
      { from_name: "小美", message_key: "smile" },
      { from_name: null, message_key: "bogus_unknown_key" },
    ] }) });
  } catch (e) { threw = e; }
  if (threw) { failed = true; console.error(`  ❌ 星海寄語：message handler 拋例外：${threw && threw.message}`); }
  const r = pump("星海寄語", 6);
  if (r instanceof Error) { failed = true; console.error("  ❌ 星海寄語：未捕捉例外"); }
  const newCaught2 = caughtRenderErrors.slice(before);
  if (newCaught2.length) { failed = true; console.error(`  ❌ 星海寄語：safeRender 攔下 ${newCaught2.length} 個繪製例外（底層真 bug）`); }
  else if (!threw && !(r instanceof Error)) console.log("  ✅ 星海寄語：海上數量＋撈到瓶（回贈鈕）＋空海保底＋未知 key 保底＋信箱列出皆乾淨");
}

// ── 鎮民派系一覽（ROADMAP 355）──
// town_factions 是 snapshot 內欄位（非獨立 ServerMsg），驗證 updateTownFactionsHud 重建 HUD
// 不拋例外：結盟＋敵對列出、未知 bond 保底略過、收合徽章路徑，最後切回「無派系（和平）」面板自動隱去。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：鎮民派系（結盟＋敵對＋未知 bond 保底＋和平隱去，連跑 6 幀）──");
  const fSnap = JSON.parse(JSON.stringify(snapshot));
  fSnap.town_factions = [
    { npc_a: "merchant", npc_b: "procurement_npc", npc_a_name: "商人薇拉", npc_b_name: "採購代理人諾亞", bond: "alliance", affinity: 88 },
    { npc_a: "bounty_npc", npc_b: "workshop_npc", npc_a_name: "獵手蘭卡", npc_b_name: "工匠鐸恩", bond: "rivalry", affinity: 15 },
    { npc_a: "village_chief", npc_b: "farm_fair_npc", npc_a_name: "凱爾長老", npc_b_name: "評審卡特", bond: "bogus_unknown_bond", affinity: 50 },
  ];
  lastWS.onmessage({ data: JSON.stringify({ ...fSnap, type: "snapshot" }) });
  const r = pump("鎮民派系", 6);
  // 再餵一份「無派系」snapshot，驗證面板自動隱去路徑不拋例外。
  const peaceSnap = JSON.parse(JSON.stringify(snapshot));
  peaceSnap.town_factions = [];
  lastWS.onmessage({ data: JSON.stringify({ ...peaceSnap, type: "snapshot" }) });
  const r2 = pump("鎮民派系和平", 2);
  if (r instanceof Error || r2 instanceof Error) { failed = true; console.error("  ❌ 鎮民派系：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 鎮民派系：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error) && !(r2 instanceof Error)) console.log("  ✅ 鎮民派系：結盟＋敵對列出＋未知 bond 保底＋和平自動隱去皆乾淨");
}

// 居民和解委託（ROADMAP 364）：reconcile_offer / reconcile_result 是獨立 ServerMsg，
// 驗證面板渲染（可接委託→進行中→送達回饋）與「鎮上祥和」保底皆不拋例外、有寫入 DOM。
{
  console.log("── 情境：居民和解委託（可接→進行中→送達＋祥和保底）──");
  let ok = true;
  try {
    // 1. 可接的新委託 → renderReconcile 走「列出請求台詞＋接下鈕」分支。
    lastWS.onmessage({ data: JSON.stringify({
      type: "reconcile_offer", available: true, active: false,
      from_id: "bounty_npc", from_name: "獵手蘭卡", to_id: "workshop_npc", to_name: "工匠鐸恩",
      to_x: 2120, to_y: 2080, token: "一包野地帶回的乾糧",
      plea: "獵手蘭卡：我和工匠鐸恩前陣子鬧得有點僵……能替我捎去嗎？",
    }) });
    // 2. 進行中的委託 → 走「交付指引＋就近提示＋交付鈕」分支（含本機距離估算）。
    lastWS.onmessage({ data: JSON.stringify({
      type: "reconcile_offer", available: true, active: true,
      from_id: "bounty_npc", from_name: "獵手蘭卡", to_id: "workshop_npc", to_name: "工匠鐸恩",
      to_x: 2120, to_y: 2080, token: "一包野地帶回的乾糧", plea: "",
    }) });
    // 3. 接下回饋（accepted 分支→重新請求）。
    lastWS.onmessage({ data: JSON.stringify({
      type: "reconcile_result", ok: true, accepted: true, done: false,
      from_name: "獵手蘭卡", to_name: "工匠鐸恩", warmth: 0, reward_ether: 0,
    }) });
    // 4. 送達成功回饋（done 分支→給獎飄字＋刷新）。
    lastWS.onmessage({ data: JSON.stringify({
      type: "reconcile_result", ok: true, accepted: false, done: true,
      from_name: "獵手蘭卡", to_name: "工匠鐸恩", warmth: 61, reward_ether: 8,
    }) });
    // 5. 交付太遠回饋（ok=false 分支→提示再靠近）。
    lastWS.onmessage({ data: JSON.stringify({
      type: "reconcile_result", ok: false, accepted: false, done: false,
      from_name: "獵手蘭卡", to_name: "工匠鐸恩", warmth: 0, reward_ether: 0,
    }) });
    // 6. 鎮上祥和保底（available=false 分支）。
    lastWS.onmessage({ data: JSON.stringify({
      type: "reconcile_offer", available: false, active: false,
      from_id: "", from_name: "", to_id: "", to_name: "", to_x: 0, to_y: 0, token: "", plea: "",
    }) });
  } catch (e) {
    ok = false; console.error("  ❌ 居民和解委託：拋出例外", e && e.message);
  }
  if (!ok) failed = true;
  else console.log("  ✅ 居民和解委託：可接→進行中→接下→送達→太遠→祥和保底全分支皆乾淨、無例外");
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

// 冬日寒雀（238）：寒雀多半原地啄食、偶爾忽地短距蹦躍，蹦躍中才跑「拋物線抬升＋展翅」分支；
// 蹦跳頻率低（每循環 ~4~7s），6 幀的一般情境碰不到蹦躍中段，故這裡連跑 ~480 幀（每幀 +16ms ≈ 7.7s）
// 實跑「啄食點頭→停—衝短距蹦躍→拋物線弧＋展翅→落地續啄」完整路徑（彩蝶 236／蜻蜓 237 的冬日對偶）。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：冬日（冬日寒雀，連跑 480 幀觸發蹦躍弧＋展翅）──");
  const winterSnap = JSON.parse(JSON.stringify(snapshot));
  winterSnap.current_season = "winter";
  winterSnap.daynight = { phase: "day", light: 0.82, night_danger: false };
  winterSnap.weather = { weather_type: "clear", intensity: 0.0 };
  lastWS.onmessage({ data: JSON.stringify({ ...winterSnap, type: "snapshot" }) });
  const r = pump("冬日寒雀蹦躍", 480);
  if (r instanceof Error) { failed = true; console.error("  ❌ 冬日寒雀蹦躍：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 冬日寒雀蹦躍：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 冬日寒雀蹦躍：乾淨");
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

// 雷光驚畜（248）：暴雨閃電（243）乍亮時，畫面上的野生動物被雷光嚇得縮身下伏。閃電首記延遲
// LIGHTNING_FIRST_DELAY_MS（1.2s）才點燃、單記僅 750ms，6 幀的一般情境碰不到泛光中段，故連跑
// ~200 幀（每幀 +16ms）讓 _lightningFlash>0 的幀實跑 drawWildlife 的 cowerDuck 縮身位移分支。
// 同時擺上鹿/狼/狐/鳥/小獸五種，確認各種動物在雷光下縮身繪製零例外（暴雨 grassland_rain intensity
// 0.9 > LIGHTNING_STORM_INTENSITY 0.55 才打雷）。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：暴雨（雷光驚畜，連跑 200 幀觸發閃電泛光與動物縮身）──");
  const stormSnap = JSON.parse(JSON.stringify(snapshot));
  stormSnap.daynight = { phase: "day", light: 0.7, night_danger: false };
  stormSnap.weather = { weather_type: "grassland_rain", intensity: 0.9 };
  const sm0 = stormSnap.players[0];
  if (stormSnap.wildlife?.length) {
    stormSnap.wildlife[0] = { ...stormSnap.wildlife[0], kind: "wild_deer", x: sm0.x + 30, y: sm0.y, state: "wandering" };
    if (stormSnap.wildlife[1]) stormSnap.wildlife[1] = { ...stormSnap.wildlife[1], kind: "wild_wolf", x: sm0.x - 30, y: sm0.y + 20, state: "wandering" };
    if (stormSnap.wildlife[2]) stormSnap.wildlife[2] = { ...stormSnap.wildlife[2], kind: "wild_fox", x: sm0.x, y: sm0.y + 40, state: "wandering" };
    if (stormSnap.wildlife[3]) stormSnap.wildlife[3] = { ...stormSnap.wildlife[3], kind: "wild_bird", x: sm0.x + 50, y: sm0.y - 20, state: "wandering" };
    if (stormSnap.wildlife[4]) stormSnap.wildlife[4] = { ...stormSnap.wildlife[4], kind: "small_critter", x: sm0.x - 50, y: sm0.y - 30, state: "wandering" };
  }
  lastWS.onmessage({ data: JSON.stringify({ ...stormSnap, type: "snapshot" }) });
  const r = pump("雷光驚畜", 200);
  if (r instanceof Error) { failed = true; console.error("  ❌ 雷光驚畜：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 雷光驚畜：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 雷光驚畜：乾淨");
}

// 四季草色（235）：地面隨季染色（春嫩／夏基準／秋金／冬霜），跨季以 _groundTint 逐幀 lerp。
// 每幀 updateGroundTint→drawGround 內掃一層季節色＋drawGrassTuft 同步染色。逐季切換並各連跑 ~40 幀
//（每幀 +16ms ≈ 0.64s，足夠讓 GROUND_TINT_RATE 0.35/s 的 lerp 從上一季色明顯轉向當季色），實跑
// seasonGroundTintTarget／groundTintStep／applyGroundTint 三條純函式路徑與地表掃色 fillRect、且
// 涵蓋「換季轉場」（非僅穩態），確認跨季 lerp 與草叢染色全程零繪製例外。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：四季草色（地面隨季染色，逐季切換各跑 40 幀觸發跨季 lerp 轉場）──");
  const grassSnap = JSON.parse(JSON.stringify(snapshot));
  grassSnap.daynight = { phase: "day", light: 0.85, night_danger: false };
  grassSnap.weather = { weather_type: "clear", intensity: 0.0 };
  let grassErr = false;
  for (const season of ["spring", "summer", "autumn", "winter", "spring"]) {
    grassSnap.current_season = season;
    lastWS.onmessage({ data: JSON.stringify({ ...grassSnap, type: "snapshot" }) });
    const r = pump(`四季草色 ${season}`, 40);
    if (r instanceof Error) { grassErr = true; break; }
  }
  if (grassErr) { failed = true; console.error("  ❌ 四季草色：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 四季草色：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!grassErr) console.log("  ✅ 四季草色：乾淨");
}

// 冬日飄雪（226）：雪花緩降（vy ~26）需數百毫秒才落出畫面下緣，連跑 ~200 幀（每幀 +16ms ≈ 3.2s）
// 才實跑「淡入→撒雪花→緩降搖擺→落地 despawn→補新雪→薄霜白幕」完整迴圈（一般 6 幀只碰到開頭）。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：冬季（冬日飄雪，連跑 200 幀觸發雪花完整落下與補充）──");
  const winterSnap = JSON.parse(JSON.stringify(snapshot));
  winterSnap.current_season = "winter";
  winterSnap.daynight = { phase: "day", light: 0.82, night_danger: false };
  winterSnap.weather = { weather_type: "clear", intensity: 0.0 };
  lastWS.onmessage({ data: JSON.stringify({ ...winterSnap, type: "snapshot" }) });
  const r = pump("冬日飄雪", 200);
  if (r instanceof Error) { failed = true; console.error("  ❌ 冬日飄雪：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 冬日飄雪：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 冬日飄雪：乾淨");
}

// 秋日落葉（227）：落葉緩降（vy ~24）需數百毫秒才落出畫面下緣，連跑 ~200 幀（每幀 +16ms ≈ 3.2s）
// 才實跑「淡入→撒葉→打旋搖擺緩降→落地 despawn→補新葉→暖金薄幕」完整迴圈（一般 6 幀只碰到開頭）。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：秋季（秋日落葉，連跑 200 幀觸發落葉完整落下與補充）──");
  const autumnSnap = JSON.parse(JSON.stringify(snapshot));
  autumnSnap.current_season = "autumn";
  autumnSnap.daynight = { phase: "day", light: 0.82, night_danger: false };
  autumnSnap.weather = { weather_type: "clear", intensity: 0.0 };
  lastWS.onmessage({ data: JSON.stringify({ ...autumnSnap, type: "snapshot" }) });
  const r = pump("秋日落葉", 200);
  if (r instanceof Error) { failed = true; console.error("  ❌ 秋日落葉：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 秋日落葉：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 秋日落葉：乾淨");
}

// 春日花飛（228）：花瓣緩降（vy ~20）兼受風橫飄（vx ~8~26）需數百毫秒才飄出畫面下緣/右緣，連跑
// ~200 幀（每幀 +16ms ≈ 3.2s）才實跑「淡入→撒瓣→橫飄翻轉緩降→飄出 despawn→補新瓣→淡粉薄幕」完整迴圈。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：春季（春日花飛，連跑 200 幀觸發花瓣完整飄落與補充）──");
  const springSnap = JSON.parse(JSON.stringify(snapshot));
  springSnap.current_season = "spring";
  springSnap.daynight = { phase: "day", light: 0.82, night_danger: false };
  springSnap.weather = { weather_type: "clear", intensity: 0.0 };
  lastWS.onmessage({ data: JSON.stringify({ ...springSnap, type: "snapshot" }) });
  const r = pump("春日花飛", 200);
  if (r instanceof Error) { failed = true; console.error("  ❌ 春日花飛：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 春日花飛：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 春日花飛：乾淨");
}

// 夏日蟬夏（229）：夏絮緩緩上飄（vy ~ -10~-30）兼受熱氣橫移（vx ~6~20）需數百毫秒才飄出畫面上緣/右緣，連跑
// ~200 幀（每幀 +16ms ≈ 3.2s）才實跑「淡入→撒絮→上飄閃爍橫盪→飄出 despawn→補新絮→暖金薄幕」完整迴圈。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：夏季（夏日蟬夏，連跑 200 幀觸發夏絮完整上飄與補充）──");
  const summerSnap = JSON.parse(JSON.stringify(snapshot));
  summerSnap.current_season = "summer";
  summerSnap.daynight = { phase: "day", light: 0.82, night_danger: false };
  summerSnap.weather = { weather_type: "clear", intensity: 0.0 };
  lastWS.onmessage({ data: JSON.stringify({ ...summerSnap, type: "snapshot" }) });
  const r = pump("夏日蟬夏", 200);
  if (r instanceof Error) { failed = true; console.error("  ❌ 夏日蟬夏：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 夏日蟬夏：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 夏日蟬夏：乾淨");
}

// 冬夜極光（231）：current_season=winter ＋ 夜（light 0.12 < 0.42）→ 跑 drawAurora 的繪製分支
//（極光勢淡入→三條簾幕沿欄取樣描波動帶→垂直漸層加色填滿）。連跑 ~300 幀（每幀 +16ms ≈ 4.8s）才實
// 跑完 _auroraFade 從 0 緩緩淡入滿（AURORA_FADE_RATE 0.25 約需 4s）後的完整繪製路徑。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：冬夜（冬夜極光，連跑 300 幀觸發極光淡入與簾幕繪製）──");
  const auroraSnap = JSON.parse(JSON.stringify(snapshot));
  auroraSnap.current_season = "winter";
  auroraSnap.daynight = { phase: "night", light: 0.12, night_danger: true };
  auroraSnap.weather = { weather_type: "clear", intensity: 0.0 };
  lastWS.onmessage({ data: JSON.stringify({ ...auroraSnap, type: "snapshot" }) });
  const r = pump("冬夜極光", 300);
  if (r instanceof Error) { failed = true; console.error("  ❌ 冬夜極光：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 冬夜極光：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 冬夜極光：乾淨");
}

// 夏夜銀河（232）：current_season=summer ＋ 夜（light 0.12 < 0.42）→ 跑 drawGalaxy 的繪製分支
//（銀河勢淡入→惰性生成微星佈局→乳白銀霧柔光圓相疊→帶上密集微星沿帶落位閃爍）。連跑 ~300 幀
//（每幀 +16ms ≈ 4.8s）才實跑完 _galaxyFade 從 0 緩緩淡入滿（GALAXY_FADE_RATE 0.25 約需 4s）後的完整繪製路徑。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：夏夜（夏夜銀河，連跑 300 幀觸發銀河淡入與星帶繪製）──");
  const galaxySnap = JSON.parse(JSON.stringify(snapshot));
  galaxySnap.current_season = "summer";
  galaxySnap.daynight = { phase: "night", light: 0.12, night_danger: true };
  galaxySnap.weather = { weather_type: "clear", intensity: 0.0 };
  lastWS.onmessage({ data: JSON.stringify({ ...galaxySnap, type: "snapshot" }) });
  const r = pump("夏夜銀河", 300);
  if (r instanceof Error) { failed = true; console.error("  ❌ 夏夜銀河：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 夏夜銀河：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 夏夜銀河：乾淨");
}

// 春夜螢火（233）：current_season=spring ＋ 夜（light 0.12 < 0.42）→ 跑 drawFireflies 的繪製分支
//（螢火季勢淡入→惰性生成螢火池→各螢火緩游、三次方脈衝明滅、黃綠柔光暈）。連跑 ~300 幀
//（每幀 +16ms ≈ 4.8s）才實跑完 _fireflyFade 從 0 緩緩淡入滿（FIREFLY_FADE_RATE 0.25 約需 4s）後的完整繪製路徑。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：春夜（春夜螢火，連跑 300 幀觸發螢火淡入與明滅游移繪製）──");
  const fireflySnap = JSON.parse(JSON.stringify(snapshot));
  fireflySnap.current_season = "spring";
  fireflySnap.daynight = { phase: "night", light: 0.12, night_danger: true };
  fireflySnap.weather = { weather_type: "clear", intensity: 0.0 };
  lastWS.onmessage({ data: JSON.stringify({ ...fireflySnap, type: "snapshot" }) });
  const r = pump("春夜螢火", 300);
  if (r instanceof Error) { failed = true; console.error("  ❌ 春夜螢火：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 春夜螢火：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 春夜螢火：乾淨");
}

// 秋夜薄霧（234）：current_season=autumn ＋ 夜（light 0.12 < 0.42）→ 跑 drawAutumnMist 的繪製分支
//（秋霧勢淡入→惰性生成薄霧團池→各霧團橫向緩流、勻緩呼吸、寬扁銀白柔光暈）。連跑 ~300 幀
//（每幀 +16ms ≈ 4.8s）才實跑完 _autumnMistFade 從 0 緩緩淡入滿（MIST_FADE_RATE 0.25 約需 4s）後的完整繪製路徑。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：秋夜（秋夜薄霧，連跑 300 幀觸發薄霧淡入與橫流呼吸繪製）──");
  const mistSnap = JSON.parse(JSON.stringify(snapshot));
  mistSnap.current_season = "autumn";
  mistSnap.daynight = { phase: "night", light: 0.12, night_danger: true };
  mistSnap.weather = { weather_type: "clear", intensity: 0.0 };
  lastWS.onmessage({ data: JSON.stringify({ ...mistSnap, type: "snapshot" }) });
  const r = pump("秋夜薄霧", 300);
  if (r instanceof Error) { failed = true; console.error("  ❌ 秋夜薄霧：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 秋夜薄霧：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 秋夜薄霧：乾淨");
}

// 雷雨閃電（243）：草原暴雨（grassland_rain，intensity 0.85 > 0.55）→ 跑 drawLightning 的繪製分支
//（首記延遲 ~1.2s 後點燃→全屏冷藍白泛光急衰＋分叉電光一瞬→熄滅→排下一記）。首記延遲 ~75 幀、
// 單記 ~47 幀，故連跑 ~200 幀（每幀 +16ms ≈ 3.2s）實跑完整「點燃→泛光＋電光→熄滅→排下一記」路徑。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：草原暴雨（雷雨閃電，連跑 200 幀觸發閃電泛光＋分叉電光）──");
  const stormSnap = JSON.parse(JSON.stringify(snapshot));
  stormSnap.daynight = { phase: "day", light: 0.6, night_danger: false }; // 陰雨白天
  stormSnap.weather = { weather_type: "grassland_rain", intensity: 0.85 }; // 暴雨（> 0.55 門檻）
  lastWS.onmessage({ data: JSON.stringify({ ...stormSnap, type: "snapshot" }) });
  const r = pump("雷雨閃電", 200);
  if (r instanceof Error) { failed = true; console.error("  ❌ 雷雨閃電：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 雷雨閃電：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 雷雨閃電：乾淨");
}

// 雨打水面漣漪（247）：單個漣漪源一輪「濺起→擴散→淡盡」週期 ~1.4s，6 幀（~96ms）只碰到剛濺起那一小段；
// 連跑 ~200 幀（每幀 +16ms ≈ 3.2s）才實跑「濺起小亮環→擴散變大淡出→淡盡→換 cyc.idx 落點重濺一輪」
// 完整迴圈與內外雙環路徑（一般 6 幀情境碰不到淡盡換落點那一段）。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：水域降雨（雨打水面漣漪，連跑 200 幀觸發漣漪擴散→淡盡→換落點重濺）──");
  const rainSnap = JSON.parse(JSON.stringify(snapshot));
  rainSnap.players[0].x = -4400; rainSnap.players[0].y = -3000; // 成片水域
  rainSnap.daynight = { phase: "day", light: 0.7, night_danger: false };
  rainSnap.weather = { weather_type: "grassland_rain", intensity: 0.7 };
  lastWS.onmessage({ data: JSON.stringify({ ...rainSnap, type: "snapshot" }) });
  const r = pump("雨打水面漣漪", 200);
  if (r instanceof Error) { failed = true; console.error("  ❌ 雨打水面漣漪：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 雨打水面漣漪：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 雨打水面漣漪：乾淨");
}

// 雨後彩虹（ROADMAP 361）：伺服器權威全服天象。送 rainbow.active=true 的快照 → 跑彩虹弧
// 「淡入→駐留」與「彩虹祝福」HUD pill 路徑；再送 active=false → 跑「伺服器宣告結束→淡出→熄滅」
// 路徑（一般情境碰不到的淡出分支）。drawRainbow 改伺服器驅動後，這條確保兩個生命週期分支零例外。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：雨後彩虹（全服天象，連跑驅動淡入駐留→宣告結束淡出，含祝福 HUD pill）──");
  const rbSnap = JSON.parse(JSON.stringify(snapshot));
  rbSnap.daynight = { phase: "day", light: 0.7, night_danger: false }; // 雨過天青、白天
  rbSnap.weather = { weather_type: "clear", intensity: 0.0 };
  rbSnap.rainbow = { active: true, remaining_secs: 45 };
  lastWS.onmessage({ data: JSON.stringify({ ...rbSnap, type: "snapshot" }) });
  let r = pump("雨後彩虹·點燃", 120); // 淡入後駐留
  // 伺服器宣告彩虹結束 → 前端進入淡出分支。
  if (!(r instanceof Error)) {
    const offSnap = JSON.parse(JSON.stringify(rbSnap));
    offSnap.rainbow = { active: false, remaining_secs: 0 };
    lastWS.onmessage({ data: JSON.stringify({ ...offSnap, type: "snapshot" }) });
    r = pump("雨後彩虹·淡出", 360); // 淡出 4.5s 內熄滅、釋放狀態
  }
  if (r instanceof Error) { failed = true; console.error("  ❌ 雨後彩虹：未捕捉例外"); }
  const newCaught = caughtRenderErrors.slice(before);
  if (newCaught.length) { failed = true; console.error(`  ❌ 雨後彩虹：safeRender 攔下 ${newCaught.length} 個繪製例外（底層真 bug）`); }
  else if (!(r instanceof Error)) console.log("  ✅ 雨後彩虹：乾淨");
}

console.log("");
if (failed) {
  console.error("🔴 render-smoke 發現繪製例外（見上）。safeRender 雖防止凍結，但應根治根因。");
  process.exit(1);
}
console.log("✅✅ render-smoke 全綠：所有情境（含屍光/商人/態度越界/未知物種）連跑多幀，render 零例外、safeRender 零攔截。");
process.exit(0);
