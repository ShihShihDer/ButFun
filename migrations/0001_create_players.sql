-- Phase 0-E:玩家位置持久化。
-- 伺服器啟動時(src/store.rs::run_migration)會以 CREATE TABLE IF NOT EXISTS
-- 自動套用同一份 schema;此檔留作正式 migration / 人工套用與審查的依據。
-- 向後相容:只新增、不 drop 既有欄位資料。之後背包 / 農地以後續 migration 疊加。

CREATE TABLE IF NOT EXISTS players (
    id         UUID PRIMARY KEY,
    name       TEXT NOT NULL,
    species    TEXT NOT NULL,
    x          DOUBLE PRECISION NOT NULL,
    y          DOUBLE PRECISION NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
