// ButFun 前端 — Phase 0 垂直切片
// 連上權威伺服器，送移動意圖，渲染所有玩家的權威狀態快照。
// 刻意用原生 canvas，零外部相依；之後要做 tilemap 再換 Phaser。

(() => {
  "use strict";

  const canvas = document.getElementById("game");
  const ctx = canvas.getContext("2d");

  // ---- 像素素材(peregrine-assets)。載好才用 sprite,沒載好/載失敗都退回原本的
  //      程式繪製,確保遊戲一定能玩。規格見 docs/ASSET_INTEGRATION.md。----
  const TS = 32;
  const ART = {};
  let artReady = false;
  (function loadArt() {
    const names = ["tileset_a", "field", "player", "tree", "rock", "ship", "workshop", "fence"];
    let remaining = names.length;
    for (const n of names) {
      const img = new Image();
      const done = () => { if (--remaining === 0) artReady = true; };
      img.onload = done;
      img.onerror = done; // 缺一張也不卡住整體
      img.src = `assets/${n}.png?v=1`;
      ART[n] = img;
    }
  })();
  const artOk = (n) => artReady && ART[n] && ART[n].complete && ART[n].naturalWidth > 0;
  // 朝向弧度 → player.png 的列(0 下 / 1 左 / 2 右 / 3 上)。
  function facingToDir(rad) {
    const deg = (rad * 180) / Math.PI; // -180..180
    if (deg >= -45 && deg < 45) return 2;   // 右
    if (deg >= 45 && deg < 135) return 0;   // 下
    if (deg >= -135 && deg < -45) return 3; // 上
    return 1;                                // 左
  }

  // ---- 無障礙：尊重系統「減少動態」偏好 ----
  // 有些玩家對持續飄動／彈跳／閃爍的畫面會暈或不適（前庭敏感）。作業系統提供
  // prefers-reduced-motion 偏好，這裡即時讀取：開啟時關掉「純裝飾、不傳遞資訊」的環境
  // 動態——角色踏步彈跳、夜晚乙太微光的飄移與明滅、重連橫幅的脈動。靜態的畫面照樣資訊
  // 完整（夜色濃淡、微光位置、重連橫幅本身都還在），只是不再動。純表現層、不嵌遊戲規則。
  const reduceMotionMQ = window.matchMedia
    ? window.matchMedia("(prefers-reduced-motion: reduce)") : null;
  let reduceMotion = !!(reduceMotionMQ && reduceMotionMQ.matches);
  if (reduceMotionMQ) {
    const onRM = (e) => { reduceMotion = e.matches; };
    // 舊瀏覽器只有 addListener;兩種都掛以求相容。
    if (reduceMotionMQ.addEventListener) reduceMotionMQ.addEventListener("change", onRM);
    else if (reduceMotionMQ.addListener) reduceMotionMQ.addListener(onRM);
  }

  // ---- 狀態 ----
  let ws = null;
  let myId = null;
  let world = { width: 2000, height: 2000 };
  let myName = "";
  // 已登入者另有「已登入：X · 登出」標籤、不必重複;只有訪客需要在 HUD 看到自己的代號。
  let isGuest = true;
  // id -> { name, species, x, y (目標), rx, ry (渲染中插值位置) }
  const players = new Map();
  const keys = { up: false, down: false, left: false, right: false };
  let lastSentInput = "";
  // 伺服器廣播的農地狀態（含每格 state / dry）；進場前為 null。
  let field = null;
  // 最近一次快照數到的「有作物且缺水」格數（updateFarmHud 算好順手記下）；
  // 給離田時的「回農地」邊緣指標決定要不要強調缺水，數字與 HUD 一致、不另外再數一遍。
  let farmDryCount = 0;
  let myEther = 0;
  // 是否已同步過初始乙太：避免進場／重連時把既有存量當成一次大量「獲得」而噴一大串飄字。
  let etherKnown = false;
  // 收成得乙太時的「+N 乙太」飄字（純表現，從權威 ether 差值推得，不嵌任何遊戲規則）。
  // 每筆 { wx, wy, gain, born }：以世界座標固定在收成當下的玩家位置上方，隨時間上飄淡出。
  const etherFloaters = [];
  // 互動確認漣漪（純表現）：點/輕點田格送出農作意圖時，在該格畫一圈短暫擴張淡出的亮環，
  // 讓玩家「按下就有回饋」——尤其手機沒有桌面的 hover 高亮,輕點後到下一個快照回來前
  // 全無反饋會覺得沒點到。每筆 { wx, wy, born }（世界座標,鏡頭移動也黏在原格）。不嵌任何
  // 遊戲規則:做不做得成仍由權威伺服器決定,這裡只確認「這一下送出去了」。
  const tapFlashes = [];
  // 伺服器廣播的日夜狀態 { phase, light }；進場前為 null（render 時當白天、不疊夜色）。
  let daynight = null;
  // 是否已進場（已揭開 HUD 並啟動 render 迴圈）。自動重連時 welcome 會再來一次，
  // 用它擋住重複初始化／重啟第二個 render 迴圈。
  let started = false;

  // ---- 觸控搖桿狀態(手機沒鍵盤,用拖曳設方向) ----
  let touchOrigin = null;   // 手指按下的初始位置
  let touchCurrent = null;  // 手指目前的位置
  // 最近一次 render 用的鏡頭左上角（世界座標），給點擊換算用。
  const lastCam = { x: 0, y: 0 };
  // 滑鼠在畫面上的位置（螢幕座標），用來在桌面高亮「游標所指的田格」做操作回饋。
  // 純表現:手機沒有 hover、靠輕點即時互動,觸控時不更新它,自然不畫高亮。
  let hoverScreen = null;

  // ---- 畫布尺寸 ----
  // viewW/viewH 是「邏輯像素」的視窗尺寸,所有繪製碼一律用這兩個值(鏡頭置中、視野
  // 裁切、小地圖定位…),不直接讀 canvas.width/height——那是放大後的實體像素緩衝。
  let viewW = window.innerWidth;
  let viewH = window.innerHeight;
  let dpr = 1; // 裝置像素比,resize 時更新
  function resize() {
    dpr = window.devicePixelRatio || 1;
    viewW = window.innerWidth;
    viewH = window.innerHeight;
    // 背景緩衝放大成裝置實體像素、CSS 尺寸維持邏輯像素,成像在 retina／手機高解析
    // 螢幕上不再被瀏覽器整張放大糊掉(此前 canvas.width=邏輯像素,DPR>1 時被拉伸)。
    // 繪圖座標系以 dpr 縮放,讓所有繪製碼照舊用邏輯像素——成像更銳利,純客戶端品質,
    // 不碰任何遊戲規則(將來 WebXR renderer 連同一後端可各自實作)。
    canvas.width = Math.round(viewW * dpr);
    canvas.height = Math.round(viewH * dpr);
    canvas.style.width = viewW + "px";
    canvas.style.height = viewH + "px";
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }
  window.addEventListener("resize", resize);
  resize();

  // ---- 連線（含斷線自動重連）----
  // 手機網路一抖／分頁切背景／伺服器換版重啟都會斷 WS。對「瀏覽器即玩的療癒多人世界」
  // 來說，要玩家自己重新整理頁面太傷——這裡斷線後以指數退避自動重連，回來就接著玩。
  // 伺服器本就支援重新 join（登入者靠 cookie 認回同一玩家、訪客用記住的名字／物種），
  // 此層純屬客戶端韌性，不碰任何遊戲規則（將來 WebXR renderer 連同一後端時可各自實作）。
  let reconnectTimer = null;
  let reconnectAttempts = 0;
  let lastSpecies = "terran";
  let announcedDrop = false; // 斷線提示只報一次，重連風暴不洗聊天視窗

  function connect(name, species) {
    myName = name;
    lastSpecies = species;
    // 每條新連線都重置「屬於這條連線」的輸入同步狀態：新連線伺服器不知道我們按著什麼，
    // 清掉移動鍵並把 lastSentInput 清空，下次 sendInputIfChanged 會重新把意圖送給新連線。
    lastSentInput = "";
    keys.up = keys.down = keys.left = keys.right = false;
    const proto = location.protocol === "https:" ? "wss" : "ws";
    ws = new WebSocket(`${proto}://${location.host}/ws`);

    ws.onopen = () => {
      reconnectAttempts = 0; // 連上就重置退避
      ws.send(JSON.stringify({ type: "join", name, species }));
    };

    ws.onmessage = (ev) => {
      let msg;
      try { msg = JSON.parse(ev.data); } catch { return; }
      handleServerMsg(msg);
    };

    ws.onclose = () => {
      // 還沒進場就斷（如初次連線就被拒）也照樣重連退避，不卡死在登入後空畫面。
      if (started && !announcedDrop) {
        addChat("系統", "與伺服器的連線中斷了，正在自動重新連線…");
        announce("與伺服器的連線中斷了，正在自動重新連線…"); // 同步播給報讀器
        announcedDrop = true;
      }
      // 進場後才顯示持續橫幅（登入畫面自己會處理初次連線，不需要橫幅）。
      if (started) showConnStatus();
      scheduleReconnect();
    };
    ws.onerror = () => { try { ws.close(); } catch {} }; // 統一走 onclose 的重連路徑
  }

  // 指數退避重連：0.5s、1s、2s…上限 8s。低頻、足以撐過短暫斷網又不狂打伺服器。
  // 一次只排一個 timer，避免多次 onclose／onerror 疊出重連風暴。
  // 重連橫幅:斷線到接回之間持續顯示,給玩家「正在重連、別急著重整」的回饋。
  // 純客戶端韌性 UI——不碰任何遊戲規則,將來 WebXR renderer 可各自實作。
  function showConnStatus() {
    const el = document.getElementById("connStatus");
    if (el) el.classList.remove("hidden");
  }
  function hideConnStatus() {
    const el = document.getElementById("connStatus");
    if (el) el.classList.add("hidden");
  }
  // 螢幕報讀器播報:把重要狀態切換寫進視覺隱藏的 aria-live 區,讓看不到畫面的玩家
  // 也即時知道(脈動橫幅/飄字對他們無效)。只在「狀態真的變了」時呼叫,避免重複朗讀。
  function announce(text) {
    const el = document.getElementById("srStatus");
    if (el) el.textContent = text;
  }

  function scheduleReconnect() {
    if (reconnectTimer !== null) return;
    const delay = Math.min(8000, 500 * Math.pow(2, reconnectAttempts));
    reconnectAttempts++;
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      connect(myName, lastSpecies);
    }, delay);
  }

  function handleServerMsg(msg) {
    switch (msg.type) {
      case "welcome":
        myId = msg.id;
        world = msg.world;
        // 重連成功：先前報過斷線就回報已接回，並把乙太基準重置——否則重連後第一份快照
        // 會把「持久化存量」當成一次大獲得、噴一串飄字。
        if (announcedDrop) {
          addChat("系統", "已重新連上，繼續吧。");
          announce("已重新連上，繼續吧。"); // 同步播給報讀器
          announcedDrop = false;
          etherKnown = false;
        }
        hideConnStatus(); // 接回（或初次連上）就收掉重連橫幅
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
        updateFarmHud(field);
        const me = msg.players.find((p) => p.id === myId);
        if (me) {
          // 乙太變多 → 收成回饋飄字（首次同步不噴，否則進場/重連會把存量當成一次大獲得）。
          if (etherKnown && me.ether > myEther) {
            spawnEtherFloater(me.ether - myEther, me.x, me.y);
          }
          myEther = me.ether;
          etherKnown = true;
          document.getElementById("hudEther").textContent = `乙太：${myEther}`;
          // 訪客在 HUD 看到自己的遊戲代號——進場後才知道自己叫什麼,也確認顯示的是代號非真名。
          if (isGuest) {
            const nameEl = document.getElementById("hudName");
            nameEl.textContent = `你：${me.name}`;
            nameEl.classList.remove("hidden");
          }
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

  // 收成得乙太時，在玩家當下位置上方記一筆飄字（世界座標，鏡頭移動也黏在原地飄起）。
  function spawnEtherFloater(gain, wx, wy) {
    etherFloaters.push({ wx, wy: wy - 22, gain, born: performance.now() });
  }

  // 把飄字逐一上飄、淡出，過了壽命就移除。畫在日夜染色之後（當回饋 HUD，不被夜色蓋暗）。
  const FLOAT_MS = 1100;
  function drawEtherFloaters(camX, camY, now) {
    for (let i = etherFloaters.length - 1; i >= 0; i--) {
      const f = etherFloaters[i];
      const age = now - f.born;
      if (age >= FLOAT_MS) { etherFloaters.splice(i, 1); continue; }
      const t = age / FLOAT_MS;
      const alpha = 1 - t;
      const sx = f.wx - camX;
      const sy = f.wy - camY - t * 34; // 隨時間往上飄
      ctx.font = "bold 15px system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.fillStyle = `rgba(0,0,0,${(alpha * 0.5).toFixed(3)})`;
      ctx.fillText(`+${f.gain} 乙太 ✨`, sx + 1, sy + 1); // 描影,任何地表上都讀得到
      ctx.fillStyle = `rgba(255,210,74,${alpha.toFixed(3)})`;
      ctx.fillText(`+${f.gain} 乙太 ✨`, sx, sy);
    }
  }

  // 送出農作意圖時記一筆確認漣漪。落在田格內就吸附到該格中心(漣漪剛好框住整格);
  // 否則(腳下沒田格的邊角點)就用原始點。座標為世界座標,鏡頭移動也黏在原處。
  function spawnTapFlash(wx, wy) {
    let cx = wx, cy = wy;
    if (field) {
      const ts = field.tile_size;
      const col = Math.floor((wx - field.origin_x) / ts);
      const row = Math.floor((wy - field.origin_y) / ts);
      if (col >= 0 && row >= 0 && col < field.cols && row < field.rows) {
        cx = field.origin_x + (col + 0.5) * ts;
        cy = field.origin_y + (row + 0.5) * ts;
      }
    }
    tapFlashes.push({ wx: cx, wy: cy, born: performance.now() });
  }

  // 漣漪：由小擴張到約一格大、同時淡出。畫在日夜染色之後（當回饋 HUD,不被夜色蓋暗）。
  const TAP_MS = 360;
  function drawTapFlashes(camX, camY, now) {
    const ts = field ? field.tile_size : 24;
    for (let i = tapFlashes.length - 1; i >= 0; i--) {
      const f = tapFlashes[i];
      const age = now - f.born;
      if (age >= TAP_MS) { tapFlashes.splice(i, 1); continue; }
      const t = age / TAP_MS;
      const sx = f.wx - camX;
      const sy = f.wy - camY;
      const r = ts * (0.28 + t * 0.42); // 由小擴張到約半格半徑
      ctx.save();
      ctx.lineWidth = 2.5 * (1 - t * 0.5);
      ctx.strokeStyle = `rgba(255,210,74,${(0.85 * (1 - t)).toFixed(3)})`;
      ctx.beginPath();
      ctx.arc(sx, sy, r, 0, Math.PI * 2);
      ctx.stroke();
      ctx.restore();
    }
  }

  // 日夜染色（純表現，色相由權威 phase、濃度由權威 light 推得，不嵌任何遊戲規則；
  // 將來 WebXR renderer 自有環境光，這層只屬 2D 客戶端）：
  //  ① 夜色：亮度越低疊越濃的冷藍，午夜最濃但仍微光不全黑（療癒、非全黑）。
  //  ② 金色時刻：破曉／黃昏疊一層暖橘，呼應 HUD 的 🌅🌇。強度取三角函數——破曉與黃昏
  //     兩段亮度都落在 [0.421, 0.781]（伺服器 phase_for/light_for 推得），故把暖光峰設在
  //     中點 0.6、兩端 0.18 處歸零：每個相位邊界都正好在亮度極端、暖光已近 0，相位切換
  //     不會有色相突跳（金光在破曉／黃昏的正中最濃，自然地淡入淡出）。
  function drawDayNightTint() {
    if (!daynight) return;
    const light = daynight.light;
    const dark = Math.max(0, Math.min(1, 1 - light));
    if (dark > 0.001) {
      ctx.fillStyle = `rgba(14,20,52,${(dark * 0.72).toFixed(3)})`;
      ctx.fillRect(0, 0, viewW, viewH);
    }
    if (daynight.phase === "dawn" || daynight.phase === "dusk") {
      const warm = Math.max(0, 1 - Math.abs(light - 0.6) / 0.18);
      if (warm > 0.001) {
        ctx.fillStyle = `rgba(255,150,60,${(warm * 0.18).toFixed(3)})`;
        ctx.fillRect(0, 0, viewW, viewH);
      }
    }
  }

  // ---- 夜晚漂浮的乙太微光（純表現）----
  // 日夜系統目前只把夜晚畫暗,抵達夜晚沒有任何視覺「回饋」。配合登入畫面的世界觀
  //「大靜默之後,乙太緩緩回流」,在夜色濃時讓畫面浮起一層緩緩飄動、微微明滅的金色
  // 乙太微光——讓已建好的日夜循環在夜裡有療癒感的呈現,而非只是變暗。亮度越低（越
  // 接近午夜）浮現越明顯,白天完全不畫。座標走螢幕比例、自有極緩漂移（當作飄在鏡頭
  // 與地表之間的空中微塵,刻意不跟地面捲動,resize 也不會跑掉）,不嵌任何遊戲規則
  //（將來 WebXR renderer 自有環境呈現,這層只屬 2D 客戶端）。
  const NIGHT_MOTES = [];
  function initNightMotes() {
    if (NIGHT_MOTES.length) return;
    for (let i = 0; i < 30; i++) {
      NIGHT_MOTES.push({
        x: Math.random(),                    // 螢幕寬的比例 [0,1)
        y: Math.random(),                    // 螢幕高的比例 [0,1)
        vx: (Math.random() - 0.5) * 0.006,   // 每秒漂移（比例/秒）,極緩、左右隨機
        vy: -0.004 - Math.random() * 0.006,  // 緩緩上飄
        r: 1.2 + Math.random() * 1.8,        // 半徑（邏輯像素）
        tw: Math.random() * Math.PI * 2,     // 明滅相位
        tws: 0.8 + Math.random() * 1.2,      // 明滅速度
      });
    }
  }
  let lastMoteT = 0;
  function drawNightMotes(now) {
    if (!daynight) return;
    const dark = Math.max(0, Math.min(1, 1 - daynight.light));
    // 白天／接近白天不畫;同步推進 lastMoteT,讓再次入夜時從當下續飄、不因停畫期間
    // 累積出一大段 dt 造成瞬移。
    if (dark < 0.12) { lastMoteT = now; return; }
    initNightMotes();
    let dt = (now - lastMoteT) / 1000;
    lastMoteT = now;
    if (!(dt > 0) || dt > 0.1) dt = 0.016; // 首幀／分頁切回的大跳用固定步,避免瞬移
    ctx.save();
    for (const m of NIGHT_MOTES) {
      // 開「減少動態」時：微光留在原位、不飄不明滅，只剩一層靜態柔光點綴夜色。
      if (!reduceMotion) {
        m.x += m.vx * dt;
        m.y += m.vy * dt;
        // 飄出邊界就從另一側繞回（無限飄,畫面永遠均勻散著微光）。
        if (m.x < 0) m.x += 1; else if (m.x >= 1) m.x -= 1;
        if (m.y < 0) m.y += 1; else if (m.y >= 1) m.y -= 1;
        m.tw += m.tws * dt;
      }
      const twinkle = reduceMotion ? 0.8 : 0.55 + 0.45 * Math.sin(m.tw); // 0.1..1 之間明滅
      const alpha = dark * twinkle * 0.5;           // 夜越濃越亮,最濃也僅半透不刺眼
      if (alpha < 0.02) continue;
      const px = m.x * viewW;
      const py = m.y * viewH;
      // 柔光暈（中心金、外緣透明）讓微光看起來像漂浮的乙太,不是硬邊圓點。
      const g = ctx.createRadialGradient(px, py, 0, px, py, m.r * 3);
      g.addColorStop(0, `rgba(255,224,150,${alpha.toFixed(3)})`);
      g.addColorStop(1, "rgba(255,224,150,0)");
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.arc(px, py, m.r * 3, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.restore();
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

  // 農地缺水提醒：數出快照裡「有作物且缺水」的格數，顯示在 HUD，讓玩家離開田去
  // 探索時也知道作物渴了該回去澆水。缺水格的判定刻意對齊 drawTile 畫藍點的條件
  //（state 2~4 且 dry），HUD 數字與看得到的提示點一致；沒有缺水格時隱藏整行。
  // 純從權威快照數得的表現層回饋，不嵌任何遊戲規則（將來 WebXR renderer 可各自實作）。
  function updateFarmHud(f) {
    const el = document.getElementById("hudFarm");
    if (!el) return;
    let dry = 0;
    if (f && f.cells) {
      for (const cell of f.cells) {
        if (cell.dry && cell.state >= 2 && cell.state <= 4) dry++;
      }
    }
    farmDryCount = dry;
    if (dry > 0) {
      el.textContent = `🌱 ${dry} 格作物缺水`;
      el.classList.remove("hidden");
    } else {
      el.classList.add("hidden");
    }
  }

  // ---- 離田時的「回農地」邊緣指標 ----
  // 農地完全移出畫面時，在螢幕邊緣畫一個指向農地的小箭頭，讓玩家走遠探索後一眼知道
  // 「我的田在哪個方向」、要回去澆水時不必先開小地圖對位。農地在畫面內時不畫（不打擾）。
  // 純從農地世界座標 + 鏡頭推得的表現層回饋，不嵌任何遊戲規則（WebXR renderer 可各自實作）。
  function drawFarmPointer(camX, camY) {
    if (!field || !field.cols || !field.rows) return;
    // 農地中心（世界座標）→ 螢幕座標。
    const cx = field.origin_x + (field.cols * field.tile_size) / 2;
    const cy = field.origin_y + (field.rows * field.tile_size) / 2;
    const sx = cx - camX;
    const sy = cy - camY;
    // 農地矩形是否與畫面相交：相交（看得到田）就不畫指標。用半尺寸當判定半徑近似即可。
    const halfW = (field.cols * field.tile_size) / 2;
    const halfH = (field.rows * field.tile_size) / 2;
    if (sx + halfW >= 0 && sx - halfW <= viewW &&
        sy + halfH >= 0 && sy - halfH <= viewH) return;

    // 從畫面中心朝農地方向、夾到邊緣內側的安全框上，當作箭頭落點。
    const ccx = viewW / 2, ccy = viewH / 2;
    const ang = Math.atan2(sy - ccy, sx - ccx);
    const m = 46; // 邊緣留白，避開 HUD / 小地圖最外圈
    const px = Math.max(m, Math.min(viewW - m, sx));
    const py = Math.max(m, Math.min(viewH - m, sy));

    const dry = farmDryCount > 0;
    // 缺水時用田格藍色澆水語彙 + 顯示格數，催玩家回去澆水；不缺水用低調黃銅。
    const color = dry ? "#7fbfff" : "rgba(201,162,75,0.85)";

    ctx.save();
    // 底盤圓，讓箭頭在任何地表都讀得到。
    ctx.beginPath();
    ctx.arc(px, py, 15, 0, Math.PI * 2);
    ctx.fillStyle = "rgba(10,16,30,0.6)";
    ctx.fill();
    // 三角箭頭，尖端指向農地方向。
    ctx.translate(px, py);
    ctx.rotate(ang);
    ctx.beginPath();
    ctx.moveTo(9, 0);
    ctx.lineTo(-5, -6);
    ctx.lineTo(-5, 6);
    ctx.closePath();
    ctx.fillStyle = color;
    ctx.fill();
    ctx.restore();

    // 圖示（與 HUD 缺水提示同一顆 🌱）+ 缺水格數，貼在箭頭旁。
    ctx.font = "12px system-ui, sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    const label = dry ? `🌱${farmDryCount}` : "🌱";
    // 標籤擺在箭頭「背向農地」那側，不擋住指向。
    const lx = px - Math.cos(ang) * 24;
    const ly = py - Math.sin(ang) * 24;
    ctx.lineJoin = "round";
    ctx.lineWidth = 3;
    ctx.strokeStyle = "rgba(0,0,0,0.55)";
    ctx.strokeText(label, lx, ly);
    ctx.fillStyle = dry ? "#bfe0ff" : "#e8e0cf";
    ctx.fillText(label, lx, ly);
    ctx.textBaseline = "alphabetic"; // 復原預設，免得影響其後文字繪製
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
    if (e.key === "Enter") {
      // 開聊天打字＝玩家從操控移動切換到打字。比照失焦/切背景的修復家族：先放開所有
      // 移動鍵並送出「停止」，免得按著方向鍵開聊天時，角色在你打字的整段時間持續亂走
      //（keydown 守衛此時已不再更新 keys，物理按著的鍵會卡在 true 直到放開）。
      releaseAllKeys();
      document.getElementById("chatText").focus();
      return;
    }
    const dir = keyToDir(e);
    if (dir) { keys[dir] = true; sendInputIfChanged(); e.preventDefault(); return; }
    // 採集鍵:空白鍵 / E / F 對腳下田格互動,讓沒滑鼠的玩家也能農作。
    // 用 e.repeat 擋住長按連發(一次按一次,跟滑鼠單擊一致)。
    if (e.key === " " || e.key === "e" || e.key === "E" || e.key === "f" || e.key === "F") {
      if (!e.repeat) farmAtPlayer();
      e.preventDefault();
    }
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
    // 每幀重設基準變換(dpr 縮放),確保前一幀任何 save/restore 失衡也不會累積偏移。
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.imageSmoothingEnabled = false; // 像素風禁止插值放大,否則糊邊
    ctx.clearRect(0, 0, viewW, viewH);

    const me = myId ? players.get(myId) : null;
    // 插值所有玩家位置，讓 15Hz 快照看起來平滑；
    // 順手從位移量推出「朝向」與「走路相位」，給角色一點走動感（無美術素材的程式替代）。
    for (const p of players.values()) {
      const ddx = p.x - p.rx;
      const ddy = p.y - p.ry;
      const speed = Math.hypot(ddx, ddy);
      p.moving = speed > 0.6;
      if (p.moving) {
        p.facing = Math.atan2(ddy, ddx); // 朝移動方向（弧度）
        p.walk = (p.walk || 0) + Math.min(speed, 12) * 0.06; // 越快踏步越快
      }
      if (p.facing === undefined) p.facing = Math.PI / 2; // 預設面向下方
      p.rx += ddx * 0.3;
      p.ry += ddy * 0.3;
    }

    // 鏡頭跟隨自己
    const camX = me ? me.rx - viewW / 2 : world.width / 2 - viewW / 2;
    const camY = me ? me.ry - viewH / 2 : world.height / 2 - viewH / 2;
    lastCam.x = camX;
    lastCam.y = camY;

    drawGround(camX, camY);
    drawField(camX, camY);

    // 畫玩家
    for (const p of players.values()) {
      const sx = p.rx - camX;
      const sy = p.ry - camY;
      const isMe = p.id === myId;
      // 走路時上下彈跳一點，腳下陰影固定不跟著跳 → 讀起來像在踏步走動。
      // 開「減少動態」時不彈跳（避免持續上下晃造成不適），sprite 仍逐格切換不受影響。
      const bob = (p.moving && !reduceMotion) ? Math.abs(Math.sin(p.walk)) * 3 : 0;
      const by = sy - bob;

      // 腳下陰影（固定在地面，賣出彈跳的踏地感）
      ctx.beginPath();
      ctx.ellipse(sx, sy + 12, 11, 4, 0, 0, Math.PI * 2);
      ctx.fillStyle = "rgba(0,0,0,0.22)";
      ctx.fill();

      if (artOk("player")) {
        // 像素角色:列=朝向(0下1左2右3上)、欄=走路影格(0-3);靜止用第 0 格。
        const dir = facingToDir(p.facing);
        const frame = p.moving ? (Math.floor(p.walk) % 4) : 0;
        // sprite 32x32,放大成 36 比較好看;腳對齊 sy(陰影位置)。
        const dw = 36, dh = 36;
        ctx.drawImage(
          ART.player, frame * TS, dir * TS, TS, TS,
          Math.round(sx - dw / 2), Math.round(by - dh + 14), dw, dh
        );
      } else {
        // fallback:程式畫的圓 + 朝向小護目鏡點
        ctx.beginPath();
        ctx.arc(sx, by, 14, 0, Math.PI * 2);
        ctx.fillStyle = isMe ? "#c9a24b" : "#6fa8dc";
        ctx.fill();
        ctx.lineWidth = 2;
        ctx.strokeStyle = "rgba(0,0,0,0.4)";
        ctx.stroke();
        const fx = sx + Math.cos(p.facing) * 8;
        const fy = by + Math.sin(p.facing) * 8;
        ctx.beginPath();
        ctx.arc(fx, fy, 4, 0, Math.PI * 2);
        ctx.fillStyle = isMe ? "#3a2818" : "#23415c";
        ctx.fill();
      }

      // 自己的名字描金,讓玩家一眼找到自己。先描一圈深色外框再填字——白天的亮草地
      // 紋理上米白字會糊掉(飄字/小地圖都有襯底,唯獨頭上名字沒有),描邊讓名字在任何
      // 地表、任何日夜亮度下都讀得清。lineJoin=round 讓尖角不溢出成毛刺。
      ctx.font = "13px system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.lineJoin = "round";
      ctx.lineWidth = 3;
      ctx.strokeStyle = "rgba(0,0,0,0.55)";
      ctx.strokeText(p.name, sx, sy - 24);
      ctx.fillStyle = isMe ? "#ffd24a" : "#e8e0cf";
      ctx.fillText(p.name, sx, sy - 24);
    }

    // 日夜染色（疊在世界與玩家上，但在觸控搖桿與 HUD/小地圖之前）。
    drawDayNightTint();

    // 夜晚漂浮的乙太微光：在日夜染色「之後」畫（浮在變暗的世界上），但在飄字／漣漪／
    // 小地圖／HUD「之前」（那些互動回饋與 HUD 仍蓋在最上層、不被微光干擾）。
    drawNightMotes(performance.now());

    // 收成乙太飄字：在日夜染色「之後」畫，當回饋 HUD 不被夜色蓋暗。
    drawEtherFloaters(camX, camY, performance.now());

    // 互動確認漣漪：同在日夜染色之後畫，點/輕點田格的當下回饋不被夜色蓋暗。
    drawTapFlashes(camX, camY, performance.now());

    // 離田時的「回農地」邊緣指標：同在日夜染色之後畫，當 HUD 不被夜色蓋暗。
    drawFarmPointer(camX, camY);

    // 小地圖（右下角縮圖）：在日夜染色「之後」畫，當作 HUD 不被夜色蓋暗。
    drawMinimap();

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

  // ---- 小地圖（玩家建議：2000x2000 大世界容易迷路）----
  // 右下角畫一張固定大小的世界縮圖：世界邊界、農地位置、自己（亮點）、其他玩家（暗點）。
  // 純螢幕座標、每幀依最新快照重畫，不參與鏡頭換算。
  const MM = { maxSize: 150, minSize: 96, margin: 16, pad: 6 };
  // 小地圖邊長依畫面自適應:手機直式窄螢幕縮小(別吃掉太多空間、也少跟左下聊天框
  // 在底部重疊),平板/桌面維持上限。取畫面短邊的一個比例,夾在 min/max 之間。
  function minimapSize() {
    const shorter = Math.min(viewW, viewH);
    return Math.round(Math.max(MM.minSize, Math.min(MM.maxSize, shorter * 0.26)));
  }
  function drawMinimap() {
    if (!world || !world.width || !world.height) return;
    const w = world.width, h = world.height;
    // 等比縮到 size 方框內，長寬各自映射（世界目前是正方，但不假設）。
    const size = minimapSize();
    const scale = size / Math.max(w, h);
    const mw = w * scale, mh = h * scale;
    const ox = viewW - MM.margin - mw;   // 縮圖內容左上角（螢幕座標）
    const oy = viewH - MM.margin - mh;
    const clampUnit = (v, hi) => Math.max(0, Math.min(v, hi));

    // 半透明深底面板（對齊夜色色調），讓縮圖在任何地表上都讀得到。
    ctx.fillStyle = "rgba(10,16,30,0.55)";
    ctx.fillRect(ox - MM.pad, oy - MM.pad, mw + MM.pad * 2, mh + MM.pad * 2);

    // 農地位置（黃銅外框前先畫，免得被框線蓋住）。
    if (field) {
      const fx = ox + clampUnit(field.origin_x, w) * scale;
      const fy = oy + clampUnit(field.origin_y, h) * scale;
      const fw = field.cols * field.tile_size * scale;
      const fh = field.rows * field.tile_size * scale;
      ctx.fillStyle = "rgba(123,80,40,0.95)";
      ctx.fillRect(fx, fy, Math.max(3, fw), Math.max(3, fh));
    }

    // 世界邊界（沿用世界邊框的黃銅設計語彙）。
    ctx.strokeStyle = "rgba(201,162,75,0.7)";
    ctx.lineWidth = 2;
    ctx.strokeRect(ox, oy, mw, mh);

    // 目前畫面看得到的範圍（鏡頭視野框）：在 2000x2000 大世界裡，光看自己的點還
    // 不知道「這一眼看到多大一塊」。用最近一次 render 的鏡頭左上角 lastCam + 畫布
    // 尺寸推出可見世界矩形（夾在世界界內），畫成細白框。純表現、純從鏡頭狀態推得，
    // 不嵌任何遊戲規則。畫在玩家點之前，讓玩家點疊在最上層仍醒目。
    const vx0 = clampUnit(lastCam.x, w);
    const vy0 = clampUnit(lastCam.y, h);
    const vx1 = clampUnit(lastCam.x + viewW, w);
    const vy1 = clampUnit(lastCam.y + viewH, h);
    ctx.strokeStyle = "rgba(255,255,255,0.5)";
    ctx.lineWidth = 1;
    ctx.strokeRect(
      ox + vx0 * scale, oy + vy0 * scale,
      Math.max(2, (vx1 - vx0) * scale), Math.max(2, (vy1 - vy0) * scale)
    );

    // 玩家：自己亮、其他人暗。用渲染插值座標 rx/ry，跟主畫面同步不跳動。
    for (const p of players.values()) {
      const isMe = p.id === myId;
      const px = ox + clampUnit(p.rx, w) * scale;
      const py = oy + clampUnit(p.ry, h) * scale;
      ctx.beginPath();
      ctx.arc(px, py, isMe ? 4 : 2.5, 0, Math.PI * 2);
      ctx.fillStyle = isMe ? "#ffd24a" : "rgba(111,168,220,0.7)";
      ctx.fill();
    }
  }

  // 畫地面 + 世界邊界。有像素 tileset 就鋪草地瓦片,否則退回程式草叢紋理。
  function drawGround(camX, camY) {
    ctx.fillStyle = "#12331f";
    ctx.fillRect(0, 0, viewW, viewH);

    if (artOk("tileset_a")) {
      // 鋪草地瓦片(tileset 第 0 列 = 草地,4 個變體靠座標雜湊挑,免機械重複)。
      const tx0 = Math.floor(camX / TS) - 1;
      const ty0 = Math.floor(camY / TS) - 1;
      const tx1 = Math.floor((camX + viewW) / TS) + 1;
      const ty1 = Math.floor((camY + viewH) / TS) + 1;
      for (let ty = ty0; ty <= ty1; ty++) {
        for (let tx = tx0; tx <= tx1; tx++) {
          const variant = (grassHash(tx, ty) * 4) | 0; // 0..3
          const dx = Math.round(tx * TS - camX);
          const dy = Math.round(ty * TS - camY);
          ctx.drawImage(ART.tileset_a, variant * TS, 0, TS, TS, dx, dy, TS, TS);
        }
      }
      drawDecorations(camX, camY);
    } else {
      // fallback:程式草叢紋理 + 網格
      drawGrassTexture(camX, camY);
      const grid = 80;
      ctx.strokeStyle = "rgba(255,255,255,0.05)";
      ctx.lineWidth = 1;
      const startX = -((camX % grid) + grid) % grid;
      const startY = -((camY % grid) + grid) % grid;
      for (let x = startX; x < viewW; x += grid) {
        ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, viewH); ctx.stroke();
      }
      for (let y = startY; y < viewH; y += grid) {
        ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(viewW, y); ctx.stroke();
      }
    }

    // 世界邊界
    ctx.strokeStyle = "rgba(201,162,75,0.6)";
    ctx.lineWidth = 4;
    ctx.strokeRect(-camX, -camY, world.width, world.height);
  }

  // 把「繼承的工坊農莊」氛圍擺出來:工坊與墜毀飛船放在農地附近(固定世界座標)。
  // 飛船就是 GDD 的星際北極星勾子(玩家看得到、知道未來能飛出去)。
  function drawDecorations(camX, camY) {
    if (!field) return;
    const place = (img, wx, wy) => {
      if (!img || !img.complete || !img.naturalWidth) return;
      const dw = img.naturalWidth, dh = img.naturalHeight;
      const dx = Math.round(wx - camX - dw / 2);
      const dy = Math.round(wy - camY - dh); // 底部對齊 wy
      if (dx + dw < 0 || dy + dh < 0 || dx > viewW || dy > viewH) return;
      ctx.drawImage(img, dx, dy);
    };
    const fx = field.origin_x, fy = field.origin_y;
    place(ART.workshop, fx - 70, fy + 10);             // 工坊在田地左邊
    place(ART.ship, fx + field.cols * field.tile_size + 80, fy + 40); // 飛船在田地右邊
    place(ART.tree, fx - 30, fy - 40);
    place(ART.rock, fx + field.cols * field.tile_size + 20, fy - 20);
  }

  // 32-bit 整數雜湊:給定世界格座標,回傳穩定的 [0,1) 偽亂數。
  // 同一格永遠得到同一值——草叢才不會在移動時亂跳。
  function grassHash(ix, iy) {
    let h = (Math.imul(ix | 0, 374761393) + Math.imul(iy | 0, 668265263)) | 0;
    h = Math.imul(h ^ (h >>> 13), 1274126177) | 0;
    return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
  }

  // 依世界座標撒草叢:對齊美術色票(苔綠明暗三階),只畫可見範圍內、世界界內的格。
  const GRASS_CELL = 30; // 草叢取樣格(世界 px)
  const GRASS_SHADES = ["#2d4d2a", "#3e6a3a", "#173b22"];
  function drawGrassTexture(camX, camY) {
    const cell = GRASS_CELL;
    const gx0 = Math.floor(camX / cell) - 1;
    const gy0 = Math.floor(camY / cell) - 1;
    const gx1 = Math.floor((camX + viewW) / cell) + 1;
    const gy1 = Math.floor((camY + viewH) / cell) + 1;
    for (let gx = gx0; gx <= gx1; gx++) {
      for (let gy = gy0; gy <= gy1; gy++) {
        const r = grassHash(gx, gy);
        if (r < 0.45) continue; // 留白,別每格都長草
        const wxp = gx * cell + grassHash(gx + 101, gy) * (cell - 6);
        const wyp = gy * cell + grassHash(gx, gy + 211) * (cell - 6);
        if (wxp < 0 || wyp < 0 || wxp > world.width || wyp > world.height) continue;
        const sx = wxp - camX;
        const sy = wyp - camY;
        ctx.fillStyle = GRASS_SHADES[(r * 1000) % GRASS_SHADES.length | 0];
        const len = 3 + r * 3; // 一叢三根短草,輕量(fillRect)
        ctx.fillRect(sx, sy - len, 1.5, len);
        ctx.fillRect(sx - 2, sy - len * 0.7, 1.5, len * 0.7);
        ctx.fillRect(sx + 2, sy - len * 0.8, 1.5, len * 0.8);
      }
    }
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
        if (sx + ts < 0 || sy + ts < 0 || sx > viewW || sy > viewH) continue;
        drawTile(sx, sy, ts, cell);
      }
    }

    // 桌面 hover 高亮:游標所指的田格描一圈亮框 + 淡填,讓玩家清楚「點下去會作用在這格」。
    // 只在能照顧(夠近)時顯示,跟「太遠整塊變淡、點了沒反應」的回饋一致;手機無 hover 自然不畫。
    if (reachable && hoverScreen) {
      const t = fieldTileAtScreen(hoverScreen.x, hoverScreen.y);
      if (t) {
        const hx = field.origin_x + t.col * ts - camX;
        const hy = field.origin_y + t.row * ts - camY;
        ctx.fillStyle = "rgba(255,210,74,0.12)";
        ctx.fillRect(hx + 1, hy + 1, ts - 2, ts - 2);
        ctx.strokeStyle = "rgba(255,210,74,0.9)";
        ctx.lineWidth = 2;
        ctx.strokeRect(hx + 1, hy + 1, ts - 2, ts - 2);
      }
    }

    // 周圍畫一圈黃銅色邊框(對齊世界邊界的設計語彙),從遠處也看得到
    // 「那邊有一塊我的地」。
    ctx.strokeStyle = "#c9a24b";
    ctx.lineWidth = 3;
    ctx.strokeRect(fx - 2, fy - 2, fw + 4, fh + 4);

    // 木柵欄:沿田邊立等距木樁 + 兩條橫桿,讓田看起來像「圈起來的農莊」而非
    // 一塊浮在草地上的色塊(玩家回饋:想要田地周圍的木柵欄)。純程式畫,
    // 之後真 sprite 進來可直接替換。
    drawFence(fx, fy, fw, fh);

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

  // 我方狀態 → field.png 欄位(0 未翻 1 翻土 2 澆水 3 種子 4 發芽 5 成長 6 成熟 7 缺水疊圖)。
  function fieldColumn(cell) {
    switch (cell.state) {
      case 0: return 0;
      case 1: return cell.dry === false ? 2 : 1;
      case 2: return 3;
      case 3: return 4;
      case 4: return 6;
      default: return 0;
    }
  }

  function drawTile(sx, sy, ts, cell) {
    if (artOk("field")) {
      const dx = Math.round(sx), dy = Math.round(sy);
      ctx.drawImage(ART.field, fieldColumn(cell) * TS, 0, TS, TS, dx, dy, ts, ts);
      // 缺水疊圖(藍點 overlay,欄 7),只在有作物且缺水時。
      if (cell.dry && cell.state >= 2 && cell.state <= 4) {
        ctx.drawImage(ART.field, 7 * TS, 0, TS, TS, dx, dy, ts, ts);
      }
      // 成熟再疊一層 additive 金光,夜裡更亮(療癒、乙太味)。
      if (cell.state === 4) {
        ctx.save();
        ctx.globalCompositeOperation = "lighter";
        const g = ctx.createRadialGradient(sx + ts / 2, sy + ts / 2, 1, sx + ts / 2, sy + ts / 2, ts / 2);
        g.addColorStop(0, "rgba(255,210,74,0.5)");
        g.addColorStop(1, "rgba(255,210,74,0)");
        ctx.fillStyle = g;
        ctx.fillRect(sx, sy, ts, ts);
        ctx.restore();
      }
      return;
    }

    // ---- 以下為無 sprite 時的程式繪製 fallback ----
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

  // 木柵欄:在田四邊立等距木樁,兩條橫桿把樁串起來。樁位由邊長算出固定間距,
  // 所以鏡頭移動時柵欄「貼」在田邊不會抖。柵欄畫在田框外側一點,不蓋到作物格。
  function drawFence(fx, fy, fw, fh) {
    const POST_GAP = 26;   // 木樁間距(畫面 px)
    const POST_W = 4;      // 木樁寬
    const POST_H = 14;     // 木樁高(往田外長)
    const wood = "#6b4a2b";
    const woodHi = "#8a6438"; // 樁頂亮面,給一點立體感
    const margin = 4;         // 離黃銅框外側一點
    const left = fx - margin;
    const top = fy - margin;
    const right = fx + fw + margin;
    const bottom = fy + fh + margin;

    // 兩條橫桿(上下圍一圈),柵欄感主要靠這個。
    ctx.strokeStyle = wood;
    ctx.lineWidth = 2;
    for (const inset of [3, POST_H - 4]) {
      ctx.strokeRect(left, top - POST_H + inset, right - left, bottom - top + 2 * (POST_H - inset));
    }

    // 沿四邊撒木樁。每邊用 round 讓兩端對齊角落、間距平均。
    function postsAlong(x0, y0, x1, y1, vertical) {
      const span = vertical ? Math.abs(y1 - y0) : Math.abs(x1 - x0);
      const n = Math.max(1, Math.round(span / POST_GAP));
      for (let i = 0; i <= n; i++) {
        const t = i / n;
        const px = vertical ? x0 : x0 + (x1 - x0) * t;
        const py = vertical ? y0 + (y1 - y0) * t : y0;
        ctx.fillStyle = wood;
        ctx.fillRect(px - POST_W / 2, py - POST_H, POST_W, POST_H);
        ctx.fillStyle = woodHi;
        ctx.fillRect(px - POST_W / 2, py - POST_H, POST_W, 3);
      }
    }
    postsAlong(left, top, right, top, false);       // 上邊
    postsAlong(left, bottom, right, bottom, false); // 下邊
    postsAlong(left, top, left, bottom, true);      // 左邊
    postsAlong(right, top, right, bottom, true);    // 右邊
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
    spawnTapFlash(wx, wy); // 純確認回饋:這一下已送出
  }
  // 純鍵盤/無滑鼠玩家:對「自己腳下這格」送農作意圖(空白鍵 / E / F)。玩家回饋
  // 「不一定有滑鼠」——走得動卻點不到田格。這裡只挑目標格(自己的位置)送原始世界
  // 座標,做什麼仍由權威伺服器決定,不在客戶端判規則。
  function farmAtPlayer() {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const me = myId ? players.get(myId) : null;
    if (!me) return;
    if (field && !withinFieldReach(me.x, me.y)) {
      const now = Date.now();
      if (now - lastReachHint > 2500) {
        addChat("系統", "走進農地再按採集鍵照顧作物哦。");
        lastReachHint = now;
      }
      return;
    }
    ws.send(JSON.stringify({ type: "farm", x: me.x, y: me.y }));
    spawnTapFlash(me.x, me.y); // 純確認回饋:這一下已送出
  }
  // 桌面：滑鼠點擊即互動（移動走鍵盤，不衝突）。
  canvas.addEventListener("click", (e) => farmAtScreen(e.clientX, e.clientY));
  // 桌面 hover:記住游標位置以高亮所指田格;移出畫布就清掉(別留個鬼影高亮)。
  canvas.addEventListener("mousemove", (e) => { hoverScreen = { x: e.clientX, y: e.clientY }; });
  canvas.addEventListener("mouseleave", () => { hoverScreen = null; });

  // 螢幕座標 → 農地格 {col,row};落在田格範圍內才回傳,否則 null。純表現用
  // (高亮游標所指格),不參與任何互動判定——互動仍送原始世界座標給權威伺服器決定。
  function fieldTileAtScreen(sx, sy) {
    if (!field) return null;
    const rect = canvas.getBoundingClientRect();
    const wx = sx - rect.left + lastCam.x;
    const wy = sy - rect.top + lastCam.y;
    const col = Math.floor((wx - field.origin_x) / field.tile_size);
    const row = Math.floor((wy - field.origin_y) / field.tile_size);
    if (col < 0 || row < 0 || col >= field.cols || row >= field.rows) return null;
    return { col, row };
  }

  // ---- 聊天 ----
  const MAX_CHAT_LINES = 60;
  let chatUnread = 0; // 收合狀態下累積的未讀數,展開即清零
  // 收合時有新訊息就在標題列冒未讀數,讓收著的玩家知道有人說話、不漏訊息。
  function bumpChatUnread() {
    chatUnread++;
    const badge = document.getElementById("chatUnread");
    if (badge) badge.textContent = `(${chatUnread})`;
  }
  function addChat(who, text) {
    const log = document.getElementById("chatLog");
    log.style.display = "block";
    // 第一次有訊息才顯示「聊天」標題列(沒人說話時不佔位);收合中則累計未讀。
    const toggle = document.getElementById("chatToggle");
    if (toggle) toggle.style.display = "block";
    if (document.getElementById("chat").classList.contains("chat-collapsed")) bumpChatUnread();
    // 加新行「之前」先量玩家是否已捲在底部附近:玩家往上捲讀舊訊息時,新訊息不該把他
    // 硬拉回底部(讀不完歷史是聊天介面經典 bug)。容差 24px 吸收次像素誤差與行高。
    const atBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 24;
    const line = document.createElement("div");
    // 系統訊息(連線中斷、靠近農地提示)淡化斜體,跟真人發言視覺區隔,不互相搶眼。
    if (who === "系統") line.className = "sys";
    line.innerHTML = `<span class="who"></span>: <span class="msg"></span>`;
    line.querySelector(".who").textContent = who;
    line.querySelector(".msg").textContent = text;
    log.appendChild(line);
    // 長時間掛機聊天會無上限堆 DOM(慢慢吃記憶體、捲動也變重);只留最近 N 則,舊的移除。
    while (log.childElementCount > MAX_CHAT_LINES) log.removeChild(log.firstElementChild);
    // 只有本來就貼著底部(在追最新)才自動捲到底;正在讀舊訊息就保持原位不打擾。
    if (atBottom) log.scrollTop = log.scrollHeight;
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
  // 關閉建議箱(收掉 modal、清掉殘留狀態字)。集中成一個函式,讓「取消鈕／點背景
  // 遮罩／按 Esc」三條關閉路徑行為一致。
  function closeSuggestModal() {
    modal.style.display = "none";
    document.getElementById("suggestStatus").textContent = "";
  }
  document.getElementById("suggestCancel").addEventListener("click", closeSuggestModal);
  // 點 modal 外的暗色遮罩關閉(只在點到遮罩本身、非點到內層面板時)——對話框「點外面
  // 關掉」是普遍預期,少一步找取消鈕。
  modal.addEventListener("click", (e) => { if (e.target === modal) closeSuggestModal(); });
  // Esc 關閉:即使焦點正在建議箱的輸入欄/文字框內也要能關(故獨立監聽,不受遊戲
  // 按鍵守衛影響);只在 modal 開著時作用,不干擾平常的遊戲操作。
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && modal.style.display === "flex") closeSuggestModal();
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

  // ---- 操作說明可收合 ----
  // 點標題列收起／展開;狀態存 localStorage,玩家選過就尊重他的選擇。
  // 沒選過時依視窗寬度給預設:窄畫面(手機直式)預設收起省空間,寬畫面預設展開讓新手看得到。
  function initHelpToggle() {
    const hud = document.getElementById("hud");
    const toggle = document.getElementById("helpToggle");
    if (!hud || !toggle) return;
    let collapsed;
    try { collapsed = localStorage.getItem("butfun.helpCollapsed"); } catch {}
    if (collapsed === null || collapsed === undefined) {
      collapsed = window.innerWidth < 560 ? "1" : "0"; // 窄畫面預設收起
    }
    const apply = (v) => {
      const isCollapsed = v === "1";
      hud.classList.toggle("help-collapsed", isCollapsed);
      // 收起＝內容隱藏＝aria-expanded false,讓螢幕報讀器報出展開/收合狀態
      toggle.setAttribute("aria-expanded", isCollapsed ? "false" : "true");
    };
    apply(collapsed);
    const flip = () => {
      const next = hud.classList.contains("help-collapsed") ? "0" : "1";
      apply(next);
      try { localStorage.setItem("butfun.helpCollapsed", next); } catch {}
    };
    toggle.addEventListener("click", flip);
    // 鍵盤可達:Tab 聚焦後 Enter / 空白鍵也能收合。stopPropagation 擋掉全域 keydown,
    // 免得空白被當「採腳下格」、Enter 被搶去 focus 聊天。
    toggle.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") { e.preventDefault(); e.stopPropagation(); flip(); }
    });
  }

  // ---- 聊天紀錄可收合 ----
  // 同操作說明:點標題列收/展,狀態存 localStorage,尊重玩家選擇。預設不收(維持
  // 既有行為,新手看得到聊天);展開時清零未讀並捲到最新。收著時 addChat 會累計未讀數。
  function initChatToggle() {
    const chat = document.getElementById("chat");
    const toggle = document.getElementById("chatToggle");
    if (!chat || !toggle) return;
    let collapsed = "0";
    try {
      const v = localStorage.getItem("butfun.chatCollapsed");
      if (v !== null && v !== undefined) collapsed = v;
    } catch {}
    chat.classList.toggle("chat-collapsed", collapsed === "1");
    toggle.setAttribute("aria-expanded", collapsed === "1" ? "false" : "true");
    const flip = () => {
      const nowCollapsed = chat.classList.toggle("chat-collapsed");
      toggle.setAttribute("aria-expanded", nowCollapsed ? "false" : "true");
      try { localStorage.setItem("butfun.chatCollapsed", nowCollapsed ? "1" : "0"); } catch {}
      if (!nowCollapsed) {
        chatUnread = 0;
        document.getElementById("chatUnread").textContent = "";
        const log = document.getElementById("chatLog");
        log.scrollTop = log.scrollHeight; // 展開直接看到最新
      }
    };
    toggle.addEventListener("click", flip);
    // 鍵盤可達:同操作說明,Enter / 空白鍵收合,並擋掉全域 keydown 的採集/聊天行為。
    toggle.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") { e.preventDefault(); e.stopPropagation(); flip(); }
    });
  }

  // ---- 進場流程 ----
  function enterGame() {
    if (started) return; // 自動重連時 welcome 會再來一次，別重複初始化、別啟動第二個 render 迴圈
    started = true;
    initHelpToggle();
    initChatToggle();
    document.getElementById("login").classList.add("hidden");
    for (const id of ["hud", "suggestBtn", "chat"]) {
      document.getElementById(id).classList.remove("hidden");
    }
    requestAnimationFrame(render);
  }

  // 訪客新玩家也配個與主題相襯的隨機代號(玩家建議:新玩家用隨機角色名稱)。
  // 過去訪客不打字就一律叫「拓荒者」,單調又容易撞名;這裡前端自備一份同語彙池
  // (蒸汽龐克太空歌劇:材質/天象 + 角色職),預填進名字欄、玩家可改可重骰。
  // 與後端 users.rs 的 codename 池各自獨立(原生前端不共用 Rust),語彙刻意對齊。
  const CODENAME_ADJ = ["黃銅", "霧鏽", "星塵", "發條", "蒸汽", "月光", "琥珀", "雲頂", "銅環", "微光", "漂浮", "齒輪"];
  const CODENAME_NOUN = ["拓荒者", "領航員", "技師", "夢行者", "旅人", "園丁", "信使", "觀星人", "拾荒者", "鐘錶匠"];
  function randomCodename() {
    const adj = CODENAME_ADJ[(Math.random() * CODENAME_ADJ.length) | 0];
    const noun = CODENAME_NOUN[(Math.random() * CODENAME_NOUN.length) | 0];
    const num = 100 + ((Math.random() * 900) | 0); // 100..=999,降低撞名
    return `${adj}${noun}-${num}`;
  }

  // 在這台裝置上記住名字與種族,refresh 不用重打(訪客流程才用)
  try {
    const savedName = localStorage.getItem("butfun.name");
    // 有存過名字就帶回;沒有(全新訪客)就預填一個隨機代號,不再一律「拓荒者」。
    document.getElementById("nameInput").value = savedName || randomCodename();
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
  // 🎲 重骰:換一個隨機代號(玩家想骰到喜歡的為止)。
  document.getElementById("rerollBtn").addEventListener("click", () => {
    document.getElementById("nameInput").value = randomCodename();
  });

  // 開頁就查 /auth/me:已登入就跳過進場畫面、直接連線(同一帳號跨裝置同一玩家)
  fetch("/auth/me", { credentials: "same-origin" })
    .then((r) => (r.ok ? r.json() : null))
    .then((me) => {
      if (!me) return; // 訪客流程,維持顯示登入畫面
      isGuest = false; // 已登入 → 用下方「已登入：X · 登出」標籤,不另顯示 hudName
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
