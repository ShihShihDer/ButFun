//! NPC 作息與移動系統（ROADMAP 73）。
//!
//! NPC 有日/夜作息（模板/啟發式驅動，不每步呼叫 LLM），在世界裡移動。
//! 玩家看得到鎮上有人在活動，增加世界生命感。

use std::collections::HashMap;
use crate::daynight::{Phase, DayNight};

/// NPC 移動速度（像素 / 秒）。比玩家慢一些，看起來較悠閒。
pub const NPC_SPEED: f32 = 64.0;

/// NPC 位置定義。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pos {
    pub x: f32,
    pub y: f32,
}

impl Pos {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// NPC 作息定義。
#[derive(Debug, Clone)]
pub struct NpcSchedule {
    pub id: &'static str,
    /// 白天站崗位置（辦公點）。
    pub station_pos: Pos,
    /// 夜晚休憩位置（聚會點）。
    pub night_pos: Pos,
}

/// 預定義的村落 NPC 作息（與各系統現有硬編碼座標對齊）。
pub const VILLAGE_NPCS: &[NpcSchedule] = &[
    NpcSchedule {
        id: "merchant",
        station_pos: Pos::new(2120.0, 2328.0),
        night_pos: Pos::new(2400.0, 2200.0),
    },
    NpcSchedule {
        id: "workshop_npc",
        station_pos: Pos::new(2120.0, 2080.0),
        night_pos: Pos::new(2450.0, 2180.0),
    },
    NpcSchedule {
        id: "bounty_npc",
        station_pos: Pos::new(2240.0, 2080.0),
        night_pos: Pos::new(2500.0, 2200.0),
    },
    NpcSchedule {
        id: "expedition_npc",
        station_pos: Pos::new(2360.0, 2080.0),
        night_pos: Pos::new(2420.0, 2250.0),
    },
    NpcSchedule {
        id: "procurement_npc",
        station_pos: Pos::new(2480.0, 2080.0),
        night_pos: Pos::new(2380.0, 2180.0),
    },
    NpcSchedule {
        id: "farm_fair_npc",
        station_pos: Pos::new(2600.0, 2080.0),
        night_pos: Pos::new(2450.0, 2230.0),
    },
    NpcSchedule {
        id: "village_chief",
        station_pos: Pos::new(2720.0, 2080.0),
        night_pos: Pos::new(2720.0, 2080.0), // 里長守在原地（或是他的家）
    },
];

/// 運行中的 NPC 狀態。
#[derive(Debug, Clone)]
pub struct NpcState {
    pub id: String,
    pub x: f32,
    pub y: f32,
}

/// NPC 作息管理器。
pub struct NpcScheduleManager {
    /// 當前 NPC 狀態。
    pub npcs: HashMap<String, NpcState>,
}

impl NpcScheduleManager {
    pub fn new() -> Self {
        let mut npcs = HashMap::new();
        for s in VILLAGE_NPCS {
            npcs.insert(s.id.to_string(), NpcState {
                id: s.id.to_string(),
                x: s.station_pos.x,
                y: s.station_pos.y,
            });
        }
        Self { npcs }
    }

    /// 依據目前時刻推進 NPC 位置。
    pub fn tick(&mut self, dt: f32, daynight: &DayNight) {
        let phase = daynight.phase();
        
        for s in VILLAGE_NPCS {
            if let Some(state) = self.npcs.get_mut(s.id) {
                // 決定當前目標。
                let target = match phase {
                    Phase::Night => s.night_pos,
                    _ => s.station_pos, // 破曉、白天、黃昏都在崗位上
                };

                // 向目標移動。
                let dx = target.x - state.x;
                let dy = target.y - state.y;
                let dist_sq = dx * dx + dy * dy;
                
                if dist_sq > 1.0 {
                    let dist = dist_sq.sqrt();
                    let step = (NPC_SPEED * dt).min(dist);
                    state.x += (dx / dist) * step;
                    state.y += (dy / dist) * step;
                } else {
                    state.x = target.x;
                    state.y = target.y;
                }
            }
        }
    }

    /// 取得特定 NPC 位置。
    pub fn get_pos(&self, id: &str) -> Option<(f32, f32)> {
        self.npcs.get(id).map(|n| (state_to_pos(n)))
    }
}

fn state_to_pos(n: &NpcState) -> (f32, f32) {
    (n.x, n.y)
}

/// 取得 NPC 的崗位座標作為後備（`get_pos` 找不到時使用）。
/// 確保即使 NPC 尚未初始化，NpcSpeech 泡泡仍有合理的預設定位點。
pub fn fallback_pos(id: &str) -> (f32, f32) {
    VILLAGE_NPCS.iter()
        .find(|s| s.id == id)
        .map(|s| (s.station_pos.x, s.station_pos.y))
        .unwrap_or((2400.0, 2200.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daynight::DayNight;

    #[test]
    fn npcs_initialize_at_stations() {
        let mgr = NpcScheduleManager::new();
        let merchant = mgr.npcs.get("merchant").unwrap();
        assert_eq!(merchant.x, 2120.0);
        assert_eq!(merchant.y, 2328.0);
    }

    #[test]
    fn npcs_move_towards_night_pos_at_night() {
        let mut mgr = NpcScheduleManager::new();
        // 設為深夜
        let dn = DayNight::at(500.0); // Night starts at 0.65 * 600 = 390
        assert_eq!(dn.phase(), Phase::Night);

        let initial_x = mgr.npcs.get("merchant").unwrap().x;
        mgr.tick(1.0, &dn); // 走 1 秒
        let after_x = mgr.npcs.get("merchant").unwrap().x;
        
        assert!(after_x > initial_x, "商人在夜裡應該往右移 (2120 -> 2400)");
    }

    #[test]
    fn npcs_stay_at_target_when_reached() {
        let mut mgr = NpcScheduleManager::new();
        let dn = DayNight::new(); // Day
        mgr.tick(1.0, &dn);
        let merchant = mgr.npcs.get("merchant").unwrap();
        assert_eq!(merchant.x, 2120.0);
        assert_eq!(merchant.y, 2328.0);
    }

    #[test]
    fn fallback_pos_returns_station_for_known_npc() {
        let (x, y) = fallback_pos("merchant");
        assert_eq!(x, 2120.0);
        assert_eq!(y, 2328.0);
    }

    #[test]
    fn fallback_pos_returns_default_for_unknown_npc() {
        let (x, y) = fallback_pos("unknown_npc_xyz");
        // 預設廣場座標，保證不是 0
        assert!(x > 0.0 && y > 0.0, "未知 NPC 後備座標應有合理預設");
    }
}
