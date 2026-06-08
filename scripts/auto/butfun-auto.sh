#!/usr/bin/env bash
# ButFun 半自動營運迴圈（單一 systemd user timer 驅動，每 ~20 分一輪）。
#
# 設計：單一 worker + 一個 review 把關，用一個「交接旗標」(turn) 二選一，永不重疊：
#   - turn=work   → 跑 worker：推進下一切片 / 處理 review 的退回；開 PR 不自 merge。
#   - turn=review → 跑 reviewer：審 PR；綠+安全就 merge、有問題退回、要人決策就升級。
#   - turn=human  → 升級給人，閒置等人；人處理完 `echo work > ~/.cache/butfun-auto/turn` 恢復。
#   - turn=done   → 切片全做完，閒置。
#
# 安全：worker/reviewer 都用普通帳號跑、碰不到 sudo/上線；**部署永不自動**
#   （prod 上線是維護窗 deploy.sh + 人）。merge 後 staging 會自動更新供你玩。
#
# 一鍵停：  systemctl --user disable --now butfun-auto.timer
# 暫停一下：touch ~/.cache/butfun-auto/paused   （刪掉即恢復）
# 看它做了什麼：journalctl --user -u butfun-auto -n 100 ；GitHub PR ；butfun-coord/for_human.md
set -euo pipefail

REPO="${BUTFUN_REPO:-/home/shihshih/ButFun}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STATE="$HOME/.cache/butfun-auto"; mkdir -p "$STATE"
TURN_FILE="$STATE/turn"
PAUSE="$STATE/paused"

cd "$REPO"

[ -f "$PAUSE" ] && { echo "[auto] paused（$PAUSE 存在），本輪不動"; exit 0; }

# 互斥：同一時間只准一輪（worker 或 reviewer），搶不到就讓位
exec 9>/tmp/butfun-auto.lock
if ! flock -n 9; then echo "[auto] 上一輪還在跑，本輪讓位"; exit 0; fi

git fetch --quiet origin main || true

turn="$(cat "$TURN_FILE" 2>/dev/null || echo work)"
echo "[auto] turn=$turn"

run_claude() {  # $1 = prompt 檔
  cd "$REPO"
  exec claude -p --dangerously-skip-permissions "$(cat "$1")"
}

case "$turn" in
  review) run_claude "$HERE/reviewer.prompt" ;;
  work)   run_claude "$HERE/worker.prompt" ;;
  human)  echo "[auto] turn=human：等人處理 butfun-coord/for_human.md，本輪閒置"; exit 0 ;;
  done)   echo "[auto] turn=done：切片全做完，本輪閒置"; exit 0 ;;
  *)      echo "[auto] 未知 turn=$turn，當 work 處理"; run_claude "$HERE/worker.prompt" ;;
esac
