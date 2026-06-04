#!/usr/bin/env bash
# ButFun 開發迴圈：每隔一小段時間（由 butfun-devloop.timer 觸發）叫起一個
# 無互動的 Claude Code，做「一輪」營運開發——讀後端 error log 與玩家建議、
# 修問題或推進 BACKLOG、build+test 綠了才 commit/push。
#
# 重點：
#   - 用普通帳號跑，**不碰 sudo、不碰上線**。它只負責把程式做好、推上 git。
#   - 真正換版上線由 scripts/deploy.sh 在維護窗執行（與本腳本無關）。
#   - 沒事可做就快速結束，省 token（判斷邏輯寫在 docs/AUTONOMOUS_OPS.md，
#     由 Claude 自己依該劇本決定要不要動工）。
#
# 前置：這台機器要先把 Claude Code 安裝好並完成登入／設好 API 金鑰。
# 環境變數：
#   BUTFUN_REPO   repo 根目錄（預設 /opt/butfun）

set -euo pipefail

REPO="${BUTFUN_REPO:-/opt/butfun}"
cd "$REPO"

# 取最新程式，但**只在乾淨且能 fast-forward 時才同步**：
# 若工作目錄有未 commit 改動，或本地 main 已超前 origin（你正在 oncall 手改），
# 就跳過這一輪，**絕不**用 reset --hard 吃掉你的手改。
git fetch --quiet origin main || true
if [ -n "$(git status --porcelain)" ]; then
  echo "[devloop] 工作目錄有未 commit 改動，跳過這一輪（等你 commit/push 再來）"
  exit 0
fi
git checkout --quiet main || true
# 只接受 fast-forward；要 rebase / merge 的時候停手，留給人處理。
if ! git merge --ff-only --quiet origin/main; then
  echo "[devloop] 本地 main 與 origin/main 分歧（多半是你手改後還沒 push），跳過這一輪"
  exit 0
fi

# 把這一輪的指示交給 Claude Code（無互動模式）。
# 它會照 docs/AUTONOMOUS_OPS.md 的「每一輪做什麼」自走一個小增量。
PROMPT='你是 ButFun 的常駐營運+開發團隊，現在被排程叫起來做一輪。\
嚴格照 docs/AUTONOMOUS_OPS.md 的「每一輪做什麼」與安全護欄執行：\
先判斷有沒有值得做的事（後端 error、玩家建議、BACKLOG 下一項），\
沒有就什麼都別改、直接結束；有就只做一個小而完整的增量，\
build+test 全綠才 commit/push 到 main，風險大的改動只開 PR 不自己 merge。'

# 注意：headless 自走需要放行常用指令，請依 docs/AUTONOMOUS_OPS.md 設好
# .claude/settings.json 的允許清單，不要全域略過權限檢查。
exec claude -p "$PROMPT"
