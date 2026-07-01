//! 乙太方界·下雨天氣 v1（ROADMAP 700）。
//!
//! 玩家遊玩軸（`docs/PLAN_ETHERVOX.md`「種田」）第一次接上「天氣」維度：世界偶爾下雨，
//! 下雨時所有農地視同水耕（不必湊在水源旁），也讓天空/世界視覺第一次有「非晝夜」變化。
//!
//! 純邏輯：機率式晴/雨切換純函式，無 IO、無鎖、可測；由 `voxel_ws.rs` 每次
//! `tick_farm`（15 秒一輪）呼叫一次、套用進 `WeatherState`，再隨快照廣播 `raining` 給前端。

/// 天氣檢查間隔（秒）——與 `tick_farm` 週期一致，不必額外開 tick 迴圈。
pub const WEATHER_CHECK_INTERVAL_SECS: f32 = 15.0;

/// 晴天時，每次檢查開始下雨的機率（≈ 每 15 秒檢查一次，平均約 12.5 分鐘下一場雨）。
pub const RAIN_START_CHANCE: f32 = 0.02;

/// 下雨時，每次檢查停止下雨的機率（≈ 平均一場雨持續 2.5 分鐘）。
pub const RAIN_STOP_CHANCE: f32 = 0.10;

/// 純函式：給定目前是否下雨 + 一顆隨機骰（呼叫端傳 `rand::random::<f32>()`），
/// 回傳下一輪的下雨狀態。確定性、可測、無副作用。
pub fn next_raining(raining: bool, roll: f32) -> bool {
    if raining {
        roll >= RAIN_STOP_CHANCE
    } else {
        roll < RAIN_START_CHANCE
    }
}

// ── 居民雨天反應 v1（ROADMAP 701）─────────────────────────────────────────────
//
// 天氣至今只影響農地/天空視覺，AI 居民對「正在下雨」這件事毫無反應——本節補上這條連結：
// 雨剛開始下的那一刻，附近居民冒一句應景台詞（純展示、零 LLM），世界的天氣第一次也是
// 居民生活的一部分，不只是背景特效。

/// 雨剛開始下時，居民隨機冒出的應景台詞池（確定性選句，由呼叫端傳 `pick` 索引）。
const RAIN_STARTED_LINES: [&str; 4] = [
    "下雨了，我先進屋躲一下～",
    "咦，這雨來得突然呢！",
    "淋雨會感冒的，回家避一避比較好。",
    "下雨天正好，田裡總算不缺水了。",
];

/// 依 `pick` 選一句雨天反應台詞（`pick % len`，永遠有值、確定性、可測）。
pub fn rain_started_line(pick: usize) -> &'static str {
    RAIN_STARTED_LINES[pick % RAIN_STARTED_LINES.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_raining_when_roll_below_start_threshold() {
        assert!(next_raining(false, 0.0));
        assert!(next_raining(false, RAIN_START_CHANCE - 0.001));
    }

    #[test]
    fn stays_dry_when_roll_at_or_above_start_threshold() {
        assert!(!next_raining(false, RAIN_START_CHANCE));
        assert!(!next_raining(false, 0.99));
    }

    #[test]
    fn keeps_raining_when_roll_at_or_above_stop_threshold() {
        assert!(next_raining(true, RAIN_STOP_CHANCE));
        assert!(next_raining(true, 0.99));
    }

    #[test]
    fn stops_raining_when_roll_below_stop_threshold() {
        assert!(!next_raining(true, 0.0));
        assert!(!next_raining(true, RAIN_STOP_CHANCE - 0.001));
    }

    #[test]
    fn boundary_rolls_never_panic_across_full_range() {
        // 掃過 0.0..1.0 的骰值，兩種狀態都不該 panic，且回傳值恆為 bool。
        let mut roll = 0.0_f32;
        while roll < 1.0 {
            let _ = next_raining(false, roll);
            let _ = next_raining(true, roll);
            roll += 0.05;
        }
    }

    // ── rain_started_line（ROADMAP 701）───────────────────────────────────────

    #[test]
    fn rain_started_line_always_non_empty() {
        for pick in 0..(RAIN_STARTED_LINES.len() * 3) {
            assert!(!rain_started_line(pick).is_empty());
        }
    }

    #[test]
    fn rain_started_line_wraps_deterministically() {
        let len = RAIN_STARTED_LINES.len();
        assert_eq!(rain_started_line(0), rain_started_line(len));
        assert_eq!(rain_started_line(1), rain_started_line(len + 1));
    }

    #[test]
    fn rain_started_line_deterministic_same_pick_same_line() {
        assert_eq!(rain_started_line(2), rain_started_line(2));
    }
}
