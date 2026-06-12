// 城鎮 render crash 取證 probe：連 /ws、以訪客進場、走進新手村，撈出「城裡」最豐富的一份
// 完整 snapshot（含 npcs/residents/wildlife/colonies/carion/merchant/...）存檔，供分析哪筆資料
// 會讓前端 game.js 的繪製函式拋例外（render 迴圈一拋就凍結→人物消失）。
// 用法：node scripts/qa/town-snapshot-probe.mjs [ws-url]
import { WebSocket } from "ws";
import { writeFileSync } from "fs";

const URL = process.argv[2] || "ws://localhost:3000/ws";
const TOWN = [2344, 2296]; // 新手村中心
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let myId = null;
let best = null, bestScore = -1;
const seenKinds = new Set();
const snapshots = [];

const ws = new WebSocket(URL);
let send;

function score(m) {
  // 「城裡感」評分：npc/居民/商人/建物越多越像在城中心。
  return (m.npcs?.length || 0) * 3
    + (m.residents?.length || 0) * 3
    + (m.town_buildings?.length || 0) * 2
    + (m.wildlife?.length || 0)
    + (m.colonies?.length || 0)
    + (m.carion_orbs?.length || 0)
    + (m.wandering_merchant_secs ? 5 : 0);
}

ws.on("open", () => {
  send = (o) => ws.send(JSON.stringify(o));
  send({ type: "join", name: "城測QA", species: "terran" });
});

ws.on("message", (raw) => {
  let m; try { m = JSON.parse(raw); } catch { return; }
  if (m.type === "welcome") { myId = m.id; return; }
  if (m.type !== "snapshot" || !myId) return;

  // 記錄出現過的所有 top-level 欄位 + 各實體 kind，幫助看資料形狀。
  for (const k of Object.keys(m)) seenKinds.add(k);

  const me = (m.players || []).find((p) => p.id === myId);
  if (me) {
    // 朝城中心走。
    const dx = TOWN[0] - me.x, dy = TOWN[1] - me.y;
    const d = Math.hypot(dx, dy) || 1;
    send({ type: "move", dx: dx / d, dy: dy / d });
  }
  const s = score(m);
  snapshots.push({ score: s, x: me?.x, y: me?.y,
    counts: { players: m.players?.length, npcs: m.npcs?.length, residents: m.residents?.length,
      wildlife: m.wildlife?.length, colonies: m.colonies?.length, carion: m.carion_orbs?.length,
      town_buildings: m.town_buildings?.length, merchant: m.wandering_merchant_secs } });
  if (s > bestScore) { bestScore = s; best = m; }
});

ws.on("error", (e) => { console.error("ws error", e.message); process.exit(1); });

await sleep(9000); // 走 ~9 秒，足夠從出生點走進城中心並停留採樣

if (best) {
  writeFileSync("/tmp/town-snapshot.json", JSON.stringify(best, null, 2));
  console.log("最佳城鎮 snapshot 已存 /tmp/town-snapshot.json，score =", bestScore);
  console.log("top-level 欄位:", [...seenKinds].sort().join(", "));
  console.log("實體計數:", JSON.stringify({
    players: best.players?.length, npcs: best.npcs?.length, residents: best.residents?.length,
    wildlife: best.wildlife?.length, colonies: best.colonies?.length,
    carion_orbs: best.carion_orbs?.length, town_buildings: best.town_buildings?.length,
    species_attitudes: best.species_attitudes?.length, wandering_merchant_secs: best.wandering_merchant_secs,
  }));
  // 抽樣印各陣列第一筆，看欄位形狀。
  for (const key of ["npcs", "residents", "wildlife", "colonies", "carion_orbs", "species_attitudes"]) {
    if (Array.isArray(best[key]) && best[key].length) {
      console.log(`  ${key}[0] =`, JSON.stringify(best[key][0]));
    }
  }
} else {
  console.log("沒撈到任何 snapshot");
}
ws.close();
process.exit(0);
