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
}
