#!/usr/bin/env bash
# 發 ntfy.sh 推播到施大手機（只在「需要你決策 / 省電暫停」時用，不吵）。
# 用法： notify.sh "訊息"
# topic 存在本機（**刻意不入 repo**，避免公開後被人偷看/灌訊息）：
#   ~/.cache/butfun-auto/ntfy-topic   （或設環境變數 BUTFUN_NTFY_TOPIC）
set -uo pipefail
msg="${1:-ButFun 有事要你看}"
TOPIC="${BUTFUN_NTFY_TOPIC:-$(cat "$HOME/.cache/butfun-auto/ntfy-topic" 2>/dev/null)}"
[ -z "${TOPIC:-}" ] && { echo "[notify] 無 ntfy topic，略過"; exit 0; }
if curl -fsS --max-time 10 \
     -H "Title: ButFun 自走" -H "Priority: high" -H "Tags: robot,video_game" \
     -d "$msg" "https://ntfy.sh/${TOPIC}" >/dev/null 2>&1; then
  echo "[notify] 已推播：$msg"
else
  echo "[notify] 推播失敗（不影響主流程）"
fi
