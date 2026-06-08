#!/usr/bin/env bash
# 編 world-core 成 wasm 給前端(瀏覽器 instantiate 後呼叫 biome_code 等)。輸出到 web/wasm/。
# world-core 改動後重跑此腳本。需先 `rustup target add wasm32-unknown-unknown`。
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build -p world-core --target wasm32-unknown-unknown --release
mkdir -p web/wasm
cp target/wasm32-unknown-unknown/release/world_core.wasm web/wasm/world_core.wasm
echo "✅ world_core.wasm → web/wasm/"
