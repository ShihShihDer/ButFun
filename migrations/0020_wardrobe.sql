-- ROADMAP 99：換造型系統（衣櫥）——玩家可切換服裝造型，他人也看得到。
-- costume 欄位：0~5 六套蒸汽龐克原創造型。DEFAULT 0 向後相容，不影響既有帳號。
ALTER TABLE users ADD COLUMN IF NOT EXISTS costume SMALLINT NOT NULL DEFAULT 0;
