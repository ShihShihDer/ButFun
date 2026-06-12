//! 旅人野外見聞（ROADMAP 171）——旅人抵達時根據生態狀況廣播見聞。
//!
//! 純函式、零 LLM、零 migration、零副作用。
//! 生態壓力高 / Alpha 活躍 / 巢穴密集 → 帶警示；生態平靜 → 靜音。

/// 旅人帶回的生態快報輸入（純資料，無外部相依）。
#[derive(Debug, Clone)]
pub struct EcoReportContext {
    /// 目前生態壓力（0.0–100.0）。
    pub pressure: f32,
    /// 目前活躍的 Alpha 首領數量。
    pub active_alpha_count: usize,
    /// 族群最密集的巢穴名稱（若有）。
    pub top_colony_name: Option<String>,
}

/// 依生態狀況挑選旅人帶回的見聞台詞。
///
/// 生態平靜（壓力 < 40 且無 Alpha）時回 None，不廣播多餘訊息。
pub fn pick_eco_report(traveler_name: &str, ctx: &EcoReportContext) -> Option<String> {
    // 最高優先：多 Alpha 互搏 + 高壓——雙重危機
    if ctx.active_alpha_count >= 2 && ctx.pressure >= 60.0 {
        let colony = ctx.top_colony_name.as_deref().unwrap_or("野外");
        return Some(format!(
            "🌿 【旅人目擊・{traveler_name}】野外情況危急！兩頭 Alpha 霸主正在廝殺、{colony} 巢穴已飽和。所有人請做好戰備！"
        ));
    }
    // 單 Alpha + 中高壓——帶頭目警示
    if ctx.active_alpha_count >= 1 && ctx.pressure >= 50.0 {
        let colony = ctx.top_colony_name.as_deref().unwrap_or("野外某處");
        return Some(format!(
            "🌿 【旅人警示・{traveler_name}】{colony} 附近有戴冠的怪物頭目在巡邏，生態壓力也在上升——出去前備好藥。"
        ));
    }
    // 高壓無 Alpha——族群爆炸
    if ctx.pressure >= 75.0 {
        let colony = ctx.top_colony_name.as_deref().unwrap_or("東側野外");
        return Some(format!(
            "🌿 【旅人目擊・{traveler_name}】野外怪物密度超高，{colony} 一帶族群爆滿，輕裝旅人慎入！"
        ));
    }
    // 有 Alpha 但壓力低——目擊報告
    if ctx.active_alpha_count >= 1 {
        let colony = ctx.top_colony_name.as_deref().unwrap_or("野外");
        return Some(format!(
            "🌿 【旅人見聞・{traveler_name}】路過 {colony} 時目擊頭戴金冠的強力怪物，不像普通野怪。有備而來。"
        ));
    }
    // 中等壓力——留意建議
    if ctx.pressure >= 40.0 {
        let colony = ctx.top_colony_name.as_deref().unwrap_or("野外");
        return Some(format!(
            "🌿 【旅人見聞・{traveler_name}】野外不算太亂，但 {colony} 方向有些活躍，採集的留意一下。"
        ));
    }
    // 生態平靜，不廣播
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(pressure: f32, alpha: usize, colony: Option<&str>) -> EcoReportContext {
        EcoReportContext {
            pressure,
            active_alpha_count: alpha,
            top_colony_name: colony.map(str::to_string),
        }
    }

    #[test]
    fn dual_alpha_high_pressure_returns_crisis() {
        let r = pick_eco_report("科拿", &ctx(70.0, 2, Some("廢料無人機陣")));
        assert!(r.is_some());
        let msg = r.unwrap();
        assert!(msg.contains("兩頭"), "應提到兩頭 Alpha");
        assert!(msg.contains("廢料無人機陣"), "應包含巢穴名稱");
    }

    #[test]
    fn single_alpha_medium_pressure_returns_warning() {
        let r = pick_eco_report("妮維", &ctx(55.0, 1, Some("水晶傀儡巢穴")));
        assert!(r.is_some());
        assert!(r.unwrap().contains("水晶傀儡巢穴"));
    }

    #[test]
    fn high_pressure_no_alpha_returns_density_warning() {
        let r = pick_eco_report("歐爾", &ctx(80.0, 0, Some("靈蛾巢")));
        assert!(r.is_some());
        assert!(r.unwrap().contains("靈蛾巢"), "應提到巢穴名稱");
    }

    #[test]
    fn single_alpha_low_pressure_returns_sighting() {
        let r = pick_eco_report("璃安", &ctx(10.0, 1, Some("蘑菇窟")));
        assert!(r.is_some());
        let msg = r.unwrap();
        assert!(msg.contains("金冠") || msg.contains("頭目") || msg.contains("強力"));
    }

    #[test]
    fn medium_pressure_no_alpha_returns_caution() {
        let r = pick_eco_report("鐵克", &ctx(45.0, 0, Some("乙太霧潭")));
        assert!(r.is_some());
        assert!(r.unwrap().contains("乙太霧潭"));
    }

    #[test]
    fn low_pressure_no_alpha_returns_none() {
        let r = pick_eco_report("科拿", &ctx(10.0, 0, None));
        assert!(r.is_none(), "生態平靜時不廣播");
    }

    #[test]
    fn pressure_39_no_alpha_returns_none() {
        let r = pick_eco_report("妮維", &ctx(39.9, 0, Some("任何巢穴")));
        assert!(r.is_none(), "壓力 <40 且無 Alpha 應靜音");
    }

    #[test]
    fn traveler_name_in_all_messages() {
        let name = "測試旅人";
        for (p, a) in [(80.0, 0), (80.0, 1), (80.0, 2), (45.0, 0)] {
            let r = pick_eco_report(name, &ctx(p, a, Some("測試巢穴")));
            if let Some(msg) = r {
                assert!(msg.contains(name), "旅人名字要出現在訊息中 p={p} a={a}");
            }
        }
    }

    #[test]
    fn dual_alpha_low_pressure_uses_alpha_path() {
        // 2 隻 Alpha 但壓力低 → 走「alpha >= 1」分支，仍應回 Some
        let r = pick_eco_report("鐵克", &ctx(15.0, 2, Some("廢料無人機陣")));
        assert!(r.is_some(), "有 Alpha 即使壓力低仍應回報");
    }

    #[test]
    fn no_colony_name_uses_fallback_text() {
        let r = pick_eco_report("科拿", &ctx(80.0, 1, None));
        assert!(r.is_some());
        assert!(!r.unwrap().is_empty());
    }

    #[test]
    fn pressure_boundary_exactly_75_triggers_density_path() {
        // 無 Alpha，壓力 == 75 應走「高壓無 Alpha」路徑
        let r = pick_eco_report("妮維", &ctx(75.0, 0, Some("乙太霧潭")));
        assert!(r.is_some(), "壓力 75 無 Alpha 應警告");
    }
}
