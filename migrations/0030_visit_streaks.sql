-- ROADMAP 397：連日歸鄉·歸鄉印記（visit streak）。
-- 為每位登入玩家記下「上次回訪是哪一天」與「連續回訪天數」，作為跨日的留存鉤子。
-- 向後相容：純 CREATE TABLE IF NOT EXISTS，不動 users 帳號表、不動任何玩家遊戲資料表。
--   user_id        — 玩家帳號 id（對齊 users.id）。
--   last_visit_day  — 上次回訪的 UTC 曆日序（Unix epoch 起的整數天，對齊 visit_streak.rs 的 i64）。
--   streak          — 連續回訪天數（>=1；斷日重置為 1）。
-- 重啟後保留，跨日的歸屬感才有意義。
CREATE TABLE IF NOT EXISTS visit_streaks (
    user_id        UUID        PRIMARY KEY,
    last_visit_day BIGINT      NOT NULL,
    streak         INT         NOT NULL,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
