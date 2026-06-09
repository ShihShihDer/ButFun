-- 玩家經驗值欄位（ROADMAP 17 升級系統）。
--
-- exp 隨殺怪 / 採礦累積，等級由前後端各自從 exp 推算（不另存 level）。
-- 向後相容：不動既有任何表或欄位，純新增。

ALTER TABLE players ADD COLUMN IF NOT EXISTS exp BIGINT NOT NULL DEFAULT 0;
