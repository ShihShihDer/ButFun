//! 季節豐收獎（ROADMAP 493）。
//!
//! 在此之前，品種的季節偏好（`CropVariety::season_affinity`）只默默影響成長速度——
//! 玩家很難感受到「在對的季節種對的品種」這件事有多值得。
//! 本切片把「在當季旺季收穫旺作」這個動作，第一次變成一個看得見的獎勵瞬間：
//! 額外 3 乙太 + 專屬飄字「🌾 當季旺收！」，讓季節感不再只是隱形的成長倍率，
//! 而是收成瞬間的驚喜與肯定。
//!
//! 伺服器追蹤**本季全服旺收次數**，每逢里程碑（10、30、100、每整 100）廣播一則
//! 「豐收節公告」，讓更多玩家注意到這個獎勵並感受到集體成就。
//!
//! 與既有系統的正交性：
//! - ROADMAP 453 季節成長（`season_affinity`）：旺季作物長得快——成長側的隱性紅利。
//! - ROADMAP 455 市集行情（`crop_demand`）：刻意把搶手季排在**淡季**——
//!   讓玩家在「順季種得快」vs「逆季賣得貴」之間取捨。
//! - 本模組（ROADMAP 493）：旺季收穫給明確的**正向慶祝**——三條各自獨立、三者同達最甜。
//!
//! 純邏輯、無 IO、無副作用，易於單元測試。

use crate::crop_variety::CropVariety;
use crate::season::Season;

/// 當季旺收：品種在自己的旺季（`peak_season`）被收穫時觸發。
/// `Staple`（主食穀）無偏好季節，永遠回 `false`；
/// `Sprout`（速生菜）冬天旺、`Etherbloom`（乙太瓜）夏天旺。
/// 純函式、確定性、無副作用。
pub fn is_peak_harvest(variety: CropVariety, season: Season) -> bool {
    variety.peak_season().map_or(false, |peak| peak == season)
}

/// 當季旺收的固定乙太獎勵（每株，不含品質加成與市集溢價——三條各自獨立疊加）。
pub const SEASON_AWARD_BONUS: u32 = 3;

/// 豐收節全服公告文字；`count` 為本季旺收累計次數。
/// 里程碑：10、30、100、此後每整 100 次，避免次數爆增後每次都公告。
/// 純函式。
pub fn milestone_announce(count: u32, season: Season) -> Option<String> {
    if count == 10 || count == 30 || count == 100 || (count > 100 && count % 100 == 0) {
        Some(format!(
            "🌾 【豐收節】{}旺收達 {} 次！越來越多旅人掌握了時令農耕的訣竅！",
            season.display_name(),
            count,
        ))
    } else {
        None
    }
}

// ─── 純邏輯單元測試 ───────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    // ── is_peak_harvest ──────────────────────────────────────────────────────

    #[test]
    fn sprout_peaks_in_winter() {
        assert!(is_peak_harvest(CropVariety::Sprout, Season::Winter), "速生菜冬天旺");
    }

    #[test]
    fn sprout_not_peak_in_summer() {
        assert!(!is_peak_harvest(CropVariety::Sprout, Season::Summer), "速生菜夏天非旺");
    }

    #[test]
    fn sprout_not_peak_in_spring() {
        assert!(!is_peak_harvest(CropVariety::Sprout, Season::Spring));
    }

    #[test]
    fn sprout_not_peak_in_autumn() {
        assert!(!is_peak_harvest(CropVariety::Sprout, Season::Autumn));
    }

    #[test]
    fn etherbloom_peaks_in_summer() {
        assert!(is_peak_harvest(CropVariety::Etherbloom, Season::Summer), "乙太瓜夏天旺");
    }

    #[test]
    fn etherbloom_not_peak_in_winter() {
        assert!(!is_peak_harvest(CropVariety::Etherbloom, Season::Winter));
    }

    #[test]
    fn staple_never_peaks() {
        for season in [Season::Spring, Season::Summer, Season::Autumn, Season::Winter] {
            assert!(
                !is_peak_harvest(CropVariety::Staple, season),
                "主食穀 {:?} 季不應旺收",
                season
            );
        }
    }

    // ── SEASON_AWARD_BONUS ───────────────────────────────────────────────────

    #[test]
    fn bonus_is_positive() {
        assert!(SEASON_AWARD_BONUS > 0, "旺收獎勵必須為正");
    }

    // ── milestone_announce ───────────────────────────────────────────────────

    #[test]
    fn milestone_at_10() {
        assert!(milestone_announce(10, Season::Spring).is_some());
    }

    #[test]
    fn milestone_at_30() {
        assert!(milestone_announce(30, Season::Summer).is_some());
    }

    #[test]
    fn milestone_at_100() {
        assert!(milestone_announce(100, Season::Autumn).is_some());
    }

    #[test]
    fn milestone_at_200() {
        assert!(milestone_announce(200, Season::Winter).is_some(), "每整 100 次應公告");
    }

    #[test]
    fn no_milestone_at_5() {
        assert!(milestone_announce(5, Season::Spring).is_none());
    }

    #[test]
    fn no_milestone_at_50() {
        assert!(milestone_announce(50, Season::Summer).is_none());
    }

    #[test]
    fn no_milestone_at_150() {
        assert!(milestone_announce(150, Season::Autumn).is_none(), "150 非整 100、非特殊值，不公告");
    }

    #[test]
    fn milestone_text_contains_count_and_season() {
        let text = milestone_announce(10, Season::Spring).unwrap();
        assert!(text.contains("10"), "公告應包含次數");
        assert!(text.contains("春"), "公告應包含季節名稱");
    }

    #[test]
    fn milestone_at_300_includes_count() {
        let text = milestone_announce(300, Season::Winter).unwrap();
        assert!(text.contains("300"));
    }
}
