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

/// 正午聚食時段（ROADMAP 327）：以日夜循環比例 `fraction`（[0,1)）界定的午休窗。
/// 落在白天階段（[0.15, 0.5)）的正中央、環繞日正當中（`PEAK_FRACTION` ≈ 0.325），
/// 約是一輪 10 分鐘日夜裡的第 3～3.6 分鐘——一段約一分鐘的午歇，看得出節律又不拖沓。
const LUNCH_START_FRACTION: f32 = 0.30;
const LUNCH_END_FRACTION: f32 = 0.40;

/// 鎮中廣場座標（ROADMAP 327）：正午七大 NPC 離開各自崗位、聚到這裡一起用餐的聚會點。
/// 取站崗那排（y≈2080）與商人攤（y≈2328）之間的中庭，落在店鋪群的視覺重心，
/// 玩家一眼看得出「大家都聚到鎮中央了」。刻意與任一夜宿點錯開，作為可辨識的鎮心。
pub const PLAZA_POS: Pos = Pos::new(2400.0, 2260.0);

/// 是否正值正午聚食時段。純函式、可測。
///
/// 只在「白天階段」且 `fraction` 落在午休窗內才成立——午休本就嵌在白天裡，
/// 破曉 / 黃昏 / 夜晚一律否（夜晚另有歸宿邏輯，優先於午休）。
/// `fraction` 非有限時保守回 `false`（不讓壞值把 NPC 誤導去廣場）。
pub fn is_lunch_time(phase: Phase, fraction: f32) -> bool {
    phase == Phase::Day
        && fraction.is_finite()
        && fraction >= LUNCH_START_FRACTION
        && fraction < LUNCH_END_FRACTION
}

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
    /// 正午聚食（ROADMAP 327）：已抵達鎮中廣場，七大 NPC 聚在一起用餐歇腳。
    Lunching,
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
            NpcActivity::Lunching => "lunching",
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
/// 3. 已就定位且正值正午聚食（`lunching`）→ `Lunching`（聚到鎮中廣場用餐，ROADMAP 327）。
/// 4. 已就定位、非夜晚也非午休（破曉 / 午前午後白天 / 黃昏都在崗位）→ 該職責專屬的工作姿態。
///
/// 未知 NPC（不在 `VILLAGE_NPCS`，例如其他星球商人 / 旅人 / 路人居民）在白天 / 午休回 `None`，
/// 不替它們安排工作 / 聚食狀態。
pub fn work_activity(id: &str, phase: Phase, lunching: bool, commuting: bool) -> Option<NpcActivity> {
    if commuting {
        return Some(NpcActivity::Commuting);
    }
    if phase == Phase::Night {
        // 里長守在原地（night_pos == station_pos），夜裡也算就地歇息。
        return Some(NpcActivity::Resting);
    }
    if lunching {
        // 正午聚食：只有故鄉七大 NPC 會離崗到廣場聚餐（其餘 NPC 不參與，回 None）。
        return if is_village_npc(id) {
            Some(NpcActivity::Lunching)
        } else {
            None
        };
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

/// 是否為故鄉七大 NPC（在 `VILLAGE_NPCS` 名單內）。
pub fn is_village_npc(id: &str) -> bool {
    VILLAGE_NPCS.iter().any(|s| s.id == id)
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
                activity: work_activity(s.id, Phase::Day, false, false),
            });
        }
        Self { npcs }
    }

    /// 依據目前時刻推進 NPC 位置。
    pub fn tick(&mut self, dt: f32, daynight: &DayNight) {
        let phase = daynight.phase();
        // 正午聚食判定（ROADMAP 327）：白天午休窗內，七大 NPC 離崗聚到鎮中廣場。
        let lunching = is_lunch_time(phase, daynight.fraction());

        for s in VILLAGE_NPCS {
            if let Some(state) = self.npcs.get_mut(s.id) {
                // 決定當前目標：夜晚回夜宿點、正午聚到廣場、其餘時段在崗位。
                let target = if phase == Phase::Night {
                    s.night_pos
                } else if lunching {
                    PLAZA_POS // 正午聚食：離崗走到鎮中廣場
                } else {
                    s.station_pos // 破曉、午前午後白天、黃昏都在崗位上
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

                // 重算工作 / 活動狀態（ROADMAP 324 + 327）：趕路中、夜間休憩、正午聚食、或白天各職責工作。
                state.activity = work_activity(s.id, phase, lunching, commuting);
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
        assert_eq!(work_activity("merchant", Phase::Day, false, true), Some(NpcActivity::Commuting));
        assert_eq!(work_activity("village_chief", Phase::Night, false, true), Some(NpcActivity::Commuting));
        // 連正午聚食時段也壓不過趕路：往廣場走的路上仍是 Commuting。
        assert_eq!(work_activity("merchant", Phase::Day, true, true), Some(NpcActivity::Commuting));
    }

    #[test]
    fn activity_resting_at_night_when_arrived() {
        // 已就定位的夜晚，所有故鄉 NPC 都休憩（含守在原地的里長）。
        for s in VILLAGE_NPCS {
            assert_eq!(
                work_activity(s.id, Phase::Night, false, false),
                Some(NpcActivity::Resting),
                "{} 夜裡就定位應休憩", s.id
            );
        }
    }

    #[test]
    fn activity_each_role_has_distinct_daytime_work() {
        // 白天就定位時，每個職責有自己專屬的工作姿態，彼此不撞。
        assert_eq!(work_activity("merchant", Phase::Day, false, false), Some(NpcActivity::Tallying));
        assert_eq!(work_activity("workshop_npc", Phase::Day, false, false), Some(NpcActivity::Hammering));
        assert_eq!(work_activity("bounty_npc", Phase::Day, false, false), Some(NpcActivity::Sharpening));
        assert_eq!(work_activity("expedition_npc", Phase::Day, false, false), Some(NpcActivity::Mapping));
        assert_eq!(work_activity("procurement_npc", Phase::Day, false, false), Some(NpcActivity::Stocktaking));
        assert_eq!(work_activity("farm_fair_npc", Phase::Day, false, false), Some(NpcActivity::Judging));
        assert_eq!(work_activity("village_chief", Phase::Day, false, false), Some(NpcActivity::Patrolling));
    }

    #[test]
    fn activity_dawn_and_dusk_also_work_at_station() {
        // 破曉與黃昏只要已就定位（非趕路），也是在崗位工作（不休憩）。
        assert_eq!(work_activity("merchant", Phase::Dawn, false, false), Some(NpcActivity::Tallying));
        assert_eq!(work_activity("merchant", Phase::Dusk, false, false), Some(NpcActivity::Tallying));
    }

    #[test]
    fn activity_none_for_unknown_npc() {
        // 不在 VILLAGE_NPCS 的 NPC（其他星球商人 / 旅人 / 居民）不安排工作狀態。
        assert_eq!(work_activity("verdant_merchant", Phase::Day, false, false), None);
        assert_eq!(work_activity("traveler", Phase::Day, false, false), None);
        assert_eq!(work_activity("resident_3", Phase::Day, false, false), None);
    }

    #[test]
    fn activity_codes_are_unique_and_stable() {
        // 前後端契約：每個活動代碼唯一、穩定。
        use std::collections::HashSet;
        let acts = [
            NpcActivity::Commuting, NpcActivity::Resting, NpcActivity::Lunching,
            NpcActivity::Tallying, NpcActivity::Hammering, NpcActivity::Sharpening,
            NpcActivity::Mapping, NpcActivity::Stocktaking, NpcActivity::Judging,
            NpcActivity::Patrolling,
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

    // —— ROADMAP 327：正午聚食 ——

    /// 取一個落在午休窗正中央（日正當中 ≈ 0.325）的 elapsed 秒數。
    fn noon_secs() -> f32 {
        0.325 * crate::daynight::DAY_LENGTH_SECS
    }

    #[test]
    fn is_lunch_time_only_in_window_during_day() {
        // 窗內（白天午休）→ true。
        assert!(is_lunch_time(Phase::Day, 0.30));
        assert!(is_lunch_time(Phase::Day, 0.35));
        // 窗外的白天（午前 / 午後）→ false。
        assert!(!is_lunch_time(Phase::Day, 0.20));
        assert!(!is_lunch_time(Phase::Day, 0.45));
        // 上界開區間：剛好 0.40 不算。
        assert!(!is_lunch_time(Phase::Day, 0.40));
    }

    #[test]
    fn is_lunch_time_false_outside_day_phase() {
        // 即便比例落在窗內，破曉 / 黃昏 / 夜晚都不聚食（午休只嵌在白天）。
        assert!(!is_lunch_time(Phase::Dawn, 0.32));
        assert!(!is_lunch_time(Phase::Dusk, 0.32));
        assert!(!is_lunch_time(Phase::Night, 0.32));
    }

    #[test]
    fn is_lunch_time_rejects_non_finite() {
        // 壞值（NaN / 無限）保守回 false，不把 NPC 誤導去廣場。
        assert!(!is_lunch_time(Phase::Day, f32::NAN));
        assert!(!is_lunch_time(Phase::Day, f32::INFINITY));
    }

    #[test]
    fn work_activity_lunching_for_village_npc() {
        // 白天午休、已就定位（非趕路）→ 七大 NPC 皆聚食。
        for s in VILLAGE_NPCS {
            assert_eq!(
                work_activity(s.id, Phase::Day, true, false),
                Some(NpcActivity::Lunching),
                "{} 正午就定位於廣場應聚食", s.id
            );
        }
    }

    #[test]
    fn work_activity_lunching_none_for_unknown() {
        // 非故鄉七大 NPC 不參與聚食。
        assert_eq!(work_activity("traveler", Phase::Day, true, false), None);
        assert_eq!(work_activity("resident_3", Phase::Day, true, false), None);
    }

    #[test]
    fn work_activity_night_overrides_lunch_flag() {
        // 夜晚優先於午休旗標（理論上不會同時成立，仍守防線）：回 Resting 而非 Lunching。
        assert_eq!(work_activity("merchant", Phase::Night, true, false), Some(NpcActivity::Resting));
    }

    #[test]
    fn is_village_npc_membership() {
        assert!(is_village_npc("merchant"));
        assert!(is_village_npc("village_chief"));
        assert!(!is_village_npc("traveler"));
        assert!(!is_village_npc("resident_1"));
    }

    #[test]
    fn manager_npcs_walk_to_plaza_and_lunch_at_noon() {
        // 正午時段：七大 NPC 會離崗走到廣場、抵達後活動切為 Lunching。
        let mut mgr = NpcScheduleManager::new();
        let dn = DayNight::at(noon_secs());
        assert_eq!(dn.phase(), Phase::Day);
        assert!(is_lunch_time(dn.phase(), dn.fraction()), "noon_secs 應落在午休窗內");
        // 走久一點讓所有 NPC 都抵達廣場（最遠約 360px，NPC_SPEED 64 → 約 6 秒）。
        for _ in 0..30 {
            mgr.tick(1.0, &dn);
        }
        for s in VILLAGE_NPCS {
            let (x, y) = mgr.get_pos(s.id).unwrap();
            assert!(
                (x - PLAZA_POS.x).abs() < 1.0 && (y - PLAZA_POS.y).abs() < 1.0,
                "{} 正午應抵達廣場 ({},{})，實際 ({},{})", s.id, PLAZA_POS.x, PLAZA_POS.y, x, y
            );
            assert_eq!(mgr.get_activity(s.id), Some(NpcActivity::Lunching), "{} 抵達廣場應聚食", s.id);
        }
    }

    #[test]
    fn manager_marks_commuting_on_the_way_to_plaza() {
        // 午休剛開始、還沒走到廣場時，活動是 Commuting。
        let mut mgr = NpcScheduleManager::new();
        let dn = DayNight::at(noon_secs());
        mgr.tick(0.1, &dn); // 只走一小步，必定還在去廣場的路上
        assert_eq!(mgr.get_activity("merchant"), Some(NpcActivity::Commuting));
    }

    #[test]
    fn manager_returns_to_station_after_lunch() {
        // 午休結束（午後白天）→ NPC 回崗位、活動恢復職責工作。
        let mut mgr = NpcScheduleManager::new();
        // 先在正午把商人帶到廣場。
        let noon = DayNight::at(noon_secs());
        for _ in 0..30 { mgr.tick(1.0, &noon); }
        assert_eq!(mgr.get_activity("merchant"), Some(NpcActivity::Lunching));
        // 切到午後（白天、午休窗外）。
        let afternoon = DayNight::at(0.45 * crate::daynight::DAY_LENGTH_SECS);
        assert_eq!(afternoon.phase(), Phase::Day);
        assert!(!is_lunch_time(afternoon.phase(), afternoon.fraction()));
        for _ in 0..30 { mgr.tick(1.0, &afternoon); }
        let (x, y) = mgr.get_pos("merchant").unwrap();
        assert!((x - 2120.0).abs() < 1.0 && (y - 2328.0).abs() < 1.0, "午後商人應回崗位");
        assert_eq!(mgr.get_activity("merchant"), Some(NpcActivity::Tallying));
    }
}
