-- ROADMAP 341：喝采人氣（player cheers）。其他玩家對你「👏 喝采」即替你累積一點人氣，
-- 累積值壓成單一 u64 持久化（跨重啟保留，人氣身份才有意義）。
-- 向後相容：ADD COLUMN IF NOT EXISTS，舊資料列讀為 0（人氣全空、重新累積）。
-- 型別 BIGINT 對應 Rust u64（以 i64 綁定，僅單調遞增的小計數，安全），與 0026 codex／0027 atlas／0028 skylog 同調。
ALTER TABLE players
  ADD COLUMN IF NOT EXISTS cheers BIGINT NOT NULL DEFAULT 0;
