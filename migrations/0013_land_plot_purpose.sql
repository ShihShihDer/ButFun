-- ROADMAP 35：城外地塊新增用途欄（向後相容；既有地塊預設 free_build）。
ALTER TABLE land_plots ADD COLUMN IF NOT EXISTS purpose TEXT NOT NULL DEFAULT 'free_build';
