-- Phase ③ 後續：無限地圖「全清重置」（使用者 2026-06-08 明確授權）。
-- 動機：世界重生、大家從零開始攢；clean slate，不留舊資料包袱、不用一直閃舊狀態。
--
-- 清空（所有玩家遊戲狀態）：
--   fields       — 地塊 / 作物
--   inventories  — 背包
--   players      — 位置 + 乙太
-- 保留：
--   users        — 帳號 / 登入身分（清掉玩家就登不回來，務必保留）
--   suggestions  — 玩家回饋
--   daynight     — 世界時鐘（singleton）
--
-- ⚠️ 不可逆：staging（butfun_staging）會在自動部署時套用；prod（butfun）只在人工
-- 執行 scripts/deploy.sh 換版時才套用。清空前先備份到 *_fullclean_backup 當安全網。
-- 全庫無外鍵，TRUNCATE 三表安全、不會 CASCADE 誤清其他表。

CREATE TABLE IF NOT EXISTS fields_fullclean_backup      AS SELECT * FROM fields;
CREATE TABLE IF NOT EXISTS inventories_fullclean_backup AS SELECT * FROM inventories;
CREATE TABLE IF NOT EXISTS players_fullclean_backup     AS SELECT * FROM players;

TRUNCATE TABLE fields, inventories, players;
