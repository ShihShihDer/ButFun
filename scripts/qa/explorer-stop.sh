#!/usr/bin/env bash
# 停止瀏覽器版長住 AI 探索者：讀 pid 送 SIGTERM（bot 會乾淨關 browser 後 exit）。
set -euo pipefail

DIR="/tmp/butfun-residents"
PIDFILE="$DIR/explorer.pid"

if [[ ! -f "$PIDFILE" ]]; then
  echo "沒有 pid 檔（$PIDFILE）——探索者似乎沒在跑。"
  exit 0
fi

pid="$(cat "$PIDFILE" 2>/dev/null || true)"
if [[ -z "$pid" ]]; then
  echo "pid 檔是空的，清掉。"
  rm -f "$PIDFILE"
  exit 0
fi

if kill -0 "$pid" 2>/dev/null; then
  echo "送 SIGTERM 給探索者 pid=$pid（乾淨關 browser 中）…"
  kill -TERM "$pid" 2>/dev/null || true
  # 等最多 ~10 秒讓它收尾。
  for _ in $(seq 1 20); do
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.5
  done
  if kill -0 "$pid" 2>/dev/null; then
    echo "還沒退，強制 SIGKILL。"
    kill -KILL "$pid" 2>/dev/null || true
  fi
  echo "已停止。"
else
  echo "pid=$pid 已不在跑。"
fi
rm -f "$PIDFILE"
