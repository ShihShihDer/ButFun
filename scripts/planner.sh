#!/usr/bin/env bash
# ButFun AI 開發團隊 — Planner(排序迴圈,AI 原生分工的「Plan」loop)。
#
# 不是人類職稱的 PM,而是一個「決定下一個最該做什麼才好玩」的迴圈:讀 GDD + 玩家建議 +
# BACKLOG + 最近進度,把優先序收斂成一份 docs/PLAN.md,讓四條工程線開工前先看它。
#
# 安全:**只改 docs/(文件),絕不碰程式/web/、絕不 merge main**。壞不了 build/prod——
# 最差只是一份排序不理想的文件,可逆。commit 到自己的分支 auto/planner,由 integrator 合。
#
# 用法:planner.sh
set -euo pipefail

DIR="${BUTFUN_WORKTREES_DIR:-/home/shihshih}/bf-planner"
BRANCH="auto/planner"
cd "$DIR"

git fetch --quiet origin main || true
if [ -n "$(git status --porcelain)" ]; then
  echo "[planner] 工作樹有未 commit 改動,跳過本輪"
  exit 0
fi
git checkout --quiet "$BRANCH" 2>/dev/null || git checkout --quiet -b "$BRANCH"
if ! git rebase --quiet origin/main; then
  git rebase --abort 2>/dev/null || true
  echo "[planner] rebase 與 main 衝突,跳過本輪"
  exit 0
fi

PROMPT='你是 ButFun AI 開發團隊的【Planner】——不是人類 PM,而是「決定下一個最該做什麼才好玩」的規劃迴圈。

你的唯一產出是維護 docs/PLAN.md 這一份檔(若不存在就建立)。**嚴禁改任何程式碼、web/、src/、Cargo、migrations——只能改 docs/。嚴禁 merge / push main。**

這一輪請做:
1. 讀 docs/GAME_DESIGN.md(願景/北極星)、docs/BACKLOG.md(現有清單與已完成)、docs/PLAN.md(你上次的規劃)、data/suggestions.jsonl 的最後 ~30 行(玩家真實回饋)、以及 `git log --oneline -30 origin/main`(最近實際做了什麼)。
2. 判斷「現在這遊戲最缺的『好玩』是什麼」。鐵律:守 GDD 北極星與紀律——**一次只主攻一個垂直切片、不跳級、Phase 0 沒穩前不碰飛船/多星球**;經濟迴圈(乙太有產出也要有去處)、玩家擁有感、上手第一分鐘的鉤子,優先於無止盡的打磨。
3. 把 docs/PLAN.md 改寫成精簡的「當前主攻」指南,格式:
   - ## 🎯 現在主攻(一句話 + 為什麼這個最能提升可玩性)
   - ## 接下來 1-3 個切片(由上往下、每個附「驗收 = 玩家能做到什麼」)
   - ## 給各線的具體指示(backend / frontend / feature / feedback 各一兩句:這陣子請把力氣放哪、別放哪)
   - ## 暫緩(列出「現在先別做」的事,例如過度打磨、行銷、跳級功能)
   保持精簡(整份 < 120 行),這是給其他 AI 開工前 30 秒讀的導航,不是長篇。
4. 若這輪判斷與上次 PLAN.md 沒有實質差異,就**什麼都別改、直接結束**(省 token、別製造無謂 commit)。

有實質更新才 `git add docs/PLAN.md && git commit`(訊息開頭標 [planner],一句話說這輪調整了什麼方向)。只 commit 到 '"$BRANCH"'。完成就結束。'

exec claude -p --dangerously-skip-permissions "$PROMPT"
