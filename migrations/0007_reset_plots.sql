-- Phase ③ Slice D：領地重置 + 空世界開局。
-- 重置所有玩家的地塊歸屬，讓世界回到「空世界」狀態，對齊「自己攢乙太買地」的主軸方向。
-- ⚠️ 資料安全：保留原有資料至備份表，絕不直接 DROP 玩家的努力（符合「保留作物」契約）。

-- 1. 將既有農地資料備份至 fields_v1_backup。
CREATE TABLE IF NOT EXISTS fields_v1_backup AS SELECT * FROM fields;

-- 2. 空世界開局由程式邏輯保證：index_of 對從未 claim 的玩家回 None，
-- 不需要 TRUNCATE fields——新玩家天然沒有地，現有玩家保留作物（向後相容）。

-- 註：positions / inventories / users / suggestions 保持不動，保留玩家的乙太、背包與帳號。
