# ButFun Voxel 世界 — AI 生態世界的 voxel 基底（設計藍圖）

> 狀態：**進行中（切片① 已落地）**。維護者 2026-06-29 拍板：ButFun 的「由 AI 棲居、AI 主導
> 的持續世界」（見 `PLAN_AI_INHABITED_WORLD.md` / `VISION_AI_EMERGENT_ECOSYSTEM.md`）改用
> **voxel／麥塊基底**——一個方塊組成、可挖可蓋的 3D 世界，讓自主 AI agent 真的能「拿方塊蓋東西」。
>
> **這份是新方向的第一刀。鐵律：並行於現有世界、絕不打掉現有的東西。**

---

## 0. 一句話

**Rust 後端＝權威 voxel 世界；前端（Three.js）只渲染。** 延續 ButFun 一貫的
「**後端權威、前端渲染**」骨架，把世界的「真相」從 2D tile 換成 3D 方塊（voxel chunk），
讓既有的靈魂——AI 大腦＋LLM 池、禱告→實現迴圈、帳號、經濟——日後直接搬進來，
並讓 AI agent 能在世界裡放／挖方塊、自己蓋家、長出小社會。

---

## 1. 為什麼是 voxel

- **AI 要能「動手蓋」**：自主 agent 的湧現行為需要一個「可被程式化改變的世界」。方塊世界是
  最簡單、最通用、最好讓 LLM 規劃「在 (x,y,z) 放一塊木頭」的表徵——把「蓋房子」化約成一串放塊指令。
- **後端權威天然契合**：voxel 世界＝一堆 `HashMap<座標, 方塊>`，本就是伺服器端確定性資料結構，
  完全延續 ButFun「世界真相在 Rust、前端只畫」的既有模式（對齊 `world-core` 的設計哲學）。
- **無限延伸**：chunked + 程序生成 → 世界可隨 agent 探索無限長大，不必預先做地圖。

---

## 2. 邊界與隔離（最重要）

- **並行開新路由 `/voxel/`**（新前端頁）。**現有 `/`、`/3d/`、`/play3d/`、2D、所有現有
  系統完全不動。** 這是 spike→長大的新世界，**穩了才談切換**。
- 後端 voxel 邏輯放**獨立模組**（`src/voxel.rs` 純世界邏輯、`src/voxel_ws.rs` 連線層），
  **不碰 `game.rs` / `state.rs` / `ws.rs` / `protocol.rs`**。voxel 連線用**自己的 WS 路由
  `/voxel/ws`**＋自己的玩家登錄（模組內 `OnceLock` 全域 hub），與現有 2D/3D 協定零交集。
- 不破壞既有 `cargo build` / `cargo test` / 玩法（voxel 全隔離、additive）。

## 3. 法律

- **不 clone Minecraft 的碼或美術。** 只借「方塊世界」這個通用概念。
- 地形噪聲**自己寫**（`voxel.rs` 內的 hash-based value noise，零外部相依，確定性、可測），
  不引入第三方 noise crate（避免授權與相依膨脹；日後若需要再挑 MIT/Apache 的）。
- 方塊顏色程序生成（純色 + 法線著色），貼圖如要再用 CC0／程序生成。

---

## 4. 漸進切片規劃

- **① 可走的 voxel 世界（本輪 ✅）**：Rust 權威 chunk 儲存＋程序化地形生成；WS 串周邊
  chunk 給 `/voxel/` 前端；Three.js 收 chunk → 合併 mesh（面剔除）→ 玩家能走在地形上
  （重力＋voxel 碰撞）＋鏡頭跟隨＋鍵盤／觸控。
- **② 放／挖方塊**：客戶端送「放/挖 @ (x,y,z)」→ 伺服器改權威 chunk → 廣播 delta → 各
  客戶端就地重建受影響 chunk 的 mesh。方塊改動持久化（接 `state.rs` 後的抽換點，或新表）。
- **③ AI agent 移進來＋會蓋**：把現有 NPC agent（腦＋記憶＋需求＋性格）接進 voxel 世界，
  讓它們在世界裡走動、並用②的放塊能力蓋簡單結構。
- **④ 禱告驅動蓋家**：把「禱告→實現」迴圈接上 voxel——玩家／居民的願望變成 agent 的蓋造目標。
- **⑤ 搬農田／NPC／經濟＋持久化＋居民散佈**：把驗證過的系統逐步移植進 voxel 世界。
- **⑥ 成熟後切換**：voxel 世界穩到能取代現有試驗場時，才談把它變成預設。

---

## 5. 切片① 的技術做法（本輪實作）

### 5.1 後端權威 voxel（`src/voxel.rs`，純邏輯、可測）

- **方塊型別** `Block`（`#[repr(u8)]`）：Air / Grass / Dirt / Stone / Sand / Wood / Leaves / Water。
- **chunk**：邊長 `CHUNK = 16`（16³ = 4096 方塊），`Chunk { blocks: Vec<u8> }`，
  以 `ChunkCoord { cx, cy, cz }` 索引（世界用 `HashMap<ChunkCoord, Chunk>`，本輪伺服器
  其實「無狀態程序生成」——不存 chunk，收到請求就地用噪聲算，省記憶體、天然無限）。
- **程序化地形**：`height_at(wx, wz)` 用自寫 hash value noise（多 octave）算高度圖；
  `block_at(wx, wy, wz)` 依高度填 草／沙（近海）／土／石，海平面下補水。**確定性**
  （同座標永遠同結果）→ 前後端、多人之間天然一致。
- **精簡串流格式**：`pack_chunk(cx,cy,cz)` 產生該 chunk 的 4096 bytes → base64 字串；
  **全空氣的 chunk 回 `None`**（不傳，省頻寬，高空 chunk 幾乎都被略過）。
- 純函式測試：chunk 索引往返、座標換算、地形決定性、海平面/草地規則、全空氣偵測。

### 5.2 WS 協定（additive，獨立路由 `/voxel/ws`，JSON）

- Client→Server：`join{name}`、`move{x,y,z,yaw}`、`req{cx,cz}`（走到新區塊時要更多 chunk）。
- Server→Client：`welcome{id,spawn,sea,chunk}`、`chunks{chunks:[{cx,cy,cz,data(base64)}]}`、
  `players{players:[{id,name,x,y,z,yaw}]}`（其他玩家位置）。
- 連線層 `voxel_ws.rs`：模組內 `OnceLock<VoxelHub>`（玩家表＋broadcast）做多人位置同步，
  **完全不碰 AppState**。入場送周邊半徑的 chunk；move 即更新並廣播。

### 5.3 前端 `/voxel/`（`web/voxel/index.html` + `main.js`）

- Three.js（CDN importmap，比照 `/3d/`）。收 chunk → **每 chunk 合併成一個
  BufferGeometry**（面剔除：只送與空氣相鄰的面）→ 一個 Mesh。**絕不每方塊一個 mesh**
  （記取 #614 教訓）。
- 移動：WASD／觸控搖桿，相對鏡頭 yaw；重力 + 對權威方塊做逐軸 AABB 碰撞 → 能站在地形上、
  能走。鏡頭第三人稱跟隨，滑鼠／觸控轉視角。走到 chunk 邊界送 `req` 取更多。
- 版本：後端 `serve_voxel_index` 注入 `main.js` 內容雜湊（比照 `serve_3d_index`）＋
  `?debug` 顯示 FPS／chunk 數。

### 5.4 效能鐵律

- **一個 chunk 一個（或少數）合併 geometry**，面剔除去掉看不見的內面。別每方塊一 mesh。

---

## 6. 品質閘門

`cargo build` + `cargo test` 全綠（voxel 純邏輯附測試）；`node --check` 過；
現有 build/測試/玩法零回歸（voxel 全隔離）；伺服器啟動不 panic；
真瀏覽器（puppeteer + 系統 chrome）驗：載入 `/voxel/`、真的渲染出方塊地形、角色能走、非黑屏、量 FPS。
