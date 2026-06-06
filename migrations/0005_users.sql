-- Phase 0-E：使用者帳號持久化（跨伺服器重啟保住玩家身分／改名／物種）。
-- 帳號（users.rs）是所有「已登入」狀態的根：位置／背包／農地都以這個穩定 id 為鍵。
-- 過去帳號只存 data/users.jsonl（append-only），換成 Postgres 版後，這張表讓帳號跟著
-- 一起走 DB，對齊 BACKLOG 0-E「一次一個 store 接 PG」——users 是位置／背包／農地／日夜
-- 之後最後一個還在 JSONL 的核心 store。
--
-- 與其他 0-E 表一致、刻意不設外鍵：各 store 的 flush 彼此獨立、無寫入順序耦合。
-- (provider, external_id) 是登入比對鍵（Google 的 sub、AI 居民的 uuid），設 UNIQUE 鎖死
-- 「同一個外部身分只對到一個內部帳號」——find_or_create 靠它判斷 returning 玩家。
--
-- id 是內部主鍵；改名走 ON CONFLICT (id) DO UPDATE（沿用 users.rs 的「同 id 後寫覆蓋」契約，
-- 取代 JSONL 的 append-last-wins）。created_at 存 Unix 毫秒（BIGINT，對齊 User.created_at: u64）。
-- name / species 是會廣播給所有人的顯示用欄位，寫入與載入都過 sanitizer（見 users.rs），
-- 故磁碟即使被竄改也不會把控制字元帶進廣播。email 不顯示給其他玩家、可為 NULL。
CREATE TABLE IF NOT EXISTS users (
    id          UUID        PRIMARY KEY,
    provider    TEXT        NOT NULL,
    external_id TEXT        NOT NULL,
    email       TEXT,
    name        TEXT        NOT NULL,
    species     TEXT        NOT NULL,
    created_at  BIGINT      NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (provider, external_id)
);
