// 移動探針：連 /ws、以訪客進場，系統性地往各方向走，偵測「輸入有送但位置沒如預期前進＝卡住」，
// 回報精確卡點座標 + 該處地形（本地重算 biome / tile，跟前後端同一套），給開發者 ground truth。
// 用法： node /tmp/movement-probe.mjs [ws-url]
import { WebSocket } from "ws";

const URL = process.argv[2] || "ws://localhost:3000/ws";
const SPEED = 320; // PLAYER_SPEED（px/s），與後端一致

// ── 地形重算（複製自 web/game.js，與前後端逐位元一致）──
const imul = Math.imul;
function grassHash(ix, iy) {
  let h = (imul(ix | 0, 374761393) + imul(iy | 0, 668265263)) | 0;
  h = imul(h ^ (h >>> 13), 1274126177) | 0;
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
}
function biomeNoise(wx, wy, scale, seed) {
  const gx = wx / scale, gy = wy / scale;
  const x0 = Math.floor(gx), y0 = Math.floor(gy);
  const fx = gx - x0, fy = gy - y0;
  const h = (a, b) => grassHash((imul(a | 0, 1009) + seed) | 0, (imul(b | 0, 9176) + imul(seed, 31)) | 0);
  const v00 = h(x0, y0), v10 = h(x0 + 1, y0), v01 = h(x0, y0 + 1), v11 = h(x0 + 1, y0 + 1);
  const sx = fx * fx * (3 - 2 * fx), sy = fy * fy * (3 - 2 * fy);
  const a = v00 + (v10 - v00) * sx, b = v01 + (v11 - v01) * sx;
  return a + (b - a) * sy;
}
function biomeAt(wx, wy) {
  const e = biomeNoise(wx, wy, 1500, 7), m = biomeNoise(wx, wy, 1200, 137);
  if (e < 0.30) return "water";
  if (e < 0.355) return "sand";
  if (e > 0.76) return "rocky";
  return m > 0.56 ? "forest" : "meadow";
}
function tileHash(gx, gy) {
  const ix = (imul(gx | 0, 1031) ^ imul(gy | 0, 2053)) | 0;
  const iy = ((gx | 0) ^ imul(gy | 0, 1009)) | 0;
  return grassHash(ix, iy);
}
const deltaMap = new Map(); // 伺服器廣播的真實地形差異（別人挖/放過的格子）
function tileKindAt(wx, wy) {
  // 先查 delta（你/別人挖過放過的）——這是 ground truth，純生成看不到。
  const gx = Math.floor(wx / 32), gy = Math.floor(wy / 32);
  const CT = 16;
  const cx = Math.floor(gx / CT), cy = Math.floor(gy / CT);
  const tx = ((gx % CT) + CT) % CT, ty = ((gy % CT) + CT) % CT;
  const d = deltaMap.get(`${cx},${cy},${tx},${ty}`);
  if (d !== undefined) return d;
  const sdx = wx - 2344, sdy = wy - 2296;
  if (sdx * sdx + sdy * sdy <= 640 * 640) return "empty"; // 安全區
  const b = biomeAt(wx, wy);
  if (b === "water") return "empty";
  // 地形空曠度（對齊後端 tile_kind_at：cave < 門檻 → 空地）。實心格的細分種類(crystal/mushroom…)
  // 這裡一律當「實心」（stone/dirt），夠用來偵測「卡住 vs 走得動」。
  const cave = biomeNoise(wx, wy, 160, 123);
  if (cave < (b === "rocky" ? 0.50 : 0.82)) return "empty"; // 礦區密、其餘疏（對齊後端）
  const h = tileHash(gx, gy);
  if (b === "rocky") return h < 0.12 ? "ore" : "stone";
  if (b === "forest") return h < 0.10 ? "stone" : "dirt";
  if (b === "meadow") return h < 0.05 ? "stone" : "dirt";
  if (b === "sand") return h < 0.08 ? "stone" : "dirt";
  return "empty";
}
// 玩家周圍 8px 四角的地形 + 中心 biome，描述「卡點是什麼擋住」。
function describe(x, y) {
  const r = 8;
  const corners = [[r, r], [-r, r], [r, -r], [-r, -r]]
    .map(([ox, oy]) => tileKindAt(x + ox, y + oy));
  const waterCorners = [[r, r], [-r, r], [r, -r], [-r, -r]]
    .filter(([ox, oy]) => biomeAt(x + ox, y + oy) === "water").length;
  return `biome=${biomeAt(x, y)} tiles四角=[${corners.join(",")}] 水四角=${waterCorners}`;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function connect() {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(URL);
    let myId = null;
    const state = { x: 0, y: 0, ready: false };
    const t = setTimeout(() => reject(new Error("連線超時")), 12000);
    ws.on("open", () => ws.send(JSON.stringify({ type: "join", name: "QA探針", species: "terran" })));
    ws.on("message", (raw) => {
      let m; try { m = JSON.parse(raw.toString()); } catch { return; }
      if (m.type === "welcome") { myId = m.id; return; }
      if (m.type === "snapshot" && myId) {
        if (Array.isArray(m.terrain)) {
          for (const d of m.terrain) deltaMap.set(`${d.cx},${d.cy},${d.tx},${d.ty}`, d.kind);
        }
        const me = m.players && m.players.find((p) => p.id === myId);
        if (me) { state.x = me.x; state.y = me.y; state.hp = me.hp; if (!state.ready) { state.ready = true; clearTimeout(t); resolve({ ws, state, send: (o) => ws.send(JSON.stringify(o)) }); } }
      }
    });
    ws.on("error", reject);
  });
}

// 從 (x,y) 往 (dx,dy) 方向掃描正前方 ~70px，找第一個擋路的(實心 tile 或水)，回報它在哪、是什麼。
function scanAhead(x, y, dx, dy) {
  const len = Math.hypot(dx, dy) || 1; const ux = dx / len, uy = dy / len;
  for (let d = 4; d <= 70; d += 4) {
    const px = x + ux * d, py = y + uy * d;
    // 玩家四角會碰到的：檢查前緣四角
    for (const [ox, oy] of [[8, 8], [-8, 8], [8, -8], [-8, -8]]) {
      const tk = tileKindAt(px + ox, py + oy);
      const bm = biomeAt(px + ox, py + oy);
      if (tk !== "empty") return `前方 ${d.toFixed(0)}px 有實心地形 ${tk}（角 ${ox},${oy}）`;
      if (bm === "water") return `前方 ${d.toFixed(0)}px 有水域（角 ${ox},${oy}）`;
    }
  }
  return "前方 70px 內沒有實心/水（→ 卡的不是地形!）";
}

// 往某方向持續按住 ms 毫秒，回報實際位移 vs 期望，卡住時掃描正前方找真正擋路的東西。
async function probe(send, state, keys, label, ms = 1800) {
  const x0 = state.x, y0 = state.y;
  const dx = (keys.right ? 1 : 0) - (keys.left ? 1 : 0);
  const dy = (keys.down ? 1 : 0) - (keys.up ? 1 : 0);
  send({ type: "input", up: !!keys.up, down: !!keys.down, left: !!keys.left, right: !!keys.right });
  const samples = [];
  const steps = Math.floor(ms / 100);
  for (let i = 0; i < steps; i++) { await sleep(100); samples.push([state.x, state.y, state.hp]); }
  send({ type: "input", up: false, down: false, left: false, right: false });
  await sleep(150);
  const dist = Math.hypot(state.x - x0, state.y - y0);
  const exp = SPEED * (ms / 1000);
  const minHp = Math.min(...samples.map((s) => s[2] ?? 999));
  console.log(`\n[${label}] (${x0.toFixed(0)},${y0.toFixed(0)})→(${state.x.toFixed(0)},${state.y.toFixed(0)})  位移 ${dist.toFixed(0)}/${exp.toFixed(0)}px (${(dist / exp * 100).toFixed(0)}%)  最低HP=${minHp}`);
  if (dist < exp * 0.6) {
    const ahead = scanAhead(state.x, state.y, dx, dy);
    console.log(`  ⚠️ 卡住! ${ahead}`);
    const traj = samples.map((s, i) => (i % 2 === 0 ? `${(Math.hypot(s[0] - x0, s[1] - y0)).toFixed(0)}` : null)).filter((v) => v !== null);
    console.log(`     軌跡(每0.2s累積位移): ${traj.join(" ")}`);
    // 若「前方沒地形卻卡」→ 對前方那格挖一下，看伺服器是否其實有實心格（前後端不一致的證據）。
    if (ahead.includes("沒有實心")) {
      const len = Math.hypot(dx, dy) || 1;
      const wx = state.x + (dx / len) * 24, wy = state.y + (dy / len) * 24;
      const before = state.x, beforeY = state.y;
      send({ type: "dig", wx, wy });
      await sleep(400);
      send({ type: "input", up: !!keys.up, down: !!keys.down, left: !!keys.left, right: !!keys.right });
      await sleep(600);
      send({ type: "input", up: false, down: false, left: false, right: false });
      const moved2 = Math.hypot(state.x - before, state.y - beforeY);
      console.log(`     → 對前方(${wx.toFixed(0)},${wy.toFixed(0)})挖一下後再走，位移 ${moved2.toFixed(0)}px ${moved2 > 20 ? "（挖開後就能走了 → 確實有隱形實心格 = 前後端 tile 不一致!）" : "（挖了也沒用 → 不是地形）"}`);
    }
  } else {
    console.log(`  ✓ 走得動`);
  }
}

const main = async () => {
  console.log(`連線 ${URL} …`);
  const { ws, state, send } = await connect();
  console.log(`進場成功，出生點 (${state.x.toFixed(0)}, ${state.y.toFixed(0)})  地形: ${describe(state.x, state.y)}`);
  for (const [keys, label] of [
    [{ right: true }, "右"], [{ left: true }, "左"], [{ down: true }, "下"], [{ up: true }, "上"],
    [{ right: true, down: true }, "右下"], [{ left: true, up: true }, "左上"],
    // 走遠一點再試（離開安全區、進入真實地形）
    [{ right: true }, "右(續)"], [{ right: true }, "右(再續)"], [{ down: true }, "下(續)"],
  ]) {
    await probe(send, state, keys, label);
  }
  ws.close();
  console.log("\n完成。");
  process.exit(0);
};
main().catch((e) => { console.error("探針失敗:", e.message); process.exit(1); });
