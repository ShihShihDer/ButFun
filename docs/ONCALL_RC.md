# 應急客服 Claude（/rc 遠端控制）

讓你人在外面玩、用手機就能隨時呼叫一個跑在筆電上的 Claude 來「救火」。

## 它是什麼

`/rc`（= `/remote-control`，Claude Code 官方指令）把**筆電上的 Claude 會話**
橋接到 **Claude 手機 App / claude.ai**。你在哪都能對它下指令、看進度、批准動作。

- **只走 outbound HTTPS、不開 inbound port** → 不用設 NAT/路由器，跟 Cloudflare Tunnel 同理。
- **程式都在你筆電上跑**，不是雲端 → 它能直接看你的線上伺服器、改檔、重啟、回滾。

## 三個 Claude 各司其職（建立正確心智模型）

| 角色 | 怎麼跑 | 做什麼 | 互動性 |
|---|---|---|---|
| **排程開發迴圈** | systemd timer（headless） | 自己慢慢做大世界、修後端 error，綠了才 push | 無人、自走 |
| **應急客服（本檔）** | 筆電上 tmux 會話 + `/rc` | 你隨時從手機呼叫處理緊急狀況 | 你直接對話 |
| **遊戲服務本身** | systemd `butfun` + `cloudflared` | 讓玩家 24h 能玩 | 跟 Claude 無關 |

> 遊戲的命由服務保著；開發由排程迴圈推進；**救火靠這個 /rc 會話**。

## 開起來

1. 確認筆電已裝 Claude Code 並登入、也裝了 `tmux`。
2. 在 repo 目錄跑：
   ```bash
   ./scripts/oncall.sh
   ```
   （已存在就接上、不存在就新建一個叫 `butfun-oncall` 的常駐 tmux 會話。）
3. 在會話裡輸入 `/rc` 開啟 Remote Control，依提示在手機 Claude App 連上。
4. 之後關掉終端機都沒關係（tmux 保著）；**重開機後再跑一次** `./scripts/oncall.sh`。

## 跟開發迴圈的協調（避免互踩）

你用 /rc 做**即時緊急修復**時，最好先暫停排程開發迴圈，免得兩邊同時改、commit 打架：
```bash
sudo systemctl disable --now butfun-devloop.timer   # 暫停自走開發
# …你救完火…
sudo systemctl enable --now butfun-devloop.timer    # 恢復
```
（遊戲服務 `butfun` 不受影響，玩家照玩。）

## 救火常見動作（給應急客服 Claude 的提示）

- 看線上錯誤：`journalctl -u butfun --since "10 min ago" --no-pager`
- 緊急修復後**立即**上線（不等維護窗）：改好 → `cargo build --release && cargo test` →
  綠了 → `sudo systemctl restart butfun` → `curl -fsS localhost:3000/healthz` 確認。
- 出包回滾：把 `target/release/butfun-server.prev` 換回 `butfun-server` 再 `restart`
  （見 `scripts/deploy.sh` 的回滾邏輯）。
- **底線不變**：不破壞玩家資料、不提交密鑰、修不好就照實說並回滾，別把壞版本留在線上。

## 安全

- `/rc` 是加密 outbound 橋接、不開 port；但這個會話**能對你的線上機器做任何事**，
  所以只在你信任的裝置、用你自己的帳號連。不用時可在會話裡 `/rc` 關掉或直接結束會話。
