-- Phase 0-E：日夜時鐘持久化（跨伺服器重啟接續世界時刻，不每次換版都跳回破曉）。
-- 日夜循環（daynight.rs）是 0-G 療癒核心的一部分，0-G 全程反覆標注「仍待：日夜時刻
-- 持久化（接 0-E）」——這張表補上最後一塊：重啟後從同一個時刻接續，而非歸零回破曉。
-- 部署窗在深夜（03:00–05:00），沒持久化時每次換版都把世界從夜晚硬跳回破曉，這裡讓它平順接續。
--
-- 與 players／inventories／fields 不同：日夜是**單一全域時鐘**、不分玩家，故是 singleton
-- 一列表——用固定主鍵 `id = 1` + CHECK 約束鎖死只會有一列，upsert 永遠落在同一列。
--
-- elapsed 存「循環內已經過的秒數」（REAL，對齊 daynight.rs 的 f32）。載入時一律經
-- `DayNight::at(elapsed)` 還原：非有限退回破曉、界外／負值取模繞回，故磁碟即使存進壞值也不會
-- 把時鐘帶成 NaN／界外（延續 positions REAL 欄位存讀、載入時 `spawn_at` 守門的同一套思路）。
CREATE TABLE IF NOT EXISTS daynight (
    id         INTEGER     PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    elapsed    REAL        NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
