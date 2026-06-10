#!/usr/bin/env bash
# 由「人」執行：啟用 ButFun 半自動營運迴圈的 systemd user timer。
# 用法（在 Claude Code 對話框，以你的身分執行）：  ! bash /home/shihshih/ButFun/scripts/auto/enable.sh
# 一鍵停：  systemctl --user disable --now butfun-auto.timer
set -euo pipefail

echo "[enable] 收掉舊的互動 worker（避免兩個 worker 搶同一棵樹）…"
tmux kill-session -t butfun-dev 2>/dev/null && echo "  butfun-dev 已關" || echo "  （沒有 butfun-dev，略過）"

echo "[enable] 建 systemd user 單元…"
mkdir -p ~/.config/systemd/user ~/.cache/butfun-auto

cat > ~/.config/systemd/user/butfun-auto.service <<'EOF'
[Unit]
Description=ButFun 半自動營運迴圈（Gemini worker + Claude 把關，一輪）
After=network-online.target

[Service]
Type=oneshot
WorkingDirectory=/home/shihshih/ButFun
Environment=BUTFUN_REPO=/home/shihshih/ButFun
Environment=PATH=/home/shihshih/.local/bin:/home/shihshih/.cargo/bin:/home/shihshih/.nvm/versions/node/v22.16.0/bin:/usr/local/bin:/usr/bin:/bin
ExecStart=/home/shihshih/ButFun/scripts/auto/butfun-auto.sh
TimeoutStartSec=1500
EOF

cat > ~/.config/systemd/user/butfun-auto.timer <<'EOF'
[Unit]
Description=ButFun 半自動迴圈心跳（每 2 分；實際節奏由 butfun-auto.sh 依週額度自適應）

[Timer]
OnBootSec=3min
OnUnitActiveSec=2min
Persistent=true

[Install]
WantedBy=timers.target
EOF

echo "[enable] 讓 user 服務在你登出時也能跑（linger，可能要 sudo 密碼，失敗不影響當前 session）…"
sudo loginctl enable-linger shihshih || echo "  （linger 設定略過；只要你還有登入/tmux 在，timer 照跑）"

echo "[enable] 起手旗標＝work（worker 先補 PR #39 的 spawn_at），載入並啟用 timer…"
echo work > ~/.cache/butfun-auto/turn
systemctl --user daemon-reload
systemctl --user enable --now butfun-auto.timer
systemctl --user start --no-block butfun-auto.service   # 背景立刻跑第一輪

echo
echo "[enable] ✅ 已啟用。"
echo "  看它做什麼： journalctl --user -u butfun-auto -f"
echo "  給你看的窗口： /home/shihshih/butfun-coord/for_human.md"
echo "  一鍵停：       systemctl --user disable --now butfun-auto.timer"
