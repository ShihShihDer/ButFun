//! ROADMAP 164：怪物巢穴=聚落——怪物從命名巢穴出生/回巢，
//! 族群可清剿衰退或放著壯大；與野生動物聚落同一設計哲學。
//!
//! 每個巢穴固定在世界座標，關聯一種怪物（EnemyKind）。
//! 玩家在巢穴附近（COLONY_KILL_RADIUS 像素內）擊殺同種怪物，族群數量下降；
//! 若族群歸零，巢穴暫時廢棄並加長冷卻；放著不管，每 RESPAWN_SECS 秒補充一隻。
//!
//! 效能：全純算術、零 LLM、零 migration；記憶體模式，重啟全重置。

use serde::Serialize;
use crate::combat::EnemyKind;

// ─── 常數 ────────────────────────────────────────────────────────────────────

/// 正常族群補充間隔（秒）：每 2 分鐘嘗試補充一隻。
const RESPAWN_SECS: f32 = 120.0;
/// 清剿後加長冷卻倍率：族群歸零後需等 3 倍才開始復生。
const WIPED_COOLDOWN_MULT: f32 = 3.0;
/// 玩家在此半徑（像素）內擊殺同類怪物，計入巢穴族群損失。
pub const COLONY_KILL_RADIUS: f32 = 420.0;

// ─── 型別 ────────────────────────────────────────────────────────────────────

/// 單個怪物巢穴。
pub struct MonsterColony {
    pub id: u32,
    pub kind: EnemyKind,
    /// 巢穴顯示名稱（繁中）。
    pub name: &'static str,
    /// 巢穴中心世界座標 X（像素）。
    pub cx: f32,
    /// 巢穴中心世界座標 Y（像素）。
    pub cy: f32,
    /// 怪物出生散佈半徑（像素）。
    pub spawn_radius: f32,
    /// 目前活躍族群數（0 = 巢穴暫時廢棄）。
    pub population: u32,
    /// 最大族群容量。
    pub max_population: u32,
    /// 下次嘗試補充的倒數計時器（秒）。
    spawn_timer: f32,
    /// 累計生成次數，用作出生點散佈的鹽值（確保分佈不重疊）。
    spawn_count: u32,
}

/// 給協議層用的巢穴視圖（隨快照廣播，讓玩家在地圖/態度面板看到巢穴）。
#[derive(Debug, Clone, Serialize)]
pub struct MonsterColonyView {
    pub id: u32,
    pub kind: String,
    pub name: String,
    pub cx: f32,
    pub cy: f32,
    pub spawn_radius: f32,
    /// 族群密度：0=廢棄 1=稀疏 2=正常 3=茂盛（讓玩家有感而不顯示精確數字）。
    pub density: u32,
}

/// 巢穴管理器發出的事件，由 game.rs 消化。
pub enum MonsterColonyEvent {
    /// 應在此座標注入一隻怪物（由 EnemyField::inject_event_enemy 執行）。
    SpawnAt { colony_id: u32, kind: EnemyKind, x: f32, y: f32 },
    /// 巢穴族群被清空（可廣播全服聊天）。
    ColonyCleared { name: &'static str, cx: f32, cy: f32 },
    /// 廢棄巢穴族群復生（可廣播全服聊天）。
    ColonyRevived { name: &'static str },
}

/// 管理所有怪物巢穴。
pub struct MonsterColonyManager {
    pub colonies: Vec<MonsterColony>,
}

impl MonsterColonyManager {
    pub fn new() -> Self {
        Self { colonies: build_colonies() }
    }

    /// 每幀推進：倒數補充計時器，到期且族群未滿則發出 SpawnAt 事件。
    pub fn tick(&mut self, dt: f32) -> Vec<MonsterColonyEvent> {
        let mut events = Vec::new();
        for col in &mut self.colonies {
            if col.population >= col.max_population {
                continue;
            }
            col.spawn_timer -= dt;
            if col.spawn_timer > 0.0 {
                continue;
            }
            col.spawn_timer = RESPAWN_SECS;
            let was_empty = col.population == 0;
            col.population += 1;
            col.spawn_count += 1;
            let (sx, sy) = colony_spawn_pos(col);
            events.push(MonsterColonyEvent::SpawnAt { colony_id: col.id, kind: col.kind, x: sx, y: sy });
            if was_empty {
                events.push(MonsterColonyEvent::ColonyRevived { name: col.name });
            }
        }
        events
    }

    /// 玩家在 (kill_x, kill_y) 擊殺了 kill_kind 種類的怪 →
    /// 找最近的同類巢穴（在 COLONY_KILL_RADIUS 內）並扣族群數。
    pub fn on_monster_killed_near(
        &mut self,
        kill_x: f32,
        kill_y: f32,
        kill_kind: EnemyKind,
    ) -> Vec<MonsterColonyEvent> {
        let mut events = Vec::new();
        let radius_sq = COLONY_KILL_RADIUS * COLONY_KILL_RADIUS;
        let mut best: Option<usize> = None;
        let mut best_dist_sq = radius_sq;
        for (idx, col) in self.colonies.iter().enumerate() {
            if col.kind != kill_kind || col.population == 0 {
                continue;
            }
            let dx = col.cx - kill_x;
            let dy = col.cy - kill_y;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best = Some(idx);
            }
        }
        if let Some(idx) = best {
            let col = &mut self.colonies[idx];
            col.population -= 1;
            if col.population == 0 {
                // 巢穴清空：加長冷卻再復生
                col.spawn_timer = RESPAWN_SECS * WIPED_COOLDOWN_MULT;
                events.push(MonsterColonyEvent::ColonyCleared { name: col.name, cx: col.cx, cy: col.cy });
            }
        }
        events
    }

    /// 回傳供快照廣播的視圖清單。
    pub fn colony_views(&self) -> Vec<MonsterColonyView> {
        self.colonies.iter().map(|col| MonsterColonyView {
            id:           col.id,
            kind:         col.kind.as_str().to_string(),
            name:         col.name.to_string(),
            cx:           col.cx,
            cy:           col.cy,
            spawn_radius: col.spawn_radius,
            density:      colony_density(col.population, col.max_population),
        }).collect()
    }
}

impl Default for MonsterColonyManager {
    fn default() -> Self { Self::new() }
}

// ─── 輔助函式 ─────────────────────────────────────────────────────────────────

/// 族群密度等級：0=廢棄 1=稀疏 2=正常 3=茂盛。
fn colony_density(pop: u32, max: u32) -> u32 {
    if pop == 0 || max == 0 { return 0; }
    let ratio = pop as f32 / max as f32;
    if ratio <= 0.33 { 1 } else if ratio <= 0.66 { 2 } else { 3 }
}

/// 依巢穴 id + spawn_count 決定性散佈出生位置（純函式，不隨機）。
fn colony_spawn_pos(col: &MonsterColony) -> (f32, f32) {
    let mut s = (col.id as u64).wrapping_mul(0x9E3779B97F4A7C15);
    s = s.wrapping_add((col.spawn_count as u64).wrapping_mul(0xBF58476D1CE4E5B9));
    s ^= s >> 30;
    s = s.wrapping_mul(0x94D049BB133111EB);
    s ^= s >> 27;
    // 角度均勻分佈，半徑 [0.2, 1.0] × spawn_radius
    let angle = (s & 0xFFFF) as f32 / 65535.0 * std::f32::consts::TAU;
    let r_frac = 0.2 + 0.8 * ((s >> 16 & 0xFFFF) as f32 / 65535.0);
    let r = col.spawn_radius * r_frac;
    (col.cx + r * angle.cos(), col.cy + r * angle.sin())
}

/// 世界座標巢穴列表（城外安全區外，分散四方供玩家探索）。
///
/// 城鎮中心像素 ≈ (2336, 2272)，安全區半徑 ≈ 1344px（42 格）。
/// 各巢穴均距城鎮中心 > 1500px，確保在安全區外。
fn build_colonies() -> Vec<MonsterColony> {
    vec![
        MonsterColony {
            id: 0, kind: EnemyKind::FlutterSprite,
            name: "靈蛾巢（東北荒野）",
            cx: 4000.0, cy: 1800.0, spawn_radius: 220.0,
            population: 5, max_population: 8,
            spawn_timer: RESPAWN_SECS, spawn_count: 0,
        },
        MonsterColony {
            id: 1, kind: EnemyKind::MushroomStalker,
            name: "蘑菇潛行窟（東南澤地）",
            cx: 3900.0, cy: 3200.0, spawn_radius: 240.0,
            population: 5, max_population: 7,
            spawn_timer: RESPAWN_SECS, spawn_count: 0,
        },
        MonsterColony {
            id: 2, kind: EnemyKind::ScrapDrone,
            name: "廢料無人機陣（南方廢墟）",
            cx: 2200.0, cy: 3900.0, spawn_radius: 200.0,
            population: 4, max_population: 6,
            spawn_timer: RESPAWN_SECS, spawn_count: 0,
        },
        MonsterColony {
            id: 3, kind: EnemyKind::CrystalGolem,
            name: "水晶魔像坑（西岸礦脈）",
            cx: 700.0, cy: 2400.0, spawn_radius: 260.0,
            population: 3, max_population: 5,
            spawn_timer: RESPAWN_SECS, spawn_count: 0,
        },
        MonsterColony {
            id: 4, kind: EnemyKind::EtherWisp,
            name: "乙太幽靈霧潭（西北霧區）",
            cx: 1100.0, cy: 800.0, spawn_radius: 210.0,
            population: 5, max_population: 7,
            spawn_timer: RESPAWN_SECS, spawn_count: 0,
        },
    ]
}

// ─── 測試 ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colony_ids_unique() {
        let mgr = MonsterColonyManager::new();
        let mut ids: Vec<u32> = mgr.colonies.iter().map(|c| c.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), mgr.colonies.len(), "巢穴 ID 必須唯一");
    }

    #[test]
    fn colonies_start_with_positive_population() {
        let mgr = MonsterColonyManager::new();
        for col in &mgr.colonies {
            assert!(col.population > 0, "巢穴 {} 初始族群應 > 0", col.name);
            assert!(col.population <= col.max_population, "族群不應超過上限");
        }
    }

    #[test]
    fn tick_spawns_when_below_max() {
        let mut mgr = MonsterColonyManager::new();
        // 清空第一個巢穴並讓計時歸零
        mgr.colonies[0].population = 0;
        mgr.colonies[0].spawn_timer = 0.0;
        let events = mgr.tick(0.1);
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::SpawnAt { .. })),
            "族群未滿且計時歸零應觸發 SpawnAt"
        );
    }

    #[test]
    fn tick_no_spawn_when_full() {
        let mut mgr = MonsterColonyManager::new();
        let col = &mut mgr.colonies[0];
        col.population = col.max_population;
        let events = mgr.tick(RESPAWN_SECS + 1.0);
        assert!(
            !events.iter().any(|e| matches!(e, MonsterColonyEvent::SpawnAt { colony_id: 0, .. })),
            "族群已滿不應觸發 SpawnAt"
        );
    }

    #[test]
    fn tick_emits_revived_when_empty_colony_respawns() {
        let mut mgr = MonsterColonyManager::new();
        mgr.colonies[0].population = 0;
        mgr.colonies[0].spawn_timer = 0.0;
        let events = mgr.tick(0.1);
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::ColonyRevived { .. })),
            "廢棄巢穴復生應發出 ColonyRevived"
        );
    }

    #[test]
    fn kill_near_reduces_population() {
        let mut mgr = MonsterColonyManager::new();
        let col = &mgr.colonies[0];
        let (cx, cy, kind, initial) = (col.cx, col.cy, col.kind, col.population);
        mgr.on_monster_killed_near(cx, cy, kind);
        assert_eq!(mgr.colonies[0].population, initial - 1);
    }

    #[test]
    fn kill_different_kind_does_not_affect_colony() {
        let mut mgr = MonsterColonyManager::new();
        let col = &mgr.colonies[0]; // FlutterSprite
        let (cx, cy) = (col.cx, col.cy);
        let initial = col.population;
        // 在巢穴中心殺其他種怪，不應影響
        mgr.on_monster_killed_near(cx, cy, EnemyKind::CrystalGolem);
        assert_eq!(mgr.colonies[0].population, initial, "殺不同種怪不影響此巢穴");
    }

    #[test]
    fn kill_far_does_not_reduce_population() {
        let mut mgr = MonsterColonyManager::new();
        let kind = mgr.colonies[0].kind;
        let initial = mgr.colonies[0].population;
        // (0, 0) 距所有巢穴均遠超 COLONY_KILL_RADIUS
        mgr.on_monster_killed_near(0.0, 0.0, kind);
        assert_eq!(mgr.colonies[0].population, initial, "距離超出半徑不應扣族群");
    }

    #[test]
    fn wiping_colony_emits_cleared_event() {
        let mut mgr = MonsterColonyManager::new();
        mgr.colonies[0].population = 1;
        let (cx, cy, kind) = (mgr.colonies[0].cx, mgr.colonies[0].cy, mgr.colonies[0].kind);
        let events = mgr.on_monster_killed_near(cx, cy, kind);
        assert!(
            events.iter().any(|e| matches!(e, MonsterColonyEvent::ColonyCleared { .. })),
            "族群清空應發出 ColonyCleared"
        );
        assert_eq!(mgr.colonies[0].population, 0);
    }

    #[test]
    fn wiped_colony_has_longer_respawn_timer() {
        let mut mgr = MonsterColonyManager::new();
        mgr.colonies[0].population = 1;
        let (cx, cy, kind) = (mgr.colonies[0].cx, mgr.colonies[0].cy, mgr.colonies[0].kind);
        mgr.on_monster_killed_near(cx, cy, kind);
        assert!(
            mgr.colonies[0].spawn_timer > RESPAWN_SECS,
            "清剿後補充計時器應比正常更長"
        );
    }

    #[test]
    fn colony_views_count_matches() {
        let mgr = MonsterColonyManager::new();
        assert_eq!(mgr.colony_views().len(), mgr.colonies.len());
    }

    #[test]
    fn density_levels() {
        assert_eq!(colony_density(0, 8), 0, "族群 0 = 廢棄");
        assert_eq!(colony_density(1, 8), 1, "1/8 ≤ 33% → 稀疏");
        assert_eq!(colony_density(4, 8), 2, "4/8 = 50% → 正常");
        assert_eq!(colony_density(7, 8), 3, "7/8 > 66% → 茂盛");
        assert_eq!(colony_density(8, 8), 3, "8/8 = 100% → 茂盛");
    }

    #[test]
    fn spawn_pos_within_radius() {
        let mgr = MonsterColonyManager::new();
        for col in &mgr.colonies {
            let (sx, sy) = colony_spawn_pos(col);
            let dx = sx - col.cx;
            let dy = sy - col.cy;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(
                dist <= col.spawn_radius,
                "巢穴 {} 出生點距離 {} > spawn_radius {}",
                col.name, dist, col.spawn_radius
            );
        }
    }

    #[test]
    fn colony_views_density_reflects_population() {
        let mgr = MonsterColonyManager::new();
        for view in mgr.colony_views() {
            assert!(view.density <= 3, "密度等級應在 0~3 之間");
        }
    }
}
