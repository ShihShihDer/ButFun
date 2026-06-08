#!/usr/bin/env bash
# 發 ntfy.sh 推播到施大手機。三種訊息：
#   alert  → 要你做決定 / 卡住 / 省電暫停（高優先、會吵你）       topic: ntfy-topic-alert
#   update → 新版本好了（切片 merged / 部署成功，知會用）          topic: ntfy-topic-update
#   beat   → 心跳：還在忙（低優先、安靜，只讓你知道沒死掉）        topic: ntfy-topic-update
# 用法： notify.sh <alert|update|beat> "訊息"   （沒給型別時當 alert）
# topic 存在本機（**刻意不入 repo**，避免公開後被人偷看/灌訊息）。
set -uo pipefail
case "${1:-}" in
  alert|update|beat) ch="$1"; shift ;;
  *) ch="alert" ;;
esac
msg="${1:-ButFun 有事要你看}"
# beat 與 update 共用同一個 topic 檔
topicfile="ntfy-topic-${ch}"; [ "$ch" = beat ] && topicfile="ntfy-topic-update"
TOPIC="$(cat "$HOME/.cache/butfun-auto/${topicfile}" 2>/dev/null)"
[ -z "${TOPIC:-}" ] && { echo "[notify] 無 ${ch} topic，略過"; exit 0; }
case "$ch" in
  alert)  title="ButFun ⚠️ 要你決定"; prio="high";    tags="warning" ;;
  update) title="ButFun 🎮 新版本好了"; prio="default"; tags="video_game" ;;
  beat)   title="ButFun ⏳ 還在忙";    prio="low";     tags="hourglass" ;;
esac
if curl -fsS --max-time 10 \
     -H "Title: ${title}" -H "Priority: ${prio}" -H "Tags: ${tags}" \
     -d "$msg" "https://ntfy.sh/${TOPIC}" >/dev/null 2>&1; then
  echo "[notify:${ch}] 已推播：$msg"
else
  echo "[notify:${ch}] 推播失敗（不影響主流程）"
fi
