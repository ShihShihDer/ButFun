#!/usr/bin/env bash
# 收掉 residents-start.sh detached 啟動的長住居民：讀 pids、逐一 SIGTERM 讓它們乾淨關閉 WS、清 pids。
set -uo pipefail

DIR="/tmp/butfun-residents"
PIDS="$DIR/pids"

if [[ ! -f "$PIDS" ]]; then
  echo "找不到 $PIDS，沒有在跑的居民。"
  exit 0
fi

stopped=0
while read -r pid name persona; do
  [[ -z "${pid:-}" ]] && continue
  if kill -0 "$pid" 2>/dev/null; then
    kill -TERM "$pid" 2>/dev/null && echo "SIGTERM → $name（$persona）pid=$pid" && stopped=$((stopped+1))
  else
    echo "已不在跑：$name pid=$pid（略過）"
  fi
done < "$PIDS"

# 留 ~1.5s 讓 bot 收尾（送 WS close frame）後清掉 pids 檔。
sleep 1.5
rm -f "$PIDS"
echo "已送出 $stopped 個 SIGTERM，清掉 $PIDS。"
