-- ROADMAP 336：探索圖鑑（explorer's atlas）。玩家走近各種奇景地形即「探索」，
-- 已踏足地形壓成單一 u64 bitmask 持久化（跨重啟保留，蒐集才有意義）。
-- 向後相容：ADD COLUMN IF NOT EXISTS，舊資料列讀為 0（探索全空、重新蒐集）。
-- 型別 BIGINT 對應 Rust u64（以 i64 綁定，僅低位 0..TOTAL 有值，安全），與 0026 codex 同調。
ALTER TABLE players
  ADD COLUMN IF NOT EXISTS atlas BIGINT NOT NULL DEFAULT 0;
