-- Phase 0-E：玩家最後狀態持久化（跨伺服器重啟回到原位）。
-- 對齊 BACKLOG「players 表（id, name, species, x, y, updated_at）」,並含 ether——
-- 記憶體前置 `positions::Saved` 已一併記住收成的乙太,持久化要保留以免重啟歸零。
--
-- 只「已登入」玩家（穩定 id）會被存:訪客 id 每次連線隨機,記了也對不上（見 positions.rs）。
-- 載入時座標仍一律過 `positions::spawn_at` 驗證（非有限退回地圖中央、界外夾回邊界）,
-- 故 DB 即使存進壞值也不會把玩家生到非法位置。
CREATE TABLE IF NOT EXISTS players (
    id         UUID PRIMARY KEY,
    name       TEXT        NOT NULL,
    species    TEXT        NOT NULL,
    x          REAL        NOT NULL,
    y          REAL        NOT NULL,
    ether      BIGINT      NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
