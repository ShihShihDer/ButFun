-- ROADMAP 333：生態圖鑑（field guide）。玩家走近野生動物／守護者怪物即「發現」，
-- 已發現物種壓成單一 u64 bitmask 持久化（跨重啟保留，蒐集才有意義）。
-- 向後相容：ADD COLUMN IF NOT EXISTS，舊資料列讀為 0（圖鑑全空、重新蒐集）。
-- 型別 BIGINT 對應 Rust u64（以 i64 綁定，僅低位 0..TOTAL 有值，安全）。
ALTER TABLE players
  ADD COLUMN IF NOT EXISTS codex BIGINT NOT NULL DEFAULT 0;
