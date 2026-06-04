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
- [x] **Phase 0-F：帳號身份模型（provider 無關）+ Google OAuth**
  ✅ provider 無關 `User` 模型(`src/users.rs`,目前以 `data/users.jsonl` 持久化,
  之後接 0-E Postgres 時直接 swap);Google OAuth flow(`src/auth.rs`)含
  `/auth/google/start|callback|me|logout`、CSRF state cookie、HMAC-SHA256 簽章
  session cookie(stateless);WebSocket 升級前讀 cookie → 同 Google 帳號跨裝置
  /重連即同玩家;前端「以 Google 登入」按鈕 + 登入後跳過進場畫面、HUD 顯示用戶名
  與登出。`.env`/EnvironmentFile 載入秘密,gitignored 不入 repo。
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

- [ ] **Phase 0-F-1：補 auth 純邏輯單元測試**
  `sign_session` / `verify_session`(含偽造 token 拒絕)、`read_cookie`(多 cookie、
  含空白)、`constant_time_eq`、`sanitize_name`。純函式、無 IO、易測,補上即可
  push。驗收:`cargo test` 全綠且涵蓋上述函式。

- [ ] **Phase 0-G：種田起源（地球人 / 繼承農莊）—— 療癒核心**
  讓世界「玩起來有感覺」的第一個玩法循環。
  - 地表一塊可耕地：點格子翻土 → 種乙太作物 → 澆水 → 隨日夜成長 → 收成得乙太。
  - 伺服器驅動的日夜 / 成長計時；農地狀態持久化（接 0-E）。
  - 前端顯示耕地格與作物階段。
  - 驗收：種下、澆水、過一段時間收成拿到乙太；重啟後農地狀態還在；`cargo test` 涵蓋成長邏輯。

- [ ] **Phase 0-H 進階(選用)：容器化 + 雲端代管**
  目前已透過 Cloudflare Tunnel 自架上線。之後若想關機也能玩或要切到雲端,
  容器化 + Fly.io 是路。

## 之後（Phase 1+，先別碰，見 GDD）

開車與採集、自動化與牧場、職業與社交、卡丁車競速、星際擴張。
這些在 Phase 0 穩定可玩、且使用者點頭前，**不要動工**。
