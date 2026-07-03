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

// ── 雨後彩虹 v1（ROADMAP 780）─────────────────────────────────────────────────
//
// 至今天氣只有「晴 ↔ 雨」兩態，雨停就默默轉回晴天、天空毫無回饋。本節補上雨過天晴最動人的
// 一筆：一場雨停下的那一刻，天邊掛起一道彩虹，停留一小段時間；附近居民抬頭望見而歡呼一句、
// 心情也跟著亮一格（`mood_boost` 是驅動行為的真狀態，非純美術）。世界的天氣第一次不只有陰鬱，
// 也有雨後放晴的療癒。純邏輯：彩虹以「天氣檢查 tick 數」計時（每 tick≈WEATHER_CHECK_INTERVAL_SECS
// ＝15 秒），整數好窮舉測試；視覺與連線都在前端 / `voxel_ws.rs`。

/// 雨停後彩虹停留的天氣檢查 tick 數（每 tick≈15 秒）：4 tick ≈ 60 秒——夠玩家抬頭望見、
/// 居民歡呼，又不會久留成常態。
pub const RAINBOW_TICKS_AFTER_RAIN: u32 = 4;

/// 純函式：依上一輪彩虹剩餘 tick 數 + 本輪天氣的「雨→晴」轉換，回傳新的剩餘 tick 數。
///
/// - 正在下雨（`now_raining`）→ `0`（雨中不掛彩虹）。
/// - 剛從下雨轉為晴天（`was_raining && !now_raining`）→ 重設為 [`RAINBOW_TICKS_AFTER_RAIN`]（彩虹升起）。
/// - 其餘（持續晴天）→ 每輪遞減 1、減到 0 為止（`saturating_sub` 自然淡出、永不下溢）。
///
/// 確定性、無副作用、可窮舉測試。呼叫端據「上一輪為 0、這一輪 > 0」判定「彩虹剛升起」的一次性事件。
pub fn next_rainbow(prev_ticks: u32, was_raining: bool, now_raining: bool) -> u32 {
    if now_raining {
        0
    } else if was_raining {
        RAINBOW_TICKS_AFTER_RAIN
    } else {
        prev_ticks.saturating_sub(1)
    }
}

/// 雨過天晴、彩虹掛上天邊時，居民抬頭望見而隨機冒出的驚喜台詞池（確定性選句，呼叫端傳 `pick`）。
const RAINBOW_LINES: [&str; 4] = [
    "雨停了……你看，天邊掛著一道彩虹！",
    "哇，好美的彩虹呀～",
    "雨後的彩虹，總讓人覺得一切都會好起來的。",
    "快看那道彩虹！今天真是幸運的一天呢。",
];

/// 依 `pick` 選一句彩虹反應台詞（`pick % len`，永遠有值、確定性、可測）。
pub fn rainbow_line(pick: usize) -> &'static str {
    RAINBOW_LINES[pick % RAINBOW_LINES.len()]
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

    // ── 雨後彩虹 v1（ROADMAP 780）─────────────────────────────────────────────

    #[test]
    fn rainbow_appears_when_rain_just_stopped() {
        // 雨→晴 轉換：無論上一輪剩幾 tick，都重設為滿值（彩虹升起）。
        assert_eq!(next_rainbow(0, true, false), RAINBOW_TICKS_AFTER_RAIN);
        assert_eq!(next_rainbow(2, true, false), RAINBOW_TICKS_AFTER_RAIN);
    }

    #[test]
    fn rainbow_absent_while_raining() {
        // 雨中一律 0（不論上一輪或是否剛開始下）。
        assert_eq!(next_rainbow(RAINBOW_TICKS_AFTER_RAIN, false, true), 0);
        assert_eq!(next_rainbow(3, true, true), 0);
    }

    #[test]
    fn rainbow_fades_one_tick_per_check_when_clear() {
        // 持續晴天：每輪遞減 1、減到 0 為止（不下溢）。
        assert_eq!(next_rainbow(4, false, false), 3);
        assert_eq!(next_rainbow(1, false, false), 0);
        assert_eq!(next_rainbow(0, false, false), 0);
    }

    #[test]
    fn rainbow_lifespan_counts_down_to_zero() {
        // 從雨停升起後，連續 RAINBOW_TICKS_AFTER_RAIN 個晴天 tick 恰好淡出到 0。
        let mut ticks = next_rainbow(0, true, false);
        assert_eq!(ticks, RAINBOW_TICKS_AFTER_RAIN);
        for _ in 0..RAINBOW_TICKS_AFTER_RAIN {
            assert!(ticks > 0, "淡出過程中彩虹應仍在");
            ticks = next_rainbow(ticks, false, false);
        }
        assert_eq!(ticks, 0, "壽命耗盡後彩虹應消失");
    }

    #[test]
    fn rainbow_appear_edge_detectable_from_zero() {
        // 呼叫端用「prev==0 && next>0」判定「彩虹剛升起」的一次性事件——確認此判定成立。
        let prev = 0;
        let next = next_rainbow(prev, true, false);
        assert!(prev == 0 && next > 0, "雨停該可偵測到彩虹升起緣");
    }

    #[test]
    fn rainbow_line_always_non_empty_and_wraps() {
        let len = RAINBOW_LINES.len();
        for pick in 0..(len * 3) {
            assert!(!rainbow_line(pick).is_empty());
        }
        assert_eq!(rainbow_line(0), rainbow_line(len));
        assert_eq!(rainbow_line(3), rainbow_line(3));
    }
}
