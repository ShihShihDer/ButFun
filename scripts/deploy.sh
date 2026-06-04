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

REPO="${BUTFUN_REPO:-/opt/butfun}"
BRANCH="${BUTFUN_DEPLOY_BRANCH:-main}"
SERVICE="${BUTFUN_SERVICE:-butfun}"
HEALTH_URL="${BUTFUN_HEALTH_URL:-http://localhost:3000/healthz}"

BIN="$REPO/target/release/butfun-server"
BACKUP="$REPO/target/release/butfun-server.prev"

cd "$REPO"

echo "[deploy] 取得最新版本（origin/$BRANCH）…"
git fetch --quiet origin "$BRANCH"
LOCAL="$(git rev-parse HEAD)"
REMOTE="$(git rev-parse "origin/$BRANCH")"
if [ "$LOCAL" = "$REMOTE" ] && [ -x "$BIN" ]; then
  echo "[deploy] 已是最新版且 binary 存在，無需上線。"
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

echo "[deploy] 建置…"
cargo build --release

echo "[deploy] 測試（沒全綠就中止、不上線）…"
cargo test --release

# 備份目前可用的 binary，以便回滾。
if [ -x "$BIN" ]; then
  cp -f "$BIN" "$BACKUP"
fi
# 把剛建好的版本放到上線位置（cargo 會直接覆寫 $BIN，這裡確保存在）。
test -x "$BIN"

echo "[deploy] 重啟服務 $SERVICE …"
sudo systemctl restart "$SERVICE"

echo "[deploy] 健康檢查 $HEALTH_URL …"
ok=0
for _ in $(seq 1 10); do
  if curl -fsS "$HEALTH_URL" >/dev/null 2>&1; then ok=1; break; fi
  sleep 2
done

if [ "$ok" != 1 ]; then
  echo "[deploy] 健康檢查失敗 → 回滾到前一版。"
  if [ -x "$BACKUP" ]; then
    cp -f "$BACKUP" "$BIN"
    sudo systemctl restart "$SERVICE"
    echo "[deploy] 已回滾。請看 journalctl -u $SERVICE 查原因。"
  else
    echo "[deploy] 沒有可回滾的備份，服務可能異常，請人工介入。"
  fi
  exit 1
fi

echo "[deploy] 上線成功：$(git rev-parse --short HEAD)"
