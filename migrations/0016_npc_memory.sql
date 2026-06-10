-- NPC 對每位玩家的個人記憶（印象 / 往來次數 / 是否已送禮）持久化。
-- 向後相容：舊玩家沒有資料等同初次見面，NPC 從空印象開始——零感知。
CREATE TABLE IF NOT EXISTS npc_memory (
    player_id UUID    NOT NULL,
    npc_id    TEXT    NOT NULL,
    impression TEXT   NOT NULL DEFAULT '',
    talks     INTEGER NOT NULL DEFAULT 0,
    gifted    BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (player_id, npc_id)
);

-- NPC 自身的送禮餘裕（送完就沒了，不能無中生有）。
-- 初次啟動時由應用程式以 initial_gift_stock() 預設值補 upsert，不在 migration 寫死。
CREATE TABLE IF NOT EXISTS npc_gift_stock (
    npc_id TEXT    PRIMARY KEY,
    stock  INTEGER NOT NULL DEFAULT 0
);
