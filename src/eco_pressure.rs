//! ROADMAP 166：生態壓力計算——獸潮從生態狀態長出，不再純計時器。
//!
//! `compute_eco_pressure` 讀取三個指標，回傳 0.0～100.0 的生態壓力值：
//!
//! 1. **巢穴過剩壓力（0～40）**：族群飽和度超過 50% 的部分換算為壓力；
//!    巢穴越滿→物種數量越過剩→越急於擴張→攻城壓力+。
//! 2. **怪物敵視壓力（0～40）**：怪物種類越多處於敵視/警覺態度→壓力+；
//!    玩家積欠「血債」越多，怪物越想報復。
//! 3. **獵物食物短缺壓力（0～20）**：玩家過度獵殺野生獵物物種（WildBird/WildDeer/SmallCritter）
//!    → 態度跌低 → 食物鏈斷裂 → 怪物飢餓 → 攻城壓力+。
//!
//! 純函式，零 LLM，零 migration，零副作用。

/// 巢穴族群輸入（傳入 population 與 max_population 供計算飽和度）。
#[derive(Debug, Clone)]
pub struct ColonyPressureInput {
    pub population:     u32,
    pub max_population: u32,
}

/// 計算生態壓力（0.0～100.0）。
///
/// * `colonies` — 所有怪物巢穴的族群狀態。
/// * `monster_hostile_count` — 目前處於「敵視」層級的怪物種類數。
/// * `monster_wary_count` — 目前處於「警覺」層級的怪物種類數。
/// * `monster_total` — 受追蹤的怪物種類總數（用於正規化）。
/// * `prey_avg_attitude` — 獵物物種的平均態度值（0-100）；低 = 過度被獵殺 = 食物短缺。
pub fn compute_eco_pressure(
    colonies:              &[ColonyPressureInput],
    monster_hostile_count: u32,
    monster_wary_count:    u32,
    monster_total:         u32,
    prey_avg_attitude:     i32,
) -> f32 {
    // 1. 巢穴過剩壓力：飽和度超過 50% 的部分折算壓力（50%=0, 100%=40）。
    let colony_pressure = if colonies.is_empty() {
        0.0_f32
    } else {
        let avg_sat: f32 = colonies.iter()
            .filter(|c| c.max_population > 0)
            .map(|c| c.population as f32 / c.max_population as f32)
            .sum::<f32>()
            / colonies.len() as f32;
        // avg_sat 0.5 → 0, 1.0 → 40
        let excess = (avg_sat - 0.5).max(0.0) * 2.0; // 0.0～1.0
        (excess * 40.0).min(40.0)
    };

    // 2. 怪物敵視壓力：敵視種 ×2 + 警覺種 ×1，對總種數歸一化後映射到 0～40。
    let hostility_pressure = if monster_total == 0 {
        0.0_f32
    } else {
        let score     = (monster_hostile_count * 2 + monster_wary_count) as f32;
        let max_score = (monster_total * 2) as f32;
        (score / max_score * 40.0).min(40.0)
    };

    // 3. 食物短缺壓力：態度值 < 40 的部分換算為短缺（40→0, 0→20）。
    let scarcity_pressure = {
        let shortage = (40 - prey_avg_attitude.clamp(0, 40)) as f32;
        // shortage 0→40 → 壓力 0→20
        (shortage / 40.0 * 20.0).min(20.0)
    };

    (colony_pressure + hostility_pressure + scarcity_pressure).min(100.0)
}

// ─── 測試 ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn half_colonies(n: usize) -> Vec<ColonyPressureInput> {
        (0..n).map(|_| ColonyPressureInput { population: 4, max_population: 8 }).collect()
    }

    fn full_colonies(n: usize) -> Vec<ColonyPressureInput> {
        (0..n).map(|_| ColonyPressureInput { population: 8, max_population: 8 }).collect()
    }

    fn empty_colonies(n: usize) -> Vec<ColonyPressureInput> {
        (0..n).map(|_| ColonyPressureInput { population: 0, max_population: 8 }).collect()
    }

    #[test]
    fn balanced_ecology_low_pressure() {
        // 50% 飽和度、無敵視/警覺、獵物態度中立 → 接近 0
        let p = compute_eco_pressure(&half_colonies(5), 0, 0, 14, 50);
        // 巢穴 50% → excess = 0 → colony_pressure = 0；hostility = 0；scarcity 態度 50 > 40 → 0
        assert!(p < 5.0, "生態均衡時壓力應接近 0，實際 {p}");
    }

    #[test]
    fn full_colonies_add_maximum_colony_pressure() {
        // 100% 飽和度 → colony_pressure = 40
        let p = compute_eco_pressure(&full_colonies(5), 0, 0, 14, 50);
        let colony = p; // 只有巢穴貢獻
        assert!((colony - 40.0).abs() < 0.1, "全滿巢穴應帶 40 壓力，實際 {colony}");
    }

    #[test]
    fn empty_colonies_zero_colony_pressure() {
        // 族群全空 → colony_pressure = 0（avg_sat = 0 < 0.5）
        let p = compute_eco_pressure(&empty_colonies(5), 0, 0, 14, 50);
        assert!(p < 1.0, "空巢穴不帶壓力，實際 {p}");
    }

    #[test]
    fn hostile_monsters_raise_pressure() {
        // 14 種全部敵視 → hostile_count=14, wary=0 → score=28, max=28 → 40
        let p = compute_eco_pressure(&half_colonies(5), 14, 0, 14, 50);
        assert!((p - 40.0).abs() < 0.5, "全敵視應貢獻 40 壓力，實際 {p}");
    }

    #[test]
    fn wary_monsters_add_partial_pressure() {
        // 7 種警覺 → score=7, max=28 → 10
        let p = compute_eco_pressure(&empty_colonies(5), 0, 7, 14, 50);
        assert!((p - 10.0).abs() < 0.5, "7 警覺種應貢獻 ~10 壓力，實際 {p}");
    }

    #[test]
    fn prey_overhunted_adds_scarcity_pressure() {
        // 獵物態度 0 → shortage = 40 → scarcity = 20
        let p = compute_eco_pressure(&empty_colonies(5), 0, 0, 14, 0);
        assert!((p - 20.0).abs() < 0.5, "態度 0 應貢獻 20 食物短缺壓力，實際 {p}");
    }

    #[test]
    fn prey_healthy_no_scarcity() {
        // 獵物態度 ≥ 40 → scarcity = 0
        let p = compute_eco_pressure(&empty_colonies(5), 0, 0, 14, 60);
        assert!(p < 0.5, "獵物態度健康（≥40）不加食物短缺壓力，實際 {p}");
    }

    #[test]
    fn pressure_caps_at_100() {
        // 最壞情況：全滿巢穴 + 全敵視 + 食物枯竭
        let p = compute_eco_pressure(&full_colonies(5), 14, 14, 14, 0);
        assert!(p <= 100.0, "壓力不應超過 100，實際 {p}");
        assert!(p > 90.0, "最壞情況壓力應接近 100，實際 {p}");
    }

    #[test]
    fn empty_colony_list_no_panic() {
        // 空巢穴清單不 panic
        let p = compute_eco_pressure(&[], 0, 0, 0, 50);
        assert_eq!(p, 0.0);
    }
}
