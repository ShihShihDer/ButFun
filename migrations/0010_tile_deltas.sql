-- 地形格差異表（C-1 可挖地形起步）。
--
-- 初始地形由伺服器確定性生成、前端同步計算（world-core tile_kind_at），**不存整張世界**；
-- 只存玩家挖 / 建後偏離預設的格子 → 稀疏節省空間（C-1 此表初始為空）。
-- 向後相容：不動既有任何表，純新增。

CREATE TABLE IF NOT EXISTS tile_deltas (
    chunk_cx   INT      NOT NULL,
    chunk_cy   INT      NOT NULL,
    cell_x     SMALLINT NOT NULL,
    cell_y     SMALLINT NOT NULL,
    kind       TEXT     NOT NULL DEFAULT 'empty',
    PRIMARY KEY (chunk_cx, chunk_cy, cell_x, cell_y)
);
