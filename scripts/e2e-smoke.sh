#!/usr/bin/env bash
# ButFun E2E 煙霧測試:用 Node WebSocket 客戶端模擬玩家行為打 staging,驗
# 「實際操作會不會更新」。devloop 可在每輪結束後跑一下,失敗就回報。
#
# 目前涵蓋:
#   - 訪客進場、收到 welcome+snapshot
#   - 移動意圖→位置真的改變
#   - 重連(模擬 refresh)位置/乙太是否保留——抓住「refresh 跑掉」這條 bug
#
# 後續(BACKLOG):加 Playwright 跑 UI 觸控/滑桿/田地點擊。
set -euo pipefail
URL="${BUTFUN_STAGING_URL:-ws://localhost:3001/ws}"
exec node "$(dirname "$0")/e2e/smoke.mjs" "$URL"
