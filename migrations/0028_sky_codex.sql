-- ROADMAP 337：天象圖鑑（sky-watcher's almanac）。玩家身處某種天象之下即「目睹」，
-- 已目睹天象壓成單一 u64 bitmask 持久化（跨重啟保留，蒐集才有意義）。
-- 向後相容：ADD COLUMN IF NOT EXISTS，舊資料列讀為 0（天象全空、重新蒐集）。
-- 型別 BIGINT 對應 Rust u64（以 i64 綁定，僅低位 0..TOTAL 有值，安全），與 0026 codex／0027 atlas 同調。
ALTER TABLE players
  ADD COLUMN IF NOT EXISTS skylog BIGINT NOT NULL DEFAULT 0;
