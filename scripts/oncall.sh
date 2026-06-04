#!/usr/bin/env bash
# 啟動／接上常駐的 on-call Claude Code 會話（應急客服）。
#
# - 跑在 repo 目錄、包在 tmux 裡 → 關掉終端機也不會斷。
# - 進去之後輸入 /rc（= /remote-control）開啟 Remote Control，
#   就能從手機 Claude App / claude.ai 隨時呼叫這個會話來救火。
# - /rc 只走 outbound HTTPS、不開 port，免設 NAT。
#
# 用法：./scripts/oncall.sh   （已存在就接上、不存在就新建）
# 重開機後再跑一次即可。
set -euo pipefail

REPO="${BUTFUN_REPO:-/opt/butfun}"
SESSION="${BUTFUN_ONCALL_TMUX:-butfun-oncall}"

if ! command -v tmux >/dev/null 2>&1; then
  echo "需要 tmux：請先安裝（例：sudo apt install tmux / brew install tmux）。" >&2
  exit 1
fi

echo "[on-call] 會話：$SESSION（新建後請在裡面輸入 /rc，再用手機 App 連上）"
# -A：存在就接上、不存在就以 REPO 為工作目錄新建並啟動 claude。
exec tmux new-session -A -s "$SESSION" -c "$REPO" claude
