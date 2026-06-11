-- ROADMAP 96：好友系統——玩家可加其他帳號為好友。
-- 單向 follow：A 加了 B，A 的清單有 B；B 未加 A 則 B 清單不含 A。
-- user_id / friend_id 均 UUID，對應 users.id；不設外鍵（與其他 store 一致：耐久層彼此獨立）。
-- created_at 記錄加好友時間，留作日後排序或顯示用。
CREATE TABLE IF NOT EXISTS friends (
    user_id    UUID        NOT NULL,
    friend_id  UUID        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, friend_id)
);
CREATE INDEX IF NOT EXISTS idx_friends_user_id ON friends(user_id);
