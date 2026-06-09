# ButFun 主軸 Roadmap（自走 worker 照這個由上往下，不准漂）

> 自走 worker 每輪從這裡挑「**最上面未完成的主軸項**」做。讀這個檔（小），**不要**為了找事做去
> 掃整份 `docs/BACKLOG.md`（那會載入大量 context 燒 token，也是過去「只補洞不長主軸」的病根）。
>
> **鐵律：每個 PR 都要能讓玩家「看得出遊戲長大了」**——大玩法／大地圖／世界成長優先。
> **純補洞、重構、輸入加固、效能微調都不算主軸**；只有「**擋住主軸**」或「**線上真的壞了**
> （`journalctl -u butfun` 有 error/panic）」才做，做完立刻回主軸。

## ✅ 已完成（可靠性 — 2026-06-09 prod 事故後）
0. ✅ **e2e 冒煙閘**
   - `scripts/e2e/gameloop-smoke.mjs`：連 `/ws` → 斷言快照含自身 id → 斷言兩幀 tick 推進。
   - `scripts/e2e-gameloop.sh`：自起 server（PORT 19847，記憶體模式）→ 跑冒煙 → 殺 server；可無頭跑。
   - `scripts/deploy.sh`：healthz 過後加 WS 冒煙閘，失敗自動回滾。
   - reviewer 硬閘備忘：合併前請在 PR 分支跑 `bash scripts/e2e-gameloop.sh` 確認通過。

## ✅ 已完成（新手體驗 — 2026-06-09）
4. ✅ **新手村 + 死亡重生**（PR #56）
   - 出生點改成**無怪安全城鎮**（把現有公共農地圈進去當起手村）；怪只生在城鎮外一定半徑之外。
   - 死亡要有後果：倒地/定身 → 回新手村重生（可掉少量背包，療癒向不硬核）；**死亡狀態下不能再移動/採集/挖**。
   - 純邏輯（生怪排除半徑、死亡狀態機、重生點）抽成可測函式。

## ✅ 已完成（城鎮交易 — 2026-06-09）
5. ✅ **城鎮交易（NPC 商人 / 商店）**（PR #57）
   - 新手村公共農地旁擺固定商人 NPC（`npc.rs`）：收購木材/石頭/乙太礦 → 玩家得乙太；賣鎬子/武器 → 玩家花乙太。
   - 給新手穩定的「第一桶乙太」出口，不必等農地收成或玩家市場。
   - 前端：繪製商人圖示、靠近自動亮 dock 點、商店面板顯示買賣目錄與數量輸入。

## ✅ 已完成（可挖地形地基 — 2026-06-09）
6a. ✅ **可挖地形 C-1：tile 層地基**（PR #58）
   - `world-core`：`TileKind` enum + `tile_kind_at` 確定性生成。
   - `tiles.rs` + `tile_store.rs`：記憶體 delta map + DB 持久化層（`tile_deltas` 表）。
   - `protocol.rs`：`TileDeltaView` + Snapshot 加 `terrain` 欄位（C-1 永遠空）。
   - 前端：`tileKindAt` 本地確定性生成（零帶寬）+ `drawTerrain` 畫色塊地形。

## 現在做
6d. **可挖地形 C-4：建造（Place handler）**（PR 開發中）
   - `ClientMsg::Place { wx, wy, material }` + ws.rs handler（可及性 80px、空格才放、背包扣 1 + delta 設實心 + 持久化）。
   - `tiles::tile_for_item` / `item_for_placeable_tile` 純函式（dirt / stone 可放；ore / ether 不可放）。
   - 前端：背包行加 🏗️選取鈕 → 右鍵點空格放置；HUD 放置模式 pill。
   - 至此玩家可「挖通地形 → 把材料堆回去蓋洞穴家 / 洞穴城堡」。
6e. **正宗 Core Keeper：實心世界 + 洞窟（玩家 2026-06-09 拍板的核心地形）→ 見 `docs/PLAN_DIGGABLE_WORLD.md` Phase 2**
   - **順序很重要**：先 **D-1 怪/資源只生在開放格**（spawn 落實心就搬到最近 Empty；先上、現在無害）
     → 再 **D-2 反轉地形為實心 + 洞窟雜訊**（安全區外預設實心、主動挖隧道開路；密度 playtest 微調）
     → **D-3 小地圖導航**（修「像素隨移動變形」+ 畫實心/洞窟/已挖路 + 新手村「家」標記）。
   - 動機：玩家實測「開放地 + 散落土塊」是兩邊缺點；要 Core Keeper 那種「整片實心、主動挖隧道」。
     **別跳序**：先讓怪/資源避開實心（D-1），再反轉地形（D-2），否則怪會被埋進石頭被碰撞卡死。

## 已完成（C-2、C-3）
6c. ✅ **可挖地形 C-3：tile 碰撞**（PR #61）
   - `Player::step` 加入 `tile_solid` 閉包參數，以中心判逃脫 + 四角判碰牆的兩層策略。
   - `game.rs`：每 tick 快照 tile deltas（先釋放讀鎖再取 players 寫鎖，避免死鎖）。
   - 3 個純邏輯測試：擋住直線移動、沿牆滑行、受困逃脫。

6b. ✅ **可挖地形 C-2：Dig handler**（PR #59）
   - 後端：`ClientMsg::Dig { wx, wy }` + ws.rs handler（可及性 80px、實心→Empty + 材料入背包 + 持久化）。
   - `ItemKind::Dirt`（土磚，C-2 掉落 + C-4 建造材料）加入物品宇宙。
   - `game.rs`：快照的 `terrain` 欄位從 TileWorld deltas 填充（C-1 時永遠為空）。
   - ws.rs AOI 過濾：地形 delta 依格中心距離剔除。
   - 前端：點實心地形格→送 dig + 接收 terrain delta（含 empty 覆蓋）。

## 已完成
1. ✅ **③ 無限世界**（切片 A~D 全進 main：拿掉邊界、區塊確定性生成、AOI 剔除、領地重置）
2. ✅ **生態域有內容**：資源/敵人依生態域分佈（森林/草地/岩地/沙漠），水域擋路
3. ✅ **領地 + 經濟**
   - ✅ 買斷地（ClaimPlot，花 20 乙太獲得保護 + 所有權）
   - ✅ 地價系統（擴地費用線性遞增，economy.rs）
   - ✅ soft raid（公共農地，任何登入玩家均可耕種，PR #46）
   - ✅ 市場（玩家對玩家交易：掛單/購買/取消，listings 含 AOI 廣播，PR #51）

## 接著（北極星玩法，已拍板 2026-06-09）
6. **可挖地形 + 建造（Core Keeper 風・俯視）→ 詳見 `docs/PLAN_DIGGABLE_WORLD.md`**
   - 格子化可破壞地形：挖通礦脈/泥土、把挖到的材料**堆回去蓋「洞穴家 / 洞穴城堡」**。
   - **使用者明講要的核心風格**：從療癒農場往「挖掘 + 建造」長。**俯視 top-down（Core Keeper），非缺氧那種側視剖面**。
   - **大切片、架構級**：照設計文件 additive-first 切；最危險的「tile 碰撞」那刀**務必先有冒煙閘**守。

## 之後（願景，別搶跑）
7. 角色自由創造（外觀分件 + 配色、屬性自由分配）
8. 多生態玩法 / 多星球前奏

## 「主軸 vs 補洞」判準（worker 與 reviewer 都照這個）
- 會讓玩家**看到新東西 / 新玩法 / 更大的世界** → 主軸，做。
- 只是讓既有東西**更安全 / 更快 / 更乾淨、玩家無感** → 補洞，**除非擋路否則跳過**。
- 玩家建議（`data/suggestions.jsonl`）：符合主軸的排進來；要取捨的升級給人（`for_human.md`）。
