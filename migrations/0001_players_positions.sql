-- Phase 0-E：玩家最後狀態（位置 + 收成乙太）跨伺服器重啟持久化。
-- 對齊 src/positions.rs 的 `Saved`：x/y 為世界座標（f32 → REAL）、ether 為收成累積。
--
-- 只存 PositionStore 擁有的欄位；玩家的 name/species 屬 UserStore（data/users.jsonl），
-- 之後另接一個 store 時再 migrate，避免單一巨大改動。
-- 載入時的壞值防線沿用 positions::spawn_at（非有限退回中央、界外夾回邊界），不在 schema 重複。
CREATE TABLE IF NOT EXISTS players (
    id         UUID PRIMARY KEY,
    x          REAL        NOT NULL,
    y          REAL        NOT NULL,
    -- ether 在程式裡是 u32；Postgres 無 unsigned，用 BIGINT(i64) 才裝得下整個 u32 範圍。
    ether      BIGINT      NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
