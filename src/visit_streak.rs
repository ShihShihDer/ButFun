//! 連日歸鄉·歸鄉印記（ROADMAP 397）的純邏輯層。
//!
//! 留存維度第一個「跨真實日曆日」的回訪鉤子：玩家每連續一天回來，歸鄉印記就增長一階，
//! 故鄉送上一份小小的迎歸乙太。斷一天就溫和地從第 1 天重新數起（不懲罰、只重新累積），
//! 同一天重複上線不會重複領（以日為界把關）。
//!
//! 本檔**零 IO、零鎖**：只有確定性的純函式，吃「上次回訪是哪一天」與「今天是哪一天」算出結果，
//! 方便單元測試。持久化在 `visit_streak_store.rs`，接線在 `ws.rs` 登入路徑。

/// 每連續一天回訪的基礎迎歸乙太。刻意很小——這是療癒向的「謝你常回家」，不是經濟核心。
pub const REWARD_PER_DAY: u32 = 3;

/// 一次回訪計數推進的結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StreakOutcome {
    /// 推進後的歸鄉印記天數（同日重複上線時維持原值）。
    pub streak: u32,
    /// 這次是否真的往前數了一天（同日重複上線／時鐘倒退為 false）。
    pub advanced: bool,
    /// 這次該發的迎歸乙太（含里程碑暖禮；未前進時為 0）。
    pub reward: u32,
    /// 這次是否踏上里程碑（3／7／14／30…天），供前端讓迎歸卡金光更盛。
    pub milestone: bool,
}

/// 里程碑暖禮：連訪 3／7／14／30 天各有額外迎歸乙太，30 天後每滿 30 天再給一次。
/// 非里程碑回 0。數值刻意溫和（療癒向、不擾經濟）。
pub fn milestone_bonus(streak: u32) -> u32 {
    match streak {
        3 => 10,
        7 => 25,
        14 => 50,
        s if s >= 30 && s % 30 == 0 => 100,
        _ => 0,
    }
}

/// 依「上次回訪日」與「今天」推進歸鄉印記。
///
/// - `prev_day = None`（從未來訪）→ 第 1 天、發獎。
/// - 上次就是今天 → 同日重複上線：不前進、不發獎（防重複領），印記維持原值。
/// - 上次是昨天（`today - 1`）→ 連續：印記 +1、發獎。
/// - 中間斷過（上次早於昨天）→ 溫和重置為第 1 天、發獎（重新累積，不懲罰）。
/// - 時鐘倒退（`today <= prev_day` 但不等於今天的情況已被上面攔下；這裡保守處理 today < prev_day）
///   → 不前進、不發獎（防刷）。
pub fn advance_streak(prev_day: Option<i64>, prev_streak: u32, today: i64) -> StreakOutcome {
    let new_streak = match prev_day {
        // 從未來訪：踏出第一天。
        None => 1,
        Some(d) => {
            if d == today {
                // 同一天重複上線：維持原印記、不發獎。
                let streak = prev_streak.max(1);
                return StreakOutcome { streak, advanced: false, reward: 0, milestone: false };
            }
            if today < d {
                // 時鐘倒退／異常：保守不動、不發獎，避免被刷。
                let streak = prev_streak.max(1);
                return StreakOutcome { streak, advanced: false, reward: 0, milestone: false };
            }
            if today - d == 1 {
                // 昨天來過：連續 +1。
                prev_streak.max(1).saturating_add(1)
            } else {
                // 斷了至少一天：溫和重新從第 1 天數起。
                1
            }
        }
    };

    let bonus = milestone_bonus(new_streak);
    StreakOutcome {
        streak: new_streak,
        advanced: true,
        reward: REWARD_PER_DAY + bonus,
        milestone: bonus > 0,
    }
}

/// 今天的 UTC 曆日序（自 Unix epoch 起的整數天）。跨時區一致、登入時計算。
/// 與純邏輯解耦：`advance_streak` 只吃整數天，這個 helper 負責把牆上時鐘換算成天序。
pub fn today_utc_day() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    millis / 86_400_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_ever_visit_starts_at_one_and_rewards() {
        let out = advance_streak(None, 0, 100);
        assert_eq!(out.streak, 1);
        assert!(out.advanced);
        assert_eq!(out.reward, REWARD_PER_DAY);
        assert!(!out.milestone);
    }

    #[test]
    fn consecutive_day_increments() {
        let out = advance_streak(Some(100), 1, 101);
        assert_eq!(out.streak, 2);
        assert!(out.advanced);
        assert_eq!(out.reward, REWARD_PER_DAY);
    }

    #[test]
    fn same_day_relogin_does_not_advance_or_reward() {
        let out = advance_streak(Some(100), 5, 100);
        assert_eq!(out.streak, 5);
        assert!(!out.advanced);
        assert_eq!(out.reward, 0);
        assert!(!out.milestone);
    }

    #[test]
    fn gap_resets_to_one_but_still_rewards() {
        // 上次在第 100 天，今天第 104 天：斷了，溫和重置為 1。
        let out = advance_streak(Some(100), 9, 104);
        assert_eq!(out.streak, 1);
        assert!(out.advanced);
        assert_eq!(out.reward, REWARD_PER_DAY);
    }

    #[test]
    fn clock_going_backwards_is_conservative_no_op() {
        let out = advance_streak(Some(100), 6, 98);
        assert_eq!(out.streak, 6);
        assert!(!out.advanced);
        assert_eq!(out.reward, 0);
    }

    #[test]
    fn milestones_grant_warm_bonus() {
        // 第 2 天 → 第 3 天：踏上 3 天里程碑。
        let out = advance_streak(Some(100), 2, 101);
        assert_eq!(out.streak, 3);
        assert!(out.milestone);
        assert_eq!(out.reward, REWARD_PER_DAY + 10);

        assert_eq!(milestone_bonus(7), 25);
        assert_eq!(milestone_bonus(14), 50);
        assert_eq!(milestone_bonus(30), 100);
        assert_eq!(milestone_bonus(60), 100); // 30 天後每滿 30 再給
        assert_eq!(milestone_bonus(90), 100);
    }

    #[test]
    fn non_milestone_days_have_no_bonus() {
        assert_eq!(milestone_bonus(1), 0);
        assert_eq!(milestone_bonus(2), 0);
        assert_eq!(milestone_bonus(4), 0);
        assert_eq!(milestone_bonus(8), 0);
        assert_eq!(milestone_bonus(29), 0);
        assert_eq!(milestone_bonus(31), 0);
    }

    #[test]
    fn reward_is_positive_only_when_advanced() {
        // 前進的各種情形 reward 都 > 0；不前進時恆為 0。
        assert!(advance_streak(None, 0, 1).reward > 0);
        assert!(advance_streak(Some(1), 1, 2).reward > 0);
        assert!(advance_streak(Some(1), 1, 5).reward > 0);
        assert_eq!(advance_streak(Some(5), 3, 5).reward, 0);
        assert_eq!(advance_streak(Some(5), 3, 4).reward, 0);
    }

    #[test]
    fn deterministic_reproducible() {
        let a = advance_streak(Some(200), 6, 201);
        let b = advance_streak(Some(200), 6, 201);
        assert_eq!(a, b);
        assert_eq!(a.streak, 7);
        assert!(a.milestone);
    }

    #[test]
    fn corrupt_zero_prev_streak_treated_as_at_least_one() {
        // 防呆：prev_streak 存成 0（不該發生）時，連日推進仍從 1 起跳到 2，不會停在 1。
        let out = advance_streak(Some(10), 0, 11);
        assert_eq!(out.streak, 2);
    }
}
