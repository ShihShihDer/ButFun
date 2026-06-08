-- Phase ③ Slice D：領地重置 + 空世界開局。
-- 重置所有玩家的地塊歸屬，讓世界回到「空世界」狀態，對齊「自己攢乙太買地」的主軸方向。
-- ⚠️ 資料安全：保留原有資料至備份表，絕不直接 DROP 玩家的努力（符合「保留作物」契約）。

-- 1. 將既有農地資料備份至 fields_v1_backup。
CREATE TABLE IF NOT EXISTS fields_v1_backup AS SELECT * FROM fields;

-- 2. 清空 fields 表，達成「空世界開局」。
-- 這樣所有玩家進場後都沒有地，必須透過新開發的 ClaimPlot 動作購買。
TRUNCATE fields;

-- 註：positions / inventories / users / suggestions 保持不動，保留玩家的乙太、背包與帳號。
