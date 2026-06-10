-- 為 npc_memory 加入真實交易統計欄位（ROADMAP 61 關係綁真實交易）。
-- 向後相容：舊資料預設 0（等同沒有交易紀錄），NPC 從「無交易往來」開始感知。
ALTER TABLE npc_memory
    ADD COLUMN IF NOT EXISTS sell_count INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS buy_count  INTEGER NOT NULL DEFAULT 0;
