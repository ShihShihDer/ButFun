//! 世界風場（ROADMAP 430）——伺服器權威、全服共享的環境風。
//!
//! 讓整個「桌上微縮世界」第一次有了風的概念：所有玩家眼裡的樹冠與作物
//! 朝同一個方向、同一個節拍輕輕搖曳；起風天氣（沙暴最烈、細雨次之）擺得
//! 更明顯，晴天也始終留著一縷微風。
//!
//! 純邏輯層（無 IO、無 WebSocket、無 LLM）：只把「天氣風力 / 強度 / 經過時間」
//! 解析成一個確定性的風向＋風力，供 `WeatherView` 廣播、前端繪製搖曳。
//! 純表現層——風只決定畫面擺動，不影響任何採集／戰鬥／經濟判定。

use serde::Serialize;
use std::f32::consts::TAU;

/// 晴天也保底的一縷微風：世界永遠「活著」，不會徹底靜止。
pub const BREEZE_FLOOR: f32 = 0.18;

/// 風向漂移速率（弧度/秒）：風向隨時間極緩慢轉動，世界不會永遠吹同一個方向。
pub const DIR_DRIFT: f32 = 0.012;

/// 風向相位的週期（秒）：`age` 累加到此就 `rem_euclid` 回捲，避免浮點長大、
/// 又讓風向連續（一整圈 = 一個完整羅盤循環）。
pub const DIR_PERIOD: f32 = TAU / DIR_DRIFT;

/// 風場快照：風向（單位向量）＋風力強度 `[0,1]`。
///
/// `Copy` + `Serialize`：可直接嵌進 `WeatherView` 隨天氣快照廣播。
/// `Default`（全 0）僅供協議佔位建構式用——strength 0 時前端不畫搖曳，dir 不被讀。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Default)]
pub struct Wind {
    /// 風向水平分量（決定樹/作物往左或往右傾）。
    pub dir_x: f32,
    /// 風向垂直分量（湊成單位向量，前端目前主要用水平分量）。
    pub dir_y: f32,
    /// 風力強度，`[0,1]`：愈大樹/作物擺動幅度愈大。
    pub strength: f32,
}

/// 把「此天氣的額外風力 `weather_base`、此刻天氣強度 `intensity`（淡入淡出）、
/// 累積時間 `age_secs`」解析成確定性的 `Wind`。
///
/// - 風力＝`(BREEZE_FLOOR + weather_base × intensity)` 夾在 `[0,1]`：
///   晴天（base 0／intensity 0）也保底 `BREEZE_FLOOR`，起風天氣按強度疊加。
/// - 風向＝`age_secs × DIR_DRIFT` 緩慢轉動的單位向量；`age_secs`／結果非有限時
///   安全退回正東 `(1,0)`，永不產生 NaN 風向。
///
/// 確定性、無副作用、好測。
pub fn wind_for(weather_base: f32, intensity: f32, age_secs: f32) -> Wind {
    let base = if weather_base.is_finite() { weather_base.max(0.0) } else { 0.0 };
    let i = if intensity.is_finite() { intensity.clamp(0.0, 1.0) } else { 0.0 };
    let strength = (BREEZE_FLOOR + base * i).clamp(0.0, 1.0);

    // 風向角：壞值或非有限退正東（避免 NaN 傳到前端）。
    let angle = if age_secs.is_finite() {
        (age_secs * DIR_DRIFT).rem_euclid(TAU)
    } else {
        0.0
    };
    let (dir_x, dir_y) = (angle.cos(), angle.sin());

    Wind { dir_x, dir_y, strength }
}

// ── 單元測試 ─────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    /// 兩浮點近似相等。
    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn clear_weather_keeps_floor_breeze() {
        // 晴天：base 0、intensity 0 → 仍有保底微風，恰等於 BREEZE_FLOOR。
        let w = wind_for(0.0, 0.0, 0.0);
        assert!(close(w.strength, BREEZE_FLOOR), "晴天微風應為 {BREEZE_FLOOR}，得 {}", w.strength);
        assert!(w.strength > 0.0, "世界永遠有一縷風、不徹底靜止");
    }

    #[test]
    fn sandstorm_peak_is_strongest_and_clamped() {
        // 沙暴 base 0.8、滿強度 → 0.18+0.8 = 0.98，仍在 [0,1]。
        let storm = wind_for(0.8, 1.0, 0.0);
        let rain = wind_for(0.35, 1.0, 0.0);
        assert!(storm.strength > rain.strength, "沙暴應比細雨更強");
        assert!(storm.strength <= 1.0 && storm.strength >= 0.0);
        // 即便 base 異常大也夾到 1，不爆表。
        let crazy = wind_for(5.0, 1.0, 0.0);
        assert!(close(crazy.strength, 1.0), "超大風力應夾到 1");
    }

    #[test]
    fn strength_always_within_unit_range() {
        for &base in &[0.0, 0.2, 0.35, 0.8, 3.0] {
            for &i in &[0.0, 0.25, 0.5, 1.0] {
                let s = wind_for(base, i, 0.0).strength;
                assert!((0.0..=1.0).contains(&s), "strength {s} 應在 [0,1]");
            }
        }
    }

    #[test]
    fn intensity_fades_extra_wind() {
        // 同天氣下，強度愈大額外風力愈強（淡入淡出反映在風力上）。
        let lo = wind_for(0.8, 0.2, 0.0).strength;
        let hi = wind_for(0.8, 0.9, 0.0).strength;
        assert!(hi > lo, "強度大時風力應更強：lo={lo} hi={hi}");
    }

    #[test]
    fn direction_is_unit_vector() {
        for &age in &[0.0, 10.0, 123.4, 999.0, DIR_PERIOD * 0.5] {
            let w = wind_for(0.3, 1.0, age);
            let mag = (w.dir_x * w.dir_x + w.dir_y * w.dir_y).sqrt();
            assert!(close(mag, 1.0), "age={age} 方向應為單位向量，長度得 {mag}");
        }
    }

    #[test]
    fn direction_rotates_with_age() {
        // 時間推進，風向應改變（不卡死同一方向）。
        let a = wind_for(0.3, 1.0, 0.0);
        let b = wind_for(0.3, 1.0, 100.0);
        assert!(!close(a.dir_x, b.dir_x) || !close(a.dir_y, b.dir_y), "風向應隨時間轉動");
    }

    #[test]
    fn direction_wraps_one_full_period() {
        // 走滿一個 DIR_PERIOD 應回到起點（rem_euclid 回捲、方向連續）。
        let a = wind_for(0.3, 1.0, 0.0);
        let b = wind_for(0.3, 1.0, DIR_PERIOD);
        assert!(close(a.dir_x, b.dir_x) && close(a.dir_y, b.dir_y), "滿一週期風向應回到起點");
    }

    #[test]
    fn non_finite_inputs_degrade_safely() {
        // 壞值不應產生 NaN：方向退正東、強度仍有限且在範圍內。
        let w = wind_for(f32::NAN, f32::INFINITY, f32::NAN);
        assert!(w.dir_x.is_finite() && w.dir_y.is_finite(), "方向不得為 NaN");
        assert!(close(w.dir_x, 1.0) && close(w.dir_y, 0.0), "壞 age 應退正東");
        assert!((0.0..=1.0).contains(&w.strength), "壞值強度仍應在 [0,1]");
    }

    #[test]
    fn deterministic_same_input_same_output() {
        assert_eq!(wind_for(0.35, 0.7, 42.0), wind_for(0.35, 0.7, 42.0));
    }

    #[test]
    fn default_is_calm_zero() {
        // 協議佔位用的 Default：strength 0（前端不畫搖曳）。
        let d = Wind::default();
        assert_eq!(d.strength, 0.0);
    }
}
