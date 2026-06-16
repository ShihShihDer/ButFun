#!/usr/bin/env bash
# ButFun 半自動營運迴圈（單一 systemd user timer 每 2 分心跳驅動；實際節奏自適應：
# 週額度 <50% 全速接力、≥50% 降回每 20 分巡航、≥80% 省電暫停——見下方守衛/節奏段）。
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
BUDGET_WEEKLY_USD="${BUTFUN_BUDGET_WEEKLY_USD:-250}"     # 退路：ccusage totalCost 代理（真實%數天過舊/缺才用）
PCT_STALE_MAX_SEC="${BUTFUN_PCT_STALE_MAX_SEC:-86400}"   # 真實%「過期但仍沿用」上限(秒)，預設 1 天：兼顧修半夜假停＋失花費追蹤最多 1 天就退$保底
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
pct_age="$(( now - ${pct_ts:-0} ))"
if [ -n "$seven_pct" ] && [ -n "$pct_ts" ] && [ "$pct_age" -lt 43200 ]; then
  # 真實%新鮮（<12h）：照舊用真實%。
  log "週額度 ${seven_pct}% / 上限 ${BUDGET_WEEKLY_PCT}%（Claude 真實 7d%）"
  awk "BEGIN{exit !(${seven_pct}+0 >= ${BUDGET_WEEKLY_PCT}+0)}" 2>/dev/null \
    && { over_budget=1; budget_reason="週額度 ${seven_pct}% ≥ ${BUDGET_WEEKLY_PCT}%（Claude 真實 7d%）"; }
elif [ -n "$seven_pct" ] && [ -n "$pct_ts" ] && [ "$pct_age" -lt "$PCT_STALE_MAX_SEC" ]; then
  # 真實%過期但仍在數天內：沿用「上次的真實%」判斷（週用量幾小時不會大跳），
  # 不退回不準的 $ 代理——避免半夜無 Claude session 刷新%時假停。
  log "週額度 ${seven_pct}%（上次真實 7d%，已 $((pct_age/3600))h 未更新，仍沿用）/ 上限 ${BUDGET_WEEKLY_PCT}%"
  awk "BEGIN{exit !(${seven_pct}+0 >= ${BUDGET_WEEKLY_PCT}+0)}" 2>/dev/null \
    && { over_budget=1; budget_reason="週額度 ${seven_pct}% ≥ ${BUDGET_WEEKLY_PCT}%（上次真實 7d%，沿用）"; }
else
  # 完全沒有真實%（或已數天過舊）→ 最後才退回 $ 代理保底。
  week_cost="$(ccusage weekly --json 2>/dev/null | jq -r '.weekly | last | .totalCost // 0' 2>/dev/null || echo 0)"
  log "本週等值花費 \$$week_cost / \$$BUDGET_WEEKLY_USD（\$代理；真實%缺或數天過舊）"
  awk "BEGIN{exit !(${week_cost:-0}+0 >= ${BUDGET_WEEKLY_USD}+0)}" 2>/dev/null \
    && { over_budget=1; budget_reason="本週等值花費 \$$week_cost ≥ \$$BUDGET_WEEKLY_USD（\$代理）"; }
fi
if [ -n "$over_budget" ]; then
  log "省電模式：$budget_reason → 暫停自走"
  cd "$COORD" && git pull --rebase -q || true
  printf '\n## [%s] 系統 | 省電模式\n%s，自走已暫停。新的一週會自動降回，或 `rm ~/.cache/butfun-auto/paused` 強制續跑。\n' \
    "$(date '+%Y-%m-%d %H:%M')" "$budget_reason" >> for_human.md
  git add for_human.md && git commit -q -m "chore: 省電模式（週預算達標，暫停自走）" && git push -q || true
  "$HERE/notify.sh" alert "省電模式：$budget_reason，自走已暫停" >/dev/null 2>&1 || true
  touch "$PAUSE"
  exit 0
fi

# ── 自適應節奏（timer 只是每 2 分的心跳，真正節奏這裡決定；純 shell、零 token）──
#   週額度 < THROTTLE_PCT（預設 50%）→ 全速：每次心跳都跑，上一輪結束即接力。
#   週額度 ≥ THROTTLE_PCT          → 巡航：至少隔 THROTTLE_INTERVAL_MIN（預設 20 分）一輪。
#   （≥ BUDGET_WEEKLY_PCT 80% 的省電暫停在上面，照舊是最後防線。）
#   真實 % 拿不到（快取過期）→ 保守視同已過半，走巡航節奏。
THROTTLE_PCT="${BUTFUN_THROTTLE_PCT:-50}"
THROTTLE_INTERVAL_MIN="${BUTFUN_THROTTLE_INTERVAL_MIN:-20}"
LAST_START_FILE="$STATE/last_turn_start"
full_speed=""
if [ -n "$seven_pct" ] && [ -n "$pct_ts" ] && [ "$((now - pct_ts))" -lt 43200 ]; then
  awk "BEGIN{exit !(${seven_pct}+0 < ${THROTTLE_PCT}+0)}" 2>/dev/null && full_speed=1
fi
if [ -z "$full_speed" ]; then
  last_start="$(cat "$LAST_START_FILE" 2>/dev/null || echo 0)"
  if [ "$((now - last_start))" -lt "$((THROTTLE_INTERVAL_MIN * 60))" ]; then
    log "巡航節奏（週額度 ${seven_pct:-?}% ≥ ${THROTTLE_PCT}% 或真實%不可得）：距上輪 $(((now - last_start) / 60)) 分 < ${THROTTLE_INTERVAL_MIN} 分，本輪略過"
    exit 0
  fi
  log "巡航節奏：距上輪已滿 ${THROTTLE_INTERVAL_MIN} 分，開跑"
else
  log "全速接力（週額度 ${seven_pct}% < ${THROTTLE_PCT}%）"
fi
date +%s > "$LAST_START_FILE"

cd "$REPO"
# 只 git fetch 更新 ref，**絕不**動主工作樹的 checkout/merge：worker 與 reviewer 各自用隔離
# worktree、都直接接 origin/main，主樹永遠保持不變 → 不會跟「在主樹編輯/commit 的人」競態
# （踩過雷：在主樹 checkout 跟人撞，commit 被倒回、檔案消失）。
git fetch --quiet origin main || true
turn="$(cat "$TURN_FILE" 2>/dev/null || echo work)"
log "turn=$turn"
# 離開 human 狀態就清掉推播去重旗標（下次再升級會再推一次）
[ "$turn" != "human" ] && rm -f "$STATE/human_notified" 2>/dev/null || true

case "$turn" in
  work|done)  # done 也跑 worker：ROADMAP 主軸做完時改進自主提案模式，絕不空轉（AI 自營運）
    WT="${BUTFUN_WORKER_WORKTREE:-/tmp/bf-worker}"
    git -C "$WT" rev-parse --git-dir >/dev/null 2>&1 || git worktree add --detach "$WT" >/dev/null 2>&1 || true
    cd "$WT" 2>/dev/null || cd "$REPO"
    "$HERE/notify.sh" beat "🔨 $(date '+%H:%M') 還在開發中（做好會通知你）…" >/dev/null 2>&1 || true
    log "worker：先試 Gemini（獨立額度，但也有限會見底）"
    # 注意：set -e 下「gout=$(gemini…)」失敗會直接 kill 腳本、根本跑不到 fallback——故用 && / || 保住 grc
    gout="$(gemini --yolo --skip-trust -p "$(cat "$HERE/worker.prompt")" 2>&1)" && grc=0 || grc=$?
    printf '%s\n' "$gout" | tail -25
    # Gemini 失敗（額度用盡 429 或任何錯）→ fallback 改用 Claude Sonnet（agentic、比 Opus 省、受 80% 守衛保護）
    if [ "$grc" -ne 0 ]; then
      log "Gemini 失敗/額度用盡（rc=$grc）→ fallback 用 Claude $WORKER_FALLBACK_MODEL 當 worker"
      cd "$WT" 2>/dev/null || cd "$REPO"
      exec claude -p --dangerously-skip-permissions --model "$WORKER_FALLBACK_MODEL" "$(cat "$HERE/worker.prompt")"
    fi
    ;;
  review)
    log "reviewer（Claude $REVIEW_MODEL）把關"
    "$HERE/notify.sh" beat "🔍 $(date '+%H:%M') 正在檢查剛做好的東西（跑測試,約 5-8 分）…" >/dev/null 2>&1 || true
    RWT="${BUTFUN_REVIEW_WORKTREE:-/tmp/bf-review}"
    git -C "$RWT" rev-parse --git-dir >/dev/null 2>&1 || git worktree add --detach "$RWT" >/dev/null 2>&1 || true
    before="$(git -C "$REPO" rev-parse origin/main 2>/dev/null)"
    cd "$RWT" 2>/dev/null || cd "$REPO"
    claude -p --dangerously-skip-permissions --model "$REVIEW_MODEL" "$(cat "$HERE/reviewer.prompt")" || true
    # 版本更新通知（backstop，不靠 reviewer 記得）：本輪若 origin/main 前進＝有 PR 被 merge → 推 update
    git -C "$REPO" fetch -q origin main 2>/dev/null || true
    after="$(git -C "$REPO" rev-parse origin/main 2>/dev/null)"
    if [ -n "${before:-}" ] && [ "$before" != "${after:-}" ]; then
      merged="$(git -C "$REPO" log --oneline "${before}..${after}" 2>/dev/null | grep -viE "Merge (pull request|branch)" | head -1 | sed -E 's/^[0-9a-f]+ //')"
      "$HERE/notify.sh" update "🎮 新功能上 staging 可以玩了：${merged:-（看 for_human.md）}" >/dev/null 2>&1 || true
    fi
    ;;
  human)
    if [ ! -f "$STATE/human_notified" ]; then
      "$HERE/notify.sh" alert "需要你決策 — 看 butfun-coord/for_human.md" >/dev/null 2>&1 || true
      touch "$STATE/human_notified"
    fi
    log "turn=human：(已推播) 等人處理 for_human.md，閒置"; exit 0 ;;
  # 註：done 不再單獨閒置，已併入上面 work|done)＝改跑自主提案模式（AI 自營運，絕不空轉）。
  *)     log "未知 turn=$turn，當 work"; WT="${BUTFUN_WORKER_WORKTREE:-/tmp/bf-worker}"; git -C "$WT" rev-parse --git-dir >/dev/null 2>&1 || git worktree add --detach "$WT" >/dev/null 2>&1 || true; cd "$WT" 2>/dev/null || cd "$REPO"; exec gemini --yolo --skip-trust -p "$(cat "$HERE/worker.prompt")" ;;
esac
