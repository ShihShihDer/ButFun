/// 居民廣場聚會系統（ROADMAP 124）。
///
/// 每隔固定冷卻時間（白天時段），廣場自動觸發一次聚會，持續固定時長。
/// 聚會期間全服 EXP +20%、TownSquare 居民思想泡泡加快，廣播公告開始/結束。
/// 純邏輯模組——不呼叫 LLM，不持久化，記憶體模式，重啟清零。

use crate::daynight::Phase;

/// 聚會冷卻時間（秒）——兩次聚會之間的最短間隔。
pub const GATHERING_COOLDOWN_SECS: f32 = 1800.0;
/// 單次聚會持續時間（秒）。
pub const GATHERING_DURATION_SECS: f32 = 600.0;
/// 聚會期間全服 EXP 加成百分比（整數，+20%）。
pub const GATHERING_EXP_BONUS_PCT: u32 = 20;
/// 聚會期間 TownSquare 居民思想泡泡計時縮短比例（乘以此值）。
pub const THOUGHT_TIMER_GATHERING_FACTOR: f32 = 0.4;

/// 廣場聚會事件，由 `tick()` 回傳後由 `game.rs` 廣播。
#[derive(Debug, Clone, PartialEq)]
pub enum GatheringEvent {
    /// 聚會剛開始，附上廣播文字。
    Started { text: String },
    /// 聚會結束，附上廣播文字。
    Ended { text: String },
}

/// 廣場聚會狀態機。
#[derive(Debug)]
pub struct CommunityGatheringState {
    /// 距下次觸發聚會的剩餘冷卻秒數。冷卻歸零後，下一個白天 tick 就會觸發。
    pub cooldown: f32,
    /// 本次聚會剩餘秒數。0.0 表示目前無進行中的聚會。
    pub remaining: f32,
}

impl CommunityGatheringState {
    /// 初始狀態：冷卻 5 分鐘後第一次聚會就啟動（讓伺服器啟動不久後就有第一場）。
    pub fn new() -> Self {
        Self {
            cooldown: 300.0,
            remaining: 0.0,
        }
    }

    /// 目前是否有進行中的聚會。
    pub fn is_active(&self) -> bool {
        self.remaining > 0.0
    }

    /// 聚會剩餘整數秒數，供快照廣播。
    pub fn remaining_secs(&self) -> u32 {
        self.remaining.ceil() as u32
    }

    /// 推進時間，回傳本 tick 產生的事件清單。
    ///
    /// * `dt` — 本 tick 秒數（game loop 傳入，通常 1/15）
    /// * `phase` — 當前日夜時段（只有 Day 才能觸發新聚會）
    pub fn tick(&mut self, dt: f32, phase: Phase) -> Vec<GatheringEvent> {
        let mut events = Vec::new();

        if self.remaining > 0.0 {
            // 聚會進行中，倒數剩餘時間
            self.remaining -= dt;
            if self.remaining <= 0.0 {
                self.remaining = 0.0;
                self.cooldown = GATHERING_COOLDOWN_SECS;
                events.push(GatheringEvent::Ended {
                    text: "🌆 廣場聚會散場了，居民們各自回到崗位，城鎮恢復平日節奏。".to_string(),
                });
            }
        } else {
            // 無聚會，冷卻倒數
            self.cooldown -= dt;
            if self.cooldown <= 0.0 && phase == Phase::Day {
                self.cooldown = 0.0;
                self.remaining = GATHERING_DURATION_SECS;
                events.push(GatheringEvent::Started {
                    text: format!(
                        "🎊 廣場聚會開始！居民們熱鬧起來，接下來 {} 分鐘全服 EXP +{}%！",
                        (GATHERING_DURATION_SECS / 60.0) as u32,
                        GATHERING_EXP_BONUS_PCT,
                    ),
                });
            }
        }

        events
    }
}

impl Default for CommunityGatheringState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state_with(cooldown: f32, remaining: f32) -> CommunityGatheringState {
        CommunityGatheringState { cooldown, remaining }
    }

    #[test]
    fn new_state_is_not_active() {
        let s = CommunityGatheringState::new();
        assert!(!s.is_active());
    }

    #[test]
    fn tick_cooldown_triggers_started_on_day() {
        let mut s = make_state_with(0.5, 0.0);
        let evs = s.tick(1.0, Phase::Day);
        assert!(evs.iter().any(|e| matches!(e, GatheringEvent::Started { .. })));
        assert!(s.is_active());
    }

    #[test]
    fn tick_cooldown_does_not_trigger_outside_day() {
        for phase in [Phase::Dawn, Phase::Dusk, Phase::Night] {
            let mut s = make_state_with(0.1, 0.0);
            let evs = s.tick(1.0, phase);
            assert!(evs.is_empty(), "phase {:?} 不應在非白天觸發聚會", phase);
        }
    }

    #[test]
    fn remaining_secs_rounds_up() {
        let s = make_state_with(0.0, 9.3);
        assert_eq!(s.remaining_secs(), 10);
    }

    #[test]
    fn active_gathering_counts_down_to_ended() {
        let mut s = make_state_with(0.0, 1.0);
        assert!(s.is_active());
        let evs = s.tick(2.0, Phase::Day);
        assert!(evs.iter().any(|e| matches!(e, GatheringEvent::Ended { .. })));
        assert!(!s.is_active());
    }

    #[test]
    fn ended_resets_cooldown() {
        let mut s = make_state_with(0.0, 0.1);
        s.tick(1.0, Phase::Day);
        assert!(!s.is_active());
        assert!((s.cooldown - GATHERING_COOLDOWN_SECS).abs() < 1.0);
    }

    #[test]
    fn no_event_during_active_gathering() {
        let mut s = make_state_with(0.0, 100.0);
        let evs = s.tick(1.0, Phase::Day);
        assert!(evs.is_empty());
        assert!(s.is_active());
    }

    #[test]
    fn gathering_exp_bonus_constant_is_positive() {
        assert!(GATHERING_EXP_BONUS_PCT > 0);
    }

    #[test]
    fn thought_timer_factor_is_less_than_one() {
        assert!(THOUGHT_TIMER_GATHERING_FACTOR < 1.0);
        assert!(THOUGHT_TIMER_GATHERING_FACTOR > 0.0);
    }
}
