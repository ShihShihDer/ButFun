#!/usr/bin/env bash
# ButFun 安全部署腳本
#
# 在每日維護窗（由 butfun-deploy.timer 觸發，例如 03:00）把最新通過測試的
# 版本換上線。設計原則：
#   1. 測試沒全綠 → 不上線。
#   2. 上線前備份目前可用的 binary，失敗可一鍵回滾。
#   3. 上線後做健康檢查，連不上就自動回滾到前一版。
#
# Claude 的開發迴圈「碰不到」這支腳本——上線是確定性、與 AI 無關的動作。
#
# 可用環境變數覆寫（見 deploy/systemd/butfun-deploy.service）：
#   BUTFUN_REPO           repo 根目錄（預設 /opt/butfun）
#   BUTFUN_DEPLOY_BRANCH  要上線的分支（預設 main）
#   BUTFUN_SERVICE        systemd 服務名（預設 butfun）
#   BUTFUN_HEALTH_URL     健康檢查網址（預設 http://localhost:3000/healthz）

set -euo pipefail

REPO="${BUTFUN_REPO:-/home/shihshih/ButFun}"
BRANCH="${BUTFUN_DEPLOY_BRANCH:-main}"
SERVICE="${BUTFUN_SERVICE:-butfun}"
HEALTH_URL="${BUTFUN_HEALTH_URL:-http://localhost:3000/healthz}"

BIN="$REPO/target/release/butfun-server"
BACKUP="$REPO/target/release/butfun-server.prev"
# 記錄「實際已部署的 commit」。不能用工作樹 HEAD 判斷——HEAD 會被 push / 別的 actor ff 前進，
# 但 binary 沒重建，會誤判「已最新」而跳過真正的換版（踩過這個雷）。
DEPLOYED_FILE="${BUTFUN_DEPLOYED_FILE:-$HOME/.cache/butfun-last-deployed}"
mkdir -p "$(dirname "$DEPLOYED_FILE")" 2>/dev/null || true

cd "$REPO"

# 簡單推播:prod 真正換版成功時推一條到使用者手機(NTFY_TOPIC 在 systemd 環境、不入 repo)。
notify() {
  [ -n "${NTFY_TOPIC:-}" ] || return 0
  curl -s -m 6 -H "Title: ButFun 已上線 🚀" -H "Tags: rocket" \
    -d "$1" "https://ntfy.sh/${NTFY_TOPIC}" >/dev/null 2>&1 || true
}

echo "[deploy] 取得最新版本（origin/$BRANCH）…"
git fetch --quiet origin "$BRANCH"
REMOTE="$(git rev-parse "origin/$BRANCH")"
DEPLOYED="$(cat "$DEPLOYED_FILE" 2>/dev/null || echo none)"
if [ "$DEPLOYED" = "$REMOTE" ] && [ -x "$BIN" ]; then
  echo "[deploy] 已部署過此版（$REMOTE）且 binary 存在，無需上線。"
  exit 0
fi

git checkout --quiet "$BRANCH"
# 安全變更：只接受 fast-forward；若本地有未 push 的手改、或有未 commit 變動，
# 就中止這一輪上線（不要用 reset --hard 把線上 oncall 手改吃掉）。
if [ -n "$(git status --porcelain)" ]; then
  echo "[deploy] 工作目錄有未 commit 改動，中止上線（等手改清理乾淨再說）"
  exit 1
fi
if ! git merge --ff-only --quiet "origin/$BRANCH"; then
  echo "[deploy] 本地 $BRANCH 與 origin 分歧，中止上線（可能有 oncall 手改未 push）"
  exit 1
fi

# 先備份「目前正在跑的舊 binary」以便回滾——一定要在 build 之前，
# 否則 cargo 覆寫 $BIN 後備到的是新版、回滾等於沒回滾（踩過這個雷）。
if [ -x "$BIN" ]; then
  cp -f "$BIN" "$BACKUP"
fi

echo "[deploy] 建置…"
# sqlx::migrate! 在 src/db.rs 編譯期把 migrations/ 內嵌進 binary。增量編譯下，**新增**
# migration 檔不會自動讓那個巨集重新展開 → binary 內嵌的 migration 會停在舊集合，配上
# DB 已套用的新版本就開機 panic（Migrate(VersionMissing(N))，prod-down crash loop）。
# 每次部署前 touch db.rs，強制重新內嵌「當前所有」migration，根治這類崩潰。
touch "$REPO/src/db.rs"
cargo build --release

# wasm 地形（空氣牆根治）：world-core 編成 .wasm 供前端載入，前後端同一份實作。
# 軟降級：wasm 建置失敗不擋遊戲上線——前端載不到 .wasm 會自動退回 JS 後備地形。
echo "[deploy] 建置 world-core wasm…"
if ! bash "$REPO/scripts/build-wasm.sh"; then
  echo "[deploy] ⚠️ wasm 建置失敗（前端將用 JS 後備地形），繼續上線。"
fi

# 官網更新日誌：從 git 歷史產生 web/site/news.json（零 token，AI 合的 PR 自動上官網）。
# 軟降級：失敗只是官網日誌停在舊檔，不擋遊戲上線。
echo "[deploy] 產生官網更新日誌…"
if ! node "$REPO/scripts/site/gen-news.mjs"; then
  echo "[deploy] ⚠️ news.json 產生失敗（官網日誌維持舊檔），繼續上線。"
fi

echo "[deploy] 測試（沒全綠就中止、不上線）…"
cargo test --release

# 確保上線位置有 binary（cargo 已覆寫 $BIN 為新版）。
test -x "$BIN"

echo "[deploy] 重啟服務 $SERVICE …"
sudo systemctl restart "$SERVICE"

echo "[deploy] 健康檢查 $HEALTH_URL …"
ok=0
for _ in $(seq 1 10); do
  if curl -fsS "$HEALTH_URL" >/dev/null 2>&1; then ok=1; break; fi
  sleep 2
done

rollback() {
  if [ -x "$BACKUP" ]; then
    cp -f "$BACKUP" "$BIN"
    sudo systemctl restart "$SERVICE"
    echo "[deploy] 已回滾。請看 journalctl -u $SERVICE 查原因。"
  else
    echo "[deploy] 沒有可回滾的備份，服務可能異常，請人工介入。"
  fi
  exit 1
}

if [ "$ok" != 1 ]; then
  echo "[deploy] /healthz 健康檢查失敗 → 回滾。"
  rollback
fi

# /healthz 只驗 HTTP 活著，無法偵測「遊戲迴圈 tokio task 靜默炸死」這一型事故。
# 再跑 WS 冒煙閘：連 /ws → 斷言快照含自身 id → 斷言兩幀 tick 有推進。
echo "[deploy] WS 遊戲迴圈冒煙閘…"
WS_PORT=$(echo "$HEALTH_URL" | sed 's|.*localhost:\([0-9]*\).*|\1|;t;s|.*|3000|')
WS_URL="ws://localhost:${WS_PORT}/ws"
if ! node "$REPO/scripts/e2e/gameloop-smoke.mjs" "$WS_URL"; then
  echo "[deploy] WS 冒煙閘失敗（遊戲迴圈可能已死）→ 回滾。"
  rollback
fi

git rev-parse HEAD > "$DEPLOYED_FILE"
echo "[deploy] 上線成功：$(git rev-parse --short HEAD)"
notify "玩家現在玩到的就是這版：$(git log -1 --format=%s)"
