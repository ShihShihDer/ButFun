//! 釣魚系統（ROADMAP 47）。
//!
//! 玩家站在水域邊緣（80px 內有 Water 生態域格）即可垂釣。
//! 每次釣魚有 5 秒冷卻；成功上鉤後依機率分配三種魚：
//!   - 小魚    70%（FishSmall）
//!   - 星星魚  25%（FishStar）
//!   - 深海魚   5%（FishDeep）
//! 每次釣魚給 10 點農夫熟練度 XP（讓農夫路線不只是種田）。

use world_core::{biome_at, Biome};

use crate::inventory::ItemKind;

/// 釣魚冷卻秒數。
pub const FISH_COOLDOWN_SECS: f32 = 5.0;

/// 農夫熟練度 XP（每次釣魚）。
pub const FISH_FARMER_XP: u32 = 10;

/// 判定玩家是否站在水域邊緣（`FISH_REACH` 像素內有至少一格 Water biome）。
pub fn is_near_water(px: f32, py: f32) -> bool {
    const FISH_REACH: f32 = 80.0;
    const STEP: f32 = 24.0; // 採樣間距（比 tile 小，確保不漏掉邊緣格）

    let mut dx = -FISH_REACH;
    while dx <= FISH_REACH {
        let mut dy = -FISH_REACH;
        while dy <= FISH_REACH {
            if dx * dx + dy * dy <= FISH_REACH * FISH_REACH
                && biome_at((px + dx) as f64, (py + dy) as f64) == Biome::Water
            {
                return true;
            }
            dy += STEP;
        }
        dx += STEP;
    }
    false
}

/// ROADMAP 475 打水漂：回傳由玩家指向「最近一格水域」的單位方向（甩石朝水面飛）。
/// 與 `is_near_water` 同一套採樣（`FISH_REACH` 內），但取最近的 Water 格、回正規化方向；
/// 周遭沒有水域則回 `None`（呼叫端應已先以 `is_near_water` 把關，理論上不會走到）。
/// 站在水格正上方（offset≈0）時退而回一個預設方向（向右），避免除以零。
pub fn water_dir_near(px: f32, py: f32) -> Option<(f32, f32)> {
    const FISH_REACH: f32 = 80.0;
    const STEP: f32 = 24.0;

    let mut best: Option<(f32, f32, f32)> = None; // (dist2, dx, dy)
    let mut dx = -FISH_REACH;
    while dx <= FISH_REACH {
        let mut dy = -FISH_REACH;
        while dy <= FISH_REACH {
            let d2 = dx * dx + dy * dy;
            if d2 <= FISH_REACH * FISH_REACH
                && biome_at((px + dx) as f64, (py + dy) as f64) == Biome::Water
                && best.map_or(true, |(bd2, _, _)| d2 < bd2)
            {
                best = Some((d2, dx, dy));
            }
            dy += STEP;
        }
        dx += STEP;
    }
    best.map(|(d2, bdx, bdy)| {
        let len = d2.sqrt();
        if len > 1e-3 {
            (bdx / len, bdy / len)
        } else {
            (1.0, 0.0) // 正站在水格上：退一個預設方向，永不除零
        }
    })
}

/// 依確定性種子決定上鉤的魚種（小魚 70%、星星魚 25%、深海魚 5%）。
///
/// 種子建議帶入 `player_id_u64 ^ tick ^ fish_attempt_count`，確保每次釣魚結果不同。
pub fn roll_fish(seed: u64) -> ItemKind {
    match seed % 100 {
        0..=69  => ItemKind::FishSmall,
        70..=94 => ItemKind::FishStar,
        _       => ItemKind::FishDeep,
    }
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 機率分布：10 000 次採樣，允許 ±5% 誤差。
    #[test]
    fn fish_roll_distribution_roughly_matches_spec() {
        let (mut small, mut star, mut deep) = (0u32, 0u32, 0u32);
        for i in 0u64..10_000 {
            match roll_fish(i.wrapping_mul(7).wrapping_add(13)) {
                ItemKind::FishSmall => small += 1,
                ItemKind::FishStar  => star  += 1,
                ItemKind::FishDeep  => deep  += 1,
                _ => panic!("roll_fish 回傳非魚物品"),
            }
        }
        assert!(small > 6_500 && small < 7_500, "小魚比例異常: {small}/10000");
        assert!(star  > 2_000 && star  < 3_000, "星星魚比例異常: {star}/10000");
        assert!(deep  > 300   && deep  < 700,   "深海魚比例異常: {deep}/10000");
    }

    /// roll_fish 一定回傳 Fish* 之一。
    #[test]
    fn fish_roll_always_returns_fish_kind() {
        for seed in 0u64..200 {
            let kind = roll_fish(seed);
            assert!(
                matches!(kind, ItemKind::FishSmall | ItemKind::FishStar | ItemKind::FishDeep),
                "roll_fish({seed}) 回傳非魚物品: {kind:?}"
            );
        }
    }

    /// seed mod 100 覆蓋三個分支。
    #[test]
    fn fish_roll_covers_all_three_kinds() {
        assert_eq!(roll_fish(0),  ItemKind::FishSmall);  // 0  → 小魚
        assert_eq!(roll_fish(70), ItemKind::FishStar);   // 70 → 星星魚
        assert_eq!(roll_fish(95), ItemKind::FishDeep);   // 95 → 深海魚
    }

    /// is_near_water：公共農地中心（確定是 Meadow 生態域，不在水邊）不應觸發。
    #[test]
    fn is_near_water_false_on_pub_field() {
        // 公共農地在 (2200+, 2200+) 的 Meadow 生態域，不應觸發水邊判定。
        assert!(!is_near_water(2700.0, 2300.0));
    }

    /// is_near_water：函式可正常呼叫且回傳 bool（可呼叫性確認）。
    #[test]
    fn is_near_water_callable() {
        // 只驗函式可呼叫，不對特定座標的 biome 做假設。
        let _ = is_near_water(3000.0, 3000.0);
    }

    /// FISH_COOLDOWN_SECS 在合理範圍（1~30 秒）。
    #[test]
    fn fish_cooldown_is_reasonable() {
        assert!(FISH_COOLDOWN_SECS >= 1.0 && FISH_COOLDOWN_SECS <= 30.0);
    }

    /// FISH_FARMER_XP 為正。
    #[test]
    fn fish_farmer_xp_is_positive() {
        assert!(FISH_FARMER_XP > 0);
    }
}
