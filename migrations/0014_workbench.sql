-- ROADMAP 36：地塊工作台欄位（向後相容；既有地塊預設無工作台）。
ALTER TABLE land_plots ADD COLUMN IF NOT EXISTS has_workbench BOOLEAN NOT NULL DEFAULT FALSE;
