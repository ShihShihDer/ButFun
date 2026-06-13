#!/usr/bin/env bash
# 長住型 AI 居民：detached 啟動 3 個常駐 bot，survive 當前 session（24/7 長住世界生活 + 持續軟測）。
# 用 setsid nohup 脫離終端；PID 寫進 /tmp/butfun-residents/pids；各自 log 到同目錄。
#
# 安全鐵律：bot 數量上限 3（先不開 socializer——聊天洗版風險，等確認穩再說）。
# 注意：detached 啟動的 bot 不在 git 控管、會繼續跑——這是刻意的（長住）。
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BOT="$REPO/scripts/qa/ai-resident-bot.mjs"
DIR="/tmp/butfun-residents"
PIDS="$DIR/pids"
mkdir -p "$DIR"

# 已在跑就別重複啟動（避免超過 3 個上限）。
if [[ -f "$PIDS" ]] && grep -qE '[0-9]' "$PIDS" 2>/dev/null; then
  alive=0
  while read -r pid _; do [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null && alive=$((alive+1)); done < "$PIDS"
  if [[ "$alive" -gt 0 ]]; then
    echo "已有 $alive 個居民在跑（見 $PIDS）。先跑 residents-stop.sh 再重啟。"
    exit 1
  fi
fi

# 世界觀風格名字，避開現有 NPC/居民。先不開 socializer。
# 格式：名字:人格
ROSTER=(
  "露安:wanderer"
  "霍克:hunter"
  "菲歐:gatherer"
)

: > "$PIDS"
for entry in "${ROSTER[@]}"; do
  name="${entry%%:*}"; persona="${entry##*:}"
  setsid nohup env BOT_NAME="$name" BOT_PERSONA="$persona" \
    node "$BOT" > "$DIR/$name.boot.log" 2>&1 < /dev/null &
  pid=$!
  echo "$pid $name $persona" >> "$PIDS"
  echo "啟動居民 $name（$persona）pid=$pid → log $DIR/$name.log"
  sleep 0.5
done

echo
echo "3 個居民已 detached 啟動。查看：cat $DIR/*.log | tail"
echo "停止：scripts/qa/residents-stop.sh"
