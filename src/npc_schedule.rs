//! NPC 作息與移動系統（ROADMAP 73）。
//!
//! NPC 有日/夜作息（模板/啟發式驅動，不每步呼叫 LLM），在世界裡移動。
//! 玩家看得到鎮上有人在活動，增加世界生命感。

use std::collections::HashMap;
use crate::daynight::{Phase, DayNight};

/// NPC 移動速度（像素 / 秒）。比玩家慢一些，看起來較悠閒。
pub const NPC_SPEED: f32 = 64.0;

/// 判定 NPC「仍在趕路」的距離平方閾值（像素²）：離目標點還有這麼遠就算 Commuting。
/// 與 `tick` 裡「已抵達」的判定（`dist_sq <= 1.0`）對齊，避免抵達後仍誤判趕路。
const ARRIVED_EPS_SQ: f32 = 1.0;

/// NPC 當前的工作 / 活動狀態（ROADMAP 324）。
///
/// 純由「職責 × 作息時段 × 是否在趕路」推導，零 LLM、零隨機；前端據此在 NPC 頭頂
/// 浮現一枚活動符號，讓玩家進城一眼看出「鎮上每個人都在忙自己的事」，而非呆站的對話樁。
/// 與既有移動邏輯連動：夜裡都回去歇、清晨還在趕路、白天各司其職——城鎮看起來真的在運作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpcActivity {
    /// 趕路中（尚未抵達當前目標點，例如破曉從夜宿點走回崗位）。
    Commuting,
    /// 夜間休憩（已抵達夜宿聚會點）。
    Resting,
    /// 商人點算貨銀。
    Tallying,
    /// 工匠敲打鍛造。
    Hammering,
    /// 獵手擦拭、上弦整備。
    Sharpening,
    /// 探勘員端詳地圖。
    Mapping,
    /// 採購代理清點備貨。
    Stocktaking,
    /// 評審埋頭打分。
    Judging,
    /// 里長巡視村務。
    Patrolling,
}

impl NpcActivity {
    /// 穩定的字串代碼（snake_case），作為前後端契約傳給前端對應到頭頂符號。
    /// 面向玩家的文字 / emoji 一律由前端決定，後端只傳語意代碼，保留 i18n 空間。
    pub fn code(self) -> &'static str {
        match self {
            NpcActivity::Commuting => "commuting",
            NpcActivity::Resting => "resting",
            NpcActivity::Tallying => "tallying",
            NpcActivity::Hammering => "hammering",
            NpcActivity::Sharpening => "sharpening",
            NpcActivity::Mapping => "mapping",
            NpcActivity::Stocktaking => "stocktaking",
            NpcActivity::Judging => "judging",
            NpcActivity::Patrolling => "patrolling",
        }
    }
}

/// 依「NPC 職責 × 作息時段 × 是否在趕路」推導當前活動。純函式、可測。
///
/// 規則（由急到緩）：
/// 1. 還在趕路（`commuting`）→ 一律 `Commuting`（不管時段、職責）。
/// 2. 已就定位但在夜晚 → `Resting`（回夜宿點歇息）。
/// 3. 已就定位且非夜晚（破曉 / 白天 / 黃昏都在崗位）→ 該職責專屬的工作姿態。
///
/// 未知 NPC（不在 `VILLAGE_NPCS`，例如其他星球商人 / 旅人 / 路人居民）回 `None`，
/// 不替它們安排工作狀態。
pub fn work_activity(id: &str, phase: Phase, commuting: bool) -> Option<NpcActivity> {
    if commuting {
        return Some(NpcActivity::Commuting);
    }
    if phase == Phase::Night {
        // 里長守在原地（night_pos == station_pos），夜裡也算就地歇息。
        return Some(NpcActivity::Resting);
    }
    let act = match id {
        "merchant" => NpcActivity::Tallying,
        "workshop_npc" => NpcActivity::Hammering,
        "bounty_npc" => NpcActivity::Sharpening,
        "expedition_npc" => NpcActivity::Mapping,
        "procurement_npc" => NpcActivity::Stocktaking,
        "farm_fair_npc" => NpcActivity::Judging,
        "village_chief" => NpcActivity::Patrolling,
        _ => return None,
    };
    Some(act)
}

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
    /// 當前工作 / 活動狀態（ROADMAP 324），每幀依時段與是否抵達崗位重算。
    pub activity: Option<NpcActivity>,
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
                // 初始就在崗位上，依職責給定白天工作狀態（破曉前的 new() 也合理）。
                activity: work_activity(s.id, Phase::Day, false),
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
                
                let commuting = dist_sq > ARRIVED_EPS_SQ;
                if commuting {
                    let dist = dist_sq.sqrt();
                    let step = (NPC_SPEED * dt).min(dist);
                    state.x += (dx / dist) * step;
                    state.y += (dy / dist) * step;
                } else {
                    state.x = target.x;
                    state.y = target.y;
                }

                // 重算工作 / 活動狀態（ROADMAP 324）：趕路中、夜間休憩、或白天各職責工作。
                state.activity = work_activity(s.id, phase, commuting);
            }
        }
    }

    /// 取得特定 NPC 位置。
    pub fn get_pos(&self, id: &str) -> Option<(f32, f32)> {
        self.npcs.get(id).map(|n| (state_to_pos(n)))
    }

    /// 取得特定 NPC 的當前工作 / 活動狀態（ROADMAP 324）。
    pub fn get_activity(&self, id: &str) -> Option<NpcActivity> {
        self.npcs.get(id).and_then(|n| n.activity)
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

    // —— ROADMAP 324：NPC 工作 / 活動狀態 ——

    #[test]
    fn activity_commuting_overrides_everything() {
        // 還在趕路時，不管時段與職責，一律 Commuting。
        assert_eq!(work_activity("merchant", Phase::Day, true), Some(NpcActivity::Commuting));
        assert_eq!(work_activity("village_chief", Phase::Night, true), Some(NpcActivity::Commuting));
    }

    #[test]
    fn activity_resting_at_night_when_arrived() {
        // 已就定位的夜晚，所有故鄉 NPC 都休憩（含守在原地的里長）。
        for s in VILLAGE_NPCS {
            assert_eq!(
                work_activity(s.id, Phase::Night, false),
                Some(NpcActivity::Resting),
                "{} 夜裡就定位應休憩", s.id
            );
        }
    }

    #[test]
    fn activity_each_role_has_distinct_daytime_work() {
        // 白天就定位時，每個職責有自己專屬的工作姿態，彼此不撞。
        assert_eq!(work_activity("merchant", Phase::Day, false), Some(NpcActivity::Tallying));
        assert_eq!(work_activity("workshop_npc", Phase::Day, false), Some(NpcActivity::Hammering));
        assert_eq!(work_activity("bounty_npc", Phase::Day, false), Some(NpcActivity::Sharpening));
        assert_eq!(work_activity("expedition_npc", Phase::Day, false), Some(NpcActivity::Mapping));
        assert_eq!(work_activity("procurement_npc", Phase::Day, false), Some(NpcActivity::Stocktaking));
        assert_eq!(work_activity("farm_fair_npc", Phase::Day, false), Some(NpcActivity::Judging));
        assert_eq!(work_activity("village_chief", Phase::Day, false), Some(NpcActivity::Patrolling));
    }

    #[test]
    fn activity_dawn_and_dusk_also_work_at_station() {
        // 破曉與黃昏只要已就定位（非趕路），也是在崗位工作（不休憩）。
        assert_eq!(work_activity("merchant", Phase::Dawn, false), Some(NpcActivity::Tallying));
        assert_eq!(work_activity("merchant", Phase::Dusk, false), Some(NpcActivity::Tallying));
    }

    #[test]
    fn activity_none_for_unknown_npc() {
        // 不在 VILLAGE_NPCS 的 NPC（其他星球商人 / 旅人 / 居民）不安排工作狀態。
        assert_eq!(work_activity("verdant_merchant", Phase::Day, false), None);
        assert_eq!(work_activity("traveler", Phase::Day, false), None);
        assert_eq!(work_activity("resident_3", Phase::Day, false), None);
    }

    #[test]
    fn activity_codes_are_unique_and_stable() {
        // 前後端契約：每個活動代碼唯一、穩定。
        use std::collections::HashSet;
        let acts = [
            NpcActivity::Commuting, NpcActivity::Resting, NpcActivity::Tallying,
            NpcActivity::Hammering, NpcActivity::Sharpening, NpcActivity::Mapping,
            NpcActivity::Stocktaking, NpcActivity::Judging, NpcActivity::Patrolling,
        ];
        let codes: HashSet<&str> = acts.iter().map(|a| a.code()).collect();
        assert_eq!(codes.len(), acts.len(), "活動代碼必須唯一");
    }

    #[test]
    fn manager_initializes_with_work_activity() {
        // 管理器一建好，已就崗位的 NPC 就帶白天工作狀態。
        let mgr = NpcScheduleManager::new();
        assert_eq!(mgr.get_activity("merchant"), Some(NpcActivity::Tallying));
        assert_eq!(mgr.get_activity("village_chief"), Some(NpcActivity::Patrolling));
    }

    #[test]
    fn manager_tick_sets_resting_at_night_after_arrival() {
        // 夜裡走到夜宿點後，活動切為 Resting。
        let mut mgr = NpcScheduleManager::new();
        let dn = DayNight::at(500.0); // Night
        assert_eq!(dn.phase(), Phase::Night);
        // 走久一點讓所有 NPC 都抵達夜宿點。
        for _ in 0..30 {
            mgr.tick(1.0, &dn);
        }
        assert_eq!(mgr.get_activity("merchant"), Some(NpcActivity::Resting));
    }

    #[test]
    fn manager_tick_marks_commuting_while_moving() {
        // 夜裡剛起步、還沒走到夜宿點時，活動是 Commuting。
        let mut mgr = NpcScheduleManager::new();
        let dn = DayNight::at(500.0); // Night
        mgr.tick(0.1, &dn); // 只走一小步，必定還在路上（merchant 2120→2400）
        assert_eq!(mgr.get_activity("merchant"), Some(NpcActivity::Commuting));
    }

    #[test]
    fn manager_get_activity_none_for_unknown() {
        let mgr = NpcScheduleManager::new();
        assert_eq!(mgr.get_activity("nobody"), None);
    }
}
