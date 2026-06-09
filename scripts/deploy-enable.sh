#!/usr/bin/env bash
# 由「人」執行：啟用 ButFun prod 自動上線（每 15 分，測試綠 + healthz + WS 冒煙閘才換版，失敗自動回滾）。
#
# 用法（在 Claude Code 對話框，以你身分跑——只用 systemctl --user，不會被 sudo 密碼卡）：
#   ! bash /home/shihshih/ButFun/scripts/deploy-enable.sh
#
# 一鍵停：    systemctl --user disable --now butfun-deploy.timer
# 立刻上一次：systemctl --user start butfun-deploy.service
set -euo pipefail

REPO=/home/shihshih/ButFun
SRC="$REPO/deploy/systemd"
DST="$HOME/.config/systemd/user"
mkdir -p "$DST"

# 上線成功推播 → 你的「版本更新」topic（秘密不入 repo，從本機 cache 讀）。
TOPIC="$(cat "$HOME/.cache/butfun-auto/ntfy-topic-update" 2>/dev/null || echo '')"
if [ -n "$TOPIC" ]; then
  echo "[deploy-enable] 上線推播 → topic $TOPIC"
else
  echo "[deploy-enable] （沒找到更新 topic，略過推播；之後設好再重跑本腳本即可）"
fi

echo "[deploy-enable] 安裝 prod 部署 user 單元（注入 ntfy topic）…"
sed "s|@NTFY_TOPIC@|${TOPIC}|" "$SRC/butfun-deploy.service" > "$DST/butfun-deploy.service"
cp -f "$SRC/butfun-deploy.timer" "$DST/butfun-deploy.timer"

echo "[deploy-enable] 啟用 timer（linger 已開，登出也會跑）…"
systemctl --user daemon-reload
systemctl --user enable --now butfun-deploy.timer

echo
echo "[deploy-enable] ✅ prod 自動上線已啟用：啟用後約 2 分上第一次，之後每 15 分檢查一次（沒新東西自動跳過）。"
echo "  看上線紀錄：  journalctl --user -u butfun-deploy -f"
echo "  下次觸發：    systemctl --user list-timers butfun-deploy.timer"
echo "  立刻上一次：  systemctl --user start butfun-deploy.service"
echo "  一鍵停：      systemctl --user disable --now butfun-deploy.timer"
echo
echo "  ※ 若第一次上線卡在「重啟服務 $REPO」失敗，代表 prod 的 systemctl restart 還沒放行 passwordless，"
echo "    需在 /etc/sudoers.d/ 加：shihshih ALL=(ALL) NOPASSWD: /usr/bin/systemctl restart butfun"
echo "    deploy.sh 在那步之前若失敗會安全中止、不動到 prod（測試/備份/build 都在 restart 之前）。"
