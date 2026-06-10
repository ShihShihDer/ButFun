-- ROADMAP 36：裝備槽（顯式裝備欄）。inventories 表加 equipment 欄，存三槽 JSON。
-- 預設 NULL → 視為空槽，既有玩家首次登入時伺服器依背包自動裝上最強武器/護甲（零感知遷移）。
ALTER TABLE inventories ADD COLUMN IF NOT EXISTS equipment TEXT;
