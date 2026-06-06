-- Phase 0-E：玩家農地持久化（跨伺服器重啟保住種田進度：翻土／播種／澆水／成長）。
-- 採集/打怪/收成三來源灌背包已接 PG（0002）；農地是同一個缺口的另一半——種了一半的地
-- 同樣撐不過換版重啟。這張表讓已登入玩家的整塊地跟著重啟回來，對齊 BACKLOG 0-E
-- 「一次一個 store 接 PG」（背包之後輪到農地）。
--
-- 與 players / inventories 同樣只記「已登入」玩家（穩定 id）；訪客 id 每次隨機、不分地。
-- 刻意不對 players 表設外鍵：農地 flush 與位置/背包 flush 各自獨立、無寫入順序耦合，
-- 任一方先落地都不該因另一方還沒寫而失敗（理由同 0002_inventories.sql）。
--
-- plot_index 是「這塊地的序號」（由 PlotRegistry 分配、餵 plots::plot_origin 定位）。必須一起
-- 存：Field 的 origin 不入存檔（見 field.rs），載回時靠序號用 `Field::reseated` 安置回正確
-- 位置；序號本身也餵 `PlotRegistry::from_saved` 重建歸屬，確保重啟後續發序號不撞既有地塊。
-- 對 plot_index 設 UNIQUE：一塊地一個地主、序號不重疊（與記憶體層「序號只增不減、互異」對齊）。
--
-- tiles 存「整塊 Field 序列化後的 JSON 字串」（形如 {"tiles":[...]}，origin 因 serde skip 不入）。
-- 用 TEXT 而非 JSONB，與 inventories / positions 同一套「序列化成字串再存」思路，不動 sqlx
-- feature 集。載入時逐列過 `Field::reseated`（格數正確 + 每株作物健全）雙重驗證，壞檔／被竄改
-- （格數錯、作物 NaN/Inf/負成長）整列丟棄、不把壞值帶進世界。
CREATE TABLE IF NOT EXISTS fields (
    player_id  UUID PRIMARY KEY,
    plot_index INTEGER     NOT NULL,
    tiles      TEXT        NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS fields_plot_index_key ON fields (plot_index);
