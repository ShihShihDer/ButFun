#!/usr/bin/env bash
# 自包含 e2e 冒煙閘：起 server → 跑 WS 遊戲迴圈驗證 → 殺 server。
# 無 DATABASE_URL 時走記憶體模式（測試用）；有設則對 DB 測試。
#
# 用法：
#   scripts/e2e-gameloop.sh              # 用現有 release binary
#   scripts/e2e-gameloop.sh --build      # 強制 cargo build --release 後再跑
#
# 成功 exit 0；失敗 exit 1。可接進 deploy.sh 或 reviewer 前置閘。

set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
BUILD=0
for arg in "$@"; do [ "$arg" = "--build" ] && BUILD=1; done

BIN="$REPO/target/release/butfun-server"

if [ "$BUILD" = 1 ] || [ ! -x "$BIN" ]; then
  echo "[e2e-gameloop] cargo build --release ..."
  (cd "$REPO" && cargo build --release --quiet)
fi

# 選一個本機沒在用的埠（避免撞到正式線上的 3000）。
PORT=19847

# 確認埠號空閒。
if ss -ltn | grep -q ":${PORT} "; then
  echo "[e2e-gameloop] 埠 ${PORT} 被占用，中止" >&2
  exit 1
fi

SERVER_PID=""
cleanup() {
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

echo "[e2e-gameloop] 啟動測試用伺服器（PORT=${PORT}，記憶體模式）…"
PORT="$PORT" "$BIN" &>/tmp/butfun-e2e-gameloop.log &
SERVER_PID=$!

# 等 healthz 就緒（最多 10 秒）。
ok=0
for i in $(seq 1 20); do
  if curl -fsS "http://localhost:${PORT}/healthz" >/dev/null 2>&1; then
    ok=1; break
  fi
  sleep 0.5
done

if [ "$ok" != 1 ]; then
  echo "[e2e-gameloop] 伺服器啟動失敗（healthz 未回應）" >&2
  cat /tmp/butfun-e2e-gameloop.log >&2
  exit 1
fi

echo "[e2e-gameloop] 伺服器就緒，跑 WS 冒煙測試…"
set +e
node "$REPO/scripts/e2e/gameloop-smoke.mjs" "ws://localhost:${PORT}/ws"
RESULT=$?
set -e

if [ "$RESULT" != 0 ]; then
  echo "[e2e-gameloop] ❌ 冒煙測試失敗！伺服器 log 如下：" >&2
  tail -30 /tmp/butfun-e2e-gameloop.log >&2
fi

exit "$RESULT"
