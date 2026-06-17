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

/// 圍桌共食的座位環半徑（像素，ROADMAP 328）：正午七大 NPC 不再全擠在 `PLAZA_POS` 同一點
/// 互相重疊，而是各自落在環繞鎮心一圈的座位上，看起來像圍著一桌坐下用餐。
const LUNCH_SEAT_RADIUS: f32 = 64.0;

/// 黃昏串門子（ROADMAP 356）：訪客 NPC 走到主人崗位旁站定的橫向偏移（像素）。
/// 不直接踩到主人身上，而是並肩站在攤前一點，看起來像兩人湊在一塊寒暄。
const VISIT_BESIDE_OFFSET: f32 = 56.0;

/// 取得某 NPC 正午聚食時的「座位」座標（ROADMAP 328）。
///
/// 七大 NPC 依其在 `VILLAGE_NPCS` 中的固定次序，等角分佈在以 `PLAZA_POS` 為圓心、
/// `LUNCH_SEAT_RADIUS` 為半徑的座位環上，彼此錯開、不再疊在同一點。純函式、可測：
/// 同一 id 永遠回同一座位、各 NPC 座位互異。非村落 NPC（不在名單內）回鎮心本身作後備。
///
/// 角度與座位偏移皆為編譯期可算的常數三角值，避免在熱路徑每幀重算；此處用查表近似
/// （七等分圓周）讓座位穩定、可重現，且不引入浮點不確定性。
pub fn lunch_seat(id: &str) -> Pos {
    // 找出該 NPC 在權威次序中的索引；找不到（非村落 NPC）則回鎮心。
    let Some(idx) = VILLAGE_NPCS.iter().position(|s| s.id == id) else {
        return PLAZA_POS;
    };
    let n = VILLAGE_NPCS.len();
    // 七等分圓周的單位方向（cos, sin），預先算好避免熱路徑三角運算與浮點不確定性。
    // 由 i 從 0 起、每格 2π/7 ≈ 51.43°，第一個座位朝正上方（-y 為上）。
    let (ux, uy) = unit_dir_seven(idx % n);
    Pos::new(
        PLAZA_POS.x + ux * LUNCH_SEAT_RADIUS,
        PLAZA_POS.y + uy * LUNCH_SEAT_RADIUS,
    )
}

/// 七等分圓周第 `i` 個座位的單位方向（cos, sin）。
/// 第 0 個朝正上方，順時針排開；查表避免熱路徑三角運算、保證可重現。
fn unit_dir_seven(i: usize) -> (f32, f32) {
    // 角度（度）：-90（正上）起，每格 +360/7 ≈ 51.4286°，逐一硬編 cos/sin。
    const DIRS: [(f32, f32); 7] = [
        (0.0, -1.0),       // -90°：正上
        (0.7818, -0.6235), // -38.57°
        (0.9749, 0.2225),  //  12.86°
        (0.4339, 0.9009),  //  64.29°
        (-0.4339, 0.9009), // 115.71°
        (-0.9749, 0.2225), // 167.14°
        (-0.7818, -0.6235),// 218.57°（≈ -141.43°）
    ];
    DIRS[i % 7]
}

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
    /// 黃昏串門子（ROADMAP 356）：與盟友結伴——黃昏時離崗走到結盟 NPC 的攤前寒暄。
    Visiting,
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
            NpcActivity::Visiting => "visiting",
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

/// 取得某故鄉 NPC 的白天崗位座標（不在名單內回 `None`）。純查表、可測。
pub fn station_pos(id: &str) -> Option<Pos> {
    VILLAGE_NPCS.iter().find(|s| s.id == id).map(|s| s.station_pos)
}

/// 黃昏串門子計畫（ROADMAP 356）：把「當前結盟配對」轉成「誰去拜訪誰」的對應表。純函式、可測。
///
/// 輸入 `alliances`：當下結盟的 NPC 有序對（取自 355 `current_standings` 過濾出 `Alliance`，
/// 已照「最鐵的盟友排前」排好）。輸出：`visitor_id → host_id`。
///
/// 規則（確定性、無隨機）：
/// - 只結盟對才串門子（敵對／中性不來往，呼叫端已過濾）。
/// - 依傳入次序貪婪挑選：每個 NPC 至多參與一段串門關係（當訪客**或**當主人其一），
///   已被佔用的 NPC 出現在後續配對裡就跳過——確保兩人不對撞、全鎮錯開、誰也不會分身兩地。
/// - 里長（`village_chief`）守在鎮上不外出串門，只當「被拜訪的主人」（延續既有「里長守在原地」設定）。
/// - 其餘配對固定取 id 字典序較小者當訪客、較大者當主人（與輸入排序無關，結果穩定可重現）。
pub fn dusk_visit_plan<'a>(alliances: &[(&'a str, &'a str)]) -> HashMap<&'a str, &'a str> {
    let mut plan: HashMap<&'a str, &'a str> = HashMap::new();
    let mut busy: std::collections::HashSet<&'a str> = std::collections::HashSet::new();
    for &(a, b) in alliances {
        // 任一方已在別段串門關係中（當訪客或主人）→ 跳過，避免分身／對撞。
        if busy.contains(a) || busy.contains(b) {
            continue;
        }
        // 決定主人與訪客：里長一律當主人；否則 id 字典序小者當訪客。
        let (visitor, host) = if a == "village_chief" {
            (b, a)
        } else if b == "village_chief" {
            (a, b)
        } else if a <= b {
            (a, b)
        } else {
            (b, a)
        };
        plan.insert(visitor, host);
        busy.insert(visitor);
        busy.insert(host);
    }
    plan
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
    ///
    /// `visits`（ROADMAP 356 黃昏串門子）：`visitor_id → host_id` 的計畫——黃昏時，名單上的訪客 NPC
    /// 離崗走到結盟主人 NPC 的攤前並肩寒暄。空 map（非黃昏或無結盟）時退回原本作息，行為完全不變、向後相容。
    pub fn tick(&mut self, dt: f32, daynight: &DayNight, visits: &HashMap<&str, &str>) {
        let phase = daynight.phase();
        // 正午聚食判定（ROADMAP 327）：白天午休窗內，七大 NPC 離崗聚到鎮中廣場。
        let lunching = is_lunch_time(phase, daynight.fraction());
        // 串門子只在黃昏發生；其餘時段一律當作沒有串門計畫（即使誤傳 visits 也不生效，邊界安全）。
        let visiting_window = phase == Phase::Dusk;

        for s in VILLAGE_NPCS {
            if let Some(state) = self.npcs.get_mut(s.id) {
                // 黃昏串門子：若此 NPC 是訪客且能查到主人崗位，目標改為主人攤前（並肩偏移，不重疊）。
                let visit_target = if visiting_window {
                    visits
                        .get(s.id)
                        .and_then(|host| station_pos(host))
                        .map(|host_station| Pos::new(host_station.x + VISIT_BESIDE_OFFSET, host_station.y))
                } else {
                    None
                };
                let visiting = visit_target.is_some();

                // 決定當前目標：黃昏串門 > 夜晚回夜宿點 > 正午聚到廣場 > 其餘時段在崗位。
                let target = if let Some(vt) = visit_target {
                    vt
                } else if phase == Phase::Night {
                    s.night_pos
                } else if lunching {
                    lunch_seat(s.id) // 正午聚食：離崗走到鎮中廣場的專屬座位（ROADMAP 328 圍桌錯開）
                } else {
                    s.station_pos // 破曉、午前午後白天在崗位上（黃昏未被指派串門者亦在崗位）
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
                let mut activity = work_activity(s.id, phase, lunching, commuting);
                // ROADMAP 356：已抵達盟友攤前（黃昏串門、非趕路中）→ 覆寫成「串門寒暄」。
                if visiting && !commuting {
                    activity = Some(NpcActivity::Visiting);
                }
                state.activity = activity;
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
        mgr.tick(1.0, &dn, &HashMap::new()); // 走 1 秒
        let after_x = mgr.npcs.get("merchant").unwrap().x;
        
        assert!(after_x > initial_x, "商人在夜裡應該往右移 (2120 -> 2400)");
    }

    #[test]
    fn npcs_stay_at_target_when_reached() {
        let mut mgr = NpcScheduleManager::new();
        let dn = DayNight::new(); // Day
        mgr.tick(1.0, &dn, &HashMap::new());
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
            mgr.tick(1.0, &dn, &HashMap::new());
        }
        assert_eq!(mgr.get_activity("merchant"), Some(NpcActivity::Resting));
    }

    #[test]
    fn manager_tick_marks_commuting_while_moving() {
        // 夜裡剛起步、還沒走到夜宿點時，活動是 Commuting。
        let mut mgr = NpcScheduleManager::new();
        let dn = DayNight::at(500.0); // Night
        mgr.tick(0.1, &dn, &HashMap::new()); // 只走一小步，必定還在路上（merchant 2120→2400）
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
            mgr.tick(1.0, &dn, &HashMap::new());
        }
        for s in VILLAGE_NPCS {
            let (x, y) = mgr.get_pos(s.id).unwrap();
            // ROADMAP 328：抵達的是各自的座位（環繞鎮心一圈），而非同一點。
            let seat = lunch_seat(s.id);
            assert!(
                (x - seat.x).abs() < 1.0 && (y - seat.y).abs() < 1.0,
                "{} 正午應抵達座位 ({},{})，實際 ({},{})", s.id, seat.x, seat.y, x, y
            );
            // 座位仍落在鎮心一圈內（離 PLAZA_POS 約一個座位環半徑）。
            let d = ((x - PLAZA_POS.x).powi(2) + (y - PLAZA_POS.y).powi(2)).sqrt();
            assert!(d <= LUNCH_SEAT_RADIUS + 1.0, "{} 座位應在鎮心一圈內", s.id);
            assert_eq!(mgr.get_activity(s.id), Some(NpcActivity::Lunching), "{} 抵達座位應聚食", s.id);
        }
    }

    #[test]
    fn manager_marks_commuting_on_the_way_to_plaza() {
        // 午休剛開始、還沒走到廣場時，活動是 Commuting。
        let mut mgr = NpcScheduleManager::new();
        let dn = DayNight::at(noon_secs());
        mgr.tick(0.1, &dn, &HashMap::new()); // 只走一小步，必定還在去廣場的路上
        assert_eq!(mgr.get_activity("merchant"), Some(NpcActivity::Commuting));
    }

    #[test]
    fn manager_returns_to_station_after_lunch() {
        // 午休結束（午後白天）→ NPC 回崗位、活動恢復職責工作。
        let mut mgr = NpcScheduleManager::new();
        // 先在正午把商人帶到廣場。
        let noon = DayNight::at(noon_secs());
        for _ in 0..30 { mgr.tick(1.0, &noon, &HashMap::new()); }
        assert_eq!(mgr.get_activity("merchant"), Some(NpcActivity::Lunching));
        // 切到午後（白天、午休窗外）。
        let afternoon = DayNight::at(0.45 * crate::daynight::DAY_LENGTH_SECS);
        assert_eq!(afternoon.phase(), Phase::Day);
        assert!(!is_lunch_time(afternoon.phase(), afternoon.fraction()));
        for _ in 0..30 { mgr.tick(1.0, &afternoon, &HashMap::new()); }
        let (x, y) = mgr.get_pos("merchant").unwrap();
        assert!((x - 2120.0).abs() < 1.0 && (y - 2328.0).abs() < 1.0, "午後商人應回崗位");
        assert_eq!(mgr.get_activity("merchant"), Some(NpcActivity::Tallying));
    }

    #[test]
    fn lunch_seats_are_distinct_for_all_village_npcs() {
        // ROADMAP 328：七大 NPC 的座位互不相同（不再疊在同一點）。
        let seats: Vec<Pos> = VILLAGE_NPCS.iter().map(|s| lunch_seat(s.id)).collect();
        for i in 0..seats.len() {
            for j in (i + 1)..seats.len() {
                let close = (seats[i].x - seats[j].x).abs() < 1.0
                    && (seats[i].y - seats[j].y).abs() < 1.0;
                assert!(
                    !close,
                    "{} 與 {} 的座位不應重疊", VILLAGE_NPCS[i].id, VILLAGE_NPCS[j].id
                );
            }
        }
    }

    #[test]
    fn lunch_seats_ring_around_plaza_center() {
        // 每個座位都落在以鎮心為圓心、座位環半徑的圓上（誤差容忍查表近似）。
        for s in VILLAGE_NPCS {
            let seat = lunch_seat(s.id);
            let d = ((seat.x - PLAZA_POS.x).powi(2) + (seat.y - PLAZA_POS.y).powi(2)).sqrt();
            assert!(
                (d - LUNCH_SEAT_RADIUS).abs() < 1.0,
                "{} 座位距鎮心應約等於座位環半徑，實際 {}", s.id, d
            );
        }
    }

    #[test]
    fn lunch_seat_is_deterministic_and_defaults_for_unknown() {
        // 同一 id 永遠回同一座位；非村落 NPC 回鎮心本身。
        assert_eq!(lunch_seat("merchant"), lunch_seat("merchant"));
        assert_eq!(lunch_seat("unknown_npc"), PLAZA_POS);
    }

    // —— ROADMAP 356：黃昏串門子 ——

    #[test]
    fn station_pos_known_and_unknown() {
        // 已知 NPC 回其崗位；未知 NPC 回 None。
        assert_eq!(station_pos("merchant"), Some(Pos::new(2120.0, 2328.0)));
        assert_eq!(station_pos("village_chief"), Some(Pos::new(2720.0, 2080.0)));
        assert_eq!(station_pos("traveler"), None);
    }

    #[test]
    fn visit_plan_empty_when_no_alliances() {
        // 沒有結盟對 → 沒人串門子。
        assert!(dusk_visit_plan(&[]).is_empty());
    }

    #[test]
    fn visit_plan_assigns_lexicographic_visitor() {
        // 一般結盟對：id 字典序小者當訪客、大者當主人（與輸入次序無關）。
        let plan = dusk_visit_plan(&[("merchant", "workshop_npc")]);
        assert_eq!(plan.get("merchant"), Some(&"workshop_npc"));
        // 反序輸入結果相同（穩定可重現）。
        let plan2 = dusk_visit_plan(&[("workshop_npc", "merchant")]);
        assert_eq!(plan2.get("merchant"), Some(&"workshop_npc"));
    }

    #[test]
    fn visit_plan_chief_is_always_host() {
        // 里長守在原地：永遠當被拜訪的主人，不外出串門。
        let plan = dusk_visit_plan(&[("merchant", "village_chief")]);
        assert_eq!(plan.get("merchant"), Some(&"village_chief"));
        assert!(plan.get("village_chief").is_none(), "里長不該當訪客");
    }

    #[test]
    fn visit_plan_no_npc_double_booked() {
        // 每個 NPC 至多參與一段串門關係：商人已和工匠配對後，與獵手的後續配對被跳過。
        let plan = dusk_visit_plan(&[
            ("merchant", "workshop_npc"),
            ("merchant", "bounty_npc"),
        ]);
        assert_eq!(plan.len(), 1, "商人已佔用，第二對應被跳過");
        assert_eq!(plan.get("merchant"), Some(&"workshop_npc"));
        // 互不重疊的兩對則都成立。
        let plan2 = dusk_visit_plan(&[
            ("merchant", "workshop_npc"),
            ("bounty_npc", "expedition_npc"),
        ]);
        assert_eq!(plan2.len(), 2);
    }

    #[test]
    fn visitor_walks_to_host_at_dusk() {
        // 黃昏時，被指派為訪客的 NPC 朝主人崗位旁移動（離開自己崗位）。
        let mut mgr = NpcScheduleManager::new();
        let dusk = DayNight::at(330.0); // fraction 0.55 → Dusk
        assert_eq!(dusk.phase(), Phase::Dusk);
        let mut visits: HashMap<&str, &str> = HashMap::new();
        visits.insert("merchant", "village_chief"); // 商人去拜訪里長

        let start_x = mgr.npcs.get("merchant").unwrap().x; // 2120
        // 走幾秒讓位移看得出來。
        for _ in 0..5 {
            mgr.tick(1.0, &dusk, &visits);
        }
        let after_x = mgr.npcs.get("merchant").unwrap().x;
        // 里長崗位在 x=2720（更右），訪客應往右靠近主人攤前。
        assert!(after_x > start_x, "商人黃昏串門應朝里長攤前（右）移動：{} -> {}", start_x, after_x);
    }

    #[test]
    fn visitor_arrives_and_shows_visiting_activity() {
        // 訪客抵達主人攤前後，活動狀態為 Visiting。
        let mut mgr = NpcScheduleManager::new();
        let dusk = DayNight::at(330.0);
        let mut visits: HashMap<&str, &str> = HashMap::new();
        visits.insert("merchant", "village_chief");
        // 走久一點讓商人抵達里長攤前（距離夠遠，多 tick 幾次）。
        for _ in 0..600 {
            mgr.tick(1.0, &dusk, &visits);
        }
        let m = mgr.npcs.get("merchant").unwrap();
        let host = station_pos("village_chief").unwrap();
        assert!((m.x - (host.x + VISIT_BESIDE_OFFSET)).abs() < 2.0, "商人應停在里長攤前並肩位");
        assert_eq!(m.activity, Some(NpcActivity::Visiting), "抵達後應為串門寒暄狀態");
    }

    #[test]
    fn no_visiting_outside_dusk() {
        // 非黃昏（白天）即使誤傳 visits，也不生效——商人照常守崗位、不串門。
        let mut mgr = NpcScheduleManager::new();
        let day = DayNight::new(); // Day
        let mut visits: HashMap<&str, &str> = HashMap::new();
        visits.insert("merchant", "village_chief");
        mgr.tick(1.0, &day, &visits);
        let m = mgr.npcs.get("merchant").unwrap();
        assert_eq!(m.x, 2120.0, "白天商人應守在崗位、不串門");
        assert_eq!(m.activity, Some(NpcActivity::Tallying));
    }
}
