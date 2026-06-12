//! 城鎮入侵警報（ROADMAP 158）& 入侵首領（ROADMAP 159）。
//!
//! 每 90 分鐘，12 隻野怪 + 1 名「乙太霸主」在城鎮外圍同時現身，玩家攜手抵禦；
//! 5 分鐘後波次消退：
//!   - 首領被擊殺 → 全服 +10 乙太特別獎勵 + 記憶石記錄「首領擊敗」。
//!   - 首領逃脫  → 全服 +5 乙太 + 記憶石記錄「首領逃脫」。
//!
//! 純邏輯層：不呼叫 LLM、不依賴 IO、記憶體模式，重啟清零。

use crate::combat::EnemyKind;

/// 每次入侵的間隔秒數（90 分鐘）。
pub const INVASION_INTERVAL_SECS: f32 = 90.0 * 60.0;
/// 首次入侵等待秒數（伺服器暖機，5 分鐘）。
pub const INVASION_FIRST_DELAY_SECS: f32 = 5.0 * 60.0;
/// 入侵波次持續秒數（5 分鐘）。
pub const INVASION_DURATION_SECS: f32 = 5.0 * 60.0;
/// 每波次生成的普通入侵怪物數量（不含首領）。
pub const INVASION_ENEMY_COUNT: usize = 12;
/// 入侵怪物生成環的內半徑（城鎮中心外側）。
pub const INVASION_INNER_RADIUS: f32 = 800.0;
/// 入侵怪物生成環的外半徑（交錯排列讓分布更自然）。
pub const INVASION_OUTER_RADIUS: f32 = 950.0;
/// 首領生成位置的半徑（略在內外圈中間，正南方向突進以示帶頭）。
pub const INVASION_BOSS_RADIUS: f32 = 870.0;

/// 城鎮中心 X，與 world-core::SAFE_ZONE_CX 同步。
pub const TOWN_CX: f32 = 2344.0;
/// 城鎮中心 Y，與 world-core::SAFE_ZONE_CY 同步。
pub const TOWN_CY: f32 = 2296.0;

/// 遊戲迴圈每 tick 呼叫 `InvasionState::tick` 可能得到的事件。
pub enum InvasionEvent {
    /// 入侵剛開始：返回「(種類, 世界座標 x, 世界座標 y)」列表（含首領在最前），
    /// 遊戲迴圈負責注入敵人。
    Started(Vec<(EnemyKind, f32, f32)>),
    /// 入侵波次消退；附帶首領是否在本波被玩家擊殺。
    Ended { boss_killed: bool },
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
    /// 首領「乙太霸主」目前是否存活（入侵開始時設 true，被擊殺後設 false）。
    pub boss_alive: bool,
    /// 本波次首領是否已被玩家擊殺（波次結束時用於決定獎勵層級）。
    boss_killed_this_wave: bool,
}

impl InvasionState {
    pub fn new() -> Self {
        Self {
            timer: INVASION_FIRST_DELAY_SECS,
            active: false,
            remaining: 0.0,
            wave_count: 0,
            boss_alive: false,
            boss_killed_this_wave: false,
        }
    }

    /// 推進計時器，回傳 `Some(event)` 表示本 tick 發生了可廣播的重要事件。
    pub fn tick(&mut self, dt: f32) -> Option<InvasionEvent> {
        if self.active {
            self.remaining -= dt;
            if self.remaining <= 0.0 {
                let boss_killed = self.boss_killed_this_wave;
                self.active = false;
                self.remaining = 0.0;
                self.timer = INVASION_INTERVAL_SECS;
                self.wave_count = self.wave_count.saturating_add(1);
                self.boss_alive = false;
                self.boss_killed_this_wave = false;
                return Some(InvasionEvent::Ended { boss_killed });
            }
            None
        } else {
            self.timer -= dt;
            if self.timer <= 0.0 {
                self.active = true;
                self.remaining = INVASION_DURATION_SECS;
                self.boss_alive = true;
                self.boss_killed_this_wave = false;
                return Some(InvasionEvent::Started(self.spawn_list()));
            }
            None
        }
    }

    /// 入侵剩餘秒數（供快照廣播）。
    pub fn remaining_secs(&self) -> f32 {
        self.remaining.max(0.0)
    }

    /// 玩家在入侵進行中擊殺首領時呼叫；回傳 true 表示首次擊殺（避免重複廣播）。
    pub fn mark_boss_killed(&mut self) -> bool {
        if self.active && self.boss_alive {
            self.boss_alive = false;
            self.boss_killed_this_wave = true;
            true
        } else {
            false
        }
    }

    /// 生成本波次所有敵人（首領在前，12 隻普通怪接續）。
    fn spawn_list(&self) -> Vec<(EnemyKind, f32, f32)> {
        // 首領：乙太霸主在正南方（π/2 角）半徑中點登場，帶頭突入。
        let boss_x = TOWN_CX;
        let boss_y = TOWN_CY + INVASION_BOSS_RADIUS;
        let mut list = vec![(EnemyKind::EtherOverlord, boss_x, boss_y)];

        // 12 隻普通入侵怪物，均勻分布在城鎮外圍圓環上。
        let kinds = INVASION_KINDS;
        for i in 0..INVASION_ENEMY_COUNT {
            let angle = (i as f32) * std::f32::consts::TAU / (INVASION_ENEMY_COUNT as f32);
            // 奇偶交替使用兩個半徑，讓怪物不全堆在同一圈。
            let radius = if i % 2 == 0 {
                INVASION_INNER_RADIUS
            } else {
                INVASION_OUTER_RADIUS
            };
            let x = TOWN_CX + radius * angle.cos();
            let y = TOWN_CY + radius * angle.sin();
            list.push((kinds[i % kinds.len()], x, y));
        }
        list
    }
}

impl Default for InvasionState {
    fn default() -> Self {
        Self::new()
    }
}

/// 入侵波次使用的普通怪物種類（城鎮生態域、無超高等 BOSS）。
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
        assert!(!s.boss_alive, "初始狀態首領應未登場");
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
        assert!(s.boss_alive, "入侵開始時首領應登場");
    }

    #[test]
    fn spawn_list_has_correct_count() {
        let s = make();
        let list = s.spawn_list();
        // 首領 1 + 普通怪 12 = 13
        assert_eq!(list.len(), INVASION_ENEMY_COUNT + 1, "生成列表應有 13 隻（首領+12 普通怪）");
    }

    #[test]
    fn spawn_list_first_is_boss() {
        let s = make();
        let list = s.spawn_list();
        assert_eq!(list[0].0, EnemyKind::EtherOverlord, "生成列表第一隻應是乙太霸主");
    }

    #[test]
    fn spawn_list_all_outside_safe_zone() {
        let s = make();
        // 安全區半徑 640，生成位置應在 640+ 以外。
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
    fn boss_spawn_is_at_expected_radius() {
        let s = make();
        let (_, bx, by) = s.spawn_list()[0];
        let dist = ((bx - TOWN_CX).powi(2) + (by - TOWN_CY).powi(2)).sqrt();
        assert!(
            (dist - INVASION_BOSS_RADIUS).abs() < 1.0,
            "首領應生在 INVASION_BOSS_RADIUS 處，實際 {:.1}",
            dist
        );
    }

    #[test]
    fn invasion_ends_after_duration_with_boss_alive() {
        let mut s = make();
        // 觸發入侵。
        s.tick(INVASION_FIRST_DELAY_SECS + 0.1);
        assert!(s.active);
        // 推進超過持續時間（首領未被擊殺）。
        let ev = s.tick(INVASION_DURATION_SECS + 0.1);
        assert!(
            matches!(ev, Some(InvasionEvent::Ended { boss_killed: false })),
            "首領存活時 boss_killed 應為 false"
        );
        assert!(!s.active, "結束後不應 active");
        assert!(!s.boss_alive, "結束後首領應重置");
    }

    #[test]
    fn boss_kill_changes_end_reward() {
        let mut s = make();
        s.tick(INVASION_FIRST_DELAY_SECS + 0.1);
        // 模擬首領被擊殺。
        let first_kill = s.mark_boss_killed();
        assert!(first_kill, "首次擊殺應回傳 true");
        assert!(!s.boss_alive, "擊殺後 boss_alive 應為 false");
        // 再次呼叫應回傳 false（首領已倒）。
        let second_kill = s.mark_boss_killed();
        assert!(!second_kill, "重複呼叫應回傳 false（已倒）");
        // 波次結束事件應攜帶 boss_killed = true。
        let ev = s.tick(INVASION_DURATION_SECS + 0.1);
        assert!(
            matches!(ev, Some(InvasionEvent::Ended { boss_killed: true })),
            "首領被擊殺後 boss_killed 應為 true"
        );
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
        assert!(s.boss_alive, "下一波入侵開始時首領應重新登場");
    }

    #[test]
    fn remaining_secs_clamps_to_zero() {
        let s = make();
        assert_eq!(s.remaining_secs(), 0.0, "未入侵時 remaining_secs 應為 0");
    }

    #[test]
    fn boss_not_killable_outside_invasion() {
        let mut s = make();
        // 入侵未觸發時，mark_boss_killed 不應回傳 true。
        let r = s.mark_boss_killed();
        assert!(!r, "非入侵期間 mark_boss_killed 應回傳 false");
    }

    #[test]
    fn invasion_kinds_not_empty() {
        assert!(!INVASION_KINDS.is_empty(), "入侵種類列表不能為空");
    }
}
