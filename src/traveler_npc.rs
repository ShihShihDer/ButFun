//! 城外旅人 NPC 系統（ROADMAP 74）。
//!
//! 每 15 分鐘從城外走進一位旅人，停留 5 分鐘後繼續上路。
//! 旅人抵達/離開時廣播全服聊天；玩家可靠近聊天（走 Groq→ollama→罐頭降級鏈）。
//! 純記憶體模式，重啟清零，零 migration。

use std::collections::HashMap;
use uuid::Uuid;

/// 旅人到訪間隔（秒）。
pub const SPAWN_INTERVAL_SECS: f32 = 900.0; // 15 分鐘
/// 旅人在廣場停留時長（秒）。
pub const STAY_DURATION_SECS: f32 = 300.0; // 5 分鐘
/// 旅人移動速度（像素/秒）。
pub const TRAVELER_SPEED: f32 = 70.0;
/// 玩家可與旅人聊天的距離（像素）。
pub const TRAVELER_REACH: f32 = 100.0;

/// 旅人入城起點（城南門外）。
const ENTRY_X: f32 = 2300.0;
const ENTRY_Y: f32 = 2520.0;
/// 旅人在廣場的目標位置（鄰近夜晚聚會點）。
pub const PLAZA_X: f32 = 2430.0;
pub const PLAZA_Y: f32 = 2200.0;
/// 旅人離城的目標（城北側出口）。
const EXIT_X: f32 = 2300.0;
const EXIT_Y: f32 = 1900.0;

/// 旅人名字與身份描述（5 種，輪換出場）。
pub const TRAVELER_PROFILES: &[(&str, &str)] = &[
    ("科拿", "科拿星的流亡機械師，正在尋找修理古代機械的零件"),
    ("妮維", "赤焰星的探險家，徒步考察各星球的地形構造"),
    ("歐爾", "走遍五大星球的星際旅行商人，知道每個角落的奇聞"),
    ("璃安", "霧醚星的考古學者，專門研究大靜默留下的遺跡"),
    ("鐵克", "退休的星際運輸老船長，靠雙腳重走年輕走過的路"),
];

/// 旅人所處狀態。
#[derive(Debug, Clone, PartialEq)]
pub enum TravelerPhase {
    /// 尚未出場，等待下次到訪。timer 倒數完後切入 Approaching。
    Waiting { timer: f32 },
    /// 正從城外走進廣場中。
    Approaching,
    /// 在廣場停留，玩家可互動；timer 倒數完後切到 Leaving。
    Staying { timer: f32 },
    /// 正從廣場走向出口。
    Leaving,
}

/// 一位旅人的完整運行狀態。
#[derive(Debug, Clone)]
pub struct TravelerNpc {
    /// 目前輪到哪個旅人（索引 TRAVELER_PROFILES）。
    pub profile_idx: usize,
    pub x: f32,
    pub y: f32,
    pub phase: TravelerPhase,
    /// 本次停留期間對各玩家的對話次數（用來豐富 AI prompt）。
    pub talk_count: HashMap<Uuid, u32>,
}

/// 旅人狀態轉換事件，供上層觸發廣播與 world_log。
#[derive(Debug, PartialEq)]
pub enum TravelerEvent {
    /// 旅人剛從城外走進廣場（發公告）。
    Arrived { name: &'static str, origin: &'static str },
    /// 旅人離開主城（發公告）。
    Departed { name: &'static str },
}

impl TravelerNpc {
    /// 建立初始旅人：立即開始第一次等待計時（不縮短到訪間隔）。
    pub fn new() -> Self {
        Self {
            profile_idx: 0,
            x: ENTRY_X,
            y: ENTRY_Y,
            phase: TravelerPhase::Waiting { timer: SPAWN_INTERVAL_SECS },
            talk_count: HashMap::new(),
        }
    }

    /// 旅人當前名字。
    pub fn name(&self) -> &'static str {
        TRAVELER_PROFILES[self.profile_idx % TRAVELER_PROFILES.len()].0
    }

    /// 旅人當前身份描述。
    pub fn origin(&self) -> &'static str {
        TRAVELER_PROFILES[self.profile_idx % TRAVELER_PROFILES.len()].1
    }

    /// 旅人目前是否在世界中可見（Approaching 或 Staying）。
    pub fn is_visible(&self) -> bool {
        matches!(self.phase, TravelerPhase::Approaching | TravelerPhase::Staying { .. })
    }

    /// 玩家對這位旅人說了幾句話（0 = 初次相遇）。
    pub fn talk_count_for(&self, player_id: Uuid) -> u32 {
        self.talk_count.get(&player_id).copied().unwrap_or(0)
    }

    /// 記錄與某玩家的一次對話。
    pub fn record_talk(&mut self, player_id: Uuid) {
        *self.talk_count.entry(player_id).or_insert(0) += 1;
    }

    /// 推進一個 tick（dt 秒），回傳發生的狀態轉換事件（若有）。
    pub fn tick(&mut self, dt: f32) -> Option<TravelerEvent> {
        match self.phase.clone() {
            TravelerPhase::Waiting { timer } => {
                let new_t = timer - dt;
                if new_t <= 0.0 {
                    // 旅人從入城點出發
                    self.x = ENTRY_X;
                    self.y = ENTRY_Y;
                    self.phase = TravelerPhase::Approaching;
                    return Some(TravelerEvent::Arrived {
                        name: self.name(),
                        origin: self.origin(),
                    });
                }
                self.phase = TravelerPhase::Waiting { timer: new_t };
            }
            TravelerPhase::Approaching => {
                if self.move_toward(PLAZA_X, PLAZA_Y, dt) {
                    self.phase = TravelerPhase::Staying { timer: STAY_DURATION_SECS };
                }
            }
            TravelerPhase::Staying { timer } => {
                let new_t = timer - dt;
                if new_t <= 0.0 {
                    self.phase = TravelerPhase::Leaving;
                } else {
                    self.phase = TravelerPhase::Staying { timer: new_t };
                }
            }
            TravelerPhase::Leaving => {
                if self.move_toward(EXIT_X, EXIT_Y, dt) {
                    let name = self.name();
                    // 輪換到下一個旅人，重置等待計時，清空對話紀錄。
                    self.profile_idx = (self.profile_idx + 1) % TRAVELER_PROFILES.len();
                    self.talk_count.clear();
                    self.phase = TravelerPhase::Waiting { timer: SPAWN_INTERVAL_SECS };
                    return Some(TravelerEvent::Departed { name });
                }
            }
        }
        None
    }

    /// 向目標座標移動，回傳「是否已到達」（距離 ≤ 2px 視為抵達）。
    fn move_toward(&mut self, tx: f32, ty: f32, dt: f32) -> bool {
        let dx = tx - self.x;
        let dy = ty - self.y;
        let dist_sq = dx * dx + dy * dy;
        if dist_sq <= 4.0 {
            self.x = tx;
            self.y = ty;
            return true;
        }
        let dist = dist_sq.sqrt();
        let step = (TRAVELER_SPEED * dt).min(dist);
        self.x += (dx / dist) * step;
        self.y += (dy / dist) * step;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_traveler_starts_waiting() {
        let t = TravelerNpc::new();
        assert!(matches!(t.phase, TravelerPhase::Waiting { .. }));
        assert!(!t.is_visible());
    }

    #[test]
    fn traveler_becomes_visible_after_waiting() {
        let mut t = TravelerNpc::new();
        // 快轉超過等待時間
        let ev = t.tick(SPAWN_INTERVAL_SECS + 1.0);
        assert_eq!(ev, Some(TravelerEvent::Arrived { name: "科拿", origin: TRAVELER_PROFILES[0].1 }));
        assert!(t.is_visible());
        assert!(matches!(t.phase, TravelerPhase::Approaching));
    }

    #[test]
    fn traveler_moves_toward_plaza() {
        let mut t = TravelerNpc::new();
        // 先觸發到達
        t.tick(SPAWN_INTERVAL_SECS + 1.0);
        assert_eq!(t.x, ENTRY_X);
        // 走幾秒，位置應靠近廣場
        t.tick(2.0);
        assert!(t.y < ENTRY_Y, "旅人應向廣場（y 更小）移動");
    }

    #[test]
    fn traveler_leaves_after_stay_duration() {
        let mut t = TravelerNpc::new();
        t.tick(SPAWN_INTERVAL_SECS + 1.0); // Approaching
        // 快轉到廣場（假設足夠時間）
        for _ in 0..1000 {
            t.tick(0.1);
            if matches!(t.phase, TravelerPhase::Staying { .. }) {
                break;
            }
        }
        assert!(matches!(t.phase, TravelerPhase::Staying { .. }));
        // 停留結束
        let ev = t.tick(STAY_DURATION_SECS + 1.0);
        assert_eq!(ev, None); // Staying → Leaving 沒有事件
        assert!(matches!(t.phase, TravelerPhase::Leaving));
    }

    #[test]
    fn traveler_cycles_to_next_profile_after_departure() {
        let mut t = TravelerNpc::new();
        // 觸發到達
        t.tick(SPAWN_INTERVAL_SECS + 1.0);
        // 快轉到廣場
        for _ in 0..1000 {
            t.tick(0.1);
            if matches!(t.phase, TravelerPhase::Staying { .. }) { break; }
        }
        // 快轉停留結束 → Leaving
        t.tick(STAY_DURATION_SECS + 1.0);
        // 快轉離場（旅人走到 EXIT）
        let mut departed = None;
        for _ in 0..5000 {
            if let Some(ev) = t.tick(0.1) {
                departed = Some(ev);
                break;
            }
        }
        assert!(matches!(departed, Some(TravelerEvent::Departed { .. })));
        // 現在應輪換到下一個旅人
        assert_eq!(t.profile_idx, 1);
        assert_eq!(t.name(), TRAVELER_PROFILES[1].0);
        assert!(matches!(t.phase, TravelerPhase::Waiting { .. }));
    }

    #[test]
    fn talk_count_tracks_per_player() {
        let mut t = TravelerNpc::new();
        let uid = Uuid::new_v4();
        assert_eq!(t.talk_count_for(uid), 0);
        t.record_talk(uid);
        t.record_talk(uid);
        assert_eq!(t.talk_count_for(uid), 2);
    }

    #[test]
    fn profiles_count_correct() {
        assert_eq!(TRAVELER_PROFILES.len(), 5);
        for (name, origin) in TRAVELER_PROFILES {
            assert!(!name.is_empty());
            assert!(!origin.is_empty());
        }
    }
}
