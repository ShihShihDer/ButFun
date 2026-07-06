// ============================================================
// voxel-player-stats-qa.mjs — 玩家生存指標第一階段（溫和版）WS 功能 QA
// ============================================================
// 用真 WebSocket 直連隔離伺服器（記憶體/jsonl 模式、獨立 port），驗後端權威行為：
//   (b) 吃麵包 → 飢餓回復 + 背包扣（QA 授予幾個食物後真的吃）
//   (c) 跳高處落地 → 扣血（送一連串 Move：升到高處再落地，伺服器算落差扣血 + player_hurt）
//   (d) 潛水久 → 扣血（把頭移進水裡連續上報 → 溺水緩衝後扣血）※水體不一定在出生點，找不到就跳過
//   (e) 血歸零 → 溫柔重生（連摔到死 → 收到 respawn，座標回村/床）
//   (f) 重登 → 狀態保留（斷線重連，血/飢與斷線前一致）
//   + 濫用防護：空背包吃 → eat_fail（後端拒絕，不信客戶端）
// 不抄外部碼；全繁中註解；node --check 過。
//
// 需要伺服器以 BUTFUN_QA_DEBUG=1 啟動（qa_grant 才生效；正式線上該訊息惰性忽略）。

import WebSocket from "ws";

const PORT = process.env.VQA_PORT || 3939;
const WS_URL = `ws://127.0.0.1:${PORT}/voxel/ws`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let pass = 0, fail = 0;
function check(label, ok, extra = "") {
  if (ok) { pass++; console.log(`  ✅ ${label}${extra ? "  " + extra : ""}`); }
  else { fail++; console.log(`  ❌ ${label}${extra ? "  " + extra : ""}`); }
}

// 一條 QA 連線的薄封裝：收訊分流到 latest（各型別最後一則）＋事件旗標。
function makeClient(name) {
  const ws = new WebSocket(WS_URL);
  const state = {
    ws,
    ready: false,
    welcome: null,
    latestStats: null,   // 最後一則 player_stats
    hurtCount: 0,        // 收到 player_hurt 次數
    respawn: null,       // 最後一則 respawn
    eatOk: null,
    eatFail: null,
    inv: new Map(),      // block_id -> count
  };
  ws.on("message", (buf) => {
    let m; try { m = JSON.parse(buf.toString()); } catch { return; }
    switch (m.t) {
      case "welcome": state.welcome = m; state.ready = true; break;
      case "player_stats": state.latestStats = m; break;
      case "player_hurt": state.hurtCount++; break;
      case "respawn": state.respawn = m; break;
      case "eat_ok": state.eatOk = m; break;
      case "eat_fail": state.eatFail = m; break;
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

(async () => {
  console.log(`\n【玩家生存指標 WS 功能 QA】連 ${WS_URL}`);

  // ── 進場 ────────────────────────────────────────────────
  const NAME = "QA玩家_" + Math.floor(Math.random() * 100000);
  const c = makeClient(NAME);
  await new Promise((res, rej) => { c.ws.on("open", res); c.ws.on("error", rej); });
  send(c, { t: "join", name: NAME });
  const ok = await waitReady(c);
  check("進場收到 welcome", ok, ok ? `spawn=(${c.welcome.spawn.x.toFixed(1)},${c.welcome.spawn.y.toFixed(1)},${c.welcome.spawn.z.toFixed(1)})` : "");
  if (!ok) { c.ws.close(); process.exit(2); }
  await sleep(400);

  // 進場應立即收到一則 player_stats（後端權威、滿血滿飢）。
  check("進場收到 player_stats（後端權威）", !!c.latestStats,
    c.latestStats ? `HP=${c.latestStats.health}/${c.latestStats.max_health} 飢=${c.latestStats.hunger}/${c.latestStats.max_hunger}` : "");
  const startFull = c.latestStats && c.latestStats.health === c.latestStats.max_health && c.latestStats.hunger === c.latestStats.max_hunger;
  check("進場滿血滿飢", !!startFull);

  const spawn = c.welcome.spawn;

  // ── (b) 吃麵包 → 飢餓回復 + 背包扣 ─────────────────────────
  console.log("\n(b) 吃麵包 → 飢餓回復 + 背包扣");
  // 先把飢餓消耗一點：不好等 30 分鐘，改用「已飽時吃會被拒」反證＋授予後真的吃。
  // 授予 3 個麵包（19）。
  send(c, { t: "qa_grant", item_id: 19, count: 3 });
  await sleep(400);
  check("QA 授予麵包後背包有 3 個", c.inv.get(19) === 3, `bread=${c.inv.get(19)}`);

  // 滿飢時吃 → 應被拒（吃不下，濫用防護：飽了不刷）。
  c.eatFail = null; c.eatOk = null;
  send(c, { t: "eat", item_id: 19 });
  await sleep(400);
  check("滿飢時吃被拒（eat_fail：吃不下）", !!c.eatFail && !c.eatOk, c.eatFail ? c.eatFail.reason : "");
  check("被拒時背包沒被扣", c.inv.get(19) === 3, `bread=${c.inv.get(19)}`);

  // 讓飢餓降下來：等伺服器 tick 衰減（衰減慢，改用溺水/摔？不影響飢餓）。
  // 改法：先摔一次不影響飢餓；直接等幾秒讓飢餓略降，然後這裡吃不一定回復可見量。
  // 為了穩定驗「吃真的扣背包＋回復」，改成：先用溺水/摔把血扣一點（不動飢餓），再無法讓飢餓降太多。
  // → 最穩：直接等到飢餓 < 100（tick 每秒 -0.055，等約 20 秒讓它掉 ~1）。
  console.log("  等飢餓自然衰減一點（tick 每秒 -0.055）…");
  const t0 = Date.now();
  while (Date.now() - t0 < 22000) {
    if (c.latestStats && c.latestStats.hunger < c.latestStats.max_hunger) break;
    await sleep(500);
  }
  const hungerBefore = c.latestStats.hunger;
  check("飢餓隨時間衰減（<100）", hungerBefore < 100, `飢=${hungerBefore}`);

  // 現在吃一個麵包 → 飢餓回復（回滿或往上）＋背包 -1。
  c.eatOk = null; c.eatFail = null;
  const breadBefore = c.inv.get(19);
  send(c, { t: "eat", item_id: 19 });
  await sleep(500);
  check("吃麵包成功（eat_ok）", !!c.eatOk && !c.eatFail);
  check("吃後背包 -1", c.inv.get(19) === breadBefore - 1, `bread ${breadBefore}→${c.inv.get(19)}`);
  check("吃後飢餓回復（≥吃前）", c.latestStats.hunger >= hungerBefore, `飢 ${hungerBefore}→${c.latestStats.hunger}`);

  // ── (c) 跳高處落地 → 扣血 + 受傷事件 ───────────────────────
  console.log("\n(c) 高處落地 → 扣血 + player_hurt");
  const hpBeforeFall = c.latestStats.health;
  c.hurtCount = 0;
  // 模擬：先「升」到高處（Move 到 spawn.y + 12），再逐步「落地」回 spawn.y。
  // 伺服器用位置序列的峰值-落地算落差；>4 格才扣血，12 格會扣 ~8 點。
  const highY = spawn.y + 12;
  send(c, { t: "move", x: spawn.x, y: spawn.y, z: spawn.z, yaw: 0 });
  await sleep(120);
  send(c, { t: "move", x: spawn.x, y: highY, z: spawn.z, yaw: 0 }); // 升到高處（記峰值）
  await sleep(120);
  // 逐步下降（下降中）→ 落地停住（結算）。
  for (let yy = highY; yy >= spawn.y; yy -= 3) {
    send(c, { t: "move", x: spawn.x, y: yy, z: spawn.z, yaw: 0 });
    await sleep(80);
  }
  send(c, { t: "move", x: spawn.x, y: spawn.y, z: spawn.z, yaw: 0 }); // 落地
  await sleep(120);
  send(c, { t: "move", x: spawn.x, y: spawn.y, z: spawn.z, yaw: 0 }); // 站定（觸發結算）
  await sleep(500);
  const hpAfterFall = c.latestStats.health;
  check("高處落地扣血", hpAfterFall < hpBeforeFall, `HP ${hpBeforeFall}→${hpAfterFall}`);
  check("落地觸發 player_hurt（前端閃紅暈）", c.hurtCount >= 1, `hurt×${c.hurtCount}`);

  // ── (d) 潛水久 → 扣血 ──────────────────────────────────────
  console.log("\n(d) 潛水久 → 溺水扣血（頭泡水，撐過緩衝後開始扣）");
  // 先回滿血（吃/等，避免血已低干擾），再把頭移進海水裡連續上報。
  // 海洋在遠處（SEA_LEVEL=5），y=3 頭在 y≈4.5 仍在水面下。試幾個遠座標，找到水就驗。
  const drownSpots = [[-120, 3, 120], [120, 3, 120], [120, 3, -120], [-120, 3, -120], [300, 3, 0], [0, 3, 300]];
  let drowned = false, drownWhere = null, drownHurt = 0;
  for (const [x, y, z] of drownSpots) {
    c.hurtCount = 0;
    const hp0 = c.latestStats.health;
    for (let i = 0; i < 95 && !drowned; i++) {
      send(c, { t: "move", x, y, z, yaw: 0 });
      await sleep(100);
      if (c.hurtCount > 0 && c.latestStats.health < hp0) { drowned = true; drownWhere = [x, y, z]; drownHurt = c.hurtCount; }
      if (c.respawn) { drowned = true; drownWhere = [x, y, z]; drownHurt = c.hurtCount; }
    }
    if (drowned) break;
    send(c, { t: "move", x, y: 24, z, yaw: 0 }); // 浮出水面重置溺水累加器
    await sleep(600);
  }
  check("頭泡水久 → 溺水扣血", drowned, drowned ? `at (${drownWhere}) hurt×${drownHurt} HP=${c.latestStats.health}` : "（探測的遠海座標都沒水，跳過——溺水純邏輯已有 3 個單元測試覆蓋）");
  // 浮出水面 → 溺水累加器歸零（回地面準備後續測試）。
  send(c, { t: "move", x: spawn.x, y: spawn.y, z: spawn.z, yaw: 0 });
  await sleep(400);

  // ── (e) 連摔到死 → 溫柔重生 ────────────────────────────────
  console.log("\n(e) 連摔到死 → 溫柔重生（背包不掉、血飢回滿）");
  c.respawn = null;
  const breadBeforeDeath = c.inv.get(19) || 0;
  // 連續從高處落地把血摔光（每摔 ~8 點，血 20 → 3 摔內見底）。
  for (let i = 0; i < 4 && !c.respawn; i++) {
    send(c, { t: "move", x: spawn.x, y: spawn.y, z: spawn.z, yaw: 0 });
    await sleep(100);
    send(c, { t: "move", x: spawn.x, y: spawn.y + 16, z: spawn.z, yaw: 0 });
    await sleep(100);
    for (let yy = spawn.y + 16; yy >= spawn.y; yy -= 4) {
      send(c, { t: "move", x: spawn.x, y: yy, z: spawn.z, yaw: 0 });
      await sleep(70);
    }
    send(c, { t: "move", x: spawn.x, y: spawn.y, z: spawn.z, yaw: 0 });
    await sleep(300);
  }
  check("血歸零 → 收到 respawn（溫柔重生）", !!c.respawn, c.respawn ? `→(${c.respawn.x.toFixed(1)},${c.respawn.y.toFixed(1)},${c.respawn.z.toFixed(1)})` : "");
  check("重生帶暖心提示", !!(c.respawn && c.respawn.message), c.respawn ? `「${c.respawn.message}」` : "");
  await sleep(300);
  check("重生後血飢回滿", c.latestStats && c.latestStats.health === c.latestStats.max_health && c.latestStats.hunger === c.latestStats.max_hunger,
    c.latestStats ? `HP=${c.latestStats.health} 飢=${c.latestStats.hunger}` : "");
  check("重生後背包不掉落（麵包還在）", (c.inv.get(19) || 0) === breadBeforeDeath, `bread=${c.inv.get(19)}（死前 ${breadBeforeDeath}）`);

  // ── (f) 重登 → 狀態保留 ────────────────────────────────────
  console.log("\n(f) 重登 → 狀態保留");
  // 先製造一個「非滿」狀態並讓它落地：摔一次讓血非滿。
  // 先站定一拍（清掉 (e) 死亡迴圈的快速位移殘留），再做一次乾淨的高處落地。
  send(c, { t: "move", x: spawn.x, y: spawn.y, z: spawn.z, yaw: 0 });
  await sleep(300);
  send(c, { t: "move", x: spawn.x, y: spawn.y, z: spawn.z, yaw: 0 });
  await sleep(200);
  send(c, { t: "move", x: spawn.x, y: spawn.y + 10, z: spawn.z, yaw: 0 });
  await sleep(120);
  for (let yy = spawn.y + 10; yy >= spawn.y; yy -= 3) { send(c, { t: "move", x: spawn.x, y: yy, z: spawn.z, yaw: 0 }); await sleep(70); }
  send(c, { t: "move", x: spawn.x, y: spawn.y, z: spawn.z, yaw: 0 });
  await sleep(120);
  send(c, { t: "move", x: spawn.x, y: spawn.y, z: spawn.z, yaw: 0 });
  await sleep(150); // 短等：讓落地結算送出 player_stats，但別等到飽食回血把血補回去
  const hpToPersist = c.latestStats.health;
  check("重登前造出非滿血狀態", hpToPersist < c.latestStats.max_health, `HP=${hpToPersist}`);
  // 斷線（伺服器 cleanup 時落地 stats；但訪客名不一定持久——本 QA 用訪客名，
  // 故驗「同一 session 內記憶體保留」＋「檔案有寫入」兩件事的其中之一）。
  // 重連同名 → 記憶體仍在（同 hub、未重啟），血應保留。
  c.ws.close();
  await sleep(600);
  const c2 = makeClient(NAME);
  await new Promise((res, rej) => { c2.ws.on("open", res); c2.ws.on("error", rej); });
  send(c2, { t: "join", name: NAME });
  await waitReady(c2);
  await sleep(500);
  check("重連收到 player_stats", !!c2.latestStats,
    c2.latestStats ? `HP=${c2.latestStats.health} 飢=${c2.latestStats.hunger}` : "");
  check("重登狀態保留（血與斷線前一致）", c2.latestStats && c2.latestStats.health === hpToPersist,
    c2.latestStats ? `斷線前 HP=${hpToPersist}，重登 HP=${c2.latestStats.health}` : "");

  // ── 濫用防護：空背包吃 → eat_fail ──────────────────────────
  console.log("\n濫用防護：吃背包沒有的食物 → eat_fail（後端拒絕）");
  c2.eatFail = null; c2.eatOk = null;
  send(c2, { t: "eat", item_id: 63 }); // 烤魚，背包沒有
  await sleep(400);
  check("吃背包沒有的食物被拒（後端權威）", !!c2.eatFail && !c2.eatOk, c2.eatFail ? c2.eatFail.reason : "");

  c2.ws.close();
  await sleep(200);

  console.log(`\n══════════════════════════════════════════`);
  console.log(`玩家生存指標 WS 功能 QA：${pass} 通過 / ${fail} 失敗`);
  console.log(`══════════════════════════════════════════`);
  process.exit(fail === 0 ? 0 : 1);
})().catch((e) => { console.error("QA 失敗:", e); process.exit(2); });
