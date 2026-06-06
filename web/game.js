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
  // 動態——角色踏步彈跳、夜晚乙太微光的飄移與明滅、收成「+N 乙太」飄字的上飄、輕點田格
  // 漣漪的擴張、重連橫幅的脈動。靜態的畫面照樣資訊完整（夜色濃淡、微光位置、飄字的 +N、
  // 漣漪框住的那格、重連橫幅本身都還在），只是不再動。純表現層、不嵌遊戲規則。
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
  // 進出場提示用的在場名單基準：上一份快照看到的「其他玩家」id→name。
  // 對照每份新快照的差集，推出誰剛來、誰剛走，在聊天裡冒一行系統提示——
  // 這個療癒小世界裡知道身邊有人來/走是社交存在感，純從快照差得，不嵌任何遊戲規則。
  const knownPlayers = new Map();
  // 是否已建立過在場基準：進場/重連的第一份快照只默默記名單、不把既有在場者當成「剛進場」
  // 洗一排提示（對齊 etherKnown 對乙太的同款防洗處理）。
  let presenceKnown = false;
  const keys = { up: false, down: false, left: false, right: false };
  let lastSentInput = "";
  // 伺服器廣播的各玩家農地（per-player，每塊含 owner / origin / 每格 state·dry）；
  // 進場前為空陣列。自己那塊靠 owner === myId 認出（見 myField）。
  let fields = [];
  // 伺服器廣播的世界採集節點（樹/石/乙太礦,每個含 kind/x/y/remaining/harvestable）;進場前為空。
  let nodes = [];
  // 伺服器廣播的世界敵人（戰鬥 1-F,每個含 kind/x/y/hp/max_hp/alive）;進場前為空。
  let enemies = [];
  // 敵人受擊／被打倒的視覺回饋(純表現,從快照 hp 差值觸發):你看得到自己正在打中敵人、
  // 把牠打趴——鏡像玩家受擊紅光(damageFlash)的對稱面。敵人血條很細、移動中採集中很容易
  // 漏看「我正在輸出」,補這道一閃讓「有來有回」一眼可讀。以陣列索引當身分——伺服器每幀
  // 以固定順序輸出同一批敵人(spawn 槽位穩定、被打倒只是原地重生),同槽同 kind 才比對,
  // 避免序變誤觸。只在血量下降／轉被打倒時觸發,不在前端判任何戰鬥規則(伺服器權威)。
  let enemyFx = []; // 每槽 { until:ms, lethal:bool };render 依剩餘時間淡出
  // 是否已同步過初始敵人快照。和乙太/背包/血量同理:進場/重連的第一份快照不拿來比 hp 差值
  // (伺服器若換版重啟,敵人血量可能不同,會誤閃一輪),之後的快照差值才是真的受擊。
  let enemiesSynced = false;
  // 採集判定半徑(像素),與伺服器 GATHER_REACH 對齊:玩家離節點這麼近才採得到。
  const GATHER_REACH = 56;
  // 最近一次快照數到「自己那塊」有作物且缺水的格數（updateFarmHud 算好順手記下）；
  // 給離田時的「回農地」邊緣指標決定要不要強調缺水，數字與 HUD 一致、不另外再數一遍。
  let farmDryCount = 0;
  // 同上,數到「自己那塊」已成熟可收成（state 4）的格數。種田迴圈的回報這一步先前沒有
  // 任何離田指標——澆水這個雜務有缺水提醒,收成這個甜頭卻沒,玩家走遠就不知道乙太已熟在等。
  // 補一個讓「收成迴圈的回報」也一眼可讀,數字與 HUD 一致、給邊緣指標共用。
  let farmRipeCount = 0;
  let myEther = 0;
  // 是否已同步過初始乙太：避免進場／重連時把既有存量當成一次大量「獲得」而噴一大串飄字。
  let etherKnown = false;
  // 上一次快照的背包數量（item → qty）+ 是否已同步過初始背包。和乙太同理:進場/重連時
  // 既有存量不算「採到」,不噴飄字;之後某品項數量變多才是真的採進來,才噴「+N 🪵」。
  let myInv = new Map();
  let invKnown = false;
  // 上一次快照的生命值 + 是否已同步過初始生命。和乙太/背包同理:進場/重連時不把既有血量
  // 當成「掉血/回血」播報;之後血量變化才是真的受擊或恢復。戰鬥(1-F)剛上線、HP 有 HUD
  // 但看不到畫面的玩家完全收不到受擊——補這條把無障礙弧線延伸到戰鬥(連線/採集/收成/日夜之後)。
  let myHp = 0;
  let hpKnown = false;
  // 受擊時畫面邊緣紅光一閃(damage vignette):看得到畫面的玩家受擊時,HUD 只有一個小數字在變、
  // 移動中很容易漏看「我正在挨打」。報讀器那條已補受擊播報(給看不到畫面的玩家),這條是它對稱的
  // 視覺版——純表現,從權威 HP 差值觸發,不嵌任何規則。記下「閃到何時為止」,render 依剩餘時間淡出。
  let damageFlashUntil = 0;
  let damageFlashLethal = false; // 被打趴(hp<=0)時閃得更重一點
  // 收成得乙太、採集進背包時的「+N」飄字（純表現，從權威數值差值推得，不嵌任何遊戲規則）。
  // 每筆 { wx, wy, text, color, born }：以世界座標固定在獲得當下的玩家位置上方，隨時間上飄淡出。
  const floaters = [];
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
  // 正在當搖桿的那根手指 identifier:多指處理的關鍵。手機上玩時第二指常會碰到畫面
  // (誤觸／手掌／想點田格),先前用 e.touches[0] 追蹤、又在「任何」touchend 都清掉搖桿,
  // 會被第二指的按下重設原點、第二指一放開就整個停止移動——即使主控手指還按著。
  // 鎖定第一根落下的手指,其餘手指的 start/move/end 一律不干擾它。
  let touchId = null;
  // 這次觸碰是否已「成為拖曳」(超過 TAP_SLOP 就再也回不去當點按)。先前用「按下到放開
  // 的直線位移 < 22px」當點按判定,留了個尷尬縫:位移 14~21px 時,setTouchKeys 的方向
  // 死區(14)已讓角色抽動一下走幾格,放開又因 <22 被算成點按去農作——想點田卻先讓角色
  // 顫一下。改用「這次觸碰是否曾拖過 TAP_SLOP」當單一真實來源:沒拖過就純點按(角色全程
  // 不動),拖過才是搖桿。連「拖遠又滑回原點附近」也正確算成移動而非誤判點按。
  let touchDragged = false;
  const TAP_SLOP = 22; // 點按/拖曳的分水嶺(px):>移動死區 14,手指自然微滑不會被當拖曳
  // 最近一次 render 用的鏡頭左上角（世界座標），給點擊換算用。
  const lastCam = { x: 0, y: 0 };
  // 滑鼠在畫面上的位置（螢幕座標），用來在桌面高亮「游標所指的田格」做操作回饋。
  // 純表現:手機沒有 hover、靠輕點即時互動,觸控時不更新它,自然不畫高亮。
  let hoverScreen = null;
  // 新手只要照顧過一次田就不再需要「怎麼按」提示——記在 localStorage,回訪玩家不再被打擾。
  // 純前端引導狀態(不嵌規則):腳下格的動作詞在此旗標為 false 時多帶一行按鍵提示。
  let tendedOnce = false;
  try { tendedOnce = localStorage.getItem("butfun.tendedOnce") === "1"; } catch {}
  function markTendedOnce() {
    if (tendedOnce) return;
    tendedOnce = true;
    try { localStorage.setItem("butfun.tendedOnce", "1"); } catch {}
  }
  // 採集是後來才上的新玩法,跟農作分開記:已農過田的回訪玩家(tendedOnce=1)沒採過,
  // 仍該在第一次走到節點旁時看到「怎麼採」。採過一次就不再提示,不長期擾人。
  let gatheredOnce = false;
  try { gatheredOnce = localStorage.getItem("butfun.gatheredOnce") === "1"; } catch {}
  function markGatheredOnce() {
    if (gatheredOnce) return;
    gatheredOnce = true;
    try { localStorage.setItem("butfun.gatheredOnce", "1"); } catch {}
  }

  // 上一拍「最近可採節點」的穩定鍵（kind@x,y）。看得到的玩家走進可採範圍會看到黃環+「採X」+
  // 「按空白鍵或點一下」;報讀器玩家原本毫無回饋,只能到處亂按鍵碰運氣。用來在「走進新可採節點
  // 範圍」那拍播一句給報讀器,延續採空/採到/連線/日夜的無障礙弧線。離開再進來(鍵變了)才重播,
  // 同顆站著不重複擾人。純讀快照、不在前端判規則(判定半徑 GATHER_REACH 與伺服器一致)。
  let lastReachableKey = null;

  // ---- 畫布尺寸 ----
  // viewW/viewH 是「邏輯像素」的視窗尺寸,所有繪製碼一律用這兩個值(鏡頭置中、視野
  // 裁切、小地圖定位…),不直接讀 canvas.width/height——那是放大後的實體像素緩衝。
  let viewW = window.innerWidth;
  let viewH = window.innerHeight;
  let dpr = 1; // 裝置像素比,resize 時更新
  // 螢幕安全區內距(瀏海/圓角/底部手勢條)。DOM HUD 已用 CSS env(safe-area-inset-*) 讓開被
  // 切的邊,但 canvas 內畫的小地圖／農地指標是用 viewW/viewH 從邊緣定位、讀不到那些 inset,
  // 在 notched 手機(尤其橫式或底部手勢條)會被瀏海/圓角/手勢條切掉一角。用一顆隱形探針把
  // 四邊 inset 量成數字(getComputedStyle 讀 env() 解出的 padding),resize 時更新,讓 canvas
  // HUD 跟 DOM 面板共用同一套安全區、一起讓邊。一般螢幕 inset 恆為 0,無副作用。延續 HUD
  // safe-area 修復家族到 canvas 層,不碰任何遊戲規則(WebXR renderer 可各自實作)。
  const safeArea = { top: 0, right: 0, bottom: 0, left: 0 };
  const saProbe = document.createElement("div");
  saProbe.style.cssText =
    "position:fixed;visibility:hidden;pointer-events:none;top:0;left:0;width:0;height:0;" +
    "padding-top:env(safe-area-inset-top);padding-right:env(safe-area-inset-right);" +
    "padding-bottom:env(safe-area-inset-bottom);padding-left:env(safe-area-inset-left);";
  document.body.appendChild(saProbe);
  function readSafeArea() {
    const cs = getComputedStyle(saProbe);
    safeArea.top = parseFloat(cs.paddingTop) || 0;
    safeArea.right = parseFloat(cs.paddingRight) || 0;
    safeArea.bottom = parseFloat(cs.paddingBottom) || 0;
    safeArea.left = parseFloat(cs.paddingLeft) || 0;
  }
  function resize() {
    dpr = window.devicePixelRatio || 1;
    viewW = window.innerWidth;
    viewH = window.innerHeight;
    readSafeArea();
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
      // 還沒進場就斷＝初次連線沒成功（伺服器慢／剛重啟／網路抖）。登入畫面沒有橫幅,
      // 改在登入狀態列說明「還在重試」,玩家才知道按鈕沒壞、不必狂點。
      else setLoginStatus("連線不太順，重試中…");
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
  // 登入畫面的連線回饋（按進場到收到 welcome 之間）。enterGame 會整塊隱藏登入畫面,
  // 狀態字隨之消失,不必另外清空。
  function setLoginStatus(text) {
    const el = document.getElementById("loginStatus");
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
          hpKnown = false; // 同乙太:重連後第一份快照重建血量基準,別把既有血量當成一次受擊/回血
          presenceKnown = false; // 重連後第一份快照重建在場基準，別把還在線的人當「剛進場」
          enemiesSynced = false; // 同上:重連後第一份快照重建敵人基準,別把換版後的血量差當成受擊
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
            // 戰鬥(1-F)血量也要逐快照更新,否則 map 裡的 hp 會停在進場那一刻——
            // drawPlayer 的「被打趴變淡」要靠它讀到每位玩家當下的權威血量。
            existing.hp = p.hp;
            existing.max_hp = p.max_hp;
          } else {
            players.set(p.id, { ...p, rx: p.x, ry: p.y });
          }
        }
        // 移除快照中已不存在的玩家
        for (const id of players.keys()) {
          if (!seen.has(id)) players.delete(id);
        }
        // 進出場提示：拿這份快照的「其他玩家」對照上一份的基準，誰新出現＝剛進場、
        // 誰不見了＝剛離開，各冒一行系統提示。第一份快照（presenceKnown=false）只建基準
        // 不提示，免得進場/重連把既有在場者洗成一排「剛進場」。
        const otherIds = new Set();
        for (const p of msg.players) {
          if (p.id === myId) continue;
          otherIds.add(p.id);
          if (presenceKnown && !knownPlayers.has(p.id)) {
            addChat("系統", `${p.name} 來到了邊境星 ✨`);
          }
        }
        for (const [id, name] of knownPlayers) {
          if (!otherIds.has(id)) {
            if (presenceKnown) addChat("系統", `${name} 離開了邊境星`);
            knownPlayers.delete(id);
          }
        }
        for (const p of msg.players) {
          if (p.id !== myId) knownPlayers.set(p.id, p.name);
        }
        presenceKnown = true;
        document.getElementById("hudPlayers").textContent = `線上：${msg.players.length}`;
        // 各玩家農地狀態（per-player）+ 世界採集節點 + 我的乙太/背包 + 日夜
        fields = msg.fields || [];
        nodes = msg.nodes || []; // 防呆:舊版伺服器沒這欄 → 空陣列,不崩
        // 敵人受擊回饋:比對新舊快照同槽(索引穩定,見 enemyFx 宣告),血量下降就在那隻
        // 身上閃一下、被打倒(alive 轉 false)閃得更重。純表現,不改任何狀態。
        const prevEnemies = enemies;
        const nextEnemies = msg.enemies || []; // 防呆:舊版伺服器沒這欄 → 空陣列,不崩
        if (enemiesSynced) {
          const fxNow = performance.now();
          for (let i = 0; i < nextEnemies.length; i++) {
            const ne = nextEnemies[i];
            const oe = prevEnemies[i];
            // 同槽同 kind 才比(避免敵人組成變動時誤觸);hp 下降或被打倒才閃
            if (!oe || oe.kind !== ne.kind) continue;
            const died = oe.alive && !ne.alive;
            if (died || (ne.alive && ne.hp < oe.hp)) {
              enemyFx[i] = { until: fxNow + (died ? 480 : 280), lethal: died };
            }
          }
        }
        enemies = nextEnemies;
        enemiesSynced = true;
        daynight = msg.daynight;
        if (daynight) updateDayNightHud(daynight);
        updateFarmHud(myField());
        const me = msg.players.find((p) => p.id === myId);
        if (me) {
          // 乙太變多 → 收成回饋飄字（首次同步不噴，否則進場/重連會把存量當成一次大獲得）。
          if (etherKnown && me.ether > myEther) {
            spawnEtherFloater(me.ether - myEther, me.x, me.y);
            // 飄字看不到畫面的玩家收不到——比照採集(下方 gained)補一句 aria-live 播報,
            // 讓報讀器玩家照顧作物收成時也即時聽到「+N 乙太」。延續採集/背包/日夜的無障礙弧線。
            announce(`收成 ${me.ether - myEther} 乙太`);
          }
          myEther = me.ether;
          etherKnown = true;
          document.getElementById("hudEther").textContent = `乙太：${myEther}`;
          const inv = me.inventory || []; // 防呆:舊版沒這欄 → 空背包
          // 背包某品項變多 → 採集回饋飄字（首次同步不噴,否則進場/重連會把存量當成一次大採）。
          if (invKnown) {
            let stack = 0; // 同一拍多項一起變多時上下疊開,不互相蓋住
            const gained = []; // 同一拍採到的品項,湊成一句報給報讀器
            for (const s of inv) {
              const gain = s.qty - (myInv.get(s.item) || 0);
              if (gain > 0) {
                spawnGatherFloater(s.item, gain, me.x, me.y, stack++);
                gained.push(`${gain} ${ITEM_NAME[s.item] || s.item}`);
              }
            }
            // 飄字對看不到畫面的玩家無效,補一句 aria-live 播報——延續日夜/連線/聊天的
            // 無障礙弧線,讓報讀器玩家也即時知道「採到什麼」。只在真的採到時報。
            if (gained.length) announce(`採到 ${gained.join("、")}`);
          }
          myInv = new Map(inv.map((s) => [s.item, s.qty]));
          invKnown = true;
          updateBagHud(inv);
          updateCraftPanel(inv); // 合成台:夠不夠料的反灰隨背包快照更新
          updateExpandPanel(me); // 擴地:下一格價/夠不夠買隨乙太(與未來 expansions)快照更新
          updateHpHud(me.hp, me.max_hp); // 戰鬥 1-F:血量 HUD
          // 血量變化 → 補一句 aria-live 播報。HP HUD 是純視覺,看不到畫面的玩家在戰鬥中
          // 完全不知道自己正在挨打;受擊最該即時知道(攸關生死),回血則報一句安心。從快照
          // 差值推得、不嵌任何規則,延續採集/收成/日夜/連線的無障礙弧線。首次同步不報。
          if (hpKnown && me.hp !== myHp) {
            if (me.hp < myHp) {
              announce(me.hp <= 0 ? "你被打趴了" : `受到攻擊,生命 ${me.hp}/${me.max_hp}`);
              damageFlashLethal = me.hp <= 0; // 被打趴閃得更重
              damageFlashUntil = performance.now() + (damageFlashLethal ? 600 : 380); // 一閃即逝、隨即淡出
            } else {
              announce(`恢復生命 ${me.hp}/${me.max_hp}`);
            }
          }
          myHp = me.hp;
          hpKnown = true;

          // 訪客在 HUD 看到自己的遊戲代號——進場後才知道自己叫什麼,也確認顯示的是代號非真名。
          if (isGuest) {
            const nameEl = document.getElementById("hudName");
            nameEl.textContent = `你：${me.name}`;
            nameEl.classList.remove("hidden");
            // 訪客回饋最大宗是「一進來沒農地、不知道下一步」——進場就亮出明確的下一步
            // (登入即有專屬田)。登入者 isGuest=false 永不顯示,不打擾老玩家。
            document.getElementById("hudGuestHint").classList.remove("hidden");
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

  // 收成得乙太時，在玩家當下位置上方記一筆金色飄字（世界座標，鏡頭移動也黏在原地飄起）。
  function spawnEtherFloater(gain, wx, wy) {
    floaters.push({ wx, wy: wy - 22, text: `+${gain} 乙太 ✨`, color: "255,210,74", born: performance.now() });
  }

  // 採集進背包時，在玩家當下位置上方記飄字（多項一起獲得就上下疊開,不互相蓋住）。
  // 顏色用背包品項色,讓「採到什麼」一眼可分;同樣從快照差值推得,不嵌規則。
  function spawnGatherFloater(item, gain, wx, wy, stackIdx) {
    const icon = ITEM_LOOK[item] || item;
    const color = ITEM_FLOAT_COLOR[item] || "230,230,230";
    floaters.push({ wx, wy: wy - 22 - stackIdx * 18, text: `+${gain} ${icon}`, color, born: performance.now() });
  }

  // 把飄字逐一上飄、淡出，過了壽命就移除。畫在日夜染色之後（當回饋 HUD，不被夜色蓋暗）。
  const FLOAT_MS = 1100;
  function drawFloaters(camX, camY, now) {
    for (let i = floaters.length - 1; i >= 0; i--) {
      const f = floaters[i];
      const age = now - f.born;
      if (age >= FLOAT_MS) { floaters.splice(i, 1); continue; }
      const t = age / FLOAT_MS;
      const alpha = 1 - t;
      const sx = f.wx - camX;
      // 開「減少動態」時不上飄,只在原地淡出（+N 的資訊由文字本身傳達,上飄純裝飾）。
      const sy = f.wy - camY - (reduceMotion ? 0 : t * 34);
      ctx.font = "bold 15px system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.fillStyle = `rgba(0,0,0,${(alpha * 0.5).toFixed(3)})`;
      ctx.fillText(f.text, sx + 1, sy + 1); // 描影,任何地表上都讀得到
      ctx.fillStyle = `rgba(${f.color},${alpha.toFixed(3)})`;
      ctx.fillText(f.text, sx, sy);
    }
  }

  // 送出農作意圖時記一筆確認漣漪。落在田格內就吸附到該格中心(漣漪剛好框住整格);
  // 否則(腳下沒田格的邊角點)就用原始點。座標為世界座標,鏡頭移動也黏在原處。
  function spawnTapFlash(wx, wy) {
    let cx = wx, cy = wy;
    const mf = myField();
    if (mf) {
      const ts = mf.tile_size;
      const col = Math.floor((wx - mf.origin_x) / ts);
      const row = Math.floor((wy - mf.origin_y) / ts);
      if (col >= 0 && row >= 0 && col < mf.cols && row < mf.rows) {
        cx = mf.origin_x + (col + 0.5) * ts;
        cy = mf.origin_y + (row + 0.5) * ts;
      }
    }
    tapFlashes.push({ wx: cx, wy: cy, born: performance.now() });
  }

  // 漣漪：由小擴張到約一格大、同時淡出。畫在日夜染色之後（當回饋 HUD,不被夜色蓋暗）。
  const TAP_MS = 360;
  function drawTapFlashes(camX, camY, now) {
    const mf = myField();
    const ts = mf ? mf.tile_size : 24;
    for (let i = tapFlashes.length - 1; i >= 0; i--) {
      const f = tapFlashes[i];
      const age = now - f.born;
      if (age >= TAP_MS) { tapFlashes.splice(i, 1); continue; }
      const t = age / TAP_MS;
      const sx = f.wx - camX;
      const sy = f.wy - camY;
      // 開「減少動態」時不擴張,改畫固定半格大的環、只淡出（「點在這格」由位置本身傳達,
      // 由小到大的擴張純裝飾,前庭敏感的玩家輕點仍有靜態回饋）。
      const r = reduceMotion ? ts * 0.5 : ts * (0.28 + t * 0.42); // 由小擴張到約半格半徑
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
  // 階段「換場」時播給報讀器的整句（不帶 emoji——報讀器唸 emoji 名稱常突兀）。
  // 視覺上日夜變化已有 HUD 文字＋夜晚乙太微光,看不到畫面的玩家卻完全收不到時段切換,
  // 補上這條讓報讀器在天色轉換時報一句,延續 srStatus(連線/聊天)的無障礙弧線。
  const PHASE_ANNOUNCE = {
    dawn: "破曉了，天色漸亮",
    day: "天亮了",
    dusk: "黃昏了，天色漸暗",
    night: "入夜了",
  };
  // 首份快照只建基準、不報(比照 presenceKnown:進場/重連不該把「現在的時段」當成切換)。
  let lastPhase = null;
  function updateDayNightHud(dn) {
    const el = document.getElementById("hudTime");
    if (el) el.textContent = PHASE_LABELS[dn.phase] || "—";
    // 只在階段真的變了、且已過基準時報讀器播一句,避免每幀重唸。
    if (dn.phase !== lastPhase) {
      if (lastPhase !== null && PHASE_ANNOUNCE[dn.phase]) announce(PHASE_ANNOUNCE[dn.phase]);
      lastPhase = dn.phase;
    }
  }

  // 農地缺水提醒：數出快照裡「有作物且缺水」的格數，顯示在 HUD，讓玩家離開田去
  // 探索時也知道作物渴了該回去澆水。缺水格的判定刻意對齊 drawTile 畫藍點的條件
  //（state 2~4 且 dry），HUD 數字與看得到的提示點一致；沒有缺水格時隱藏整行。
  // 純從權威快照數得的表現層回饋，不嵌任何遊戲規則（將來 WebXR renderer 可各自實作）。
  function updateFarmHud(f) {
    const el = document.getElementById("hudFarm");
    const ripeEl = document.getElementById("hudRipe");
    let dry = 0;
    let ripe = 0;
    if (f && f.cells) {
      for (const cell of f.cells) {
        if (cell.dry && cell.state >= 2 && cell.state <= 4) dry++;
        if (cell.state === 4) ripe++; // 4=成熟可收成(鏡像 field.rs 的 state 定義)
      }
    }
    farmDryCount = dry;
    farmRipeCount = ripe;
    if (el) {
      if (dry > 0) {
        el.textContent = `🌱 ${dry} 格作物缺水`;
        el.classList.remove("hidden");
      } else {
        el.classList.add("hidden");
      }
    }
    // 收成提醒:有熟透的格子就告訴玩家「去收乙太」,把迴圈的回報那步顯到 HUD,
    // 跟缺水提醒對稱;沒有就隱藏不佔行。純從權威快照數得,不嵌任何遊戲規則。
    if (ripeEl) {
      if (ripe > 0) {
        ripeEl.textContent = `✨ ${ripe} 格可收成`;
        ripeEl.classList.remove("hidden");
      } else {
        ripeEl.classList.add("hidden");
      }
    }
  }

  // ---- 離田時的「回農地」邊緣指標 ----
  // 農地完全移出畫面時，在螢幕邊緣畫一個指向農地的小箭頭，讓玩家走遠探索後一眼知道
  // 「我的田在哪個方向」、要回去澆水時不必先開小地圖對位。農地在畫面內時不畫（不打擾）。
  // 純從農地世界座標 + 鏡頭推得的表現層回饋，不嵌任何遊戲規則（WebXR renderer 可各自實作）。
  function drawFarmPointer(camX, camY) {
    const mf = myField();
    if (!mf || !mf.cols || !mf.rows) return;
    // 農地中心（世界座標）→ 螢幕座標。
    const cx = mf.origin_x + (mf.cols * mf.tile_size) / 2;
    const cy = mf.origin_y + (mf.rows * mf.tile_size) / 2;
    const sx = cx - camX;
    const sy = cy - camY;
    // 農地矩形是否與畫面相交：相交（看得到田）就不畫指標。用半尺寸當判定半徑近似即可。
    const halfW = (mf.cols * mf.tile_size) / 2;
    const halfH = (mf.rows * mf.tile_size) / 2;
    if (sx + halfW >= 0 && sx - halfW <= viewW &&
        sy + halfH >= 0 && sy - halfH <= viewH) return;

    // 從畫面中心朝農地方向、夾到邊緣內側的安全框上，當作箭頭落點。
    const ccx = viewW / 2, ccy = viewH / 2;
    const ang = Math.atan2(sy - ccy, sx - ccx);
    const m = 46; // 邊緣留白，避開 HUD / 小地圖最外圈
    // 邊緣框再加安全區內距,指標不躲到瀏海/圓角/手勢條底下(與小地圖同套安全區)。
    const px = Math.max(m + safeArea.left, Math.min(viewW - m - safeArea.right, sx));
    const py = Math.max(m + safeArea.top, Math.min(viewH - m - safeArea.bottom, sy));

    const dry = farmDryCount > 0;
    const ripe = !dry && farmRipeCount > 0; // 缺水(雜務)較急,先顯;不缺水才換成可收成(回報)
    // 缺水→田格藍色澆水語彙;否則熟透→亮金催玩家回去收乙太;都沒有→低調黃銅。
    const color = dry ? "#7fbfff" : ripe ? "#ffd24a" : "rgba(201,162,75,0.85)";

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
    const label = dry ? `🌱${farmDryCount}` : ripe ? `✨${farmRipeCount}` : "🌱";
    // 標籤擺在箭頭「背向農地」那側，不擋住指向。
    const lx = px - Math.cos(ang) * 24;
    const ly = py - Math.sin(ang) * 24;
    ctx.lineJoin = "round";
    ctx.lineWidth = 3;
    ctx.strokeStyle = "rgba(0,0,0,0.55)";
    ctx.strokeText(label, lx, ly);
    ctx.fillStyle = dry ? "#bfe0ff" : ripe ? "#ffe9a0" : "#e8e0cf";
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
    }
    // 退回實體鍵位:非 QWERTY 鍵盤(AZERTY/QWERTZ/Dvorak…)上,WASD 字母落在別處——
    // 法文鍵盤按左手那叢實體鍵送出的是 z/q/s/d,上面的 e.key 比對會落空,玩家照說明按
    // 「W」卻不動。e.code 是與鍵盤布局無關的實體位置,讓「按 WASD 鍵位移動」對所有布局
    // 成立(方向鍵本就無此問題)。keydown/keyup 共用本函式,同一次實體按放一律對到同一向,
    // 不會出現按下/放開比對到不同鍵的卡鍵。
    switch (e.code) {
      case "KeyW": return "up";
      case "KeyS": return "down";
      case "KeyA": return "left";
      case "KeyD": return "right";
      default: return null;
    }
  }

  window.addEventListener("keydown", (e) => {
    // 建議箱對話框開著時，完全不攔截遊戲按鍵——背景遮罩雖設了 inert（封閉滑鼠/報讀器），
    // 但這個監聽掛在 window 上、inert 擋不住，焦點落在對話框的按鈕（取消/送出，activeElement
    // 是 <button> 不是 INPUT/TEXTAREA、漏過下面那道守衛）時按 WASD 角色會在 modal 背後偷走、
    // 按 M 切換背後小地圖、按 Enter 還把焦點搶去背景聊天框（與按鈕本身的 Enter 啟用衝突）。
    // 對話框的 Esc 關閉／Tab 焦點環是另一條獨立監聽，不受此早退影響。延續「角色別在玩家沒在
    // 控時亂走」的修復家族。
    if (modal.style.display === "flex") return;
    // 在任何文字輸入欄打字時，完全不攔截遊戲按鍵——否則 w/a/s/d、方向鍵會被
    // e.preventDefault() 吃掉、角色還在背景亂走，Enter 也被搶去 focus 聊天。
    // 尤其建議箱 #suggestText 是 <textarea>（先前只擋 INPUT 沒擋它），玩家寫
    // 回饋時打到這些字就壞掉——而建議箱正是 devloop 收回饋的主要管道。
    const el = document.activeElement;
    if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA")) return;
    // 有 Ctrl / Cmd / Alt 修飾鍵時完全不攔截：那些是瀏覽器／系統快捷鍵（Ctrl+F 找頁面、
    // Ctrl+S 存檔、Cmd/Ctrl+W 關分頁、Ctrl+A 全選、Cmd+M 縮小視窗…）。遊戲把單字母鍵
    // 當移動／採集（空白·E·F）／地圖（M）用並 e.preventDefault()，沒擋修飾鍵時會把這些
    // 快捷鍵整個吃掉失效，角色還順手亂動一下。延續「角色別在玩家沒在控時亂走」的修復家族：
    // 按組合鍵＝玩家在操作瀏覽器、不是在操控角色，放行給瀏覽器處理。Shift 不算（Shift 不
    // 改變鍵的快捷義，且 Shift+WASD 仍該照常移動）。
    if (e.ctrlKey || e.metaKey || e.altKey) return;
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
      return;
    }
    // M:收起／展開小地圖(給鍵盤玩家一條與 canvas 收合鈕等效的入口,觸控/滑鼠點鈕亦可)。
    if (e.key === "m" || e.key === "M") {
      if (!e.repeat) toggleMinimap();
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

  // ---- 遊戲手把(玩家建議:不一定有滑鼠/鍵盤,接手把也要能玩) ----
  // 純客戶端輸入源:讀左類比搖桿與十字鍵,折算成與鍵盤／觸控相同的四個方向布林,送一樣的
  // input 訊息給(權威)伺服器——不碰任何遊戲規則(將來 WebXR renderer 連同一後端各自實作)。
  // 用「邊緣觸發」:只在方向有變化的那一幀寫 keys(像 keydown/keyup),空檔不覆寫 keys,
  // 才不會每幀把鍵盤/觸控正按住的方向硬清成 false、彼此打架。
  const gpPrev = { up: false, down: false, left: false, right: false };
  let gpPrevAct = false; // 採集鍵的上一幀按壓狀態,用來做邊緣觸發(一次按一次,不長按連發)
  function pollGamepad() {
    if (!navigator.getGamepads) return;
    let pad = null;
    for (const p of navigator.getGamepads()) { if (p) { pad = p; break; } }
    if (!pad) return;
    const dz = 0.35; // 死區:類比搖桿歸中常有微小漂移,小於此量不算方向
    const ax = pad.axes[0] || 0, ay = pad.axes[1] || 0;
    const btn = (i) => !!(pad.buttons[i] && pad.buttons[i].pressed); // 標準佈局十字鍵 12~15
    const cur = {
      up:    ay < -dz || btn(12),
      down:  ay >  dz || btn(13),
      left:  ax < -dz || btn(14),
      right: ax >  dz || btn(15),
    };
    let changed = false;
    for (const d of ["up", "down", "left", "right"]) {
      if (cur[d] !== gpPrev[d]) { keys[d] = cur[d]; gpPrev[d] = cur[d]; changed = true; }
    }
    if (changed) sendInputIfChanged();
    // 採集鍵:標準佈局臉鈕(A=0 / B=1 / X=2 / Y=3)任一按下都對腳下田格互動,等同鍵盤的
    // 空白·E·F——手把支援先前只接了走動,玩家(尤其無滑鼠/鍵盤者)走到田邊卻沒鈕能翻土／
    // 播種／澆水／收成。邊緣觸發(只在 false→true 那一幀)讓一次按一次、不長按連發,跟鍵盤
    // 的 !e.repeat 與滑鼠單擊一致。純客戶端輸入源,農作規則仍只在(權威)伺服器。
    const act = btn(0) || btn(1) || btn(2) || btn(3);
    if (act && !gpPrevAct) farmAtPlayer();
    gpPrevAct = act;
  }
  // 手把接上時報一聲:玩家(尤其無滑鼠/報讀器使用者)知道可以直接用手把走動。
  window.addEventListener("gamepadconnected", () =>
    announce("已連接遊戲手把,可用左類比搖桿或十字鍵走動,按臉鈕(A／B／X／Y)照顧腳下作物"));

  // ---- 觸控:任何地方按下拖曳當搖桿,放開即停止 ----
  function setTouchKeys(dx, dy) {
    const dead = 14;
    keys.up = dy < -dead;
    keys.down = dy > dead;
    keys.left = dx < -dead;
    keys.right = dx > dead;
    sendInputIfChanged();
  }
  // 從 TouchList 取出搖桿那根手指(依 identifier);找不到回 null。
  function findTouch(list) {
    for (let i = 0; i < list.length; i++) {
      if (list[i].identifier === touchId) return list[i];
    }
    return null;
  }
  canvas.addEventListener("touchstart", (e) => {
    // 已有手指在當搖桿就不被後續手指接管(避免第二指重設原點、跳動搖桿)。
    if (touchId === null && e.changedTouches.length) {
      const t = e.changedTouches[0];
      touchId = t.identifier;
      touchOrigin = { x: t.clientX, y: t.clientY };
      touchCurrent = { x: t.clientX, y: t.clientY };
      touchDragged = false; // 每次新觸碰先當點按,拖過 TAP_SLOP 才升級成搖桿
    }
    e.preventDefault();
  }, { passive: false });
  canvas.addEventListener("touchmove", (e) => {
    if (touchId === null || !touchOrigin) return;
    const t = findTouch(e.touches);
    if (!t) return; // 這次 move 不含搖桿那根手指(別根手指在動),不理
    touchCurrent = { x: t.clientX, y: t.clientY };
    const dx = t.clientX - touchOrigin.x;
    const dy = t.clientY - touchOrigin.y;
    // 還沒拖過分水嶺前不送任何方向——點按(含手指自然微滑)時角色全程不抽動;
    // 一旦拖過就鎖定成搖桿,之後即使滑回原點附近仍持續吃方向。
    if (!touchDragged && Math.hypot(dx, dy) >= TAP_SLOP) touchDragged = true;
    if (touchDragged) setTouchKeys(dx, dy);
    e.preventDefault();
  }, { passive: false });
  function endTouch(e) {
    // 只在「搖桿那根手指」抬起／取消時才收掉搖桿;別根手指放開不影響移動。
    const t = touchId === null ? null : findTouch(e.changedTouches);
    if (!t) return;
    // 從沒拖過分水嶺(TAP_SLOP)的觸碰當成「輕點」→ 農地互動;拖過的是搖桿移動,不互動。
    // 用「是否曾拖過」而非「放開瞬間的直線位移」判定:拖遠又滑回原點附近也正確算成移動,
    // 不會在放手那刻因離原點近被誤判成點按。
    if (touchOrigin && !touchDragged) farmAtScreen(t.clientX, t.clientY);
    touchId = null;
    touchOrigin = null;
    touchCurrent = null;
    touchDragged = false;
    setTouchKeys(0, 0);
    e.preventDefault();
  }
  canvas.addEventListener("touchend", endTouch, { passive: false });
  canvas.addEventListener("touchcancel", endTouch, { passive: false });

  // 把名牌雜湊成一個穩定的色相(0-359)→ 同族群恆得同色,不同族群一眼分得開。
  // 玩家反覆回報「周圍有各種族群(銅齒/發條/琥珀/電弧/齒輪)卻分不出來」。先前是依 species
  // 上色,但目前可選種族只開放「地球人(terran)」一種,幾乎每位玩家 species 都相同、雜湊後同色
  // ——等於沒分。玩家實際拿來辨識彼此「族群」的,是代號開頭那個材質詞(黃銅/發條/琥珀…,見
  // users.rs 與本檔 CODENAME_ADJ),固定為開頭 2 個字。改以這個材質詞雜湊:同材質族群恆同色、
  // 不同族群一眼分得開,且將來 species 真開放多族時這層也不受影響(屆時可再疊第二層線索)。
  // 純表現層,不需後端改、不嵌規則,WebXR renderer 可各自挑呈現;亮度壓在偏亮區間配名牌既有的
  // 深色描邊,在任何地表/日夜都讀得清,顏色只是「附加線索」、名字本身仍在,不依賴辨色力(無障礙)。
  function kinAccent(name) {
    // 代號形如「黃銅領航員-417」,材質詞固定佔開頭 2 字;自訂短名則退化成用整串名字,
    // 仍得到穩定色。空名給個定值避免 charCodeAt 取到 NaN。
    const s = (name || "拓荒者").trim();
    const kin = s.length >= 2 ? s.slice(0, 2) : s;
    let h = 0;
    for (let i = 0; i < kin.length; i++) h = (h * 31 + kin.charCodeAt(i)) >>> 0;
    return `hsl(${h % 360}, 62%, 78%)`;
  }

  // ---- 渲染迴圈 ----
  // 畫單一玩家(角色 sprite／fallback 圓 + 頭上名字)。抽成獨立函式,讓 render 能
  // 控制畫的順序——別人先畫、自己最後畫,確保自己永遠在最上層。純表現層,不嵌任何
  // 遊戲規則(將來 WebXR renderer 自有角色呈現,這層只屬 2D 客戶端)。
  function drawPlayer(p, camX, camY) {
    const sx = p.rx - camX;
    const sy = p.ry - camY;
    const isMe = p.id === myId;
    // 被打趴(hp<=0,休息復原中):純讀權威快照血量,不在前端判任何戰鬥規則。被打趴的玩家
    // 在世界上要看得出來——HUD 只有自己看得到,其他玩家被打趴在畫面上原本和站著沒兩樣。
    // 鏡像敵人被打倒變淡(globalAlpha 0.25)的對稱面:整個角色壓暗、頭上掛 💤,讓「誰趴下了
    // /休息中」一眼可讀。hp 缺值(訪客或舊伺服器)時當沒打趴,不影響原行為。
    const downed = typeof p.hp === "number" && p.max_hp > 0 && p.hp <= 0;
    // 走路時上下彈跳一點，腳下陰影固定不跟著跳 → 讀起來像在踏步走動。
    // 開「減少動態」時不彈跳（避免持續上下晃造成不適），sprite 仍逐格切換不受影響。
    // 被打趴時不彈跳(在地上休息,不該還在踏步)。
    const bob = (p.moving && !reduceMotion && !downed) ? Math.abs(Math.sin(p.walk)) * 3 : 0;
    const by = sy - bob;

    // 腳下陰影（固定在地面，賣出彈跳的踏地感）
    ctx.beginPath();
    ctx.ellipse(sx, sy + 12, 11, 4, 0, 0, Math.PI * 2);
    ctx.fillStyle = "rgba(0,0,0,0.22)";
    ctx.fill();

    // 被打趴的角色整個壓暗(連同 sprite/fallback),畫完角色再還原,名字保持清晰好認。
    if (downed) ctx.globalAlpha = 0.4;
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
    // 角色畫完還原透明度,名字與 💤 標記保持滿不透明、清晰好認。
    if (downed) {
      ctx.globalAlpha = 1;
      // 頭上掛 💤:被打趴/休息復原中的持續標記(對稱敵人被打倒的一閃,這個是長駐狀態)。
      ctx.font = "14px system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.fillText("💤", sx + 12, sy - 26);
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
    // 自己恆金色(一眼找到自己);別人依代號材質詞(族群)上色,讓不同族群在名牌就分得開。
    ctx.fillStyle = isMe ? "#ffd24a" : kinAccent(p.name);
    ctx.fillText(p.name, sx, sy - 24);
  }

  function render() {
    // 每幀重設基準變換(dpr 縮放),確保前一幀任何 save/restore 失衡也不會累積偏移。
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.imageSmoothingEnabled = false; // 像素風禁止插值放大,否則糊邊
    ctx.clearRect(0, 0, viewW, viewH);

    // 手把沒有事件式的「按住中」回呼,必須每幀輪詢;放在繪製前讓本幀就反映方向。
    pollGamepad();

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
    drawNodes(camX, camY); // 採集節點畫在地表/農地之上、玩家之下
    drawEnemies(camX, camY); // 敵人(戰鬥 1-F)畫在地表之上、玩家之下
    maybeAnnounceReachable(me); // 走進可採節點範圍時播一句給報讀器(鏡像視覺的黃環+「按鍵採集」提示)

    // 畫玩家:先畫別人,最後才畫自己——當別的玩家站到你頭上時,你那顆描金的名字
    // 與角色仍蓋在最上層,不被別人的 sprite／名字遮住,「一眼找到自己」才真的成立。
    for (const p of players.values()) {
      if (p.id !== myId) drawPlayer(p, camX, camY);
    }
    if (me) drawPlayer(me, camX, camY);

    // 日夜染色（疊在世界與玩家上，但在觸控搖桿與 HUD/小地圖之前）。
    drawDayNightTint();

    // 夜晚漂浮的乙太微光：在日夜染色「之後」畫（浮在變暗的世界上），但在飄字／漣漪／
    // 小地圖／HUD「之前」（那些互動回饋與 HUD 仍蓋在最上層、不被微光干擾）。
    drawNightMotes(performance.now());

    // 收成乙太 / 採集進背包的「+N」飄字：在日夜染色「之後」畫，當回饋 HUD 不被夜色蓋暗。
    drawFloaters(camX, camY, performance.now());

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

    // 受擊紅光:畫在最上層(連搖桿/小地圖之上),受擊的當下不被任何東西蓋住,一眼就知道在挨打。
    drawDamageFlash(performance.now());

    requestAnimationFrame(render);
  }

  // 受擊時的全螢幕邊緣紅光:四周往中心淡出的紅暈(中央保持透明、不擋視線),依剩餘時間淡出。
  // 純螢幕座標、純表現——觸發來自權威 HP 差值,這裡只負責畫,不參與任何戰鬥判定。
  function drawDamageFlash(now) {
    if (now >= damageFlashUntil) return;
    const dur = damageFlashLethal ? 600 : 380;
    const t = (damageFlashUntil - now) / dur; // 1→0 隨時間淡出
    const peak = damageFlashLethal ? 0.55 : 0.38;
    const alpha = peak * Math.max(0, Math.min(1, t));
    const cx = viewW / 2;
    const cy = viewH / 2;
    const inner = Math.min(viewW, viewH) * 0.35; // 中央留一塊透明、不擋視線
    const outer = Math.hypot(viewW, viewH) / 2;
    const g = ctx.createRadialGradient(cx, cy, inner, cx, cy, outer);
    g.addColorStop(0, "rgba(200,30,30,0)");
    g.addColorStop(1, `rgba(200,30,30,${alpha.toFixed(3)})`);
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, viewW, viewH);
  }

  // ---- 小地圖（玩家建議：2000x2000 大世界容易迷路）----
  // 右下角畫一張固定大小的世界縮圖：世界邊界、農地位置、自己（亮點）、其他玩家（暗點）。
  // 純螢幕座標、每幀依最新快照重畫，不參與鏡頭換算。
  const MM = { maxSize: 150, minSize: 96, margin: 16, pad: 6 };
  // 小地圖可收合:手機螢幕小,右下角縮圖會吃掉空間(窄直式螢幕還會跟左下聊天框
  // 在底部邊緣相鄰),給玩家一鍵收起的選擇——沿用說明/聊天收合的語彙(點切換、
  // 狀態存 localStorage)。收起時只留一顆小「展開地圖」鈕,點它或按 M 重新展開。
  // 預設展開(維持既有行為,大世界容易迷路靠它定位),只在玩家自己收過才記住。
  let minimapHidden = false;
  try { minimapHidden = localStorage.getItem("butfun.minimapHidden") === "1"; } catch {}
  // 收合鈕在螢幕上的熱區(每幀於 drawMinimap 更新);點擊命中即切換,不當作農作。
  let mmToggleHit = null;
  function toggleMinimap() {
    minimapHidden = !minimapHidden;
    try { localStorage.setItem("butfun.minimapHidden", minimapHidden ? "1" : "0"); } catch {}
  }
  // 小地圖邊長依畫面自適應:手機直式窄螢幕縮小(別吃掉太多空間、也少跟左下聊天框
  // 在底部重疊),平板/桌面維持上限。取畫面短邊的一個比例,夾在 min/max 之間。
  function minimapSize() {
    const shorter = Math.min(viewW, viewH);
    return Math.round(Math.max(MM.minSize, Math.min(MM.maxSize, shorter * 0.26)));
  }
  function drawMinimap() {
    if (!world || !world.width || !world.height) return;
    // 收合狀態:只畫一顆小「展開地圖」鈕在右下角,省下整塊縮圖的空間。
    if (minimapHidden) {
      const bw = 34, bh = 26;
      const bx = viewW - MM.margin - safeArea.right - bw;
      const by = viewH - MM.margin - safeArea.bottom - bh;
      ctx.fillStyle = "rgba(10,16,30,0.7)";
      ctx.fillRect(bx, by, bw, bh);
      ctx.strokeStyle = "rgba(201,162,75,0.7)";
      ctx.lineWidth = 1.5;
      ctx.strokeRect(bx, by, bw, bh);
      ctx.fillStyle = "#c9a24b";
      ctx.font = "16px system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText("🗺", bx + bw / 2, by + bh / 2 + 1);
      ctx.textBaseline = "alphabetic";
      mmToggleHit = { x: bx, y: by, w: bw, h: bh };
      return;
    }
    const w = world.width, h = world.height;
    // 等比縮到 size 方框內，長寬各自映射（世界目前是正方，但不假設）。
    const size = minimapSize();
    const scale = size / Math.max(w, h);
    const mw = w * scale, mh = h * scale;
    // 右下錨點扣掉安全區內距,notched 手機不被瀏海/圓角/手勢條切到。
    const ox = viewW - MM.margin - safeArea.right - mw;   // 縮圖內容左上角（螢幕座標）
    const oy = viewH - MM.margin - safeArea.bottom - mh;
    const clampUnit = (v, hi) => Math.max(0, Math.min(v, hi));

    // 半透明深底面板（對齊夜色色調），讓縮圖在任何地表上都讀得到。
    ctx.fillStyle = "rgba(10,16,30,0.55)";
    ctx.fillRect(ox - MM.pad, oy - MM.pad, mw + MM.pad * 2, mh + MM.pad * 2);

    // 各玩家農地位置（黃銅外框前先畫，免得被框線蓋住）。自己那塊畫亮、別人的暗。
    for (const f of fields) {
      const fx = ox + clampUnit(f.origin_x, w) * scale;
      const fy = oy + clampUnit(f.origin_y, h) * scale;
      const fw = f.cols * f.tile_size * scale;
      const fh = f.rows * f.tile_size * scale;
      ctx.fillStyle = f.owner === myId ? "rgba(201,162,75,0.95)" : "rgba(123,80,40,0.85)";
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

    // 敵人:戰鬥(1-F)已上線、世界又大(2000x2000),光看主畫面只知道身邊有沒有怪,
    // 不知道遠處哪裡成群。小地圖畫出活著的敵人(紅點),讓玩家一眼看出威脅聚在哪、
    // 要避開還是去刷。被打倒(重生中)的不畫——只標當下真正的威脅。沿用敵人血條/
    // 受擊回饋的紅(#d65a5a)當「危險」色語彙,跟玩家點(亮黃/暗藍)區隔。畫在玩家點
    // 之前,讓自己的點疊在最上層仍醒目。純表現、純讀既有快照,不嵌任何戰鬥判定。
    for (const e of enemies) {
      if (!e.alive) continue;
      const ex = ox + clampUnit(e.x, w) * scale;
      const ey = oy + clampUnit(e.y, h) * scale;
      ctx.beginPath();
      ctx.arc(ex, ey, 2.5, 0, Math.PI * 2);
      ctx.fillStyle = "rgba(214,90,90,0.85)";
      ctx.fill();
    }

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

    // 收合鈕:面板右上角一顆小「–」,點它(或按 M)把小地圖收起。畫在最後蓋在縮圖上。
    const tb = 18;
    const tx = ox + mw + MM.pad - tb;
    const ty = oy - MM.pad;
    ctx.fillStyle = "rgba(10,16,30,0.85)";
    ctx.fillRect(tx, ty, tb, tb);
    ctx.strokeStyle = "rgba(201,162,75,0.7)";
    ctx.lineWidth = 1;
    ctx.strokeRect(tx, ty, tb, tb);
    ctx.fillStyle = "#c9a24b";
    ctx.font = "14px system-ui, sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText("–", tx + tb / 2, ty + tb / 2 + 1);
    ctx.textBaseline = "alphabetic";
    mmToggleHit = { x: tx, y: ty, w: tb, h: tb };
  }

  // 命中小地圖收合鈕的熱區?(螢幕座標)命中即切換顯示並回 true,讓點擊不被當作農作。
  // 收合「–」鈕畫得很小(18px)、展開「🗺」鈕也才 34x26,手指在手機上很難精準點到
  // (低於 WCAG 2.5.5 建議的 44px 觸控目標)。命中判定刻意比畫出來的方框外擴一圈,
  // 讓「點在鈕附近」也算命中——視覺不變、只放寬熱區,改善手機操作手感。鈕固定在右下
  // 角(遠離通常置中的農地),外擴不會誤吃田格的輕點。純表現層,不嵌任何遊戲規則。
  const MM_TAP_MIN = 44; // 觸控目標的最小邊長(邏輯像素)
  function minimapToggleHit(clientX, clientY) {
    if (!mmToggleHit) return false;
    const rect = canvas.getBoundingClientRect();
    const sx = clientX - rect.left, sy = clientY - rect.top;
    const t = mmToggleHit;
    // 把熱區從中心放大到至少 MM_TAP_MIN 見方(鈕本來就更大時不縮)。
    const padX = Math.max(0, (MM_TAP_MIN - t.w) / 2);
    const padY = Math.max(0, (MM_TAP_MIN - t.h) / 2);
    if (sx >= t.x - padX && sx <= t.x + t.w + padX &&
        sy >= t.y - padY && sy <= t.y + t.h + padY) {
      toggleMinimap();
      return true;
    }
    return false;
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

    // 滿世界的裝飾(草叢 + 偶爾的樹/石),填滿大世界的空曠感。畫在地表之上、農地/節點/玩家之下。
    drawScenery(camX, camY);

    // 世界邊界
    ctx.strokeStyle = "rgba(201,162,75,0.6)";
    ctx.lineWidth = 4;
    ctx.strokeRect(-camX, -camY, world.width, world.height);
  }

  // 確定性雜湊 [0,1)，給裝飾佈置用（同格座標永遠同結果,不隨鏡頭閃爍）。
  function sceneryHash(a, b) {
    let n = (Math.imul(a | 0, 73856093) ^ Math.imul(b | 0, 19349663)) >>> 0;
    n = (n ^ (n >>> 13)) >>> 0;
    n = Math.imul(n, 1274126177) >>> 0;
    return ((n ^ (n >>> 16)) >>> 0) / 4294967296;
  }

  // 一小撮程式草叢（純畫,不需素材）。
  function drawGrassTuft(sx, sy, h) {
    ctx.strokeStyle = `rgba(${(90 + h * 70) | 0}, ${(140 + h * 60) | 0}, 80, 0.45)`;
    ctx.lineWidth = 1.5;
    for (let i = 0; i < 3; i++) {
      const ang = -Math.PI / 2 + (i - 1) * 0.45;
      const len = 5 + h * 7;
      const bx = sx + (i - 1) * 2.5;
      ctx.beginPath();
      ctx.moveTo(bx, sy);
      ctx.lineTo(bx + Math.cos(ang) * len, sy + Math.sin(ang) * len);
      ctx.stroke();
    }
  }

  // 滿世界撒裝飾:每個格子靠雜湊決定有沒有、是什麼(草叢居多、偶爾樹/石)。只畫視野內的。
  function drawScenery(camX, camY) {
    const cell = 120;
    const tx0 = Math.floor(camX / cell) - 1;
    const ty0 = Math.floor(camY / cell) - 1;
    const tx1 = Math.floor((camX + viewW) / cell) + 1;
    const ty1 = Math.floor((camY + viewH) / cell) + 1;
    for (let ty = ty0; ty <= ty1; ty++) {
      for (let tx = tx0; tx <= tx1; tx++) {
        const h = sceneryHash(tx, ty);
        if (h < 0.4) continue; // 這格留白
        const wx = tx * cell + sceneryHash(tx * 7 + 1, ty * 3) * cell;
        const wy = ty * cell + sceneryHash(tx * 3, ty * 7 + 1) * cell;
        if (wx < 0 || wy < 0 || wx > world.width || wy > world.height) continue;
        const sx = wx - camX;
        const sy = wy - camY;
        ctx.save();
        if (h > 0.93 && artOk("tree")) {
          const s = 30;
          ctx.globalAlpha = 0.9;
          ctx.drawImage(ART.tree, sx - s / 2, sy - s + 6, s, s);
        } else if (h > 0.86 && artOk("rock")) {
          const s = 24;
          ctx.globalAlpha = 0.9;
          ctx.drawImage(ART.rock, sx - s / 2, sy - s + 5, s, s);
        } else {
          drawGrassTuft(sx, sy, h);
        }
        ctx.restore();
      }
    }
  }

  // 把「繼承的工坊農莊」氛圍擺出來:工坊與墜毀飛船放在農地附近(固定世界座標)。
  // 飛船就是 GDD 的星際北極星勾子(玩家看得到、知道未來能飛出去)。
  function drawDecorations(camX, camY) {
    // 工坊／飛船是「你繼承的工坊農莊」氛圍，擺在自己那塊地附近。沒有自己的地
    // （訪客 / 尚未分到）就不擺。
    const f = myField();
    if (!f) return;
    const place = (img, wx, wy) => {
      if (!img || !img.complete || !img.naturalWidth) return;
      const dw = img.naturalWidth, dh = img.naturalHeight;
      const dx = Math.round(wx - camX - dw / 2);
      const dy = Math.round(wy - camY - dh); // 底部對齊 wy
      if (dx + dw < 0 || dy + dh < 0 || dx > viewW || dy > viewH) return;
      ctx.drawImage(img, dx, dy);
    };
    const fx = f.origin_x, fy = f.origin_y;
    place(ART.workshop, fx - 70, fy + 10);             // 工坊在田地左邊
    place(ART.ship, fx + f.cols * f.tile_size + 80, fy + 40); // 飛船在田地右邊
    place(ART.tree, fx - 30, fy - 40);
    place(ART.rock, fx + f.cols * f.tile_size + 20, fy - 20);
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

  // ---- 農地（Phase 0-G 種田起源 / 0-G-O1 per-player）----
  // 自己擁有的那塊地（owner === myId）；訪客或尚未分到地時為 null。
  function myField() {
    return myId ? fields.find((f) => f.owner === myId) : null;
  }

  // 玩家(權威座標 px,py)是否近到能照顧**指定的**那塊地：鏡像伺服器的 within_reach，
  // 用快照帶來的 f.reach 當同一個來源，前後端對「多近才算」不會各說各話。
  function withinFieldReach(f, px, py) {
    if (!f) return false;
    const right = f.origin_x + f.cols * f.tile_size;
    const bottom = f.origin_y + f.rows * f.tile_size;
    const nx = Math.max(f.origin_x, Math.min(px, right));
    const ny = Math.max(f.origin_y, Math.min(py, bottom));
    const dx = px - nx, dy = py - ny;
    return dx * dx + dy * dy <= f.reach * f.reach;
  }

  // 畫出世界上所有玩家的地塊（per-player）。只有自己那塊套用照顧距離回饋；
  // 別人的地照常畫、標上地主名，但點不動（伺服器也只接受對自己地的動作）。
  function drawField(camX, camY) {
    for (const f of fields) drawOnePlot(camX, camY, f);
  }

  // ---- 採集節點（Phase 1-A：樹/石/乙太礦）----
  // 每種節點的外觀(emoji + 底色),純程式畫,沒美術素材也讀得懂。
  const NODE_LOOK = {
    tree: { icon: "🌳", tint: "#2f5d34", act: "採木材" },
    rock: { icon: "🪨", tint: "#5b5f63", act: "採石頭" },
    ether_ore: { icon: "✨", tint: "#3a4a78", act: "採乙太礦" },
  };
  // 節點 kind → 現成 sprite 名（assets/*.png）。有圖就畫真的樹/石(不再是圓點 emoji);
  // 乙太礦沒專屬圖,留 emoji 發光。圖還沒載入也自動退回 emoji(artOk 把關)。
  const NODE_SPRITE = { tree: "tree", rock: "rock" };
  // 報讀器用的節點中文名（採空播報時念名字而非 emoji,對齊背包的 ITEM_NAME 作法）。
  const NODE_NAME = { tree: "樹", rock: "石礦", ether_ore: "乙太礦" };

  // 回傳「玩家搆得到(GATHER_REACH 內)的最近可採節點」,沒有就 null。採集判定與伺服器一致。
  function nearestHarvestable(me) {
    if (!me) return null;
    let best = null;
    let bestD = GATHER_REACH * GATHER_REACH;
    for (const n of nodes) {
      if (!n.harvestable) continue;
      const dx = n.x - me.x;
      const dy = n.y - me.y;
      const d = dx * dx + dy * dy;
      if (d <= bestD) {
        bestD = d;
        best = n;
      }
    }
    return best;
  }

  // 回傳「玩家搆得到、但已採空(harvestable=false)的最近節點」,沒有就 null。
  // 用來在玩家站在採空節點旁時標「已採空」,免得按鍵沒反應卻不知為何(切片 1 無重生,採空即長期變淡)。
  function nearestDepleted(me) {
    if (!me) return null;
    let best = null;
    let bestD = GATHER_REACH * GATHER_REACH;
    for (const n of nodes) {
      if (n.harvestable) continue;
      const dx = n.x - me.x;
      const dy = n.y - me.y;
      const d = dx * dx + dy * dy;
      if (d <= bestD) {
        bestD = d;
        best = n;
      }
    }
    return best;
  }

  // 畫世界上的採集節點。可採的亮、採空(重生中)的壓暗;玩家搆得到的那顆描一圈亮環提示「可採」。
  function drawNodes(camX, camY) {
    const me = myId ? players.get(myId) : null;
    const reachable = nearestHarvestable(me);
    // 沒有可採的在搆得到範圍內時,才標最近的採空節點為「已採空」——有可採的就不打擾,
    // 引導玩家去採那顆。純讀快照,不在前端判規則。
    const depleted = reachable ? null : nearestDepleted(me);
    for (const n of nodes) {
      const sx = n.x - camX;
      const sy = n.y - camY;
      if (sx < -40 || sy < -40 || sx > viewW + 40 || sy > viewH + 40) continue;
      const look = NODE_LOOK[n.kind] || { icon: "❔", tint: "#555" };
      ctx.save();
      ctx.globalAlpha = n.harvestable ? 1 : 0.4; // 採空的畫淡(重生中)
      const spriteName = NODE_SPRITE[n.kind];
      if (spriteName && artOk(spriteName)) {
        // 有對應 sprite(樹/石)→ 畫真的 pixel art,底部對齊節點位置、放大些好看清。
        const img = ART[spriteName];
        const s = 44;
        ctx.drawImage(img, sx - s / 2, sy - s + 8, s, s);
      } else {
        // 沒 sprite(乙太礦)或圖還沒載入 → 退回圓盤 + emoji。
        ctx.beginPath();
        ctx.arc(sx, sy, 16, 0, Math.PI * 2);
        ctx.fillStyle = look.tint;
        ctx.fill();
        ctx.font = "20px system-ui, sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(look.icon, sx, sy + 1);
      }
      // 玩家搆得到的那顆:描一圈黃環,提示「走到了、可以採」。
      if (n === reachable) {
        ctx.strokeStyle = "rgba(255,210,74,0.9)";
        ctx.lineWidth = 2.5;
        ctx.beginPath();
        ctx.arc(sx, sy, 19, 0, Math.PI * 2);
        ctx.stroke();
        // 光描黃環,沒採過的玩家仍不知道那是要按鍵的——在節點正上方標出「採什麼」,
        // 第一次再多帶一行「怎麼按」(鏡像農地腳下格提示)。採過一次(gatheredOnce)
        // 就只留動作詞,不長期擾人。動作詞純讀節點 kind,不在前端判規則。
        ctx.font = "12px system-ui, sans-serif";
        ctx.textAlign = "center";
        const ty = sy - 24; // 貼在圖示上方
        ctx.lineWidth = 3; // 深色描邊讓字在任何地表上都讀得清
        ctx.strokeStyle = "rgba(0,0,0,0.6)";
        ctx.strokeText(look.act, sx, ty);
        ctx.fillStyle = "rgba(255,235,180,0.95)";
        ctx.fillText(look.act, sx, ty);
        if (!gatheredOnce) {
          ctx.font = "10px system-ui, sans-serif";
          const hint = "按空白鍵或點一下";
          const hy = ty - 13; // 疊在動作詞正上方
          ctx.strokeText(hint, sx, hy);
          ctx.fillStyle = "rgba(255,235,180,0.8)";
          ctx.fillText(hint, sx, hy);
        }
      } else if (n === depleted) {
        // 玩家就站在採空節點旁、附近又沒有可採的:標一行淡「已採空」,解釋為何按鍵沒反應,
        // 別讓人對著按不動的節點乾按。alpha 已壓暗,字提到 1 才讀得清。
        ctx.globalAlpha = 1;
        ctx.font = "11px system-ui, sans-serif";
        ctx.textAlign = "center";
        const ty = sy - 24;
        ctx.lineWidth = 3;
        ctx.strokeStyle = "rgba(0,0,0,0.6)";
        ctx.strokeText("已採空", sx, ty);
        ctx.fillStyle = "rgba(220,220,220,0.85)";
        ctx.fillText("已採空", sx, ty);
      }
      ctx.restore();
    }
  }

  // 走進「最近可採節點」範圍時,播一句給報讀器玩家——他們看不到黃環/動作詞/「按鍵採集」那組
  // 視覺提示,沒這句就只能到處亂按鍵才知道哪裡採得到。鍵(kind@x,y)變了才播:離開再進來會重播、
  // 同顆站著不重複擾人。和視覺一樣讀 nearestHarvestable(GATHER_REACH 內最近可採),不嵌規則。
  function maybeAnnounceReachable(me) {
    const n = nearestHarvestable(me);
    const key = n ? `${n.kind}@${n.x},${n.y}` : null;
    if (key === lastReachableKey) return;
    lastReachableKey = key;
    // 鏡像視覺的 gatheredOnce 收斂(見 drawNodes):採過一次的玩家已懂怎麼按,走近只報「可採」,
    // 不再每次都唸一整句操作教學,免得對熟手囉嗦;新手(還沒採過)才帶「按空白鍵或點一下」。
    if (n) announce(gatheredOnce
      ? `走到${NODE_NAME[n.kind] || "資源"}旁,可採`
      : `走到${NODE_NAME[n.kind] || "資源"}旁,可採——按空白鍵或點一下採集`);
  }

  // ---- 敵人（戰鬥 1-F）----
  const ENEMY_LOOK = {
    scrap_drone: { icon: "🤖", tint: "#6b4a3a" },
    ether_wisp: { icon: "👻", tint: "#46407a" },
  };
  // 畫世界上的敵人 + 血條。被打倒(重生中)的畫很淡;走近會自動開打(伺服器每秒結算,前端只呈現)。
  function drawEnemies(camX, camY) {
    const fxNow = performance.now();
    for (let i = 0; i < enemies.length; i++) {
      const e = enemies[i];
      const sx = e.x - camX;
      const sy = e.y - camY;
      if (sx < -40 || sy < -40 || sx > viewW + 40 || sy > viewH + 40) continue;
      const look = ENEMY_LOOK[e.kind] || { icon: "❔", tint: "#555" };
      // 受擊／被打倒一閃:t 1→0 隨剩餘時間淡出。被打倒(lethal)閃得更白更大,當作擊倒確認。
      const fx = enemyFx[i];
      const fxT = fx && fxNow < fx.until
        ? Math.max(0, Math.min(1, (fx.until - fxNow) / (fx.lethal ? 480 : 280)))
        : 0;
      ctx.save();
      if (!e.alive) ctx.globalAlpha = 0.25; // 被打倒、重生中
      ctx.beginPath();
      ctx.arc(sx, sy, 16, 0, Math.PI * 2);
      ctx.fillStyle = look.tint;
      ctx.fill();
      ctx.font = "20px system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(look.icon, sx, sy + 1);
      // 受擊閃光疊在敵人身上:一圈往外擴張淡出的亮環(被打倒時更白更大),純表現。
      // reduceMotion 時不做擴張動畫,只畫一圈靜態亮邊——仍傳遞「打中了」的資訊但不晃動。
      if (fxT > 0) {
        const base = fx.lethal ? 20 : 17;
        const r = reduceMotion ? base : base + (1 - fxT) * (fx.lethal ? 16 : 10);
        ctx.globalAlpha = (e.alive ? 1 : 0.25) * fxT;
        ctx.lineWidth = fx.lethal ? 3 : 2;
        ctx.strokeStyle = fx.lethal ? "#fff" : "#ffd9a0";
        ctx.beginPath();
        ctx.arc(sx, sy, r, 0, Math.PI * 2);
        ctx.stroke();
        ctx.globalAlpha = e.alive ? 1 : 0.25;
      }
      // 血條:活著且不滿血才畫。
      if (e.alive && e.hp < e.max_hp) {
        const bw = 28;
        const bx = sx - bw / 2;
        const by = sy - 22;
        ctx.fillStyle = "rgba(0,0,0,0.5)";
        ctx.fillRect(bx - 1, by - 1, bw + 2, 6);
        ctx.fillStyle = "#d65a5a";
        ctx.fillRect(bx, by, bw * (e.hp / e.max_hp), 4);
      }
      ctx.restore();
    }
  }

  // 生命 HUD:顯示「生命：hp/max」,低血/被打趴變紅。
  function updateHpHud(hp, maxHp) {
    const el = document.getElementById("hudHp");
    if (!el) return;
    if (hp <= 0) {
      el.textContent = "💀 被打趴,休息復原中…";
      el.style.color = "#f88";
    } else {
      el.textContent = `生命：${hp}/${maxHp}`;
      el.style.color = hp < maxHp * 0.35 ? "#f88" : "";
    }
    el.setAttribute("aria-label", el.textContent);
  }

  // 背包 HUD:把 [{item,qty}] 顯示成「🪵 N　🪨 N　✨ N」。空背包就只留標頭。
  // pickaxe 是合成產物(1-C/1-D),會隨背包快照回來;補進這三張表,讓合成出的鎬子在
  // 背包明細/飄字/報讀器都跟採集三資源一樣有 emoji、中文名與色,不掉回裸字串。
  const ITEM_LOOK = { wood: "🪵", stone: "🪨", ether: "✨", pickaxe: "⛏️" };
  // 報讀器用的品項中文名（emoji 對報讀器無意義,播報時念名字而非圖示）。
  const ITEM_NAME = { wood: "木材", stone: "石頭", ether: "乙太", pickaxe: "鎬子" };
  // 採集飄字的品項色（與節點底色同調,讓「採到什麼」一眼可分）。
  const ITEM_FLOAT_COLOR = { wood: "150,210,140", stone: "200,205,210", ether: "255,210,74", pickaxe: "210,180,120" };
  // 合成配方表(前端呈現用,與伺服器 crafting.rs 的 RECIPES 對齊):產物 ← 素材。
  // 只用來畫面板與「夠不夠料」的提示反灰——真正查表扣料一律由伺服器說了算(規則只在伺服器)。
  // 接線後 client 送 { type:"craft", recipe_id:id },產物隨既有背包快照回來,零契約變更。
  const CRAFT_RECIPES = [
    { id: "pickaxe", out: "pickaxe", outQty: 1, inputs: [["wood", 3], ["stone", 2]] },
  ];
  // 擴地價格（與伺服器 src/economy.rs 對齊;規則只在伺服器,前端只拿來顯示與反灰提示）：
  // 基準 10 乙太、逐格線性漲（第 n+1 格 = 10×(n+1)）、一塊地最多擴 12 格。
  const EXPANSION_BASE_COST = 10;
  const MAX_EXPANSIONS = 12;
  // 已購 owned 格時,下一格要多少乙太。對齊 economy::expansion_cost——前端只算來顯示,
  // 真正扣款/開格仍由伺服器查餘額決定。
  const expansionCost = (owned) => EXPANSION_BASE_COST * (owned + 1);
  function updateBagHud(inv) {
    // 收合背包面板由三塊組成:常駐標題列(toggle，掛無障礙標籤)、收起時也看得到的摘要計數
    // (summary)、展開才顯示的明細列(body)。沿用採集飄字/播報同一套 ITEM_LOOK / ITEM_NAME。
    const toggle = document.getElementById("bagToggle");
    const summary = document.getElementById("bagSummary");
    const body = document.getElementById("bagBody");
    if (!toggle || !summary || !body) return;
    if (!inv || inv.length === 0) {
      summary.textContent = "：空";
      body.textContent = "背包是空的——走到 🌳 / 🪨 / ✨ 旁採集吧";
      // 無障礙標籤掛在常駐可見的標題鈕上(收起時 body 被 display:none、報讀器讀不到),
      // 空背包也標一句,讓還沒採過的玩家知道這顆鈕是背包。title 為滑鼠停留的原生提示。
      toggle.setAttribute("aria-label", "背包：空");
      toggle.setAttribute("title", "背包：空");
      return;
    }
    // 標題列摘要用 emoji（看得到的玩家收起時也一眼分品項、看到存量,沿用舊那行的快速一瞥）。
    summary.textContent =
      "　" + inv.map((s) => `${ITEM_LOOK[s.item] || s.item} ${s.qty}`).join("　");
    // 展開的明細:每項素材一行 emoji + 中文名 + 數量。素材從採集/打怪/農地三方湧入、堆積得快,
    // 明細列讓「手上有什麼、各多少」清楚可讀(item 是伺服器列舉字串、非玩家文字,無注入風險)。
    body.innerHTML = inv
      .map((s) => {
        const icon = ITEM_LOOK[s.item] || "";
        const name = ITEM_NAME[s.item] || s.item;
        return `<div class="bag-row"><span class="bag-ico">${icon}</span>${name}<span class="bag-qty">×${s.qty}</span></div>`;
      })
      .join("");
    // emoji 對報讀器無意義(會亂念或跳過),把中文品項名同步成標題鈕的 aria-label,讓盲人玩家
    // 「採到那瞬間」聽過播報後,之後想查背包現況時也讀得出來。title 給滑鼠玩家對稱的那一半。
    // 延續日夜/連線/採集的背包無障礙弧線。
    const label =
      "背包：" + inv.map((s) => `${ITEM_NAME[s.item] || s.item} ${s.qty}`).join("、");
    toggle.setAttribute("aria-label", label);
    toggle.setAttribute("title", label);
  }

  // 合成台:把背包快照(品項→數量)對照配方表,畫出每條「產物 ← 素材」與一顆合成鈕。
  // 缺料的素材標紅、合成鈕反灰停用——這只是前端提示,讓玩家一眼知道差什麼;扣不扣得成
  // 仍由伺服器查表決定。點鈕送 { type:"craft", recipe_id:id } 意圖,產物隨既有背包快照回來。
  let lastCraftSig = null; // 上次重建用的「相關背包」簽章——沒變就不重建,保住焦點與效能
  function updateCraftPanel(inv) {
    const summary = document.getElementById("craftSummary");
    const body = document.getElementById("craftBody");
    const toggle = document.getElementById("craftToggle");
    if (!summary || !body || !toggle) return;
    const have = new Map((inv || []).map((s) => [s.item, s.qty]));
    // 世界快照每個 tick 都來,但合成台只取決於「配方用到的素材數量」。沒變就提早返回:
    // 否則每拍 innerHTML 重建會把鍵盤/報讀器停在「合成」鈕上的焦點打掉、手機也白耗電。
    const sig = CRAFT_RECIPES.map((r) =>
      r.inputs.map(([item, qty]) => have.get(item) || 0).join(",")
    ).join("|");
    if (sig === lastCraftSig) return;
    lastCraftSig = sig;
    let craftable = 0; // 此刻夠料的配方數,寫進標題列摘要(收著時也一眼知道能不能合)
    body.innerHTML = "";
    for (const r of CRAFT_RECIPES) {
      const ok = r.inputs.every(([item, qty]) => (have.get(item) || 0) >= qty);
      if (ok) craftable++;
      const row = document.createElement("div");
      row.className = "craft-row";
      // 素材描述:每項「emoji×需求」,不夠的標紅(.lack)。item 是伺服器列舉字串、非玩家文字,無注入風險。
      const needs = r.inputs
        .map(([item, qty]) => {
          const lack = (have.get(item) || 0) < qty;
          const ico = ITEM_LOOK[item] || item;
          return `<span class="craft-need${lack ? " lack" : ""}">${ico}×${qty}</span>`;
        })
        .join(" ");
      const outIco = ITEM_LOOK[r.out] || r.out;
      const outName = ITEM_NAME[r.out] || r.out;
      const desc = document.createElement("div");
      desc.className = "craft-desc";
      desc.innerHTML = `<span class="craft-out">${outIco} ${outName}</span> ← ${needs}`;
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "craft-btn";
      btn.textContent = "合成";
      btn.disabled = !ok;
      // 報讀器把整條配方念清楚:夠料念「合成 鎬子,需要 木材×3、石頭×2」,缺料補「(素材不足)」。
      const needLabel = r.inputs
        .map(([item, qty]) => `${ITEM_NAME[item] || item}×${qty}`)
        .join("、");
      btn.setAttribute(
        "aria-label",
        `合成 ${outName},需要 ${needLabel}${ok ? "" : "（素材不足）"}`
      );
      btn.title = ok ? `合成 ${outName}` : "素材不足";
      btn.addEventListener("click", () => {
        if (btn.disabled) return;
        // 只送意圖:伺服器查配方扣料、產物隨既有背包快照回來(規則只在伺服器,前端不自行加道具)。
        // 欄位用 recipe_id 對齊伺服器 ClientMsg::Craft{recipe_id}——serde 的 rename_all="snake_case"
        // 只改 variant 名、不串到 struct 欄位(既有 name/species/text 皆原 Rust 名),故欄位即 recipe_id。
        try { ws.send(JSON.stringify({ type: "craft", recipe_id: r.id })); } catch {}
        announce(`合成 ${outName}`);
      });
      row.appendChild(desc);
      row.appendChild(btn);
      body.appendChild(row);
    }
    summary.textContent = craftable > 0 ? `：${craftable} 可合成` : "";
    // 標題鈕的無障礙標籤同步可合成數(收起時 body 被 display:none、報讀器讀不到)。
    const label = craftable > 0 ? `合成台：${craftable} 項可合成` : "合成台：素材不足";
    toggle.setAttribute("aria-label", label);
    toggle.setAttribute("title", label);
  }

  // 擴地面板:依「我的乙太」與「已購擴張格數」算下一格價,畫出價格提示與一顆「擴地」鈕。
  // owned 走快照未來欄位 expansions(伺服器接線後才有,沒有就當 0 防呆);乙太夠才亮鈕、
  // 達上限改顯示「已達上限」並反灰。點鈕只送 buy_expansion 意圖,伺服器查餘額扣乙太、農地
  // 多開一格,新地塊隨既有 fields 快照回來(零契約變更);夠不夠的反灰只是前端提示,扣款/
  // 開格規則仍只在伺服器(權威),前端不自行改地塊。me 缺欄位一律防呆,接線落地即生效。
  let lastExpandSig = null; // 上次重建用的「乙太|已購格數」簽章——沒變就不重建,保住焦點與效能
  function updateExpandPanel(me) {
    const summary = document.getElementById("expandSummary");
    const body = document.getElementById("expandBody");
    const toggle = document.getElementById("expandToggle");
    if (!summary || !body || !toggle) return;
    const ether = (me && me.ether) || 0;
    const owned = (me && me.expansions) || 0; // 防呆:伺服器還沒接擴地 → 視為一格都還沒買
    // 同合成台:快照每拍都來,但擴地面板只取決於乙太與已購格數。沒變就提早返回,
    // 免得每拍重建把停在「擴地」鈕上的焦點打掉、手機白耗電。
    const sig = `${ether}|${owned}`;
    if (sig === lastExpandSig) return;
    lastExpandSig = sig;
    const atMax = owned >= MAX_EXPANSIONS;
    const cost = expansionCost(owned);
    const canBuy = !atMax && ether >= cost;
    body.innerHTML = "";

    const row = document.createElement("div");
    row.className = "expand-row";
    const desc = document.createElement("div");
    desc.className = "expand-desc";
    if (atMax) {
      desc.textContent = "農地已擴到最大";
    } else {
      const lack = ether < cost;
      // item 字串非玩家輸入,無注入風險;價格不足標紅讓玩家一眼知道還差多少乙太。
      desc.innerHTML = `下一格 <span class="expand-cost${lack ? " lack" : ""}">✨ ${cost}</span>`;
    }
    row.appendChild(desc);

    if (!atMax) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "expand-btn";
      btn.textContent = "擴地";
      btn.disabled = !canBuy;
      btn.setAttribute(
        "aria-label",
        `花 ${cost} 乙太擴一格農地${canBuy ? "" : "（乙太不足）"}`
      );
      btn.title = canBuy ? `花 ${cost} 乙太擴一格` : "乙太不足";
      btn.addEventListener("click", () => {
        if (btn.disabled) return;
        // 只送意圖:伺服器查餘額扣乙太、農地多開一格,結果隨既有快照回來(規則只在伺服器)。
        try { ws.send(JSON.stringify({ type: "buy_expansion" })); } catch {}
        announce(`花 ${cost} 乙太擴地`);
      });
      row.appendChild(btn);
    }
    body.appendChild(row);

    // 標題列摘要 + 無障礙標籤(收起時 body 被 display:none、報讀器讀不到,摘要掛常駐標題鈕上)。
    summary.textContent = atMax ? "：已滿" : canBuy ? "：可擴" : "";
    const label = atMax
      ? "擴地：農地已擴到最大"
      : canBuy
        ? `擴地：可花 ${cost} 乙太擴一格`
        : `擴地：下一格 ${cost} 乙太（不足）`;
    toggle.setAttribute("aria-label", label);
    toggle.setAttribute("title", label);
  }

  // 依伺服器廣播的每格 state/dry 畫出一塊地的耕地與作物階段。
  function drawOnePlot(camX, camY, f) {
    if (!f || !f.cells) return;
    const ts = f.tile_size;
    const mine = f.owner === myId;
    // 自己離自己的農地太遠時把整塊地畫淡，並提示走近——讓伺服器「太遠就拒絕照顧」
    // 不再表現成「點了沒反應像壞掉」。別人的地不套這個（本就不能由你照顧）。
    const me = myId ? players.get(myId) : null;
    const reachable = mine ? (me ? withinFieldReach(f, me.x, me.y) : true) : true;
    const fx = f.origin_x - camX;
    const fy = f.origin_y - camY;
    const fw = f.cols * ts;
    const fh = f.rows * ts;
    // 整塊在畫面外就略過（多塊地時省繪製）。右/下界用 viewW/viewH（邏輯像素）而非
    // canvas.width/height（DPR 放大後的實體像素緩衝）：fx/fy 是邏輯像素,DPR>1 的手機/平板
    // 上若拿實體像素當界,右/下邊的界會大上一個 DPR 倍、捲出畫面的地塊culling 不掉、白跑
    // row×col 繪製迴圈。對齊本檔「繪製一律用 viewW/viewH」的約定（見上方畫布尺寸註解）。
    if (fx + fw < -40 || fy + fh < -40 || fx > viewW + 40 || fy > viewH + 40) return;

    ctx.save();
    if (!reachable) ctx.globalAlpha = 0.55;
    else if (!mine) ctx.globalAlpha = 0.82; // 別人的地稍微壓一點，凸顯自己的

    // 整塊田的「土底」墊一層深褐色,讓它跟草地一眼分得開。
    ctx.fillStyle = "#3a2818";
    ctx.fillRect(fx - 2, fy - 2, fw + 4, fh + 4);

    for (let row = 0; row < f.rows; row++) {
      for (let col = 0; col < f.cols; col++) {
        const cell = f.cells[row * f.cols + col];
        const sx = f.origin_x + col * ts - camX;
        const sy = f.origin_y + row * ts - camY;
        if (sx + ts < 0 || sy + ts < 0 || sx > viewW || sy > viewH) continue;
        drawTile(sx, sy, ts, cell);
      }
    }

    // hover 高亮 + 腳下格指示:只在「自己的地」且夠近時畫——互動只作用在自己的地,
    // 別人的地點不動,自然不需要這些「點下去會作用在這格」的目標回饋。
    if (mine && reachable) {
      // 桌面 hover 高亮:游標所指的田格描一圈亮框 + 淡填。手機無 hover 自然不畫。
      if (hoverScreen) {
        const t = fieldTileAtScreen(hoverScreen.x, hoverScreen.y);
        if (t) {
          const hx = f.origin_x + t.col * ts - camX;
          const hy = f.origin_y + t.row * ts - camY;
          ctx.fillStyle = "rgba(255,210,74,0.12)";
          ctx.fillRect(hx + 1, hy + 1, ts - 2, ts - 2);
          ctx.strokeStyle = "rgba(255,210,74,0.9)";
          ctx.lineWidth = 2;
          ctx.strokeRect(hx + 1, hy + 1, ts - 2, ts - 2);
        }
      }
      // 腳下格指示:踩在某田格上時描一圈虛線框,給用「空白鍵 / E 採腳下格」的鍵盤/觸控
      // 玩家同等的目標回饋(桌面靠滑鼠 hover 亮框)。虛線 + 較低透明度跟 hover 實線框區分。
      if (me) {
        const fcol = Math.floor((me.x - f.origin_x) / ts);
        const frow = Math.floor((me.y - f.origin_y) / ts);
        if (fcol >= 0 && frow >= 0 && fcol < f.cols && frow < f.rows) {
          const ex = f.origin_x + fcol * ts - camX;
          const ey = f.origin_y + frow * ts - camY;
          ctx.save();
          ctx.strokeStyle = "rgba(255,210,74,0.55)";
          ctx.lineWidth = 1.5;
          ctx.setLineDash([4, 3]); // 虛線:跟滑鼠 hover 的實線亮框一眼分得開
          ctx.strokeRect(ex + 2, ey + 2, ts - 4, ts - 4);
          ctx.restore(); // 還原 lineDash,免得漏進後面黃銅框/柵欄的實線繪製
          // 腳下格動作提示:在虛線框正上方標出「現在按採集鍵/鈕會做什麼」(翻土/播種/澆水/
          // 收成),讓玩家不必先試一次才知道腳下這格按下去的結果——新手最常卡在「站在田上卻
          // 不知道下一步」。動作詞鏡像 field.rs interact() 只讀快照 state:規則仍在伺服器,
          // 這裡純把權威狀態翻成一個可讀詞,不自己判斷能不能做。
          const act = tendActionLabel(f.cells[frow * f.cols + fcol]);
          if (act) {
            ctx.save();
            ctx.font = "12px system-ui, sans-serif";
            ctx.textAlign = "center";
            const tx = ex + ts / 2;
            const ty = ey - 3; // 貼在虛線框正上方
            ctx.lineWidth = 3; // 深色描邊讓字在土黃/作物任何底色上都讀得清
            ctx.strokeStyle = "rgba(0,0,0,0.6)";
            ctx.strokeText(act, tx, ty);
            ctx.fillStyle = "rgba(255,235,180,0.95)";
            ctx.fillText(act, tx, ty);
            // 新手第一次:動作詞上方再補一行「怎麼按」——光標出動詞還不夠,沒按過的玩家不知道
            // 要按空白鍵或點一下才會發生。照顧過一次(markTendedOnce)就不再顯示,不長期擾人。
            if (!tendedOnce) {
              ctx.font = "10px system-ui, sans-serif";
              const hint = "按空白鍵或點一下";
              const hy = ty - 13; // 疊在動作詞正上方,兩行一起貼在田格上緣
              ctx.strokeText(hint, tx, hy);
              ctx.fillStyle = "rgba(255,235,180,0.8)";
              ctx.fillText(hint, tx, hy);
            }
            ctx.restore();
          }
        }
      }
    }

    // 周圍畫一圈邊框：自己的黃銅亮框、別人的暗一點，一眼分得出哪塊是自己的。
    ctx.strokeStyle = mine ? "#c9a24b" : "#8a7340";
    ctx.lineWidth = 3;
    ctx.strokeRect(fx - 2, fy - 2, fw + 4, fh + 4);

    // 木柵欄:沿田邊立等距木樁 + 兩條橫桿,讓田看起來像「圈起來的農莊」。純程式畫。
    drawFence(fx, fy, fw, fh);

    // 田地名字標籤：自己的標「你的乙太田」，別人的標地主名。
    const owner = players.get(f.owner);
    const label = mine ? "你的乙太田 🌱" : `${owner ? owner.name : "拓荒者"} 的乙太田`;
    ctx.fillStyle = mine ? "rgba(232,224,207,0.9)" : "rgba(232,224,207,0.7)";
    ctx.font = "13px system-ui, sans-serif";
    ctx.textAlign = "center";
    ctx.fillText(label, fx + fw / 2, fy - 8);

    ctx.restore();

    if (mine && !reachable) {
      ctx.fillStyle = "rgba(232,224,207,0.85)";
      ctx.font = "12px system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.fillText("(走近一點才能照顧)", fx + fw / 2, fy + fh + 18);
    }
  }

  // 腳下這格「按採集鍵會發生什麼」的可讀動作詞。鏡像 field.rs 的 interact():
  // 自然地→翻土、空土→播種、成熟→收成、其餘已種未熟→澆水。只讀權威快照的 state,
  // 不在前端決定能不能做(那是伺服器的事),純把狀態翻成一個給玩家看的詞。
  function tendActionLabel(cell) {
    if (!cell) return null;
    switch (cell.state) {
      case 0: return "翻土";
      case 1: return "播種";
      case 4: return "收成";
      default: return "澆水"; // 2=種子 3=發芽:未熟作物,interact 一律當澆水
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

  // 生長階段小標記:在格子右上角畫 1~3 顆對比色圓點,顆數 = 階段(種子1 / 發芽2 / 成熟3)。
  // 顆數本身就能分辨,不必只靠色相——回應玩家「種田狀態顏色有些好近,不好分辨」。深描邊讓
  // 它在任何底色(sprite 或程式 fallback)上都看得清。純表現層:階段一律讀權威伺服器快照的
  // cell.state,不嵌任何遊戲規則,將來換 WebXR renderer 也是各自照同一份 state 自己畫。
  function drawStagePips(sx, sy, ts, cell) {
    // 三階段色相刻意拉開、避免擠在同一片暖黃(舊版種子土黃 #d8b25a 與成熟亮金 #ffd24a
    // 幾乎同色相,玩家反映「好近、不好分辨」)。改成 深棕褐→鮮綠→亮金 的明確漸進:
    // 種子壓低明度成土裡的棕,發芽是飽和綠,成熟才亮金,三者明度+色相都分得開。
    // 避開藍色——藍已被「缺水/澆水」指示佔用,免得跟剛播種仍缺水的格子撞色。
    const STAGE = {
      2: { n: 1, color: "#a8662e" }, // 種子:深棕褐(壓暗,讀作土裡的種子)
      3: { n: 2, color: "#5ad94f" }, // 發芽:鮮綠
      4: { n: 3, color: "#ffd24a" }, // 成熟:亮金
    };
    const r = Math.max(2, ts * 0.05);
    const gap = r * 2 + 2;
    const x0 = sx + ts - 4 - r; // 靠右上角,由右往左排,不蓋掉左上常用的角落視覺
    const y0 = sy + 4 + r;
    ctx.lineWidth = 1;
    ctx.strokeStyle = "rgba(0,0,0,0.55)";

    // state 1=翻好待播的空土:畫一顆「空心環」。先前只標 2~4 的作物階段,翻好的空土
    // 與未翻的自然地(state 0,無標記)在兩路徑都是相近棕、難分——玩家反映種田狀態
    // 顏色太近。空心環讀作「翻好的空播種位」,跟生地(無環)和已播的填色點都分得開,
    // 也接上 無環→空環→1棕→2綠→3金 的清楚進程。用較粗的 stroke 畫環,中心透出底下
    // 背景(art sprite 或 fallback 土色皆通用),不必猜底色去填中心。
    if (cell.state === 1) {
      ctx.beginPath();
      ctx.arc(x0, y0, r * 0.7, 0, Math.PI * 2);
      ctx.lineWidth = Math.max(1.5, r * 0.45);
      ctx.strokeStyle = "rgba(0,0,0,0.55)"; // 先描深底,環在亮土上仍讀得到
      ctx.stroke();
      ctx.strokeStyle = "rgba(232,224,207,0.95)"; // 淺米白環,讀作翻鬆待播的空位
      ctx.lineWidth = Math.max(1, r * 0.32);
      ctx.stroke();
      return;
    }

    const s = STAGE[cell.state];
    if (!s) return;
    for (let i = 0; i < s.n; i++) {
      ctx.beginPath();
      ctx.arc(x0 - i * gap, y0, r, 0, Math.PI * 2);
      ctx.fillStyle = s.color;
      ctx.fill();
      ctx.stroke();
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
      drawStagePips(sx, sy, ts, cell);
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
    drawStagePips(sx, sy, ts, cell);
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
    // 先看是不是點到小地圖收合鈕(純 UI 切換,不需連線、也不該被當成農作意圖)。
    if (minimapToggleHit(clientX, clientY)) return;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const me = myId ? players.get(myId) : null;
    const f = myField();
    const now = Date.now();
    // 先判採集:點到「玩家搆得到的可採節點」就送採集——要擺在「不在自己農地就早退」的守衛
    // 之前(採集發生在離田的節點旁)。伺服器一律用玩家權威位置採最近的,客戶端只判「點到節點」當觸發。
    {
      const gn = nearestHarvestable(me);
      if (gn) {
        const rect = canvas.getBoundingClientRect();
        const twx = clientX - rect.left + lastCam.x;
        const twy = clientY - rect.top + lastCam.y;
        const dx = gn.x - twx;
        const dy = gn.y - twy;
        if (dx * dx + dy * dy <= 34 * 34) {
          ws.send(JSON.stringify({ type: "gather" }));
          markGatheredOnce();
          spawnTapFlash(gn.x, gn.y);
          return;
        }
      }
    }
    // 沒有自己的地（訪客 / 尚未分到）：伺服器只接受對自己地的動作，先給回饋、不白送一則。
    if (me && !f) {
      if (now - lastReachHint > 2500) {
        addChat("系統", "登入後就有自己的乙太田可以照顧哦。");
        lastReachHint = now;
      }
      return;
    }
    // 自己離自己的農地太遠：伺服器一律拒絕，這裡先給回饋、不白送一則。
    if (me && f && !withinFieldReach(f, me.x, me.y)) {
      if (now - lastReachHint > 2500) {
        addChat("系統", "走近你的農地才能照顧作物哦。");
        lastReachHint = now;
      }
      return;
    }
    const rect = canvas.getBoundingClientRect();
    const wx = clientX - rect.left + lastCam.x;
    const wy = clientY - rect.top + lastCam.y;
    ws.send(JSON.stringify({ type: "farm", x: wx, y: wy }));
    markTendedOnce(); // 照顧過一次就不再顯示「怎麼按」新手提示
    spawnTapFlash(wx, wy); // 純確認回饋:這一下已送出
  }
  // 純鍵盤/無滑鼠玩家:對「自己腳下這格」送農作意圖(空白鍵 / E / F)。玩家回饋
  // 「不一定有滑鼠」——走得動卻點不到田格。這裡只挑目標格(自己的位置)送原始世界
  // 座標,做什麼仍由權威伺服器決定,不在客戶端判規則。
  function farmAtPlayer() {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const me = myId ? players.get(myId) : null;
    if (!me) return;
    // 先判採集:站在搆得到的可採節點旁,按動作鍵就採集。
    if (nearestHarvestable(me)) {
      ws.send(JSON.stringify({ type: "gather" }));
      markGatheredOnce();
      spawnTapFlash(me.x, me.y);
      return;
    }
    // 沒有可採的:若就站在採空節點旁,鏡像視覺的「已採空」標,播一句給報讀器玩家。
    // 看不到那行字的玩家原本只會聽到下面誤導的「走進農地」提示(或按了全無回饋),
    // 卻對著按不動的節點乾按。延續採集播報/背包 aria-label 的無障礙弧線,只在真的
    // 站在採空節點旁時報,並用 lastReachHint 同套節流避免連按重複朗讀。
    const depleted = nearestDepleted(me);
    if (depleted) {
      const now = Date.now();
      if (now - lastReachHint > 2500) {
        announce(`這裡的${NODE_NAME[depleted.kind] || "資源"}已採空`);
        lastReachHint = now;
      }
      return;
    }
    const f = myField();
    if (f && !withinFieldReach(f, me.x, me.y)) {
      const now = Date.now();
      if (now - lastReachHint > 2500) {
        addChat("系統", "走進農地再按採集鍵照顧作物哦。");
        lastReachHint = now;
      }
      return;
    }
    ws.send(JSON.stringify({ type: "farm", x: me.x, y: me.y }));
    markTendedOnce(); // 照顧過一次就不再顯示「怎麼按」新手提示
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
    const f = myField();
    if (!f) return null;
    const rect = canvas.getBoundingClientRect();
    const wx = sx - rect.left + lastCam.x;
    const wy = sy - rect.top + lastCam.y;
    const col = Math.floor((wx - f.origin_x) / f.tile_size);
    const row = Math.floor((wy - f.origin_y) / f.tile_size);
    if (col < 0 || row < 0 || col >= f.cols || row >= f.rows) return null;
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
    const collapsed = document.getElementById("chat").classList.contains("chat-collapsed");
    if (collapsed) {
      bumpChatUnread();
      // 收合時 #chatLog 是 display:none,其 aria-live 區被報讀器忽略——收著聊天的報讀器玩家
      // 聽不到任何人發言(那顆 (N) 未讀數是純視覺)。把真人發言(who 非「系統」)改走一直在的
      // #srStatus 播報區,讓收著也聽得到社交訊息。連線中斷/接回等「系統」訊息本就各自呼叫
      // announce()(見 onclose/welcome),這裡略過以免雙重朗讀。延續 srStatus 的無障礙弧線。
      if (who !== "系統") announce(`${who}：${text}`);
    }
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
  // Esc 退出聊天:用 Enter 開了聊天又改變主意時,給一條鍵盤退路退回操控(對齊建議箱
  // 的「Esc 關閉」慣例)。先前唯一離開輸入的方式是硬送出或滑鼠點別處——鍵盤玩家會卡住。
  // 清掉打到一半的內容(取消即丟棄,不誤送)並 blur,焦點還給遊戲、移動鍵恢復作用。
  document.getElementById("chatText").addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.target.value = "";
      e.target.blur();
    }
  });

  // ---- 建議箱 ----
  const modal = document.getElementById("suggestModal");
  // 記住開啟建議箱前焦點所在的元素(通常是那顆「💡 給點建議」鈕),關閉時把焦點還回去。
  // 用鍵盤/報讀器的玩家關掉對話框後不會掉到 body 從頭找,焦點回到他原本的位置。
  let suggestOpener = null;
  // 開對話框時把背後的遊戲畫面與面板設為 inert。Tab 焦點環(見下方 keydown)已擋住
  // 鍵盤焦點跑出對話框,但螢幕報讀器的虛擬游標／觸控報讀器的滑動瀏覽仍能讀到背景的
  // canvas、HUD、聊天等內容——`aria-modal` 單獨並不可靠地把它們藏起來。inert 把整個
  // 背景移出無障礙樹與互動,讓只用報讀器的玩家也真正被限制在對話框內,延續建議箱這串
  // 無障礙修補(欄位標籤／aria-live 狀態播報)。刻意不含 #srStatus:那是連線狀態的
  // aria-live 區,開著對話框時若斷線重連仍要能播報出來。inert 屬性在不支援的舊瀏覽器
  // 上是無害 no-op,焦點環仍照常運作。
  const bgInertIds = ["game", "hud", "suggestBtn", "chat", "connStatus"];
  function setBackgroundInert(on) {
    for (const id of bgInertIds) {
      const el = document.getElementById(id);
      if (el) el.inert = on;
    }
  }
  function openSuggestModal() {
    suggestOpener = document.activeElement;
    modal.style.display = "flex";
    setBackgroundInert(true); // 背景對報讀器/觸控瀏覽封閉,直到關閉還原
    document.getElementById("suggestFrom").value = myName;
    // 開啟即把焦點移進對話框的主要欄位,鍵盤玩家不必先盲目 Tab 穿過遮罩才開始打字。
    document.getElementById("suggestText").focus();
  }
  document.querySelector("#suggestBtn button").addEventListener("click", openSuggestModal);
  // 關閉建議箱(收掉 modal、清掉殘留狀態字、把焦點還回開啟者)。集中成一個函式,
  // 讓「取消鈕／點背景遮罩／按 Esc」三條關閉路徑行為一致。
  function closeSuggestModal() {
    modal.style.display = "none";
    setBackgroundInert(false); // 還原背景的互動與無障礙可達性
    document.getElementById("suggestStatus").textContent = "";
    if (suggestOpener && suggestOpener.focus) suggestOpener.focus();
    suggestOpener = null;
  }
  document.getElementById("suggestCancel").addEventListener("click", closeSuggestModal);
  // 點 modal 外的暗色遮罩關閉(只在點到遮罩本身、非點到內層面板時)——對話框「點外面
  // 關掉」是普遍預期,少一步找取消鈕。
  modal.addEventListener("click", (e) => { if (e.target === modal) closeSuggestModal(); });
  // Esc 關閉 + Tab 焦點環:即使焦點正在建議箱的輸入欄/文字框內也要能關(故獨立監聽,
  // 不受遊戲按鍵守衛影響);只在 modal 開著時作用,不干擾平常的遊戲操作。
  document.addEventListener("keydown", (e) => {
    if (modal.style.display !== "flex") return;
    if (e.key === "Escape") { closeSuggestModal(); return; }
    // 焦點留在對話框內:Tab 到最後一個可聚焦元素再按會繞回第一個,Shift+Tab 反之。
    // 沒有這個環,Tab 會跑到對話框背後被遮罩蓋住、看不見焦點的元素上。
    if (e.key === "Tab") {
      const f = modal.querySelectorAll("input, textarea, button");
      if (!f.length) return;
      const first = f[0], last = f[f.length - 1];
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
    }
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
        setTimeout(closeSuggestModal, 1200); // 一併把焦點還回開啟者
      } else {
        status.textContent = "送出失敗，稍後再試。";
      }
    } catch {
      status.textContent = "送出失敗，稍後再試。";
    }
  });

  // ---- 操作說明可收合 ----
  // 點標題列收起／展開;狀態存 localStorage,玩家選過就尊重他的選擇。
  // 沒選過時依視窗大小給預設:小畫面預設收起省空間,大畫面預設展開讓新手看得到。
  function initHelpToggle() {
    const hud = document.getElementById("hud");
    const toggle = document.getElementById("helpToggle");
    if (!hud || !toggle) return;
    // 沒選過時的預設依視窗大小算:窄(手機直式)或矮(手機橫式:寬而矮,667×375 之類)都收起。
    // 橫式手機寬度過關卻只有一點垂直空間,展開的多行說明會把矮螢幕的地表擠掉——垂直才是橫式
    // 的稀缺資源,故寬高任一不足就收起。抽成純函式,好在轉螢幕/改視窗時重算。
    const defaultCollapsed = () =>
      (window.innerWidth < 560 || window.innerHeight < 480) ? "1" : "0";
    let chosen; // 玩家手動選過的值(localStorage 有值或本次 session 內按過);沒有則為 null
    try { chosen = localStorage.getItem("butfun.helpCollapsed"); } catch {}
    const apply = (v) => {
      const isCollapsed = v === "1";
      hud.classList.toggle("help-collapsed", isCollapsed);
      // 收起＝內容隱藏＝aria-expanded false,讓螢幕報讀器報出展開/收合狀態
      toggle.setAttribute("aria-expanded", isCollapsed ? "false" : "true");
    };
    apply(chosen === null || chosen === undefined ? defaultCollapsed() : chosen);
    const flip = () => {
      const next = hud.classList.contains("help-collapsed") ? "0" : "1";
      apply(next);
      chosen = next; // 標記玩家已自選——之後轉螢幕不再自動覆蓋(localStorage 寫不進也成立)
      try { localStorage.setItem("butfun.helpCollapsed", next); } catch {}
    };
    toggle.addEventListener("click", flip);
    // 鍵盤可達:Tab 聚焦後 Enter / 空白鍵也能收合。stopPropagation 擋掉全域 keydown,
    // 免得空白被當「採腳下格」、Enter 被搶去 focus 聊天。
    toggle.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") { e.preventDefault(); e.stopPropagation(); flip(); }
    });
    // 玩家還沒手動選過前,轉螢幕/改視窗大小就重算預設:直式(窄→收)轉成寬高都夠的橫式該展開、
    // 桌機窄窗放大成寬高足夠時也該展開,反之亦然。一旦玩家手動收/展就完全尊重他、不再覆蓋。
    window.addEventListener("resize", () => {
      if (chosen === null || chosen === undefined) apply(defaultCollapsed());
    });
  }

  // ---- 背包面板可收合 ----
  // 同操作說明:點標題列收/展,狀態存 localStorage,尊重玩家選擇。預設收起——標題列的摘要
  // 計數(updateBagHud 寫的 #bagSummary)收著時仍看得到存量,等於沿用舊那行的快速一瞥、不損失
  // 資訊;想看每項中文名明細再展開。鍵盤(Tab→Enter/空白)可達,焦點框語彙與其他收合鈕一致。
  function initBagToggle() {
    const bag = document.getElementById("hudBag");
    const toggle = document.getElementById("bagToggle");
    if (!bag || !toggle) return;
    let chosen;
    try { chosen = localStorage.getItem("butfun.bagCollapsed"); } catch {}
    const apply = (v) => {
      const isCollapsed = v !== "0"; // 預設收起(摘要已顯示計數),只有顯式選過展開才為 "0"
      bag.classList.toggle("bag-collapsed", isCollapsed);
      // 收起＝明細隱藏＝aria-expanded false,讓螢幕報讀器報出展開/收合狀態
      toggle.setAttribute("aria-expanded", isCollapsed ? "false" : "true");
    };
    apply(chosen === null || chosen === undefined ? "1" : chosen);
    const flip = () => {
      const next = bag.classList.contains("bag-collapsed") ? "0" : "1";
      apply(next);
      try { localStorage.setItem("butfun.bagCollapsed", next); } catch {}
    };
    toggle.addEventListener("click", flip);
    // 鍵盤可達:Tab 聚焦後 Enter / 空白鍵也能收合。stopPropagation 擋掉全域 keydown,
    // 免得空白被當「採腳下格」、Enter 被搶去 focus 聊天。比照操作說明/聊天收合鈕。
    toggle.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") { e.preventDefault(); e.stopPropagation(); flip(); }
    });
  }

  // ---- 合成台面板可收合 ----
  // 與背包同一套語彙:點標題列收/展,狀態存 localStorage,預設收起(標題列摘要已顯示
  // 「N 可合成」,收著也一眼知道能不能合)。鍵盤(Tab→Enter/空白)可達,焦點框語彙一致。
  function initCraftToggle() {
    const panel = document.getElementById("hudCraft");
    const toggle = document.getElementById("craftToggle");
    if (!panel || !toggle) return;
    let chosen;
    try { chosen = localStorage.getItem("butfun.craftCollapsed"); } catch {}
    const apply = (v) => {
      const isCollapsed = v !== "0"; // 預設收起(摘要已顯示可合成數),只有顯式展開才為 "0"
      panel.classList.toggle("craft-collapsed", isCollapsed);
      toggle.setAttribute("aria-expanded", isCollapsed ? "false" : "true");
    };
    apply(chosen === null || chosen === undefined ? "1" : chosen);
    const flip = () => {
      const next = panel.classList.contains("craft-collapsed") ? "0" : "1";
      apply(next);
      try { localStorage.setItem("butfun.craftCollapsed", next); } catch {}
    };
    toggle.addEventListener("click", flip);
    // 鍵盤可達:同背包/聊天收合鈕,Enter/空白收合並擋掉全域 keydown(免空白被當採集)。
    toggle.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") { e.preventDefault(); e.stopPropagation(); flip(); }
    });
  }

  // ---- 擴地面板可收合 ----
  // 與合成台/背包同一套語彙:點標題列收/展,狀態存 localStorage,預設收起(標題列摘要已顯示
  // 可不可擴,收著也一眼知道)。鍵盤(Tab→Enter/空白)可達,焦點框語彙一致。
  function initExpandToggle() {
    const panel = document.getElementById("hudExpand");
    const toggle = document.getElementById("expandToggle");
    if (!panel || !toggle) return;
    let chosen;
    try { chosen = localStorage.getItem("butfun.expandCollapsed"); } catch {}
    const apply = (v) => {
      const isCollapsed = v !== "0"; // 預設收起(摘要已顯示可不可擴),只有顯式展開才為 "0"
      panel.classList.toggle("expand-collapsed", isCollapsed);
      toggle.setAttribute("aria-expanded", isCollapsed ? "false" : "true");
    };
    apply(chosen === null || chosen === undefined ? "1" : chosen);
    const flip = () => {
      const next = panel.classList.contains("expand-collapsed") ? "0" : "1";
      apply(next);
      try { localStorage.setItem("butfun.expandCollapsed", next); } catch {}
    };
    toggle.addEventListener("click", flip);
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

  // ---- 手機軟鍵盤不擋聊天 ----
  // 手機點聊天輸入時系統鍵盤從底部彈起,而聊天面板是 position:fixed 貼底,會被鍵盤整個
  // 蓋住——看不到自己正在打的字、也看不到剛冒出的訊息。用 visualViewport 量出鍵盤吃掉的
  // 高度,聚焦輸入時把整個聊天面板抬到鍵盤之上,失焦或鍵盤收起就放回原位。桌機沒有
  // visualViewport 縮放(overlap 恆 0)行為不變。純客戶端自適應,不碰任何遊戲規則。
  function initChatKeyboardLift() {
    const vv = window.visualViewport;
    const chat = document.getElementById("chat");
    const input = document.getElementById("chatText");
    if (!vv || !chat || !input) return;
    let focused = false;
    const apply = () => {
      // 鍵盤吃掉的高度 = 版面視窗底 與 視覺視窗底 的落差(含視覺視窗上移量)
      const overlap = Math.max(0, window.innerHeight - vv.height - vv.offsetTop);
      chat.style.transform = focused && overlap > 0 ? `translateY(${-overlap}px)` : "";
    };
    input.addEventListener("focus", () => { focused = true; apply(); });
    input.addEventListener("blur", () => { focused = false; apply(); });
    vv.addEventListener("resize", apply);
    vv.addEventListener("scroll", apply);
  }

  // ---- 進場流程 ----
  function enterGame() {
    if (started) return; // 自動重連時 welcome 會再來一次，別重複初始化、別啟動第二個 render 迴圈
    started = true;
    initHelpToggle();
    initChatToggle();
    initBagToggle();
    initCraftToggle();
    initExpandToggle();
    updateCraftPanel([]); // 首個背包快照前先畫出配方(全反灰),不留空面板
    initChatKeyboardLift();
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
    const btn = document.getElementById("joinBtn");
    // 已在連線就忽略重複點擊：否則每點一次都 new 一條 WebSocket＋一條重連鏈,疊出
    // 多條平行連線。連線一旦成功 welcome→enterGame 會隱藏整個登入畫面,不必解鎖。
    if (btn.disabled) return;
    const name = document.getElementById("nameInput").value.trim() || "拓荒者";
    const species = document.getElementById("speciesInput").value;
    try {
      localStorage.setItem("butfun.name", name);
      localStorage.setItem("butfun.species", species);
    } catch {}
    btn.disabled = true;
    setLoginStatus("連線中…");
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
      // 自動以登入身份連線期間,登入畫面仍短暫可見(到 welcome 抵達才隱藏):比照訪客
      // 進場路徑擋掉「以訪客進場」鈕——否則玩家在連上前點它會疊出一條訪客連線、與登入
      // 身份打架;並顯示「連線中…」回饋(慢網路/伺服器剛重啟時才知道在連、別乾等)。
      // 延續登入連線回饋(loginStatus / connStatus / srStatus)的韌性弧線。
      document.getElementById("joinBtn").disabled = true;
      setLoginStatus("連線中…");
      // 顯示登入狀態 + 一鍵登出
      const hud = document.getElementById("hud");
      const tag = document.createElement("div");
      tag.style.opacity = "0.7";
      tag.innerHTML = `已登入：<b></b> · <a href="#" id="renameLink" style="color:#c9a24b">✏️改名</a> · <a href="#" id="logoutLink" style="color:#c9a24b">登出</a>`;
      tag.querySelector("b").textContent = me.name;
      hud.appendChild(tag);
      // 改名:輸入新顯示名 → PATCH /api/profile → 成功即更新標籤(世界名牌/HUD 靠下一張快照即時換)。
      tag.querySelector("#renameLink").addEventListener("click", async (e) => {
        e.preventDefault();
        const next = prompt("改新的顯示名(最多 24 字,其他玩家會看到):", tag.querySelector("b").textContent);
        if (next === null) return;
        try {
          const res = await fetch("/api/profile", {
            method: "PATCH",
            credentials: "same-origin",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ name: next }),
          });
          if (res.ok) {
            const data = await res.json();
            tag.querySelector("b").textContent = data.name; // 伺服器清理後的實際新名
            addChat("系統", `已改名為「${data.name}」。`);
          } else {
            addChat("系統", "改名失敗,請稍後再試。");
          }
        } catch {
          addChat("系統", "改名失敗(連線問題)。");
        }
      });
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
