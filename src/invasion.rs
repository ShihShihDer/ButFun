//! 城鎮入侵警報（ROADMAP 158）。
//!
//! 每 90 分鐘，12 隻野怪在城鎮外圍（安全區外 800px 處）同時現身，玩家攜手抵禦；
//! 5 分鐘後波次消退，城鎮繁榮 +3，事件記錄進城鎮記憶石。
//!
//! 純邏輯層：不呼叫 LLM、不依賴 IO、記憶體模式，重啟清零。

use crate::combat::EnemyKind;

/// 每次入侵的間隔秒數（90 分鐘）。
pub const INVASION_INTERVAL_SECS: f32 = 90.0 * 60.0;
/// 首次入侵等待秒數（伺服器暖機，5 分鐘）。
pub const INVASION_FIRST_DELAY_SECS: f32 = 5.0 * 60.0;
/// 入侵波次持續秒數（5 分鐘）。
pub const INVASION_DURATION_SECS: f32 = 5.0 * 60.0;
/// 每波次生成的敵人數量。
pub const INVASION_ENEMY_COUNT: usize = 12;
/// 入侵怪物生成環的內半徑（城鎮中心外側）。
pub const INVASION_INNER_RADIUS: f32 = 800.0;
/// 入侵怪物生成環的外半徑（交錯排列讓分布更自然）。
pub const INVASION_OUTER_RADIUS: f32 = 950.0;

/// 城鎮中心 X，與 world-core::SAFE_ZONE_CX 同步。
pub const TOWN_CX: f32 = 2344.0;
/// 城鎮中心 Y，與 world-core::SAFE_ZONE_CY 同步。
pub const TOWN_CY: f32 = 2296.0;

/// 遊戲迴圈每 tick 呼叫 `InvasionState::tick` 可能得到的事件。
pub enum InvasionEvent {
    /// 入侵剛開始：返回「(種類, 世界座標 x, 世界座標 y)」列表，遊戲迴圈負責注入敵人。
    Started(Vec<(EnemyKind, f32, f32)>),
    /// 入侵波次消退。
    Ended,
}

/// 城鎮入侵警報的全局狀態，由 `AppState` 持有。
#[derive(Debug, Clone)]
pub struct InvasionState {
    /// 距下次入侵觸發的倒數（秒）。降到 0 時觸發入侵。
    timer: f32,
    /// 入侵是否進行中。
    pub active: bool,
    /// 入侵剩餘持續時間（秒）。
    pub remaining: f32,
    /// 已完成的入侵波次計數（每波結束後 +1）。
    pub wave_count: u32,
}

impl InvasionState {
    pub fn new() -> Self {
        Self {
            timer: INVASION_FIRST_DELAY_SECS,
            active: false,
            remaining: 0.0,
            wave_count: 0,
        }
    }

    /// 推進計時器，回傳 `Some(event)` 表示本 tick 發生了可廣播的重要事件。
    pub fn tick(&mut self, dt: f32) -> Option<InvasionEvent> {
        if self.active {
            self.remaining -= dt;
            if self.remaining <= 0.0 {
                self.active = false;
                self.remaining = 0.0;
                self.timer = INVASION_INTERVAL_SECS;
                self.wave_count = self.wave_count.saturating_add(1);
                return Some(InvasionEvent::Ended);
            }
            None
        } else {
            self.timer -= dt;
            if self.timer <= 0.0 {
                self.active = true;
                self.remaining = INVASION_DURATION_SECS;
                return Some(InvasionEvent::Started(self.spawn_list()));
            }
            None
        }
    }

    /// 入侵剩餘秒數（供快照廣播）。
    pub fn remaining_secs(&self) -> f32 {
        self.remaining.max(0.0)
    }

    /// 生成 12 隻怪物的種類與位置，均勻分布在城鎮外圍圓環上。
    fn spawn_list(&self) -> Vec<(EnemyKind, f32, f32)> {
        let kinds = INVASION_KINDS;
        (0..INVASION_ENEMY_COUNT)
            .map(|i| {
                let angle = (i as f32) * std::f32::consts::TAU / (INVASION_ENEMY_COUNT as f32);
                // 奇偶交替使用兩個半徑，讓怪物不全堆在同一圈。
                let radius = if i % 2 == 0 {
                    INVASION_INNER_RADIUS
                } else {
                    INVASION_OUTER_RADIUS
                };
                let x = TOWN_CX + radius * angle.cos();
                let y = TOWN_CY + radius * angle.sin();
                (kinds[i % kinds.len()], x, y)
            })
            .collect()
    }
}

impl Default for InvasionState {
    fn default() -> Self {
        Self::new()
    }
}

/// 入侵波次使用的怪物種類（城鎮生態域、無超高等 BOSS）。
const INVASION_KINDS: &[EnemyKind] = &[
    EnemyKind::ScrapDrone,
    EnemyKind::EtherWisp,
    EnemyKind::FlutterSprite,
    EnemyKind::MushroomStalker,
    EnemyKind::CrystalGolem,
    EnemyKind::RuneGuardian,
];

// ─── 純邏輯單元測試 ────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    fn make() -> InvasionState {
        InvasionState::new()
    }

    #[test]
    fn starts_inactive() {
        let s = make();
        assert!(!s.active, "初始狀態應為未入侵");
    }

    #[test]
    fn first_delay_before_invasion() {
        let mut s = make();
        // 推進不足首次等待時間，不應觸發。
        let ev = s.tick(INVASION_FIRST_DELAY_SECS - 1.0);
        assert!(ev.is_none(), "未到首次等待時間不應觸發");
        assert!(!s.active);
    }

    #[test]
    fn triggers_after_first_delay() {
        let mut s = make();
        let ev = s.tick(INVASION_FIRST_DELAY_SECS + 0.1);
        assert!(matches!(ev, Some(InvasionEvent::Started(_))), "超過首次等待應觸發");
        assert!(s.active, "入侵觸發後應為 active");
        assert!(s.remaining > 0.0, "入侵觸發後 remaining 應 > 0");
    }

    #[test]
    fn spawn_list_has_correct_count() {
        let s = make();
        let list = s.spawn_list();
        assert_eq!(list.len(), INVASION_ENEMY_COUNT, "生成列表應有 12 隻怪");
    }

    #[test]
    fn spawn_list_all_outside_safe_zone() {
        let s = make();
        // 安全區半徑 640，生成位置應在 800+ 以外。
        let safe_r = 640.0_f32;
        for (_, x, y) in s.spawn_list() {
            let dx = x - TOWN_CX;
            let dy = y - TOWN_CY;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(
                dist > safe_r,
                "怪物應生在安全區外 (dist={:.1})",
                dist
            );
        }
    }

    #[test]
    fn invasion_ends_after_duration() {
        let mut s = make();
        // 觸發入侵。
        s.tick(INVASION_FIRST_DELAY_SECS + 0.1);
        assert!(s.active);
        // 推進超過持續時間。
        let ev = s.tick(INVASION_DURATION_SECS + 0.1);
        assert!(matches!(ev, Some(InvasionEvent::Ended)), "持續時間到應結束");
        assert!(!s.active, "結束後不應 active");
    }

    #[test]
    fn wave_count_increments_on_end() {
        let mut s = make();
        s.tick(INVASION_FIRST_DELAY_SECS + 0.1);
        assert_eq!(s.wave_count, 0);
        s.tick(INVASION_DURATION_SECS + 0.1);
        assert_eq!(s.wave_count, 1, "波次結束後 wave_count 應 +1");
    }

    #[test]
    fn timer_resets_for_next_invasion() {
        let mut s = make();
        s.tick(INVASION_FIRST_DELAY_SECS + 0.1);
        s.tick(INVASION_DURATION_SECS + 0.1);
        assert!(!s.active);
        // 不足間隔時間不再觸發。
        let ev = s.tick(INVASION_INTERVAL_SECS - 1.0);
        assert!(ev.is_none(), "未到間隔時間不應再次觸發");
    }

    #[test]
    fn triggers_again_after_interval() {
        let mut s = make();
        s.tick(INVASION_FIRST_DELAY_SECS + 0.1);
        s.tick(INVASION_DURATION_SECS + 0.1);
        let ev = s.tick(INVASION_INTERVAL_SECS + 0.1);
        assert!(matches!(ev, Some(InvasionEvent::Started(_))), "間隔後應再次觸發");
    }

    #[test]
    fn remaining_secs_clamps_to_zero() {
        let s = make();
        assert_eq!(s.remaining_secs(), 0.0, "未入侵時 remaining_secs 應為 0");
    }

    #[test]
    fn spawn_positions_are_in_expected_radius_range() {
        let s = make();
        for (_, x, y) in s.spawn_list() {
            let dx = x - TOWN_CX;
            let dy = y - TOWN_CY;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(
                dist >= INVASION_INNER_RADIUS - 1.0 && dist <= INVASION_OUTER_RADIUS + 1.0,
                "距離應在 [{}, {}]，實際 {:.1}",
                INVASION_INNER_RADIUS,
                INVASION_OUTER_RADIUS,
                dist
            );
        }
    }

    #[test]
    fn invasion_kinds_not_empty() {
        assert!(!INVASION_KINDS.is_empty(), "入侵種類列表不能為空");
    }
}
