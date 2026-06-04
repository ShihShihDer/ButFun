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
    // 聊天輸入中不攔截移動鍵
    if (document.activeElement && document.activeElement.tagName === "INPUT" &&
        document.activeElement.id !== "chatText") return;
    if (document.activeElement && document.activeElement.id === "chatText") return;
    if (e.key === "Enter") { document.getElementById("chatText").focus(); return; }
    const dir = keyToDir(e);
    if (dir) { keys[dir] = true; sendInputIfChanged(); e.preventDefault(); }
  });
  window.addEventListener("keyup", (e) => {
    const dir = keyToDir(e);
    if (dir) { keys[dir] = false; sendInputIfChanged(); }
  });

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

    drawGround(camX, camY);

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

  document.getElementById("joinBtn").addEventListener("click", () => {
    const name = document.getElementById("nameInput").value.trim() || "拓荒者";
    const species = document.getElementById("speciesInput").value;
    connect(name, species);
  });
  document.getElementById("nameInput").addEventListener("keydown", (e) => {
    if (e.key === "Enter") document.getElementById("joinBtn").click();
  });
})();
