#!/usr/bin/env bash
# ButFun AI 開發團隊 — 整合者(integrator)。
#
# 把各 lane 的 auto/<lane> 分支「綠燈」的成果合回 main。唯一會 push main 的角色,
# 所以 main 的 push 不會多方競爭。流程(對每個 lane):
#   1. 用一個 scratch 分支 auto/integrate 對齊最新 origin/main
#   2. 試著 merge auto/<lane>
#   3. 合得進來(無衝突)→ build + test;全綠 → push 到 main;紅 → 還原、留待下輪
#   4. 衝突 → 放棄這個 lane、開/更新 PR 讓人或該 lane rebase 後再來
#
# 跑在自己的 worktree(bf-integrator)。綠燈自動合(使用者選的策略)。
set -euo pipefail

DIR="${BUTFUN_WORKTREES_DIR:-/home/shihshih}/bf-integrator"
LANES=(${BUTFUN_LANES:-backend frontend feature feedback})
cd "$DIR"

git fetch --quiet origin main
git checkout --quiet -B auto/integrate origin/main
git reset --hard --quiet origin/main

for lane in "${LANES[@]}"; do
  BR="auto/${lane}"
  git show-ref --verify --quiet "refs/heads/${BR}" || continue   # 分支還沒建就跳過
  # 分支沒有領先 main 就跳過(沒有新東西)
  if git merge-base --is-ancestor "$BR" HEAD; then continue; fi

  echo "[integrator] 嘗試合併 $BR …"
  if git merge --no-edit --quiet "$BR"; then
    if cargo build --release -q 2>/dev/null && cargo test --release -q 2>/dev/null; then
      if git push --quiet origin auto/integrate:main; then
        echo "[integrator] ✅ 已合併 $lane 到 main：$(git rev-parse --short HEAD)"
        git fetch --quiet origin main
        git reset --hard --quiet origin/main   # 對齊新 main,接著合下一個 lane
      else
        echo "[integrator] push 撞車(別人剛推),還原本輪、下輪再來"
        git reset --hard --quiet origin/main
      fi
    else
      echo "[integrator] ⚠️ $lane 合併後 build/test 紅,還原(不污染 main)"
      git reset --hard --quiet origin/main
    fi
  else
    git merge --abort 2>/dev/null || true
    echo "[integrator] ⚠️ $lane 與 main 衝突,跳過(該 lane 下輪 rebase 後會自動解)"
    # 衝突時把分支推上去開 PR,方便人看(沒有 gh 或失敗就算了)
    git push --quiet origin "$BR:$BR" 2>/dev/null || true
    command -v gh >/dev/null 2>&1 && gh pr create --base main --head "$BR" \
      --title "[$lane] 與 main 衝突,待整合" --body "integrator 自動合併時與 main 衝突,需 rebase 解。" 2>/dev/null || true
  fi
done
