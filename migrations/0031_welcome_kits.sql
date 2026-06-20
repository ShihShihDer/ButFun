-- ROADMAP 444：新手見面禮·故鄉的起手禮（welcome kit）。
-- 只記一件事：某帳號有沒有領過一次性的新手見面禮，避免重連／重啟後重複發放。
-- 向後相容：純 CREATE TABLE IF NOT EXISTS，不動 users 帳號表、不動任何玩家遊戲資料表。
--   user_id    — 玩家帳號 id（對齊 users.id），同時也是「已領」的存在性標記。
--   claimed_at — 領取時間（純記錄／除錯用，發放判斷只看這列存不存在）。
-- 重啟後保留，「只領一次」才跨重啟成立。
CREATE TABLE IF NOT EXISTS welcome_kits (
    user_id    UUID        PRIMARY KEY,
    claimed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
