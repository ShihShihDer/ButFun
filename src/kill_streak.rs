//! 連殺熱度（ROADMAP 381）——玩家在 GAP_SECS 秒內連續擊殺怪物，累積「戰意」加成。
//!
//! 純邏輯層（無 IO / 無 WebSocket）：加成乘數由連殺數決定、到期自動衰退。
//! 狀態為記憶體前置（不持久化、零 migration），接線層（ws.rs）呼叫並套用。

use std::time::Instant;

/// 兩次擊殺之間允許的最大間隔（秒）；超過即連殺中斷、歸零重算。
pub const GAP_SECS: u64 = 8;

/// 連殺達到此門檻時廣播「連殺里程碑」訊息（2 / 4 / 8 連殺）。
/// 數值為「*達到*此連殺數時廣播」，向前端顯示特效。
const MILESTONES: &[u8] = &[2, 4, 8];

/// 依連殺數回傳傷害加成百分比（0 / 10 / 20 / 35）。
pub fn streak_bonus_pct(count: u8) -> u32 {
    if count >= 8 { 35 }
    else if count >= 4 { 20 }
    else if count >= 2 { 10 }
    else { 0 }
}

/// 依連殺數回傳傷害乘數（1.0 / 1.10 / 1.20 / 1.35）。
/// 乘在基礎攻擊力上，讓打出節奏的玩家攻擊更有力。
pub fn streak_bonus_mult(count: u8) -> f32 {
    1.0 + streak_bonus_pct(count) as f32 / 100.0
}

/// 若上次擊殺到現在已超過 GAP_SECS，則把 count 歸零（連殺中斷）。
/// 在每次出招前呼叫，讓「過了太久沒殺怪」的玩家熱度自然冷卻。
pub fn decay_if_expired(count: &mut u8, last_kill: &mut Option<Instant>, now: Instant) {
    if let Some(t) = *last_kill {
        let elapsed = now.duration_since(t).as_secs();
        if elapsed >= GAP_SECS {
            *count = 0;
            *last_kill = None;
        }
    }
}

/// 玩家打倒一隻怪物時呼叫，推進連殺計數並更新時間戳。
/// 回傳 `(new_count, is_milestone)` — `is_milestone` 為 true 表示剛踏入新門檻，
/// 呼叫層可據此廣播「連殺！」特效。
pub fn on_kill(count: &mut u8, last_kill: &mut Option<Instant>, now: Instant) -> (u8, bool) {
    let old = *count;
    *count = count.saturating_add(1);
    *last_kill = Some(now);
    let new = *count;
    let is_milestone = MILESTONES.contains(&new) && old < new;
    (new, is_milestone)
}

// ──────────────────────────────────────────────────────────────────────────────
// 測試
// ──────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bonus_pct_thresholds() {
        assert_eq!(streak_bonus_pct(0), 0);
        assert_eq!(streak_bonus_pct(1), 0);
        assert_eq!(streak_bonus_pct(2), 10);
        assert_eq!(streak_bonus_pct(3), 10);
        assert_eq!(streak_bonus_pct(4), 20);
        assert_eq!(streak_bonus_pct(7), 20);
        assert_eq!(streak_bonus_pct(8), 35);
        assert_eq!(streak_bonus_pct(99), 35);
    }

    #[test]
    fn bonus_mult_values() {
        let epsilon = 1e-6_f32;
        assert!((streak_bonus_mult(0) - 1.00).abs() < epsilon);
        assert!((streak_bonus_mult(2) - 1.10).abs() < epsilon);
        assert!((streak_bonus_mult(4) - 1.20).abs() < epsilon);
        assert!((streak_bonus_mult(8) - 1.35).abs() < epsilon);
    }

    #[test]
    fn decay_resets_on_expiry() {
        let mut count = 5u8;
        let mut last = Some(Instant::now() - std::time::Duration::from_secs(GAP_SECS + 1));
        let now = Instant::now();
        decay_if_expired(&mut count, &mut last, now);
        assert_eq!(count, 0);
        assert!(last.is_none());
    }

    #[test]
    fn decay_preserves_within_gap() {
        let mut count = 3u8;
        let mut last = Some(Instant::now() - std::time::Duration::from_secs(GAP_SECS - 1));
        let now = Instant::now();
        decay_if_expired(&mut count, &mut last, now);
        assert_eq!(count, 3);
        assert!(last.is_some());
    }

    #[test]
    fn decay_no_op_on_first_kill() {
        let mut count = 0u8;
        let mut last: Option<Instant> = None;
        decay_if_expired(&mut count, &mut last, Instant::now());
        assert_eq!(count, 0);
        assert!(last.is_none());
    }

    #[test]
    fn on_kill_increments_count() {
        let mut count = 0u8;
        let mut last = None;
        let now = Instant::now();
        let (new, milestone) = on_kill(&mut count, &mut last, now);
        assert_eq!(new, 1);
        assert!(!milestone); // 1 不是門檻
        assert_eq!(count, 1);
        assert!(last.is_some());
    }

    #[test]
    fn on_kill_milestone_at_two() {
        let mut count = 1u8;
        let mut last = Some(Instant::now());
        let (new, milestone) = on_kill(&mut count, &mut last, Instant::now());
        assert_eq!(new, 2);
        assert!(milestone); // 2 是門檻
    }

    #[test]
    fn on_kill_milestone_at_four() {
        let mut count = 3u8;
        let mut last = Some(Instant::now());
        let (new, milestone) = on_kill(&mut count, &mut last, Instant::now());
        assert_eq!(new, 4);
        assert!(milestone);
    }

    #[test]
    fn on_kill_no_duplicate_milestone() {
        // 已是門檻值、再殺一隻不算再次里程碑
        let mut count = 4u8;
        let mut last = Some(Instant::now());
        let (new, milestone) = on_kill(&mut count, &mut last, Instant::now());
        assert_eq!(new, 5);
        assert!(!milestone); // 5 不是門檻
    }

    #[test]
    fn on_kill_saturates_at_u8_max() {
        let mut count = u8::MAX;
        let mut last = Some(Instant::now());
        let (new, _) = on_kill(&mut count, &mut last, Instant::now());
        assert_eq!(new, u8::MAX); // 不溢位
    }
}
