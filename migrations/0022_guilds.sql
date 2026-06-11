-- ROADMAP 113：工會持久化——工會跨伺服器重啟也不消失。
-- guilds 存公會主體（id, name, tag, founder_id, treasury）；guild_members 存成員清單。
-- 不設外鍵（與其他 store 一致：耐久層彼此獨立，耐 schema 漂移）。
CREATE TABLE IF NOT EXISTS guilds (
    id          UUID        PRIMARY KEY,
    name        TEXT        NOT NULL UNIQUE,
    tag         TEXT        NOT NULL UNIQUE,
    founder_id  UUID        NOT NULL,
    treasury    INTEGER     NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS guild_members (
    guild_id    UUID        NOT NULL,
    player_id   UUID        NOT NULL,
    joined_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, player_id)
);
CREATE INDEX IF NOT EXISTS idx_guild_members_player_id ON guild_members(player_id);
