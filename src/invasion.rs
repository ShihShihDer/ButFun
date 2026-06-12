//! 城鎮入侵警報（ROADMAP 158）& 入侵首領（ROADMAP 159）& 入侵升級制度（ROADMAP 161）。
//!
//! 每 90 分鐘，12 隻野怪 + 1 名「乙太霸主」在城鎮外圍同時現身，玩家攜手抵禦；
//! 5 分鐘後波次消退：
//!   - 首領被擊殺 → 全服乙太特別獎勵（隨等級遞增）+ 記憶石記錄。
//!   - 首領逃脫  → 全服 +5 乙太 + 連勝重置。
//!
//! 入侵升級制度（ROADMAP 161）：連勝守城提升入侵等級，等級越高怪更多、獎勵更豐。
//!   Lv.1（0-2 連勝）：12 怪，90 分鐘間隔，首領 +10 乙太，分 1 顆晶核。
//!   Lv.2（3-5 連勝）：15 怪，85 分鐘間隔，首領 +15 乙太，分 2 顆晶核。
//!   Lv.3（6+ 連勝）  ：18 怪，80 分鐘間隔，首領 +20 乙太，分 3 顆晶核。
//!
//! 純邏輯層：不呼叫 LLM、不依賴 IO、記憶體模式，重啟清零。

use crate::combat::EnemyKind;

/// 首次入侵等待秒數（伺服器暖機，5 分鐘）。
pub const INVASION_FIRST_DELAY_SECS: f32 = 5.0 * 60.0;
/// 入侵波次持續秒數（5 分鐘）。
pub const INVASION_DURATION_SECS: f32 = 5.0 * 60.0;
/// 入侵怪物生成環的內半徑（城鎮中心外側）。
pub const INVASION_INNER_RADIUS: f32 = 800.0;
/// 入侵怪物生成環的外半徑（交錯排列讓分布更自然）。
pub const INVASION_OUTER_RADIUS: f32 = 950.0;
/// 首領生成位置的半徑（略在內外圈中間，正南方向突進以示帶頭）。
pub const INVASION_BOSS_RADIUS: f32 = 870.0;

// ─── 入侵升級制度閾值（ROADMAP 161）─────────────────────────────────────────
/// 達到 Lv.2 所需的連勝次數。
pub const WAVE_LEVEL_2_THRESHOLD: u32 = 3;
/// 達到 Lv.3 所需的連勝次數。
pub const WAVE_LEVEL_3_THRESHOLD: u32 = 6;

/// 城鎮中心 X，與 world-core::SAFE_ZONE_CX 同步。
pub const TOWN_CX: f32 = 2344.0;
/// 城鎮中心 Y，與 world-core::SAFE_ZONE_CY 同步。
pub const TOWN_CY: f32 = 2296.0;

/// 遊戲迴圈每 tick 呼叫 `InvasionState::tick` 可能得到的事件。
pub enum InvasionEvent {
    /// 入侵剛開始：包含生成列表、本波等級、觸發前連勝數。
    /// 遊戲迴圈負責注入敵人。
    Started {
        spawns: Vec<(EnemyKind, f32, f32)>,
        /// 本波入侵等級（1/2/3）。
        wave_level: u32,
        /// 觸發前已累積的連勝數。
        consecutive_successes: u32,
    },
    /// 入侵波次消退；附帶首領是否在本波被玩家擊殺、本波等級、更新後連勝數。
    Ended {
        boss_killed: bool,
        /// 本波入侵等級（1/2/3），用於決定獎勵規模。
        wave_level: u32,
        /// 結束後更新過的連勝數（boss_killed=true 則已 +1，否則已重置為 0）。
        consecutive_successes: u32,
    },
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
    /// 連續成功守城次數（首領被擊殺計 +1，逃脫重置為 0）。
    pub consecutive_successes: u32,
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
            consecutive_successes: 0,
        }
    }

    /// 當前入侵等級（1/2/3），依連勝次數計算。
    pub fn wave_level(&self) -> u32 {
        if self.consecutive_successes >= WAVE_LEVEL_3_THRESHOLD {
            3
        } else if self.consecutive_successes >= WAVE_LEVEL_2_THRESHOLD {
            2
        } else {
            1
        }
    }

    /// 本波次應生成的普通入侵怪物數量（不含首領）。
    pub fn enemy_count(&self) -> usize {
        match self.wave_level() {
            3 => 18,
            2 => 15,
            _ => 12,
        }
    }

    /// 首領被擊殺時全服乙太獎勵（隨等級遞增）。
    pub fn ether_boss_reward(&self) -> u32 {
        match self.wave_level() {
            3 => 20,
            2 => 15,
            _ => 10,
        }
    }

    /// 首領被擊殺時每位玩家獲得的霸主晶核數量（隨等級遞增）。
    pub fn cores_reward(&self) -> u32 {
        match self.wave_level() {
            3 => 3,
            2 => 2,
            _ => 1,
        }
    }

    /// 本波次結束後的下次入侵間隔（等級越高越頻繁）。
    fn next_interval(&self) -> f32 {
        match self.wave_level() {
            3 => 80.0 * 60.0,
            2 => 85.0 * 60.0,
            _ => 90.0 * 60.0,
        }
    }

    /// 推進計時器，回傳 `Some(event)` 表示本 tick 發生了可廣播的重要事件。
    pub fn tick(&mut self, dt: f32) -> Option<InvasionEvent> {
        if self.active {
            self.remaining -= dt;
            if self.remaining <= 0.0 {
                let boss_killed = self.boss_killed_this_wave;
                let level = self.wave_level();
                self.active = false;
                self.remaining = 0.0;
                self.boss_alive = false;
                self.boss_killed_this_wave = false;
                // 連勝計數：擊殺首領 +1，逃脫重置。
                if boss_killed {
                    self.consecutive_successes = self.consecutive_successes.saturating_add(1);
                } else {
                    self.consecutive_successes = 0;
                }
                // 依新連勝數決定下次間隔。
                self.timer = self.next_interval();
                let successes_after = self.consecutive_successes;
                self.wave_count = self.wave_count.saturating_add(1);
                return Some(InvasionEvent::Ended {
                    boss_killed,
                    wave_level: level,
                    consecutive_successes: successes_after,
                });
            }
            None
        } else {
            self.timer -= dt;
            if self.timer <= 0.0 {
                let level = self.wave_level();
                let successes = self.consecutive_successes;
                self.active = true;
                self.remaining = INVASION_DURATION_SECS;
                self.boss_alive = true;
                self.boss_killed_this_wave = false;
                let spawns = self.spawn_list();
                return Some(InvasionEvent::Started {
                    spawns,
                    wave_level: level,
                    consecutive_successes: successes,
                });
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

    /// 生成本波次所有敵人（首領在前，依等級決定普通怪數量）。
    fn spawn_list(&self) -> Vec<(EnemyKind, f32, f32)> {
        // 首領：乙太霸主在正南方（π/2 角）半徑中點登場，帶頭突入。
        let boss_x = TOWN_CX;
        let boss_y = TOWN_CY + INVASION_BOSS_RADIUS;
        let mut list = vec![(EnemyKind::EtherOverlord, boss_x, boss_y)];

        let count = self.enemy_count();
        let kinds = INVASION_KINDS;
        for i in 0..count {
            let angle = (i as f32) * std::f32::consts::TAU / (count as f32);
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

    // 快速讓入侵觸發。
    fn start_invasion(s: &mut InvasionState) {
        s.tick(INVASION_FIRST_DELAY_SECS + 0.1);
    }

    // 快速結束一波（不殺首領）。
    fn end_invasion(s: &mut InvasionState) -> Option<InvasionEvent> {
        s.tick(INVASION_DURATION_SECS + 0.1)
    }

    // 以首領被擊殺方式結束一波。
    fn end_with_boss_kill(s: &mut InvasionState) -> Option<InvasionEvent> {
        s.mark_boss_killed();
        end_invasion(s)
    }

    // 等待最長間隔後觸發下一波。
    fn wait_and_trigger_next(s: &mut InvasionState) -> Option<InvasionEvent> {
        s.tick(90.0 * 60.0 + 1.0)
    }

    // ─── 基礎行為 ───────────────────────────────────────────────────────────

    #[test]
    fn starts_inactive() {
        let s = make();
        assert!(!s.active, "初始狀態應為未入侵");
        assert!(!s.boss_alive, "初始狀態首領應未登場");
    }

    #[test]
    fn first_delay_before_invasion() {
        let mut s = make();
        let ev = s.tick(INVASION_FIRST_DELAY_SECS - 1.0);
        assert!(ev.is_none(), "未到首次等待時間不應觸發");
        assert!(!s.active);
    }

    #[test]
    fn triggers_after_first_delay() {
        let mut s = make();
        let ev = s.tick(INVASION_FIRST_DELAY_SECS + 0.1);
        assert!(
            matches!(ev, Some(InvasionEvent::Started { .. })),
            "超過首次等待應觸發"
        );
        assert!(s.active);
        assert!(s.remaining > 0.0);
        assert!(s.boss_alive);
    }

    #[test]
    fn spawn_list_lv1_has_13_entities() {
        let s = make();
        // Lv.1：12 普通怪 + 1 首領 = 13
        assert_eq!(s.spawn_list().len(), 13);
    }

    #[test]
    fn spawn_list_lv2_has_16_entities() {
        let mut s = make();
        s.consecutive_successes = WAVE_LEVEL_2_THRESHOLD;
        assert_eq!(s.wave_level(), 2);
        // Lv.2：15 普通怪 + 1 首領 = 16
        assert_eq!(s.spawn_list().len(), 16);
    }

    #[test]
    fn spawn_list_lv3_has_19_entities() {
        let mut s = make();
        s.consecutive_successes = WAVE_LEVEL_3_THRESHOLD;
        assert_eq!(s.wave_level(), 3);
        // Lv.3：18 普通怪 + 1 首領 = 19
        assert_eq!(s.spawn_list().len(), 19);
    }

    #[test]
    fn spawn_list_first_is_boss() {
        let s = make();
        assert_eq!(s.spawn_list()[0].0, EnemyKind::EtherOverlord);
    }

    #[test]
    fn spawn_list_all_outside_safe_zone() {
        let s = make();
        let safe_r = 640.0_f32;
        for (_, x, y) in s.spawn_list() {
            let dx = x - TOWN_CX;
            let dy = y - TOWN_CY;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(dist > safe_r, "怪物應生在安全區外 (dist={:.1})", dist);
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
        start_invasion(&mut s);
        assert!(s.active);
        let ev = end_invasion(&mut s);
        assert!(
            matches!(ev, Some(InvasionEvent::Ended { boss_killed: false, .. })),
            "首領存活時 boss_killed 應為 false"
        );
        assert!(!s.active);
        assert!(!s.boss_alive);
    }

    #[test]
    fn boss_kill_changes_end_reward() {
        let mut s = make();
        start_invasion(&mut s);
        let first_kill = s.mark_boss_killed();
        assert!(first_kill, "首次擊殺應回傳 true");
        assert!(!s.boss_alive);
        let second_kill = s.mark_boss_killed();
        assert!(!second_kill, "重複呼叫應回傳 false");
        let ev = s.tick(INVASION_DURATION_SECS + 0.1);
        assert!(
            matches!(ev, Some(InvasionEvent::Ended { boss_killed: true, .. })),
            "首領被擊殺後 boss_killed 應為 true"
        );
    }

    #[test]
    fn wave_count_increments_on_end() {
        let mut s = make();
        start_invasion(&mut s);
        assert_eq!(s.wave_count, 0);
        end_invasion(&mut s);
        assert_eq!(s.wave_count, 1);
    }

    #[test]
    fn triggers_again_after_interval() {
        let mut s = make();
        start_invasion(&mut s);
        end_invasion(&mut s);
        let ev = wait_and_trigger_next(&mut s);
        assert!(
            matches!(ev, Some(InvasionEvent::Started { .. })),
            "間隔後應再次觸發"
        );
        assert!(s.boss_alive);
    }

    #[test]
    fn remaining_secs_clamps_to_zero() {
        let s = make();
        assert_eq!(s.remaining_secs(), 0.0);
    }

    #[test]
    fn boss_not_killable_outside_invasion() {
        let mut s = make();
        assert!(!s.mark_boss_killed(), "非入侵期間 mark_boss_killed 應回傳 false");
    }

    #[test]
    fn invasion_kinds_not_empty() {
        assert!(!INVASION_KINDS.is_empty());
    }

    // ─── 升級制度測試（ROADMAP 161）────────────────────────────────────────

    #[test]
    fn wave_level_starts_at_1() {
        let s = make();
        assert_eq!(s.wave_level(), 1);
    }

    #[test]
    fn wave_level_reaches_2_after_threshold() {
        let mut s = make();
        s.consecutive_successes = WAVE_LEVEL_2_THRESHOLD;
        assert_eq!(s.wave_level(), 2);
    }

    #[test]
    fn wave_level_reaches_3_after_threshold() {
        let mut s = make();
        s.consecutive_successes = WAVE_LEVEL_3_THRESHOLD;
        assert_eq!(s.wave_level(), 3);
    }

    #[test]
    fn consecutive_successes_increments_on_boss_kill() {
        let mut s = make();
        start_invasion(&mut s);
        end_with_boss_kill(&mut s);
        assert_eq!(s.consecutive_successes, 1);
    }

    #[test]
    fn consecutive_successes_resets_on_boss_escape() {
        let mut s = make();
        start_invasion(&mut s);
        end_with_boss_kill(&mut s);
        assert_eq!(s.consecutive_successes, 1);
        wait_and_trigger_next(&mut s);
        end_invasion(&mut s); // 首領逃脫
        assert_eq!(s.consecutive_successes, 0, "首領逃脫後連勝應重置為 0");
    }

    #[test]
    fn ether_boss_reward_scales_with_level() {
        let mut s = make();
        assert_eq!(s.ether_boss_reward(), 10, "Lv.1 應給 +10 乙太");
        s.consecutive_successes = WAVE_LEVEL_2_THRESHOLD;
        assert_eq!(s.ether_boss_reward(), 15, "Lv.2 應給 +15 乙太");
        s.consecutive_successes = WAVE_LEVEL_3_THRESHOLD;
        assert_eq!(s.ether_boss_reward(), 20, "Lv.3 應給 +20 乙太");
    }

    #[test]
    fn cores_reward_scales_with_level() {
        let mut s = make();
        assert_eq!(s.cores_reward(), 1, "Lv.1 應分 1 顆晶核");
        s.consecutive_successes = WAVE_LEVEL_2_THRESHOLD;
        assert_eq!(s.cores_reward(), 2, "Lv.2 應分 2 顆晶核");
        s.consecutive_successes = WAVE_LEVEL_3_THRESHOLD;
        assert_eq!(s.cores_reward(), 3, "Lv.3 應分 3 顆晶核");
    }

    #[test]
    fn started_event_carries_wave_level() {
        let mut s = make();
        s.consecutive_successes = WAVE_LEVEL_2_THRESHOLD;
        let ev = s.tick(INVASION_FIRST_DELAY_SECS + 0.1);
        match ev {
            Some(InvasionEvent::Started { wave_level, .. }) => {
                assert_eq!(wave_level, 2, "Started 應攜帶 wave_level=2");
            }
            _ => panic!("應觸發 Started 事件"),
        }
    }

    #[test]
    fn ended_event_carries_wave_level_and_successes() {
        let mut s = make();
        start_invasion(&mut s);
        let ev = end_with_boss_kill(&mut s);
        match ev {
            Some(InvasionEvent::Ended { wave_level, consecutive_successes, boss_killed }) => {
                assert!(boss_killed);
                assert_eq!(wave_level, 1, "第一波應為 Lv.1");
                assert_eq!(consecutive_successes, 1, "首殺後連勝應為 1");
            }
            _ => panic!("應觸發 Ended 事件"),
        }
    }

    #[test]
    fn level_upgrade_after_3_consecutive_wins() {
        let mut s = make();
        // 第 1 波：首次觸發 → 殺首領。
        start_invasion(&mut s);
        end_with_boss_kill(&mut s);
        assert_eq!(s.consecutive_successes, 1);
        // 第 2、3 波：wait_and_trigger_next 本身會觸發 Started，直接接 end_with_boss_kill。
        for _ in 0..2 {
            wait_and_trigger_next(&mut s); // 觸發 Started，invasion 已 active
            end_with_boss_kill(&mut s);    // 殺首領並結束
        }
        assert_eq!(s.consecutive_successes, 3);
        assert_eq!(s.wave_level(), 2);
    }
}
