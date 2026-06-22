//! 乙太暴走事件（ROADMAP 504）。
//!
//! 每隔 SURGE_INTERVAL_SECS 秒，地圖某個隨機採集點「暴走爆發」——發出強光，
//! 在 SURGE_RADIUS 像素內採集額外得 SURGE_BONUS 乙太；持續 SURGE_DURATION_SECS
//! 秒後消退。觸發時廣播落點方向，讓全服旅人有機會趕到。
//!
//! 純邏輯、零 IO、零 LLM、零 migration：狀態記憶體前置，重啟清零。

/// 暴走間隔（秒）——約 20 分鐘一次。
pub const SURGE_INTERVAL_SECS: f32 = 1200.0;
/// 首次觸發等待（秒）——伺服器啟動後約 6 分鐘，讓玩家先適應再看到事件。
pub const FIRST_WAIT_SECS: f32 = 360.0;
/// 暴走持續時間（秒）——90 秒窗口。
pub const SURGE_DURATION_SECS: f32 = 90.0;
/// 暴走有效採集半徑（像素）。
pub const SURGE_RADIUS: f32 = 180.0;
/// 暴走時額外採集量（疊加在工具/天氣/協作等既有加成上）。
pub const SURGE_BONUS: u32 = 3;

/// 可能的暴走位置（戶外、城鎮保護圈外）及對應廣播方向標籤。
/// 格式：(世界座標 x, 世界座標 y, 廣播用方向描述)。
pub const SURGE_LOCATIONS: &[(f32, f32, &str)] = &[
    (4200.0,  800.0, "東北"),
    ( 700.0, 3900.0, "西南"),
    (4200.0, 3900.0, "東南"),
    (2400.0, 4500.0, "正南"),
    ( 800.0,  800.0, "西北"),
];

/// 乙太暴走狀態（記憶體前置，重啟清零）。
#[derive(Debug)]
pub struct EtherSurge {
    /// 距下次暴走觸發的倒數（秒）。降到 0 時觸發。
    timer: f32,
    /// 目前暴走是否啟動。
    pub active: bool,
    /// 暴走位置 X（世界座標）。
    pub x: f32,
    /// 暴走位置 Y（世界座標）。
    pub y: f32,
    /// 暴走剩餘持續秒數。降到 0 時關閉並重設計時。
    pub remaining: f32,
    /// 循環索引，決定下次暴走的位置。
    location_idx: usize,
}

impl EtherSurge {
    pub fn new() -> Self {
        Self {
            timer: FIRST_WAIT_SECS,
            active: false,
            x: 0.0,
            y: 0.0,
            remaining: 0.0,
            location_idx: 0,
        }
    }

    /// 推進計時器。
    /// 回傳 `Some((x, y, dir))` 表示這個 tick 觸發了新暴走，需廣播事件。
    pub fn tick(&mut self, dt: f32) -> Option<(f32, f32, &'static str)> {
        if self.active {
            self.remaining -= dt;
            if self.remaining <= 0.0 {
                self.active = false;
                self.timer = SURGE_INTERVAL_SECS;
            }
            None
        } else {
            self.timer -= dt;
            if self.timer <= 0.0 {
                let idx = self.location_idx % SURGE_LOCATIONS.len();
                let (x, y, dir) = SURGE_LOCATIONS[idx];
                self.location_idx = self.location_idx.wrapping_add(1);
                self.active = true;
                self.x = x;
                self.y = y;
                self.remaining = SURGE_DURATION_SECS;
                self.timer = SURGE_INTERVAL_SECS;
                Some((x, y, dir))
            } else {
                None
            }
        }
    }

    /// 暴走剩餘秒數（用於快照廣播）。未啟動時為 0。
    pub fn remaining_secs(&self) -> u32 {
        if self.active { self.remaining.ceil() as u32 } else { 0 }
    }
}

/// 純函式：玩家在 `(px, py)` 採集時，是否在暴走加成圈內？
/// 是則回傳 `SURGE_BONUS`，否則 0。確定性、零副作用、好測。
pub fn surge_bonus_at(active: bool, surge_x: f32, surge_y: f32, px: f32, py: f32) -> u32 {
    if !active {
        return 0;
    }
    let dx = px - surge_x;
    let dy = py - surge_y;
    // 以半徑平方比較，避免開根號
    if dx * dx + dy * dy <= SURGE_RADIUS * SURGE_RADIUS {
        SURGE_BONUS
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── surge_bonus_at 純函式 ────────────────────────────────────────────────

    #[test]
    fn inactive_gives_zero() {
        assert_eq!(surge_bonus_at(false, 1000.0, 1000.0, 1000.0, 1000.0), 0);
    }

    #[test]
    fn active_within_radius_gives_bonus() {
        // 玩家在暴走點正中央
        assert_eq!(surge_bonus_at(true, 1000.0, 1000.0, 1000.0, 1000.0), SURGE_BONUS);
    }

    #[test]
    fn active_just_inside_radius_gives_bonus() {
        // 玩家距暴走點恰好等於半徑（邊界包含）
        let px = 1000.0 + SURGE_RADIUS;
        assert_eq!(surge_bonus_at(true, 1000.0, 1000.0, px, 1000.0), SURGE_BONUS);
    }

    #[test]
    fn active_outside_radius_gives_zero() {
        // 玩家距暴走點超出半徑 1 像素
        let px = 1000.0 + SURGE_RADIUS + 1.0;
        assert_eq!(surge_bonus_at(true, 1000.0, 1000.0, px, 1000.0), 0);
    }

    #[test]
    fn active_far_away_gives_zero() {
        assert_eq!(surge_bonus_at(true, 1000.0, 1000.0, 9999.0, 9999.0), 0);
    }

    // ── EtherSurge 狀態機 ───────────────────────────────────────────────────

    #[test]
    fn initial_state_inactive() {
        let surge = EtherSurge::new();
        assert!(!surge.active);
        assert_eq!(surge.remaining_secs(), 0);
    }

    #[test]
    fn tick_before_timer_does_nothing() {
        let mut surge = EtherSurge::new();
        // 經過 FIRST_WAIT_SECS 的一半還不觸發
        let result = surge.tick(FIRST_WAIT_SECS * 0.5);
        assert!(result.is_none());
        assert!(!surge.active);
    }

    #[test]
    fn tick_past_timer_triggers_surge() {
        let mut surge = EtherSurge::new();
        // 超過首次等待時間觸發
        let result = surge.tick(FIRST_WAIT_SECS + 1.0);
        assert!(result.is_some());
        assert!(surge.active);
        let (_, _, dir) = result.unwrap();
        // dir 是預定位置清單的第一個方向
        assert_eq!(dir, SURGE_LOCATIONS[0].2);
    }

    #[test]
    fn tick_during_active_returns_none() {
        let mut surge = EtherSurge::new();
        surge.tick(FIRST_WAIT_SECS + 1.0); // 觸發暴走
        let result = surge.tick(1.0);       // 還在持續中
        assert!(result.is_none());
        assert!(surge.active);
    }

    #[test]
    fn surge_deactivates_when_duration_expires() {
        let mut surge = EtherSurge::new();
        surge.tick(FIRST_WAIT_SECS + 1.0); // 觸發
        surge.tick(SURGE_DURATION_SECS + 1.0); // 超過持續時間
        assert!(!surge.active);
        assert_eq!(surge.remaining_secs(), 0);
    }

    #[test]
    fn location_idx_cycles() {
        let mut surge = EtherSurge::new();
        // 觸發足夠多次讓索引循環
        let n = SURGE_LOCATIONS.len() + 1;
        for _ in 0..n {
            // 重設為短計時以快速觸發
            surge.active = false;
            surge.timer = 0.0;
            surge.tick(1.0);
        }
        // 不 panic、location_idx 正常循環
        assert!(surge.location_idx > 0);
    }

    #[test]
    fn remaining_secs_rounds_up() {
        let mut surge = EtherSurge::new();
        surge.tick(FIRST_WAIT_SECS + 1.0); // 觸發
        // remaining 起始為 SURGE_DURATION_SECS，ceil 後與 u32 轉換正確
        let secs = surge.remaining_secs();
        assert_eq!(secs, SURGE_DURATION_SECS.ceil() as u32);
    }
}
