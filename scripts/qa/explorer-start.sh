#!/usr/bin/env bash
# 瀏覽器版長住 AI 探索者：detached 啟動，survive 當前 session（長住世界 + 持續抓前端錯誤）。
# 用 setsid nohup 脫離終端；PID 寫進 /tmp/butfun-residents/explorer.pid；log 到 explorer.log。
#
# 安全鐵律：① 只連 localhost:3000（腳本本身強制）。② 單一實例（已在跑就拒絕重啟）。
# 注意：detached 啟動的探索者不在 git 控管、會繼續跑——這是刻意的（長住）。
#
# 用法：scripts/qa/explorer-start.sh            # 預設跑 RUN_MIN（60）分鐘
#       RUN_MIN=120 scripts/qa/explorer-start.sh
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BOT="$REPO/scripts/qa/ai-explorer-live.mjs"
DIR="/tmp/butfun-residents"
PIDFILE="$DIR/explorer.pid"
LOG="$DIR/explorer.log"
# puppeteer-core 裝在 /tmp/butfun-browptest（範本所在）；讓 node 找得到。
NODE_MODULES="${EXPLORER_NODE_MODULES:-/tmp/butfun-browptest/node_modules}"
mkdir -p "$DIR"

# 安全鐵律 ②：單一實例。已在跑就拒絕（避免雙開洪水）。
if [[ -f "$PIDFILE" ]]; then
  pid="$(cat "$PIDFILE" 2>/dev/null || true)"
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    echo "探索者已在跑（pid=$pid）。先跑 explorer-stop.sh 再重啟。"
    exit 1
  fi
fi

if [[ ! -d "$NODE_MODULES/puppeteer-core" ]]; then
  echo "找不到 puppeteer-core（$NODE_MODULES）。設 EXPLORER_NODE_MODULES 指向有裝的位置。"
  exit 1
fi

setsid nohup env EXPLORER_NODE_MODULES="$NODE_MODULES" RUN_MIN="${RUN_MIN:-60}" \
  node "$BOT" >> "$DIR/explorer.boot.log" 2>&1 < /dev/null &
pid=$!
echo "$pid" > "$PIDFILE"
echo "探索者已 detached 啟動 pid=$pid（RUN_MIN=${RUN_MIN:-60} 分鐘）"
echo "  log：$LOG"
echo "  查看心跳：tail -f $LOG"
echo "  停止：scripts/qa/explorer-stop.sh"
