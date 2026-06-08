#!/usr/bin/env bash
# ButFun 半自動營運迴圈（單一 systemd user timer 驅動，每 ~20 分一輪）。
#
# 省 token 結構：
#   - dev worker = Gemini CLI（另一份額度，--yolo -w 自走、自動隔離 worktree）→ Claude 不做苦力。
#   - reviewer/總監 = Claude（Sonnet，低頻；judgment 值錢處）→ 只在有 PR 待審時跑。
#   - 閘門 = 純 shell：沒事不喚醒任何 LLM（事件驅動、零 token）。
#   - 本機 cargo 全綠才開 PR（編譯/測試在地端攔，不燒 LLM 試錯）。
#   - 預算守衛：Claude 週花費逼近上限就轉「省電」（暫停自走、通知人）。
#
# 方向：worker 照 docs/ROADMAP.md 主軸由上往下，不准漂去補洞（治「只優化小問題不長主軸」）。
# 部署：永不自動。prod 上線是 deploy.sh + 人；merge 後 staging 自動更新供玩。
#
# 一鍵停： systemctl --user disable --now butfun-auto.timer
# 暫停：   touch ~/.cache/butfun-auto/paused（刪掉即恢復）
# 看紀錄： journalctl --user -u butfun-auto -n 100 ; butfun-coord/for_human.md ; GitHub PR
set -euo pipefail

REPO="${BUTFUN_REPO:-/home/shihshih/ButFun}"
COORD="${BUTFUN_COORD:-/home/shihshih/butfun-coord}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STATE="$HOME/.cache/butfun-auto"; mkdir -p "$STATE"
TURN_FILE="$STATE/turn"
PAUSE="$STATE/paused"
# 預算守衛門檻。優先用「真實週額度%」（Claude Code 注入 statusline 的 rate_limits.seven_day，
# 由 statusline-expo.sh 快取到 ~/.cache/butfun-auto/seven_day_pct）；拿不到才退回 $ 代理。
BUDGET_WEEKLY_PCT="${BUTFUN_BUDGET_WEEKLY_PCT:-80}"      # 你的目標：週用量壓在 80% 以下
BUDGET_WEEKLY_USD="${BUTFUN_BUDGET_WEEKLY_USD:-250}"     # 退路：ccusage totalCost 代理（真實%過期時用）
REVIEW_MODEL="${BUTFUN_REVIEW_MODEL:-claude-sonnet-4-6}"          # 把關用（Sonnet，比 Opus 省）
WORKER_FALLBACK_MODEL="${BUTFUN_WORKER_FALLBACK_MODEL:-claude-sonnet-4-6}"  # Gemini 沒額度時的備胎 worker

log(){ echo "[auto $(date '+%H:%M')] $*"; }

[ -f "$PAUSE" ] && { log "paused（$PAUSE 存在），本輪不動"; exit 0; }

# 互斥：同一時間只准一輪
exec 9>/tmp/butfun-auto.lock
flock -n 9 || { log "上一輪還在跑，本輪讓位"; exit 0; }

# ── 預算守衛：優先真實週額度%，退回 $ 代理 ──────────────────────
over_budget=""; budget_reason=""
pct_line="$(cat "$STATE/seven_day_pct" 2>/dev/null || true)"
seven_pct="${pct_line%% *}"; pct_ts="${pct_line##* }"; now="$(date +%s)"
if [ -n "$seven_pct" ] && [ -n "$pct_ts" ] && [ "$((now - pct_ts))" -lt 43200 ]; then
  log "週額度 ${seven_pct}% / 上限 ${BUDGET_WEEKLY_PCT}%（Claude 真實 7d%）"
  awk "BEGIN{exit !(${seven_pct}+0 >= ${BUDGET_WEEKLY_PCT}+0)}" 2>/dev/null \
    && { over_budget=1; budget_reason="週額度 ${seven_pct}% ≥ ${BUDGET_WEEKLY_PCT}%（Claude 真實 7d%）"; }
else
  week_cost="$(ccusage weekly --json 2>/dev/null | jq -r '.weekly | last | .totalCost // 0' 2>/dev/null || echo 0)"
  log "本週等值花費 \$$week_cost / \$$BUDGET_WEEKLY_USD（\$代理；真實%快取過期或缺）"
  awk "BEGIN{exit !(${week_cost:-0}+0 >= ${BUDGET_WEEKLY_USD}+0)}" 2>/dev/null \
    && { over_budget=1; budget_reason="本週等值花費 \$$week_cost ≥ \$$BUDGET_WEEKLY_USD（\$代理）"; }
fi
if [ -n "$over_budget" ]; then
  log "省電模式：$budget_reason → 暫停自走"
  cd "$COORD" && git pull --rebase -q || true
  printf '\n## [%s] 系統 | 省電模式\n%s，自走已暫停。新的一週會自動降回，或 `rm ~/.cache/butfun-auto/paused` 強制續跑。\n' \
    "$(date '+%Y-%m-%d %H:%M')" "$budget_reason" >> for_human.md
  git add for_human.md && git commit -q -m "chore: 省電模式（週預算達標，暫停自走）" && git push -q || true
  "$HERE/notify.sh" "省電模式：$budget_reason，自走已暫停" >/dev/null 2>&1 || true
  touch "$PAUSE"
  exit 0
fi

cd "$REPO"
git fetch --quiet origin main || true
# 盡力把主工作樹同步到最新 main，讓 gemini -w 開出的隔離 worktree 接在最新 main 上
# （髒了或分歧就跳過、不破壞——worker 在自己 worktree 內還會再 rebase origin/main 一次）
git checkout main --quiet 2>/dev/null || true
git merge --ff-only --quiet origin/main 2>/dev/null || true
turn="$(cat "$TURN_FILE" 2>/dev/null || echo work)"
log "turn=$turn"
# 離開 human 狀態就清掉推播去重旗標（下次再升級會再推一次）
[ "$turn" != "human" ] && rm -f "$STATE/human_notified" 2>/dev/null || true

case "$turn" in
  work)
    WT="${BUTFUN_WORKER_WORKTREE:-/tmp/bf-worker}"
    git -C "$WT" rev-parse --git-dir >/dev/null 2>&1 || git worktree add --detach "$WT" >/dev/null 2>&1 || true
    cd "$WT" 2>/dev/null || cd "$REPO"
    log "worker：先試 Gemini（獨立額度、不吃 Claude，但也有限會見底）"
    gout="$(gemini --yolo --skip-trust -p "$(cat "$HERE/worker.prompt")" 2>&1)"; grc=$?
    printf '%s\n' "$gout" | tail -25
    # Gemini 額度用光（重試後仍失敗）→ fallback 改用 Claude Sonnet 當 worker（一樣 agentic、比 Opus 省；
    # 頂端 80% 預算守衛已保護，所以備胎不會爆 Claude 週額度）。免裝 aider、可靠度比地端高。
    if [ "$grc" -ne 0 ] && printf '%s' "$gout" | grep -qiE "exhausted|quota|RESOURCE_EXHAUSTED|\b429\b"; then
      log "Gemini 額度用盡（rc=$grc）→ fallback 用 Claude $WORKER_FALLBACK_MODEL 當 worker"
      cd "$WT" 2>/dev/null || cd "$REPO"
      exec claude -p --dangerously-skip-permissions --model "$WORKER_FALLBACK_MODEL" "$(cat "$HERE/worker.prompt")"
    fi
    ;;
  review)
    log "reviewer（Claude $REVIEW_MODEL）把關"
    cd "$REPO"
    exec claude -p --dangerously-skip-permissions --model "$REVIEW_MODEL" "$(cat "$HERE/reviewer.prompt")"
    ;;
  human)
    if [ ! -f "$STATE/human_notified" ]; then
      "$HERE/notify.sh" "需要你決策 — 看 butfun-coord/for_human.md" >/dev/null 2>&1 || true
      touch "$STATE/human_notified"
    fi
    log "turn=human：(已推播) 等人處理 for_human.md，閒置"; exit 0 ;;
  done)  log "turn=done：主軸都做完，閒置"; exit 0 ;;
  *)     log "未知 turn=$turn，當 work"; WT="${BUTFUN_WORKER_WORKTREE:-/tmp/bf-worker}"; git -C "$WT" rev-parse --git-dir >/dev/null 2>&1 || git worktree add --detach "$WT" >/dev/null 2>&1 || true; cd "$WT" 2>/dev/null || cd "$REPO"; exec gemini --yolo --skip-trust -p "$(cat "$HERE/worker.prompt")" ;;
esac
