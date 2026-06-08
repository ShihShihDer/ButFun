-- 玩家農地擴張格數（PlotWallet.expansions）持久化。
-- DEFAULT 0 表示既有玩家視為尚未擴張，向後相容。
ALTER TABLE players ADD COLUMN IF NOT EXISTS wallet_expansions BIGINT NOT NULL DEFAULT 0;
