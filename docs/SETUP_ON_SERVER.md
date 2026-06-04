# 在伺服器機器上用 Claude Code 自動架起 ButFun

> **這份是寫給「跑在目標伺服器機器上的 Claude Code」看的操作劇本。**
> 一般人看的版本見 `docs/DEPLOY_CLOUDFLARE_TUNNEL.md`。
>
> 使用方式：在那台要當伺服器的電腦上裝好 Claude Code、在本 repo 目錄開啟，
> 對它說「照 `docs/SETUP_ON_SERVER.md` 把 ButFun 架起來並常駐」即可。

## 目標

在這台機器上：建置 ButFun → 以**系統服務**常駐執行 → 透過 **Cloudflare Tunnel**
公開到使用者的網域 → 驗證手機可連。**遊戲要獨立於 Claude Code 視窗存活**
（關掉 Claude、甚至重開機後仍在跑）。

## 一定要先問人 / 由人操作的步驟（STOP）

這些 Claude 無法代勞，遇到請**停下來請使用者完成**，完成後再繼續：

1. **網域接管**：使用者需在 Cloudflare 註冊、Add site、把網域商的 nameserver 改成
   Cloudflare 給的兩台，等狀態 **Active**。先問使用者要用的 hostname（例：`play.例子.com`）。
2. **`cloudflared tunnel login`**：此指令會印出一個授權網址。把網址**交給使用者**在瀏覽器
   點開、選網域授權；完成後本機會出現 `~/.cloudflared/cert.pem`，再繼續。
3. **privileged 操作**（`sudo`、安裝系統服務）：執行前向使用者說明並取得同意。

## 安全界線

- **絕不**把 `~/.cloudflared/` 下的 `cert.pem`、`<UUID>.json` 憑證或任何金鑰提交進 repo。
- 跟著 `CLAUDE.md` 的邊界走；privileged / 難回復的動作先問人。

---

## 步驟

### 1. 工具鏈與程式碼
- 確認在 repo 目錄；`git pull` 取最新。
- 沒有 Rust 就裝（rustup）。
- `cargo build --release` 要成功、`cargo test` 要全綠（過不了就先修，別硬上）。

### 2. 安裝 cloudflared
- 依平台安裝（`brew install cloudflared` / `apt` / 官方下載），`cloudflared --version` 驗證。

### 3. 建立通道（含上面 STOP #2 的登入）
```bash
cloudflared tunnel login                 # 印出授權網址 → 交給使用者點
cloudflared tunnel create butfun         # 產生 Tunnel UUID 與 ~/.cloudflared/<UUID>.json
cloudflared tunnel route dns butfun <使用者的hostname>
```

### 4. 寫 cloudflared 設定
- 以 `deploy/cloudflared/config.example.yml` 為範本，產生 `~/.cloudflared/config.yml`，
  填入真實的 **Tunnel UUID**、**credentials-file 路徑**、**hostname**，
  `service` 維持 `http://localhost:3000`。

### 5. 把 ButFun 伺服器裝成 systemd 服務（Linux）
- 取得 repo 絕對路徑與執行帳號。建立 `/etc/systemd/system/butfun.service`：
```ini
[Unit]
Description=ButFun game server
After=network.target

[Service]
# WorkingDirectory 必須是 repo 根目錄：伺服器要相對讀 web/ 與寫 data/
WorkingDirectory=/絕對路徑/ButFun
Environment=PORT=3000
ExecStart=/絕對路徑/ButFun/target/release/butfun-server
Restart=always
RestartSec=3
User=你的帳號

[Install]
WantedBy=multi-user.target
```
- `sudo systemctl daemon-reload && sudo systemctl enable --now butfun`
- （非 Linux 或不想用 systemd：先用 `tmux`/`screen` 跑 `PORT=3000 cargo run --release` 也行，
  但重開機不會自動起。）

### 6. 把通道也裝成服務
```bash
sudo cloudflared service install        # 會使用 ~/.cloudflared/config.yml
sudo systemctl enable --now cloudflared
```

### 7. 驗證（做完才算成功）
- 本機：`curl -fsS http://localhost:3000/healthz` 要回 OK。
- 公開：`curl -fsS https://<使用者的hostname>/healthz` 要回 OK（網域 Active 後）。
- 請使用者用手機開 `https://<使用者的hostname>`，確認能進場走動、聊天（WebSocket 通）。
- 兩個服務狀態：`systemctl status butfun cloudflared`。

### 8. 回報
- 列出：公開網址、兩個服務是否 enabled、驗證結果。
- 提醒使用者：**之後要更新遊戲版本**＝`git pull && cargo build --release && sudo systemctl restart butfun`。
