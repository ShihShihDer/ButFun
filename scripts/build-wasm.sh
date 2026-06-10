#!/usr/bin/env bash
# 編 world-core 成 wasm 給前端（瀏覽器 instantiate 後呼叫 biome_code / tile_kind_code）。
# 這是「空氣牆根治」的核心：前端地貌與伺服器用**同一份 Rust 實作**，鏡像不再漂移。
#
# 純數值進出，不需要 wasm-pack / wasm-bindgen——cargo 直編 wasm32 即可，產物僅數 KB。
# 產物 web/wasm/world_core.wasm 已 gitignore（建置產物不入 repo）；deploy.sh 每次上線
# 會重跑這支。本地開發不跑也行——前端載不到 .wasm 會自動退回 JS 後備地形。
set -euo pipefail
cd "$(dirname "$0")/.."

# wasm32 target 沒裝就先裝（rustup 環境一行搞定）。
if ! rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
  echo "[wasm] 安裝 wasm32-unknown-unknown target…"
  rustup target add wasm32-unknown-unknown
fi

cargo build -p world-core --target wasm32-unknown-unknown --release
mkdir -p web/wasm
cp -f target/wasm32-unknown-unknown/release/world_core.wasm web/wasm/world_core.wasm
echo "✅ world_core.wasm → web/wasm/（$(stat -c %s web/wasm/world_core.wasm) bytes）"
