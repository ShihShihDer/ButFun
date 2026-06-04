# 用 Cloudflare Tunnel 把 ButFun 上線（手機隨時能玩）

這份手冊讓你把**自己電腦上跑的 ButFun**，透過你的網域公開到網路上，
手機在外面也能連——**完全不用設定 NAT / 路由器轉發**。

## 為什麼這招適合你

- 你有**網域**、有**能上網的電腦**，但**無法設定路由器**。
- Cloudflare Tunnel 由你電腦上的 `cloudflared`**主動撥出**連到 Cloudflare，
  外部流量再從 Cloudflare 繞回來 → 不需要任何 router 設定。
- **免費**、自動 HTTPS、支援 WebSocket（ButFun 即時連線就靠這個）。
- 前端已會在 HTTPS 下自動改用 `wss://`，不用改程式。

> ⚠️ 取捨：遊戲**只有在那台電腦開著、且 `cargo run` 與 `cloudflared` 都在跑時**才上線。
> 想要關機也能玩，得改走雲端代管（見 `docs/BACKLOG.md` Phase 0-H）。

---

## 前置：把網域交給 Cloudflare 管理

1. 註冊（免費）Cloudflare 帳號：<https://dash.cloudflare.com/sign-up>
2. 在 Dashboard 點 **Add a site**，輸入你的網域。
3. Cloudflare 會給你兩台 **nameserver**；到你買網域的地方（GoDaddy / Namecheap / Gandi…）
   把 nameserver 改成 Cloudflare 給的那兩台。生效需數分鐘到數小時。
4. 確認 Dashboard 顯示網域狀態為 **Active**。
5. （預設就開）到 **Network** 確認 **WebSockets** 是開啟的。

> 不必綁信用卡、不必設付費額度。

---

## 安裝 cloudflared（在要當伺服器的那台電腦）

- macOS：`brew install cloudflared`
- Debian/Ubuntu：見 <https://pkg.cloudflare.com/>（apt 安裝）
- Windows / 其他：<https://developers.cloudflare.com/cloudflare-tunnel/downloads/>

驗證：`cloudflared --version`

---

## 建立通道

```bash
# 1) 用瀏覽器登入並授權這台機器（會跳出 Cloudflare 頁面選你的網域）
cloudflared tunnel login

# 2) 建立一個叫 butfun 的通道（會印出 Tunnel UUID 並產生憑證檔）
cloudflared tunnel create butfun

# 3) 把你的網域指向這個通道（用你要的子網域，例如 play.你的網域.com）
cloudflared tunnel route dns butfun play.你的網域.com
```

---

## 設定 ingress

把本 repo 的 `deploy/cloudflared/config.example.yml` 複製成 `~/.cloudflared/config.yml`，
填入第 2 步印出的 **Tunnel UUID**、憑證檔路徑、以及你的 **hostname**。
`service` 維持 `http://localhost:3000`（ButFun 預設埠）。

---

## 啟動（兩個程式都要開著）

開兩個終端機：

```bash
# 終端機 A：啟動 ButFun 伺服器（release 較省資源）
cd /path/to/ButFun
PORT=3000 cargo run --release

# 終端機 B：啟動通道
cloudflared tunnel run butfun
```

手機打開 `https://play.你的網域.com` → 應該就能進場走動、聊天了。🎉

---

## 讓它常駐（選用）

不想每次手動開兩個視窗，可以把它們裝成開機自動啟動的服務：

- **cloudflared**：`sudo cloudflared service install`（會用 `~/.cloudflared/config.yml`）。
- **ButFun 伺服器**：用 systemd 寫一個 unit（`ExecStart=/path/to/target/release/butfun-server`、
  `Environment=PORT=3000`），或先用 `tmux` / `screen` 跑著。

---

## 疑難排解

- **手機打不開** → 先確認電腦上 `http://localhost:3000` 自己能開；再看終端機 B 的 `cloudflared` 有沒有連上。
- **能進首頁、但進不了遊戲（WebSocket 連不上）** → 確認 Cloudflare **Network → WebSockets** 是開的。
- **`wss` 連線一下就斷** → 多半是伺服器那邊掛了，看終端機 A 的 log。
- **網域還沒生效** → nameserver 切換需要時間，等 Dashboard 顯示 Active 再試。
