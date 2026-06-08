# ③ 無限世界 + 領地經濟 — 開發藍圖（dev session 交棒）

> 這份檔是「上一個 dev session（context 已壓縮多次、很肥）」交棒給「全新乾淨 session」的藍圖。
> 新 session 請依序讀：本檔 → `CLAUDE.md`（邊界/品質閘門）→ `docs/GAME_DESIGN.md`（世界觀）→
> `docs/BACKLOG.md`（Phase 0-G-O2）→ 記憶索引 `MEMORY.md`。讀完就從「起手式」開工。

## 使用者（施育群）這個 session 拍板、但還沒進其他文件的設計決定

1. **維持 2D**（療癒、好操作、手機友善），但**強化 Rust 可參與的部分**：render-agnostic
   的 `world-core` crate（已建），WASM-ready；**保留未來 AR/VR 擴充性**（邏輯與渲染分離，
   之後換 3D 前端不用重寫世界規則）。不要為了酷炫上 3D / Three.js / Bevy——使用者明確選 2D。
2. **規模目標**：至少星露谷、甚至到 RO 規模；無接縫世界、多樣生態（洞穴、海）、本機小地圖。
   已上線：程序生態域地表（草原/森林/岩/沙/水、無接縫）、本機小地圖（可縮放）。
3. **③ 無限世界** = 拿掉現有 2000×2000 邊界、區塊化/離屏渲染、per-chunk 確定性程序生成。
4. **領地系統在 ③ 時重置**（使用者**明確授權**，data-safety 對「地塊配置」這一項轉換放寬）：
   一開始世界**空的**、要自己慢慢攢，**不要一進場就送地**（現在是進場分配地塊，要改掉）。
   ⚠️ 重置只清「地塊配置/歸屬」，**不准** drop 玩家的乙太/背包/帳號/作物資料——走 migration。
5. **領地經濟**（③ 之後再做）：
   - 買地 = 買「**保護 + 所有權**」，**買斷不租**（已拍板）。
   - 買下的土地上的東西**不會被偷採/破壞**；沒買的土地**一樣能耕種**，但**有機會被盜/被破壞**。
   - 被盜被破壞要做成 **soft raiding**（保留療癒感，別硬核 PvP 掠奪）。
   - 有**地價系統 + 市場機制**。遊戲內貨幣類比**比特幣**（未來「或許」接真加密貨幣＝**遠期、
     有法務/安全顧慮、現在絕對別做**）。
6. **礦物**：採完消失→隨機重生（vanish 已上線；relocate-on-respawn 已有）；有些地方不能生成
   （水）；**裝備壞了去找礦長回來**（裝備耐久＝待做，會動 inventory model＝玩家資料級重構，走 PR）。
7. **野怪洞穴**：可**挖進去**（⑥ 可挖地形，Core Keeper 風的 delta-save 地形）。

## 目前架構現況（接得上的基件，別重造）

- **`crates/world-core`**（render-agnostic 確定性世界邏輯，native lib 給 server + wasm32 cdylib 給前端）：
  - `biome_at(x,y) -> Biome`：確定性 value-noise 生態域（Water/Sand/Meadow/Forest/Rocky），
    任意座標、無接縫 → **世界「地形上」其實已經無限**，前端只是被夾在邊界。
  - `resolve_move(cur,new,blocked)`：滑動碰撞純函式，**已 land、尚未接線**，等 ⑤/③ 接上。
  - JS↔Rust noise **位元一致**（整數 hash），已驗 2000 隨機點 byte-identical。改 noise 要兩邊同動。
  - WASM 走 `#[no_mangle] pub extern "C"` + 瀏覽器 `WebAssembly.instantiate`（不靠 wasm-bindgen）；
    `scripts/build-wasm.sh` 建到 `web/wasm/`。
- **內容生成現況（這是「有限世界」的做法，③ 要改）**：節點（樹/石/乙太礦，`src/gather_field.rs`）
  與敵人（`src/enemy_field.rs`）目前是「**固定 N 個、在世界範圍內螺旋撒點**」+ biome 適配
  （`scatter_position` / `place_for_kind` / `biome_suits_kind`）。③ 要改成「跟著探索區/玩家周邊、
  **區塊式無限生成**」。
- **邊界現況**：前端 `web/game.js` 把鏡頭/移動夾到世界邊界、小地圖畫世界邊框；後端 `Player::step`
  clamp 到世界範圍。③ 要拿掉這些夾制。
- **持久化**：位置/背包/農地/日夜/帳號**全接 Postgres**（0-E 完成，`src/*_store.rs` 三後端
  Postgres/Jsonl/Memory）；農地 **per-player**（PR #12 上線，`src/plots.rs` 螺旋往外排、序號只增不減、
  `src/plot_registry.rs` 記 user→序號）。**領地重置就是要動這兩塊**（走 migration）。

## ③ 建議切片（小而可測、每片開 PR；架構級/動玩家資料的不自走 merge）

- **切片 A — 拿掉邊界（地形側）**：後端 `Player::step` 不再 clamp 到 2000；前端鏡頭/小地圖拿掉
  世界邊框。因為 `biome_at` 本就無限，地表會自然延伸。先確保「能一直走、地表持續生成」。
  風險：節點/敵人仍在舊有限環裡 → 走遠沒內容，所以要配 B。
- **切片 B — 內容無限化（區塊式確定性生成）**：把節點/敵人從「固定 N 撒在 2000 環」改成
  「**以區塊（如 512×512 格）為單位、用確定性 hash 決定該區塊生成什麼**」——同座標永遠長一樣、
  **不用存**（跟 `biome_at` 同思路：確定性、免存、前後端可一致）。採集/擊殺狀態才需要 delta-save。
- **切片 C — 離屏剔除 / AOI（最重的一塊，netcode）**：快照從「全世界實體」改成「**以玩家為中心、
  附近區塊內的實體**」（interest management / area-of-interest），前端只渲染可見區塊。這是讓無限世界
  不會 O(世界) 爆廣播的關鍵，也是 ③ 風險最高、最該開 PR 審的部分。
- **切片 D — 領地重置 + 空世界開局**：改「**進場不送地、自己攢乙太買地**」。
  ✅ **重置範圍 = FULL CLEAN（使用者 2026-06-08 明確拍板，理由：clean slate、不留包袱不用一直閃舊資料）**：
  migration **可清空所有玩家遊戲狀態**——地塊/作物/乙太/背包（TRUNCATE 那些表 OK），讓世界重生、大家從零開始攢。
  **唯一必須保留 `users` 帳號表**（provider/external_id/name/species，讓玩家登得回來）。
  先在 staging（`butfun_staging` DB）跑、施大玩過再按 `deploy.sh` 上 prod（`butfun` DB）——兩庫隔離，prod 等人。
- **（⑤ 水擋路接線）**：排在 D 之後——plots 落陸地後，把 `resolve_move` 用
  `biome_at(..)==Water → blocked` 接上移動，玩家就不會被鎖在水田裡。碰撞「料」已備好。

## 護欄（務必遵守，來自 CLAUDE.md + 記憶）

- **品質閘門**：`cargo build`/`cargo test` 全綠、伺服器 `cargo run` 不 panic、新功能附測試。
- **架構級 / 動玩家資料 → 只開 PR、不自走 merge**（③ 切片 C/D 屬此類）；小而綠的可直接進 main。
- **上線一律走維護窗 `scripts/deploy.sh`**，開發迴圈**永不**自己 `systemctl`。
- **編輯一律用隔離 worktree（`/tmp/bf-*`）**，別在共用工作樹 `/home/shihshih/ButFun` 留未提交編輯
  （會外洩進別人 commit、污染 main——這是踩過的雷）。
- **不抄外部遊戲碼**（Stardew/Minecraft/Core Keeper 反編譯、rAthena/Hercules GPL）——
  只**吸收設計概念寫原創碼**（鐵律：AI 當老師不當影印機）。
- **不破壞性刪玩家資料**；schema 變更走 migration、向後相容、別 drop 既有欄位資料。
- 機敏值走 `.env`（已 gitignored）；`data/` 是執行期資料、不提交。

## LLM 外包（省 Claude token）

- **Gemini CLI** 當主力草擬器：`gemini --skip-trust -p "...產出純邏輯 Rust 函式..."`
  （`/home/shihshih/.nvm/.../bin/gemini`，v0.45+）。雲端、快、品質好。
- **地端 gemma4:26b** 當免費 bulk 備援：
  `curl http://100.102.229.41:8434/api/generate -d '{"model":"gemma4:26b","prompt":"...","stream":false,"think":false,"options":{"num_thread":16,"num_predict":1024}}'`
  （難邏輯可切 `qwen3.5:35b-a3b`，think 乾淨但慢 ~6.6 tok/s；**別用** `qwen3:30b-a3b`，thinking 會洩漏）。
- **Claude（你）只做審 + 接線**。草擬器產的碼**一定要審**——曾抓到真 bug 與 clippy lint。
- 維運 session（expo）協作信箱在 `/home/shihshih/butfun-coord`（`dev2expo.md` 只你寫、`expo2dev.md` 唯讀）。

## 起手式

1. `gh pr list` 看有沒有 in-flight PR（避免重做）。
2. `cd /tmp && git clone/worktree` 開隔離工作樹（**別**直接在共用樹改）。
3. 從**切片 A**開始（最小、可立即線上驗證），一片一片往下；每片綠了再開 PR。
4. 動工前先 `git pull` 一次當收信（看 expo 有無留言）。
