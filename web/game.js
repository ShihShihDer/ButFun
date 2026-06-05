// ButFun 前端 — Phase 0 垂直切片
// 連上權威伺服器，送移動意圖，渲染所有玩家的權威狀態快照。
// 刻意用原生 canvas，零外部相依；之後要做 tilemap 再換 Phaser。

(() => {
  "use strict";

  const canvas = document.getElementById("game");
  const ctx = canvas.getContext("2d");

  // ---- 狀態 ----
  let ws = null;
  let myId = null;
  let world = { width: 2000, height: 2000 };
  let myName = "";
  // id -> { name, species, x, y (目標), rx, ry (渲染中插值位置) }
  const players = new Map();
  const keys = { up: false, down: false, left: false, right: false };
  let lastSentInput = "";
  // 伺服器廣播的農地狀態（含每格 state / dry）；進場前為 null。
  let field = null;
  let myEther = 0;
  // 伺服器廣播的日夜狀態 { phase, light }；進場前為 null（render 時當白天、不疊夜色）。
  let daynight = null;

  // ---- 觸控搖桿狀態(手機沒鍵盤,用拖曳設方向) ----
  let touchOrigin = null;   // 手指按下的初始位置
  let touchCurrent = null;  // 手指目前的位置
  // 最近一次 render 用的鏡頭左上角（世界座標），給點擊換算用。
  const lastCam = { x: 0, y: 0 };

  // ---- 畫布尺寸 ----
  function resize() {
    canvas.width = window.innerWidth;
    canvas.height = window.innerHeight;
  }
  window.addEventListener("resize", resize);
  resize();

  // ---- 連線 ----
  function connect(name, species) {
    myName = name;
    const proto = location.protocol === "https:" ? "wss" : "ws";
    ws = new WebSocket(`${proto}://${location.host}/ws`);

    ws.onopen = () => {
      ws.send(JSON.stringify({ type: "join", name, species }));
    };

    ws.onmessage = (ev) => {
      let msg;
      try { msg = JSON.parse(ev.data); } catch { return; }
      handleServerMsg(msg);
    };

    ws.onclose = () => {
      addChat("系統", "與伺服器的連線中斷了，重新整理頁面再試。");
    };
  }

  function handleServerMsg(msg) {
    switch (msg.type) {
      case "welcome":
        myId = msg.id;
        world = msg.world;
        enterGame();
        break;
      case "snapshot": {
        const seen = new Set();
        for (const p of msg.players) {
          seen.add(p.id);
          const existing = players.get(p.id);
          if (existing) {
            existing.name = p.name;
            existing.species = p.species;
            existing.x = p.x;
            existing.y = p.y;
          } else {
            players.set(p.id, { ...p, rx: p.x, ry: p.y });
          }
        }
        // 移除快照中已不存在的玩家
        for (const id of players.keys()) {
          if (!seen.has(id)) players.delete(id);
        }
        document.getElementById("hudPlayers").textContent = `線上：${msg.players.length}`;
        // 農地狀態 + 我的乙太 + 日夜
        field = msg.field;
        daynight = msg.daynight;
        if (daynight) updateDayNightHud(daynight);
        const me = msg.players.find((p) => p.id === myId);
        if (me) {
          myEther = me.ether;
          document.getElementById("hudEther").textContent = `乙太：${myEther}`;
        }
        break;
      }
      case "chat":
        addChat(msg.from, msg.text);
        break;
      case "player_left":
        players.delete(msg.id);
        break;
    }
  }

  // 日夜階段 → HUD 顯示文字（emoji + 繁中），讓玩家一眼知道現在是一天的哪個時段。
  const PHASE_LABELS = {
    dawn: "🌅 破曉",
    day: "☀️ 白天",
    dusk: "🌇 黃昏",
    night: "🌙 夜晚",
  };
  function updateDayNightHud(dn) {
    const el = document.getElementById("hudTime");
    if (el) el.textContent = PHASE_LABELS[dn.phase] || "—";
  }

  // ---- 輸入 ----
  function sendInputIfChanged() {
    const sig = `${keys.up}${keys.down}${keys.left}${keys.right}`;
    if (sig !== lastSentInput && ws && ws.readyState === WebSocket.OPEN) {
      lastSentInput = sig;
      ws.send(JSON.stringify({ type: "input", ...keys }));
    }
  }

  function keyToDir(e) {
    switch (e.key) {
      case "w": case "W": case "ArrowUp": return "up";
      case "s": case "S": case "ArrowDown": return "down";
      case "a": case "A": case "ArrowLeft": return "left";
      case "d": case "D": case "ArrowRight": return "right";
      default: return null;
    }
  }

  window.addEventListener("keydown", (e) => {
    // 在任何文字輸入欄打字時，完全不攔截遊戲按鍵——否則 w/a/s/d、方向鍵會被
    // e.preventDefault() 吃掉、角色還在背景亂走，Enter 也被搶去 focus 聊天。
    // 尤其建議箱 #suggestText 是 <textarea>（先前只擋 INPUT 沒擋它），玩家寫
    // 回饋時打到這些字就壞掉——而建議箱正是 devloop 收回饋的主要管道。
    const el = document.activeElement;
    if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA")) return;
    if (e.key === "Enter") { document.getElementById("chatText").focus(); return; }
    const dir = keyToDir(e);
    if (dir) { keys[dir] = true; sendInputIfChanged(); e.preventDefault(); }
  });
  window.addEventListener("keyup", (e) => {
    const dir = keyToDir(e);
    if (dir) { keys[dir] = false; sendInputIfChanged(); }
  });

  // 視窗失焦 / 分頁切到背景時，瀏覽器多半不會再送 keyup——若此時還按著方向鍵，
  // keys[dir] 會卡在 true，伺服器持續整合位置，角色在背景一直走（玩家切回來時
  // 人已飄到別處／撞牆）。延續「角色別在玩家沒在控時亂走」的修復家族：失焦就清掉
  // 所有移動鍵並把「停止」意圖送給伺服器。
  function releaseAllKeys() {
    keys.up = keys.down = keys.left = keys.right = false;
    sendInputIfChanged();
  }
  window.addEventListener("blur", releaseAllKeys);
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) releaseAllKeys();
  });

  // ---- 觸控:任何地方按下拖曳當搖桿,放開即停止 ----
  function setTouchKeys(dx, dy) {
    const dead = 14;
    keys.up = dy < -dead;
    keys.down = dy > dead;
    keys.left = dx < -dead;
    keys.right = dx > dead;
    sendInputIfChanged();
  }
  canvas.addEventListener("touchstart", (e) => {
    if (e.touches.length === 0) return;
    const t = e.touches[0];
    touchOrigin = { x: t.clientX, y: t.clientY };
    touchCurrent = { x: t.clientX, y: t.clientY };
    e.preventDefault();
  }, { passive: false });
  canvas.addEventListener("touchmove", (e) => {
    if (!touchOrigin || e.touches.length === 0) return;
    const t = e.touches[0];
    touchCurrent = { x: t.clientX, y: t.clientY };
    setTouchKeys(t.clientX - touchOrigin.x, t.clientY - touchOrigin.y);
    e.preventDefault();
  }, { passive: false });
  function endTouch(e) {
    // 幾乎沒移動的觸碰當成「輕點」→ 農地互動（拖曳則是搖桿移動，不互動）。
    // 容差 22px(>移動死區 14px):手指按下自然會稍微滑動,先前 12px 太嚴,
    // 玩家想點田格常被誤判成搖桿微動、整個 tap 被吃掉。
    if (touchOrigin && e.changedTouches && e.changedTouches.length) {
      const t = e.changedTouches[0];
      const moved = Math.hypot(t.clientX - touchOrigin.x, t.clientY - touchOrigin.y);
      if (moved < 22) farmAtScreen(t.clientX, t.clientY);
    }
    touchOrigin = null;
    touchCurrent = null;
    setTouchKeys(0, 0);
    e.preventDefault();
  }
  canvas.addEventListener("touchend", endTouch, { passive: false });
  canvas.addEventListener("touchcancel", endTouch, { passive: false });

  // ---- 渲染迴圈 ----
  function render() {
    ctx.clearRect(0, 0, canvas.width, canvas.height);

    const me = myId ? players.get(myId) : null;
    // 插值所有玩家位置，讓 15Hz 快照看起來平滑
    for (const p of players.values()) {
      p.rx += (p.x - p.rx) * 0.3;
      p.ry += (p.y - p.ry) * 0.3;
    }

    // 鏡頭跟隨自己
    const camX = me ? me.rx - canvas.width / 2 : world.width / 2 - canvas.width / 2;
    const camY = me ? me.ry - canvas.height / 2 : world.height / 2 - canvas.height / 2;
    lastCam.x = camX;
    lastCam.y = camY;

    drawGround(camX, camY);
    drawField(camX, camY);

    // 畫玩家
    for (const p of players.values()) {
      const sx = p.rx - camX;
      const sy = p.ry - camY;
      const isMe = p.id === myId;

      ctx.beginPath();
      ctx.arc(sx, sy, 14, 0, Math.PI * 2);
      ctx.fillStyle = isMe ? "#c9a24b" : "#6fa8dc";
      ctx.fill();
      ctx.lineWidth = 2;
      ctx.strokeStyle = "rgba(0,0,0,0.4)";
      ctx.stroke();

      ctx.fillStyle = "#e8e0cf";
      ctx.font = "13px system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.fillText(p.name, sx, sy - 22);
    }

    // 日夜染色：亮度越低，疊越濃的夜色（蓋住世界與玩家，但不蓋觸控搖桿）。
    // light 落在 [0.2, 1.0]；白天(≈1)幾乎不疊、午夜(0.2)疊最濃但仍看得見（療癒、非全黑）。
    if (daynight) {
      const dark = Math.max(0, Math.min(1, 1 - daynight.light));
      if (dark > 0.001) {
        ctx.fillStyle = `rgba(14,20,52,${(dark * 0.72).toFixed(3)})`;
        ctx.fillRect(0, 0, canvas.width, canvas.height);
      }
    }

    // 觸控搖桿視覺(只在按住時出現)
    if (touchOrigin && touchCurrent) {
      const dx = touchCurrent.x - touchOrigin.x;
      const dy = touchCurrent.y - touchOrigin.y;
      const dist = Math.hypot(dx, dy);
      const cap = 50;
      const r = dist > 0 ? Math.min(dist, cap) / dist : 0;
      ctx.strokeStyle = "rgba(255,255,255,0.35)";
      ctx.lineWidth = 2;
      ctx.beginPath(); ctx.arc(touchOrigin.x, touchOrigin.y, cap, 0, Math.PI * 2); ctx.stroke();
      ctx.fillStyle = "rgba(201,162,75,0.9)";
      ctx.beginPath(); ctx.arc(touchOrigin.x + dx * r, touchOrigin.y + dy * r, 18, 0, Math.PI * 2); ctx.fill();
    }

    requestAnimationFrame(render);
  }

  // 畫一張帶網格的地面 + 世界邊界，給空間感
  function drawGround(camX, camY) {
    const grid = 80;
    ctx.fillStyle = "#12331f";
    ctx.fillRect(0, 0, canvas.width, canvas.height);

    ctx.strokeStyle = "rgba(255,255,255,0.05)";
    ctx.lineWidth = 1;
    const startX = -((camX % grid) + grid) % grid;
    const startY = -((camY % grid) + grid) % grid;
    for (let x = startX; x < canvas.width; x += grid) {
      ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, canvas.height); ctx.stroke();
    }
    for (let y = startY; y < canvas.height; y += grid) {
      ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(canvas.width, y); ctx.stroke();
    }

    // 世界邊界
    ctx.strokeStyle = "rgba(201,162,75,0.6)";
    ctx.lineWidth = 4;
    ctx.strokeRect(-camX, -camY, world.width, world.height);
  }

  // ---- 農地（Phase 0-G 種田起源）----
  // 玩家(權威座標 px,py)是否近到能照顧農地：鏡像伺服器的 within_field_reach，
  // 用快照帶來的 field.reach 當同一個來源，前後端對「多近才算」不會各說各話。
  function withinFieldReach(px, py) {
    if (!field) return false;
    const right = field.origin_x + field.cols * field.tile_size;
    const bottom = field.origin_y + field.rows * field.tile_size;
    const nx = Math.max(field.origin_x, Math.min(px, right));
    const ny = Math.max(field.origin_y, Math.min(py, bottom));
    const dx = px - nx, dy = py - ny;
    return dx * dx + dy * dy <= field.reach * field.reach;
  }

  // 依伺服器廣播的每格 state/dry 畫出耕地與作物階段。
  function drawField(camX, camY) {
    if (!field) return;
    const ts = field.tile_size;
    // 自己離農地太遠時把整塊地畫淡，並提示走近——讓伺服器「太遠就拒絕照顧」
    // 不再表現成「點了沒反應像壞掉」。沒有自己（如剛進場）就照常畫。
    const me = myId ? players.get(myId) : null;
    const reachable = me ? withinFieldReach(me.x, me.y) : true;
    const fx = field.origin_x - camX;
    const fy = field.origin_y - camY;
    const fw = field.cols * ts;
    const fh = field.rows * ts;

    ctx.save();
    if (!reachable) ctx.globalAlpha = 0.55;

    // 整塊田的「土底」墊一層深褐色,讓它跟草地一眼分得開(原本未翻土的格子
    // 顏色和草地太接近,玩家完全看不出腳下站著一塊田)。
    ctx.fillStyle = "#3a2818";
    ctx.fillRect(fx - 2, fy - 2, fw + 4, fh + 4);

    for (let row = 0; row < field.rows; row++) {
      for (let col = 0; col < field.cols; col++) {
        const cell = field.cells[row * field.cols + col];
        const sx = field.origin_x + col * ts - camX;
        const sy = field.origin_y + row * ts - camY;
        if (sx + ts < 0 || sy + ts < 0 || sx > canvas.width || sy > canvas.height) continue;
        drawTile(sx, sy, ts, cell);
      }
    }

    // 周圍畫一圈黃銅色邊框(對齊世界邊界的設計語彙),從遠處也看得到
    // 「那邊有一塊我的地」。
    ctx.strokeStyle = "#c9a24b";
    ctx.lineWidth = 3;
    ctx.strokeRect(fx - 2, fy - 2, fw + 4, fh + 4);

    // 田地名字標籤(平時也顯示,不只在太遠時)。
    ctx.fillStyle = "rgba(232,224,207,0.9)";
    ctx.font = "13px system-ui, sans-serif";
    ctx.textAlign = "center";
    ctx.fillText("你的乙太田 🌱", fx + fw / 2, fy - 8);

    ctx.restore();

    if (!reachable) {
      ctx.fillStyle = "rgba(232,224,207,0.85)";
      ctx.font = "12px system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.fillText("(走近一點才能照顧)", fx + fw / 2, fy + fh + 18);
    }
  }

  function drawTile(sx, sy, ts, cell) {
    // 底色:未翻土 = 暖土黃(像未開墾的乾土);翻好的 = 深咖啡(翻過的潮土)。
    // 兩者都跟草地(深綠)明顯不同,玩家一眼看得到「這裡是一塊田」。
    ctx.fillStyle = cell.state === 0 ? "#7a5f3c" : "#5b4636";
    ctx.fillRect(sx + 1, sy + 1, ts - 2, ts - 2);
    ctx.strokeStyle = "rgba(0,0,0,0.35)";
    ctx.lineWidth = 1;
    ctx.strokeRect(sx + 0.5, sy + 0.5, ts - 1, ts - 1);

    const cx = sx + ts / 2;
    const cy = sy + ts / 2;
    if (cell.state === 2) {
      // 種子：一顆小土點
      ctx.fillStyle = "#caa46a";
      ctx.beginPath(); ctx.arc(cx, cy, 3, 0, Math.PI * 2); ctx.fill();
    } else if (cell.state === 3) {
      // 發芽：綠莖 + 兩片小葉
      ctx.strokeStyle = "#7ec850"; ctx.lineWidth = 3;
      ctx.beginPath(); ctx.moveTo(cx, cy + 8); ctx.lineTo(cx, cy - 6); ctx.stroke();
      ctx.fillStyle = "#7ec850";
      ctx.beginPath(); ctx.arc(cx - 4, cy - 4, 3, 0, Math.PI * 2); ctx.fill();
      ctx.beginPath(); ctx.arc(cx + 4, cy - 6, 3, 0, Math.PI * 2); ctx.fill();
    } else if (cell.state === 4) {
      // 成熟乙太作物：莖 + 發光金果
      ctx.fillStyle = "#7ec850"; ctx.fillRect(cx - 1.5, cy - 4, 3, 12);
      ctx.shadowColor = "#ffe9a0"; ctx.shadowBlur = 10;
      ctx.fillStyle = "#ffd24a";
      ctx.beginPath(); ctx.arc(cx, cy - 6, 6, 0, Math.PI * 2); ctx.fill();
      ctx.shadowBlur = 0;
    }
    // 需澆水：藍色虛線框提示。
    if (cell.dry) {
      ctx.strokeStyle = "rgba(90,170,255,0.9)";
      ctx.lineWidth = 2;
      ctx.setLineDash([4, 3]);
      ctx.strokeRect(sx + 2, sy + 2, ts - 4, ts - 4);
      ctx.setLineDash([]);
    }
  }

  // 距離提示節流：太遠時只偶爾提醒一次，不洗聊天視窗。
  let lastReachHint = 0;
  // 點/輕觸地表某點 → 換算世界座標 → 送農地互動意圖（伺服器決定做什麼）。
  function farmAtScreen(clientX, clientY) {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    // 自己離農地太遠：伺服器一律拒絕，這裡先給回饋、不白送一則。
    const me = myId ? players.get(myId) : null;
    if (me && field && !withinFieldReach(me.x, me.y)) {
      const now = Date.now();
      if (now - lastReachHint > 2500) {
        addChat("系統", "走近農地才能照顧作物哦。");
        lastReachHint = now;
      }
      return;
    }
    const rect = canvas.getBoundingClientRect();
    const wx = clientX - rect.left + lastCam.x;
    const wy = clientY - rect.top + lastCam.y;
    ws.send(JSON.stringify({ type: "farm", x: wx, y: wy }));
  }
  // 桌面：滑鼠點擊即互動（移動走鍵盤，不衝突）。
  canvas.addEventListener("click", (e) => farmAtScreen(e.clientX, e.clientY));

  // ---- 聊天 ----
  function addChat(who, text) {
    const log = document.getElementById("chatLog");
    log.style.display = "block";
    const line = document.createElement("div");
    line.innerHTML = `<span class="who"></span>: <span class="msg"></span>`;
    line.querySelector(".who").textContent = who;
    line.querySelector(".msg").textContent = text;
    log.appendChild(line);
    log.scrollTop = log.scrollHeight;
  }

  document.getElementById("chatForm").addEventListener("submit", (e) => {
    e.preventDefault();
    const input = document.getElementById("chatText");
    const text = input.value.trim();
    if (text && ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "chat", text }));
    }
    input.value = "";
    input.blur();
  });

  // ---- 建議箱 ----
  const modal = document.getElementById("suggestModal");
  document.querySelector("#suggestBtn button").addEventListener("click", () => {
    modal.style.display = "flex";
    document.getElementById("suggestFrom").value = myName;
  });
  document.getElementById("suggestCancel").addEventListener("click", () => {
    modal.style.display = "none";
    document.getElementById("suggestStatus").textContent = "";
  });
  document.getElementById("suggestSend").addEventListener("click", async () => {
    const text = document.getElementById("suggestText").value.trim();
    const from = document.getElementById("suggestFrom").value.trim();
    const status = document.getElementById("suggestStatus");
    if (!text) { status.textContent = "寫點東西再送吧。"; return; }
    status.textContent = "送出中…";
    try {
      const res = await fetch("/api/suggestions", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ from, text }),
      });
      if (res.ok) {
        status.textContent = "收到了，謝謝你！🙏";
        document.getElementById("suggestText").value = "";
        setTimeout(() => { modal.style.display = "none"; status.textContent = ""; }, 1200);
      } else {
        status.textContent = "送出失敗，稍後再試。";
      }
    } catch {
      status.textContent = "送出失敗，稍後再試。";
    }
  });

  // ---- 進場流程 ----
  function enterGame() {
    document.getElementById("login").classList.add("hidden");
    for (const id of ["hud", "suggestBtn", "chat"]) {
      document.getElementById(id).classList.remove("hidden");
    }
    requestAnimationFrame(render);
  }

  // 在這台裝置上記住名字與種族,refresh 不用重打(訪客流程才用)
  try {
    const savedName = localStorage.getItem("butfun.name");
    if (savedName) document.getElementById("nameInput").value = savedName;
    const savedSpecies = localStorage.getItem("butfun.species");
    if (savedSpecies) {
      const sel = document.getElementById("speciesInput");
      if ([...sel.options].some((o) => o.value === savedSpecies && !o.disabled)) {
        sel.value = savedSpecies;
      }
    }
  } catch {}

  document.getElementById("joinBtn").addEventListener("click", () => {
    const name = document.getElementById("nameInput").value.trim() || "拓荒者";
    const species = document.getElementById("speciesInput").value;
    try {
      localStorage.setItem("butfun.name", name);
      localStorage.setItem("butfun.species", species);
    } catch {}
    connect(name, species);
  });
  document.getElementById("nameInput").addEventListener("keydown", (e) => {
    if (e.key === "Enter") document.getElementById("joinBtn").click();
  });

  // 開頁就查 /auth/me:已登入就跳過進場畫面、直接連線(同一帳號跨裝置同一玩家)
  fetch("/auth/me", { credentials: "same-origin" })
    .then((r) => (r.ok ? r.json() : null))
    .then((me) => {
      if (!me) return; // 訪客流程,維持顯示登入畫面
      // 顯示登入狀態 + 一鍵登出
      const hud = document.getElementById("hud");
      const tag = document.createElement("div");
      tag.style.opacity = "0.7";
      tag.innerHTML = `已登入：<b></b> · <a href="#" id="logoutLink" style="color:#c9a24b">登出</a>`;
      tag.querySelector("b").textContent = me.name;
      hud.appendChild(tag);
      tag.querySelector("#logoutLink").addEventListener("click", async (e) => {
        e.preventDefault();
        await fetch("/auth/logout", { method: "POST", credentials: "same-origin" });
        location.reload();
      });
      // 已登入 → 伺服器會用 cookie 拿到 user_id、忽略 name/species,但還是送個空 Join
      // 確保走原本的訊息流程。
      connect(me.name, me.species || "terran");
    })
    .catch(() => {});
})();
