#!/usr/bin/env bash
# ButFun AI 開發團隊 — 單一 lane 的 worker。
#
# 每個 lane 跑在**自己的 git worktree、自己的分支**(auto/<lane>),彼此不共用工作樹
# → 真並行、不互相改壞(取代先前共用 checkout + flock 互斥的做法)。
# Worker **只 commit 到自己的分支、不碰 main**;由 integrator 把綠燈分支合回 main。
#
# 用法:agent.sh <lane>   lane ∈ backend|frontend|feature|feedback
# 環境:BUTFUN_WORKTREES_DIR(預設 /home/shihshih),worktree 在 $DIR/bf-<lane>
set -euo pipefail

LANE="${1:?用法: agent.sh <lane>}"
DIR="${BUTFUN_WORKTREES_DIR:-/home/shihshih}/bf-${LANE}"
BRANCH="auto/${LANE}"
cd "$DIR"

# 取最新 main,rebase 到自己分支(拿到別人已合進 main 的成果)。工作樹要乾淨才動;
# 上輪沒收完的就跳過,別把半成品弄亂。rebase 衝突就放棄這輪、留給人/下輪。
git fetch --quiet origin main || true
if [ -n "$(git status --porcelain)" ]; then
  echo "[$LANE] 工作樹有未 commit 改動,跳過本輪"
  exit 0
fi
git checkout --quiet "$BRANCH" 2>/dev/null || git checkout --quiet -b "$BRANCH"
if ! git rebase --quiet origin/main; then
  git rebase --abort 2>/dev/null || true
  echo "[$LANE] rebase 與 main 衝突,跳過本輪(等 integrator/人處理)"
  exit 0
fi

# 各 lane 的焦點(界線清楚 → 合併衝突最少)。
case "$LANE" in
  backend)
    FOCUS='只做後端 Rust 系統(src/*.rs、migrations/):Phase 0-E 持久化、per-player 地塊擁有、伺服器邏輯與效能、測試。**不要動 web/**(那是前端 lane 的事)。' ;;
  frontend)
    FOCUS='只做前端(web/*):自適應 UI(直式/橫式、手機/平板)、可收合面板(說明/公告/數值)、sprite 渲染與操作手感、公告顯示。**不要改 src/*.rs 的伺服器邏輯**。' ;;
  feature)
    FOCUS='做一個玩法功能的垂直切片,依 BACKLOG Phase 1 順序(採集→背包→合成→…→戰鬥→交易)。後端狀態與前端呈現都可碰,但一次只推進一個功能、小步走,盡量集中在新檔以減少跟 backend/frontend lane 撞同一個檔。' ;;
  feedback)
    FOCUS='只讀 data/suggestions.jsonl 的玩家建議(時間戳新於上次處理),挑一個小而明確的做(小視覺/UX/提示)。架構級的大改交給其他 lane。' ;;
  *)
    echo "未知 lane: $LANE"; exit 1 ;;
esac

PROMPT="你是 ButFun AI 開發團隊的 [$LANE] 成員,在自己的 git worktree($DIR)與分支 $BRANCH 上做一輪。
${FOCUS}
**開工前先讀 docs/PLAN.md**(Planner 迴圈排好的「當前主攻方向」):讓你這輪挑的事盡量對齊它指向的優先序,別去做它列在「暫緩」的事。PLAN.md 是導航,細項仍以 docs/BACKLOG.md 為準;與你 lane 的 FOCUS 邊界衝突時,以 FOCUS 邊界優先。
嚴格遵守 docs/AUTONOMOUS_OPS.md 的「每一輪做什麼」與 CLAUDE.md 的邊界與品質閘門。
先判斷你 lane 內有沒有值得做的事;沒有就什麼都別改、直接結束(省 token)。
有就**只做一個小而完整的增量**,cargo build + cargo test 全綠才 commit 到 $BRANCH
(commit 訊息開頭標 [$LANE])。**絕對不要自己合併到 main、不要 push main、不要碰其他
lane 的 worktree**——integrator 會把綠燈分支合回 main。風險大/架構級/動玩家資料的只在
分支上做、必要時開 PR 描述,讓人或 integrator 決定。
鐵律:遊戲規則只在伺服器(權威),不寫進 2D 繪製碼——為將來 WebXR(AR/VR)renderer
當另一個客戶端連同一後端留路。"

exec claude -p --dangerously-skip-permissions "$PROMPT"
