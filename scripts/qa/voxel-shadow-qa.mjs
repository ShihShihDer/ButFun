// ============================================================
// voxel-shadow-qa.mjs — 暗影生物 v1（怪物/抵禦第一刀）WS 功能 QA
// ============================================================
// 用真 WebSocket 直連隔離伺服器（記憶體/jsonl 模式、獨立 port），驗後端權威行為：
//   (a) 撥鐘入夜 → 遠離村莊的暗處生成暗影（數量 ≤ 6）、漂向玩家、觸碰緩慢扣血（溫柔節奏）
//   (b) 在暗影旁點火把 → 光=庇護：暗影進入亮區立即化成輕煙（shadow_puff）＋掉乙太礦
//   (c) 挖擊 3 下 → 消散＋掉乙太礦；隔半張地圖打 → 伺服器靜默拒絕（客戶端不能自報擊殺）
//   (d) 撥鐘回白天 → 全部暗影消散、快照陣列清空
//   (e) 站在遠方居民家旁 → 暗影靠近時居民冒害怕泡泡（best-effort：漂移需時，逾時只警告）
// 需要伺服器以 BUTFUN_QA_DEBUG=1 啟動（qa_set_time / qa_grant 才生效；正式線上皆惰性忽略）。
// 不抄外部碼；全繁中註解；node --check 過。

import WebSocket from "ws";

const PORT = process.env.VQA_PORT || 3941;
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

// 一條 QA 連線的薄封裝：收訊分流到 latest（各型別最後一則）＋事件累計。
function makeClient(name) {
  const ws = new WebSocket(WS_URL);
  const state = {
    ws, name,
    ready: false,
    welcome: null,
    latestStats: null,     // 最後一則 player_stats
    hurtEvents: [],        // 每次 player_hurt 的收到時刻（量觸傷節奏用）
    respawn: null,
    shadows: [],           // 最後一份 players 快照裡的暗影陣列
    residents: [],         // 最後一份 players 快照裡的居民陣列
    maxShadowCount: 0,     // 觀測到的暗影同時數量峰值（驗上限）
    puffs: [],             // shadow_puff 事件（消散輕煙）
    hitOks: [],            // shadow_hit_ok 事件
    drops: [],             // item_dropped 事件
    inv: new Map(),
  };
  ws.on("message", (buf) => {
    let m; try { m = JSON.parse(buf.toString()); } catch { return; }
    switch (m.t) {
      case "welcome": state.welcome = m; state.ready = true; break;
      case "player_stats": state.latestStats = m; break;
      case "player_hurt": state.hurtEvents.push(Date.now()); break;
      case "respawn": state.respawn = m; break;
      case "players":
        state.shadows = m.shadows || [];
        state.residents = m.residents || [];
        state.maxShadowCount = Math.max(state.maxShadowCount, state.shadows.length);
        break;
      case "shadow_puff": state.puffs.push(m); break;
      case "shadow_hit_ok": state.hitOks.push(m); break;
      case "item_dropped": state.drops.push(m); break;
      case "inv_sync":
        state.inv.clear();
        for (const [b, c] of (m.items || [])) state.inv.set(b, c);
        break;
      case "inv_update":
        if (m.count > 0) state.inv.set(m.block_id, m.count);
        else state.inv.delete(m.block_id);
        break;
    }
  });
  return state;
}
async function waitReady(c, ms = 5000) {
  const t0 = Date.now();
  while (!c.ready && Date.now() - t0 < ms) await sleep(50);
  return c.ready;
}
function send(c, obj) { c.ws.send(JSON.stringify(obj)); }
async function waitFor(fn, ms, step = 200) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    const v = fn();
    if (v) return v;
    await sleep(step);
  }
  return null;
}

(async () => {
  console.log(`\n【暗影生物 WS 功能 QA】連 ${WS_URL}`);
  const NAME = "QA夜行者_" + Math.floor(Math.random() * 100000);
  const c = makeClient(NAME);
  await new Promise((res, rej) => { c.ws.on("open", res); c.ws.on("error", rej); });
  send(c, { t: "join", name: NAME });
  const ok = await waitReady(c);
  check("進場收到 welcome", ok);
  if (!ok) { c.ws.close(); process.exit(2); }
  const spawn = c.welcome.spawn;
  await sleep(400);

  // ── 前置：白天不該有暗影 ─────────────────────────────────
  console.log("\n(前置) 白天世界無暗影");
  send(c, { t: "qa_set_time", time: 0.5 }); // 正午
  await sleep(800);
  check("白天快照暗影陣列為空", c.shadows.length === 0, `shadows=${c.shadows.length}`);

  // ── (a) 撥鐘入夜 → 暗處生成、漂向玩家、觸碰緩慢扣血 ───────
  console.log("\n(a) 入夜生成 → 漂近 → 觸碰緩慢扣血（溫柔節奏）");
  // 走遠離村莊（村莊庇護半徑 48 格內不生成）：水平大跳（y 不變，不觸發跌落結算）。
  const fx = spawn.x + 150, fz = spawn.z;
  send(c, { t: "move", x: fx, y: spawn.y, z: fz, yaw: 0 });
  await sleep(300);
  send(c, { t: "qa_set_time", time: 0.86 }); // 入夜（整夜約 200 秒真實時間，足夠驗完 a–c）
  // 生成檢查每 3 秒擲一次骰（機率 0.5）→ 等最多 30 秒應該有暗影冒出來。
  const spawned = await waitFor(() => (c.shadows.length > 0 ? c.shadows : null), 30000);
  check("夜裡暗處生成暗影", !!spawned, spawned ? `第一隻在 (${spawned[0].x.toFixed(1)},${spawned[0].y.toFixed(1)},${spawned[0].z.toFixed(1)})` : "30 秒無生成");
  if (!spawned) { c.ws.close(); process.exit(1); }
  // 生成距離：應在玩家 14–26 格環帶（暗處視野邊緣現身、不貼臉跳臉）。
  {
    const d = Math.hypot(spawned[0].x - fx, spawned[0].z - fz);
    check("生成點在玩家視野邊緣環帶（14–26 格）", d >= 13 && d <= 27, `d=${d.toFixed(1)}`);
  }
  // 漂向玩家：取同一隻 id，比對數秒後距離變近。
  {
    const first = spawned[0];
    const d0 = Math.hypot(first.x - fx, first.z - fz);
    await sleep(5000);
    const now = c.shadows.find((s) => s.id === first.id);
    softCheck("暗影緩慢漂向玩家（5 秒後距離縮短）", !!now && Math.hypot(now.x - fx, now.z - fz) < d0,
      now ? `d ${d0.toFixed(1)}→${Math.hypot(now.x - fx, now.z - fz).toFixed(1)}` : "（該隻已消散/超出）");
  }
  // 觸碰扣血：直接站到最近那隻暗影的位置（QA 走捷徑瞬移；正式玩家是被漂近碰到）。
  const target = c.shadows[0];
  c.hurtEvents = [];
  send(c, { t: "move", x: target.x, y: target.y, z: target.z, yaw: 0 });
  // 貼身站 9 秒：期間每 300ms 跟著暗影微調位置（它還會漂），收集 player_hurt。
  for (let i = 0; i < 30; i++) {
    const cur = c.shadows.find((s) => s.id === target.id) || c.shadows[0];
    if (cur) send(c, { t: "move", x: cur.x, y: cur.y, z: cur.z, yaw: 0 });
    await sleep(300);
  }
  const hurts = c.hurtEvents.length;
  check("觸碰暗影會扣血（player_hurt）", hurts >= 2, `9 秒內 hurt×${hurts}`);
  // 溫柔節奏釘樁：冷卻 2 秒 → 9 秒內觸傷次數必 ≤ 6（跌落結算最多多 1 次），絕非狂扣。
  check("扣血節奏溫柔（≤ 6 次/9 秒，1 血/2 秒冷卻）", hurts <= 6, `hurt×${hurts}`);
  check("血量仍遠高於 0（絕不秒殺）", c.latestStats && c.latestStats.health > 0 && !c.respawn,
    c.latestStats ? `HP=${c.latestStats.health}/${c.latestStats.max_health}` : "");

  // ── (b) 光=庇護：在暗影旁點火把 → 立即消散＋掉乙太礦 ───────
  console.log("\n(b) 光=庇護：暗影旁點火把 → 化成輕煙＋掉乙太礦");
  send(c, { t: "qa_set_time", time: 0.9 }); // 重釘夜晚（防前段耗時讓黎明自然到來）
  send(c, { t: "qa_grant", item_id: 31, count: 8 }); // 火把
  await sleep(400);
  check("QA 授予火把", (c.inv.get(31) || 0) >= 8, `torch=${c.inv.get(31)}`);
  const victim = await waitFor(() => c.shadows[0] || null, 15000);
  check("場上有暗影可驗光庇護", !!victim);
  if (victim) {
    c.puffs = []; c.drops = [];
    // 瞬移到暗影旁、對腳邊一格空氣放火把（reach 內；光圈 8 格立即罩住它）。
    send(c, { t: "move", x: victim.x + 1.5, y: victim.y, z: victim.z, yaw: 0 });
    await sleep(200);
    const tx = Math.floor(victim.x + 1.5), ty = Math.floor(victim.y) + 1, tz = Math.floor(victim.z);
    send(c, { t: "place", x: tx, y: ty, z: tz, b: 31 });
    const gonePuff = await waitFor(() => (c.puffs.length > 0 ? c.puffs : null), 4000);
    check("暗影進入亮區 → 化成輕煙（shadow_puff）", !!gonePuff, gonePuff ? `puff×${gonePuff.length}` : "4 秒內無 puff");
    // 快照 10Hz 廣播，等到該 id 從快照消失（instant 檢查會 race 到上一份快照）。
    const removed = await waitFor(() => (!c.shadows.find((s) => s.id === victim.id) ? true : null), 3000);
    check("該暗影已從世界移除", !!removed);
    const shard = await waitFor(() => c.drops.find((d) => d.item_id === 58) || null, 3000);
    check("亮區消散掉一枚乙太礦（溫柔獎勵）", !!shard, shard ? `by=${shard.dropped_by}` : "");
    // 光圈內不再生成：站在火把旁 10 秒，環帶內若再生成也不會貼進光圈（無 hurt）。
    c.hurtEvents = [];
    send(c, { t: "move", x: tx + 0.5, y: victim.y, z: tz + 0.5, yaw: 0 });
    await sleep(10000);
    check("站在火把光圈內 10 秒未被觸傷（光圈是安全區）", c.hurtEvents.length === 0, `hurt×${c.hurtEvents.length}`);
  }

  // ── (c) 反擊：挖擊 3 下消散＋掉乙太礦；遠距挖擊被拒 ─────────
  console.log("\n(c) 反擊：挖擊 3 下消散；隔半張地圖打 → 伺服器靜默拒絕");
  send(c, { t: "qa_set_time", time: 0.9 }); // 重釘夜晚
  // 走出光圈遠一點，等下一隻暗影。
  send(c, { t: "move", x: fx + 60, y: spawn.y, z: fz, yaw: 0 });
  const prey = await waitFor(() => {
    const s = c.shadows[0];
    return s && Math.hypot(s.x - (fx + 60), s.z - fz) > 2 ? s : null;
  }, 30000);
  check("場上有暗影可驗反擊", !!prey);
  if (prey) {
    // 濫用防護：先隔遠打 → 應無任何 shadow_hit_ok（伺服器驗 reach，靜默拒絕）。
    c.hitOks = [];
    send(c, { t: "move", x: prey.x + 40, y: prey.y, z: prey.z, yaw: 0 });
    await sleep(300);
    send(c, { t: "shadow_hit", id: prey.id });
    await sleep(800);
    check("隔 40 格挖擊被伺服器拒絕（無 hit_ok，客戶端不能自報擊殺）", c.hitOks.length === 0);
    // 貼近打 3 下（間隔 > 揮擊節流 0.25s）。
    c.puffs = []; c.drops = []; c.hitOks = [];
    for (let i = 0; i < 3; i++) {
      const cur = c.shadows.find((s) => s.id === prey.id);
      if (!cur) break;
      send(c, { t: "move", x: cur.x + 1.2, y: cur.y, z: cur.z, yaw: 0 });
      await sleep(150);
      send(c, { t: "shadow_hit", id: prey.id });
      await sleep(450);
    }
    check("三下挖擊皆獲伺服器回饋（shadow_hit_ok×3）", c.hitOks.length === 3,
      `hit_ok=${JSON.stringify(c.hitOks.map((h) => h.hits))}`);
    check("第三下標記消散（gone=true）", c.hitOks.length === 3 && c.hitOks[2].gone === true);
    check("擊散後化成輕煙", c.puffs.length >= 1);
    check("擊散後從世界移除", !c.shadows.find((s) => s.id === prey.id));
    const shard = await waitFor(() => c.drops.find((d) => d.item_id === 58 && d.dropped_by === NAME) || null, 3000);
    check("擊散掉一枚乙太礦（署名擊散者）", !!shard);
  }

  // ── (d) 黎明消散 ──────────────────────────────────────────
  console.log("\n(d) 黎明 → 全部暗影整批消散");
  // 等場上至少一隻，再撥鐘到白天。
  await waitFor(() => (c.shadows.length > 0 ? true : null), 20000);
  const beforeDawn = c.shadows.length;
  c.puffs = [];
  send(c, { t: "qa_set_time", time: 0.5 });
  const cleared = await waitFor(() => (c.shadows.length === 0 ? true : null), 4000);
  check("撥回白天後暗影全部消散", !!cleared, `${beforeDawn} → ${c.shadows.length}`);
  if (beforeDawn > 0) check("黎明消散有輕煙", c.puffs.length >= beforeDawn, `puff×${c.puffs.length}`);
  check("整場觀測暗影同時數量峰值 ≤ 上限 6", c.maxShadowCount <= 6, `峰值=${c.maxShadowCount}`);

  // ── (e) 居民害怕（best-effort）────────────────────────────
  console.log("\n(e) 居民見暗影冒害怕泡泡（best-effort：漂移需時）");
  // 賽勒住西方 (-75,0)（遠離村莊庇護半徑）；站他家旁讓暗影生成後漂近他。
  const resident = c.residents.find((r) => Math.hypot(r.x, r.z) > 55) || c.residents[0];
  if (resident) {
    send(c, { t: "move", x: resident.x + 4, y: resident.y, z: resident.z, yaw: 0 });
    await sleep(300);
    send(c, { t: "qa_set_time", time: 0.86 });
    const fearWords = ["黑黑的", "影子"];
    const seenFear = await waitFor(() => {
      const r = c.residents.find((q) => q.id === resident.id);
      return r && r.say && fearWords.some((w) => r.say.includes(w)) ? r.say : null;
    }, 60000);
    softCheck("居民冒害怕泡泡", !!seenFear, seenFear ? `「${seenFear}」（${resident.name}）` : "60 秒內未觀測到（暗影漂移路徑隨機）");
  } else {
    softCheck("居民害怕（找不到遠方居民可驗）", false);
  }
  // 療癒底線：整場觀測期間居民永不掉血（居民本就無血量系統——此處驗證沒有任何居民消失）。
  check("居民一位都沒少（療癒底線：只會怕、不會死）", c.residents.length >= 4, `居民=${c.residents.length}`);

  send(c, { t: "qa_set_time", time: 0.5 }); // 收尾撥回白天
  await sleep(300);
  c.ws.close();
  await sleep(200);

  console.log(`\n══════════════════════════════════════════`);
  console.log(`暗影生物 WS 功能 QA：${pass} 通過 / ${fail} 失敗 / ${warn} 警告`);
  console.log(`══════════════════════════════════════════`);
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
