-- ROADMAP 131: 城鎮大工程 — 建設蒸汽天文台
CREATE TABLE IF NOT EXISTS town_project (
    project_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    status TEXT NOT NULL, -- 'planning', 'building', 'completed'
    target_ether INTEGER NOT NULL DEFAULT 0,
    current_ether INTEGER NOT NULL DEFAULT 0,
    target_wood INTEGER NOT NULL DEFAULT 0,
    current_wood INTEGER NOT NULL DEFAULT 0,
    target_stone INTEGER NOT NULL DEFAULT 0,
    current_stone INTEGER NOT NULL DEFAULT 0,
    target_crystal INTEGER NOT NULL DEFAULT 0,
    current_crystal INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS town_project_donations (
    user_id UUID NOT NULL,
    project_id TEXT NOT NULL,
    ether_donated INTEGER NOT NULL DEFAULT 0,
    wood_donated INTEGER NOT NULL DEFAULT 0,
    stone_donated INTEGER NOT NULL DEFAULT 0,
    crystal_donated INTEGER NOT NULL DEFAULT 0,
    total_score INTEGER NOT NULL DEFAULT 0, -- 用於貢獻排行榜
    PRIMARY KEY (user_id, project_id)
);

-- 初始工程：蒸汽天文台
INSERT INTO town_project (project_id, name, status, target_ether, target_wood, target_stone, target_crystal)
VALUES ('observatory', '蒸汽天文台', 'building', 10000, 500, 500, 200)
ON CONFLICT DO NOTHING;
