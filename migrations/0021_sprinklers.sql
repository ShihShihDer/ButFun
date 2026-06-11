-- ROADMAP 112：灑水器——玩家放置的自動澆灌裝置，每 30 秒澆灌周圍 2 格作物。
-- 向後相容：純 CREATE TABLE IF NOT EXISTS，不影響既有任何表。
CREATE TABLE IF NOT EXISTS sprinklers (
    id      BIGSERIAL PRIMARY KEY,
    user_id UUID      NOT NULL,
    wx      DOUBLE PRECISION NOT NULL,
    wy      DOUBLE PRECISION NOT NULL
);
