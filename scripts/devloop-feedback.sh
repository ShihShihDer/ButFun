#!/usr/bin/env bash
# 第二條 devloop:**只**處理 data/suggestions.jsonl 進來的玩家建議。主 devloop 走
# BACKLOG 大方向,這條專盯玩家反饋。兩條跑同一 repo,各自挑不同來源減少打架。
set -euo pipefail

REPO="${BUTFUN_REPO:-/opt/butfun}"
cd "$REPO"

git fetch --quiet origin main || true
if [ -n "$(git status --porcelain)" ]; then exit 0; fi
git checkout --quiet main || true
git merge --ff-only --quiet origin/main || exit 0

PROMPT='你是 ButFun 玩家回饋專員,排程叫起做一輪。\
**只**讀 data/suggestions.jsonl 找新進的玩家建議(時間戳新於最近一次處理的);\
找一個小而明確的建議去做(小視覺/UX 修、新提示、新內容變體都行),\
build+test 全綠才 commit/push 到 main(commit 訊息標明「玩家建議」+\
建議摘要)。BACKLOG 主流程交給另一條 devloop 推,你不要去碰 0-E/0-G\
那些大改架構的事。沒新建議就直接結束。風險大的只開 PR 不自 merge。'

exec claude -p --dangerously-skip-permissions "$PROMPT"
