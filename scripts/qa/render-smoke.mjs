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
    // 主遊戲畫布共用 ctxSingleton；其餘任何元素（含程式 createElement 的離屏 canvas，
    // 如畫風縮圖 styleThumb*、黏土地面烘焙離屏 canvas）按需各給一個各自的假 2d ctx，
    // 讓 getContext("2d") 永不回 null（真實瀏覽器上 canvas 一定有 2d ctx）。
    getContext: (type) => {
      if (type && type !== "2d") return null;
      if (isCanvas) return ctxSingleton;
      return real.__ctx || (real.__ctx = makeCtx(real));
    },
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
  // 黏土角色轉身（420）：玩家朝左移動 → 在 clay 模式走 player_side 側面圖路徑（pickClayPlayerSprite）。
  // 朝右另設一名其他玩家走 flip=false 分支。零 render 例外即通過（BUTFUN_SMOKE_STYLE=clay 跑到）。
  variant("黏土玩家側走轉身(420)", (s) => {
    if (s.players?.length) {
      s.players[0] = { ...s.players[0], facing: Math.PI, moving: true, walk: 2 };       // 朝左
      if (s.players[1]) s.players[1] = { ...s.players[1], facing: 0, moving: true, walk: 1 }; // 朝右
    }
  }),
  // 黏土日影晷（428）：破曉低光（_shadowCast 非 null、拉長偏移）下，讓黏土世界的物件（採集節點／
  // 作物／星晶）走 drawClaySprite→drawGroundShadow 的方向性投影路徑。clay 模式（BUTFUN_SMOKE_STYLE=clay）
  // 才會走黏土 sprite；零 render 例外即通過（驗證接線後低光偏移/拉長/調色不拋例外）。
  variant("黏土日影晷破曉投影(428)", (s) => {
    s.daynight = { phase: "dawn", light: 0.25, night_danger: false };
    // 確保場上有會落黏土影子的物件：採集節點（樹/石）＋星晶。破曉低光下 _shadowCast 非 null，
    // drawClaySprite→drawGroundShadow 會走「偏移＋拉長＋調色」分支（clay 模式才走黏土 sprite）。
    s.nodes = [{ kind: "tree", x: me0.x + 40, y: me0.y, remaining: 5, harvestable: true },
               { kind: "rock", x: me0.x - 36, y: me0.y + 18, remaining: 5, harvestable: true }];
    s.star_crystals = [{ id: 903, x: me0.x + 60, y: me0.y - 20 }];
  }),
  // 個人地塊作物熟成進度條（ROADMAP 457）：給玩家腳邊的地塊注入幾株作物——含成長中（青綠／暖金門檻
  // 兩側）與已成熟的——驗證 drawCropSlotBar 在 clay 與 emoji 兩條渲染路徑都不拋例外。把 plot 0 移到
  // 玩家身邊（me0≈3000,3000；TILE_PX=32→格 ~93）才畫得到（否則視野外剔除、不跑進度條）。
  variant("個人地塊作物熟成進度條(457)", (s) => {
    s.land_plots = JSON.parse(JSON.stringify(s.land_plots || []));
    if (s.land_plots[0]) {
      Object.assign(s.land_plots[0], { plot_id: 0, min_gx: 90, min_gy: 90, max_gx: 97, max_gy: 97, owner_id: me0.id, purpose: "farm" });
    }
    s.farm_crop_plots = [{ plot_id: 0, crops: [
      { kind: "wheat",  ripe: false, grow: 30 },   // 成長中·青綠
      { kind: "carrot", ripe: false, grow: 90 },   // 即將成熟·暖金（≥80%）
      { kind: "potato", ripe: true,  grow: 100 },  // 已成熟·✅
    ] }];
  }),
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
  // 鎮民陣營（ROADMAP 366）：驗證「陣營」段渲染（核心 👑＋成員＋凝聚配色）、size<3 保底略過皆不拋例外。
  fSnap.town_blocs = [
    { members: ["merchant", "workshop_npc", "bounty_npc"], member_names: ["商人薇拉", "工匠鐸恩", "獵手蘭卡"], figurehead: "workshop_npc", figurehead_name: "工匠鐸恩", cohesion: 86 },
    { members: ["village_chief", "farm_fair_npc"], member_names: ["凱爾長老", "評審卡特"], figurehead: "village_chief", figurehead_name: "凱爾長老", cohesion: 90 },
  ];
  // 親手植樹成蔭（ROADMAP 370）：注入四個成長階段的世界樹（含視野外一棵、畸形 stage），
  // 驗證 drawWorldGroves 各階段圖示／柔影／擺動／剔除路徑皆不拋例外。
  fSnap.world_groves = [
    { x: 120, y: 140, stage: 0 },   // 🌱 嫩芽
    { x: 200, y: 160, stage: 1 },   // 🌿 樹苗
    { x: 280, y: 180, stage: 2 },   // 🌲 幼樹
    { x: 360, y: 200, stage: 3 },   // 🌳 大樹
    { x: 99999, y: 99999, stage: 3 }, // 視野外：應被剔除
    { x: 150, y: 150, stage: 7 },   // 畸形 stage：應被夾到合法範圍
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

// 用心栽培·作物品質（ROADMAP 406）：成熟作物依品質碼點光點（優質 2 金星／用心 1 綠點／平凡 0 不點），
// 收成事件 harvest_result 走自己 id 的飄字。驗證 drawQualityGlint 與 harvest_result handler 皆不拋例外。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：用心栽培作物品質（成熟品質光點＋收成飄字 premium/fine/plain）＋豐收迸發（458 金光揚穀）──");
  let ok = true;
  try {
    const qSnap = JSON.parse(JSON.stringify(snapshot));
    if (qSnap.fields && qSnap.fields[0] && Array.isArray(qSnap.fields[0].cells)) {
      const cells = qSnap.fields[0].cells;
      // 把前幾格設成成熟＋不同品質，逼出金星／綠點／不點三條繪製分支。
      if (cells[0]) cells[0] = { ...cells[0], state: 4, dry: false, quality: 2 }; // 優質金星
      if (cells[1]) cells[1] = { ...cells[1], state: 4, dry: false, quality: 1 }; // 用心綠點
      if (cells[2]) cells[2] = { ...cells[2], state: 4, dry: false, quality: 0 }; // 平凡不點
    }
    lastWS.onmessage({ data: JSON.stringify({ ...qSnap, type: "snapshot" }) });
    pump("作物品質光點", 4);
    // 收成飄字：三種品質各一則（自己 id 才演出）。
    for (const q of ["premium", "fine", "plain"]) {
      lastWS.onmessage({ data: JSON.stringify({ type: "harvest_result", player_id: myId, quality: q, ether: 5, x: me0.x, y: me0.y }) });
    }
    // 旁觀者（別人 id）應被忽略、不拋例外。
    lastWS.onmessage({ data: JSON.stringify({ type: "harvest_result", player_id: "someone_else", quality: "premium", ether: 5, x: me0.x, y: me0.y }) });
    pump("作物收成飄字", 3);
  } catch (e) {
    ok = false; console.error("  ❌ 用心栽培作物品質：拋出例外", e && e.message);
  }
  const newCaught = caughtRenderErrors.slice(before);
  if (!ok || newCaught.length) { failed = true; console.error(`  ❌ 用心栽培作物品質：${newCaught.length} 個繪製例外`); }
  else console.log("  ✅ 用心栽培作物品質：品質光點三分支＋收成飄字三品質＋豐收迸發（458）spawn/draw＋旁觀者忽略皆乾淨");
}

// 一鍵收成（ROADMAP 446）：田裡有成熟作物時，hudRipe 那行同時當「✨一鍵收成」按鈕——對稱於
// 缺水行的一鍵澆水。驗證 updateFarmHud 把 ripeEl 接成可點、點下會送 harvest_all；無成熟時解除點按。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：一鍵收成（hudRipe 可點→送 harvest_all、無成熟則解除）──");
  let ok = true;
  try {
    // 有成熟作物的快照（fields[0] 設成自己的地，updateFarmHud 才會數到自家田）。
    const hSnap = JSON.parse(JSON.stringify(snapshot));
    if (hSnap.fields && hSnap.fields[0] && Array.isArray(hSnap.fields[0].cells)) {
      hSnap.fields[0].owner = myId;
      const cells = hSnap.fields[0].cells;
      if (cells[0]) cells[0] = { ...cells[0], state: 4, dry: false, quality: 2 }; // 成熟可收
      if (cells[1]) cells[1] = { ...cells[1], state: 4, dry: false, quality: 0 }; // 成熟可收
    }
    lastWS.onmessage({ data: JSON.stringify({ ...hSnap, type: "snapshot" }) });
    pump("一鍵收成 HUD", 2);
    // 點 hudRipe 應送出 harvest_all（暫時攔截 send 擷取，不污染其他情境）。
    const ripeEl = documentStub.getElementById("hudRipe");
    let sent = null;
    const origSend = lastWS.send;
    lastWS.send = (s) => { sent = s; };
    if (typeof ripeEl.onclick === "function") ripeEl.onclick();
    lastWS.send = origSend;
    const msg = sent ? JSON.parse(sent) : null;
    if (!msg || msg.type !== "harvest_all") {
      ok = false; console.error("  ❌ 一鍵收成：點 hudRipe 沒送出 harvest_all，實得", sent);
    }
    // 無成熟作物的快照 → hudRipe 應解除點按（onclick 清成 null）。
    const emptySnap = JSON.parse(JSON.stringify(snapshot));
    if (emptySnap.fields && emptySnap.fields[0] && Array.isArray(emptySnap.fields[0].cells)) {
      emptySnap.fields[0].owner = myId;
      for (const c of emptySnap.fields[0].cells) { if (c) { c.state = 1; c.dry = false; } } // 全空土、無成熟
    }
    lastWS.onmessage({ data: JSON.stringify({ ...emptySnap, type: "snapshot" }) });
    pump("一鍵收成 HUD 解除", 2);
    const ripeEl2 = documentStub.getElementById("hudRipe");
    if (ripeEl2.onclick) {
      ok = false; console.error("  ❌ 一鍵收成：無成熟作物時 hudRipe 仍綁著點按（未解除）");
    }
  } catch (e) {
    ok = false; console.error("  ❌ 一鍵收成：拋出例外", e && e.message);
  }
  const newCaught2 = caughtRenderErrors.slice(before);
  if (!ok || newCaught2.length) { failed = true; console.error(`  ❌ 一鍵收成：${newCaught2.length} 個繪製例外`); }
  else console.log("  ✅ 一鍵收成：有成熟→可點送 harvest_all、無成熟→解除點按皆乾淨");
}

// 臨陣格擋（ROADMAP 408）：備防中的玩家頭頂畫脈動格擋環（drawGuardRing），上盾時身上罩乙太藍護盾微光；
// guard_result 事件對自己 id 演出三檔飄字（完美／一部分／沒擋好），旁觀者忽略。連跑多幀逼出相位推進，
// 並把護盾百分比設到封頂值試卸傷強度上限分支。驗證格擋環＋護盾微光＋三檔飄字皆不拋例外。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：臨陣格擋（格擋環脈動＋護盾微光＋格擋飄字 perfect/partial/whiff）──");
  let ok = true;
  try {
    const gSnap = JSON.parse(JSON.stringify(snapshot));
    // 自己備防中（guard_secs 觸發格擋環）＋上盾（guard_shield_pct 觸發護盾微光，設到封頂 85 試強度上限）。
    if (gSnap.players && gSnap.players[0]) {
      gSnap.players[0] = { ...gSnap.players[0], guard_secs: 0.5, guard_shield_pct: 85 };
    }
    // 旁觀者（第二名玩家）也在格擋，驗證別人 id 的格擋環同樣乾淨繪製。
    if (gSnap.players && gSnap.players[1]) {
      gSnap.players[1] = { ...gSnap.players[1], guard_secs: 1.0, guard_shield_pct: 40 };
    }
    lastWS.onmessage({ data: JSON.stringify({ ...gSnap, type: "snapshot" }) });
    pump("格擋環＋護盾微光", 8); // 連跑多幀逼出相位推進與脈動
    // 三檔結果飄字：自己 id 才演出。
    for (const outcome of ["perfect", "partial", "whiff"]) {
      lastWS.onmessage({ data: JSON.stringify({ type: "guard_result", player_id: myId, outcome, x: me0.x, y: me0.y }) });
    }
    // 旁觀者（別人 id）應被忽略、不拋例外。
    lastWS.onmessage({ data: JSON.stringify({ type: "guard_result", player_id: "someone_else", outcome: "perfect", x: me0.x, y: me0.y }) });
    pump("格擋飄字", 3);
  } catch (e) {
    ok = false; console.error("  ❌ 臨陣格擋：拋出例外", e && e.message);
  }
  const newCaught = caughtRenderErrors.slice(before);
  if (!ok || newCaught.length) { failed = true; console.error(`  ❌ 臨陣格擋：${newCaught.length} 個繪製例外`); }
  else console.log("  ✅ 臨陣格擋：格擋環脈動＋護盾微光＋三檔飄字＋旁觀者忽略皆乾淨");
}

// 蓄力重擊（ROADMAP 423）：蓄力中的玩家頭頂畫逐漸填滿的蓄力環（charge_progress 0→滿蓄脈動）；
// attack_hit 帶 charge_tier 時演出更大的熾金/琥珀重擊傷害飄字。連跑多幀逼出滿蓄脈動分支。
// 驗證半蓄環／滿蓄環（自己＋旁觀者）＋半蓄/滿蓄/普通命中飄字皆不拋例外。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：蓄力重擊（蓄力環填滿＋滿蓄脈動＋重擊飄字 half/full）──");
  let ok = true;
  try {
    const cSnap = JSON.parse(JSON.stringify(snapshot));
    // 自己滿蓄（charge_progress=1 觸發熾金脈動環＋「滿蓄！」提示）。
    if (cSnap.players && cSnap.players[0]) {
      cSnap.players[0] = { ...cSnap.players[0], charge_progress: 1.0 };
    }
    // 旁觀者（第二名玩家）半蓄中（charge_progress=0.6 觸發琥珀環、無提示文字）。
    if (cSnap.players && cSnap.players[1]) {
      cSnap.players[1] = { ...cSnap.players[1], charge_progress: 0.6 };
    }
    lastWS.onmessage({ data: JSON.stringify({ ...cSnap, type: "snapshot" }) });
    pump("蓄力環填滿＋滿蓄脈動", 8); // 連跑多幀逼出滿蓄脈動
    // 三種命中飄字：滿蓄(2)💥／半蓄(1)⚡／普通(0)，皆不拋例外。
    for (const charge_tier of [2, 1, 0]) {
      lastWS.onmessage({ data: JSON.stringify({ type: "attack_hit", player_id: myId, ex: me0.x + 20, ey: me0.y, dmg: 88, is_kill: false, is_crit: false, charge_tier }) });
    }
    pump("重擊飄字", 3);
  } catch (e) {
    ok = false; console.error("  ❌ 蓄力重擊：拋出例外", e && e.message);
  }
  const newCaught = caughtRenderErrors.slice(before);
  if (!ok || newCaught.length) { failed = true; console.error(`  ❌ 蓄力重擊：${newCaught.length} 個繪製例外`); }
  else console.log("  ✅ 蓄力重擊：蓄力環填滿＋滿蓄脈動＋重擊飄字（half/full/普通）＋旁觀者皆乾淨");
}

// 怪物王預警重擊（ROADMAP 424）：兇名精英 enemies[0] 帶 slam_windup（地面預警圈逐漸填滿、近滿脈動）；
// 蓄滿後伺服器廣播 boss_slam，於落點演出向外炸開的衝擊波。驗證：蓄力圈（半蓄→近滿）＋落點衝擊波
// （自己在圈內：警示飄字＋報讀／自己在圈外：純衝擊波）皆不拋例外。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：怪物王預警重擊（地面預警圈填滿＋衝擊波，自己在圈內/圈外）──");
  let ok = true;
  try {
    // 半蓄中的怪物王（slam_windup=0.4 → 預警圈淡填）。
    const s1 = JSON.parse(JSON.stringify(snapshot));
    if (s1.enemies && s1.enemies[0]) {
      s1.enemies[0] = { ...s1.enemies[0], x: me0.x + 60, y: me0.y, level: 8, alive: true, notorious: true, slam_windup: 0.4 };
    }
    lastWS.onmessage({ data: JSON.stringify({ ...s1, type: "snapshot" }) });
    pump("預警圈半蓄", 4);
    // 近蓄滿（slam_windup=0.97 → 預警圈幾乎填滿、外環脈動）。
    const s2 = JSON.parse(JSON.stringify(s1));
    if (s2.enemies && s2.enemies[0]) {
      s2.enemies[0] = { ...s2.enemies[0], slam_windup: 0.97 };
    }
    lastWS.onmessage({ data: JSON.stringify({ ...s2, type: "snapshot" }) });
    pump("預警圈近滿脈動", 6);
    // 重擊落下：自己在圈內（落點 = me0 附近）→ 警示飄字＋報讀＋衝擊波。
    lastWS.onmessage({ data: JSON.stringify({ type: "boss_slam", x: me0.x + 60, y: me0.y, radius: 150 }) });
    // 另一記落在遠方（自己在圈外）→ 只演衝擊波、無自身飄字。
    lastWS.onmessage({ data: JSON.stringify({ type: "boss_slam", x: me0.x + 5000, y: me0.y, radius: 150 }) });
    pump("重擊衝擊波", 6);
  } catch (e) {
    ok = false; console.error("  ❌ 怪物王預警重擊：拋出例外", e && e.message);
  }
  const newCaught = caughtRenderErrors.slice(before);
  if (!ok || newCaught.length) { failed = true; console.error(`  ❌ 怪物王預警重擊：${newCaught.length} 個繪製例外`); }
  else console.log("  ✅ 怪物王預警重擊：預警圈（半蓄/近滿脈動）＋衝擊波（圈內警示/圈外純波）皆乾淨");
}

// 遠遊見聞（ROADMAP 411）：locale_entered 帶 first_footfall 時，地名卡綴「✨初次踏足」金緞＋XP 飄字、
// 報讀；snapshot 帶 wayfare_count 時小地圖左下角畫足跡計數。驗證初次踏足卡／舊地重遊卡／進場 initial
// 靜默／旁觀者忽略＋足跡計數 HUD 皆不拋例外。
{
  const before = caughtRenderErrors.length;
  console.log("── 情境：遠遊見聞（初次踏足卡＋XP 飄字＋足跡計數 HUD）──");
  let ok = true;
  try {
    // 先讓 snapshot 帶上足跡計數，逼出小地圖左下角的「🧭 遠遊見聞 N 處」常駐標。
    const wSnap = JSON.parse(JSON.stringify(snapshot));
    if (wSnap.players && wSnap.players[0]) {
      wSnap.players[0] = { ...wSnap.players[0], wayfare_count: 3 };
    }
    lastWS.onmessage({ data: JSON.stringify({ ...wSnap, type: "snapshot" }) });
    pump("足跡計數 HUD", 3);
    // 初次踏足：first_footfall=true，地名卡綴金緞＋探索者 XP 飄字＋報讀。
    lastWS.onmessage({ data: JSON.stringify({ type: "locale_entered", player_id: myId, name: "晨露谷", subtitle: "薄霧在草尖上打盹", initial: false, first_footfall: true, tally: 3, xp_reward: 2 }) });
    pump("初次踏足卡", 6);
    // 本趟 XP 已封頂的初次踏足（xp_reward=0 仍是初次踏足、不畫 XP 飄字）。
    lastWS.onmessage({ data: JSON.stringify({ type: "locale_entered", player_id: myId, name: "微風原", subtitle: "野花一路鋪到天邊", initial: false, first_footfall: true, tally: 7, xp_reward: 0 }) });
    pump("封頂初次踏足卡", 6);
    // 舊地重遊：first_footfall=false，純地名卡（沿用 398 行為）。
    lastWS.onmessage({ data: JSON.stringify({ type: "locale_entered", player_id: myId, name: "翡翠林", subtitle: "苔蘚把石頭都養綠了", initial: false, first_footfall: false }) });
    pump("舊地重遊卡", 6);
    // 進場 initial：靜默、不彈卡。
    lastWS.onmessage({ data: JSON.stringify({ type: "locale_entered", player_id: myId, name: "搖籃丘", subtitle: "風把草浪梳成同一個方向", initial: true, first_footfall: false }) });
    pump("進場 initial 靜默", 2);
    // 旁觀者（別人 id）應被忽略、不拋例外。
    lastWS.onmessage({ data: JSON.stringify({ type: "locale_entered", player_id: "someone_else", name: "綠歌平野", subtitle: "陽光在這裡走得很慢", initial: false, first_footfall: true, tally: 1, xp_reward: 2 }) });
    pump("旁觀者忽略", 2);
  } catch (e) {
    ok = false; console.error("  ❌ 遠遊見聞：拋出例外", e && e.message);
  }
  const newCaught = caughtRenderErrors.slice(before);
  if (!ok || newCaught.length) { failed = true; console.error(`  ❌ 遠遊見聞：${newCaught.length} 個繪製例外`); }
  else console.log("  ✅ 遠遊見聞：初次踏足卡＋XP 飄字＋足跡計數＋舊地重遊＋initial 靜默＋旁觀者忽略皆乾淨");
}

// 旅途明信片（ROADMAP 417）：收到 postcard 單播後框成風景卡。驗證正常明信片、換片重建、
// 欄位缺漏／髒值容錯皆不拋例外（純 DOM 面板，非 canvas render 路徑，故以 try/catch 直接接
// 面板建構的例外；下載合成本身已內部 try/catch 包覆、永不外傳）。
{
  console.log("── 情境：旅途明信片（風景卡建構＋換片＋髒值容錯）──");
  let ok = true;
  try {
    // 正常一張明信片。
    lastWS.onmessage({ data: JSON.stringify({ type: "postcard", title: "晨光・🌸 春", place: "晨露谷", subtitle: "薄霧在草尖上打盹", rank: "旅者", flavor: "晨露沾著新芽，一天剛要醒來。", level: 12 }) });
    pump("明信片風景卡", 2);
    // 換一張（不同時辰季節）：sig 變、重建。
    lastWS.onmessage({ data: JSON.stringify({ type: "postcard", title: "星夜・❄️ 冬", place: "翡翠林", subtitle: "苔蘚把石頭都養綠了", rank: "冒險家", flavor: "寒夜寂靜，每一顆星都格外亮。", level: 25 }) });
    pump("重新留影", 2);
    // 欄位缺漏／型別錯亂：容錯不拋（字串退空、level 退 0）。
    lastWS.onmessage({ data: JSON.stringify({ type: "postcard", title: null, place: 123, level: "x" }) });
    pump("明信片髒值容錯", 2);
  } catch (e) {
    ok = false; console.error("  ❌ 旅途明信片：拋出例外", e && e.message);
  }
  if (!ok) { failed = true; }
  else console.log("  ✅ 旅途明信片：風景卡建構＋換片＋髒值容錯皆乾淨");
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

// 畫面動態偏好（ROADMAP 425）：先單元斷言純函式 effectiveReduceMotion 的真值表，
// 再強制進入「省電靜謐」（reduceMotion=true）連跑數幀，確保 calm 路徑零 render 例外
// （預設 smoke 下系統 matchMedia matches=false，calm 繪製路徑平時碰不到）。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.effectiveReduceMotion;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 畫面動態：game.js 未導出 effectiveReduceMotion");
  } else {
    // [pref, osReduce, 期望有效減少動態]
    const cases = [
      ["calm", false, true], ["calm", true, true],          // calm 恆強制靜謐
      ["rich", false, false], ["rich", true, false],        // rich 恆全開（覆寫系統）
      ["auto", false, false], ["auto", true, true],         // auto 跟隨系統
      ["bogus", false, false], ["bogus", true, true],       // 未知值＝auto
      [undefined, true, true],                              // 缺值＝auto
    ];
    let bad = 0;
    for (const [pref, os, want] of cases) {
      if (fn(pref, os) !== want) { bad++; console.error(`  ❌ 畫面動態：effectiveReduceMotion(${pref}, ${os}) 期望 ${want}`); }
    }
    if (bad) failed = true;
    else console.log(`  ✅ 畫面動態·偏好解析真值表：${cases.length}/${cases.length}`);
  }

  const setPref = sandbox.__bfTest && sandbox.__bfTest.setMotionPref;
  if (typeof setPref === "function") {
    const before = caughtRenderErrors.length;
    console.log("── 情境：畫面動態·省電靜謐（強制 reduceMotion 後連跑）──");
    setPref("calm");
    lastWS.onmessage({ data: JSON.stringify({ ...snapshot, type: "snapshot" }) });
    const r = pump("省電靜謐", 30);
    setPref("auto"); // 還原，免影響後續判讀
    if (r instanceof Error) { failed = true; console.error("  ❌ 省電靜謐：未捕捉例外"); }
    const newCaught = caughtRenderErrors.slice(before);
    if (newCaught.length) { failed = true; console.error(`  ❌ 省電靜謐：safeRender 攔下 ${newCaught.length} 個繪製例外`); }
    else if (!(r instanceof Error)) console.log("  ✅ 省電靜謐：乾淨");
  }
}

// 觸覺回饋（ROADMAP 440）：單元斷言純函式 hapticPattern（事件→震動波形）與 hapticEnabled（開關＋支援）。
// 純客戶端、零後端；只驗純邏輯真值表（jsdom 無 navigator.vibrate，實際震動不在 smoke 範圍）。
{
  const pat = sandbox.__bfTest && sandbox.__bfTest.hapticPattern;
  const en = sandbox.__bfTest && sandbox.__bfTest.hapticEnabled;
  if (typeof pat !== "function" || typeof en !== "function") {
    failed = true;
    console.error("  ❌ 觸覺回饋：game.js 未導出 hapticPattern／hapticEnabled");
  } else {
    let bad = 0;
    // 強回饋事件要有波形（非 null）；UI 點按與未知事件不震（null）。
    if (pat("success") == null) { bad++; console.error("  ❌ 觸覺回饋：success 應有震動波形"); }
    if (pat("etherGain") == null) { bad++; console.error("  ❌ 觸覺回饋：etherGain 應有震動波形"); }
    if (pat("levelUp") == null) { bad++; console.error("  ❌ 觸覺回饋：levelUp 應有震動波形"); }
    if (pat("achievement") == null) { bad++; console.error("  ❌ 觸覺回饋：achievement 應有震動波形"); }
    if (pat("click") !== null) { bad++; console.error("  ❌ 觸覺回饋：click 應不震（null）"); }
    if (pat("bogus") !== null) { bad++; console.error("  ❌ 觸覺回饋：未知事件應不震（null）"); }
    // 波形上限保險：所有段不應出現「轟手」級長震（單段 ≤ 60ms）。
    for (const k of ["success", "etherGain", "levelUp", "achievement"]) {
      const p = pat(k);
      const segs = Array.isArray(p) ? p : [p];
      if (segs.some((ms) => typeof ms !== "number" || ms > 60)) {
        bad++; console.error(`  ❌ 觸覺回饋：${k} 波形含過長震動段（應 ≤ 60ms）`);
      }
    }
    // hapticEnabled 真值表：開關與裝置支援都為真才震。
    const enCases = [[true, true, true], [true, false, false], [false, true, false], [false, false, false]];
    for (const [on, sup, want] of enCases) {
      if (en(on, sup) !== want) { bad++; console.error(`  ❌ 觸覺回饋：hapticEnabled(${on}, ${sup}) 期望 ${want}`); }
    }
    if (bad) failed = true;
    else console.log("  ✅ 觸覺回饋·波形與開關真值表：通過");
  }
}

// 介面字級（ROADMAP 441）：單元斷言純函式 uiFontPx（偏好→根字級 px）。純客戶端、零後端。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.uiFontPx;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 介面字級：game.js 未導出 uiFontPx");
  } else {
    let bad = 0;
    // [偏好, 期望根字級 px]：標準＝預設，大／特大遞增，壞值／缺值一律退標準。
    const cases = [["std", 16], ["large", 18], ["xlarge", 20], ["bogus", 16], [null, 16], [undefined, 16]];
    for (const [pref, want] of cases) {
      if (fn(pref) !== want) { bad++; console.error(`  ❌ 介面字級：uiFontPx(${pref}) 期望 ${want}`); }
    }
    // 單調遞增不變式：標準 < 大 < 特大（放大方向不可錯亂）。
    if (!(fn("std") < fn("large") && fn("large") < fn("xlarge"))) {
      bad++; console.error("  ❌ 介面字級：字級應隨 標準<大<特大 嚴格遞增");
    }
    if (bad) failed = true;
    else console.log(`  ✅ 介面字級·偏好解析真值表：${cases.length}/${cases.length}`);
  }
}

// 背景旋律（ROADMAP 442）：單元斷言樂理純函式 bgmScaleHz／bgmNextDegree／bgmChordDegrees。
// 純客戶端、零後端；只驗決定性樂理真值表（jsdom 無 AudioContext，實際發聲不在 smoke 範圍）。
{
  const scaleHz = sandbox.__bfTest && sandbox.__bfTest.bgmScaleHz;
  const nextDeg = sandbox.__bfTest && sandbox.__bfTest.bgmNextDegree;
  const chord = sandbox.__bfTest && sandbox.__bfTest.bgmChordDegrees;
  if (typeof scaleHz !== "function" || typeof nextDeg !== "function" || typeof chord !== "function") {
    failed = true;
    console.error("  ❌ 背景旋律：game.js 未導出 bgmScaleHz／bgmNextDegree／bgmChordDegrees");
  } else {
    let bad = 0;
    // bgmScaleHz：回傳必為正、階數夾鉗、音階上行嚴格遞增。
    if (!(scaleHz(0) > 0)) { bad++; console.error("  ❌ 背景旋律：scaleHz(0) 應為正"); }
    if (scaleHz(-5) !== scaleHz(0)) { bad++; console.error("  ❌ 背景旋律：負階應夾到最低階"); }
    if (scaleHz(999) !== scaleHz(9)) { bad++; console.error("  ❌ 背景旋律：超界應夾到最高階"); }
    if (!(scaleHz(0) < scaleHz(5) && scaleHz(5) < scaleHz(9))) { bad++; console.error("  ❌ 背景旋律：音階應上行嚴格遞增"); }
    // bgmNextDegree：行進夾在 [0,9]、級進正確、壞值退 0。
    if (nextDeg(4, 1) !== 5) { bad++; console.error("  ❌ 背景旋律：nextDegree(4,1) 期望 5"); }
    if (nextDeg(0, -3) !== 0) { bad++; console.error("  ❌ 背景旋律：下界應夾 0"); }
    if (nextDeg(9, 5) !== 9) { bad++; console.error("  ❌ 背景旋律：上界應夾 9"); }
    if (nextDeg(undefined, undefined) !== 0) { bad++; console.error("  ❌ 背景旋律：缺值應退 0"); }
    // bgmChordDegrees：循環取用、回非空陣列、元素皆合法階數。
    const c0 = chord(0), cw = chord(4); // 4 段循環，索引 4 應回到索引 0
    if (!Array.isArray(c0) || c0.length === 0) { bad++; console.error("  ❌ 背景旋律：和弦應為非空陣列"); }
    if (JSON.stringify(c0) !== JSON.stringify(cw)) { bad++; console.error("  ❌ 背景旋律：和弦進行應循環"); }
    if (c0.some((d) => !(scaleHz(d) > 0))) { bad++; console.error("  ❌ 背景旋律：和弦階數應皆有效"); }
    if (bad) failed = true;
    else console.log("  ✅ 背景旋律·樂理真值表：通過");
  }
}

// 農地待辦小結（ROADMAP 427）：單元斷言純函式 farmDigest 的優先序與計數。
// 把一塊田的格子彙整成「下一步最該做的一件農事」，回應建議箱反覆出現的待辦/優先指引/總結回饋。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.farmDigest;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 農地待辦：game.js 未導出 farmDigest");
  } else {
    const cell = (state, dry) => ({ state, dry: !!dry });
    // [說明, cells, 期望 kind, 期望 n]
    const cases = [
      ["空/無田", [], "none", 0],
      ["全自然地未開墾", [cell(0), cell(0)], "none", 0],
      ["只有空土→去播種", [cell(1), cell(1), cell(1)], "plant", 3],
      ["作物全照顧好→休整", [cell(2), cell(3)], "allgood", 2],
      ["有成熟→去收成", [cell(4), cell(3), cell(1)], "harvest", 1],
      ["缺水最優先(凌駕收成/空地)", [cell(2, true), cell(4), cell(1)], "water", 1],
      ["缺水只算未成熟作物格", [cell(2, true), cell(3, true), cell(0)], "water", 2],
      ["成熟凌駕空地", [cell(4), cell(1), cell(1)], "harvest", 1],
      ["壞值容錯(null/缺欄)", [null, {}, cell(1)], "plant", 1],
    ];
    let bad = 0;
    for (const [desc, cells, wantKind, wantN] of cases) {
      const d = fn(cells);
      if (d.kind !== wantKind || d.n !== wantN) {
        bad++;
        console.error(`  ❌ 農地待辦：${desc} 期望 {${wantKind},${wantN}} 得到 {${d.kind},${d.n}}`);
      }
    }
    if (bad) failed = true;
    else console.log(`  ✅ 農地待辦小結·優先序真值表：${cases.length}/${cases.length}`);
  }
}

// 作物品種（ROADMAP 452）：單元斷言三支純函式 seedVarietyMeta／cycleSeedVariety／seedVarietyByCode
// 的真值表——對齊後端 crop_variety.rs（線格式字串往返、未知退主食穀、品種碼 0/1/2、循環順序）。
{
  const meta = sandbox.__bfTest && sandbox.__bfTest.seedVarietyMeta;
  const cycle = sandbox.__bfTest && sandbox.__bfTest.cycleSeedVariety;
  const byCode = sandbox.__bfTest && sandbox.__bfTest.seedVarietyByCode;
  if (typeof meta !== "function" || typeof cycle !== "function" || typeof byCode !== "function") {
    failed = true;
    console.error("  ❌ 作物品種：game.js 未導出 seedVarietyMeta/cycleSeedVariety/seedVarietyByCode");
  } else {
    let bad = 0;
    const expect = (cond, msg) => { if (!cond) { bad++; console.error(`  ❌ 作物品種：${msg}`); } };
    // 線格式 → 品種碼（對齊後端 CropVariety::code：主食穀 0 / 速生菜 1 / 乙太瓜 2）。
    expect(meta("sprout").code === 1, "sprout→碼1");
    expect(meta("staple").code === 0, "staple→碼0");
    expect(meta("etherbloom").code === 2, "etherbloom→碼2");
    // 未知／空字串保守退主食穀（對齊後端 from_wire 永不失敗），不破壞耕作。
    expect(meta("turnip").wire === "staple", "未知字串退主食穀");
    expect(meta("").wire === "staple", "空字串退主食穀");
    expect(meta(undefined).wire === "staple", "undefined 退主食穀");
    // 品種碼 → 中介資料；未知碼退主食穀。
    expect(byCode(0).wire === "staple", "碼0→主食穀");
    expect(byCode(1).wire === "sprout", "碼1→速生菜");
    expect(byCode(2).wire === "etherbloom", "碼2→乙太瓜");
    expect(byCode(99).wire === "staple", "未知碼退主食穀");
    // 循環：sprout→staple→etherbloom→sprout（與後端 ALL 同序）；未知值當主食穀、下一個是乙太瓜。
    expect(cycle("sprout") === "staple", "sprout 循環→staple");
    expect(cycle("staple") === "etherbloom", "staple 循環→etherbloom");
    expect(cycle("etherbloom") === "sprout", "etherbloom 循環→sprout");
    expect(cycle("???") === "etherbloom", "未知值當主食穀、循環→etherbloom");
    // 循環三次回到原點（封閉、不漏品種）。
    expect(cycle(cycle(cycle("sprout"))) === "sprout", "循環三次回原點");
    if (bad) failed = true;
    else console.log("  ✅ 作物品種·品種解析/循環真值表：通過");
  }
}

// 作物品種季節偏好（ROADMAP 453）：單元斷言純函式 seedSeasonHint 的真值表——
// 鏡像後端 crop_variety.rs::season_affinity/peak_season（主食穀四季皆宜、速生菜冬旺夏淡、乙太瓜夏旺冬淡）。
{
  const hint = sandbox.__bfTest && sandbox.__bfTest.seedSeasonHint;
  if (typeof hint !== "function") {
    failed = true;
    console.error("  ❌ 季節偏好：game.js 未導出 seedSeasonHint");
  } else {
    let bad = 0;
    const expect = (cond, msg) => { if (!cond) { bad++; console.error(`  ❌ 季節偏好：${msg}`); } };
    const SEASONS = ["spring", "summer", "autumn", "winter"];
    // 主食穀：四季皆宜，任何季節都無提示（tag 恆 ""）。
    for (const s of SEASONS) expect(hint("staple", s).tag === "", `主食穀 ${s} 應無提示`);
    // 速生菜：冬旺、夏淡，春秋平季無提示。
    expect(hint("sprout", "winter").tag === "peak", "速生菜冬旺");
    expect(hint("sprout", "summer").tag === "lean", "速生菜夏淡");
    expect(hint("sprout", "spring").tag === "", "速生菜春平季");
    expect(hint("sprout", "autumn").tag === "", "速生菜秋平季");
    // 乙太瓜：夏旺、冬淡，春秋平季無提示。
    expect(hint("etherbloom", "summer").tag === "peak", "乙太瓜夏旺");
    expect(hint("etherbloom", "winter").tag === "lean", "乙太瓜冬淡");
    expect(hint("etherbloom", "spring").tag === "", "乙太瓜春平季");
    expect(hint("etherbloom", "autumn").tag === "", "乙太瓜秋平季");
    // 旺季／淡季提示必帶可見文字（玩家看得到「當季旺長／淡季慢長」）。
    expect(hint("etherbloom", "summer").label.length > 0, "旺季須有文字");
    expect(hint("sprout", "summer").label.length > 0, "淡季須有文字");
    // 未知品種／未知季節保守當無偏好，永不騙玩家。
    expect(hint("turnip", "summer").tag === "", "未知品種無偏好");
    expect(hint("etherbloom", "monsoon").tag === "", "未知季節無偏好");
    // 每個有偏好的品種，四季裡恰有一個旺季與一個淡季（對齊後端單峰單谷）。
    for (const w of ["sprout", "etherbloom"]) {
      const peaks = SEASONS.filter((s) => hint(w, s).tag === "peak").length;
      const leans = SEASONS.filter((s) => hint(w, s).tag === "lean").length;
      expect(peaks === 1, `${w} 恰一個旺季`);
      expect(leans === 1, `${w} 恰一個淡季`);
    }
    if (bad) failed = true;
    else console.log("  ✅ 作物品種·季節偏好真值表：通過");
  }
}

// 作物品種·市集行情（ROADMAP 455）：單元斷言純函式 cropDemandVariety 的真值表——
// 鏡像後端 crop_demand.rs::demand_variety（春主食穀／夏速生菜／秋乙太瓜／冬主食穀）。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.cropDemandVariety;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 市集行情：game.js 未導出 cropDemandVariety");
  } else {
    let bad = 0;
    const expect = (cond, msg) => { if (!cond) { bad++; console.error(`  ❌ 市集行情：${msg}`); } };
    // 四季搶手品種（必須與後端 demand_variety 一字不差）。
    expect(fn("spring") === "staple", "春→主食穀搶手");
    expect(fn("summer") === "sprout", "夏→速生菜搶手");
    expect(fn("autumn") === "etherbloom", "秋→乙太瓜搶手");
    expect(fn("winter") === "staple", "冬→主食穀搶手");
    // 一輪四季裡三個品種各至少當令一次（對齊後端 every_variety_demanded_across_year）。
    const demanded = new Set(["spring", "summer", "autumn", "winter"].map(fn));
    for (const w of ["staple", "sprout", "etherbloom"]) {
      expect(demanded.has(w), `${w} 一年內至少當令一次`);
    }
    // 未知季節保守退主食穀（永不騙玩家、永不 undefined）。
    expect(fn("monsoon") === "staple", "未知季節退主食穀");
    expect(fn(undefined) === "staple", "缺季節退主食穀");
    if (bad) failed = true;
    else console.log("  ✅ 作物品種·市集行情真值表：通過");
  }
}

// 個人地塊作物熟成進度條（ROADMAP 457，對齊公田 421）：單元斷言純函式 cropBarFillKind 的填色門檻——
// <0.8 青綠 "grow"、≥0.8 暖金 "soon"、壞值保守退 "grow"。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.cropBarFillKind;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 個人地塊作物進度條：game.js 未導出 cropBarFillKind");
  } else {
    let bad = 0;
    const expect = (cond, msg) => { if (!cond) { bad++; console.error(`  ❌ 個人地塊作物進度條：${msg}`); } };
    expect(fn(0) === "grow", "0→青綠");
    expect(fn(0.5) === "grow", "0.5→青綠");
    expect(fn(0.79) === "grow", "0.79（門檻下）→青綠");
    expect(fn(0.8) === "soon", "0.8（門檻）→暖金");
    expect(fn(0.95) === "soon", "0.95→暖金");
    expect(fn(1) === "soon", "1→暖金");
    // 壞值保守退青綠（永不爆、永不騙玩家「快好了」）。
    expect(fn(NaN) === "grow", "NaN 退青綠");
    expect(fn(undefined) === "grow", "undefined 退青綠");
    expect(fn("0.9") === "soon", "字串數字照常解析（≥0.8）");
    if (bad) failed = true;
    else console.log("  ✅ 個人地塊作物·進度條填色真值表：通過");
  }
}

// 豐收迸發（ROADMAP 458）：單元斷言純函式 harvestBurstSpec 的金穀外觀真值表——
// 收成揚穀：剛迸（t=0）最大最實、不揚（lift=0）；揚到末了（t=1）縮小淡盡、揚至頂（lift=46）；
// 半徑隨年齡遞減、不透明遞減、上揚量遞增（單調）；壞值（NaN／undefined／越界）夾鉗成「揚到末了」、不爆。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.harvestBurstSpec;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 豐收迸發：game.js 未導出 harvestBurstSpec");
  } else {
    let bad = 0;
    const approx = (a, b) => Math.abs(a - b) < 1e-9;
    const expect = (cond, msg) => { if (!cond) { bad++; console.error(`  ❌ 豐收迸發：${msg}`); } };
    const s0 = fn(0), sh = fn(0.5), s1 = fn(1);
    // 兩端外觀
    expect(approx(s0.r, 2.4) && approx(s0.alpha, 1) && approx(s0.lift, 0), "t=0 最大最實、不揚");
    expect(approx(s1.r, 1.2) && approx(s1.alpha, 0) && approx(s1.lift, 46), "t=1 最小淡盡、揚至頂");
    // 單調：半徑遞減、不透明遞減、上揚遞增
    expect(s0.r > sh.r && sh.r > s1.r, "半徑隨年齡單調遞減");
    expect(s0.alpha > sh.alpha && sh.alpha > s1.alpha, "不透明隨年齡單調遞減");
    expect(s0.lift < sh.lift && sh.lift < s1.lift, "上揚量隨年齡單調遞增");
    // 有界且有限：r 恆 > 0、alpha∈[0,1]、lift∈[0,46]
    for (const t of [0, 0.25, 0.5, 0.75, 1]) {
      const s = fn(t);
      expect(Number.isFinite(s.r) && s.r > 0, `t=${t} 半徑有限且 > 0`);
      expect(Number.isFinite(s.alpha) && s.alpha >= 0 && s.alpha <= 1, `t=${t} 不透明∈[0,1]`);
      expect(Number.isFinite(s.lift) && s.lift >= 0 && s.lift <= 46, `t=${t} 上揚∈[0,46]`);
    }
    // 非有限壞值（NaN/undefined/非數字字串/±Infinity）一律退「揚到末了」端；上界越界（2）夾成末了。
    for (const bv of [NaN, undefined, "x", Infinity, -Infinity, 2]) {
      const s = fn(bv);
      expect(approx(s.lift, 46) && approx(s.alpha, 0), `壞值/上界越界 ${String(bv)} 夾鉗成末了`);
    }
    // 下界越界（負值）夾成「剛迸」端（lift=0、alpha=1），仍不爆。
    {
      const s = fn(-1);
      expect(approx(s.lift, 0) && approx(s.alpha, 1), "下界越界 -1 夾鉗成剛迸");
    }
    if (bad) failed = true;
    else console.log("  ✅ 豐收迸發·金穀外觀真值表：通過");
  }
}

// 圍爐分食香氣（ROADMAP 462）：單元斷言純函式 mealAromaSpec 的暖香外觀真值表——
// 暖香一縷：剛飄起（t=0）最濃（alpha=0.85）、貼鍋不揚（lift=0）、暈最小（r=3）；
// 飄到末了（t=1）淡盡（alpha=0）、揚至頂（lift=34）、暈微擴（r=8）；
// 不透明隨上飄單調遞減、上揚量單調遞增、半徑單調遞增；壞值／越界夾鉗成「飄到末了」、不爆。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.mealAromaSpec;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 圍爐分食：game.js 未導出 mealAromaSpec");
  } else {
    let bad = 0;
    const approx = (a, b) => Math.abs(a - b) < 1e-9;
    const expect = (cond, msg) => { if (!cond) { bad++; console.error(`  ❌ 圍爐分食：${msg}`); } };
    const s0 = fn(0), sh = fn(0.5), s1 = fn(1);
    // 兩端外觀
    expect(approx(s0.alpha, 0.85) && approx(s0.lift, 0) && approx(s0.r, 3), "t=0 最濃、不揚、暈最小");
    expect(approx(s1.alpha, 0) && approx(s1.lift, 34) && approx(s1.r, 8), "t=1 淡盡、揚至頂、暈微擴");
    // 單調：不透明遞減、上揚遞增、半徑遞增
    expect(s0.alpha > sh.alpha && sh.alpha > s1.alpha, "不透明隨上飄單調遞減");
    expect(s0.lift < sh.lift && sh.lift < s1.lift, "上揚量隨上飄單調遞增");
    expect(s0.r < sh.r && sh.r < s1.r, "半徑隨上飄單調遞增");
    // 有界且有限：alpha∈[0,0.85]、lift∈[0,34]、r∈[3,8]
    for (const t of [0, 0.25, 0.5, 0.75, 1]) {
      const s = fn(t);
      expect(Number.isFinite(s.alpha) && s.alpha >= 0 && s.alpha <= 0.85, `t=${t} 不透明∈[0,0.85]`);
      expect(Number.isFinite(s.lift) && s.lift >= 0 && s.lift <= 34, `t=${t} 上揚∈[0,34]`);
      expect(Number.isFinite(s.r) && s.r >= 3 && s.r <= 8, `t=${t} 半徑∈[3,8]`);
    }
    // 非有限壞值（NaN/undefined/非數字字串/±Infinity）一律退「飄到末了」端；上界越界（2）夾成末了。
    for (const bv of [NaN, undefined, "x", Infinity, -Infinity, 2]) {
      const s = fn(bv);
      expect(approx(s.lift, 34) && approx(s.alpha, 0), `壞值/上界越界 ${String(bv)} 夾鉗成末了`);
    }
    // 下界越界（負值）夾成「剛飄起」端（lift=0、alpha=0.85），仍不爆。
    {
      const s = fn(-1);
      expect(approx(s.lift, 0) && approx(s.alpha, 0.85), "下界越界 -1 夾鉗成剛飄起");
    }
    if (bad) failed = true;
    else console.log("  ✅ 圍爐分食·暖香外觀真值表：通過");
  }
}

// 主音量（ROADMAP 429）：單元斷言純函式 audioVol 的字串→[0,1] 夾鉗真值表。
// 把 localStorage 存的偏好（字串／null／壞值）解析成乘在音訊上的響度係數，夾進合法區間。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.audioVol;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 主音量：game.js 未導出 audioVol");
  } else {
    // [輸入, 期望係數]
    const cases = [
      ["0.7", 0.7], ["1", 1], ["0", 0], ["0.5", 0.5], ["0.05", 0.05],
      ["-0.5", 0], ["-1", 0],            // 負值夾 0（靜音）
      ["1.5", 1], ["2", 1], ["100", 1],  // 超界夾 1（滿）
      [null, 0.7], [undefined, 0.7],     // 缺值＝預設 70%
      ["", 0.7], ["abc", 0.7], ["NaN", 0.7], // 非數字＝預設
      [0.3, 0.3], [-2, 0],               // 直接傳數字也吃
    ];
    let bad = 0;
    for (const [input, want] of cases) {
      const got = fn(input);
      if (Math.abs(got - want) > 1e-9) {
        bad++;
        console.error(`  ❌ 主音量：audioVol(${JSON.stringify(input)}) 期望 ${want} 得到 ${got}`);
      }
    }
    if (bad) failed = true;
    else console.log(`  ✅ 主音量·音量解析真值表：${cases.length}/${cases.length}`);
  }
}

// 世界風（ROADMAP 430）：單元斷言純函式 windSwayAngle 的搖曳角行為。
// 風只決定畫面擺動（純表現層）：靜風回 0、強風擺幅大於弱風、相位隨座標錯開、壞值安全。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.windSwayAngle;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 世界風：game.js 未導出 windSwayAngle");
  } else {
    const checks = [];
    // 靜風（strength 0）／無 wind／負強度 → 一律 0（不擺）。
    checks.push(["靜風 strength=0 回 0", fn(100, 200, 5000, { dirX: 1, dirY: 0, strength: 0 }) === 0]);
    checks.push(["無 wind 回 0", fn(100, 200, 5000, null) === 0]);
    checks.push(["負強度視為 0", fn(100, 200, 5000, { dirX: 1, strength: -0.5 }) === 0]);
    // 有風時為有限數、且不為 0（含靜態傾斜）。
    const a = fn(100, 200, 0, { dirX: 1, dirY: 0, strength: 0.5 });
    checks.push(["有風回有限數", Number.isFinite(a)]);
    // 強風擺幅應大於弱風（取同相位下的振盪極值近似：比較某時刻絕對角度上限）。
    // 以相同座標/時間，strength 大者 |lean|+|gust 上限| 較大。
    const tSample = 1000;
    const weak = Math.abs(fn(0, 0, tSample, { dirX: 1, dirY: 0, strength: 0.1 }));
    const strong = Math.abs(fn(0, 0, tSample, { dirX: 1, dirY: 0, strength: 1.0 }));
    checks.push(["強風擺幅大於弱風", strong > weak]);
    // 相位隨座標錯開：不同世界座標、同時間，角度不應完全相同（整片不同步）。
    const p1 = fn(0, 0, tSample, { dirX: 1, dirY: 0, strength: 1.0 });
    const p2 = fn(500, 300, tSample, { dirX: 1, dirY: 0, strength: 1.0 });
    checks.push(["相位隨座標錯開", Math.abs(p1 - p2) > 1e-6]);
    // 壞值（NaN 座標/時間、缺 dirX）安全：回有限數、不丟例外。
    const bad = fn(NaN, NaN, NaN, { strength: 0.5 });
    checks.push(["壞值安全回有限數", Number.isFinite(bad)]);
    let nbad = 0;
    for (const [name, ok] of checks) {
      if (!ok) { nbad++; console.error(`  ❌ 世界風：${name}`); }
    }
    if (nbad) failed = true;
    else console.log(`  ✅ 世界風·搖曳角度真值表：${checks.length}/${checks.length}`);
  }
}

// 水畔魚汛（ROADMAP 431）：單元斷言純函式 fishSchoolPoint 的幾何——與後端 fish_school.rs 同公式。
// 魚群中心須：確定性、恆落在自身分區內（「循汛」只查自身分區的前提）、相位回捲連續不跳、壞相位安全。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.fishSchoolPoint;
  const CELL = 1536.0, PERIOD = 2100.0;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 水畔魚汛：game.js 未導出 fishSchoolPoint");
  } else {
    const checks = [];
    // 確定性：同輸入同輸出。
    const d1 = fn(1, 1, 100), d2 = fn(1, 1, 100);
    checks.push(["確定性", d1.x === d2.x && d1.y === d2.y]);
    // 恆落在自身分區內（掃整個相位週期、數個分區）。
    let inBounds = true;
    for (let cx = -2; cx <= 2 && inBounds; cx++) {
      for (let cy = -2; cy <= 2 && inBounds; cy++) {
        for (let ph = 0; ph < PERIOD; ph += 53) {
          const s = fn(cx, cy, ph);
          if (s.x < cx * CELL || s.x > (cx + 1) * CELL || s.y < cy * CELL || s.y > (cy + 1) * CELL) {
            inBounds = false; break;
          }
        }
      }
    }
    checks.push(["魚群恆落在自身分區內", inBounds]);
    // 相位回捲連續：phase 與 phase+PERIOD 幾乎同點（兩軸各走完整數圈）。
    let cont = true;
    for (const ph of [0, 12.3, 199, 871.5, 2099]) {
      const a = fn(2, -1, ph), b = fn(2, -1, ph + PERIOD);
      if (Math.abs(a.x - b.x) > 0.05 || Math.abs(a.y - b.y) > 0.05) { cont = false; break; }
    }
    checks.push(["相位回捲連續不跳位", cont]);
    // 不同分區此刻的「格內相對位移」不應完全相同（彼此錯開）。
    const ra = fn(0, 0, 50), rb = fn(1, 0, 50);
    checks.push(["相鄰分區魚群錯開",
      Math.abs((ra.x - 0.5 * CELL) - (rb.x - 1.5 * CELL)) > 1 ||
      Math.abs((ra.y - 0.5 * CELL) - (rb.y - 0.5 * CELL)) > 1]);
    // 壞相位（NaN）安全：回有限座標、不丟例外。
    const bad = fn(0, 0, NaN);
    checks.push(["壞相位安全回有限座標", Number.isFinite(bad.x) && Number.isFinite(bad.y)]);
    let nbad = 0;
    for (const [name, ok] of checks) {
      if (!ok) { nbad++; console.error(`  ❌ 水畔魚汛：${name}`); }
    }
    if (nbad) failed = true;
    else console.log(`  ✅ 水畔魚汛·魚群幾何真值表：${checks.length}/${checks.length}`);
  }
}

// 天氣隨風飄（ROADMAP 432）：純函式 weatherWindVel 把世界風（430）換算成天氣粒子（93）的附加速度。
// 須：靜風/缺 wind/負強度/未知天氣回零位移；有風時順風向、橫向為主；風愈強漂愈快；壞值安全。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.weatherWindVel;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 天氣隨風飄：game.js 未導出 weatherWindVel");
  } else {
    const checks = [];
    const z = (v) => v.vx === 0 && v.vy === 0;
    // 零位移情境：缺 wind／strength=0（晴天本就無粒子）／負強度／未知天氣型別。
    checks.push(["無 wind 回零位移", z(fn(null, "grassland_rain"))]);
    checks.push(["靜風 strength=0 回零位移", z(fn({ dirX: 1, dirY: 0, strength: 0 }, "grassland_rain"))]);
    checks.push(["負強度回零位移", z(fn({ dirX: 1, dirY: 0, strength: -0.5 }, "grassland_rain"))]);
    checks.push(["未知天氣回零位移", z(fn({ dirX: 1, dirY: 0, strength: 1 }, "clear"))]);
    // 有風時順風向（dirX>0 → vx>0；dirX<0 → vx<0），且為有限數。
    const east = fn({ dirX: 1, dirY: 0, strength: 0.5 }, "grassland_rain");
    const west = fn({ dirX: -1, dirY: 0, strength: 0.5 }, "grassland_rain");
    checks.push(["順風向：東風 vx>0", east.vx > 0 && Number.isFinite(east.vx)]);
    checks.push(["順風向：西風 vx<0", west.vx < 0]);
    // 橫向為主：同強度同向下 |vx| 應大於 |vy|（垂直分量被壓低）。
    const diag = fn({ dirX: 0.8, dirY: 0.8, strength: 1 }, "desert_sandstorm");
    checks.push(["橫向為主 |vx|>|vy|", Math.abs(diag.vx) > Math.abs(diag.vy)]);
    // 風愈強漂愈快：strength 1 的 |vx| 大於 strength 0.2。
    const weak = Math.abs(fn({ dirX: 1, dirY: 0, strength: 0.2 }, "desert_sandstorm").vx);
    const strong = Math.abs(fn({ dirX: 1, dirY: 0, strength: 1.0 }, "desert_sandstorm").vx);
    checks.push(["風愈強漂愈快", strong > weak]);
    // 沙暴比海霧更易感（同風同強度，沙暴漂得更快）。
    const sand = Math.abs(fn({ dirX: 1, dirY: 0, strength: 1 }, "desert_sandstorm").vx);
    const mist = Math.abs(fn({ dirX: 1, dirY: 0, strength: 1 }, "water_sea_mist").vx);
    checks.push(["沙暴比海霧更易感", sand > mist]);
    // 壞值安全：strength 非有限、dirX 非有限 → 回有限數、不丟例外。
    const bad = fn({ dirX: NaN, dirY: NaN, strength: NaN }, "grassland_rain");
    checks.push(["壞值安全回有限位移", Number.isFinite(bad.vx) && Number.isFinite(bad.vy)]);
    let nbad = 0;
    for (const [name, ok] of checks) {
      if (!ok) { nbad++; console.error(`  ❌ 天氣隨風飄：${name}`); }
    }
    if (nbad) failed = true;
    else console.log(`  ✅ 天氣隨風飄·風速換算真值表：${checks.length}/${checks.length}`);
  }
}

// 進場小貼士（ROADMAP 443）：單元斷言純函式 nextTipIndex（目前索引→下一則，循環＋壞值防護）。
// 純客戶端、零後端；只驗循序輪播的決定性真值表（實際 DOM 輪播/淡入不在 smoke 範圍）。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.nextTipIndex;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 進場小貼士：game.js 未導出 nextTipIndex");
  } else {
    let bad = 0;
    // [目前索引, 陣列長度, 期望下一索引]：循序 +1、到尾循環回 0、壞值/空陣列退 0。
    const cases = [
      [0, 10, 1], [5, 10, 6], [8, 10, 9], [9, 10, 0], // 正常循序＋尾端循環
      [-1, 10, 0], [10, 10, 0], [99, 10, 0],           // 越界/負值退 0
      [NaN, 10, 0], [1.5, 10, 0],                       // 非整數/NaN 退 0
      [3, 0, 0], [0, -1, 0], [0, NaN, 0],               // 空陣列/壞長度退 0
    ];
    for (const [cur, len, want] of cases) {
      if (fn(cur, len) !== want) { bad++; console.error(`  ❌ 進場小貼士：nextTipIndex(${cur}, ${len}) 期望 ${want}`); }
    }
    // 不變式：任何輸入都落在 [0, len) 內（壞 len 視為 0）；連續推進能走遍整圈。
    let cur = 0, seen = new Set();
    for (let i = 0; i < 10; i++) { seen.add(cur); cur = fn(cur, 10); }
    if (seen.size !== 10) { bad++; console.error("  ❌ 進場小貼士：連續推進應走遍全部 10 則貼士"); }
    if (bad) failed = true;
    else console.log(`  ✅ 進場小貼士·循序輪播真值表：${cases.length}/${cases.length}`);
  }
}

// 世界此刻一瞥（ROADMAP 445）：單元斷言純函式 glimpseThemeClass（時辰主題 key→登入畫面 CSS class）。
// 純客戶端、零後端；只驗白名單映射＋壞值保底的決定性真值表（實際 DOM 填字/套色不在 smoke 範圍）。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.glimpseThemeClass;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 世界此刻一瞥：game.js 未導出 glimpseThemeClass");
  } else {
    let bad = 0;
    // [主題 key, 期望 class]：四個合法時辰各自加前綴，其餘一律退回 ""（不套色保底）。
    const cases = [
      ["dawn", "glimpse-dawn"], ["day", "glimpse-day"],
      ["dusk", "glimpse-dusk"], ["night", "glimpse-night"],
      ["", ""], ["unknown", ""], ["DAWN", ""], ["glimpse-dawn", ""], // 未知/大小寫/已加前綴皆退 ""
      [null, ""], [undefined, ""], [42, ""], [{}, ""],                 // 壞型別退 ""
    ];
    for (const [theme, want] of cases) {
      if (fn(theme) !== want) { bad++; console.error(`  ❌ 世界此刻一瞥：glimpseThemeClass(${JSON.stringify(theme)}) 期望 ${want}`); }
    }
    if (bad) failed = true;
    else console.log(`  ✅ 世界此刻一瞥·時辰主題映射真值表：${cases.length}/${cases.length}`);
  }
}

// 星光明信片（ROADMAP 447）：單元斷言純函式 postcardStarStyle（星塵印記稀有度→明信片呈現）。
// 把流星雨採集的星塵封進留念卡：none＝一般明信片、stardust＝星光、rainbow＝彩虹星光；壞值保守當一般。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.postcardStarStyle;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 星光明信片：game.js 未導出 postcardStarStyle");
  } else {
    let bad = 0;
    const check = (label, cond) => { if (!cond) { bad++; console.error(`  ❌ 星光明信片：${label}`); } };
    const none = fn("none");
    const dust = fn("stardust");
    const rainbow = fn("rainbow");
    // 一般明信片：不發光、不是彩虹、抬頭為旅途明信片。
    check("none 非星光", none.starlit === false && none.isRainbow === false);
    check("none 抬頭", none.eyebrow.includes("旅途明信片"));
    // 星塵：發光但非彩虹，抬頭為星光明信片。
    check("stardust 星光", dust.starlit === true && dust.isRainbow === false);
    check("stardust 抬頭", dust.eyebrow.includes("星光明信片"));
    // 彩虹星塵：既星光又彩虹，抬頭為彩虹星光。
    check("rainbow 星光", rainbow.starlit === true && rainbow.isRainbow === true);
    check("rainbow 抬頭", rainbow.eyebrow.includes("彩虹"));
    // 三檔外框色兩兩不同（一眼可辨）。
    check("外框色互異", none.border !== dust.border && dust.border !== rainbow.border && none.border !== rainbow.border);
    // 壞值 / 未知字串保守當一般明信片，不發光、不 throw。
    for (const junk of [undefined, null, "", "???", 42]) {
      const r = fn(junk);
      check(`壞值 ${JSON.stringify(junk)} 當一般`, r.starlit === false && r.isRainbow === false);
    }
    if (bad) failed = true;
    else console.log("  ✅ 星光明信片·星塵印記呈現真值表：通過");
  }
}

// 拓圖足跡（ROADMAP 448）：單元斷言足跡格邏輯——key 穩定、記錄揭露周圍一圈、未踏=未揭露、壞值防呆。
{
  const key = sandbox.__bfTest && sandbox.__bfTest.exploreCellKey;
  const rec = sandbox.__bfTest && sandbox.__bfTest.recordExplored;
  const seen = sandbox.__bfTest && sandbox.__bfTest.isExplored;
  const cnt = sandbox.__bfTest && sandbox.__bfTest.exploredCount;
  if (typeof key !== "function" || typeof rec !== "function" || typeof seen !== "function") {
    failed = true;
    console.error("  ❌ 拓圖足跡：game.js 未導出 exploreCellKey/recordExplored/isExplored");
  } else {
    let bad = 0;
    const check = (label, cond) => { if (!cond) { bad++; console.error(`  ❌ 拓圖足跡：${label}`); } };
    // key 確定性：同格座標同 key，跨格不同 key；96px 一格。
    check("同格同 key", key(0, 0) === key(95, 95));
    check("跨格不同 key", key(0, 0) !== key(96, 0));
    check("負座標分格正確", key(-1, -1) === "-1,-1" && key(-96, 0) === "-1,0");
    // 壞值防呆：NaN/undefined/字串一律當 0 格、不 throw。
    check("壞值當原點", key(undefined, NaN) === "0,0" && key("x", null) === "0,0");
    // 記錄前：遠處未揭露。記錄某點後：該格＋周圍一圈（±1）揭露、再外一圈仍未揭露。
    const before = cnt();
    check("記錄前未踏", seen(500 * 96, 500 * 96) === false);
    const added = rec(500 * 96 + 10, 500 * 96 + 10); // 落在 (500,500) 格內
    check("中心格已揭露", seen(500 * 96 + 10, 500 * 96 + 10) === true);
    check("周圍一圈已揭露", seen(501 * 96, 500 * 96) === true && seen(500 * 96, 499 * 96) === true);
    check("再外一圈仍未揭露", seen(503 * 96, 500 * 96) === false);
    check("新增 3x3=9 格", added === 9 && cnt() === before + 9);
    // 重複記錄同點不再新增（冪等）。
    check("重複記錄不增", rec(500 * 96 + 10, 500 * 96 + 10) === 0);
    if (bad) failed = true;
    else console.log("  ✅ 拓圖足跡·足跡格揭露真值表：通過");
  }
}

// 黏土碎屑（ROADMAP 450）：單元斷言純函式 clayCrumbSpec（採集碎屑年齡 t→外觀 r/alpha/hi）。
// 黏土畫風下採集回饋的圓潤屑：剛迸出最大最實、飛行中縮小淡出；壞值/越界夾鉗成近不可見、不爆。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.clayCrumbSpec;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 黏土碎屑：game.js 未導出 clayCrumbSpec");
  } else {
    let bad = 0;
    const check = (label, cond) => { if (!cond) { bad++; console.error(`  ❌ 黏土碎屑：${label}`); } };
    const a0 = fn(0), a1 = fn(1), ah = fn(0.5);
    // 剛迸出（t=0）：最不透明、頂光最亮、半徑最大。
    check("t=0 全不透明", a0.alpha === 1);
    check("t=0 頂光最亮", Math.abs(a0.hi - 0.55) < 1e-9);
    check("t=0 半徑最大", Math.abs(a0.r - 2.7) < 1e-9);
    // 飛行末了（t=1）：完全淡出、頂光熄、半徑仍 > 0（圓潤、不縮成點）。
    check("t=1 完全淡出", a1.alpha === 0);
    check("t=1 頂光熄", a1.hi === 0);
    check("t=1 半徑仍正", a1.r > 0);
    // 單調：半徑與不透明隨 t 遞減（中點介於兩端）。
    check("alpha 隨 t 遞減", a0.alpha > ah.alpha && ah.alpha > a1.alpha);
    check("半徑隨 t 遞減", a0.r > ah.r && ah.r > a1.r);
    // 壞值/越界夾鉗：NaN/undefined 當「飛行末了」（alpha=0）；t<0 夾成 0；t>1 夾成 1；皆有限數。
    check("NaN 當末了", fn(NaN).alpha === 0);
    check("undefined 當末了", fn(undefined).alpha === 0);
    check("t<0 夾成起點", fn(-1).alpha === 1);
    check("t>1 夾成末了", fn(2).alpha === 0);
    check("輸出皆有限數", [a0, a1, ah].every((s) => Number.isFinite(s.r) && Number.isFinite(s.alpha) && Number.isFinite(s.hi)));
    if (bad) failed = true;
    else console.log("  ✅ 黏土碎屑·採集屑外觀真值表：通過");
  }
}

// 黏土世界樹（ROADMAP 456）：單元斷言純函式 clayGroveSpec（成長階段 stage→黏土樹幾何）。
// clay 畫風下玩家親手種的世界樹（370）改捏成黏土造型：由小到大、樹幹樹冠隨階段長大；壞 stage 夾鉗、不爆。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.clayGroveSpec;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 黏土世界樹：game.js 未導出 clayGroveSpec");
  } else {
    let bad = 0;
    const check = (label, cond) => { if (!cond) { bad++; console.error(`  ❌ 黏土世界樹：${label}`); } };
    const s0 = fn(0), s1 = fn(1), s2 = fn(2), s3 = fn(3);
    const all = [s0, s1, s2, s3];
    // 四階段由小到大：樹幹高、樹冠半徑、樹冠抬升都隨階段單調遞增。
    check("樹幹隨階段變高", s0.trunkH < s1.trunkH && s1.trunkH < s2.trunkH && s2.trunkH < s3.trunkH);
    check("樹冠隨階段變大", s0.crownR < s1.crownR && s1.crownR < s2.crownR && s2.crownR < s3.crownR);
    check("樹冠抬升隨階段增", s0.crownLift < s1.crownLift && s1.crownLift < s2.crownLift && s2.crownLift < s3.crownLift);
    check("樹幹隨階段變粗", s0.trunkW < s3.trunkW);
    // 各欄皆有限正值（render 不會畫出負尺寸／NaN）。
    check("各欄有限正值", all.every((g) => [g.trunkW, g.trunkH, g.crownR, g.crownLift].every((v) => Number.isFinite(v) && v > 0)));
    // 壞 stage 夾鉗：越界夾到端點、非整數四捨五入、NaN/undefined 退階段 0、不爆。
    check("stage>3 夾成大樹", fn(5) === s3 || (fn(5).crownR === s3.crownR && fn(5).trunkH === s3.trunkH));
    check("stage<0 夾成嫩芽", fn(-2).crownR === s0.crownR && fn(-2).trunkH === s0.trunkH);
    check("非整數四捨五入", fn(2.4).crownR === s2.crownR && fn(2.6).crownR === s3.crownR);
    check("NaN 退嫩芽", fn(NaN).crownR === s0.crownR);
    check("undefined 退嫩芽", fn(undefined).crownR === s0.crownR);
    if (bad) failed = true;
    else console.log("  ✅ 黏土世界樹·樹形幾何真值表：通過");
  }
}

// clay 城鎮建築（ROADMAP 461）：單元斷言純函式 clayBuildingPalette（建築 type→暖陶土黏土色盤）。
// clay 畫風下城鎮建築改捏成陶土小屋：每棟有合法 wall/roof/trim/win 色；未知 type 退中性陶土盤、永不 undefined。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.clayBuildingPalette;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ clay 城鎮建築：game.js 未導出 clayBuildingPalette");
  } else {
    let bad = 0;
    const check = (label, cond) => { if (!cond) { bad++; console.error(`  ❌ clay 城鎮建築：${label}`); } };
    const TYPES = ["shop", "workshop", "bounty", "expedition", "procurement", "fair", "chief"];
    const isHex = (s) => typeof s === "string" && /^#[0-9a-fA-F]{6}$/.test(s);
    const validPal = (p) => p && isHex(p.wall) && isHex(p.roof) && isHex(p.trim) && isHex(p.win);
    // 每個建築 type 都回完整合法的四色盤（wall/roof/trim/win 皆 #rrggbb）。
    check("各建築 type 皆回合法色盤", TYPES.every((t) => validPal(fn(t))));
    // 未知／壞 type 安全退中性陶土盤（不回 undefined、render 不爆）。
    check("未知 type 退合法陶土盤", validPal(fn("nope")) && validPal(fn(undefined)) && validPal(fn(null)));
    // 各 type 的色盤是各自獨立的（牆色不全相同＝保留色相身份，不是一坨同色）。
    const walls = new Set(TYPES.map((t) => fn(t).wall));
    check("各建築牆色保留色相身份", walls.size >= TYPES.length - 1);
    if (bad) failed = true;
    else console.log("  ✅ clay 城鎮建築·陶土色盤真值表：通過");
  }
}

// 春夜拾螢（ROADMAP 451）：單元斷言拾螢純函式——只在春夜可拾、catch 半徑命中判定、里程碑跨越偵測、壞值防呆。
{
  const catchable = sandbox.__bfTest && sandbox.__bfTest.fireflyCatchable;
  const within = sandbox.__bfTest && sandbox.__bfTest.withinCatchRadius;
  const crossed = sandbox.__bfTest && sandbox.__bfTest.fireflyMilestoneCrossed;
  if (typeof catchable !== "function" || typeof within !== "function" || typeof crossed !== "function") {
    failed = true;
    console.error("  ❌ 春夜拾螢：game.js 未導出 fireflyCatchable/withinCatchRadius/fireflyMilestoneCrossed");
  } else {
    let bad = 0;
    const check = (label, cond) => { if (!cond) { bad++; console.error(`  ❌ 春夜拾螢：${label}`); } };
    const MS = [10, 25, 50, 100, 250, 500, 1000];
    // 只在春夜（season=spring 且 light<0.42）可拾；其餘季節／白天皆不可拾。
    check("春夜可拾", catchable("spring", 0.2) === true);
    check("春日不可拾", catchable("spring", 0.9) === false);
    check("夏夜不可拾", catchable("summer", 0.2) === false);
    check("缺光（白天預設）不可拾", catchable("spring", undefined) === false);
    // catch 半徑：圓內命中、圓外不中；壞值（NaN）／非正半徑安全退 false。
    check("中心命中", within(100, 100, 100, 100, 46) === true);
    check("半徑內命中", within(130, 100, 100, 100, 46) === true);
    check("半徑外不中", within(150, 100, 100, 100, 46) === false);
    check("壞值不中", within(NaN, 100, 100, 100, 46) === false);
    check("非正半徑不中", within(100, 100, 100, 100, 0) === false);
    // 里程碑跨越：剛跨過回該里程碑、一步跨多個回最高、未跨回 0、回退不觸發、壞值安全。
    check("剛跨 10", crossed(9, 10, MS) === 10);
    check("一步跨多個回最高", crossed(20, 60, MS) === 50);
    check("未跨任何回 0", crossed(11, 24, MS) === 0);
    check("停在里程碑上一格不重觸發", crossed(10, 11, MS) === 0);
    check("回退不觸發", crossed(30, 20, MS) === 0);
    check("壞值安全回 0", crossed(undefined, NaN, MS) === 0 && crossed(5, 12, null) === 0);
    if (bad) failed = true;
    else console.log("  ✅ 春夜拾螢·拾螢純函式真值表：通過");
  }
}

// 面板快速搜尋（ROADMAP 459）：單元斷言純函式 menuSearchMatch 的過濾真值表——
// 空查詢（含純空白／壞值）＝全部符合（不過濾）；否則大小寫不敏感子字串包含；中文原樣比對、
// emoji 前綴不影響；查無對應回 false。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.menuSearchMatch;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 面板快速搜尋：game.js 未導出 menuSearchMatch");
  } else {
    // [查詢, 標籤, 期望]
    const cases = [
      // 空查詢一律全部符合（不過濾）
      ["", "🎣 釣魚", true],
      ["   ", "🎣 釣魚", true],
      [null, "🎣 釣魚", true],
      [undefined, "🏪 市場", true],
      // 中文子字串命中（emoji 前綴不擋）
      ["釣", "🎣 釣魚", true],
      ["釣魚", "🎣 釣魚", true],
      ["市場", "🏪 市場", true],
      ["市", "🏪 市場", true],
      // 不相符
      ["釣", "🏪 市場", false],
      ["公會", "🎣 釣魚", false],
      // 前後空白先 trim
      ["  市場  ", "🏪 市場", true],
      // 拉丁字母大小寫不敏感
      ["hud", "HUD 設定", true],
      ["HUD", "hud 設定", true],
      // 壞值標籤安全（非字串）：非空查詢對非字串標籤＝不符
      ["釣", null, false],
      ["釣", undefined, false],
      // 壞值查詢（非字串）視為空＝全部符合
      [123, "🎣 釣魚", true],
    ];
    let bad = 0;
    for (const [q, label, want] of cases) {
      const got = fn(q, label);
      if (got !== want) {
        bad++;
        console.error(`  ❌ 面板快速搜尋：menuSearchMatch(${JSON.stringify(q)}, ${JSON.stringify(label)})=${got}，期望 ${want}`);
      }
    }
    if (bad) failed = true;
    else console.log(`  ✅ 面板快速搜尋·過濾真值表：${cases.length}/${cases.length}`);
  }
}

// 最近開啟（ROADMAP 460）：單元斷言純函式 recordRecentPanel／recentPanelIds——
// recordRecentPanel：新開的浮到最前、去重保序、夾上限、壞 id/壞上限保守；
// recentPanelIds：只留仍存在的、排除已釘選的、去重保序夾上限。
{
  const rec = sandbox.__bfTest && sandbox.__bfTest.recordRecentPanel;
  const ids = sandbox.__bfTest && sandbox.__bfTest.recentPanelIds;
  if (typeof rec !== "function" || typeof ids !== "function") {
    failed = true;
    console.error("  ❌ 最近開啟：game.js 未導出 recordRecentPanel／recentPanelIds");
  } else {
    let bad = 0;
    const eq = (a, b) => Array.isArray(a) && Array.isArray(b) && a.length === b.length && a.every((x, i) => x === b[i]);
    const chk = (name, got, want) => { if (!eq(got, want)) { bad++; console.error(`  ❌ 最近開啟：${name} 得 ${JSON.stringify(got)}，期望 ${JSON.stringify(want)}`); } };
    // recordRecentPanel
    chk("新開浮到最前", rec(["a", "b"], "c", 6), ["c", "a", "b"]);
    chk("重開舊的浮回最前(去重)", rec(["a", "b", "c"], "c", 6), ["c", "a", "b"]);
    chk("重開最前者原樣不變", rec(["a", "b"], "a", 6), ["a", "b"]);
    chk("夾上限(滿了擠掉最舊)", rec(["a", "b", "c"], "d", 3), ["d", "a", "b"]);
    chk("空清單", rec([], "a", 6), ["a"]);
    chk("壞清單退回空+加新", rec(null, "a", 6), ["a"]);
    chk("壞 id 原樣退回(夾上限)", rec(["a", "b"], 123, 6), ["a", "b"]);
    chk("壞 id 且夾上限", rec(["a", "b", "c"], "", 2), ["a", "b"]);
    chk("壞上限退 0(不留)", rec(["a"], "b", "x"), []);
    chk("清單內壞值濾掉", rec(["a", null, "b", 7], "c", 6), ["c", "a", "b"]);
    // recentPanelIds
    chk("排除已釘選", ids(["a", "b", "c"], ["b"], ["a", "b", "c"], 6), ["a", "c"]);
    chk("只留仍存在(剔除已移除面板)", ids(["a", "x", "b"], [], ["a", "b"], 6), ["a", "b"]);
    chk("去重保序", ids(["a", "a", "b"], [], ["a", "b"], 6), ["a", "b"]);
    chk("顯示夾上限", ids(["a", "b", "c"], [], ["a", "b", "c"], 2), ["a", "b"]);
    chk("全壞輸入退空", ids(null, null, null, 6), []);
    chk("全被釘選退空", ids(["a", "b"], ["a", "b"], ["a", "b"], 6), []);
    if (bad) failed = true;
    else console.log("  ✅ 最近開啟·清單真值表：16/16");
  }
}

// 探索接力指引（ROADMAP 463）：單元斷言純函式 nextGuideStep——回傳第一個還沒試過步驟的索引，
// 全部試過/輸入異常退 -1（畢業）；保序、忽略未知/壞 id、Set 與陣列皆可、空安全。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.nextGuideStep;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 探索接力指引：game.js 未導出 nextGuideStep");
  } else {
    let bad = 0;
    const S = [{ id: "a" }, { id: "b" }, { id: "c" }];
    const chk = (name, got, want) => { if (got !== want) { bad++; console.error(`  ❌ 探索接力指引：${name} 得 ${got}，期望 ${want}`); } };
    chk("沒試過任何 → 第 0 個", fn(S, []), 0);
    chk("試過第 0 個 → 第 1 個", fn(S, ["a"]), 1);
    chk("中間缺一仍取最前未試", fn(S, ["a", "c"]), 1);
    chk("全部試過 → -1 畢業", fn(S, ["a", "b", "c"]), -1);
    chk("末步未試 → 取末步", fn(S, ["a", "b"]), 2);
    chk("未知/壞 id 忽略不影響", fn(S, ["x", 7, null]), 0);
    chk("壞 opened 當作沒試過 → 第 0 個", fn(S, null), 0);
    chk("壞 steps → -1", fn(null, []), -1);
    chk("空 steps → -1", fn([], []), -1);
    chk("steps 內壞元素跳過取下一個", fn([null, { id: "b" }], []), 1);
    if (bad) failed = true;
    else console.log("  ✅ 探索接力指引·下一步真值表：10/10");
  }
}

// 同伴扶起·暖光救援（ROADMAP 464）：單元斷言純函式 reviveGlowSpec 的救援暖光外觀真值表——
// 剛迸起（t=0）最亮（alpha=0.7）、環最小（r=8）、🤝 未揚（lift=0）；
// 散盡（t=1）淡盡（alpha=0）、環擴至最大（r=48）、🤝 揚至頂（lift=22）；
// 不透明隨時間單調遞減、半徑與上揚單調遞增；壞值／越界夾鉗成「散到末了」端、不爆。
{
  const fn = sandbox.__bfTest && sandbox.__bfTest.reviveGlowSpec;
  if (typeof fn !== "function") {
    failed = true;
    console.error("  ❌ 同伴扶起：game.js 未導出 reviveGlowSpec");
  } else {
    let bad = 0;
    const approx = (a, b) => Math.abs(a - b) < 1e-9;
    const expect = (cond, msg) => { if (!cond) { bad++; console.error(`  ❌ 同伴扶起：${msg}`); } };
    const s0 = fn(0), sh = fn(0.5), s1 = fn(1);
    // 兩端外觀
    expect(approx(s0.alpha, 0.7) && approx(s0.r, 8) && approx(s0.lift, 0), "t=0 最亮、環最小、未揚");
    expect(approx(s1.alpha, 0) && approx(s1.r, 48) && approx(s1.lift, 22), "t=1 淡盡、環最大、揚至頂");
    // 單調：不透明遞減、半徑遞增、上揚遞增
    expect(s0.alpha > sh.alpha && sh.alpha > s1.alpha, "不透明隨時間單調遞減");
    expect(s0.r < sh.r && sh.r < s1.r, "半徑隨時間單調遞增");
    expect(s0.lift < sh.lift && sh.lift < s1.lift, "上揚量隨時間單調遞增");
    // 有界且有限：alpha∈[0,0.7]、r∈[8,48]、lift∈[0,22]
    for (const t of [0, 0.25, 0.5, 0.75, 1]) {
      const s = fn(t);
      expect(Number.isFinite(s.alpha) && s.alpha >= 0 && s.alpha <= 0.7, `t=${t} 不透明∈[0,0.7]`);
      expect(Number.isFinite(s.r) && s.r >= 8 && s.r <= 48, `t=${t} 半徑∈[8,48]`);
      expect(Number.isFinite(s.lift) && s.lift >= 0 && s.lift <= 22, `t=${t} 上揚∈[0,22]`);
    }
    // 非有限壞值（NaN/undefined/非數字字串/±Infinity）與上界越界（2）一律退「散到末了」端。
    for (const bv of [NaN, undefined, "x", Infinity, -Infinity, 2]) {
      const s = fn(bv);
      expect(approx(s.alpha, 0) && approx(s.r, 48) && approx(s.lift, 22), `壞值/上界越界 ${String(bv)} 夾鉗成末了`);
    }
    // 下界越界（負值）夾成「剛迸起」端（alpha=0.7、r=8、lift=0），仍不爆。
    {
      const s = fn(-1);
      expect(approx(s.alpha, 0.7) && approx(s.r, 8) && approx(s.lift, 0), "下界越界 -1 夾鉗成剛迸起");
    }
    if (bad) failed = true;
    else console.log("  ✅ 同伴扶起·暖光救援真值表：通過");
  }
}

console.log("");
if (failed) {
  console.error("🔴 render-smoke 發現繪製例外（見上）。safeRender 雖防止凍結，但應根治根因。");
  process.exit(1);
}
console.log("✅✅ render-smoke 全綠：所有情境（含屍光/商人/態度越界/未知物種）連跑多幀，render 零例外、safeRender 零攔截。");
process.exit(0);
