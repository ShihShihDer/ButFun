# ButFun 開發待辦（BACKLOG）

> `/goal` 自走時的工作清單。**由上往下做**，一次一個項目。
> 完成就把 `[ ]` 改成 `[x]`，並在該項下補一行「✅ 做了什麼」。
> 遵守 `CLAUDE.md` 的邊界與品質閘門。每個項目都附「驗收標準」，達成才算完成。

## 已完成

- [x] **Phase 0-A：權威伺服器即時多人骨架**
  ✅ Rust(axum) WebSocket 權威伺服器、15Hz tick、客戶端送輸入、廣播快照；
  原生 canvas 前端可進場、WASD 移動、鏡頭跟隨、看到其他玩家。
- [x] **Phase 0-B：聊天**
  ✅ Enter 開啟輸入，訊息經伺服器廣播給所有人。
- [x] **Phase 0-C：遊戲內建議箱**
  ✅ 前端 💡 表單 → `POST /api/suggestions` → 存 `data/suggestions.jsonl`。
- [x] **Phase 0-D：自動測試閘門**
  ✅ 移動整合抽成 `Player::step`，加上單元測試；`cargo test` 全綠。

## 進行中 / 下一步（由上往下）

- [ ] **Phase 0-E：Postgres 持久化**
  把玩家位置（之後含背包 / 農地）存進 Postgres，伺服器重啟後玩家回到原位。
  - 加入 `sqlx`（Postgres、非同步），`DATABASE_URL` 走環境變數。
  - 在 `state.rs` 抽換點後面接一個 `PgStore`；無 `DATABASE_URL` 時退回現有記憶體模式，方便本機跑。
  - 加 migration 建 `players` 表（id, name, species, x, y, updated_at）。
  - 玩家進場時若 DB 有舊紀錄就載入；定期 / 離線時寫回。
  - 驗收：設好 `DATABASE_URL` 跑起來，移動後重啟伺服器，重新進場位置仍在；`cargo test` 全綠。

- [ ] **Phase 0-F：帳號身份模型（provider 無關）+ Google OAuth**
  先做 provider 無關的 `users` 模型（內部 user id；外部登入只是連結），再接 Google OAuth。
  - 內部 `user_id` 為主鍵；`auth_identities`(provider, external_id, user_id) 連結外部登入。
  - 第一個 provider 實作 Google OAuth（憑證走環境變數，**不入庫**）。
  - 角色 / 存檔改綁 `user_id` 而非每次新 uuid。
  - 驗收：用 Google 登入後刷新頁面仍是同一角色；介面留好之後加別的 provider。
  - ⚠️ 需要使用者提供 Google OAuth client id/secret —— 開始前先問。

- [ ] **Phase 0-G：種田起源（地球人 / 繼承農莊）—— 療癒核心**
  讓世界「玩起來有感覺」的第一個玩法循環。
  - 地表一塊可耕地：點格子翻土 → 種乙太作物 → 澆水 → 隨日夜成長 → 收成得乙太。
  - 伺服器驅動的日夜 / 成長計時；農地狀態持久化（接 0-E）。
  - 前端顯示耕地格與作物階段。
  - 驗收：種下、澆水、過一段時間收成拿到乙太；重啟後農地狀態還在；`cargo test` 涵蓋成長邏輯。

- [ ] **Phase 0-H：發佈管線**
  讓「改版 → 發佈」變成一個動作。
  - **已選定的上線方式**：自架 + **Cloudflare Tunnel**（使用者有網域、有可當伺服器的電腦，
    但無法設 NAT/路由器轉發）。手冊見 `docs/DEPLOY_CLOUDFLARE_TUNNEL.md`，
    cloudflared 設定範例見 `deploy/cloudflared/config.example.yml`。
  - repo 已 tunnel-ready（伺服器讀 `PORT`、綁 `0.0.0.0`；前端 HTTPS 下自動用 `wss`），
    上線屬使用者本機操作，不需改碼。
  - 之後（選用，想要關機也能玩時）：容器化、CI（build+test）、雲端代管（Fly.io）。

## 之後（Phase 1+，先別碰，見 GDD）

開車與採集、自動化與牧場、職業與社交、卡丁車競速、星際擴張。
這些在 Phase 0 穩定可玩、且使用者點頭前，**不要動工**。
