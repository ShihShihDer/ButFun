# ButFun 開發待辦（BACKLOG）

> `/goal` 自走時的工作清單。**由上往下做**，一次一個項目。
> 完成就把 `[ ]` 改成 `[x]`，並在該項下補一行「✅ 做了什麼」。
> 遵守 `CLAUDE.md` 的邊界與品質閘門。每個項目都附「驗收標準」，達成才算完成。

## 已完成

- [x] **Phase 0-A：權威伺服器即時多人骨架**
  ✅ Rust(axum) WebSocket 權威伺服器、15Hz tick、客戶端送輸入、廣播快照；
  原生 canvas 前端可進場、WASD 移動、鏡頭跟隨、看到其他玩家。
  ✅ 轉發迴圈修 bug（2026-06-05）：先前 `ws.rs` 用 `while let Ok(msg) = rx.recv()`
  轉發廣播——tokio broadcast 在客戶端跟不上時回 `Err(Lagged)`，此寫法會直接
  break 把人靜默踢下線（手機網路一抖／分頁切背景一下跟不上 15Hz 快照就斷線，
  對手機上玩的療癒多人世界是真體驗 bug）。把「遇到 recv 錯誤該繼續還是停」抽成
  純函式 `forward_action`：`Lagged` 只跳過丟掉的快照繼續轉（下一則 1/15 秒就到、
  畫面自然追回），只有 `Closed`（伺服器關頻道）才結束。加 2 個單元測試，
  `cargo test` 79 綠、clippy 乾淨、伺服器二進位啟動正常（埠被正式服務占用屬預期）。
- [x] **Phase 0-B：聊天**
  ✅ Enter 開啟輸入，訊息經伺服器廣播給所有人。
  ✅ 輸入加固（2026-06-05）：聊天是最後一個還只做 inline `trim`+`take(200)`、沒抽成
  可測函式也沒過濾控制字元的公開輸入向量（聊天會廣播給所有玩家，壞客戶端可送含
  換行／NUL 的訊息廣播出多行或破壞顯示／偽造介面的內容）。抽出純函式 `sanitize_chat`
  （濾控制字元 + trim + 依字元截 200，清空回 `None` 不廣播），常數 `MAX_CHAT_CHARS`
  集中此公開輸入邊界（對齊建議 1000／署名 24／玩家名 24）。延續訪客名字／物種、建議
  長度上限的硬化弧線。加 5 個單元測試，`cargo test` 77 綠、clippy 乾淨、伺服器二進位
  啟動正常（埠被正式服務占用屬預期）。
  ✅ 聊天送達可靠性（2026-06-05）：先前聊天和 15Hz 快照共用同一條廣播頻道——上一輪
  「手機 Lagged 不踢人」修復讓跟不上的客戶端不再被踢，卻浮現後續缺口：客戶端追快照、
  收到 `Lagged` 跳過舊訊息時，會把同段時間捲過的**聊天**一起丟掉，而聊天是一次性事件
  （快照能自我修正、漏了無害；聊天漏了該客戶端就永久看不到那行）。把聊天分到獨立的
  `tx_chat` 廣播頻道：高頻快照灌滿不再淹到低頻聊天，聊天量極低幾乎不可能 Lagged，得以
  可靠送達。轉發任務改 `select!` 兩條頻道、各自沿用 `forward_action` 的 Lagged/Closed
  判斷。加 1 個頻道隔離測試，`cargo test` 84 綠、clippy 乾淨、伺服器二進位啟動正常
  （埠被正式服務占用屬預期）。
- [x] **Phase 0-C：遊戲內建議箱**
  ✅ 前端 💡 表單 → `POST /api/suggestions` → 存 `data/suggestions.jsonl`。
  ✅ 持久化載入防線（建議讀路徑也過 sanitizer，2026-06-05）：控制字元過濾原本只加在
  **寫入**路徑（`add`/`sanitize`），但建議是「存檔又重載」的持久化結構——`load_from_disk`
  直接 `serde_json::from_str` 收進記憶體、由公開 `GET /api/suggestions` 回出、維護者又常
  直接在終端機讀檔三角化。那道硬化 landing **之前**寫進的舊行、或被手動編輯／損毀的行，
  仍會帶 `ESC`(0x1B)／`NUL`／`\r` 原樣輸出、注入 ANSI 轉義偽造顯示。建議是唯一「存檔又重載」
  卻在**載入路徑沒驗證**的結構（對照 `field.rs::from_tiles`／`Crop::is_loadable`／
  `positions::spawn_at` 都在載入時驗壞值）。把讀路徑抽成純函式 `parse_and_sanitize` 並讓每則
  再過一次同一個 `sanitize`，輸出從此用「實際會被存下的乾淨內容」當單一真實來源，不論磁碟上
  那行是何時、被什麼寫進去的。刻意**不改寫／不刪除**磁碟檔（不破壞玩家資料），只過濾載進記憶體、
  回給 GET 的內容；清乾淨後變空的舊行也丟掉，比照寫入路徑「空建議不存」。加 4 個測試，
  `cargo test` 101 綠、clippy 乾淨、伺服器二進位啟動正常（埠被正式服務占用屬預期）。
  ✅ 前端修 bug（視窗失焦移動鍵卡住，2026-06-05）：`game.js` 只有 `keyup` 會清移動鍵——
  但玩家按著 w／方向鍵時切走分頁或視窗失焦時，瀏覽器多半不再送 `keyup`，`keys[dir]` 卡在
  `true`，伺服器持續整合位置，角色在背景一直走（切回來人已飄到別處／撞牆）。對「常在手機／
  分頁間切換」的療癒多人世界是真體驗 bug，且延續「角色別在玩家沒在控時亂走」（建議箱 textarea
  吃按鍵）的修復家族。加 `window` `blur` 與 `document` `visibilitychange`（隱藏時）兜底：
  失焦就清掉所有移動鍵並送出「停止」意圖。bump `index.html` 的 `game.js?v` 快取版號。
  純前端、`cargo test` 91 綠不受影響。
  ✅ 前端修 bug（建議箱 textarea 吃按鍵，2026-06-05）：`game.js` 的 keydown 守衛只擋
  `INPUT`、沒擋 `TEXTAREA`——而建議箱內容 `#suggestText` 正是 `<textarea>`。玩家寫
  回饋時，內含的 `w/a/s/d`／方向鍵會被遊戲攔截 `preventDefault` 吃掉、打不進去，角色
  還在背景亂走，Enter 也被搶去 focus 聊天而無法換行。建議箱是整個 devloop 收回饋的
  主要管道，這 bug 直接傷到它。守衛改為「焦點在任何文字輸入欄（`INPUT` 或 `TEXTAREA`）
  就完全不攔截遊戲按鍵」，語意更清楚且保留所有既有行為（聊天 input 照舊、Enter 在空白
  處仍開聊天）。bump `index.html` 的 `game.js?v` 快取版號讓玩家拿到新檔。純前端、
  `cargo test` 91 綠不受影響。
  ✅ 輸入加固（2026-06-05）：先前 `text`/`from` 無長度上限，公開 endpoint 可被
  灌入任意大小的行膨脹建議檔。抽出純函式 `sanitize`（trim + 依字元截斷 + 空署名退回
  匿名），署名截 24 字（對齊 `sanitize_name`）、內容截 1000 字，集中與聊天截 200 字
  同類的輸入邊界。加 5 個單元測試，`cargo test` 68 綠、伺服器啟動正常。
  ✅ 輸入加固補洞之三（空建議繞過 endpoint 判空，2026-06-05）：`post_suggestion` 用
  `new.text.trim().is_empty()` 擋空，但 `trim` 只去空白、不濾控制字元——一則「全控制字元」
  的內容（如 `\0`/`ESC`，皆非空白）會通過這道 raw 檢查，接著被 `sanitize` 濾光成空字串、
  仍寫進 `data/suggestions.jsonl` 留下空建議垃圾紀錄（`control_only_text_becomes_empty`
  測試正好證明 sanitize 後變空）。把「擋空」下沉到 sanitize/add 層：`sanitize` 改回
  `Option<Suggestion>`（清乾淨後 text 空回 `None`，比照 `sanitize_chat` 的模式），`add`
  連帶回 `Option`、空就不存任何東西，endpoint 依此回 400。擋空從此用「實際會被存下的內容」
  這個單一真實來源，而非較弱的 raw `trim`。改 1 個既有測試、補強成涵蓋全控制字元/全空白/
  含可見字元三情境，`cargo test` 91 綠、clippy 乾淨、伺服器二進位啟動正常（埠被正式服務占用屬預期）。
  ✅ 輸入加固補洞之二（控制字元，2026-06-05）：建議的 `sanitize` 是公開輸入硬化弧線裡
  唯一還只做 `trim`+`take`、沒濾控制字元的 sanitizer——而建議經未驗身的
  `POST /api/suggestions` 進來、又由公開的 `GET /api/suggestions` 回出，且維護者多半直接
  在終端機讀 `data/suggestions.jsonl` 三角化，`ESC`(0x1B) 可被用來注入 ANSI 轉義偽造/
  破壞顯示（`NUL`/`\r` 同理）。比照 `sanitize_name`/`sanitize_chat` 先濾控制字元（不佔截斷
  額度）：署名是單行身分欄位、濾掉全部控制字元；內容是多行回饋（前端 `<textarea>`），保留
  換行 `\n` 讓玩家分段、只濾換行以外的控制字元（換行存進 JSONL 會被 serde 轉義、不拆紀錄）。
  補齊訪客名字→物種→聊天→建議的公開輸入硬化弧線最後一塊。加 4 個測試，`cargo test` 91 綠、
  clippy 乾淨、伺服器二進位啟動正常（埠被正式服務占用屬預期）。
- [x] **Phase 0-D：自動測試閘門**
  ✅ 移動整合抽成 `Player::step`，加上單元測試；`cargo test` 全綠。
- [x] **Phase 0-F：帳號身份模型（provider 無關）+ Google OAuth**
  ✅ provider 無關 `User` 模型(`src/users.rs`,目前以 `data/users.jsonl` 持久化,
  之後接 0-E Postgres 時直接 swap);Google OAuth flow(`src/auth.rs`)含
  `/auth/google/start|callback|me|logout`、CSRF state cookie、HMAC-SHA256 簽章
  session cookie(stateless);WebSocket 升級前讀 cookie → 同 Google 帳號跨裝置
  /重連即同玩家;前端「以 Google 登入」按鈕 + 登入後跳過進場畫面、HUD 顯示用戶名
  與登出。`.env`/EnvironmentFile 載入秘密,gitignored 不入 repo。
  ✅ 跨裝置同玩家補洞（同帳號多連線互踢，2026-06-05）：已登入玩家 `player.id == user.id`，
  所以同一帳號開兩個分頁／兩台裝置時，兩條 WebSocket 連線共用同一個玩家 id——正是本項招牌
  「同 Google 帳號跨裝置即同玩家」會踩到的情境。先前 `cleanup` 無條件 `players.remove(id)`：
  關掉其中一個分頁，會把另一條還連著的同帳號 session 一起從世界移除（不再進快照、輸入被靜默
  丟棄，那條連線等於憑空變死人）。新增 `src/connections.rs` `ConnectionCounts`（每個 id 的在線
  連線數，`acquire`/`release` 純可測）：第一條連線才從記憶位置建立玩家，之後同帳號連線只增計數、
  共用既有權威狀態（不用舊存檔覆蓋當前位置、避免瞬移），最後一條離線（計數歸零）才真正移除、
  記位置、廣播 `PlayerLeft`（不再對「人還在世界」誤送離線造成閃爍）。鎖序固定「先 players 再
  conns」與遊戲迴圈無交集，無死鎖。加 6 個單元測試，`cargo test` 97 綠、clippy 乾淨、伺服器二進位
  啟動正常（埠被正式服務占用屬預期）。
  ✅ 持久化載入防線（users 讀路徑也過 sanitizer，2026-06-05）：名字濾控制字元的硬化（`sanitize_name`）原本只在
  **寫入**路徑（`find_or_create`）；但 `data/users.jsonl` 是「存檔又重載」的持久化結構——`load_from_disk` 直接
  `serde_json::from_str::<User>` 收進記憶體，而 `User.name` 正是已登入玩家進場後**廣播給所有人**的聊天 `from` 標籤與
  HUD 顯示名（`ws.rs` authed 路徑 `name: user.name`）。那道硬化 landing**之前**建立的舊帳號、或被手動編輯／損毀的行，
  殘留的 `NUL`／`ESC`(0x1B)／換行會原樣載入並隨登入玩家廣播出去，注入 ANSI 轉義偽造顯示或廣播多行。這正是上一輪
  `suggestions::parse_and_sanitize`（建議讀路徑）修的同一類缺口的孿生——`users.jsonl` 是另一個「存檔又重載卻在載入
  路徑沒驗證」的結構。把讀路徑抽成純函式 `parse_and_sanitize`，每筆的**顯示用**欄位（name/species）再過一次對應
  sanitizer；刻意不碰 `provider`／`external_id`（登入比對鍵，要與 OAuth 送來的值逐字相符，動了會讓既有帳號對不上、
  形同丟失帳號），也不改寫／不刪除磁碟檔（不破壞玩家資料），只過濾載進記憶體的內容。延續
  `field.rs::from_tiles`／`positions::spawn_at`／`suggestions::parse_and_sanitize` 的載入時驗證脈絡。加 3 個測試，
  `cargo test` 104 綠、clippy 乾淨、伺服器二進位啟動正常、`healthz` 回 `ok`。
- [x] **Phase 0-H 雛形：Cloudflare Tunnel 上線**
  ✅ 自架 + `cloudflared` 反向通道,公開於 https://peregrine.but-fun.com;手冊
  與設定範例已在 repo。

## 進行中 / 下一步（由上往下）

- [ ] **Phase 0-E：Postgres 持久化**
  把玩家位置（之後含背包 / 農地）存進 Postgres，伺服器重啟後玩家回到原位。
  - 加入 `sqlx`（Postgres、非同步），`DATABASE_URL` 走環境變數。
  - 在 `state.rs` 抽換點後面接一個 `PgStore`；無 `DATABASE_URL` 時退回現有記憶體模式，方便本機跑。
  - 加 migration 建 `players` 表（id, name, species, x, y, updated_at）。
  - 玩家進場時若 DB 有舊紀錄就載入；定期 / 離線時寫回。
  - 驗收：設好 `DATABASE_URL` 跑起來，移動後重啟伺服器，重新進場位置仍在；`cargo test` 全綠。
  - ✅ 前置（記憶體版抽換點，2026-06-05）：新增 `src/positions.rs` `PositionStore`
    （`recall` / `remember` + 純函式 `spawn_at`），已登入玩家離線時記下位置、
    重連 `spawn_at(recall)` 回到原位（訪客 id 隨機故不記，避免 map 無界成長）。
    刻意做成可抽換點：接 Postgres 時把這個 store swap 成 `PgStore`（同介面）即可，
    不用動 ws / 遊戲迴圈。仍待做：引入 `sqlx` + `DATABASE_URL` + migration + 跨重啟
    持久化（架構級／大相依，留待人決策後再上）。加 6 個單元測試，`cargo test` 25 綠。
  - ✅ 前置補強（乙太納入記憶體前置，2026-06-05）：先前重連只記回位置，收成的乙太
    仍固定歸零（網路抖動／換頁重連就丟失當場進度，且與「記住位置」不一致）。把
    `PositionStore` 的儲值從 `(x,y)` 廣化為 `Saved{x,y,ether}`，`remember`/`recall`
    同步帶上乙太；ws 重連讀回、離線寫下乙太。維持同一個可抽換點與「只記已登入玩家」
    原則，跨重啟持久化仍待 0-E。加 1 個乙太 round-trip 測試（既有 3 測試改對應新介面），
    `cargo test` 62 綠、伺服器啟動正常。
  - ✅ 持久化載入防線之三（壞掉的進場座標，2026-06-05）：`spawn_at` 先前直接信任
    recalled 位置——記憶體版的值來自 `Player::step` 已夾過的座標本就合法，但接 0-E 後
    Postgres float 欄位可能存進 `NaN`／`Inf`／界外值，不檢查就會把玩家生在地圖外、或讓
    座標變非有限（畫面/鏡頭/農地距離判斷全跟著壞）。給 `spawn_at` 補上契約「回傳一定有限
    且落在世界範圍內」：非有限退回地圖中央、界外夾回邊界（沿用 `Player::step` 的
    clamp 語意）。延續 `cell_at` 非有限座標、`from_tiles`／`Crop::is_loadable` 壞值的
    持久化載入防線脈絡。加 2 個測試，`cargo test` 86 綠、clippy 乾淨、伺服器二進位
    啟動正常（埠被正式服務占用屬預期）。

- [x] **Phase 0-F-1：補 auth 純邏輯單元測試**
  `sign_session` / `verify_session`(含偽造 token 拒絕)、`read_cookie`(多 cookie、
  含空白)、`constant_time_eq`、`sanitize_name`。純函式、無 IO、易測,補上即可
  push。驗收:`cargo test` 全綠且涵蓋上述函式。
  ✅ `src/auth.rs` 加 10 個測試:簽章 round-trip、改簽章/換 secret/換 uid/格式
  錯誤一律拒絕、`constant_time_eq` 含長度不等、`read_cookie` 多 cookie+空白+
  缺失+不誤中前綴;`src/users.rs` 加 5 個 `sanitize_name` 測試(去頭尾空白、
  空字串退回「拓荒者」、以字元而非位元組截到 24)。`cargo test` 19 passed。
  ✅ 後續(2026-06-05):消除 `ws.rs` 訪客進場與 `users.rs` 重複的名字清理邏輯——
  `sanitize_name` 改 `pub` 共用,並新增 `sanitize_species`(去空白、空退回
  `DEFAULT_SPECIES`)取代 `ws.rs` 原本 inline 且無測試的物種清理。補 3 個
  `sanitize_species` 測試,`cargo test` 28 綠。
  ✅ 輸入加固補洞(2026-06-05):聊天內容 `sanitize_chat` 已濾控制字元,但
  `sanitize_name`/`sanitize_species` 沒濾——而名字會成為**廣播給所有人**的聊天
  `from` 標籤與 HUD 顯示名。壞客戶端可把換行/NUL 塞進名字、繞過聊天自己的過濾,
  廣播出多行或破壞顯示/偽造介面的內容(物種同為訪客完全可控的顯示用單行欄位)。
  兩個共用 sanitizer 比照 `sanitize_chat`:先濾控制字元(不佔截斷額度)、再 trim、
  依字元截斷、清空退回預設。補齊訪客名字→建議→聊天公開輸入硬化弧線裡身分欄位這一塊。
  加 4 個測試,`cargo test` 83 綠、clippy 乾淨、伺服器啟動正常。

- [ ] **Phase 0-G：種田起源（地球人 / 繼承農莊）—— 療癒核心**
  讓世界「玩起來有感覺」的第一個玩法循環。
  - 地表一塊可耕地：點格子翻土 → 種乙太作物 → 澆水 → 隨日夜成長 → 收成得乙太。
  - 伺服器驅動的日夜 / 成長計時；農地狀態持久化（接 0-E）。
  - 前端顯示耕地格與作物階段。
  - 驗收：種下、澆水、過一段時間收成拿到乙太；重啟後農地狀態還在；`cargo test` 涵蓋成長邏輯。
  - ✅ 前置（純成長邏輯地基，2026-06-05）：新增 `src/crops.rs` `Crop` 模型
    （`plant` / `water` / `grow` / `harvest` + 純函式 `stage_for`）。療癒迴圈刻意做成
    「澆水才會長」：濕度隨成長被消耗，乾了停滯需再澆；累積成長 `Seed→Sprout→Ripe`，
    成熟才能 `harvest` 拿乙太並重置該格。無 IO、不碰 ws/遊戲迴圈，加 10 個單元測試，
    `cargo test` 38 綠。仍待做：接遊戲迴圈每 tick 成長、前端畫耕地與
    作物階段、狀態持久化（接 0-E）。接線時移除 `crops.rs` 暫時的 `allow(dead_code)`。
  - ✅ 前置（農地格資料結構，2026-06-05）：新增 `src/field.rs` `Field`（固定位置/大小的
    `Tile` 格陣列：`Untilled`→`Tilled`→`Planted(Crop)`）+ 純函式 `cell_at`（世界座標→格）。
    互動：`till` / `plant` / `water` / `harvest`（成熟才給乙太並回到空土）/ `tick(dt)`
    （讓全地作物成長）。延續「純邏輯可測、不碰 ws/遊戲迴圈」模式，加 13 個單元測試，
    `cargo test` 51 綠。仍待做：接遊戲迴圈 `tick`、接 ws 互動把乙太加進背包、前端畫格、
    持久化（接 0-E）。接線時移除 `field.rs`/`crops.rs` 的 `allow(dead_code)`。
  - ✅ 玩法接通（端到端可玩，2026-06-05）：把農地接上整條線——
    `AppState` 加共享 `Field`、遊戲迴圈每 tick `field.tick(dt)` 並把 `field.view()` 隨快照
    廣播；新增 `ClientMsg::Farm{x,y}` 與 `Field::interact`「一鍵照顧」（依該格狀態自動
    翻土/播種/澆水/收成），收成把乙太加進 `Player.ether`。前端 canvas 依每格 `state`/`dry`
    畫出耕地、作物階段與「該澆水」提示，點擊（手機輕觸）送 `farm`，HUD 顯示乙太。
    移除 `crops.rs`/`field.rs` 的 `allow(dead_code)`；加 `Field::interact`/`view` 與
    protocol JSON 契約共 7 個測試，`cargo test` 58 綠、伺服器啟動正常。
    **仍差最後一塊才算完成本項**：農地與乙太目前是記憶體狀態，重啟會歸零；
    需接 0-E 持久化（驗收標準「重啟後農地狀態還在」），故主項暫不打勾。
  - ✅ 權威伺服器補洞（2026-06-05）：先前 `Farm` 訊息只用 `cell_at` 過濾座標，
    沒檢查玩家自己在哪——客戶端可送任意座標隔空遙控、收成這片共享農地。新增
    `field.rs` 純函式 `within_field_reach`（點到農地矩形最近距離 ≤ `FARM_REACH`，
    站在地塊內或緊鄰邊緣才算），ws 的 `Farm` 處理先驗證玩家位置再 `interact`。
    出生點本就在農地內，正常玩家零影響，只擋「人不在農地卻操作」。加 3 個測試，
    `cargo test` 61 綠、伺服器啟動正常。
  - ✅ 前置（農地持久化格式地基，2026-06-05）：驗收標準「重啟後農地狀態還在」缺最後
    一塊——農地狀態目前完全沒有可序列化格式。給 `Crop`/`Tile`/`Field` 衍生 serde：
    `Crop` 存內部 `growth`/`moisture`（秒）而非推導階段，讓「成長到一半」的作物重啟後
    原地接續長、不被四捨五入。新增載入入口 `Field::from_tiles`（驗證格數，舊版/壞檔/
    被竄改長度一律回 `None`，呼叫端可退回全新地）。延續本檔「純邏輯可測、不接 IO、
    接線時移除 `allow(dead_code)`」的前置慣例——這層只是格式地基，實際存讀檔/Postgres
    仍待 0-E（架構級、留待人決策）。加 2 個測試（中段成長 round-trip、from_tiles 拒絕
    錯誤長度），`cargo test` 70 綠、伺服器二進位啟動正常（埠被正式服務占用屬預期）。
  - ✅ 權威伺服器補洞之二（非有限座標，2026-06-05）：純函式 `cell_at` 文件保證
    「界外回 `None`」，但對 `NaN`/`Inf` 會被騙——`NaN < 0.0` 為 false 不早退，
    且 `(NaN / TILE_SIZE) as usize` 在 Rust 飽和轉型成 0，使垃圾座標誤落到 (0,0) 格。
    在 `cell_at` 開頭加 `is_finite` 檢查、一律視為界外。雖然 `within_field_reach`
    已用權威位置擋掉實際濫用，這仍是有測試的純函式違反自身契約，修正讓契約乾淨。
    加 1 個測試，`cargo test` 63 綠、伺服器二進位啟動正常（埠被正式服務占用屬預期）。
  - ✅ 持久化載入防線之二（壞作物值，2026-06-05）：`from_tiles` 契約聲稱「壞檔／被竄改
    一律回 `None`」，但先前只驗格數——一個格數正確、卻含 `NaN`／`Inf`／負成長的作物存檔會
    直接放行，毒化整塊地的成長與顯示（JSON 較難灌入 NaN，但接 0-E 的 Postgres float 欄位可
    存進）。新增 `Crop::is_loadable`（與調校常數無關的最小不變式：成長／濕度皆有限且非負；
    上界刻意不檢查，因常數會調且 `grow` 下一 tick 自會夾回上限），`from_tiles` 載入時逐株驗證，
    任一格不健全即整塊拒收、讓呼叫端退回全新地。延續本檔持久化格式地基的硬化脈絡（接續格數／
    `cell_at` 非有限座標的防線）。加 2 個測試，`cargo test` 72 綠、clippy 乾淨、伺服器二進位
    啟動正常（埠被正式服務占用屬預期）。
  - ✅ 前置（日夜循環純邏輯地基，2026-06-05）：驗收標準「隨日夜成長／收成」與 GDD 第 9 章
    「伺服器驅動的日夜循環」缺最後一塊——日夜時間完全沒有模型。新增 `src/daynight.rs`
    `DayNight`（只存循環內 `elapsed` 秒、階段／亮度都由它推導，單一真實來源）+ 純函式
    `phase_for`（破曉／白天／黃昏／夜晚）與 `light_for`（餘弦平滑亮度，日中最亮、午夜最暗、
    保留 `MIN_LIGHT` 微光不全黑）。`advance` 擋非正／非有限 dt、`at` 載入入口取模繞回並把
    非有限退回破曉——延續 `spawn_at`／`from_tiles`／`is_loadable` 的載入時驗證脈絡，為接 0-E
    跨重啟接續預留。延續本檔「純邏輯可測、不接 IO、接線時移除 `allow(dead_code)`」的前置慣例；
    實際接遊戲迴圈 `advance`、隨快照廣播階段／亮度給前端染色、（選用）作物白天長快，留待後續。
    加 11 個單元測試，`cargo test` 115 綠、clippy 乾淨、伺服器二進位啟動正常。
  - ✅ 日夜循環接通（端到端染色，2026-06-05）：上一輪落地的 `daynight.rs` 純邏輯地基終於
    接上整條線——`AppState` 加共享 `DayNight`，遊戲迴圈每 tick `daynight.advance(dt)` 並把
    `daynight.view()`（階段 + 亮度）隨快照廣播；新增 `protocol::DayNightView`（`phase` 以
    snake_case 字串、`light` 為數值，鎖住前後端契約）。前端依 `light` 在世界上疊一層夜色
    （白天≈1 幾乎不疊、午夜 0.2 最濃但仍 `MIN_LIGHT` 微光不全黑，療癒非恐怖），HUD 顯示
    當前階段（🌅破曉/☀️白天/🌇黃昏/🌙夜晚），給「日夜流轉」的療癒體感。移除 `daynight.rs`
    的 `allow(dead_code)`（持久化載入入口 `at` 改標靶 allow，待 0-E 跨重啟接續才有呼叫端，
    比照 `crops::is_loadable`）。加 protocol 契約與 `view()` 一致性共 2 個測試、補既有快照
    測試的 `daynight` 欄位，`cargo test` 116 綠、clippy 乾淨、伺服器二進位啟動正常（埠被
    正式服務占用屬預期）。bump `index.html` 的 `game.js?v`。**仍待**：作物白天長略快（選用）、
    日夜時刻持久化（接 0-E）。
  - ✅ 照顧距離前端回饋（2026-06-05）：上一輪伺服器加了「離農地太遠就拒絕照顧」的權威
    檢查，但客戶端零回饋——玩家走遠後點農地會靜默沒反應、像壞掉。把 `FARM_REACH` 隨
    `FieldView` 快照帶給前端（新增 `reach` 欄位，伺服器常數為單一來源、前後端不各定一套），
    前端鏡像 `within_field_reach`：離太遠時整塊農地畫淡 + 顯示「走近一點才能照顧農地🌱」，
    點擊太遠時給節流的系統提示而非白送一則。後端加 1 個 view `reach` 斷言（並補既有 2 處
    `FieldView` 建構），`cargo test` 63 綠、伺服器二進位啟動正常。純前端回饋，不動玩法邏輯。

## Phase 1（採集 + 合成 + 載具 MVP）

> Phase 0 穩了再進。也由上往下做。

- [ ] **Phase 1-A：採集節點與動作**
  世界裡撒佈樹/石/乙太礦三種節點;玩家走近按鍵採集,扣節點耐久度+背包加資源;
  節點空了會在固定時間後重生。
  - 驗收:看得到節點、按一下採到、伺服器 log 顯示「採到 X」、可重複採直到節點空、
    重生計時運作;`cargo test` 涵蓋採集純邏輯(扣耐久、回滿、節點上限)。

- [ ] **Phase 1-B：背包系統 + 持久化**
  伺服器端 player.inventory(item_id → count),客戶端按 I 開背包面板顯示。
  接 0-E 持久化。
  - 驗收:採集→開背包看到資源→重連/重啟仍在;`cargo test` 涵蓋背包增減上限。

- [ ] **Phase 1-C:合成台 + 第一份配方**
  玩家可在地盤蓋一個「合成台」實體;互動開菜單,有材料就能做出產物。
  起步配方:木×3 + 石×2 = 鎬子。
  - 驗收:合成出鎬子,顯示在背包;材料不足不給合;`cargo test` 涵蓋配方檢查。

- [ ] **Phase 1-D:工具效用**
  鎬子讓採礦更快;鋤頭讓翻土更快。簡單的「拿什麼工具決定能做什麼/多快」。
  - 驗收:身上有鎬子採礦速度提升 X 倍;沒有就用拳頭(很慢)。

- [ ] **Phase 1-E:蒸汽載具 MVP**
  地圖上有可乘騎的蒸汽腳踏車實體,玩家走近按鍵「上下車」;上車後輸入直接控車,
  速度比走路快 3 倍、會慣性/輕微甩尾。
  - 驗收:玩家上車,移動明顯比走快,下車回原本走法;`cargo test` 涵蓋
    車輛物理整合純函式。

## 玩家回饋處理區(devloop 從 data/suggestions.jsonl 進來的)

> devloop 處理過的玩家建議在此打勾並引用建議 ID/摘要;處理中的也記在這。

(空)

## 選用基礎建設

- [ ] **Phase 0-H 進階(選用)：容器化 + 雲端代管**
  目前已透過 Cloudflare Tunnel 自架上線。之後若想關機也能玩或要切到雲端,
  容器化 + Fly.io 是路。

## 之後（Phase 2+，見 GDD）

自動化與牧場(缺氧層)、職業與社交(RO 層)、卡丁車競速(極速層)、星際擴張(北極星)。
這些在 Phase 0/1 穩定可玩、且使用者點頭前,**不要動工**。devloop 推進到接近時
再把該 Phase 的具體 tickets 攤下來。
