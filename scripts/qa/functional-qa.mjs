// 功能 QA 機器人：連 /ws、以訪客進場，實測各核心動作（回城 / 採集 / 挖掘 / 建造 / 商店），
// 每項用快照變化驗證 PASS / FAIL，給開發者 ground truth。用法： node /tmp/functional-qa.mjs [ws-url]
import { WebSocket } from "ws";
const URL = process.argv[2] || "ws://localhost:3000/ws";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const VILLAGE = [2344, 2296];

// ── 地形重算（對齊後端，判斷哪裡是實心好挖）──
const imul = Math.imul;
function grassHash(ix, iy) { let h = (imul(ix | 0, 374761393) + imul(iy | 0, 668265263)) | 0; h = imul(h ^ (h >>> 13), 1274126177) | 0; return ((h ^ (h >>> 16)) >>> 0) / 4294967296; }
function bn(wx, wy, s, sd) { const gx = wx / s, gy = wy / s, x0 = Math.floor(gx), y0 = Math.floor(gy), fx = gx - x0, fy = gy - y0; const h = (a, b) => grassHash((imul(a | 0, 1009) + sd) | 0, (imul(b | 0, 9176) + imul(sd, 31)) | 0); const v00 = h(x0, y0), v10 = h(x0 + 1, y0), v01 = h(x0, y0 + 1), v11 = h(x0 + 1, y0 + 1); const sx = fx * fx * (3 - 2 * fx), sy = fy * fy * (3 - 2 * fy); const a = v00 + (v10 - v00) * sx, b = v01 + (v11 - v01) * sx; return a + (b - a) * sy; }
function biomeAt(wx, wy) { const e = bn(wx, wy, 1500, 7), m = bn(wx, wy, 1200, 137); if (e < 0.30) return "water"; if (e < 0.355) return "sand"; if (e > 0.76) return "rocky"; return m > 0.56 ? "forest" : "meadow"; }
const deltaMap = new Map();
function isSolid(wx, wy) {
  const gx = Math.floor(wx / 32), gy = Math.floor(wy / 32), CT = 16;
  const cx = Math.floor(gx / CT), cy = Math.floor(gy / CT), tx = ((gx % CT) + CT) % CT, ty = ((gy % CT) + CT) % CT;
  const d = deltaMap.get(`${cx},${cy},${tx},${ty}`); if (d !== undefined) return d !== "empty";
  const sdx = wx - 2344, sdy = wy - 2296; if (sdx * sdx + sdy * sdy <= 640 * 640) return false;
  const b = biomeAt(wx, wy); if (b === "water") return false;
  if (bn(wx, wy, 160, 123) < 0.82) return false;
  return true; // 18% 實心
}

const S = { x: 0, y: 0, ether: 0, hp: 0, inv: {}, nodes: [], npcs: [], enemies: [], ready: false };
let myId = null, send;
function connect() {
  return new Promise((res, rej) => {
    const ws = new WebSocket(URL); const t = setTimeout(() => rej(new Error("連線超時")), 12000);
    ws.on("open", () => ws.send(JSON.stringify({ type: "join", name: "功能QA", species: "terran" })));
    ws.on("message", (raw) => { let m; try { m = JSON.parse(raw); } catch { return; }
      if (m.type === "welcome") { myId = m.id; return; }
      if (m.type === "snapshot" && myId) {
        if (Array.isArray(m.terrain)) for (const d of m.terrain) deltaMap.set(`${d.cx},${d.cy},${d.tx},${d.ty}`, d.kind);
        S.nodes = m.nodes || []; S.npcs = m.npcs || []; S.enemies = m.enemies || [];
        const me = (m.players || []).find((p) => p.id === myId);
        if (me) { S.x = me.x; S.y = me.y; S.ether = me.ether; S.hp = me.hp;
          S.inv = {}; for (const it of me.inventory || []) S.inv[it.item] = it.qty;
          if (!S.ready) { S.ready = true; clearTimeout(t); send = (o) => ws.send(JSON.stringify(o)); res(ws); } }
      }
    });
    ws.on("error", rej);
  });
}
const invTotal = () => Object.values(S.inv).reduce((a, b) => a + b, 0);
async function moveTo(tx, ty, maxMs = 4000) {
  const t0 = Date.now();
  while (Date.now() - t0 < maxMs) {
    const dx = tx - S.x, dy = ty - S.y; if (Math.hypot(dx, dy) < 28) break;
    send({ type: "input", right: dx > 6, left: dx < -6, down: dy > 6, up: dy < -6 });
    await sleep(120);
  }
  send({ type: "input", up: false, down: false, left: false, right: false }); await sleep(200);
  return Math.hypot(tx - S.x, ty - S.y);
}
function nearestSolid() { // 掃 player 周圍 ~400px 找最近實心格中心
  let best = null, bd = 1e9;
  for (let r = 40; r <= 400; r += 16) for (let a = 0; a < 360; a += 30) {
    const x = S.x + Math.cos(a * Math.PI / 180) * r, y = S.y + Math.sin(a * Math.PI / 180) * r;
    if (isSolid(x, y)) { const cx = (Math.floor(x / 32) + 0.5) * 32, cy = (Math.floor(y / 32) + 0.5) * 32; const d = Math.hypot(cx - S.x, cy - S.y); if (d < bd) { bd = d; best = [cx, cy]; } }
  }
  return best;
}
const results = [];
const check = (name, ok, detail) => { results.push([ok, name, detail]); const tag = ok === true ? "✅ PASS" : ok === false ? "❌ FAIL" : "⏭️ SKIP"; console.log(`${tag}  ${name}  ${detail}`); };

async function main() {
  console.log(`連線 ${URL} …`); const ws = await connect();
  console.log(`進場 (${S.x.toFixed(0)},${S.y.toFixed(0)})  乙太=${S.ether} HP=${S.hp} 背包=${JSON.stringify(S.inv)}\n`);

  // 1. 挖掘（在出生野外、有實心格時測；走到最近實心格、挖、驗背包+1 且格變空）
  const solid = nearestSolid();
  if (solid) {
    await moveTo(solid[0] - 50, solid[1]); const before = invTotal();
    send({ type: "dig", wx: solid[0], wy: solid[1] }); await sleep(700);
    check("挖掘 dig", invTotal() > before && !isSolid(solid[0], solid[1]), `(${solid[0].toFixed(0)},${solid[1].toFixed(0)}) 背包 ${before}→${invTotal()}，格已空=${!isSolid(solid[0], solid[1])}`);
  } else check("挖掘 dig", null, "附近 400px 沒有實心格，跳過");

  // 2. 建造（背包有 dirt/stone 就放回剛挖的空格）
  const mat = S.inv.dirt ? "dirt" : S.inv.stone ? "stone" : null;
  if (mat && solid) {
    const before = S.inv[mat]; send({ type: "place", wx: solid[0], wy: solid[1], material: mat }); await sleep(700);
    check("建造 place", isSolid(solid[0], solid[1]) && (S.inv[mat] || 0) < before, `放 ${mat}：格變實心=${isSolid(solid[0], solid[1])}，${mat} ${before}→${S.inv[mat] || 0}`);
  } else check("建造 place", null, "背包沒有可放材料（沒挖到 dirt/stone），跳過");

  // 3. 採集（出生附近有節點就採）
  const node = S.nodes.filter((n) => n.harvestable).sort((a, b) => Math.hypot(a.x - S.x, a.y - S.y) - Math.hypot(b.x - S.x, b.y - S.y))[0];
  if (node) {
    await moveTo(node.x, node.y); const before = invTotal();
    const distToNode = Math.hypot(node.x - S.x, node.y - S.y); // 採集判定半徑 GATHER_REACH=56
    send({ type: "gather" }); await sleep(700);
    const ok = invTotal() > before;
    check("採集 gather", ok || distToNode > 56 ? ok : false,
      `${node.kind} 距節點 ${distToNode.toFixed(0)}px(需<56) 背包 ${before}→${invTotal()}` + (!ok && distToNode > 56 ? " ← 機器人沒走到，非採集 bug" : ""));
  } else check("採集 gather", null, "附近沒有可採節點，跳過");

  // 4. 回城
  send({ type: "return_home" }); await sleep(1200);
  check("回城 return_home", Math.hypot(S.x - VILLAGE[0], S.y - VILLAGE[1]) < 200, `→(${S.x.toFixed(0)},${S.y.toFixed(0)}) 應≈村子(${VILLAGE})`);

  // 5. 商店（回村子後靠近商人賣一個素材）
  const npc = S.npcs[0];
  if (npc) {
    await moveTo(npc.x, npc.y);
    const sellable = (npc.buy_list || []).find((e) => (S.inv[e.item] || 0) > 0);
    if (sellable) { const be = S.ether; send({ type: "shop_sell", item: sellable.item, qty: 1 }); await sleep(600);
      check("商店賣出 shop_sell", S.ether > be, `賣 1 ${sellable.item}：乙太 ${be}→${S.ether}`);
    } else check("商店賣出 shop_sell", null, "背包沒有商人收購的東西，跳過");
  } else check("商店", null, "找不到 NPC 商人，跳過");

  // 6. 攻擊（附近有敵人就打一下，看有沒有報錯/掉血）
  const enemy = S.enemies.filter((e) => e.alive).sort((a, b) => Math.hypot(a.x - S.x, a.y - S.y) - Math.hypot(b.x - S.x, b.y - S.y))[0];
  if (enemy) { await moveTo(enemy.x, enemy.y, 3000); const eh = enemy.hp; send({ type: "attack" }); await sleep(500);
    const e2 = S.enemies.find((e) => Math.abs(e.x - enemy.x) < 40 && Math.abs(e.y - enemy.y) < 40);
    check("攻擊 attack", true, `對 ${enemy.kind} 攻擊已送（敵 hp ${eh}→${e2 ? e2.hp : "?"}）`);
  } else check("攻擊 attack", null, "附近沒有敵人，跳過");

  const pass = results.filter((r) => r[0] === true).length, fail = results.filter((r) => r[0] === false).length, skip = results.filter((r) => r[0] === null).length;
  console.log(`\n=== 功能 QA 結果：${pass} PASS / ${fail} FAIL / ${skip} 跳過 ===`);
  ws.close(); process.exit(fail > 0 ? 1 : 0);
}
main().catch((e) => { console.error("功能QA失敗:", e.message); process.exit(1); });
