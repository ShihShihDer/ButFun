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
  ✅ 資料曝露收口（移除公開未驗身的 GET 清單端點，2026-06-05）：`GET /api/suggestions`
  是未驗身公開端點、會把全部玩家建議（含玩家自填署名）整包吐給任何 `curl`，而前端從不
  消費它（`web/game.js` 只 POST）——線上一個沒人用卻能撈走所有玩家回饋的資料曝露點。
  建議的整條硬化弧線（長度上限／控制字元／載入路徑 sanitizer）一直在加固「回出的內容」，
  卻沒堵住「不該對外回出」本身；此輪把它收口：移除 `list_suggestions` handler、路由只留
  `POST`。維護者本就直接讀 `data/suggestions.jsonl` 三角化，零影響；刻意保留
  `SuggestionStore::list`（標 `allow(dead_code)`）當建材，日後做後台檢視走驗身路由再接上。
  **不刪除／不改寫磁碟資料**（不破壞玩家資料），只移除讀路徑。`cargo test` 122 綠、
  clippy 乾淨、伺服器二進位啟動正常（埠被正式服務占用屬預期）。
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

- [ ] **Phase 0-E：串接 PostgreSQL 持久化（backend lane 最高優先,使用者明確要求）**
  > 現況:位置+乙太已用 `PositionStore` 寫穿到 `data/positions.jsonl`(commit 8a18a84),
  > 重啟不掉了。**這份 JSONL 版正好是 Postgres 版的模板與 fallback。** DB 已就緒
  > (PostgreSQL 17、`butfun` 庫、`DATABASE_URL` 在 `.env`/EnvironmentFile)。
  做法(沿用既有抽象、最小擾動):
  - `cargo add sqlx`(features: runtime-tokio, postgres, uuid),加 `migrations/`(用 `sqlx::migrate!`)。
  - 建表 `player_state(id uuid pk, x real, y real, ether int, updated_at timestamptz)`。
  - 擴 `PositionStore`:多一個 `Option<PgPool>` 後端。**保持 recall/remember 同步介面不變**
    (避免動 ws.rs/game.rs):做法是「記憶體 cache + 背景非同步刷 PG」——
    - 啟動時若有 pool → `SELECT` 全表載入 cache(沿用 `spawn_at` 驗座標);無 pool → 維持 JSONL/記憶體。
    - 遊戲迴圈那個每 10s 的 flush:有 pool → `INSERT … ON CONFLICT DO UPDATE` upsert cache;無 pool → 維持寫 JSONL。
  - `main.rs` 啟動時依 `DATABASE_URL` 建 `PgPool`(失敗就 log + 退回 JSONL,別讓伺服器起不來)。
  - 驗收:設好 `DATABASE_URL` 跑、移動+收乙太→重啟伺服器→重連位置與乙太仍在;
    `psql -d butfun -c 'select * from player_state'` 看得到;無 `DATABASE_URL` 時仍走 JSONL、`cargo test` 全綠。
  - 之後同法把 users/suggestions/field/daynight 也接 PG(一次一個,別一個巨大 PR)。
  - ♻️ 撤回外洩進 main 的 0-E 地基(2026-06-05):一個純前端「田地可見」熱修 commit(7460edd)
    意外把 0-E 的 sqlx 依賴(+Cargo.lock 741 行)、`migrations/0001_players_positions.sql`、
    `positions.rs` 一行 unused `use sqlx::Row;`、以及一段謊稱「設了 `DATABASE_URL` 走 Postgres
    (跨重啟仍在)」的文件註解一起帶進 main——但 main 上**完全沒有**任何 Postgres 程式碼
    (`grep sqlx src/` 只剩那行 unused import,無 `PgPool`/`migrate`/`DATABASE_URL`),只剩一個
    build warning + 一段對不上實作的謊稱註解 + 一個沒人跑的孤兒 migration + 一個未使用的重依賴
    拖慢編譯。這是「devloop 共用工作樹、未提交編輯外洩進別人 commit」風險的實例。本輪把這批外洩
    內容撤回、讓 main 回到誠實的「記憶體前置」狀態(positions.rs 文件改回「Phase 0-E 的記憶體前置」),
    sqlx/migration/Postgres 留給正式的 PR #11(`feat/0e-pg-foundation`,3d9f3a3「sqlx + players
    migration」)引入。**不碰玩家資料**(migration 從未在任何地方執行過,刪檔不 drop 任何資料表)。
    `cargo build` 無 warning、`cargo test` 122 綠、clippy 乾淨、伺服器二進位啟動正常(埠被正式服務占用屬預期)。
  - ✅ **DB 已就緒(2026-06-05,使用者授權後人工裝)**:PostgreSQL 17、`butfun` 資料庫已建、
    `shihshih` 為 superuser、Unix socket peer auth(免密碼);
    `DATABASE_URL=postgresql://shihshih@/butfun?host=/var/run/postgresql` 已寫入 `.env`
    (gitignored),`butfun.service` 透過 `EnvironmentFile` 載入。**可直接 `cargo add sqlx`
    開始接,不需停下來問人**。建議步驟:加 sqlx 依賴 → 寫 migrations → 把現有
    `PositionStore`(以及 `UserStore`/`SuggestionStore`/`Field`/`DayNight` 那些既有序列化
    結構)依序接到 Postgres;無 `DATABASE_URL` 仍退回記憶體模式以利測試。一次一個 store
    incremental 接,避免單一巨大 PR。
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
  - ✅ 前置（玩家狀態持久化格式地基，2026-06-05）：`Field`／`Crop`／`DayNight` 都已在
    接 0-E 前先衍生 serde 當「格式地基」，但 `positions::Saved`（玩家離線時記下的位置 + 乙太、
    正是 0-E 要跨重啟存回的玩家狀態本體）是唯一一個「存檔又重載」結構還沒有序列化格式——
    只 derive 了 `Debug/Clone/Copy/PartialEq`。接 0-E（沿用本 repo 既有的 jsonl 持久化路數，
    如 `users.jsonl`／`suggestions.jsonl`）一定得逐筆序列化 `Saved`。給它補上 `Serialize`/
    `Deserialize`，補齊「每個存檔又重載的結構都可序列化」這組地基的最後一塊。載入防線沿用既有
    入口不重複：位置一律經 `spawn_at` 驗證（非有限退回地圖中央、界外夾回邊界，比照 `Field` 的
    `from_tiles` 當載入閘門），`ether` 是 `u32`、型別本身就擋掉 `NaN`／`Inf`／負值。加 2 個測試
    （round-trip、壞座標仍經 `spawn_at` 守門），`cargo test` 122 綠、clippy 乾淨、伺服器二進位
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
    正式服務占用屬預期）。bump `index.html` 的 `game.js?v`。**仍待**：日夜時刻持久化（接 0-E）。
  - ✅ 照顧距離前端回饋（2026-06-05）：上一輪伺服器加了「離農地太遠就拒絕照顧」的權威
    檢查，但客戶端零回饋——玩家走遠後點農地會靜默沒反應、像壞掉。把 `FARM_REACH` 隨
    `FieldView` 快照帶給前端（新增 `reach` 欄位，伺服器常數為單一來源、前後端不各定一套），
    前端鏡像 `within_field_reach`：離太遠時整塊農地畫淡 + 顯示「走近一點才能照顧農地🌱」，
    點擊太遠時給節流的系統提示而非白送一則。後端加 1 個 view `reach` 斷言（並補既有 2 處
    `FieldView` 建構），`cargo test` 63 綠、伺服器二進位啟動正常。純前端回饋，不動玩法邏輯。
  - ✅ 隨日夜成長收尾（作物白天長略快、夜裡放慢，2026-06-05）：0-G 驗收標準「隨日夜成長」與
    GDD 第 9 章日夜循環的最後一塊——日夜時間已接通並染色，但作物成長仍與時段無關。新增
    `daynight.rs` 純函式 `growth_rate_for(light)`：把當下亮度（保證落在 `[MIN_LIGHT,1]`）線性映成
    成長倍率 `[MIN_GROWTH_RATE 0.6, MAX_GROWTH_RATE 1.25]`，最暗的午夜最慢、最亮的日中最快，
    溫和的「略快／略慢」差距、夜裡不歸零（仍緩慢生長、不卡死療癒節奏）；輸入先 `clamp` 防壞值
    算出界外倍率。遊戲迴圈改先 `advance` 日夜時鐘、取 `growth_rate()`，再用它縮放餵給
    `field.tick` 的 `dt`。刻意走「縮放 dt」這個最小接線：成長與濕度一併縮放，故每次澆水的
    **總成長量不變**、只有牆鐘速度隨日夜變化——夜裡作物休眠、長得慢也喝得慢，公平又貼合療癒主題；
    `crops.rs`/`field.rs` 既有測試直接呼叫 `tick(dt)` 完全不受影響。加 3 個測試（端點對齊倍率界、
    壞亮度夾回界內、端到端日中最快午夜最慢），`cargo test` 119 綠、clippy 乾淨、伺服器二進位啟動
    正常（埠被正式服務占用屬預期）。**仍待**：日夜時刻持久化（接 0-E）。
  - ✅ 持久化載入防線（日夜 serde 路徑也過驗證，2026-06-05）：`DayNight` 同時有兩條載入路徑——
    手動入口 `at()`（非有限退回破曉、界外/負值取模繞回）與 `#[derive(Deserialize)]`（**完全不驗證**）。
    `serialized_day_night_round_trips` 測試證明 0-E 會用 serde 序列化日夜時刻，但衍生的 Deserialize
    會把磁碟上被竄改／損毀的 `elapsed`（負值、界外、接 Postgres float 後的非有限）原樣讀進來，
    違反 `elapsed` 文件白紙黑字的不變式「恆落在 `[0, DAY_LENGTH_SECS)`」、毒化 `fraction()`／階段／亮度——
    正是 `from_tiles`／`is_loadable`／`spawn_at` 一路在補的同一類「存檔又重載卻在載入路徑沒驗證」缺口。
    改用手動 `impl Deserialize`：先以無驗證的鏡像結構吃下原始 `elapsed`，再一律過 `at()` 同一道守門，
    讓兩條載入路徑共用單一真實驗證。`at()` 從此被 Deserialize 呼叫、不再是 dead code（移除標靶 allow）。
    加 1 個測試（界外／負值反序列化繞回、正常值原樣保留），`cargo test` 120 綠、clippy 乾淨、伺服器
    二進位啟動正常（埠被正式服務占用屬預期）。**仍待**：日夜時刻持久化（接 0-E）。

## 設計方向轉變(2026-06-05,使用者決定):農地改為「各自擁有」+ 地圖擴張

> **動機**:使用者問「多人農地怎麼分配?搶?購買?各自獨立耕種?」。決策:**每個玩家
> 擁有自己的一塊地(空間分開,非共享同一塊)**——貼合 GDD「繼承自己的乙太工坊農莊」,
> 不會互相踩。目前單一共享 `Field` 是 Phase 0 暫時做法,要改成 per-player。

- [ ] **Phase 0-G-O1:農地改 per-player 擁有**
  `Field` 從「全域單一塊」改成「每個 user_id 一塊」(`HashMap<Uuid, Field>` 或地圖上
  各自的地塊)。玩家進場分配/認領一塊空地(家園區往外排)。只能照顧自己的地;路過別人
  的地看得到作物、不能動(伺服器 Farm 動作驗證「這塊地屬於你」)。訪客可給暫時地或唯讀。
  - 驗收:兩個帳號各自種各自的、互不影響;路過看得到對方作物但點不動;`cargo test` 涵蓋
    「非地主的 Farm 動作被拒」。接 0-E 後每塊地持久化。
  - ✅ 前置(地塊配置幾何純邏輯地基,2026-06-05):per-player 第一個缺口是「第 N 個玩家的地
    擺世界哪裡」——現有 `field.rs` 只有一塊全域農地、origin 是寫死常數。新增 `src/plots.rs`
    純幾何層:`plot_origin(index)` 用標準方塊螺旋讓家園區從中心一圈一圈往外排,**序號 0 正好
    對齊現有全域農地位置**(接線時第一個玩家無縫接續、不平白位移既有視野);序號各異 ⇒ 整數格
    座標各異 ⇒ 任兩塊地至少差一個 stride(≥footprint)⇒ **保證不重疊**,不必另存已用座標、無
    浮點累積誤差。`PLOT_*` 尺寸常數沿用 `field.rs` 同一套(單一真實來源)。延續本檔
    `daynight`/`crops`/`field` 的前置慣例:純函式、無 IO、不碰 ws/遊戲迴圈/`Field`,標
    `allow(dead_code)`,接線輪(進場 `assign` 地塊、`Field` 帶 origin、Farm 驗地主)才有呼叫端。
    刻意不在這層擋世界邊界(往外排遲早超出 2000×2000——把世界長大是 O2 的事)。加 6 個單元測試
    (螺旋座標鎖定、序號 0 對齊、確定性、互異、**任兩塊不重疊**、相鄰留縫),`cargo test` 128 綠、
    clippy 乾淨、伺服器二進位啟動正常(埠被正式服務占用屬預期)。**仍待**:接線(HashMap+地主驗證+
    前端畫多塊)屬架構級、會動 live 廣播 shape,留待後續輪/PR。
  - ✅ 前置(地塊歸屬登記純邏輯地基,2026-06-05,回應建議 at=1780631397524「新玩家沒有自己的地」):
    `plots.rs` 解了「第 N 塊地在世界哪裡」的幾何;per-player 還缺另一半——「**哪個玩家擁有第幾塊地**」。
    新增 `src/plot_registry.rs` `PlotRegistry`(`assign` 進場分配下一個未用序號、重連拿回同一塊;
    `index_of`/`owns` 歸屬判斷,正是接線時 ws `Farm` 要驗的「這塊地屬於你」)。序號單調遞增、只增不減
    (第一個玩家拿 0 對齊現有全域農地,離開不回收避免作物歸屬錯亂,貼合 O2「序號往外排」)。延續
    `connections.rs`/`positions.rs` 的可測小 store 模式:無 IO、純粹方法、標 `allow(dead_code)` 待接線輪
    才有呼叫端。跨重啟持久化(把這張 user_id→序號 表也存進 Postgres)留待 0-E。加 6 個單元測試
    (序號從 0 遞增、同玩家重連同一塊、不同玩家互異、只增不減、`index_of` 反映分配、`owns` 只認自己的地),
    `cargo test` 137 綠、clippy 乾淨、伺服器二進位啟動正常(埠被正式服務占用屬預期)。**仍待**:接線
    (進場 `assign`+`Field` 帶 origin+Farm 驗地主+前端畫多塊)屬架構級、動 live 廣播 shape,留待後續輪/PR。
  - ✅ 前置(`Field` 帶自己的 origin——連接幾何與登記兩塊地基,2026-06-05):前兩塊地基算出
    「第 N 塊地在世界哪裡」(`plots.rs`)與「哪個玩家擁有第幾塊」(`plot_registry.rs`),但 `Field`
    本身仍寫死用全域常數 `FIELD_ORIGIN`,無法擺到別塊地。這輪補上中間最後一塊純邏輯:`Field` 改帶
    自己的 `origin_x/origin_y`,`cell_at`/`within_reach`/`view` 全改吃 `self.origin`(不再吃全域常數);
    新增 `Field::for_plot(index)`(origin 由 `plots::plot_origin(index)` 決定,序號 0 對齊現有全域農地)
    與 `origin()` 取值。**origin 刻意 `#[serde(skip)]` 不入存檔**——接 0-E 載入時由該玩家的序號重建供入,
    `from_tiles` 改收 `(index, tiles)`、origin 一律由序號說了算(per-player 的關鍵:同一份 tiles 擺哪塊
    地由序號決定,不靠磁碟值)。**行為不變**:單一全域 `Field::new()` 仍落在 `FIELD_ORIGIN`(序號 0),
    快照 shape、前端、live 廣播全不動,故可直接進 main。`ws.rs` 的 `Farm` 改用實例方法、每把鎖各自取
    各自放(同時至多持一把,沿用不互鎖的鎖序)。加 4 個測試(`for_plot(0)`==`new()`、`for_plot` origin
    對齊 plots 幾何、`cell_at`/`within_reach` 以該塊 origin 為基準、`from_tiles` origin 來自序號),改
    既有 serde round-trip 測試走 `from_tiles(0,..)` 還原。`cargo test` 141 綠、clippy 乾淨、伺服器二進位
    啟動正常(埠被正式服務占用屬預期)。**仍待**:接線(AppState 改 `HashMap<Uuid,Field>`+進場分配+Farm
    驗地主+快照送多塊+前端畫多塊)屬架構級、動 live 廣播 shape,留待後續輪/PR。
  - 🔵 接線(per-player 端到端接通,**draft PR、待人審不自走 merge**,2026-06-05,回應建議
    at=1780631397524「領地還沒鎖使用者、新玩家沒有自己的地」):把三塊純邏輯地基接上整條線——
    `AppState` 由單一 `field` 改成 `fields: HashMap<Uuid, Field>` + `plots: PlotRegistry`;已登入玩家
    進場 `plots.assign` 取序號、`Field::for_plot` 建自己那塊地(同帳號重連拿回同一塊、作物續長);
    遊戲迴圈 tick 所有地塊、快照改送 `fields: Vec<FieldView>`(每塊戳上 `owner`);ws `Farm` 只對
    `fields.get(&id)`(自己那塊)算格——**歸屬由建構性保證**:路過別人的地送來的座標落在別塊、
    `cell_at` 回 `None`,動不到別人的地,不必另存座標→地主表。前端畫出所有玩家的地、只有 owner===myId
    那塊套照顧距離回饋與互動,別塊標地主名、點不動;小地圖多塊、自己亮別人暗。**架構級、動 live 廣播
    shape(`field`→`fields`)**,故依 AUTONOMOUS_OPS 護欄只開 draft PR、不自走 merge。加 2 個測試
    (protocol `fields[].owner` 契約、`cell_at` 對別塊座標回 None 的歸屬保證),`cargo test` 142 綠、
    clippy 乾淨、伺服器二進位啟動正常。**待人決定**:訪客是否給臨時地(本 PR 取保守的「訪客唯讀、
    登入才有地」避免序號無界成長);跨重啟持久化(把 `plots` 表也存進 Postgres)接 0-E。
  - ✅ 前置(地塊登記的載入路徑驗證,2026-06-05):`PlotRegistry`(0-E 會把 user_id→序號表存進
    Postgres)是 per-player 諸 store 裡唯一還沒有**載入入口**的——其餘存檔又重載結構都已先補上
    載入時的不變式驗證(`positions::spawn_at`/`field::from_tiles`/`daynight::at`+`Deserialize`/
    `suggestions`/`users::parse_and_sanitize`)。新增 `PlotRegistry::from_saved((user_id,序號) 對)`
    重建登記表,**關鍵不變式:`next` 一律重建成「已用最大序號 + 1」**。若天真載入把 `next` 設回 0,
    重啟後 `assign` 會把序號 0(或任何已發出的序號)再發給新玩家,造成「同一塊地兩個地主、作物歸屬
    錯亂」——正是本模組「序號只增不減、不回收」白紙黑字要防的災難。重複 user_id 取後見者、空輸入＝全新
    登記表(第一個玩家仍拿序號 0 對齊現有全域農地)。純邏輯、無 IO、標 `allow(dead_code)` 待 0-E 從
    Postgres 載回才有呼叫端,**行為不變**(不動 PR #12 接線範圍的 field/game/protocol/state/ws),故直接
    進 main。加 3 個測試(空輸入如新、載回保留地主、稀疏跳號續發＝最大+1 不撞既有地主),`cargo test`
    144 綠、clippy 乾淨、伺服器二進位啟動正常(埠被正式服務占用屬預期)。**仍待**:0-E 真正把這張表
    存讀 Postgres(架構級,留待 0-E)。

- [ ] **Phase 0-G-O2:地圖擴張 + 用乙太購買土地**
  家園區可隨玩家數往外長;玩家用收成的乙太**購買**擴充地塊(乙太的消耗去處,接上經濟)。
  地圖擴張也是多星球願景的前奏(新區域/新星球 = 新主題 tileset,素材已備 b/c 兩套色票)。
  - ✅ 前置(乙太消耗純邏輯地基,2026-06-05,對齊 PLAN.md 當前主攻「把乙太的去處做出來」):
    目前玩家收成只讓乙太數字一直變大、沒地方花——沒目標、沒「再玩一下」的理由,是「沒啥好玩」
    的頭號病根。蓋第一個**乙太消耗點**的權威純邏輯:新增 `src/economy.rs`,玩家用乙太「擴大自己
    那塊農地一格」、乙太隨之變少。核心是全有全無扣款 `spend(balance,cost)`(用 `checked_sub`,
    乙太永遠不為負,比照 `crafting::craft` 的「不足不給合」),所有消耗點共用此單一真實來源;
    `expansion_cost(owned)` 逐格線性漲價(第 N 格 = N× 基準),讓乙太成為**長期**消耗去處
    (愈擴愈貴、得持續收成才買得起);`PlotWallet`(已購格數)帶 serde 與 `is_loadable`(不超
    `MAX_EXPANSIONS` 上限)當持久化格式 + 載入防線,接 0-E 跨重啟存回(驗收「擴張跨重啟還在」)。
    延續本專案「純邏輯可測、無 IO、不碰 ws/遊戲迴圈、標 `allow(dead_code)` 待接線」的前置慣例。
    加 10 個單元測試(扣款/不夠不為負/逐格漲價/全有全無/連買花光乙太/買到上限即止/載入防線收壞值/
    serde round-trip),`cargo test` 252 綠、`cargo build`/clippy 無警告、伺服器二進位啟動正常
    (埠被正式服務占用屬預期)。**仍待**:接線(ws 購買意圖扣乙太、`Field` 多開格、前端購買鈕、
    `PlotWallet` 接 0-E 持久化)屬架構級、動 live 廣播 shape——依 PLAN.md 分工屬 backend lane 的
    伺服器邏輯,本 lane 先把玩法規則做成可測純函式交棒。

## 設計方向轉變(2026-06-05,使用者決定):全自由角色創造

> **動機**:使用者目標是「**後 AI 時代,大家 VR/AR 直接進入遊戲,自由決定樣子去遊玩**」。
> 故事人種(地球人/施拉雅/布司/燐光)會變成**可選預設**,不再是必選;預設走「全自由」。
> 屬性點 / 技能也是自由分配。世界觀(蒸汽龐克太空歌劇)不動,但「你是誰」完全交給玩家。

具體實作方向(分階段做):

- [ ] **Phase 1.5-A:外觀自由創造器(身體分件 + 配色)**
  把目前 `User.species` 從單一字串擴成一個 `appearance` 結構:
  `{ body_color, accent_color, hair_color, hairstyle_id, accessory_ids, body_type }`。
  全程式組裝、不靠大量 sprite(配合素材包的 modular 角色 sprite,各部件可換色)。
  UI:創角畫面用色票+下拉組裝;**4 個原創種族變成「想速成就一鍵套用」的預設模板**,
  不是必選。驗收:創角自由組裝,進場看得到自己的外觀,跨裝置/重連保留。

- [ ] **Phase 1.5-B:屬性點自由分配**
  總點數固定(如 10 點),分配到 4 個跟世界系統咬合的屬性:
  乙太親和(精煉效率)/ 機械精通(合成成功率)/ 耐性(體力/移動續航)/ 社交(玩家
  互動加成)。玩過後可在某些里程碑重新洗點。

- [ ] **Phase 1.5-C:起手技能自由挑(1-2 個)**
  技能池:採集+/合成+/駕駛+/戰鬥+/社交+(對應 Phase 1~4 各支柱),
  選什麼決定起手走哪一條順,但**不鎖死**(之後都能學)。

- [ ] **Phase 1.5-D:起源情境(可選,跟外觀/種族脫鉤)**
  4 個 origin scenarios 變成「我的起點故事」選項:繼承農莊(療癒)/
  商船學徒(貿易)/ 礦坑出走(採集)/ 神祕受召(乙太)。**任何外觀/種族都可選任何起源**。

- [ ] **Phase 1.5-E:為 VR/AR 未來預留接口**
  資料 schema 設計成「自由欄位 + 結構化屬性」混合,角色資料可序列化匯出,
  之後若真做 VR/AR 客戶端,可接「玩家自己上傳模型/材質」的 hook(目前先不做)。

## Phase 1（採集 + 合成 + 載具 MVP）

> Phase 0 穩了再進。也由上往下做。

- [ ] **Phase 1-A：採集節點與動作**
  世界裡撒佈樹/石/乙太礦三種節點;玩家走近按鍵採集,扣節點耐久度+背包加資源;
  節點空了會在固定時間後重生。
  - 驗收:看得到節點、按一下採到、伺服器 log 顯示「採到 X」、可重複採直到節點空、
    重生計時運作;`cargo test` 涵蓋採集純邏輯(扣耐久、回滿、節點上限)。
  - ✅ 前置(採集節點純邏輯地基,2026-06-05):Phase 1 第一個垂直切片開工——新增
    `src/gather.rs` `ResourceNode`(`NodeKind` 樹/石/乙太礦,各帶 `max_durability`/
    `yield_per_gather`/`respawn_secs` 調校常數)。狀態只有「剩餘耐久」+「重生倒數」兩欄,
    可採/採空皆由耐久推導(單一真實來源,比照 `Crop` 以 `growth`/`moisture` 推導階段)。
    `gather()` 還有耐久就扣 1 並回產出、扣到 0 啟動重生倒數、採空回 `None`(比照
    `Crop::harvest`);`tick(dt)` 只對採空節點倒數、到點補滿耐久再次可採(擋非正 dt)。
    延續本專案「純邏輯可測、無 IO、不碰 ws/遊戲迴圈、標 `allow(dead_code)` 待接線」的前置慣例,
    並沿用載入防線:`is_loadable`(耐久不超上限、重生倒數有限非負,`remaining` 為 `u32` 型別本身
    擋掉 NaN/負值)供接 0-E 載入時驗證,`NodeKind`/`ResourceNode` 衍生 serde 為持久化格式地基。
    加 10 個單元測試(滿耐久可採、採空進重生、採空再採無效、倒數重生、`tick` 對可採節點 no-op、
    非正 dt no-op、整圈採→空→重生→再採、載入防線收壞值、serde round-trip),`cargo test` 154 綠、
    `cargo build`/clippy 無警告、伺服器二進位啟動正常(埠被正式服務占用屬預期)。**仍待**:接線
    (世界撒佈節點+ws 走近按鍵採集進背包+遊戲迴圈每 tick 推進重生+前端畫節點/採集回饋)屬動 live
    廣播 shape 的架構級接線,留待後續輪/PR;背包持久化接 Phase 1-B / 0-E。
  - ✅ 前置之二(節點的世界佈置與採集互動純邏輯,2026-06-05):`gather.rs` 解了「單一節點怎麼被採」,
    接線還缺另一半「節點擺世界哪裡、玩家走近採到哪一個」——比照 `plots.rs` 之於 `field.rs`。新增
    `src/gather_field.rs` `NodeField`(一組散佈的 `PlacedNode`=座標+`ResourceNode`):`new()` 用確定性
    雜湊(splitmix64 風格、不靠亂數/時鐘)把節點散在世界中央家園淨空圈外的一圈曠野,座標由序號推導故
    重啟後落在同一處;`tick(dt)` 一次推進全部節點重生;`gather_near(x,y)` 在 `GATHER_REACH` 內挑**最近**
    且仍可採的節點採一下、回 `(種類,產出)`(範圍內無可採回 `None`,權威由伺服器判定、客戶端只送意圖)。
    佈置刻意「環繞家園的曠野」:中央留空給 `plots.rs` 往外排的地塊與出生點,出門採集 vs 居家種田兩種
    節奏。沿用載入防線:`gather_near` 擋非有限座標(比照 `cell_at`)、`from_saved` 比照 `field::from_tiles`
    驗節點數/種類對齊序號/逐個 `is_loadable`,壞檔整組拒收讓呼叫端退回全新一組;接 0-E 時佈置座標由序號
    重建、只存讀會變的耐久/重生狀態。延續「純函式、無 IO、不碰 ws/遊戲迴圈/廣播 shape、標 `allow(dead_code)`
    待接線」的前置慣例。加 11 個單元測試(滿員可採、佈置確定性、避開中央淨空且在世界內、三種齊全、
    最近節點採集扣耐久、界外/非有限座標回 None、採空被跳過再 tick 重生、from_saved round-trip、拒錯誤
    節點數、拒種類不符/壞值),`cargo test` 165 綠、`cargo build`/clippy 無警告、伺服器二進位啟動正常
    (埠被正式服務占用屬預期)。**仍待**:接線(AppState 持有 `NodeField`+遊戲迴圈 tick+ws 採集進背包
    +快照廣播+前端畫節點/採集回饋)屬動 live 廣播 shape 的架構級接線,留待後續輪/PR。

- [ ] **Phase 1-B：背包系統 + 持久化**
  伺服器端 player.inventory(item_id → count),客戶端按 I 開背包面板顯示。
  接 0-E 持久化。
  - 驗收:採集→開背包看到資源→重連/重啟仍在;`cargo test` 涵蓋背包增減上限。
  - ✅ 前置(背包容器純邏輯地基,2026-06-05):採集(1-A)的產出要有地方放——新增
    `src/inventory.rs` `Inventory`(`ItemKind`→數量,內部 `BTreeMap` 故序列化/顯示順序
    確定)。`add(item,qty)` 夾 `MAX_STACK` 上限並回實際加入量(背包滿了採不進的手感日後接得上);
    `take(item,qty)` **夠才扣、不夠回 false 完全不動**(合成「材料不足不給合」要的全有全無語意,
    1-C 會用);`has`/`count`/`entries`(供前端面板)。不變式「只存數量 > 0 條目」——歸零即移除,
    「有沒有某物」永遠等同「key 在不在」、序列化不留 0 垃圾。把資源抽成 `ItemKind` enum
    (非散落字串 id):採集 `NodeKind` 直接 `From`/`.into()` 對應物品(`Tree`→`Wood`/`Rock`→
    `Stone`/`EtherOre`→`Ether`),型別擋掉拼錯 id,日後工具/合成產物只加變體、容器不動。
    沿用載入防線:`is_loadable`(無 0 條目、不超上限,`u32` 型別本身擋 NaN/負值)供接 0-E 載入時驗證,
    衍生 serde 為持久化格式地基。延續本專案「純邏輯可測、無 IO、不碰 ws/遊戲迴圈、標
    `allow(dead_code)` 待接線」的前置慣例。加 14 個單元測試(累加/夾上限/回實際量、扣料全有全無、
    歸零移除、`has`/`count`、`NodeKind`→`ItemKind` 映射、採集產出灌進背包、`entries` 排序非零、
    載入防線收壞值、serde round-trip),`cargo test` 179 綠、`cargo build`/clippy 無警告、伺服器
    二進位啟動正常(埠被正式服務占用屬預期)。**仍待**:接線(ws 採集→`add` 進背包+快照廣播該玩家
    背包+前端按 I 開面板)屬動 live 廣播 shape 的架構級接線,留待後續輪/PR;持久化接 0-E。

- [ ] **Phase 1-C:合成台 + 第一份配方**
  玩家可在地盤蓋一個「合成台」實體;互動開菜單,有材料就能做出產物。
  起步配方:木×3 + 石×2 = 鎬子。
  - 驗收:合成出鎬子,顯示在背包;材料不足不給合;`cargo test` 涵蓋配方檢查。
  - ✅ 前置(合成配方純邏輯地基,2026-06-05):背包(1-B)有了容器、採集(1-A)有了產出,
    合成是把資源變成工具的下一環——新增 `src/crafting.rs` `Recipe`(材料清單 `inputs` +
    產物 `output`/`output_qty`),第一份配方 `PICKAXE` 寫成編譯期 `&'static` 常數(木×3+石×2
    =鎬子×1),`RECIPES` 表供接線時前端列菜單。`craft(&mut inventory)` 走全有全無:
    `can_craft` 一次性確認**材料齊備且背包放得下產物**才動手,確認後逐項 `take` 必成功——
    不會「扣了一半材料才發現不夠」留半成品(正是驗收「材料不足不給合」要的語意)。產物爆倉
    也擋(`has_output_room`):材料不會被吃掉、鎬子卻因滿倉憑空消失。在 `ItemKind` 加 `Pickaxe`
    變體(inventory 註解早預告「合成產物只要加變體、容器不動」;非採集產物故無對應 `NodeKind`)。
    所有材料/產物走 `ItemKind` 型別、拼錯的 id 編譯期就擋掉。延續本專案「純邏輯可測、無 IO、
    不碰 ws/遊戲迴圈、標 `allow(dead_code)` 待接線」的前置慣例。加 8 個單元測試(配方符合驗收
    規格、扣料給產物、保留多餘材料、缺一材料全有全無不動、空背包失敗、材料夠可連合、產物爆倉
    擋下不吃料、RECIPES 表含鎬子),`cargo test` 187 綠、`cargo build`/clippy 無警告、伺服器二進位
    啟動正常(埠被正式服務占用屬預期)。**仍待**:接線(地盤蓋合成台實體+ws 互動開菜單選配方
    +前端列 `RECIPES`/反灰不可合/顯示缺料)屬動 live 廣播 shape 的架構級接線,留待後續輪/PR;
    背包持久化接 0-E。

- [ ] **Phase 1-D:工具效用**
  鎬子讓採礦更快;鋤頭讓翻土更快。簡單的「拿什麼工具決定能做什麼/多快」。
  - 驗收:身上有鎬子採礦速度提升 X 倍;沒有就用拳頭(很慢)。
  - ✅ 前置(工具效用純邏輯地基,2026-06-05):接續合成(1-C)出工具後的下一環——新增
    `src/tools.rs`,把「拿什麼工具、做什麼動作、快多少」抽成純函式。`ToolTask`(採集/翻土)、
    `ToolKind`(徒手/鎬子/鋤頭)、`ToolKind::multiplier(task)`(對的工具配對的活回
    `TOOL_MULTIPLIER`=3、其餘回基礎 `FIST_MULTIPLIER`=1)、`tool_from_item`(背包物品→工具,
    窮舉 match 故日後加工具編譯期強制補上)、`best_tool_for(inv,task)`(自動取背包裡最有效的
    工具,拿錯/沒帶就退回徒手)、`speed_multiplier`(端到端倍率)。倍率走整數 `u32`:接線時把
    「一次動作」放大成 m 下即可,不引浮點誤差、與 `gather` 整數耐久計次咬合。規則刻意「對的
    工具配對的活才有加成」(鎬子採礦快、鋤頭翻土快,拿錯只有基礎速度),鼓勵帶對工具。
    為讓 1-D 兩件工具都齊備且可取得,順手在 `inventory.rs` 加 `ItemKind::Hoe` 變體、`crafting.rs`
    加第二份配方 `HOE`(木×2+石×1=鋤頭)並入 `RECIPES`(也讓配方表不再單筆、示範加配方只需多列一筆)。
    延續本專案「純邏輯可測、無 IO、不碰 ws/遊戲迴圈、標 `allow(dead_code)` 待接線」的前置慣例。
    加 7 個 tools 測試(鎬子只快採礦/鋤頭只快翻土/徒手皆基礎/物品→工具映射/空背包退徒手/拿錯退徒手/
    兩把工具各取對的那把)+ 2 個 crafting 測試(HOE 配方符合規格且可合、RECIPES 含兩件工具),
    `cargo test` 196 綠、`cargo build`/clippy 無警告、伺服器二進位啟動正常(埠被正式服務占用屬預期)。
    **仍待**:接線(ws 採集/翻土依 `speed_multiplier` 加速+前端 HUD 顯示手上工具與效用)屬動 live
    廣播 shape 的架構級接線,留待後續輪/PR;背包/工具持久化接 0-E。
  - ✅ 前置之二(採集套用工具倍率的橋接純邏輯,2026-06-05):`tools.rs` 算出倍率 `m`、`gather.rs`
    一次採一下扣 1 耐久,但「一次採集動作相當於採 `m` 下」這段把兩者接起來的純邏輯本身還沒人寫,
    而**任何接線方案都得用到它**(否則倍率只是個算出來沒處套的數字)。在 `ResourceNode` 加
    `gather_times(times)`:連採 `times` 下、累加總產出,關鍵語意「不能超採」——節點只剩 2 耐久卻拿
    3 倍鎬子來採,只採得 2 下的量、採空那下照常啟動重生倒數,絕不憑空多挖(中途採空就停手)。
    刻意放在 `gather.rs`、只吃 `u32` 倍率而不依賴 `tools.rs`,維持兩模組解耦(接線層才把
    `node.gather_times(speed_multiplier(&inv, Gather))` 串起來);整數倍率與整數耐久咬合、全程不引入浮點。
    延續本專案「純邏輯可測、無 IO、不碰 ws/遊戲迴圈、標 `allow(dead_code)` 待接線」的前置慣例。加 4 個
    單元測試(滿耐久連採累加且仍可採、薄耐久不超採且採空進重生、`times==0` no-op、`times==1` 等同採一下),
    `cargo test` 232 綠、`cargo build`/clippy 無警告、伺服器二進位啟動正常(埠被正式服務占用屬預期)。

- [ ] **Phase 1-E:蒸汽載具 MVP**
  地圖上有可乘騎的蒸汽腳踏車實體,玩家走近按鍵「上下車」;上車後輸入直接控車,
  速度比走路快 3 倍、會慣性/輕微甩尾。
  - 驗收:玩家上車,移動明顯比走快,下車回原本走法;`cargo test` 涵蓋
    車輛物理整合純函式。
  - ✅ 前置(載具物理整合純邏輯地基,2026-06-05):Phase 1 最後一個垂直切片開工——新增
    `src/vehicle.rs` `Vehicle`(位置 + 速度向量),把驗收要的「車輛物理整合純函式」抽成
    `step(input, dt)`。模型:阻力先衰減既有速度(放開油門靠慣性滑行)→ 油門沿輸入方向
    加速 → 速率封頂 → 整合位置並夾回世界。刻意「先阻力再油門」,使滿油門對齊方向的離散
    平衡速率正好收斂到 `VEHICLE_ACCEL / VEHICLE_DRAG == VEHICLE_MAX_SPEED`(且與 dt 無關);
    最高速定為走路 3 倍(`PLAYER_SPEED * 3`,對應驗收「明顯比走快」)。慣性靠速度在 tick 間
    延續、甩尾靠轉向時舊速度衰減而非瞬間歸零(GDD「會慣性/輕微甩尾」),方向沿用
    `Player::step` 的對角線正規化(斜開不變快)、世界邊界夾制(撞牆該軸速度歸零)。沿用載入
    防線:`at(x,y)` 比照 `positions::spawn_at`(非有限退回地圖中央、界外夾回邊界)、
    `is_loadable`(位置/速度皆有限,接 0-E 載入時驗證)、衍生 serde 為持久化格式地基;非正/
    非有限 dt 視為 no-op。延續本專案「純邏輯可測、無 IO、不碰 ws/遊戲迴圈、標
    `allow(dead_code)` 待接線」的前置慣例。加 11 個單元測試(靜止起步、`at` 收壞座標、油門
    建速前進、放油門滑行、轉向帶殘速甩尾、滿油門收斂到上限且≈3倍走路、斜開不超上限、撞牆
    夾制歸零、非正/非有限 dt no-op、`is_loadable` 拒非有限、serde round-trip),`cargo test`
    207 綠、`cargo build`/clippy 無警告、伺服器二進位啟動正常(埠被正式服務占用屬預期)。
    **仍待**:接線(世界撒佈載具實體+ws 走近按鍵上下車、上車後方向輸入導向 `Vehicle::step`
    +遊戲迴圈每 tick 推進+快照廣播載具/前端畫車與騎乘)屬動 live 廣播 shape 的架構級接線,
    留待後續輪/PR;載具靜止狀態持久化接 0-E。
  - ✅ 前置之二(載具世界佈置與上車偵測純邏輯,2026-06-05):`vehicle.rs` 解了「上車後車怎麼開」,
    接線還缺另一半「載具停世界哪裡、玩家走近上的是哪一台」——比照 `gather_field.rs` 之於
    `gather.rs`。新增 `src/vehicle_field.rs` `VehicleField`(一組停在世界裡的 `Vehicle`):`new()`
    用序號在固定半徑環上**等分角度**把載具停在家園附近(出生點在中心、走幾步就騎得到,與
    `gather_field` 把節點散在更外圈曠野呼應「騎車出門採集」);等分角度 ⇒ 任兩台至少差一個夾角
    ⇒ **不重疊**。`nearest_within_reach(x,y)` 在 `BOARD_REACH` 內挑**最近**那台回序號(非有限座標
    回 `None`,比照 `gather_near`;權威由伺服器判,客戶端只送上車意圖),「這台有沒有人騎」刻意
    不在此層管——交給接線層的騎乘登記表,比照 `plots.rs`(幾何)與 `plot_registry.rs`(歸屬)分層。
    `step_ridden(index,input,dt)`/`get_mut` 供遊戲迴圈只推進有人騎的那台(空閒載具靜止)。
    **與 `gather_field` 關鍵差異**:採集節點位置固定可由序號重算,但載具會被騎著移動,位置是
    會變、要存的狀態——故 `from_saved` 不重算座標,改信任存檔位置(每台過 `Vehicle::is_loadable`
    驗證、`Vehicle::at` 已夾界),數量不符或含非有限值整組拒收讓呼叫端退回全新一組停回原位
    (延續 `field::from_tiles` 載入時驗證)。延續「純函式、無 IO、不碰 ws/遊戲迴圈/廣播 shape、
    標 `allow(dead_code)` 待接線」的前置慣例。加 13 個單元測試(滿員皆靜止、佈置確定性、停在
    世界內且近家、任兩台不重疊、最近上車挑對台、遠處/非有限回 None、只推進被騎的那台、界外
    序號 no-op、`get_mut` 界外 None、被騎移動後 from_saved round-trip、拒數量不符、拒非有限),
    `cargo test` 220 綠、`cargo build`/clippy 無警告、伺服器二進位啟動正常(埠被正式服務占用屬
    預期)。**仍待**:接線(AppState 持有 `VehicleField`+騎乘登記表+ws 上下車+遊戲迴圈推進+快照
    廣播+前端畫車與騎乘)屬動 live 廣播 shape 的架構級接線,留待後續輪/PR。
  - ✅ 前置之三(騎乘登記表純邏輯,2026-06-05):`vehicle.rs`(車怎麼開)與 `vehicle_field.rs`
    (車停哪、上哪一台)都刻意把「這台有沒有人騎、是誰在騎」往外推給一張登記表——比照
    `plots.rs`(幾何)與 `plot_registry.rs`(歸屬)分屬兩層。新增 `src/ride_registry.rs`
    `RideRegistry`:玩家 id ↔ 載具序號的雙向對應(兩張 `HashMap` 互為反向),外加權威
    不變式「一台車至多一個騎士、一個騎士至多一台車」。`board(user,index)` 對別人已騎的車
    回 `false`(招牌「一台車只准一人騎」)、對自己重複上同台 idempotent、上新車前自動讓出
    舊車;`disembark(user)` 放掉車回被釋放序號(下車與**斷線 cleanup** 都要叫,否則車被鬼
    佔住);`vehicle_of`/`rider_of`/`is_ridden` 供接線層查詢。**與 `plot_registry` 關鍵
    差異**:地塊歸屬持久、只增不減,騎乘是連線期暫態、可釋放——故無「序號單調遞增」那套,
    改維護雙向一致;也**刻意無 `from_saved`**,比照同為連線期暫態的 `connections.rs`(重啟
    時無連線存活⇒無人騎⇒全部載具空閒,`vehicle_field` 仍從存檔還原停放位置),不發明對不上
    語意的持久化路徑。延續「純函式、無 IO、不碰 ws/遊戲迴圈/廣播 shape、標 `allow(dead_code)`
    待接線」的前置慣例。加 8 個單元測試(空車可上、別人已騎拒絕且不頂掉既有騎士、同人同台
    idempotent、換車自動讓舊車、下車釋放且可被他人接手、沒在騎下車 no-op、多人各騎各的互不
    干擾、空車無騎士),`cargo test` 228 綠、`cargo build`/clippy 無警告、伺服器二進位啟動正常
    (埠被正式服務占用屬預期)。**仍待**:接線(AppState 持有 `VehicleField`+`RideRegistry`+ws
    上下車+遊戲迴圈推進+快照廣播+前端畫車與騎乘)屬動 live 廣播 shape 的架構級接線,留待後續輪/PR。

- [ ] **Phase 1-F:戰鬥 MVP(自動打怪)** —— 對齊 PLAN.md 當前主攻 slice 3
  世界出現可打的目標,角色能**自動攻擊**附近敵人、掉落物進背包。先做最薄一條:有敵人、
  會自動打、掉東西;裝備/技能/數值成長隨後一條條補。
  - 驗收:世界看得到敵人、角色靠近自動攻擊、打倒掉落進背包、敵人過一段時間重生;
    `cargo test` 涵蓋戰鬥純邏輯(扣血、致命那下一次性掉落、重生、不超殺)。
  - ✅ 前置(敵人模型純邏輯地基,2026-06-05,對齊 PLAN.md slice 3「戰鬥 MVP 自動打怪」):
    戰鬥此前完全沒有任何模型——新增 `src/combat.rs` `Enemy`,嚴格鏡像 `gather.rs` 的
    `ResourceNode`:狀態只有「剩餘生命 + 重生倒數」兩欄,存活/被打倒由生命推導(單一真實來源)。
    `attack(power)` 扣血、**只在致命那一下**回傳掉落 `Some((ItemKind, 量))`(戰利品只給一次),
    未致命回 `None`、攻擊重生中的敵人 no-op、過量傷害飽和夾到 0 不 underflow 不多掉;`tick(dt)`
    只對被打倒的敵人倒數重生、到點滿血復活(擋非正 dt)。`EnemyKind` 兩種(銹蝕巡邏機/迷途乙太靈),
    主題療癒:敵人是失控機械/野化乙太生靈,「打倒」近似安撫拆解。**掉落刻意沿用既有 `ItemKind`
    (Stone/Ether)、不另開物品變體**——戰鬥因此自包含、不動 backend 正在接線的 `inventory.rs`,
    掉落也直接咬進採集/合成已有的資源經濟。`threat`(敵人反擊傷害)留作調校常數、待玩家生命值
    那條切片才接。沿用載入防線:`is_loadable`(生命不超上限、重生倒數有限非負,`u32` 型別本身擋
    NaN/負值)供接 0-E 載入時驗證,衍生 serde 為持久化格式地基。延續本專案「純邏輯可測、無 IO、
    不碰 ws/遊戲迴圈、標 `allow(dead_code)` 待接線」的前置慣例。加 13 個單元測試(滿血可打、未致命
    不掉、致命掉落且進重生、逐下打只掉一次、打重生中敵人 no-op、零傷 no-op、過量夾零只掉一次、
    倒數重生、活著 tick no-op、非正 dt no-op、整圈打→重生→再打、掉落為既有資源、載入防線收壞值、
    serde round-trip),`cargo test` 266 綠、`cargo build`/clippy 無警告。**仍待**:接線(比照
    `gather_field.rs` 另立 `enemy_field` 管敵人佈置與自動鎖定最近敵人+ws 自動攻擊進背包+遊戲迴圈
    推進重生+快照廣播+前端畫敵人/戰鬥回饋)屬動 live 廣播 shape 的架構級接線,留待後續輪/PR。
  - ✅ 前置之二(敵人世界佈置與自動鎖定純邏輯,2026-06-05):`combat.rs` 解了「一隻敵人怎麼被打」,
    接線還缺另一半「敵人擺世界哪裡、玩家走近自動鎖定哪一隻」——比照 `gather_field.rs` 之於
    `gather.rs`。新增 `src/enemy_field.rs` `EnemyField`(一組散佈的 `PlacedEnemy`=座標+`Enemy`):
    `new()` 用確定性雜湊(splitmix64 風格、不靠亂數/時鐘,種子刻意與節點不同避免疊在同點)把敵人散在
    比採集節點**更外一圈**的曠野(中央家園→內圈採集→外圈打怪的漸進探索節奏),座標由序號推導故重啟
    後落同處;`tick(dt)` 一次推進全部敵人重生;`attack_nearest(x,y,power)` 在 `ATTACK_REACH` 內鎖定
    **最近且仍存活**的那隻打一下——回 `(種類, 掉落)`:範圍內無存活敵人回 `None`,打中未致命回
    `(kind, None)`(供前端受擊回饋),致命那下回 `(kind, Some(loot))`(沿用 `Enemy::attack` 掉落只給
    一次,權威由伺服器判定、客戶端只送「我在攻擊」意圖)。沿用載入防線:`attack_nearest` 擋非有限座標
    (比照 `gather_near`)、`from_saved` 比照 `gather_field::from_saved` 驗敵人數/種類對齊序號/逐隻
    `is_loadable`,壞檔整組拒收讓呼叫端退回全新一組;敵人不像載具會被移動(位置固定可由序號重算),故
    只存讀會變的生命/重生狀態、座標一律重推。延續「純函式、無 IO、不碰 ws/遊戲迴圈/廣播 shape、標
    `allow(dead_code)` 待接線」的前置慣例。加 12 個單元測試(滿員皆存活、佈置確定性、避開中央淨空且在
    世界內、兩種齊全、最近鎖定扣血、遠處/非有限回 None、致命掉落且 tick 重生、被打倒不再被鎖定、
    from_saved round-trip、拒數量不符、拒種類不符/壞值),`cargo test` 278 綠、`cargo build`/clippy
    無警告、伺服器二進位啟動正常(埠被正式服務占用屬預期)。**仍待**:接線(AppState 持有 `EnemyField`
    +遊戲迴圈 tick+ws 自動攻擊進背包+快照廣播+前端畫敵人/戰鬥回饋)屬動 live 廣播 shape 的架構級接線,
    留待後續輪/PR。
  - ✅ 前置之三(玩家生命值純邏輯——承接敵人 `threat`,2026-06-05,對齊 PLAN.md slice 4「戰鬥 MVP」):
    `combat.rs` 早就備好 `EnemyKind::threat`(敵人每次反擊的傷害),卻一直「無呼叫端、待玩家生命值那條
    切片才接」——戰鬥若只能無傷收割敵人就沒有風險、是空殼。新增 `src/vitals.rs` `Vitals` 補上那塊承接點,
    嚴格鏡像 `combat.rs` 的 `Enemy`:狀態以剩餘生命為單一真實來源,`take_damage(power)` 飽和扣血(過量夾 0
    不 underflow)、致命那下回 `true` 並進入「被打趴」、`tick(dt)` 對被打趴者倒數復原(到點滿血、擋非正/
    非有限 dt)。療癒主題刻意**無永久死亡**:血歸零是「虛脫」、原地休息 `RECOVERY_SECS` 秒滿血復原(比照
    `Enemy` 重生節奏);另加脫離戰鬥一段冷卻後的自然回血(整數血量靠小數累積器湊滿 1 點才加,維持
    `[0,1)` 不變式),讓人放鬆探索不必怕一路掉血到底。沿用載入防線:`is_loadable`(生命不超上限、各計時器
    有限非負、回血累積落 `[0,1)`,`u32` 型別擋 NaN/負值)供接 0-E 載入時驗證,衍生 serde 為持久化格式地基。
    延續本專案「純邏輯可測、無 IO、不碰 ws/遊戲迴圈、標 `allow(dead_code)` 待接線」的前置慣例;刻意只寫新檔、
    不碰 backend 正在接線的 ws/快照 shape。加 16 個單元測試(滿血起步、未致命扣血、致命打趴並啟動復原、過量
    夾零只打趴一次、趴著再挨打 no-op、零傷 no-op、復原倒數到點滿血、冷卻內不回血、脫戰後回血、回血不溢出、
    滿血 tick no-op、非正/非有限 dt no-op、整圈打趴→復原→再打趴、載入防線收壞值、serde round-trip、
    **敵人 `threat` 餵進 `take_damage` 的跨模組組合測試**),`cargo test` 299 綠、`cargo build`/clippy 無警告、
    伺服器二進位啟動正常(埠被正式服務占用屬預期)。**仍待**:接線(玩家帶 `Vitals`、附近敵人每 tick 用
    `threat` 反擊扣血、被打趴期間暫停行動/重生回出生點、快照廣播血量、前端畫血條)屬動 live 廣播 shape 的
    架構級接線,留待後續輪/PR;生命值跨重啟持久化接 0-E。

## 玩家回饋處理區(devloop 從 data/suggestions.jsonl 進來的)

> devloop 處理過的玩家建議在此打勾並引用建議 ID/摘要;處理中的也記在這。

- [x] **小地圖(2026-06-05,建議 at=1780624298043「2000x2000 大世界不知道自己在哪」)**：
  右下角加固定大小世界縮圖,畫出世界邊界(黃銅框,沿用世界邊框語彙)、農地位置(褐塊)、
  自己(亮金點)、其他玩家(暗藍點),每幀依最新快照重畫。純前端 `game.js` 新增 `drawMinimap`,
  用渲染插值座標 rx/ry 與主畫面同步;在日夜染色之後畫,當 HUD 不被夜色蓋暗。bump
  `index.html` 的 `game.js?v`。純前端、`cargo test` 122 綠不受影響。
- [x] **Google 登入別洩本名(2026-06-05,建議 at=1780631336007「GOOGLE登入直接本名 有隱私問題,
  建議新玩家用隨機角色名稱」)**：過去 `auth.rs` 直接拿 Google `userinfo.name`(真實姓名)當顯示名,
  而顯示名會廣播成聊天 `from` / HUD 名,等於把本名公開給所有玩家。改為新帳號配一個與主題相襯的
  隨機代號(`users::random_codename`,形如「黃銅領航員-417」,形容詞×名詞×尾碼)。既有帳號不受影響
  ——`find_or_create` 命中即早回,根本不會走到產名。純邏輯 `codename_from_seed(seed)` 抽出可測,
  加 3 個測試(形狀/通過 sanitize 不變/不同 seed 夠分散)。`GoogleUserInfo.name` 保留欄位但標
  `#[allow(dead_code)]` 文件化「收得到但刻意不採用」。`cargo test` 131 綠。**仍待**:玩家自訂暱稱
  的編輯入口(這輪只堵本名外洩;改名 UI 是更大塊,留待後續)。
  - ✅ 改名後端半(`PATCH /api/profile` + 持久化,2026-06-05,對齊 PLAN.md 高優先薄切片 slice 2
    「玩家改顯示名」):上一輪只堵本名外洩、留下「玩家自訂暱稱的編輯入口」待做——這輪補上其**後端
    半**。新增 `src/profile.rs` `PATCH /api/profile`(body `{name}`):用既有 `auth::user_id_from_cookies`
    驗身(只能改自己的名、uid 取自呼叫者簽章 session),名字過既有 `sanitize_name`(濾控制字元/截 24
    字/空退「拓荒者」,與帳號建立、訪客進場共用同一道公開輸入邊界),回出清理後實際結果供前端更新 HUD;
    未設 OAuth→503、未登入/cookie 無效→401。`UserStore` 加 `rename`:**append 一筆同 id、新 name 的
    紀錄**(沿用本檔 append-only JSONL,不重寫/不刪除舊行,非破壞),載入時後者勝出。因 `ws.rs` 連線時
    即時讀 `UserStore`(authed `user.name`),改名後**重連**即生效、重啟也還在(驗收「重連仍是新名」)。
    把索引邏輯抽成可測純函式 `index_users` 鎖住「同 id last-wins」契約(改名靠它生效)。刻意**不碰 ws/
    遊戲迴圈/廣播 shape**——「線上不重連就即時反映 HUD/聊天」的 live 廣播依 PLAN.md 分工留給 backend lane,
    前端改名面板留給 frontend lane。加 2 個 `index_users` 測試(同 id 後者勝出、互異 id 各自保留),
    `cargo test` 280 綠、`cargo build`/clippy 無警告、伺服器二進位啟動正常(埠被正式服務占用屬預期)。
    **仍待**:前端改名設定面板(frontend lane);線上即時廣播新名(backend lane);改名跨重啟持久化已
    靠 append 成立,接 0-E 後 users 改存 Postgres 時 `rename` 同步改走 upsert。
- [ ] **像素風 sprite(建議 at=1780624298053)**：屬美術方向,已有 commit 5530cb9
  「美術素材簡報 + 角色全自由化」在規劃、需素材包,非單輪小增量,留待美術素材到位。
  - ✅ 純程式先補兩塊「不靠素材也能做」的視覺(2026-06-05)：建議三點裡的第 2 點
    「草地用 noise/瓦片紋理」(commit fd0e7ac,座標雜湊撒草叢)與第 3 點「田地周圍加木柵欄」
    (本輪)都用程式生成、不引入任何美術素材即可落地。木柵欄:`game.js` `drawField` 新增
    `drawFence`,沿田四邊立等距木樁 + 兩條橫桿,樁位由邊長算固定間距(鏡頭移動不抖),
    畫在黃銅框外側不蓋作物格,讓田看起來像「圈起來的農莊」。真 tileset/sprite 進來可直接
    替換這些佔位繪製。bump `index.html` 的 `game.js?v`。純前端、`cargo test` 122 綠不受影響。
  - ✅ 第 1 點的無美術部分也補上了(2026-06-05,commit c785a59)：玩家角色有了「走路感」——
    依插值位移推出朝向(放一顆黃銅護目鏡點指向移動方向)、踏步時上下彈跳、腳下固定陰影,
    全程式生成、套用到所有玩家。建議三點的「不靠素材也能先做」部分至此全數落地。
    **仍待**:真正的 16x16 像素 sprite 與逐格走路動畫(需美術素材包,留待素材到位)——
    現有皆為程式佔位繪製,素材進來可直接替換。

## 選用基礎建設

- [ ] **Phase 0-H 進階(選用)：容器化 + 雲端代管**
  目前已透過 Cloudflare Tunnel 自架上線。之後若想關機也能玩或要切到雲端,
  容器化 + Fly.io 是路。

## 之後（Phase 2+，見 GDD）

自動化與牧場(缺氧層)、職業與社交(RO 層)、卡丁車競速(極速層)、星際擴張(北極星)。
這些在 Phase 0/1 穩定可玩、且使用者點頭前,**不要動工**。devloop 推進到接近時
再把該 Phase 的具體 tickets 攤下來。
