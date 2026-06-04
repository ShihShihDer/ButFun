# ButFun

蒸汽龐克太空歌劇療癒多人世界。瀏覽器即玩、手機電腦皆宜。
完整願景與設計請見 [`docs/GAME_DESIGN.md`](docs/GAME_DESIGN.md)。

> 狀態：**Phase 0 垂直切片**。已打通最難的一條技術主軸——
> 「網頁即時多人 + 權威伺服器」，並內建「玩家建議箱」回饋迴圈。

## 🚀 上線營運（讓 Claude Code 帶你做）

ButFun 設計成可以自走營運：你一直玩、丟反饋，Claude 自己慢慢改版、上線。
在一台要當伺服器的電腦（筆電即可）上開台，**只有一件事要你親手做**，其餘讓 cc 引導：

1. **（唯一手動起點）** 裝好 `git` 與 **Claude Code 並登入**——cc 還沒裝起來，
   沒辦法引導你裝它自己。
2. clone 本 repo，在 repo 目錄開 Claude Code，丟這一句把棒子交給它：

   > 依序照 `docs/SETUP_ON_SERVER.md` 把遊戲和 Cloudflare 通道架成常駐服務，
   > 再照 `docs/AUTONOMOUS_OPS.md` 把排程開發迴圈與維護窗部署裝好開起來。
   > 每個需要我本人操作或機敏設定的地方（裝工具、Cloudflare 接管網域、
   > `cloudflared tunnel login`、sudo）先停下來一步步帶我做。

3. 最後把應急客服會話開著：`./scripts/oncall.sh` → 進去輸入 `/rc` → 手機 Claude App 連上，
   之後人在外面也能隨時呼叫它救火。

四個角色 / 說明書：
[遊戲+通道架設](docs/SETUP_ON_SERVER.md)、
[自走營運（開發迴圈+自動上線）](docs/AUTONOMOUS_OPS.md)、
[應急客服 /rc](docs/ONCALL_RC.md)、
[開發待辦](docs/BACKLOG.md)。
全域邊界見 [`CLAUDE.md`](CLAUDE.md)。

## 目前能玩到什麼

- 進場選名字（種族起源系統已預留，MVP 先開放「地球人」）。
- 在一張地表地圖上用 **WASD / 方向鍵** 移動。
- **即時看到其他玩家**移動（權威伺服器以 15Hz 模擬並廣播狀態快照）。
- **聊天**（Enter 開啟輸入）。
- **遊戲內建議箱**（💡 給點建議）：玩家送出 → 伺服器收下並寫入 `data/suggestions.jsonl`。

## 技術

- 後端：Rust + [axum](https://github.com/tokio-rs/axum)（tokio 非同步、WebSocket）。
  權威伺服器：客戶端只送輸入意圖，伺服器模擬並廣播權威快照。
- 前端：原生 HTML5 Canvas（零相依；之後做 tilemap 再導入 Phaser）。
- 狀態目前在記憶體；持久化（Postgres）已在 `state.rs` 後留好抽換點。

## 本機執行

需要 Rust（stable）。

```bash
cargo run
# 預設 http://localhost:3000 ，可用 PORT 環境變數覆寫
```

開兩個瀏覽器分頁進場，就能看到彼此移動。

## 專案結構

```
src/
  main.rs         入口、路由、靜態檔、建議箱 HTTP API
  protocol.rs     客戶端/伺服器 WebSocket 訊息協定
  state.rs        共享狀態（玩家、廣播頻道）＋持久化抽換點
  game.rs         權威遊戲迴圈（固定 tick 整合位置、廣播快照）
  ws.rs           每連線的 WebSocket 處理
  suggestions.rs  遊戲內建議箱（玩家回饋迴圈）
web/
  index.html      進場畫面、HUD、聊天、建議箱
  game.js         連線、輸入、canvas 渲染
docs/
  GAME_DESIGN.md  遊戲設計文件（GDD）
```

## 下一步（見 GDD 分階段路線）

- 帳號身份模型（provider 無關）+ Google OAuth。
- 位置 / 背包 / 農地持久化到 Postgres。
- 第一個起源（地球人 / 繼承農莊）：耕地、種乙太作物、澆水、日夜收成。
