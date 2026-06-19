//! ROADMAP 407 拿手菜·越煮越上手——同一道料理煮得越多次越熟練，你的拿手菜吃下去暖食更久更暖。
//!
//! 料理這條維度此前有 349 掌勺照譜（順序記憶小遊戲）與 395 暖食飽足（進食後的限時回血 buff），
//! 但「煮過幾次」對你毫無影響——第一次煎的蛋與煎到第一百次的蛋，吃下去的飽足一模一樣，料理本身
//! 沒有任何「越做越精」的積累。本模組給料理第一個成長維度：每道料理各自累積烹煮次數，熟練分三階
//! （生手 → 順手 → 拿手），階位越高，那道料理吃下去的暖食飽足**持續更久、回血更暖**——你常下廚的
//! 拿手菜，最能撫慰你。於是「常煮同一道把它練成招牌」第一次有了意義。
//!
//! ## 設計鐵律
//! - **記憶體前置、零持久化、零 migration**：烹煮次數記在 `Player` 上的記憶體欄（鏡像 `meal_buff` /
//!   `pet` / `ranching` / `kill_streak` 的記憶體模式），重啟清零、重新累積。
//! - **療癒向、零平衡風險**：熟練只放大暖食 buff（純緩慢回 HP，見 `meal_buff`），**不送物品／乙太／
//!   戰力、不改進食的即時回血量、不開任何新獎勵路徑**——拿手只讓「飽足」更綿長，與戰鬥／經濟正交。
//! - **純函式可測**：階位查表（`tier_for`）與 buff 縮放（`scale_meal`）皆與 IO／鎖無關、確定可重現。

use std::collections::BTreeMap;

use crate::inventory::ItemKind;
use crate::meal_buff::MealBuff;

/// 累積烹煮到此次數（含）→ 升「順手」。
pub const SKILLED_AT: u32 = 5;
/// 累積烹煮到此次數（含）→ 升「拿手」（招牌料理）。
pub const SIGNATURE_AT: u32 = 15;

/// 一道料理的熟練階位——由累積烹煮次數推導。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DishTier {
    /// 生手：剛開始煮這道，尚無加成（與既有暖食完全相同）。
    Novice,
    /// 順手：煮過一陣子，暖食略久略暖。
    Skilled,
    /// 拿手：你的招牌料理，暖食最綿長最暖。
    Signature,
}

impl DishTier {
    /// 前端 wire key（snake_case）。生手不會被廣播（無加成、省流量）。
    pub fn wire_str(self) -> &'static str {
        match self {
            DishTier::Novice => "novice",
            DishTier::Skilled => "skilled",
            DishTier::Signature => "signature",
        }
    }

    /// 有加成的階位才回 `Some(wire key)`——供快照「只在順手以上才標記」（生手回 None 省流量）。
    pub fn badge(self) -> Option<&'static str> {
        match self {
            DishTier::Novice => None,
            other => Some(other.wire_str()),
        }
    }

    /// 暖食總時長倍率：拿手菜飽足更久（溫和，療癒向）。
    pub fn duration_mult(self) -> f32 {
        match self {
            DishTier::Novice => 1.0,
            DishTier::Skilled => 1.20,
            DishTier::Signature => 1.45,
        }
    }

    /// 暖食每秒回血倍率：拿手菜回得更暖（溫和；總量仍是緩慢回 HP，不碰戰鬥平衡）。
    pub fn regen_mult(self) -> f32 {
        match self {
            DishTier::Novice => 1.0,
            DishTier::Skilled => 1.10,
            DishTier::Signature => 1.25,
        }
    }
}

/// 由累積烹煮次數推導熟練階位（門檻查表）。
pub fn tier_for(count: u32) -> DishTier {
    if count >= SIGNATURE_AT {
        DishTier::Signature
    } else if count >= SKILLED_AT {
        DishTier::Skilled
    } else {
        DishTier::Novice
    }
}

/// 把一份暖食 buff 依熟練階位放大（總時長＋每秒回血）。`None`（非料理）原樣傳回，
/// 讓進食接線可無腦套用：`scale_meal(meal_buff_for(item), tier_of(item))`。
pub fn scale_meal(buff: Option<MealBuff>, tier: DishTier) -> Option<MealBuff> {
    buff.map(|b| b.nourished(tier.duration_mult(), tier.regen_mult()))
}

/// 一次烹煮記錄的結果——新次數、新階位、是否「剛好跨過門檻升階」（供前端慶賀飄字）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CookRecord {
    /// 煮出的料理。
    pub item: ItemKind,
    /// 這道料理累積烹煮次數（含本次）。
    pub count: u32,
    /// 本次烹煮後的熟練階位。
    pub tier: DishTier,
    /// 本次是否「剛好升階」（前一次與本次的階位不同）。
    pub tier_up: bool,
}

/// 玩家對各道料理的烹煮熟練（記憶體前置、零持久化）。
#[derive(Debug, Clone, Default)]
pub struct DishMastery {
    /// 各料理累積烹煮次數。只記「料理」（吃了帶暖食 buff 的）；非料理（工具等）不入帳。
    /// 用 `BTreeMap`（`ItemKind: Ord`）而非 `HashMap`：料理至多 7 種、不需雜湊，且順序確定好測。
    counts: BTreeMap<ItemKind, u32>,
}

impl DishMastery {
    /// 查某道料理累積烹煮次數（沒煮過＝0）。
    pub fn count_of(&self, item: ItemKind) -> u32 {
        self.counts.get(&item).copied().unwrap_or(0)
    }

    /// 查某道料理目前的熟練階位（沒煮過＝生手）。
    pub fn tier_of(&self, item: ItemKind) -> DishTier {
        tier_for(self.count_of(item))
    }

    /// 記一次「煮出某道料理」。只有真正的料理（吃了有暖食 buff）才入帳並回 `Some`；
    /// 非料理（工具等）回 `None`、不入帳。回傳的 `CookRecord` 含本次是否升階。
    pub fn record_cook(&mut self, item: ItemKind) -> Option<CookRecord> {
        // 「是不是料理」以暖食 buff 為唯一權威來源——與 395 的料理集合永遠一致，不另列白名單。
        if crate::meal_buff::meal_buff_for(item).is_none() {
            return None;
        }
        let before = self.count_of(item);
        let after = before.saturating_add(1);
        self.counts.insert(item, after);
        let tier = tier_for(after);
        Some(CookRecord {
            item,
            count: after,
            tier,
            // 熟練只增不減，故「階位變了」⇔「剛好跨過某道門檻」。
            tier_up: tier != tier_for(before),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_thresholds() {
        assert_eq!(tier_for(0), DishTier::Novice);
        assert_eq!(tier_for(4), DishTier::Novice);
        assert_eq!(tier_for(SKILLED_AT), DishTier::Skilled); // 5
        assert_eq!(tier_for(14), DishTier::Skilled);
        assert_eq!(tier_for(SIGNATURE_AT), DishTier::Signature); // 15
        assert_eq!(tier_for(999), DishTier::Signature);
    }

    #[test]
    fn higher_tier_means_longer_and_warmer() {
        // 倍率單調遞增（生手 < 順手 < 拿手），且生手＝原樣（1.0）。
        assert_eq!(DishTier::Novice.duration_mult(), 1.0);
        assert_eq!(DishTier::Novice.regen_mult(), 1.0);
        assert!(DishTier::Skilled.duration_mult() > DishTier::Novice.duration_mult());
        assert!(DishTier::Signature.duration_mult() > DishTier::Skilled.duration_mult());
        assert!(DishTier::Skilled.regen_mult() > DishTier::Novice.regen_mult());
        assert!(DishTier::Signature.regen_mult() > DishTier::Skilled.regen_mult());
    }

    #[test]
    fn badge_only_for_earned_tiers() {
        assert_eq!(DishTier::Novice.badge(), None); // 生手不標記、省流量
        assert_eq!(DishTier::Skilled.badge(), Some("skilled"));
        assert_eq!(DishTier::Signature.badge(), Some("signature"));
    }

    #[test]
    fn record_only_counts_real_dishes() {
        let mut m = DishMastery::default();
        // 料理：入帳。
        assert!(m.record_cook(ItemKind::GrilledFish).is_some());
        assert_eq!(m.count_of(ItemKind::GrilledFish), 1);
        // 非料理（材料／工具）：不入帳、回 None。
        assert!(m.record_cook(ItemKind::Wood).is_none());
        assert_eq!(m.count_of(ItemKind::Wood), 0);
    }

    #[test]
    fn count_accumulates_per_dish_independently() {
        let mut m = DishMastery::default();
        for _ in 0..3 {
            m.record_cook(ItemKind::Bread);
        }
        m.record_cook(ItemKind::FriedEgg);
        assert_eq!(m.count_of(ItemKind::Bread), 3);
        assert_eq!(m.count_of(ItemKind::FriedEgg), 1);
        // 互不干擾：麵包還是生手、煎蛋也是生手。
        assert_eq!(m.tier_of(ItemKind::Bread), DishTier::Novice);
    }

    #[test]
    fn tier_up_fires_exactly_on_threshold_crossing() {
        let mut m = DishMastery::default();
        let mut ups = 0;
        for i in 1..=SIGNATURE_AT {
            let rec = m.record_cook(ItemKind::DeepBroth).unwrap();
            assert_eq!(rec.count, i);
            if rec.tier_up {
                ups += 1;
            }
        }
        // 整段（1..=15）剛好跨兩道門檻：→順手(5)、→拿手(15)。
        assert_eq!(ups, 2);
        assert_eq!(m.tier_of(ItemKind::DeepBroth), DishTier::Signature);
    }

    #[test]
    fn tier_up_flags_align_with_counts() {
        let mut m = DishMastery::default();
        // 第 5 次升順手。
        for _ in 1..SKILLED_AT {
            assert!(!m.record_cook(ItemKind::CarrotSoup).unwrap().tier_up);
        }
        let at_five = m.record_cook(ItemKind::CarrotSoup).unwrap();
        assert!(at_five.tier_up);
        assert_eq!(at_five.tier, DishTier::Skilled);
    }

    #[test]
    fn scale_meal_grows_buff_for_signature() {
        let base = crate::meal_buff::meal_buff_for(ItemKind::Bread).unwrap();
        let novice = scale_meal(Some(base), DishTier::Novice).unwrap();
        let signature = scale_meal(Some(base), DishTier::Signature).unwrap();
        // 生手＝原樣；拿手更久、回得更多。
        assert!((novice.total_secs - base.total_secs).abs() < 1e-6);
        assert!(signature.total_secs > base.total_secs);
        assert!(signature.hp_per_sec > base.hp_per_sec);
        // 剛吃下＝剩餘等於放大後總時長。
        assert!((signature.remaining_secs - signature.total_secs).abs() < 1e-6);
    }

    #[test]
    fn scale_meal_passes_through_none() {
        // 非料理（meal_buff_for 回 None）原樣傳回 None。
        assert!(scale_meal(None, DishTier::Signature).is_none());
    }

    #[test]
    fn empty_mastery_defaults_to_novice() {
        let m = DishMastery::default();
        assert_eq!(m.count_of(ItemKind::StarSashimi), 0);
        assert_eq!(m.tier_of(ItemKind::StarSashimi), DishTier::Novice);
    }
}
