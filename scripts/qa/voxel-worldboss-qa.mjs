// ============================================================
// voxel-worldboss-qa.mjs — 遠征首領 v1 WS 功能 QA
// ============================================================
// 真 WebSocket 直連隔離伺服器（記憶體/jsonl 模式、獨立 port），驗後端權威湧現行為：
//   (a) 反覆撥鐘「夜→日」觸發白天擲骰（低機率，多次嘗試提高中獎機會）→ 收到 worldboss spawn 橫幅
//       ＋ players 快照的 world_boss 由 null 變非 null（位置在生成環內、血量=上限）
//   (b) 送 boss_hit → 收到 boss_hit_ok、血量遞減，players 快照的 world_boss.hp 同步下降
//   (c) 連續打到血量歸零 → 收到 worldboss defeat 橫幅、players 快照 world_boss 變回 null、
//       附近掉出一批乙太礦（item_dropped）
// 需要伺服器以 BUTFUN_QA_DEBUG=1 啟動（qa_set_time/qa_grant 才生效；正式線上惰性忽略）。
// 生成本身是機率事件，關鍵斷言採 best-effort 長逾時＋多次嘗試；純函式正確性另有單元測試把關。
// 不抄外部碼；全繁中註解；node --check 過。

import WebSocket from "ws";

const PORT = process.env.VQA_PORT || 48391;
const WS_URL = `ws://127.0.0.1:${PORT}/voxel/ws`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let pass = 0, fail = 0, warn = 0;
function check(label, ok, extra = "") {
  if (ok) { pass++; console.log(`  ✅ ${label}${extra ? "  " + extra : ""}`); }
  else { fail++; console.log(`  ❌ ${label}${extra ? "  " + extra : ""}`); }
}
function softCheck(label, ok, extra = "") {
  if (ok) { pass++; console.log(`  ✅ ${label}${extra ? "  " + extra : ""}`); }
  else { warn++; console.log(`  ⚠️ ${label}（best-effort，未達成只警告）${extra ? "  " + extra : ""}`); }
}

const ws = new WebSocket(WS_URL);
const send = (o) => ws.send(JSON.stringify(o));
const st = {
  ready: false, worldBoss: null, spawnBanner: null, defeatBanner: null,
  hitAcks: [], drops: [], pos: null,
};
ws.on("message", (buf) => {
  let m; try { m = JSON.parse(buf.toString()); } catch { return; }
  switch (m.t) {
    case "welcome": st.ready = true; if (m.spawn) st.pos = { x: m.spawn.x, y: m.spawn.y, z: m.spawn.z }; break;
    case "players": st.worldBoss = m.world_boss || null; break;
    case "worldboss":
      if (m.phase === "spawn") st.spawnBanner = m;
      if (m.phase === "defeat") st.defeatBanner = m;
      break;
    case "boss_hit_ok": st.hitAcks.push(m); break;
    case "item_dropped": st.drops.push(m);
      break;
  }
});

await new Promise((resolve, reject) => {
  ws.on("open", () => { send({ t: "join", name: "首領測試員" }); resolve(); });
  ws.on("error", reject);
});
await sleep(300);
console.log("① 連線就緒，開始反覆撥鐘觸發白天擲骰…");

// 反覆「夜→日」：夜間 phase!=Day 重置 BOSS_ROLLED，日間擲一次骰。0.14 機率，30 次嘗試
// 中獎機率 ≈ 1-(0.86)^30 ≈ 98.7%，足夠穩定驗證，不必改動生產機率常數。
let spawned = false;
for (let i = 0; i < 30 && !spawned; i++) {
  send({ t: "qa_set_time", time: 0.05 }); // 深夜：重置擲骰旗標
  await sleep(120);
  send({ t: "qa_set_time", time: 0.5 }); // 白天：擲一次骰
  await sleep(180);
  if (st.worldBoss) spawned = true;
}

check("反覆撥鐘後首領現身（world_boss 非 null）", !!st.worldBoss, spawned ? "" : "（30 次嘗試仍未中獎，機率事件，重跑或調高嘗試次數）");
if (st.worldBoss) {
  check("首領血量＝上限", st.worldBoss.hp === st.worldBoss.max_hp, `hp=${st.worldBoss.hp} max=${st.worldBoss.max_hp}`);
  check("首領名字非空", !!st.worldBoss.name, st.worldBoss.name);
  softCheck("收到 worldboss spawn 橫幅", !!st.spawnBanner, st.spawnBanner ? st.spawnBanner.msg : "");
}

if (st.worldBoss) {
  console.log("② 分段走位到首領身邊（M3+L1 反瞬移守衛限單則位移 ≤64 格），連續挖擊直到倒下…");
  // move_is_acceptable（voxel_ws.rs）擋單則超過 MAX_MOVE_STEP=64 格的位移，QA 探針不能一步
  // 瞬移過去——改成分段走位（每步 ≤50 格，留餘裕），驗的正是「後端權威 reach 判定」而非
  // 「client 自報位置就算數」，比一步到位更貼近真實反作弊路徑。
  const STEP = 50;
  while (st.pos) {
    const dx = st.worldBoss.x - st.pos.x, dy = st.worldBoss.y - st.pos.y, dz = st.worldBoss.z - st.pos.z;
    const dist = Math.hypot(dx, dy, dz);
    if (dist < 4) break;
    const k = Math.min(1, STEP / dist);
    const nx = st.pos.x + dx * k, ny = st.pos.y + dy * k, nz = st.pos.z + dz * k;
    send({ t: "move", x: nx, y: ny, z: nz, yaw: 0 });
    st.pos = { x: nx, y: ny, z: nz };
    await sleep(80);
  }
  await sleep(200);
  const startHp = st.worldBoss.hp;
  let swings = 0;
  while (st.worldBoss && swings < 80) {
    send({ t: "boss_hit" });
    swings++;
    await sleep(260); // 略高於 HIT_MIN_INTERVAL_SECS(0.25s)，每次都算有效一擊
  }
  check("多次挖擊後首領被打倒（world_boss 變回 null）", st.worldBoss === null, `揮擊 ${swings} 次`);
  check("收到至少一次 boss_hit_ok 且血量遞減", st.hitAcks.length > 0 && st.hitAcks[0].hp < startHp,
    `acks=${st.hitAcks.length}`);
  check("最後一次 boss_hit_ok 標記 dead=true", st.hitAcks.length > 0 && st.hitAcks[st.hitAcks.length - 1].dead === true);
  softCheck("收到 worldboss defeat 橫幅", !!st.defeatBanner, st.defeatBanner ? st.defeatBanner.msg : "");
  softCheck("擊倒處掉出乙太礦（item_dropped）", st.drops.length > 0, `drops=${st.drops.length}`);
} else {
  console.log("（跳過②：本輪未撥中首領生成，機率事件）");
}

console.log(`\n結果：${pass} 通過，${warn} 警告，${fail} 失敗`);
ws.close();
process.exit(fail > 0 ? 1 : 0);
