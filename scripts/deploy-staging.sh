#!/usr/bin/env bash
# ButFun staging 快速部署:每 2 分鐘檢查一次,有新 commit 就 build+test+
# 重啟 staging service。為了讓 devloop 的改動很快在 staging 上能跑 e2e,
# 不必等 prod 的維護窗。
#
# Staging 跟 prod 共用同一份程式碼(同一個 binary),但讀不同的 .env.staging
# (不同 DB、不同 port、無 OAuth)。所以 build 一次就行,只需把 binary 再
# 用 staging 環境變數啟動。
#
# 注意:這支故意**不**做回滾——staging 就是用來踩雷的,壞了就壞了,
# devloop 看 log 修就好。Prod 才有 deploy.sh 的回滾保護。
set -euo pipefail

REPO="${BUTFUN_REPO:-/opt/butfun}"
SERVICE="${BUTFUN_STAGING_SERVICE:-butfun-staging}"
HEALTH_URL="${BUTFUN_STAGING_HEALTH_URL:-http://localhost:3001/healthz}"

cd "$REPO"

echo "[deploy-staging] 取最新 main…"
git fetch --quiet origin main
LOCAL="$(git rev-parse main 2>/dev/null || git rev-parse HEAD)"
REMOTE="$(git rev-parse "origin/main")"
if [ "$LOCAL" = "$REMOTE" ] && [ -x target/release/butfun-server ]; then
  echo "[deploy-staging] 已是最新,binary 也在,無需動作"
  exit 0
fi

# 不要動工作目錄上的 devloop WIP——只在 main 還沒跟上時 fast-forward 同步
if [ -n "$(git status --porcelain)" ]; then
  echo "[deploy-staging] 工作目錄有未 commit 改動(可能 devloop 還在跑),跳過這輪"
  exit 0
fi

git checkout --quiet main
if ! git merge --ff-only --quiet "origin/main"; then
  echo "[deploy-staging] main 與 origin 分歧,跳過"
  exit 0
fi

echo "[deploy-staging] build…"
cargo build --release

echo "[deploy-staging] 重啟 staging service…"
sudo systemctl restart "$SERVICE"

echo "[deploy-staging] healthz…"
for _ in $(seq 1 10); do
  if curl -fsS "$HEALTH_URL" >/dev/null 2>&1; then
    echo "[deploy-staging] 上 staging 成功:$(git rev-parse --short HEAD)"
    exit 0
  fi
  sleep 2
done

echo "[deploy-staging] healthz 失敗,看 journalctl -u $SERVICE"
exit 1
