# ButFun 自走營運 —— 讓 Claude 當常駐的「營運+開發團隊」

目標：你一直玩、邊玩邊丟反饋；Claude 自己慢慢把世界做大、修後端錯誤、改版，
並在每日維護窗自動換版上線。這份文件是整套設計與安裝說明，
**也是排程叫起來的 Claude「每一輪要做什麼」的依據**（`scripts/devloop.sh` 會指它來看這裡）。

## 兩種「活著」要分開（最重要）

| 層 | 誰負責 | 怎麼活著 |
|---|---|---|
| **遊戲線上**（玩家隨時能玩） | systemd 服務 `butfun` + `cloudflared` | 開機自動起、崩了自動重啟。**跟 Claude 無關**，Claude 出錯也不影響玩家。 |
| **持續開發**（世界長大、修錯） | 排程叫起的 headless Claude（開發迴圈） | systemd timer 每隔一小段時間叫起一輪；重開機自動續、成本可控。 |

**權責切分（讓「全自動」也安全）：**
- 開發迴圈的 Claude 用**普通帳號**跑，只做「改程式 → build → test → 綠了 commit/push」。
  **它碰不到 sudo、碰不到上線。**
- 換版上線由確定性腳本 `scripts/deploy.sh` 在維護窗執行：**測試沒全綠不換、健康檢查失敗自動回滾**。

## 你選的運作方式

- **開發/修錯：短排程**（預設每 20 分鐘一輪，`butfun-devloop.timer`）——能很快抓到並修後端 error。
- **上線：全自動，但集中在每日維護窗**（預設 03:00–05:00，`butfun-deploy.timer`）——玩家白天不被打斷。

---

## 每一輪做什麼（排程的 Claude 照這個走）

1. **先判斷有沒有值得做的事**（沒有就什麼都別改、直接結束，省 token）：
   - **後端錯誤**：`journalctl -u butfun --since "30 min ago" --no-pager`，找 `ERROR` / `panic` / 異常。
   - **玩家建議**：讀 `data/suggestions.jsonl` 的新項目。
   - **既有進度**：`docs/BACKLOG.md`「進行中／下一步」最上面那個未完成項。
   - 三者都沒有 → **不動任何檔案，直接結束。**
2. **選一件「小而完整」的事**，優先序：**線上 error 修復 > 明確的小建議 > BACKLOG 下一項**。
3. **判斷風險等級：**
   - 小且安全（bug 修正、小調整、加測試、明確的小建議）→ 自己做。
   - **大／架構級／會動到玩家資料／需求有歧義** → **不要自己 commit 到 main**，
     改開一個 **draft PR** 把想法留給人，然後結束。
4. **實作**（跟現有風格走，純邏輯抽成可測函式），跑 `cargo build` + `cargo test`。
   - 沒全綠就修；修不動就**還原這次改動**、開個 PR/issue 記錄，**絕不 push 壞的**。
5. **綠了** → `commit`（繁中、聚焦）→ `push` 到 `main`；在 `docs/BACKLOG.md` 打勾並補一行「做了什麼」。
6. **結束**。**不要自己重啟服務上線**——那是維護窗 `deploy.sh` 的事。

### 安全護欄（每一輪都成立）
- 測試沒全綠不 push。
- **絕不**破壞性刪玩家資料；不提交 `data/`、不提交任何密鑰。
- 風險大 → 開 PR、不自己 merge。
- 一輪只做一個增量；做完就停。
- 開發迴圈**永不** `systemctl` / 動正式線上。

---

## 安裝（建議讓伺服器上的 Claude Code 照 `docs/SETUP_ON_SERVER.md` 先把遊戲+通道服務裝好，再做這裡）

設 repo 在 `/opt/butfun`、執行帳號 `butfun`（要改路徑/帳號就同步改 unit 檔）：

1. **腳本可執行**：`chmod +x scripts/deploy.sh scripts/devloop.sh`
2. **安裝服務檔**：把 `deploy/systemd/` 下四個檔複製到 `/etc/systemd/system/`，
   改好 `WorkingDirectory` / `User` / 路徑。
3. **窄授權上線**：給 `butfun` 帳號「只能重啟這個服務」的免密碼 sudo——
   `/etc/sudoers.d/butfun`（用 `visudo` 編輯）：
   ```
   butfun ALL=(root) NOPASSWD: /usr/bin/systemctl restart butfun
   ```
4. **Claude Code 認證**：這台機器要先裝好 Claude Code 並登入，或在
   `butfun-devloop.service` 用 `EnvironmentFile` 帶入 `ANTHROPIC_API_KEY`
   （**別把金鑰寫進 repo 或 unit 檔**）。
5. **放行常用指令**：用 `/fewer-permission-prompts` 或手動在 `.claude/settings.json`
   設允許清單（cargo、git 等），讓 headless 能動工——**不要全域略過權限檢查**。
6. **開排程**：
   ```bash
   sudo systemctl daemon-reload
   sudo systemctl enable --now butfun-devloop.timer butfun-deploy.timer
   ```

---

## 你怎麼掌控它

- **看它做了什麼**：`git log --oneline`、GitHub 上的 PR、`journalctl -u butfun-devloop`。
- **暫停自走開發**（遊戲不受影響）：`sudo systemctl disable --now butfun-devloop.timer`
- **暫停自動上線**：`sudo systemctl disable --now butfun-deploy.timer`
- **調節奏**：改 `butfun-devloop.timer` 的 `OnUnitActiveSec`、`butfun-deploy.timer` 的 `OnCalendar`。
- **你直接給反饋**：遊戲內 💡 建議箱（進 `data/suggestions.jsonl`），或隨時自己開 Claude Code 跟它講。

## 成本提醒

短排程 + headless 會**持續用 token**。上面「沒事就早退」的設計能把多數輪次壓到很便宜
（看一下沒事就結束），但仍建議你觀察前幾天的用量，再決定要不要把間隔拉長。
