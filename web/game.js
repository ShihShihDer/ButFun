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
  // 手機/搖桿挖掘輔助:朝實心地形走就自動挖（Core Keeper 式鑿隧道），免得在手機上「移動」與
  // 「點擊挖」互搶同一根手指、只能擇一。你照常用搖桿走，撞到牆就會自動一格一格挖開。
  let lastAutoDig = 0;
  function maybeAutoDig(me) {
    if (!me || !ws || ws.readyState !== 1) return;
    // 目前移動方向（鍵盤/搖桿/手把共用同一組 keys）。沒在移動就不自動挖。
    let dx = (keys.right ? 1 : 0) - (keys.left ? 1 : 0);
    let dy = (keys.down ? 1 : 0) - (keys.up ? 1 : 0);
    if (dx === 0 && dy === 0) return;
    const len = Math.hypot(dx, dy);
    dx /= len; dy /= len;
    // 探測你正前方那一格（玩家半徑 + 半格 ≈ 26px）；只有「擋住你的那格是實心」才挖。
    const px = me.x + dx * 26, py = me.y + dy * 26;
    if (tileKindAt(px, py) === "empty") return; // 前方不是實心地形,不挖（開放地正常走）
    const now = performance.now();
    if (now - lastAutoDig < 170) return;        // 節流:約每 0.17 秒鑿一格
    lastAutoDig = now;
    ws.send(JSON.stringify({ type: "dig", wx: px, wy: py }));
    spawnTapFlash(px, py);
  }
  // 伺服器廣播的各玩家農地（per-player，每塊含 owner / origin / 每格 state·dry）；
  // 進場前為空陣列。自己那塊靠 owner === myId 認出（見 myField）。
  let fields = [];
  // 伺服器廣播的世界採集節點（樹/石/乙太礦,每個含 kind/x/y/remaining/harvestable）;進場前為空。
  let nodes = [];
  // 伺服器廣播的世界敵人（戰鬥 1-F,每個含 kind/x/y/hp/max_hp/alive）;進場前為空。
  let enemies = [];
  // 伺服器廣播的市場掛單（AOI 剔除後附近的掛單，含 id/seller_id/seller_name/item/qty/price_per/x/y）。
  let listings = [];
  // 伺服器廣播的 NPC（目前只有新手村商人，含 x/y/buy_list/sell_list）。
  let npcs = [];
  // 地形格 delta（玩家挖/建後偏離確定性生成的差異）：Map<"cx,cy,tx,ty" → kind>。
  // C-1 永遠為空（所有地形由本地 tileKindAt 生成）；C-2 挖掘後才有真實差異。
  const tileDeltaMap = new Map();
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
  // C-4 建造：玩家從背包選取的放置材料（"dirt"/"stone" 或 null 表示未選）。
  // 右鍵點空格時若有選取材料且背包有貨則送 place；按 Escape 或再次點同項取消選取。
  let selectedBuildMaterial = null;
  // 上一次快照的生命值 + 是否已同步過初始生命。和乙太/背包同理:進場/重連時不把既有血量
  // 當成「掉血/回血」播報;之後血量變化才是真的受擊或恢復。戰鬥(1-F)剛上線、HP 有 HUD
  // 但看不到畫面的玩家完全收不到受擊——補這條把無障礙弧線延伸到戰鬥(連線/採集/收成/日夜之後)。
  let myHp = 0;
  let hpKnown = false;
  let wasDownedLastTick = false; // 上一快照是否倒地，用於偵測「傳回新手村」瞬間
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
  // 採集「動作」特效（治「點擊沒有採集動作」）:每次採集在目標節點記一筆揮擊 { wx, wy, born, kind },
  // 用來畫衝擊星芒 + 讓那顆節點短暫晃動;同時噴出資源碎屑(下方 gatherParticles)。
  const gatherHits = [];
  // 採集碎屑:每筆 { wx, wy, vx, vy, born, color } 從節點向外飛、受重力、淡出。純表現。
  const gatherParticles = [];
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
  // 觸控搖桿的方向死區(px):推桿離原點每軸超過此量,該軸方向才送出。判定(setTouchKeys)與
  // 視覺(死區內圈/推桿亮暗)共用這一個值,讓玩家看到的「亮起=在動」與實際送出的方向恆一致——
  // 死區是客戶端把觸控折算成方向布林的事,非伺服器規則,不嵌進權威遊戲邏輯。
  const TOUCH_DEAD = 14;
  const TAP_SLOP = 22; // 點按/拖曳的分水嶺(px):> 移動死區 TOUCH_DEAD,手指自然微滑不會被當拖曳
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
        document.getElementById("hudPlayers").textContent = `🌐 ${msg.players.length}`;
        // 各玩家農地狀態（per-player）+ 世界採集節點 + 我的乙太/背包 + 日夜
        fields = msg.fields || [];
        nodes = msg.nodes || []; // 防呆:舊版伺服器沒這欄 → 空陣列,不崩
        listings = msg.listings || [];
        npcs = msg.npcs || [];
        // 地形 delta 更新：把伺服器廣播的差異覆蓋進 tileDeltaMap。
        // C-2：kind=empty 表示該格已被挖掉（delta 覆蓋為空），前端直接更新；
        // 重置快照（完整 terrain 陣列）時先清空 map 再填入，確保不殘留舊差異。
        // 此處接收增量廣播：直接 set（含 empty），讓 tileKindAt 查到覆蓋值。
        for (const d of (msg.terrain || [])) {
          tileDeltaMap.set(`${d.cx},${d.cy},${d.tx},${d.ty}`, d.kind);
        }
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
          document.getElementById("hudEther").textContent = `✨ ${myEther}`;
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
          updateWeaponHud(inv);  // 手上武器 pill（有武器才亮）+「合武器更痛」一行引導
          updatePlaceModeHud();  // C-4 放置模式 pill（選取材料才亮）
          updateCraftPanel(inv); // 合成台:夠不夠料的反灰隨背包快照更新
          updateExpandPanel(me); // 擴地:下一格價/夠不夠買隨乙太(與未來 expansions)快照更新
          updateMarketPanel(listings, inv, me.ether, isGuest ? null : me.id); // 市場:附近掛單/張貼/取消
          updateShopPanel(npcs, me); // NPC 商店:靠近商人才能買賣
          updateHpHud(me.hp, me.max_hp); // 戰鬥 1-F:血量 HUD
          // 血量變化 → 補一句 aria-live 播報。HP HUD 是純視覺,看不到畫面的玩家在戰鬥中
          // 完全不知道自己正在挨打;受擊最該即時知道(攸關生死),回血則報一句安心。從快照
          // 差值推得、不嵌任何規則,延續採集/收成/日夜/連線的無障礙弧線。首次同步不報。
          if (hpKnown && me.hp !== myHp) {
            if (me.hp < myHp) {
              announce(me.hp <= 0 ? "你被打趴了！休息片刻後將傳回新手村…" : `受到攻擊,生命 ${me.hp}/${me.max_hp}`);
              damageFlashLethal = me.hp <= 0; // 被打趴閃得更重
              damageFlashUntil = performance.now() + (damageFlashLethal ? 600 : 380); // 一閃即逝、隨即淡出
            } else if (wasDownedLastTick && me.hp >= me.max_hp) {
              // 從倒地滿血復原 = 傳回新手村
              announce("已傳回新手村，繼續加油！");
            } else {
              announce(`恢復生命 ${me.hp}/${me.max_hp}`);
            }
          }
          wasDownedLastTick = me.hp <= 0;
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
      case "claim_plot_ok":
        // 只有購買者自己才顯示提示；其他玩家收到後忽略。
        if (msg.owner === myId) {
          announce("🏡 領地購買成功！走近農地開始耕作吧。");
          addChat("系統", "🏡 領地購買成功！走近農地開始耕作吧。");
        }
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

  // ---- 採集「動作」特效（治「點擊沒有採集動作」）----
  const NODE_PARTICLE_COLOR = { tree: "150,210,140", rock: "200,205,210", ether_ore: "255,210,74" };
  const HIT_MS = 260;
  const PARTICLE_MS = 520;
  // 採集時在目標節點記一筆揮擊 + 噴出資源碎屑。reduceMotion 下不噴碎屑(只留命中星芒)。
  function spawnGatherHit(node) {
    if (!node) return;
    const now = performance.now();
    gatherHits.push({ wx: node.x, wy: node.y, born: now, kind: node.kind });
    if (reduceMotion) return;
    const color = NODE_PARTICLE_COLOR[node.kind] || "220,220,220";
    for (let i = 0; i < 7; i++) {
      const a = -Math.PI / 2 + (Math.random() - 0.5) * 2.2;
      const sp = 60 + Math.random() * 90;
      gatherParticles.push({
        wx: node.x,
        wy: node.y - 14,
        vx: Math.cos(a) * sp,
        vy: Math.sin(a) * sp - 40,
        born: now,
        color,
      });
    }
  }

  // 畫採集特效:命中星芒 + 飛散碎屑(受重力、淡出)。畫在節點之上、當回饋層。
  function drawGatherFx(camX, camY, now) {
    for (let i = gatherHits.length - 1; i >= 0; i--) {
      const h = gatherHits[i];
      const age = now - h.born;
      if (age >= HIT_MS) { gatherHits.splice(i, 1); continue; }
      const t = age / HIT_MS;
      const sx = h.wx - camX;
      const sy = h.wy - camY - 14;
      ctx.save();
      const r = 6 + t * 18;
      ctx.strokeStyle = `rgba(255,255,255,${(0.9 * (1 - t)).toFixed(3)})`;
      ctx.lineWidth = 2.5 * (1 - t);
      for (let k = 0; k < 4; k++) {
        const ang = (k * Math.PI) / 2 + 0.4;
        ctx.beginPath();
        ctx.moveTo(sx + Math.cos(ang) * r * 0.4, sy + Math.sin(ang) * r * 0.4);
        ctx.lineTo(sx + Math.cos(ang) * r, sy + Math.sin(ang) * r);
        ctx.stroke();
      }
      ctx.restore();
    }
    for (let i = gatherParticles.length - 1; i >= 0; i--) {
      const p = gatherParticles[i];
      const age = now - p.born;
      if (age >= PARTICLE_MS) { gatherParticles.splice(i, 1); continue; }
      const tt = age / 1000;
      const t = age / PARTICLE_MS;
      const px = p.wx + p.vx * tt - camX;
      const py = p.wy + p.vy * tt + 240 * tt * tt - camY; // 重力
      ctx.fillStyle = `rgba(${p.color},${(1 - t).toFixed(3)})`;
      ctx.fillRect(px - 2, py - 2, 4, 4);
    }
  }

  // 某節點剛被採時的晃動強度 [0,1)（剛命中=1、HIT_MS 內漸消）。給 drawNodes 讓被打的樹石抖一下。
  function nodeHitWobble(node, now) {
    let best = 0;
    for (const h of gatherHits) {
      if (Math.abs(h.wx - node.x) < 12 && Math.abs(h.wy - node.y) < 12) {
        const t = (now - h.born) / HIT_MS;
        if (t < 1) best = Math.max(best, 1 - t);
      }
    }
    return best;
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

  // dock 視窗的鍵盤開關鉤子:由 initDock 設定(把 openWinFor 暴露給全域 keydown)。
  // 用 null 起手,dock 還沒初始化(進場前)時鍵盤捷徑自動失效,不會 ReferenceError。
  let toggleDockWin = null;

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
      return;
    }
    // B/C/H:鍵盤開對應 dock 視窗(背包/合成台/操作說明),與點 dock 圖示等效——延續
    // M 開地圖的鍵盤平權,讓沒滑鼠的玩家也能一鍵叫出面板。再按同鍵或 Esc 關(由 initDock
    // 的 openWinFor 自帶 toggle 與焦點歸還)。這些字母不與 WASD/EF/M 衝突。擴地(🌱)還沒接線、
    // 暫不給捷徑,待 BuyExpansion 落地再補。
    if (toggleDockWin) {
      const winBtn =
        (e.key === "b" || e.key === "B") ? "dockBag" :
        (e.key === "c" || e.key === "C") ? "dockCraft" :
        (e.key === "h" || e.key === "H") ? "dockHelp" : null;
      if (winBtn) {
        if (!e.repeat) toggleDockWin(winBtn);
        e.preventDefault();
      }
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
    keys.up = dy < -TOUCH_DEAD;
    keys.down = dy > TOUCH_DEAD;
    keys.left = dx < -TOUCH_DEAD;
    keys.right = dx > TOUCH_DEAD;
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

    // 玩家血條:活著但不滿血才畫,畫在腳下。鏡像敵人血條(走近會自動開打、敵我都該一眼看出血量)
    // 的對稱面——原本只有敵人頭上有條,玩家自己/別人挨打後即使站著也看不出血量,只剩 HUD 一個小數字
    // (移動中很容易漏看,見受擊播報/紅光的同一動機)和一閃即逝的紅光。補這條長駐血條,讓「誰受傷了
    // /我血剩多少」在世界上持續可讀。被打趴(hp<=0)另有 💤+壓暗,故只在 0<hp<max 時畫。純讀權威快照
    // 血量、純表現,不嵌任何戰鬥規則(伺服器權威);hp 缺值(訪客/舊伺服器)時不畫,不影響原行為。
    if (typeof p.hp === "number" && p.max_hp > 0 && p.hp > 0 && p.hp < p.max_hp) {
      const bw = 24;
      const bx = sx - bw / 2;
      const hby = sy + 18; // 腳下陰影(sy+12)之下,不蓋到角色與名字
      ctx.fillStyle = "rgba(0,0,0,0.5)";
      ctx.fillRect(bx - 1, hby - 1, bw + 2, 6);
      ctx.fillStyle = "#d65a5a"; // 與敵人血條同一危險紅,語彙一致
      ctx.fillRect(bx, hby, bw * (p.hp / p.max_hp), 4);
    }
  }

  function render() {
    // 每幀重設基準變換(dpr 縮放),確保前一幀任何 save/restore 失衡也不會累積偏移。
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.imageSmoothingEnabled = false; // 像素風禁止插值放大,否則糊邊
    ctx.clearRect(0, 0, viewW, viewH);

    // 手把沒有事件式的「按住中」回呼,必須每幀輪詢;放在繪製前讓本幀就反映方向。
    pollGamepad();

    const me = myId ? players.get(myId) : null;
    maybeAutoDig(me); // 朝牆走自動挖（手機免抬指點，移動即可鑿隧道）
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
    drawTerrain(camX, camY); // 可挖地形方塊（C-1 純顯示，在地表之上、農地之下）
    drawField(camX, camY);
    drawNodes(camX, camY); // 採集節點畫在地表/農地之上、玩家之下
    drawEnemies(camX, camY); // 敵人(戰鬥 1-F)畫在地表之上、玩家之下
    drawNpcs(camX, camY);   // NPC 商人畫在敵人同層
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

    // 採集動作特效（命中星芒 + 飛散碎屑）：同在日夜染色之後畫,當回饋不被夜色蓋暗。
    drawGatherFx(camX, camY, performance.now());

    // 離田時的「回農地」邊緣指標：同在日夜染色之後畫，當 HUD 不被夜色蓋暗。
    drawFarmPointer(camX, camY);

    // 小地圖（右下角縮圖）：在日夜染色「之後」畫，當作 HUD 不被夜色蓋暗。
    drawMinimap();

    // 觸控搖桿視覺(只在按住時出現)。讓畫面忠實反映「角色現在到底有沒有被這根手指驅動」:
    // 推桿一按下就跟著手指,但要拖過 TAP_SLOP 才升級成搖桿、且每軸超過 TOUCH_DEAD 才真的送方向。
    // 先前只畫外圈+恆亮推桿,玩家在死區內推卻不動會困惑。現在加一圈死區內環當參考,並讓推桿
    // 只在「真的在動」(已升級成搖桿且至少一軸出死區)時亮起金色、否則維持暗灰提示「還沒到」。
    if (touchOrigin && touchCurrent) {
      const dx = touchCurrent.x - touchOrigin.x;
      const dy = touchCurrent.y - touchOrigin.y;
      const dist = Math.hypot(dx, dy);
      const cap = 50;
      const r = dist > 0 ? Math.min(dist, cap) / dist : 0;
      // 與 setTouchKeys 同一判定:已升級成搖桿、且任一軸超過死區,角色此刻才真的在移動。
      const active = touchDragged && (Math.abs(dx) > TOUCH_DEAD || Math.abs(dy) > TOUCH_DEAD);
      ctx.strokeStyle = "rgba(255,255,255,0.35)";
      ctx.lineWidth = 2;
      ctx.beginPath(); ctx.arc(touchOrigin.x, touchOrigin.y, cap, 0, Math.PI * 2); ctx.stroke();
      // 死區內環:推桿落在這圈內代表角色不動,給玩家「推超過這圈才會動」的直觀參考。
      ctx.strokeStyle = "rgba(255,255,255,0.18)";
      ctx.beginPath(); ctx.arc(touchOrigin.x, touchOrigin.y, TOUCH_DEAD, 0, Math.PI * 2); ctx.stroke();
      ctx.fillStyle = active ? "rgba(201,162,75,0.9)" : "rgba(150,156,168,0.55)";
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
  // 小地圖縮放:身邊顯示半徑可選(近/中/遠/廣),點地圖左上的 🔍 鈕循環、記住選擇。
  const MM_RADIUS_LEVELS = [650, 1100, 1900, 3200];
  const MM_ZOOM_LABEL = ["近", "中", "遠", "廣"];
  let minimapZoom = 1; // 預設「中」(1100,與原本一致)
  try {
    const z = parseInt(localStorage.getItem("butfun.minimapZoom"), 10);
    if (z >= 0 && z < MM_RADIUS_LEVELS.length) minimapZoom = z;
  } catch {}
  let mmZoomHit = null;
  function cycleMinimapZoom() {
    minimapZoom = (minimapZoom + 1) % MM_RADIUS_LEVELS.length;
    try { localStorage.setItem("butfun.minimapZoom", String(minimapZoom)); } catch {}
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
    // 身邊範圍小地圖:不再縮整張世界,而是以玩家為中心、只畫周圍 MM_RADIUS 世界半徑那一圈
    // (玩家點名要「就身邊」)。方框 size×size,玩家永遠在正中央,北朝上。
    const size = minimapSize();
    const MM_RADIUS = MM_RADIUS_LEVELS[minimapZoom]; // 顯示半徑(世界 px),可由左上 🔍 鈕切換
    const scale = size / (2 * MM_RADIUS);
    // 中心=玩家(用渲染插值 rx/ry 跟主畫面同步);沒有自己的玩家(訪客剛進)就退回世界中心。
    const meP = myId ? players.get(myId) : null;
    const cx = meP ? meP.rx : w / 2;
    const cy = meP ? meP.ry : h / 2;
    // 世界座標 → 小地圖螢幕座標(玩家置中)。
    const toMiniX = (wx) => ox + size / 2 + (wx - cx) * scale;
    const toMiniY = (wy) => oy + size / 2 + (wy - cy) * scale;
    // 縮圖正下方留一條圖例帶(每列三欄,列數自適應)。整組往上挪 legendH,貼右下安全區內。
    const MM_LEGEND_ROWS = Math.ceil(MM_LEGEND.length / 3);
    const MM_LEGEND_H = MM_LEGEND_ROWS * 12 + 8;
    // 右下錨點扣掉安全區內距,notched 手機不被瀏海/圓角/手勢條切到。
    const ox = viewW - MM.margin - safeArea.right - size;
    const oy = viewH - MM.margin - safeArea.bottom - size - MM_LEGEND_H;

    // 半透明深底面板(對齊夜色),底色往下多包住圖例帶。
    ctx.fillStyle = "rgba(10,16,30,0.55)";
    ctx.fillRect(ox - MM.pad, oy - MM.pad, size + MM.pad * 2, size + MM_LEGEND_H + MM.pad * 2);

    // 以下內容裁切在方框內(身邊以外不畫)。
    ctx.save();
    ctx.beginPath();
    ctx.rect(ox, oy, size, size);
    ctx.clip();

    // 生態域背景:粗格取樣 biomeAt,把身邊地貌(草原/森林/岩/沙/水)用底色畫出來——
    // 跟主畫面同一套確定性噪聲,小地圖因此「看得到地形」。
    // ③ 無限世界（切片 A）：世界無邊界了,身邊一律照 biomeAt 畫,不再有界外 void。
    const MM_STEP = 8; // 每格 mini px(粗取樣,夠看地貌又省效能;之後切片3 改離屏快取)
    for (let yy = 0; yy < size; yy += MM_STEP) {
      for (let xx = 0; xx < size; xx += MM_STEP) {
        const wx = cx + (xx + MM_STEP / 2 - size / 2) / scale;
        const wy = cy + (yy + MM_STEP / 2 - size / 2) / scale;
        ctx.fillStyle = BIOME_GROUND[biomeAt(wx, wy)];
        ctx.fillRect(ox + xx, oy + yy, MM_STEP + 1, MM_STEP + 1);
      }
    }

    // 各玩家農地(在範圍內才畫)。自己那塊畫亮、別人的暗。
    for (const f of fields) {
      const fx = toMiniX(f.origin_x);
      const fy = toMiniY(f.origin_y);
      const fw = f.cols * f.tile_size * scale;
      const fh = f.rows * f.tile_size * scale;
      if (fx + fw < ox || fx > ox + size || fy + fh < oy || fy > oy + size) continue;
      ctx.fillStyle = f.owner === myId ? "rgba(201,162,75,0.95)"
          : f.owner === PUB_FIELD_OWNER ? "rgba(74,184,160,0.9)"
          : "rgba(123,80,40,0.85)";
      ctx.fillRect(fx, fy, Math.max(3, fw), Math.max(3, fh));
    }

    // 目前畫面看得到的範圍(鏡頭視野框):用最近一次 render 的鏡頭 lastCam + 畫布尺寸推得,
    // 細白框。身邊地圖上仍標出「這一眼看多大」。clip 已限制不溢出方框。
    ctx.strokeStyle = "rgba(255,255,255,0.5)";
    ctx.lineWidth = 1;
    ctx.strokeRect(
      toMiniX(lastCam.x), toMiniY(lastCam.y),
      Math.max(2, viewW * scale), Math.max(2, viewH * scale)
    );

    // 採集節點(範圍內才點,色依 kind;採空畫淡)。
    for (const n of nodes) {
      const col = MM_NODE_COLOR[n.kind];
      if (!col) continue;
      const nx = toMiniX(n.x), ny = toMiniY(n.y);
      if (nx < ox || nx > ox + size || ny < oy || ny > oy + size) continue;
      ctx.beginPath();
      ctx.arc(nx, ny, 1.8, 0, Math.PI * 2);
      ctx.fillStyle = `rgba(${col},${n.harvestable ? 0.9 : 0.3})`;
      ctx.fill();
    }

    // 敵人(活著的紅點,範圍內才畫)。
    for (const e of enemies) {
      if (!e.alive) continue;
      const ex = toMiniX(e.x), ey = toMiniY(e.y);
      if (ex < ox || ex > ox + size || ey < oy || ey > oy + size) continue;
      ctx.beginPath();
      ctx.arc(ex, ey, 2.5, 0, Math.PI * 2);
      ctx.fillStyle = "rgba(214,90,90,0.85)";
      ctx.fill();
    }

    // 玩家:自己亮(永遠在正中央)、其他人暗(範圍內才畫)。用 rx/ry 跟主畫面同步不跳動。
    for (const p of players.values()) {
      const isMe = p.id === myId;
      const px = toMiniX(p.rx), py = toMiniY(p.ry);
      if (!isMe && (px < ox || px > ox + size || py < oy || py > oy + size)) continue;
      ctx.beginPath();
      ctx.arc(px, py, isMe ? 4 : 2.5, 0, Math.PI * 2);
      ctx.fillStyle = isMe ? "#ffd24a" : "rgba(111,168,220,0.7)";
      ctx.fill();
    }

    ctx.restore(); // 解除裁切

    // 方框外框(黃銅語彙)。
    ctx.strokeStyle = "rgba(201,162,75,0.7)";
    ctx.lineWidth = 2;
    ctx.strokeRect(ox, oy, size, size);

    // 圖例帶:縮圖下方每列三欄,每格「色 swatch＋中文」。畫在縮圖之下、收合鈕之前。
    const mmColW = size / 3;
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    ctx.font = "9px system-ui, sans-serif";
    MM_LEGEND.forEach((it, i) => {
      const lx = ox + (i % 3) * mmColW + 2;
      const ly = oy + size + 9 + Math.floor(i / 3) * 12;
      ctx.fillStyle = `rgb(${it.c})`;
      if (it.sq) {
        ctx.fillRect(lx, ly - 2.2, 4.4, 4.4);
      } else {
        ctx.beginPath();
        ctx.arc(lx + 2, ly, 2.2, 0, Math.PI * 2);
        ctx.fill();
      }
      ctx.fillStyle = "rgba(230,232,238,0.92)";
      ctx.fillText(it.t, lx + 7, ly + 0.5);
    });
    ctx.textBaseline = "alphabetic";

    // 收合鈕:面板右上角一顆小「–」,點它(或按 M)把小地圖收起。畫在最後蓋在縮圖上。
    const tb = 18;
    const tx = ox + size + MM.pad - tb;
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

    // 縮放鈕:面板左上角一顆「🔍近/中/遠/廣」,點它循環身邊顯示半徑(記住選擇)。
    const zw = 30, zh = 18;
    const zx = ox - MM.pad;
    const zy = oy - MM.pad;
    ctx.fillStyle = "rgba(10,16,30,0.85)";
    ctx.fillRect(zx, zy, zw, zh);
    ctx.strokeStyle = "rgba(201,162,75,0.7)";
    ctx.lineWidth = 1;
    ctx.strokeRect(zx, zy, zw, zh);
    ctx.fillStyle = "#c9a24b";
    ctx.font = "11px system-ui, sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText("🔍" + MM_ZOOM_LABEL[minimapZoom], zx + zw / 2, zy + zh / 2 + 1);
    ctx.textAlign = "left";
    ctx.textBaseline = "alphabetic";
    mmZoomHit = { x: zx, y: zy, w: zw, h: zh };
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

  // 命中小地圖縮放鈕的熱區?命中即循環顯示半徑並回 true(同 toggle:外擴熱區、不被當農作)。
  function minimapZoomHit(clientX, clientY) {
    if (!mmZoomHit) return false;
    const rect = canvas.getBoundingClientRect();
    const sx = clientX - rect.left, sy = clientY - rect.top;
    const t = mmZoomHit;
    const padX = Math.max(0, (MM_TAP_MIN - t.w) / 2);
    const padY = Math.max(0, (MM_TAP_MIN - t.h) / 2);
    if (sx >= t.x - padX && sx <= t.x + t.w + padX &&
        sy >= t.y - padY && sy <= t.y + t.h + padY) {
      cycleMinimapZoom();
      return true;
    }
    return false;
  }

  // ── 程序生成生態域(biome)──────────────────────────────────────────────
  // biome 是「世界座標的確定性函式」:同座標永遠同結果、平滑過渡、無接縫,而且不必傳一張
  // 大地圖(避開 tilemap 那套 netcode)。先做純前端視覺(各式各樣的場景);切片 3 後端會用
  // 同一套門檻放「生態域專屬的節點/怪」時再對齊。
  // 平滑 value noise:格點雜湊(重用 grassHash)+ smoothstep 雙線性內插。scale 越大、區塊越大。
  function biomeNoise(wx, wy, scale, seed) {
    const gx = wx / scale, gy = wy / scale;
    const x0 = Math.floor(gx), y0 = Math.floor(gy);
    const fx = gx - x0, fy = gy - y0;
    const h = (a, b) => grassHash((a | 0) * 1009 + seed, (b | 0) * 9176 + seed * 31);
    const v00 = h(x0, y0), v10 = h(x0 + 1, y0), v01 = h(x0, y0 + 1), v11 = h(x0 + 1, y0 + 1);
    const sx = fx * fx * (3 - 2 * fx), sy = fy * fy * (3 - 2 * fy); // smoothstep
    const a = v00 + (v10 - v00) * sx;
    const b = v01 + (v11 - v01) * sx;
    return a + (b - a) * sy; // [0,1)
  }
  // 座標 → 生態域種類。海拔 e 決定水/沙/高地、濕度 m 在中海拔分森林/草原。
  // scale ~1500 → 走得到成片場景(不是雜訊碎點)。
  function biomeAt(wx, wy) {
    const e = biomeNoise(wx, wy, 1500, 7);
    const m = biomeNoise(wx, wy, 1200, 137);
    if (e < 0.30) return "water";
    if (e < 0.355) return "sand";
    if (e > 0.76) return "rocky";
    return m > 0.56 ? "forest" : "meadow";
  }
  // 各生態域地表底色(非草地生態域 / 無 tileset 時用)。
  const BIOME_GROUND = { water: "#27566f", sand: "#b3a06a", meadow: "#16361f", forest: "#102a18", rocky: "#4f4a44" };

  // ── 地形格生成（與 world-core tile_kind_at 逐位元對齊）──────────────────────────
  // 對齊 Rust 版：grass_hash(gx*1031 ^ gy*2053, gx ^ gy*1009)
  function tileHash(gx, gy) {
    const ix = (Math.imul(gx | 0, 1031) ^ Math.imul(gy | 0, 2053)) | 0;
    const iy = ((gx | 0) ^ Math.imul(gy | 0, 1009)) | 0;
    return grassHash(ix, iy);
  }

  // 世界像素座標 → 地形格種類（"empty"/"dirt"/"stone"/"ore"）。
  // 確定性生成：前端本地計算，與伺服器 tile_kind_at 邏輯相同（零帶寬）；
  // C-2 起若 tileDeltaMap 有覆蓋則回覆蓋值。
  function tileKindAt(wx, wy) {
    // 格索引（整數）
    const gx = Math.floor(wx / TS) | 0;
    const gy = Math.floor(wy / TS) | 0;
    // 先查 delta 覆蓋
    const CHUNK_T = 16; // TILES_PER_CHUNK
    const cx = Math.floor(gx / CHUNK_T);
    const cy = Math.floor(gy / CHUNK_T);
    const tx = ((gx % CHUNK_T) + CHUNK_T) % CHUNK_T;
    const ty = ((gy % CHUNK_T) + CHUNK_T) % CHUNK_T;
    const delta = tileDeltaMap.get(`${cx},${cy},${tx},${ty}`);
    if (delta !== undefined) return delta;
    // 新手村安全區一律乾淨地（與後端 world-core SAFE_ZONE_* 對齊，改一邊要改另一邊）。
    {
      const sdx = wx - 2344, sdy = wy - 2296;
      if (sdx * sdx + sdy * sdy <= 640 * 640) return "empty";
    }
    // 確定性生成
    const b = biomeAt(wx, wy);
    if (b === "water") return "empty";
    const h = tileHash(gx, gy);
    if (b === "rocky") {
      if (h < 0.05) return "ore";
      if (h < 0.40) return "stone";
      return "empty";
    }
    if (b === "forest") {
      if (h < 0.08) return "stone";
      if (h < 0.22) return "dirt";
      return "empty";
    }
    if (b === "meadow") {
      if (h < 0.06) return "stone";
      if (h < 0.12) return "dirt";
      return "empty";
    }
    // sand
    if (h < 0.04) return "stone";
    if (h < 0.08) return "dirt";
    return "empty";
  }

  // 畫可挖地形方塊（C-1：純顯示，在生態域底色之上、農地/節點之下）。
  // 前端本地生成（tileKindAt），零帶寬；C-2 起有挖掘 delta 才需後端同步。
  function drawTerrain(camX, camY) {
    const tx0 = Math.floor(camX / TS) - 1;
    const ty0 = Math.floor(camY / TS) - 1;
    const tx1 = Math.floor((camX + viewW) / TS) + 1;
    const ty1 = Math.floor((camY + viewH) / TS) + 1;
    const W = tx1 - tx0 + 1, H = ty1 - ty0 + 1;
    // 先把可視範圍每格種類算一遍（含外圈一格供鄰居判斷），避免每格重複呼叫 tileKindAt（手機效能）。
    const grid = new Array(W * H);
    for (let j = 0; j < H; j++)
      for (let i = 0; i < W; i++)
        grid[j * W + i] = tileKindAt((tx0 + i) * TS + TS / 2, (ty0 + j) * TS + TS / 2);
    const isEmpty = (i, j) =>
      i < 0 || j < 0 || i >= W || j >= H || grid[j * W + i] === "empty";
    for (let j = 0; j < H; j++) {
      for (let i = 0; i < W; i++) {
        const kind = grid[j * W + i];
        if (kind === "empty") continue;
        const tx = tx0 + i, ty = ty0 + j;
        const sx = Math.round(tx * TS - camX);
        const sy = Math.round(ty * TS - camY);
        // 基底色（調飽和、彼此分得開，別都糊成灰褐）。
        ctx.fillStyle = kind === "ore" ? "#7a6533" : kind === "stone" ? "#6d6a66" : "#6e4f30";
        ctx.fillRect(sx, sy, TS, TS);
        // 每格固定微抖動紋理（土感，不隨鏡頭閃爍）。
        const jitter = grassHash(tx * 5 + 3, ty * 7 + 1);
        if (jitter > 0.74) { ctx.fillStyle = "rgba(255,255,255,0.06)"; ctx.fillRect(sx, sy, TS, TS); }
        else if (jitter < 0.16) { ctx.fillStyle = "rgba(0,0,0,0.10)"; ctx.fillRect(sx, sy, TS, TS); }
        // 立體邊**只畫在碰到空格的交界**：內部實心連成一片乾淨土石，挖出的隧道牆才有亮/暗邊
        // （這是修「整片實心變醜網格」的關鍵）。
        const up = isEmpty(i, j - 1), down = isEmpty(i, j + 1);
        const left = isEmpty(i - 1, j), right = isEmpty(i + 1, j);
        if (up) { ctx.fillStyle = "rgba(255,255,255,0.16)"; ctx.fillRect(sx, sy, TS, 3); }
        if (left) { ctx.fillStyle = "rgba(255,255,255,0.10)"; ctx.fillRect(sx, sy, 3, TS); }
        if (down) { ctx.fillStyle = "rgba(0,0,0,0.30)"; ctx.fillRect(sx, sy + TS - 3, TS, 3); }
        if (right) { ctx.fillStyle = "rgba(0,0,0,0.22)"; ctx.fillRect(sx + TS - 3, sy, 3, TS); }
        // 礦脈：露出（碰到空格）才亮，提示「挖這裡有礦」；埋在深處只隱約。
        if (kind === "ore") {
          ctx.fillStyle = (up || down || left || right) ? "rgba(255,212,92,0.78)" : "rgba(205,165,70,0.40)";
          const s = 9;
          ctx.fillRect(sx + TS / 2 - s / 2, sy + TS / 2 - s / 2, s, s);
        }
      }
    }
  }

  // 畫地面(程序生態域)+ 世界邊界。草原/森林保留草地瓦片(森林壓暗一階拉層次),
  // 水/沙/岩改用生態域底色 → 走到哪、場景就不同,無接縫。
  function drawGround(camX, camY) {
    ctx.fillStyle = "#12331f";
    ctx.fillRect(0, 0, viewW, viewH);

    const tx0 = Math.floor(camX / TS) - 1;
    const ty0 = Math.floor(camY / TS) - 1;
    const tx1 = Math.floor((camX + viewW) / TS) + 1;
    const ty1 = Math.floor((camY + viewH) / TS) + 1;
    const hasTiles = artOk("tileset_a");
    for (let ty = ty0; ty <= ty1; ty++) {
      for (let tx = tx0; tx <= tx1; tx++) {
        const b = biomeAt(tx * TS + TS / 2, ty * TS + TS / 2);
        const dx = Math.round(tx * TS - camX);
        const dy = Math.round(ty * TS - camY);
        if (hasTiles && (b === "meadow" || b === "forest")) {
          const variant = (grassHash(tx, ty) * 4) | 0; // 0..3
          ctx.drawImage(ART.tileset_a, variant * TS, 0, TS, TS, dx, dy, TS, TS);
          if (b === "forest") { // 森林壓暗,與草原拉出層次
            ctx.fillStyle = "rgba(8,26,14,0.42)";
            ctx.fillRect(dx, dy, TS + 1, TS + 1);
          }
        } else {
          ctx.fillStyle = BIOME_GROUND[b];
          ctx.fillRect(dx, dy, TS + 1, TS + 1);
          // 同格固定的微亮/暗抖動,給點質感(不隨鏡頭閃爍)。
          const j = grassHash(tx * 3 + 5, ty * 7 + 2);
          if (j > 0.8) { ctx.fillStyle = "rgba(255,255,255,0.05)"; ctx.fillRect(dx, dy, TS + 1, TS + 1); }
          else if (j < 0.16) { ctx.fillStyle = "rgba(0,0,0,0.10)"; ctx.fillRect(dx, dy, TS + 1, TS + 1); }
        }
      }
    }
    if (hasTiles) drawDecorations(camX, camY);

    // 裝飾(草叢/樹/石)。畫在地表之上、農地/節點/玩家之下。水域不長草(drawScenery 內跳過)。
    drawScenery(camX, camY);
    // ③ 無限世界（切片 A）：拿掉世界邊框——世界不再有邊，往哪走都有地表延伸。
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
        // ③ 無限世界（切片 A）：不再剔除世界界外——裝飾跟著 biomeAt 無接縫延伸。
        if (biomeAt(wx, wy) === "water") continue; // 水域不長草/樹
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
        // ③ 無限世界（切片 A）：不再剔除世界界外,草地紋理隨鏡頭無接縫延伸。
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

  // 公共農地（owner === nil UUID）；伺服器啟動後一直存在。
  const PUB_FIELD_OWNER = "00000000-0000-0000-0000-000000000000";
  function pubField() {
    return fields.find((f) => f.owner === PUB_FIELD_OWNER) || null;
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
  // 小地圖上的節點點色:NODE_LOOK 的 tint 偏暗(縮圖深底上糊掉、乙太礦深藍又跟其他玩家的
  // 藍點混),這裡另給一組較亮、彼此可分的點色。樹綠/石灰/乙太礦亮紫——乙太礦是經濟核心資源,
  // 給它最跳的色好讓玩家一眼找到「該去哪採乙太」。純表現、只讀既有快照,不嵌任何規則。
  const MM_NODE_COLOR = {
    tree: "120,190,110",
    rock: "170,175,180",
    ether_ore: "165,140,240",
  };
  // 小地圖圖例:縮圖把樹/石/乙太礦/敵人/自己/夥伴都點成彩點了,但「哪個色是什麼」全靠猜——
  // 新手分不出綠點是樹還是夥伴、紫點是乙太礦還是敵人,上一輪「一眼看出資源聚在哪」其實看不懂。
  // 補一排迷你圖例(色點＋中文),色直接沿用上面各點層的同一組,讓縮圖真的一眼讀懂。乙太礦給
  // 最跳的紫、敵人沿用受擊紅、自己用亮黃——和點層完全一致,圖例與實點不會對不上。純表現、不嵌規則。
  // 田:小地圖把農地畫成黃銅／棕色「方塊」(自己亮、別人暗,見 drawMinimap 的 fields 迴圈),
  // 但上一輪圖例只列了點狀實體(樹/石/礦/敵/我/夥伴),沒解釋那塊黃銅方塊是什麼——登入有地的
  // 玩家看到方塊只能猜。補一格「田」:色用自己那塊的黃銅,swatch 刻意畫成方塊(sq)而非圓點,
  // 對齊它在地圖上本就是方塊的視覺語彙(點＝實體、方塊＝地塊),也跟相鄰「我」的亮黃圓點區隔開。
  const MM_LEGEND = [
    { c: "120,190,110", t: "樹" },
    { c: "170,175,180", t: "石" },
    { c: "165,140,240", t: "乙太" },
    { c: "255,210,74", t: "我" },
    { c: "111,168,220", t: "夥伴" },
    { c: "214,90,90", t: "敵" },
    { c: "201,162,75", t: "田", sq: true },
  ];
  // 節點 kind → 現成 sprite 名（assets/*.png）。有圖就畫真的樹/石(不再是圓點 emoji);
  // 乙太礦沒專屬圖,留 emoji 發光。圖還沒載入也自動退回 emoji(artOk 把關)。
  const NODE_SPRITE = { tree: "tree", rock: "rock" };
  // 各 kind 滿耐久(鏡像伺服器 NodeKind::max_durability):畫「還要幾下」進程點。
  const NODE_MAX = { tree: 5, rock: 4, ether_ore: 3 };
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
    const now = performance.now();
    for (const n of nodes) {
      // 採空即「消失」(麥塊式:挖完不見、稍後在他處重生)——重生倒數中的節點不畫。
      if (!n.harvestable) continue;
      const sx = n.x - camX;
      const sy = n.y - camY;
      if (sx < -40 || sy < -40 || sx > viewW + 40 || sy > viewH + 40) continue;
      const look = NODE_LOOK[n.kind] || { icon: "❔", tint: "#555" };
      // 剛被採的節點橫向抖一下(被砍/被敲的反應),強度隨時間漸消。
      const wob = nodeHitWobble(n, now);
      const dx = wob > 0 ? Math.sin(now * 0.045) * 5 * wob : 0;
      ctx.save();
      const spriteName = NODE_SPRITE[n.kind];
      if (spriteName && artOk(spriteName)) {
        // 有對應 sprite(樹/石)→ 畫真的 pixel art,底部對齊節點位置、放大些好看清。
        const img = ART[spriteName];
        const s = 44;
        ctx.drawImage(img, sx + dx - s / 2, sy - s + 8, s, s);
      } else {
        // 沒 sprite(乙太礦)或圖還沒載入 → 退回圓盤 + emoji。
        ctx.beginPath();
        ctx.arc(sx + dx, sy, 16, 0, Math.PI * 2);
        ctx.fillStyle = look.tint;
        ctx.fill();
        ctx.font = "20px system-ui, sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(look.icon, sx + dx, sy + 1);
      }
      // 進程點:被採過(remaining < 滿耐久)且還可採時,在腳下畫一排小點顯示「還要幾下」。
      const max = NODE_MAX[n.kind];
      if (n.harvestable && max && n.remaining < max) {
        const pipR = 2;
        const gap = 7;
        const totalW = (max - 1) * gap;
        for (let k = 0; k < max; k++) {
          ctx.beginPath();
          ctx.arc(sx - totalW / 2 + k * gap, sy + 12, pipR, 0, Math.PI * 2);
          ctx.fillStyle = k < n.remaining ? "rgba(255,235,180,0.95)" : "rgba(0,0,0,0.4)";
          ctx.fill();
        }
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
    scrap_drone: { tint: "#6b4a3a" },
    ether_wisp: { tint: "#46407a" },
  };

  // 銹蝕巡邏機:懸浮的故障舊機械。金屬殼 + 脈動紅色感測眼 + 頂端閃燈 + 側鰭。純 canvas 畫。
  function drawScrapDrone(cx, cy, t, phase) {
    const x = cx - 13, y = cy - 9, w = 26, h = 18, r = 7;
    ctx.beginPath();
    ctx.moveTo(x + r, y);
    ctx.arcTo(x + w, y, x + w, y + h, r);
    ctx.arcTo(x + w, y + h, x, y + h, r);
    ctx.arcTo(x, y + h, x, y, r);
    ctx.arcTo(x, y, x + w, y, r);
    ctx.closePath();
    ctx.fillStyle = "#5a4636";
    ctx.fill();
    ctx.lineWidth = 2;
    ctx.strokeStyle = "#2c211a";
    ctx.stroke();
    ctx.fillStyle = "#46362a";
    ctx.beginPath(); ctx.moveTo(cx - 13, cy - 2); ctx.lineTo(cx - 19, cy + 1); ctx.lineTo(cx - 13, cy + 5); ctx.closePath(); ctx.fill();
    ctx.beginPath(); ctx.moveTo(cx + 13, cy - 2); ctx.lineTo(cx + 19, cy + 1); ctx.lineTo(cx + 13, cy + 5); ctx.closePath(); ctx.fill();
    ctx.strokeStyle = "#3a2c22"; ctx.lineWidth = 1.5;
    ctx.beginPath(); ctx.moveTo(cx, cy - 9); ctx.lineTo(cx, cy - 15); ctx.stroke();
    const blink = Math.sin(t * 6 + phase) > 0 ? 1 : 0.3;
    ctx.fillStyle = `rgba(255,120,60,${blink})`;
    ctx.beginPath(); ctx.arc(cx, cy - 16, 2, 0, Math.PI * 2); ctx.fill();
    const pulse = 0.7 + 0.3 * Math.sin(t * 4 + phase);
    ctx.fillStyle = "#1a0f0a"; ctx.beginPath(); ctx.arc(cx, cy, 6, 0, Math.PI * 2); ctx.fill();
    ctx.fillStyle = `rgba(255,90,50,${pulse})`; ctx.beginPath(); ctx.arc(cx, cy, 4, 0, Math.PI * 2); ctx.fill();
    ctx.fillStyle = "rgba(255,210,180,0.9)"; ctx.beginPath(); ctx.arc(cx - 1.2, cy - 1.2, 1.3, 0, Math.PI * 2); ctx.fill();
  }

  // 迷途乙太靈:野化的乙太生靈。柔光球 + 下垂飄帶 + 兩顆小眼,整體脈動漂浮。純 canvas 畫。
  function drawEtherWisp(cx, cy, t, phase) {
    const glow = ctx.createRadialGradient(cx, cy, 2, cx, cy, 16);
    glow.addColorStop(0, "rgba(150,130,230,0.55)");
    glow.addColorStop(1, "rgba(70,64,122,0)");
    ctx.fillStyle = glow;
    ctx.beginPath(); ctx.arc(cx, cy, 16, 0, Math.PI * 2); ctx.fill();
    for (let k = 1; k <= 3; k++) {
      const ty = cy + 6 + k * 5;
      const tx = cx + Math.sin(t * 3 + phase + k * 0.8) * 4;
      ctx.fillStyle = `rgba(139,127,214,${0.5 - k * 0.13})`;
      ctx.beginPath(); ctx.arc(tx, ty, 5 - k * 1.2, 0, Math.PI * 2); ctx.fill();
    }
    const pulse = 0.85 + 0.15 * Math.sin(t * 4 + phase);
    ctx.fillStyle = `rgba(159,147,224,${pulse})`;
    ctx.beginPath(); ctx.arc(cx, cy, 9, 0, Math.PI * 2); ctx.fill();
    ctx.fillStyle = "rgba(20,16,40,0.9)";
    ctx.beginPath(); ctx.arc(cx - 3, cy - 1, 1.6, 0, Math.PI * 2); ctx.fill();
    ctx.beginPath(); ctx.arc(cx + 3, cy - 1, 1.6, 0, Math.PI * 2); ctx.fill();
  }

  // 畫 NPC 商人（新手村固定位置）。外觀：黃銅色頭部 + 棕色身體 + 小旗招牌。
  function drawNpcs(camX, camY) {
    const t = performance.now() / 1000;
    for (const npc of npcs) {
      const sx = npc.x - camX;
      const sy = npc.y - camY;
      if (sx < -60 || sy < -60 || sx > viewW + 60 || sy > viewH + 60) continue;

      // 輕微上下浮動（呼吸感，與敵人對稱）
      const bob = reduceMotion ? 0 : Math.sin(t * 1.2 + npc.x * 0.01) * 2;
      const by = sy + bob;

      ctx.save();
      // 身體（棕色斗篷）
      ctx.fillStyle = "#7b4f2e";
      ctx.beginPath();
      ctx.ellipse(sx, by + 10, 11, 14, 0, 0, Math.PI * 2);
      ctx.fill();
      // 頭（黃銅色）
      ctx.fillStyle = "#c9a24b";
      ctx.beginPath();
      ctx.arc(sx, by - 8, 9, 0, Math.PI * 2);
      ctx.fill();
      // 帽子
      ctx.fillStyle = "#5c3d1e";
      ctx.fillRect(sx - 11, by - 17, 22, 6);
      ctx.fillRect(sx - 6, by - 24, 12, 8);
      // 商店小旗（右上角）
      ctx.strokeStyle = "#c9a24b";
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(sx + 12, by - 20);
      ctx.lineTo(sx + 12, by - 8);
      ctx.stroke();
      ctx.fillStyle = "#e8c055";
      ctx.beginPath();
      ctx.moveTo(sx + 12, by - 20);
      ctx.lineTo(sx + 20, by - 16);
      ctx.lineTo(sx + 12, by - 12);
      ctx.fill();
      // 商人名牌
      ctx.font = "bold 10px sans-serif";
      ctx.textAlign = "center";
      ctx.fillStyle = "#c9a24b";
      ctx.fillText("🧑‍💼 商人", sx, by - 30);
      ctx.restore();
    }
  }

  // 畫世界上的敵人 + 血條。被打倒(重生中)的畫很淡;走近會自動開打(伺服器每秒結算,前端只呈現)。
  function drawEnemies(camX, camY) {
    const fxNow = performance.now();
    const t = fxNow / 1000;
    for (let i = 0; i < enemies.length; i++) {
      const e = enemies[i];
      const sx = e.x - camX;
      const sy = e.y - camY;
      if (sx < -40 || sy < -40 || sx > viewW + 40 || sy > viewH + 40) continue;
      // 每隻用座標當相位 → 動作不同步;上下浮動給生命感(reduceMotion 不動)。
      const phase = e.x * 0.7 + e.y * 0.3;
      const ey = sy + (reduceMotion ? 0 : Math.sin(t * 3 + phase) * 2.5);
      const fx = enemyFx[i];
      const fxT = fx && fxNow < fx.until
        ? Math.max(0, Math.min(1, (fx.until - fxNow) / (fx.lethal ? 480 : 280)))
        : 0;
      ctx.save();
      if (!e.alive) ctx.globalAlpha = 0.22; // 被打倒、重生中
      // 影子(定在地面、不隨浮動)
      ctx.fillStyle = "rgba(0,0,0,0.22)";
      ctx.beginPath(); ctx.ellipse(sx, sy + 13, 12, 4, 0, 0, Math.PI * 2); ctx.fill();
      // 生物造型(走近會動、會追——這層只負責長相)
      if (e.kind === "scrap_drone") drawScrapDrone(sx, ey, t, phase);
      else if (e.kind === "ether_wisp") drawEtherWisp(sx, ey, t, phase);
      else {
        const look = ENEMY_LOOK[e.kind] || { tint: "#555" };
        ctx.fillStyle = look.tint;
        ctx.beginPath(); ctx.arc(sx, ey, 12, 0, Math.PI * 2); ctx.fill();
      }
      // 受擊閃光:往外擴張淡出的亮環(被打倒時更白更大)。reduceMotion 只畫靜態亮邊。
      if (fxT > 0) {
        const base = fx.lethal ? 20 : 17;
        const r = reduceMotion ? base : base + (1 - fxT) * (fx.lethal ? 16 : 10);
        ctx.globalAlpha = (e.alive ? 1 : 0.22) * fxT;
        ctx.lineWidth = fx.lethal ? 3 : 2;
        ctx.strokeStyle = fx.lethal ? "#fff" : "#ffd9a0";
        ctx.beginPath();
        ctx.arc(sx, ey, r, 0, Math.PI * 2);
        ctx.stroke();
        ctx.globalAlpha = e.alive ? 1 : 0.22;
      }
      // 血條:活著且不滿血才畫(定在頭上)。
      if (e.alive && e.hp < e.max_hp) {
        const bw = 28;
        const bx = sx - bw / 2;
        const by = sy - 24;
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
      el.textContent = "💀 休息中…";
      el.style.color = "#f88";
      el.setAttribute("aria-label", "被打趴,休息復原中");
    } else {
      el.textContent = `❤ ${hp}/${maxHp}`;
      el.style.color = hp < maxHp * 0.35 ? "#f88" : "";
      el.setAttribute("aria-label", `生命 ${hp}/${maxHp}`);
    }
  }

  // 背包 HUD:把 [{item,qty}] 顯示成「🪵 N　🪨 N　✨ N」。空背包就只留標頭。
  // pickaxe 是合成產物(1-C/1-D),會隨背包快照回來;補進這三張表,讓合成出的鎬子在
  // 背包明細/飄字/報讀器都跟採集三資源一樣有 emoji、中文名與色,不掉回裸字串。
  // weapon 是合成產物(伺服器 crafting.rs 的 "weapon" 配方,ItemKind::Weapon → snake_case "weapon"),
  // 會隨背包快照回來;補進這三張表,讓合出的武器跟工具一樣有 emoji/中文名/色,不掉回裸字串 "weapon"。
  const ITEM_LOOK = { wood: "🪵", dirt: "🟫", stone: "🪨", ether: "✨", pickaxe: "⛏️", reinforced_pickaxe: "⚒️", weapon: "🗡️" };
  // 報讀器用的品項中文名（emoji 對報讀器無意義,播報時念名字而非圖示）。
  const ITEM_NAME = { wood: "木材", dirt: "土磚", stone: "石頭", ether: "乙太", pickaxe: "鎬子", reinforced_pickaxe: "強化鎬", weapon: "武器" };
  // 採集飄字的品項色（與節點底色同調,讓「採到什麼」一眼可分）。強化鎬比鎬子更金亮一階,呼應升級。武器走攻擊紅。
  const ITEM_FLOAT_COLOR = { wood: "150,210,140", dirt: "190,150,100", stone: "200,205,210", ether: "255,210,74", pickaxe: "210,180,120", reinforced_pickaxe: "230,195,90", weapon: "232,96,84" };
  // 合成配方表(前端呈現用,與伺服器 crafting.rs 的 RECIPES 對齊):產物 ← 素材。
  // 只用來畫面板與「夠不夠料」的提示反灰——真正查表扣料一律由伺服器說了算(規則只在伺服器)。
  // 接線後 client 送 { type:"craft", recipe_id:id },產物隨既有背包快照回來,零契約變更。
  const CRAFT_RECIPES = [
    { id: "pickaxe", out: "pickaxe", outQty: 1, inputs: [["wood", 3], ["stone", 2]] },
    // 升級配方鏈第一條:已合成的鎬子 + 素材 → 強化鎬(對齊伺服器 crafting.rs 的 reinforced_pickaxe)。
    // 鎬子本身當素材,給「採礦更快」攢出第二層進程目標;規則仍由伺服器查表扣料,前端只呈現。
    { id: "reinforced_pickaxe", out: "reinforced_pickaxe", outQty: 1, inputs: [["pickaxe", 1], ["wood", 2], ["stone", 4]] },
    // 武器:閉合「採集→合成→變強打怪」的合成側。與伺服器 crafting.rs 的 "weapon" 配方對齊
    // (stone×4 + ether×2 → weapon)。合出後拿不拿得到傷害加成由伺服器 combat 說了算,前端只呈現配方。
    { id: "weapon", out: "weapon", outQty: 1, inputs: [["stone", 4], ["ether", 2]] },
  ];
  // 擴地價格（與伺服器 src/economy.rs 對齊;規則只在伺服器,前端只拿來顯示與反灰提示）：
  // 基準 10 乙太、逐格線性漲（第 n+1 格 = 10×(n+1)）、一塊地最多擴 12 格。
  const EXPANSION_BASE_COST = 10;
  const MAX_EXPANSIONS = 12;
  // 購買第一塊領地的乙太費用——對齊 economy::PLOT_COST；前端只作顯示/反灰,扣款由伺服器決定。
  const CLAIM_PLOT_COST = 20;
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
    // C-4：可放置材料（dirt/stone）在每行右側多一個「🏗️選取」鈕，點後切換 selectedBuildMaterial。
    const PLACEABLE = new Set(["dirt", "stone"]);
    body.innerHTML = inv
      .map((s) => {
        const icon = ITEM_LOOK[s.item] || "";
        const name = ITEM_NAME[s.item] || s.item;
        const isSelected = selectedBuildMaterial === s.item;
        const placeBtn = PLACEABLE.has(s.item)
          ? `<button class="bag-place-btn${isSelected ? " selected" : ""}" data-material="${s.item}" title="${isSelected ? "取消放置選取" : "選取放置此材料"}">${isSelected ? "✅放置" : "🏗️選取"}</button>`
          : "";
        return `<div class="bag-row"><span class="bag-ico">${icon}</span>${name}<span class="bag-qty">×${s.qty}</span>${placeBtn}</div>`;
      })
      .join("");
    // 綁定 bag-place-btn 點擊事件（動態 HTML，每次重繪後重綁）。
    body.querySelectorAll(".bag-place-btn").forEach((btn) => {
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        const mat = btn.dataset.material;
        selectedBuildMaterial = selectedBuildMaterial === mat ? null : mat;
        updatePlaceModeHud();
        updateBagHud(inv); // 重繪以同步選取狀態
      });
    });
    // emoji 對報讀器無意義(會亂念或跳過),把中文品項名同步成標題鈕的 aria-label,讓盲人玩家
    // 「採到那瞬間」聽過播報後,之後想查背包現況時也讀得出來。title 給滑鼠玩家對稱的那一半。
    // 延續日夜/連線/採集的背包無障礙弧線。
    const label =
      "背包：" + inv.map((s) => `${ITEM_NAME[s.item] || s.item} ${s.qty}`).join("、");
    toggle.setAttribute("aria-label", label);
    toggle.setAttribute("title", label);
  }

  // 手上武器 HUD（接 PLAN「①手上武器小圖示 ②怎麼合武器一行提示」）:
  // 武器是合成產物（伺服器 crafting.rs "weapon" 配方 → ItemKind::Weapon），隨背包快照帶 qty 回來。
  // 伺服器 combat 接線後「身上有武器→自動戰鬥傷害更高」,前端只把「有沒有武器」如實畫出來,
  // 不在繪製碼算任何傷害（規則只在伺服器、為將來 WebXR 同後端留路）。
  // 有武器 → 上排 🗡️ pill 亮起、隱藏引導行;沒武器 → pill 收起、顯示一行合成引導。
  let weaponKnown = false; // 首次同步不把既有武器當「剛合出」播報（比照背包/乙太/血量的進場防誤報）
  let hadWeapon = false;   // 上一次快照是否持有武器,用來偵測「首次到手」只播報一次
  function updateWeaponHud(inv) {
    const pill = document.getElementById("hudWeapon");
    const hint = document.getElementById("hudWeaponHint");
    if (!pill || !hint) return;
    // 背包可能多筆 weapon 堆疊(防呆全部加總);item 是伺服器列舉字串,非玩家文字、無注入風險。
    const qty = (inv || []).reduce((n, s) => (s.item === "weapon" ? n + s.qty : n), 0);
    const has = qty > 0;
    if (has) {
      pill.textContent = qty > 1 ? `🗡️ 武器 ×${qty}` : "🗡️ 武器";
      // emoji 對報讀器無意義,pill 的 aria-label/title 給「手持武器」的中文(對齊背包無障礙作法)。
      const label = qty > 1 ? `手持武器 ${qty} 把,打怪更痛` : "手持武器,打怪更痛";
      pill.setAttribute("aria-label", label);
      pill.setAttribute("title", label);
      pill.classList.remove("hidden");
      hint.classList.add("hidden"); // 已有武器,不再顯示合成引導
    } else {
      pill.classList.add("hidden");
      // 還沒武器:一行引導,把「採集→合成→變強打怪」那步顯到 HUD（配方與合成台/伺服器 crafting.rs 一致）。
      hint.textContent = "🗡️ 合一把武器（🪨×4 ✨×2）打怪更痛";
      hint.classList.remove("hidden");
    }
    // 首次到手補一句 aria-live:pill 亮是純視覺,看不到畫面的玩家收不到「武器合出來了」。
    // 首次同步不報(進場/重連時既有武器不是剛合出)。延續採集/收成/血量的無障礙弧線。
    if (weaponKnown && has && !hadWeapon) announce("武器到手,打怪更痛了");
    hadWeapon = has;
    weaponKnown = true;
  }

  // C-4 放置模式 HUD：選取建造材料時亮起 pill；清除時隱藏。
  // pill 本身也是取消鈕（點擊 selectedBuildMaterial = null）。
  function updatePlaceModeHud() {
    const pill = document.getElementById("hudBuildMode");
    if (!pill) return;
    if (selectedBuildMaterial) {
      const icon = ITEM_LOOK[selectedBuildMaterial] || selectedBuildMaterial;
      const name = ITEM_NAME[selectedBuildMaterial] || selectedBuildMaterial;
      pill.textContent = `🏗️ ${icon} ${name}（右鍵放置 · 點此取消）`;
      pill.setAttribute("aria-label", `放置模式：${name}，右鍵點空格放置，點此取消`);
      pill.classList.remove("hidden");
      pill.onclick = () => { selectedBuildMaterial = null; updatePlaceModeHud(); };
    } else {
      pill.classList.add("hidden");
      pill.onclick = null;
    }
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
    // dock 圖示:有可合成的就點一顆黃點,玩家不開窗也知道「現在能合東西了」。
    const dock = document.getElementById("dockCraft");
    if (dock) dock.classList.toggle("dock-active", craftable > 0);
  }

  // 擴地面板:依「我的乙太」與「已購擴張格數」算下一格價,畫出價格提示與一顆「擴地」鈕。
  // owned 走快照未來欄位 expansions(伺服器接線後才有,沒有就當 0 防呆);乙太夠才亮鈕、
  // 達上限改顯示「已達上限」並反灰。點鈕只送 buy_expansion 意圖,伺服器查餘額扣乙太、農地
  // 多開一格,新地塊隨既有 fields 快照回來(零契約變更);夠不夠的反灰只是前端提示,扣款/
  // 開格規則仍只在伺服器(權威),前端不自行改地塊。me 缺欄位一律防呆,接線落地即生效。
  let lastExpandSig = null; // 上次重建用的「乙太|已購格數|有無領地」簽章——沒變就不重建,保住焦點與效能
  function updateExpandPanel(me) {
    const summary = document.getElementById("expandSummary");
    const body = document.getElementById("expandBody");
    const toggle = document.getElementById("expandToggle");
    if (!summary || !body || !toggle) return;
    const ether = (me && me.ether) || 0;
    const owned = (me && me.expansions) || 0; // 防呆:伺服器還沒接擴地 → 視為一格都還沒買
    const hasField = !!myField(); // 已有領地才顯示擴地；尚無領地則先顯示「購買領地」按鈕
    // 同合成台:快照每拍都來,但擴地面板只取決於乙太、已購格數、有無領地。沒變就提早返回,
    // 免得每拍重建把停在按鈕上的焦點打掉、手機白耗電。
    const sig = `${ether}|${owned}|${hasField ? "1" : "0"}`;
    if (sig === lastExpandSig) return;
    lastExpandSig = sig;
    body.innerHTML = "";

    // ── 購買領地（第一次取地）────────────────────────────────────────────────
    // 已登入且尚無領地：先買地才能擴地。隱藏擴地列、改顯示「購買領地」入口。
    if (myId && !hasField) {
      const canClaim = ether >= CLAIM_PLOT_COST;
      const claimRow = document.createElement("div");
      claimRow.className = "expand-row";
      const claimDesc = document.createElement("div");
      claimDesc.className = "expand-desc";
      const lack = !canClaim;
      // 文字說明費用；不足時價格標紅，與擴地缺錢一致。
      claimDesc.innerHTML = `購買領地 <span class="expand-cost${lack ? " lack" : ""}">✨ ${CLAIM_PLOT_COST}</span>`;
      claimRow.appendChild(claimDesc);
      const claimBtn = document.createElement("button");
      claimBtn.type = "button";
      claimBtn.className = "expand-btn";
      claimBtn.textContent = "購買領地";
      claimBtn.disabled = !canClaim;
      claimBtn.setAttribute(
        "aria-label",
        `花 ${CLAIM_PLOT_COST} 乙太購買第一塊領地${canClaim ? "" : "（乙太不足）"}`
      );
      claimBtn.title = canClaim ? `花 ${CLAIM_PLOT_COST} 乙太購買領地` : "需要 20 乙太";
      claimBtn.addEventListener("click", () => {
        if (claimBtn.disabled) return;
        // 只送意圖；伺服器驗餘額、扣乙太、分配序號，結果走 claim_plot_ok 廣播回來。
        try { ws.send(JSON.stringify({ type: "claim_plot" })); } catch {}
        announce(`花 ${CLAIM_PLOT_COST} 乙太購買領地`);
      });
      claimRow.appendChild(claimBtn);
      body.appendChild(claimRow);

      summary.textContent = canClaim ? "：可購地" : "";
      const claimLabel = canClaim
        ? `農地：可花 ${CLAIM_PLOT_COST} 乙太購買領地`
        : `農地：購買領地需要 ${CLAIM_PLOT_COST} 乙太`;
      toggle.setAttribute("aria-label", claimLabel);
      toggle.setAttribute("title", claimLabel);
      const dockE = document.getElementById("dockExpand");
      if (dockE) dockE.classList.toggle("dock-active", canClaim);
      return;
    }

    // ── 擴地（已有領地、繼續買格）──────────────────────────────────────────
    const atMax = owned >= MAX_EXPANSIONS;
    const cost = expansionCost(owned);
    const canBuy = !atMax && ether >= cost;

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
    // dock 圖示:乙太夠擴一格就點一顆黃點,不開窗也知道「現在能擴地了」。
    const dockE = document.getElementById("dockExpand");
    if (dockE) dockE.classList.toggle("dock-active", canBuy);
  }

  // 市場面板：附近掛單 + 自己的掛單管理 + 張貼新掛單。
  // listings = AOI 剔除後的快照；inv = 背包快照；ether = 我的乙太；uid = 我的 id。
  let lastMarketSig = null; // 快照簽章，內容未變就不重建面板（保住焦點、省 DOM 操作）。
  function updateMarketPanel(nearListings, inv, ether, uid) {
    const body = document.getElementById("marketBody");
    const summary = document.getElementById("marketSummary");
    const toggle = document.getElementById("marketTitle");
    if (!body || !summary) return;

    const others = nearListings.filter((l) => l.seller_id !== uid);
    const mine   = nearListings.filter((l) => l.seller_id === uid);

    // 簽章：掛單 id 集合 + 我的乙太（影響「夠不夠買」反灰）+ 我的背包（影響「能不能掛單」）。
    const sig = nearListings.map((l) => `${l.id}:${l.qty}`).join(",")
              + "|" + ether + "|" + (inv || []).map((s) => `${s.item}:${s.qty}`).join(",");
    if (sig === lastMarketSig) return;
    lastMarketSig = sig;
    body.innerHTML = "";

    // ── 附近掛單（其他玩家）─────────────────────────────────────────────────────
    const othersHead = document.createElement("div");
    othersHead.style.cssText = "color:var(--brass);font-weight:600;margin-bottom:4px;font-size:.85rem;";
    othersHead.textContent = others.length ? `附近掛單（${others.length}）` : "附近暫無掛單";
    body.appendChild(othersHead);

    for (const l of others) {
      const row = document.createElement("div");
      row.style.cssText = "display:flex;align-items:center;gap:6px;margin-bottom:4px;";
      const icon = ITEM_LOOK[l.item] || "?";
      const name = ITEM_NAME[l.item] || l.item;
      const total = Math.min(4294967295, l.price_per * l.qty); // 防溢位
      const canAfford = uid && ether >= total;
      const info = document.createElement("span");
      info.style.cssText = "flex:1;font-size:.8rem;";
      info.textContent = `${icon} ${name} ×${l.qty}  ✨ ${l.price_per}/個 (共 ${total})  by ${l.seller_name}`;
      row.appendChild(info);
      if (uid) {
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = "expand-btn";
        btn.textContent = "購買";
        btn.disabled = !canAfford;
        btn.title = canAfford ? `花 ${total} 乙太買` : "乙太不足";
        btn.addEventListener("click", () => {
          if (btn.disabled) return;
          try { ws.send(JSON.stringify({ type: "buy_listing", listing_id: l.id })); } catch {}
          announce(`購買 ${name} ×${l.qty}`);
        });
        row.appendChild(btn);
      }
      body.appendChild(row);
    }

    // ── 我的掛單（取消退貨）─────────────────────────────────────────────────────
    if (mine.length) {
      const sep = document.createElement("div");
      sep.style.cssText = "border-top:1px solid #3a4250;margin:6px 0 4px;";
      body.appendChild(sep);
      const mineHead = document.createElement("div");
      mineHead.style.cssText = "color:var(--brass);font-weight:600;margin-bottom:4px;font-size:.85rem;";
      mineHead.textContent = `我的掛單（${mine.length}）`;
      body.appendChild(mineHead);
      for (const l of mine) {
        const row = document.createElement("div");
        row.style.cssText = "display:flex;align-items:center;gap:6px;margin-bottom:4px;";
        const icon = ITEM_LOOK[l.item] || "?";
        const name = ITEM_NAME[l.item] || l.item;
        const info = document.createElement("span");
        info.style.cssText = "flex:1;font-size:.8rem;";
        info.textContent = `${icon} ${name} ×${l.qty}  ✨ ${l.price_per}/個`;
        row.appendChild(info);
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = "expand-btn";
        btn.textContent = "取消";
        btn.title = "取消掛單，物品歸還背包";
        btn.addEventListener("click", () => {
          try { ws.send(JSON.stringify({ type: "cancel_listing", listing_id: l.id })); } catch {}
          announce(`取消 ${name} 掛單`);
        });
        row.appendChild(btn);
        body.appendChild(row);
      }
    }

    // ── 張貼掛單（已登入才顯示）─────────────────────────────────────────────────
    if (uid) {
      const postItems = (inv || []).filter((s) => s.qty > 0);
      const sep2 = document.createElement("div");
      sep2.style.cssText = "border-top:1px solid #3a4250;margin:6px 0 4px;";
      body.appendChild(sep2);
      const postHead = document.createElement("div");
      postHead.style.cssText = "color:var(--brass);font-weight:600;margin-bottom:4px;font-size:.85rem;";
      postHead.textContent = "張貼掛單";
      body.appendChild(postHead);

      if (!postItems.length) {
        const empty = document.createElement("div");
        empty.style.cssText = "font-size:.8rem;color:#888;";
        empty.textContent = "背包是空的，無可掛單物品";
        body.appendChild(empty);
      } else {
        // 物品選擇
        const postRow = document.createElement("div");
        postRow.style.cssText = "display:flex;flex-wrap:wrap;gap:4px;align-items:center;";

        const selItem = document.createElement("select");
        selItem.style.cssText = "background:#1a2030;color:#c8d0e0;border:1px solid #3a4250;border-radius:4px;padding:2px 4px;font-size:.8rem;";
        for (const s of postItems) {
          const opt = document.createElement("option");
          opt.value = s.item;
          opt.textContent = `${ITEM_LOOK[s.item] || "?"} ${ITEM_NAME[s.item] || s.item}（有 ${s.qty}）`;
          selItem.appendChild(opt);
        }
        postRow.appendChild(selItem);

        const numQty = document.createElement("input");
        numQty.type = "number";
        numQty.min = "1";
        numQty.max = "9999";
        numQty.value = "1";
        numQty.placeholder = "數量";
        numQty.style.cssText = "width:54px;background:#1a2030;color:#c8d0e0;border:1px solid #3a4250;border-radius:4px;padding:2px 4px;font-size:.8rem;";
        postRow.appendChild(numQty);

        const numPrice = document.createElement("input");
        numPrice.type = "number";
        numPrice.min = "0";
        numPrice.max = "9999";
        numPrice.value = "1";
        numPrice.placeholder = "✨/個";
        numPrice.style.cssText = "width:54px;background:#1a2030;color:#c8d0e0;border:1px solid #3a4250;border-radius:4px;padding:2px 4px;font-size:.8rem;";
        postRow.appendChild(numPrice);

        const postBtn = document.createElement("button");
        postBtn.type = "button";
        postBtn.className = "expand-btn";
        postBtn.textContent = "掛單";
        postBtn.title = "張貼掛單，物品從背包移出";
        postBtn.addEventListener("click", () => {
          const item = selItem.value;
          const qty = Math.max(1, Math.min(9999, parseInt(numQty.value, 10) || 1));
          const price_per = Math.max(0, Math.min(9999, parseInt(numPrice.value, 10) || 0));
          try {
            ws.send(JSON.stringify({ type: "post_listing", item, qty, price_per }));
          } catch {}
          const name = ITEM_NAME[item] || item;
          announce(`掛單 ${name} ×${qty} ✨${price_per}/個`);
        });
        postRow.appendChild(postBtn);
        body.appendChild(postRow);
      }
    } else {
      // 訪客提示
      const hint = document.createElement("div");
      hint.style.cssText = "font-size:.8rem;color:#888;margin-top:6px;";
      hint.textContent = "登入後才能掛單或購買";
      body.appendChild(hint);
    }

    // 標題列摘要 + dock 活躍點
    const hasNearby = others.length > 0;
    summary.textContent = hasNearby ? `：${others.length} 筆` : "";
    if (toggle) {
      toggle.setAttribute("aria-label", hasNearby ? `市場：${others.length} 筆附近掛單` : "市場：無附近掛單");
    }
    const dockM = document.getElementById("dockMarket");
    if (dockM) dockM.classList.toggle("dock-active", hasNearby);
  }

  // NPC 商店面板：靠近商人才能互動（伺服器同樣驗距離）。
  // 顯示收購清單（我賣→NPC買）和販售清單（NPC賣→我買），各有數量輸入與按鈕。
  let lastShopSig = null;
  function updateShopPanel(npcList, me) {
    const body = document.getElementById("shopBody");
    const summary = document.getElementById("shopSummary");
    const dockBtn = document.getElementById("dockShop");
    if (!body || !summary) return;

    const SHOP_REACH_SQ = 96 * 96; // 對齊後端 SHOP_REACH = 96.0
    const nearNpc = me && npcList.find((npc) => {
      const dx = me.x - npc.x;
      const dy = me.y - npc.y;
      return dx * dx + dy * dy <= SHOP_REACH_SQ;
    });

    const isGuest_ = isGuest; // 訪客只能看，不能交易

    // 簽章：近/遠 + 乙太 + 背包 hash（控制重建頻率）
    const invSig = me ? (me.inventory || []).map((s) => `${s.item}:${s.qty}`).join(",") : "";
    const sig = `${!!nearNpc}|${me ? me.ether : 0}|${invSig}`;
    if (sig === lastShopSig) return;
    lastShopSig = sig;

    // dock 活躍點（靠近時亮）
    if (dockBtn) dockBtn.classList.toggle("dock-active", !!nearNpc);
    summary.textContent = nearNpc ? "：商人在附近" : "";

    if (!nearNpc) {
      body.innerHTML = '<div style="opacity:0.6;font-size:0.85em">走近公共農地旁的 🧑‍💼 商人才能使用商店</div>';
      return;
    }

    const ITEM_NAME_ = { wood: "木材", stone: "石頭", ether: "乙太", pickaxe: "鎬子", reinforced_pickaxe: "強化鎬", weapon: "武器" };
    const myEther_ = me ? me.ether : 0;
    const invMap = new Map((me ? me.inventory || [] : []).map((s) => [s.item, s.qty]));

    let html = "";

    // —— 賣給商人（NPC 收購）——
    html += `<div style="color:var(--brass);font-weight:bold;margin:2px 0 4px">📤 賣給商人（換乙太）</div>`;
    for (const entry of nearNpc.buy_list) {
      const name = ITEM_NAME_[entry.item] || entry.item;
      const have = invMap.get(entry.item) || 0;
      const maxSell = have;
      const canSell = !isGuest_ && have > 0;
      html += `<div class="craft-row" style="margin:3px 0;display:flex;align-items:center;gap:6px">
        <span style="flex:1">${name} ×<input id="shopSellQty_${entry.item}" type="number" min="1" max="${maxSell || 1}" value="1"
          style="width:40px;background:#1a1f26;color:var(--ink);border:1px solid #3a4250;border-radius:4px;padding:1px 3px"
          ${!canSell ? "disabled" : ""}></span>
        <span style="color:var(--brass)">+${entry.price_per}✨/個</span>
        <span style="opacity:0.7;font-size:0.82em">(持有：${have})</span>
        <button class="craft-btn" id="shopSellBtn_${entry.item}" ${!canSell ? "disabled" : ""}
          style="padding:2px 8px;font-size:0.85em">賣出</button>
      </div>`;
    }

    // —— 向商人購買（NPC 販售）——
    html += `<div style="color:var(--brass);font-weight:bold;margin:8px 0 4px">📥 向商人購買（花乙太）</div>`;
    for (const entry of nearNpc.sell_list) {
      const name = ITEM_NAME_[entry.item] || entry.item;
      const canAfford1 = myEther_ >= entry.price_per;
      const canBuy = !isGuest_ && canAfford1;
      html += `<div class="craft-row" style="margin:3px 0;display:flex;align-items:center;gap:6px">
        <span style="flex:1">${name} ×<input id="shopBuyQty_${entry.item}" type="number" min="1" max="99" value="1"
          style="width:40px;background:#1a1f26;color:var(--ink);border:1px solid #3a4250;border-radius:4px;padding:1px 3px"
          ${!canBuy ? "disabled" : ""}></span>
        <span style="color:#e0795f">-${entry.price_per}✨/個</span>
        <span style="opacity:0.7;font-size:0.82em">(餘額：${myEther_}✨)</span>
        <button class="craft-btn" id="shopBuyBtn_${entry.item}" ${!canBuy ? "disabled" : ""}
          style="padding:2px 8px;font-size:0.85em">購買</button>
      </div>`;
    }

    body.innerHTML = html;

    // 賣出按鈕事件
    for (const entry of nearNpc.buy_list) {
      const btn = document.getElementById(`shopSellBtn_${entry.item}`);
      if (!btn) continue;
      btn.addEventListener("click", () => {
        const qtyEl = document.getElementById(`shopSellQty_${entry.item}`);
        const qty = Math.max(1, parseInt(qtyEl?.value || "1", 10));
        if (ws && ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: "shop_sell", item: entry.item, qty }));
        }
      });
    }
    // 購買按鈕事件
    for (const entry of nearNpc.sell_list) {
      const btn = document.getElementById(`shopBuyBtn_${entry.item}`);
      if (!btn) continue;
      btn.addEventListener("click", () => {
        const qtyEl = document.getElementById(`shopBuyQty_${entry.item}`);
        const qty = Math.max(1, parseInt(qtyEl?.value || "1", 10));
        if (ws && ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: "shop_buy", item: entry.item, qty }));
        }
      });
    }
  }

  // 依伺服器廣播的每格 state/dry 畫出一塊地的耕地與作物階段。
  function drawOnePlot(camX, camY, f) {
    if (!f || !f.cells) return;
    const ts = f.tile_size;
    const mine = f.owner === myId;
    const isPublic = f.owner === PUB_FIELD_OWNER;
    const me = myId ? players.get(myId) : null;

    // 可互動：自己的地（私有），或公共農地（已登入均可）。
    const canInteract = mine || (isPublic && !!myId);
    // 夠近 = 已登入 + 可互動 + 在觸及範圍內；不可互動的地一律算「夠近」（反正點不動，不需淡出）。
    const reachable = canInteract
        ? (me ? withinFieldReach(f, me.x, me.y) : true)
        : true;
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
    if (!reachable && canInteract) ctx.globalAlpha = 0.55; // 可互動但太遠 → 淡出提示走近
    else if (!canInteract) ctx.globalAlpha = 0.82; // 別人的地（或未登入看公共地）壓一點

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

    // hover 高亮 + 腳下格指示:在「可互動且夠近」的農地（私有或公共）上畫——
    // 讓玩家知道「點下去會作用在這格」。公共農地用青綠色系與私有地黃銅區分。
    const hFill   = isPublic ? "rgba(74,184,160,0.12)"  : "rgba(255,210,74,0.12)";
    const hStroke = isPublic ? "rgba(74,184,160,0.9)"   : "rgba(255,210,74,0.9)";
    const fStroke = isPublic ? "rgba(74,184,160,0.55)"  : "rgba(255,210,74,0.55)";
    if (canInteract && reachable) {
      // 桌面 hover 高亮:游標所指的田格描一圈亮框 + 淡填。手機無 hover 自然不畫。
      if (hoverScreen) {
        const t = fieldTileAtScreen(hoverScreen.x, hoverScreen.y, f);
        if (t) {
          const hx = f.origin_x + t.col * ts - camX;
          const hy = f.origin_y + t.row * ts - camY;
          ctx.fillStyle = hFill;
          ctx.fillRect(hx + 1, hy + 1, ts - 2, ts - 2);
          ctx.strokeStyle = hStroke;
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
          ctx.strokeStyle = fStroke;
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

    // 周圍畫一圈邊框：自己的黃銅亮框、公共農地青綠框、別人的暗一點。
    ctx.strokeStyle = mine ? "#c9a24b" : isPublic ? "#4ab8a0" : "#8a7340";
    ctx.lineWidth = 3;
    ctx.strokeRect(fx - 2, fy - 2, fw + 4, fh + 4);

    // 柵欄:把田圈成「圍起來的農莊」。有 fence.png(蒸汽龐克 sprite)就沿田邊圍一圈、
    // 底邊中段留柵門缺口;沒載好退回程式木樁(artOk 把關,跟 tree/rock 同一套 fallback)。
    drawFence(fx, fy, fw, fh, ts);

    // 田地名字標籤：自己的→「你的乙太田」，公共→「公共農地」，別人的→地主名。
    const owner = players.get(f.owner);
    const label = mine ? "你的乙太田 🌱"
        : isPublic ? "公共農地 🌿（誰種誰得）"
        : `${owner ? owner.name : "拓荒者"} 的乙太田`;
    ctx.fillStyle = (mine || isPublic) ? "rgba(232,224,207,0.9)" : "rgba(232,224,207,0.7)";
    ctx.font = "13px system-ui, sans-serif";
    ctx.textAlign = "center";
    ctx.fillText(label, fx + fw / 2, fy - 8);

    ctx.restore();

    if (canInteract && !reachable) {
      ctx.fillStyle = "rgba(232,224,207,0.85)";
      ctx.font = "12px system-ui, sans-serif";
      ctx.textAlign = "center";
      const hintText = mine ? "(走近一點才能照顧)" : "(走近公共農地才能耕作)";
      ctx.fillText(hintText, fx + fw / 2, fy + fh + 18);
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

  // fence.png:192×32 = 6 件×32px autotile(欄 0 水平 rail / 1 垂直 rail / 2 角=連 E+S,
  // 靠旋轉處理四個方向,規格見 docs/ASSET_INTEGRATION.md §3)。一律是純繪製、不嵌任何規則,
  // 將來 WebXR renderer 可各自決定怎麼把「同一塊地」圍起來。
  const FENCE_TS = 32;
  // 在 (cx,cy) 畫一件柵欄(以中心對齊,可旋轉),放大成 size 對齊田格(48)。
  function fencePiece(cx, cy, piece, rotDeg, size) {
    ctx.save();
    ctx.translate(cx, cy);
    if (rotDeg) ctx.rotate((rotDeg * Math.PI) / 180);
    ctx.drawImage(ART.fence, piece * FENCE_TS, 0, FENCE_TS, FENCE_TS, -size / 2, -size / 2, size, size);
    ctx.restore();
  }
  // sprite 版:沿田邊外圍一圈柵欄(每件一格、底部對齊田格線),底邊中段留一格柵門缺口,
  // 像星露谷把農地圈起來、留個入口。四角用 index 2 旋轉接圖。
  function drawFenceSprite(fx, fy, fw, fh, ts) {
    const cols = Math.round(fw / ts);
    const rows = Math.round(fh / ts);
    const gateCol = Math.floor(cols / 2); // 底邊中段留柵門缺口(有入口才像家園,不是全封死)
    const left = fx - ts / 2, right = fx + fw + ts / 2;
    const top = fy - ts / 2, bottom = fy + fh + ts / 2;
    // 上下橫桿(底邊跳過柵門那格)
    for (let i = 0; i < cols; i++) {
      const cx = fx + (i + 0.5) * ts;
      fencePiece(cx, top, 0, 0, ts);
      if (i !== gateCol) fencePiece(cx, bottom, 0, 0, ts);
    }
    // 左右直桿
    for (let j = 0; j < rows; j++) {
      const cy = fy + (j + 0.5) * ts;
      fencePiece(left, cy, 1, 0, ts);
      fencePiece(right, cy, 1, 0, ts);
    }
    // 四角:piece 2(基準連 E+S),依方向旋轉——左上 0、右上 90、右下 180、左下 270。
    fencePiece(left, top, 2, 0, ts);
    fencePiece(right, top, 2, 90, ts);
    fencePiece(right, bottom, 2, 180, ts);
    fencePiece(left, bottom, 2, 270, ts);
  }

  // 柵欄入口:有 fence.png 走 sprite,沒載好退回下面程式畫的木樁(永遠有得畫,不卡玩)。
  function drawFence(fx, fy, fw, fh, ts) {
    if (artOk("fence")) { drawFenceSprite(fx, fy, fw, fh, ts); return; }
    drawFencePosts(fx, fy, fw, fh);
  }

  // fallback:沒美術素材時程式畫的木樁 + 兩條橫桿,沿田邊立等距木樁,讓田看起來像圈起來的農莊。
  function drawFencePosts(fx, fy, fw, fh) {
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
    // 先看是不是點到小地圖收合鈕 / 縮放鈕(純 UI 切換,不需連線、也不該被當成農作意圖)。
    if (minimapToggleHit(clientX, clientY)) return;
    if (minimapZoomHit(clientX, clientY)) return;
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
          spawnGatherHit(gn); // 揮擊命中 + 碎屑噴出(治「沒有採集動作」)
          return;
        }
      }
    }
    const rect = canvas.getBoundingClientRect();
    const wx = clientX - rect.left + lastCam.x;
    const wy = clientY - rect.top + lastCam.y;

    // C-2 挖掘：點到實心地形格且玩家在 DIG_REACH（80px）內 → 送 dig。
    // 挖掘優先於農耕（地形格與農地層不同,不互排斥），但在採集之後
    // （採集節點浮在地形之上，點到節點應採集不是挖土）。
    if (me) {
      const digKind = tileKindAt(wx, wy);
      if (digKind !== "empty") {
        const DIG_REACH = 80;
        const dx = wx - me.x, dy = wy - me.y;
        if (dx * dx + dy * dy <= DIG_REACH * DIG_REACH) {
          ws.send(JSON.stringify({ type: "dig", wx, wy }));
          spawnTapFlash(wx, wy); // 確認回饋
          return;
        }
      }
    }

    const pf = myId ? pubField() : null; // 已登入才有公共農地互動資格

    // 點在公共農地格內且已登入且夠近 → 直接送（不管有沒有自己的地）。
    if (me && pf && withinFieldReach(pf, me.x, me.y)) {
      const inPub = wx >= pf.origin_x && wx < pf.origin_x + pf.cols * pf.tile_size
          && wy >= pf.origin_y && wy < pf.origin_y + pf.rows * pf.tile_size;
      if (inPub) {
        ws.send(JSON.stringify({ type: "farm", x: wx, y: wy }));
        markTendedOnce();
        spawnTapFlash(wx, wy);
        return;
      }
    }

    // 沒有自己的地（訪客 / 尚未購買）：提示可用公共農地或購買。
    if (me && !f) {
      if (now - lastReachHint > 2500) {
        const hint = myId
            ? "花乙太購買農地，或走到公共農地（🌿）耕作哦。"
            : "登入後就有自己的乙太田可以照顧哦。";
        addChat("系統", hint);
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
    {
      const gn = nearestHarvestable(me);
      if (gn) {
        ws.send(JSON.stringify({ type: "gather" }));
        markGatheredOnce();
        spawnGatherHit(gn); // 揮擊命中 + 碎屑噴出
        return;
      }
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
    const pf = myId ? pubField() : null;

    // 站在公共農地旁（已登入）→ 允許動作（私有地取不到或太遠時也適用）。
    if (pf && withinFieldReach(pf, me.x, me.y)) {
      ws.send(JSON.stringify({ type: "farm", x: me.x, y: me.y }));
      markTendedOnce();
      spawnTapFlash(me.x, me.y);
      return;
    }

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
  // C-4 建造：右鍵點擊在選取材料且目標為空格時放置。
  canvas.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const me = myId ? players.get(myId) : null;
    if (!me || !selectedBuildMaterial) return;
    const rect = canvas.getBoundingClientRect();
    const wx = e.clientX - rect.left + lastCam.x;
    const wy = e.clientY - rect.top + lastCam.y;
    const targetKind = tileKindAt(wx, wy);
    if (targetKind !== "empty") return; // 只能放在空格
    const DIG_REACH = 80;
    const dx = wx - me.x, dy = wy - me.y;
    if (dx * dx + dy * dy > DIG_REACH * DIG_REACH) return; // 太遠
    if ((myInv.get(selectedBuildMaterial) || 0) <= 0) return; // 背包空了
    ws.send(JSON.stringify({ type: "place", wx, wy, material: selectedBuildMaterial }));
    spawnTapFlash(wx, wy);
  });
  // 桌面 hover:記住游標位置以高亮所指田格;移出畫布就清掉(別留個鬼影高亮)。
  canvas.addEventListener("mousemove", (e) => { hoverScreen = { x: e.clientX, y: e.clientY }; });
  canvas.addEventListener("mouseleave", () => { hoverScreen = null; });

  // 螢幕座標 → 農地格 {col,row};落在田格範圍內才回傳,否則 null。純表現用
  // (高亮游標所指格),不參與任何互動判定——互動仍送原始世界座標給權威伺服器決定。
  function fieldTileAtScreen(sx, sy, f) {
    if (!f) f = myField();
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

  // ---- 星露谷風工具列(dock)+ 可開關視窗 ----
  // 總監定調:左上常駐欄太佔空間。常駐只留精簡狀態(乙太/生命/線上/時段);背包/合成/擴地/操作說明
  // 改成 dock 一排小圖示,點圖示才開對應視窗。規則:同時只開一個;再點同一顆、按 ✕、按 Esc、
  // 點世界任一處都關。視窗內容沿用既有渲染(updateBagHud / updateCraftPanel / updateExpandPanel
  // 寫進 #bagBody / #craftBody / #expandBody),這裡只管「哪個視窗開著」。純前端互動,不碰遊戲規則。
  function initDock() {
    const dock = document.getElementById("hudDock");
    if (!dock) return;
    let openWin = null; // 目前開著的視窗元素
    let openBtn = null; // 開它的 dock 鈕(關閉時把鍵盤焦點還回去)

    function closeWin(returnFocus = true) {
      if (!openWin) return;
      openWin.classList.add("hidden");
      const btn = openBtn;
      if (btn) btn.setAttribute("aria-expanded", "false");
      openWin = null;
      openBtn = null;
      // 用鍵盤開的才把焦點還回圖示;點世界關的別硬搶焦點(玩家正在操作地表)。
      if (returnFocus && btn) btn.focus();
    }

    function openWinFor(btn) {
      const win = document.getElementById(btn.dataset.win);
      if (!win) return;
      if (openWin === win) { closeWin(); return; } // 再點同一顆 = 關
      closeWin(false); // 先收掉別的(同時只開一個)
      win.classList.remove("hidden");
      btn.setAttribute("aria-expanded", "true");
      openWin = win;
      openBtn = btn;
      // 開窗把焦點移到關閉鈕:鍵盤/報讀器玩家可直接操作、Esc 也能關。
      const closeBtn = win.querySelector(".win-close");
      if (closeBtn) closeBtn.focus();
    }

    for (const btn of dock.querySelectorAll(".dock-btn")) {
      btn.addEventListener("click", () => openWinFor(btn));
    }
    // 把「依 dock 鈕 id 切換視窗」暴露給全域 keydown(B/C/H 捷徑用)。走 openWinFor 同一條路,
    // 因此 toggle 開關、同時只開一個、焦點移到關閉鈕等行為全與點圖示完全一致。
    toggleDockWin = (btnId) => {
      const btn = document.getElementById(btnId);
      if (btn) openWinFor(btn);
    };
    // 每個視窗右上的 ✕ 關閉自己。
    for (const x of document.querySelectorAll(".hud-window .win-close")) {
      x.addEventListener("click", () => closeWin());
    }
    // Esc 關掉開著的視窗(沒開窗時不攔,留給聊天等既有 Esc 行為)。
    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && openWin) { e.preventDefault(); closeWin(); }
    });
    // 點世界任一處關掉視窗(星露谷手感:點地表收選單)。capture 階段先收掉、不 preventDefault——
    // 走動/採集等地表操作照常觸發,只是順手把選單關了。
    const canvas = document.getElementById("game");
    if (canvas) canvas.addEventListener("pointerdown", () => closeWin(false), true);
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
    initChatToggle();
    initDock(); // 星露谷風 dock + 視窗(取代原本 help/bag/craft/expand 的 inline 收合)
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
