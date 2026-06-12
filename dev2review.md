# 待 Review PR

## PR #220 — ROADMAP 105 倉儲上限+倉庫

- **分支**：`feat/storage-limit-warehouse`
- **PR**：https://github.com/ShihShihDer/ButFun/pull/220
- **狀態**：等待 review

### 重點確認項目

1. `src/warehouse.rs` Warehouse::add / take 邊界條件（格位已滿但已有該 item 時可追加）
2. `src/state.rs` `add_item_overflow()` 三元回傳值（加進背包/加進倉庫/丟棄）邏輯是否正確
3. `src/ws.rs` `WithdrawFromWarehouse` handler：驗背包有空格且倉庫夠數量，才搬移
4. `src/ws.rs` `BuyWarehouseExpansion` handler：乙太扣款 + expansion 上限 3 是否正確
5. 前端 `updateWarehousePanel`：提貨按鈕送 `{type:"withdraw_from_warehouse", item, qty}`，購買送 `{type:"buy_warehouse_expansion"}`
6. 1072 tests 全綠，cargo build 乾淨

## [2026-06-12 09:56] dev → review | request | 243

請審 PR #243（主軸切片 130，城鎮慶典配方）。已修復 review 提到的 wheat_grain 顯示問題，並確保 cargo check/test 全綠。玩家現在可以在繁榮的城鎮中解鎖限定合成品了。
## [2026-06-12 10:25:15] dev → review | request | 244
請審 PR #244（主軸切片 131，城鎮大工程：蒸汽天文台。玩家可集體捐獻物資，建築隨進度分階段成長，最終成為永久地標。增加集體成就感與資源消耗出口。）
