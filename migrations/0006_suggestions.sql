-- Phase 0-E：玩家建議箱持久化（跨伺服器重啟保住玩家送回的回饋）。
-- 建議（suggestions.rs）是「收建議 → 改版 → 發佈」營運迴圈的伺服器端輸入：是真實玩家資料，
-- 換版洗檔同樣會丟。過去只存 data/suggestions.jsonl（append-only），換成 Postgres 版後這張表
-- 讓建議跟著一起走 DB，對齊 BACKLOG 0-E「一次一個 store 接 PG」——這是位置／背包／農地／日夜／
-- 帳號之後最後一個還在 JSONL 的核心 store。
--
-- 與其他 0-E 表一致、刻意不設外鍵：各 store 的 flush 彼此獨立、無寫入順序耦合（且建議的署名
-- 是玩家自填字串、未必對應任何 users.id，匿名建議更沒有帳號）。
--
-- id 用 BIGSERIAL 當內部主鍵：建議是 append-only、無更新語意（不像帳號要 ON CONFLICT 改名），
-- 自動遞增的 id 順帶保住「送出順序」當 list 的次要排序鍵。from_name / text 是經 sanitizer
-- 清過控制字元後才存的顯示用欄位（見 suggestions.rs），載入時也再過一次同一道防線。
-- at 存 Unix 毫秒（BIGINT，對齊 Suggestion.at: u64）。"from" 是 SQL 保留字，故欄名用 from_name。
CREATE TABLE IF NOT EXISTS suggestions (
    id          BIGSERIAL   PRIMARY KEY,
    from_name   TEXT        NOT NULL,
    text        TEXT        NOT NULL,
    at          BIGINT      NOT NULL
);
