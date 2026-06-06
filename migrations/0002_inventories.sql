-- Phase 0-E：玩家背包持久化（跨伺服器重啟保住採集/打怪/收成囤積的素材）。
-- 三個來源（採集、打怪掉落、農地收成）現在都在灌背包,囤積卻撐不過換版重啟——
-- 這張表讓已登入玩家的背包跟著重連回來,對齊 BACKLOG 0-E「一次一個 store 接 PG」。
--
-- 與 players 表同樣只記「已登入」玩家（穩定 id）；訪客 id 每次隨機、記了也對不上。
-- 刻意不對 players 表設外鍵:背包 flush 與位置 flush 各自獨立、無寫入順序耦合,
-- 任一方先落地都不該因另一方還沒寫而失敗。
--
-- items 存「整個 Inventory 序列化後的 JSON 字串」(形如 {"items":{"wood":3}})。用 TEXT 而非
-- JSONB 是為了不動 sqlx 既有 feature 集（未開 json feature）,與 positions 的 JSONL 退回層
-- 同一套「序列化成字串再存」思路。載入時一律過 `Inventory::is_loadable` 驗證,
-- 壞檔/被竄改的存檔（0 條目、超過堆疊上限）會被當空背包丟掉、不會把壞值帶進世界。
CREATE TABLE IF NOT EXISTS inventories (
    player_id  UUID PRIMARY KEY,
    items      TEXT        NOT NULL DEFAULT '{"items":{}}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
