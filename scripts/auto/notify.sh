#!/usr/bin/env bash
# 發 ntfy.sh 推播到施大手機。兩個頻道（兩個 topic）：
#   alert  → 問題/需要你決策/省電暫停（高優先、要你注意）   topic 檔: ntfy-topic-alert
#   update → 版本更新/切片 merged/部署成功（知會用）          topic 檔: ntfy-topic-update
# 用法： notify.sh <alert|update> "訊息"   （沒給頻道時當 alert，向後相容）
# topic 存在本機（**刻意不入 repo**，避免公開後被人偷看/灌訊息）。
set -uo pipefail
case "${1:-}" in
  alert|update) ch="$1"; shift ;;
  *) ch="alert" ;;
esac
msg="${1:-ButFun 有事要你看}"
TOPIC="$(cat "$HOME/.cache/butfun-auto/ntfy-topic-${ch}" 2>/dev/null)"
[ -z "${TOPIC:-}" ] && { echo "[notify] 無 ${ch} topic，略過"; exit 0; }
if [ "$ch" = alert ]; then
  title="ButFun ⚠️ 要你決策"; prio="high"; tags="warning,robot"
else
  title="ButFun 🚀 版本更新"; prio="default"; tags="rocket,video_game"
fi
if curl -fsS --max-time 10 \
     -H "Title: ${title}" -H "Priority: ${prio}" -H "Tags: ${tags}" \
     -d "$msg" "https://ntfy.sh/${TOPIC}" >/dev/null 2>&1; then
  echo "[notify:${ch}] 已推播：$msg"
else
  echo "[notify:${ch}] 推播失敗（不影響主流程）"
fi
